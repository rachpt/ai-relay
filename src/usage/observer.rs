use serde_json::Value;

use super::{ObservationFormat, Provider, TokenUsage, UsageReport, filtered_model};

pub const MAX_SSE_BYTES: usize = 64 * 1024;
pub const MAX_JSON_BYTES: usize = 256 * 1024;

/// 每响应一个 observer。feed 只读字节，finish 消费 observer 并释放临时正文。
/// 不支持的 Content-Type/Content-Encoding 不读取正文；只接受无编码或 identity。
pub struct UsageObserver {
    report: UsageReport,
    buffer: Vec<u8>,
    data: Vec<u8>,
    event: String,
    event_bytes: usize,
    skip_lf: bool,
    has_data: bool,
    stopped: bool,
    final_seen: bool,
}

impl UsageObserver {
    pub fn new(content_type: &str, content_encoding: Option<&str>) -> Self {
        let format = if content_encoding.is_some_and(|s| !s.trim().eq_ignore_ascii_case("identity"))
        {
            ObservationFormat::UnsupportedContentEncoding
        } else {
            let media = content_type.split(';').next().unwrap_or_default().trim();
            if media.eq_ignore_ascii_case("text/event-stream") {
                ObservationFormat::Sse
            } else if media.eq_ignore_ascii_case("application/json") {
                ObservationFormat::Json
            } else {
                ObservationFormat::UnsupportedContentType
            }
        };
        Self {
            report: UsageReport {
                format,
                ..UsageReport::default()
            },
            buffer: Vec::new(),
            data: Vec::new(),
            event: String::new(),
            event_bytes: 0,
            skip_lf: false,
            has_data: false,
            stopped: false,
            final_seen: false,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        if self.stopped {
            return;
        }
        match self.report.format {
            ObservationFormat::Json => {
                if bytes.len() > MAX_JSON_BYTES.saturating_sub(self.buffer.len()) {
                    self.limit();
                } else {
                    self.buffer.extend_from_slice(bytes);
                }
            }
            ObservationFormat::Sse => {
                for &byte in bytes {
                    if self.stopped {
                        break;
                    }
                    if self.skip_lf && byte == b'\n' {
                        self.skip_lf = false;
                        continue;
                    }
                    self.skip_lf = false;
                    if byte == b'\r' || byte == b'\n' {
                        self.skip_lf = byte == b'\r';
                        self.line();
                    } else if self.buffer.len() == MAX_SSE_BYTES
                        || self.buffer.len() >= MAX_SSE_BYTES.saturating_sub(self.event_bytes)
                    {
                        self.limit();
                    } else {
                        self.buffer.push(byte);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn finish(mut self, complete: bool) -> UsageReport {
        self.report.truncated = !complete;
        match self.report.format {
            ObservationFormat::Json if !self.report.limit_exceeded => {
                let bytes = std::mem::take(&mut self.buffer);
                self.json(&bytes, "");
            }
            ObservationFormat::Sse if !self.stopped => {
                // SSE 必须由空行提交事件；EOF 不把残缺 final 当作成功。
                self.report.truncated |= !self.buffer.is_empty()
                    || self.event_bytes != 0
                    || (self.report.provider != Provider::Unknown && !self.final_seen);
            }
            _ => {}
        }
        if self.report.provider == Provider::Anthropic {
            let u = &mut self.report.usage;
            u.total_tokens = match (
                u.input_tokens,
                u.cache_creation_input_tokens,
                u.cache_read_input_tokens,
                u.output_tokens,
            ) {
                (Some(a), Some(b), Some(c), Some(d)) => {
                    let total = a
                        .checked_add(b)
                        .and_then(|v| v.checked_add(c))
                        .and_then(|v| v.checked_add(d));
                    self.report.arithmetic_overflow = total.is_none();
                    total
                }
                _ => None,
            };
        }
        self.report
    }

    fn limit(&mut self) {
        self.report.limit_exceeded = true;
        self.stopped = true;
        self.buffer.clear();
        self.data.clear();
        self.event.clear();
    }

    fn line(&mut self) {
        if self.buffer.is_empty() {
            if self.has_data {
                let data = std::mem::take(&mut self.data);
                let event = std::mem::take(&mut self.event);
                if data == b"[DONE]" {
                    self.final_seen = true;
                    self.stopped = true;
                } else {
                    self.json(&data, &event);
                }
            }
            self.data.clear();
            self.event.clear();
            self.has_data = false;
            self.event_bytes = 0;
            return;
        }
        // Count wire line bytes (including comments/unknown fields), not only data.
        if self.buffer.len() + 1 > MAX_SSE_BYTES.saturating_sub(self.event_bytes) {
            self.limit();
            return;
        }
        self.event_bytes += self.buffer.len() + 1;
        let line = std::mem::take(&mut self.buffer);
        let Ok(line) = std::str::from_utf8(&line) else {
            self.report.parse_error = true;
            return;
        };
        let (name, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match name {
            "data" => {
                if self.has_data {
                    self.data.push(b'\n');
                }
                self.has_data = true;
                self.data.extend_from_slice(value.as_bytes());
            }
            "event" => self.event = value.to_owned(),
            _ => {}
        }
    }

    fn json(&mut self, bytes: &[u8], event: &str) {
        let Ok(root) = serde_json::from_slice::<Value>(bytes) else {
            self.report.parse_error = true;
            return;
        };
        if !root.is_object() {
            self.report.parse_error = true;
            return;
        }
        let kind = root.get("type").and_then(Value::as_str).unwrap_or(event);
        let responses_terminal = matches!(
            kind,
            "response.completed" | "response.failed" | "response.incomplete"
        );
        let responses_error = matches!(kind, "response.failed" | "response.incomplete");
        if root.get("error").is_some_and(|v| !v.is_null())
            || kind == "error"
            || event == "error"
            || responses_error
        {
            self.report.provider_error = true;
            if !responses_terminal {
                return;
            }
        }
        if kind == "message_stop" && self.report.provider == Provider::Anthropic {
            self.final_seen = true;
            return;
        }
        let object = root
            .get("object")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (provider, payload) = if kind == "message_start" {
            (
                Provider::Anthropic,
                root.get("message").unwrap_or(&Value::Null),
            )
        } else if kind == "message_delta" || kind == "message" {
            (Provider::Anthropic, &root)
        } else if responses_terminal {
            (
                Provider::OpenAiResponses,
                root.get("response").unwrap_or(&Value::Null),
            )
        } else if object == "response" {
            (Provider::OpenAiResponses, &root)
        } else if object.starts_with("chat.completion") {
            (Provider::OpenAiChat, &root)
        } else if let Some(usage) = root.get("usage").filter(|v| v.is_object()) {
            if usage.get("prompt_tokens").is_some() || usage.get("completion_tokens").is_some() {
                (Provider::OpenAiChat, &root)
            } else if usage.get("input_tokens").is_some() && usage.get("output_tokens").is_some() {
                // Responses has total_tokens; Anthropic does not. Explicit type wins above.
                if usage.get("total_tokens").is_some() {
                    (Provider::OpenAiResponses, &root)
                } else {
                    (Provider::Anthropic, &root)
                }
            } else {
                return;
            }
        } else {
            return;
        };
        if self.report.provider != Provider::Unknown && self.report.provider != provider {
            self.report.parse_error = true;
            return;
        }
        self.report.provider = provider;
        if self.final_seen {
            return;
        }
        // Terminal means no later snapshot may replace this one, not success.
        // Mark it before reading optional/invalid usage so absence is terminal too.
        self.final_seen |= responses_terminal;
        if let Some(model) = payload
            .get("model")
            .and_then(Value::as_str)
            .and_then(filtered_model)
        {
            self.report.model = Some(model);
        }
        let Some(usage) = payload.get("usage").filter(|v| !v.is_null()) else {
            return;
        };
        if !usage.is_object() {
            self.report.parse_error = true;
            return;
        }
        let mut invalid = false;
        let mut read = |name: &str| number(usage, name, &mut invalid);
        let mut next = TokenUsage::default();
        match provider {
            Provider::OpenAiChat => {
                next.input_tokens = read("prompt_tokens");
                next.output_tokens = read("completion_tokens");
                next.total_tokens = read("total_tokens");
                next.cached_input_tokens = detail(
                    usage,
                    "prompt_tokens_details",
                    "cached_tokens",
                    &mut invalid,
                );
                next.reasoning_tokens = detail(
                    usage,
                    "completion_tokens_details",
                    "reasoning_tokens",
                    &mut invalid,
                );
            }
            Provider::OpenAiResponses => {
                next.input_tokens = read("input_tokens");
                next.output_tokens = read("output_tokens");
                next.total_tokens = read("total_tokens");
                next.cached_input_tokens =
                    detail(usage, "input_tokens_details", "cached_tokens", &mut invalid);
                next.reasoning_tokens = detail(
                    usage,
                    "output_tokens_details",
                    "reasoning_tokens",
                    &mut invalid,
                );
            }
            Provider::Anthropic => {
                next.input_tokens = read("input_tokens");
                next.output_tokens = read("output_tokens");
                next.cache_creation_input_tokens = read("cache_creation_input_tokens");
                next.cache_read_input_tokens = read("cache_read_input_tokens");
            }
            Provider::Unknown => {}
        }
        self.report.parse_error |= invalid;
        let current = &mut self.report.usage;
        // Each field is an absolute snapshot, never an increment. Missing fields retain
        // earlier observations (message_delta normally omits the input counters).
        macro_rules! merge {
            ($($field:ident),*) => { $(if next.$field.is_some() { current.$field = next.$field; })* };
        }
        merge!(
            input_tokens,
            output_tokens,
            total_tokens,
            cached_input_tokens,
            reasoning_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens
        );
        if provider == Provider::OpenAiChat
            && current.total_tokens.is_some()
            && current.input_tokens.is_some()
            && current.output_tokens.is_some()
        {
            self.final_seen = true;
        }
    }
}

fn number(value: &Value, name: &str, invalid: &mut bool) -> Option<u64> {
    match value.get(name) {
        None | Some(Value::Null) => None,
        Some(value) => {
            let result = value.as_u64();
            *invalid |= result.is_none();
            result
        }
    }
}

fn detail(value: &Value, name: &str, field: &str, invalid: &mut bool) -> Option<u64> {
    match value.get(name) {
        None | Some(Value::Null) => None,
        Some(value) if value.is_object() => number(value, field, invalid),
        Some(_) => {
            *invalid = true;
            None
        }
    }
}
