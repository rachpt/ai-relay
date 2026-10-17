//! 被动、尽力而为的 provider 用量观察；不修改流、不执行 I/O、不持久化正文。
//! JSON 仅在有界临时缓冲中解析，SSE 每事件解析后丢弃；报告不包含正文或错误文本。
//! 用量不是账单：未知字段保持 None，聚合只累加实际观察到的字段。

mod observer;
mod store;

pub use observer::{MAX_JSON_BYTES, MAX_SSE_BYTES, UsageObserver};
pub use store::{MAX_GROUPS, UsageAggregate, UsageCounters, UsageGroup, UsageSnapshot, UsageStore};

pub const MAX_MODEL_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub enum Provider {
    #[default]
    Unknown,
    OpenAiChat,
    OpenAiResponses,
    Anthropic,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UsageCompleteness {
    #[default]
    Unknown,
    Partial,
    Complete,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ObservationFormat {
    #[default]
    UnsupportedContentType,
    UnsupportedContentEncoding,
    Json,
    Sse,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    /// OpenAI 输入中的缓存子项；不得再次加到总量。
    pub cached_input_tokens: Option<u64>,
    /// OpenAI 输出中的推理子项；不得再次加到总量。
    pub reasoning_tokens: Option<u64>,
    /// Anthropic input_tokens 不包含这两个独立项。
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
}

impl TokenUsage {
    pub fn completeness(&self) -> UsageCompleteness {
        if self.input_tokens.is_some()
            && self.output_tokens.is_some()
            && self.total_tokens.is_some()
        {
            UsageCompleteness::Complete
        } else if self != &Self::default() {
            UsageCompleteness::Partial
        } else {
            UsageCompleteness::Unknown
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UsageReport {
    pub provider: Provider,
    pub model: Option<String>,
    pub usage: TokenUsage,
    pub format: ObservationFormat,
    /// 独立于字段完整度：已观察到的计数仍可保留，但不能认为整个响应可靠完成。
    pub truncated: bool,
    pub parse_error: bool,
    pub limit_exceeded: bool,
    pub provider_error: bool,
    pub arithmetic_overflow: bool,
}

impl UsageReport {
    pub fn completeness(&self) -> UsageCompleteness {
        self.usage.completeness()
    }
}

pub(crate) fn filtered_model(model: &str) -> Option<String> {
    // 只允许常见模型标识符；不截短任意文本，避免把 prompt 当模型收集。
    (!model.is_empty()
        && model.len() <= MAX_MODEL_BYTES
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/@".contains(&b)))
    .then(|| model.to_owned())
}
