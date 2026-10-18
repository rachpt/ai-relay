//! 日志筛选与有界文本处理：不访问窗口句柄或磁盘。
use super::*;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Offset, TimeZone};

pub(super) const MAX_LINES: usize = 1000;
pub(super) const LIMIT: usize = 256 * 1024;

pub(super) fn bounded_file_text(first: &str, second: &str, keep_end: bool) -> String {
    let mut joined = String::with_capacity((first.len() + second.len()).min(LIMIT * 2));
    joined.push_str(first);
    joined.push_str(second);
    if joined.len() > LIMIT {
        if keep_end {
            let mut start = joined.len() - LIMIT;
            while !joined.is_char_boundary(start) {
                start += 1;
            }
            if let Some(newline) = joined[start..].find('\n') {
                start += newline + 1;
            }
            joined.drain(..start);
        } else {
            let mut end = LIMIT;
            while !joined.is_char_boundary(end) {
                end -= 1;
            }
            if let Some(newline) = joined[..end].rfind('\n') {
                end = newline + 1;
            }
            joined.truncate(end);
        }
    }
    let lines: Vec<_> = joined.lines().collect();
    if lines.len() > MAX_LINES {
        let selected = if keep_end {
            &lines[lines.len() - MAX_LINES..]
        } else {
            &lines[..MAX_LINES]
        };
        return selected.join("\n") + "\n";
    }
    joined
}
fn sequence(line: &str) -> Option<u64> {
    line.split_whitespace()
        .nth(1)?
        .strip_prefix('#')?
        .parse()
        .ok()
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Filters {
    pub(super) search: String,
    pub(super) level: usize,
    pub(super) start: Option<u64>,
    pub(super) end: Option<u64>,
    pub(super) category: usize,
    pub(super) status: Option<(u16, u16)>,
    pub(super) errors_only: bool,
}

// Numeric endpoints remain inclusive UTC Unix seconds. Only civil dates use the
// system zone; the picker itself accepts ISO dates or empty, never raw epoch.
pub(super) fn local_bound(input: &str, end: bool) -> Result<Option<u64>, String> {
    bound_with_zone(input, end, &Local)
}

pub(super) fn bound_with_zone<T: TimeZone>(
    input: &str,
    end: bool,
    zone: &T,
) -> Result<Option<u64>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    if input.bytes().all(|c| c.is_ascii_digit()) {
        return input
            .parse()
            .map(Some)
            .map_err(|_| "UTC epoch 秒超出范围".into());
    }
    let invalid = || "时间须为非负 UTC epoch 秒或本地日期 YYYY-MM-DD（1970–9999）".to_string();
    let bytes = input.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return Err(invalid());
    }
    let date = NaiveDate::parse_from_str(input, "%Y-%m-%d").map_err(|_| invalid())?;
    if !(1970..=9999).contains(&date.year()) || date.format("%Y-%m-%d").to_string() != input {
        return Err(invalid());
    }
    // Resolve each endpoint independently, using the rules on that date rather
    // than today's offset or a fixed 86400 seconds. Folds include both copies;
    // midnight gaps use the first/last valid second of the selected civil day.
    let edge = date
        .and_hms_opt(
            if end { 23 } else { 0 },
            if end { 59 } else { 0 },
            if end { 59 } else { 0 },
        )
        .ok_or_else(invalid)?;
    let choose = |seconds: i64| {
        let wall =
            edge.checked_add_signed(Duration::seconds(if end { -seconds } else { seconds }))?;
        let result = zone.from_local_datetime(&wall);
        if end {
            result.latest()
        } else {
            result.earliest()
        }
    };
    let mut instant = choose(0);
    if instant.is_none() {
        // Minute probes keep unusual midnight gaps bounded, with second-level
        // refinement for historical transitions. Never cross into another day.
        for minute in 1..=1440 {
            let seconds = (minute * 60).min(86399);
            if choose(seconds).is_some() {
                instant = ((minute - 1) * 60 + 1..=seconds).find_map(choose);
                break;
            }
        }
    }
    let seconds = instant
        .ok_or("系统时区不存在所选日期，请选择相邻的有效日期")?
        .timestamp();
    if end && seconds < 0 {
        return Err("所选日期早于日志 UTC epoch 范围".into());
    }
    // LogEntry timestamps are nonnegative; the first local day may begin before
    // epoch in a positive-offset zone, so intersect it with the storage range.
    Ok(Some(seconds.max(0) as u64))
}

pub(super) fn status_range(input: &str) -> Result<Option<(u16, u16)>, String> {
    let input = input.trim().to_ascii_lowercase();
    if input.is_empty() {
        return Ok(None);
    }
    let invalid = || "状态须为 100–599 单码、1xx–5xx 或区间（如 400-499）".to_string();
    let code = |s: &str| -> Result<u16, String> {
        if s.len() != 3 || !s.bytes().all(|c| c.is_ascii_digit()) {
            return Err(invalid());
        }
        s.parse::<u16>()
            .ok()
            .filter(|n| (100..=599).contains(n))
            .ok_or_else(invalid)
    };
    let (start, end) = if let Some(class) = input.strip_suffix("xx") {
        let start = code(&format!("{class}00"))?;
        (start, start + 99)
    } else if let Some((a, b)) = input.split_once('-') {
        (code(a.trim())?, code(b.trim())?)
    } else {
        let value = code(&input)?;
        (value, value)
    };
    if start > end {
        return Err(invalid());
    }
    Ok(Some((start, end)))
}

impl Filters {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self
            .start
            .zip(self.end)
            .is_some_and(|(start, end)| start > end)
        {
            return Err("起始时间不得晚于结束时间".into());
        }
        Ok(())
    }
    pub(super) fn needs_metadata(&self) -> bool {
        self.level != 0
            || self.start.is_some()
            || self.end.is_some()
            || self.category != 0
            || self.status.is_some()
            || self.errors_only
    }
    pub(super) fn matches_file(&self, value: &serde_json::Value) -> bool {
        let event = &value["event"];
        let category = event["category"].as_str();
        let access = category == Some("access");
        let timestamp = value["timestamp"].as_u64();
        let level = ["", "trace", "debug", "info", "warn", "error"];
        let status = if access {
            event["metadata"]["status"].as_u64()
        } else {
            None
        };
        // Only the schema's error enum is accepted, not incidental strings/objects.
        let error = if access {
            &event["metadata"]["error"]
        } else if category == Some("runtime") {
            &event["error"]
        } else {
            &serde_json::Value::Null
        };
        let error_code = error.as_str().is_some_and(|code| {
            matches!(
                code,
                "io" | "permission_denied"
                    | "disk_full"
                    | "timeout"
                    | "cancelled"
                    | "connection"
                    | "tls"
                    | "protocol"
                    | "invalid_configuration"
                    | "resource_limit"
                    | "internal"
            )
        });
        (self.level == 0
            || level
                .get(self.level)
                .is_some_and(|l| event["level"].as_str() == Some(*l)))
            && self
                .start
                .is_none_or(|start| timestamp.is_some_and(|t| t >= start))
            && self
                .end
                .is_none_or(|end| timestamp.is_some_and(|t| t <= end))
            && (self.category == 0
                || category
                    == Some(if self.category == 1 {
                        "runtime"
                    } else {
                        "access"
                    }))
            && self
                .status
                .is_none_or(|(a, b)| status.is_some_and(|s| s >= u64::from(a) && s <= u64::from(b)))
            && (!self.errors_only
                || event["level"].as_str() == Some("error")
                || error_code
                || status.is_some_and(|s| (400..=599).contains(&s)))
    }
}

// GUI-only annotations retain the raw epoch/Debug text and the original JSON
// fields. Filtering metadata always uses the unmodified source, not these labels.
fn display_time<T: TimeZone>(timestamp: u64, zone: &T) -> Option<String> {
    let instant = DateTime::from_timestamp(i64::try_from(timestamp).ok()?, 0)?;
    let local = instant.with_timezone(zone);
    instant
        .naive_utc()
        .checked_add_offset(local.offset().fix())?;
    Some(local.fixed_offset().to_rfc3339())
}

fn file_display<T: TimeZone>(line: &str, value: &serde_json::Value, zone: &T) -> String {
    if let Some(time) = value["timestamp"]
        .as_u64()
        .and_then(|t| display_time(t, zone))
        && value.is_object()
        && value.get("timestamp_local").is_none()
        && let Some(open) = line.find('{')
    {
        // Insert instead of reserializing: preserve all original fields, number
        // precision, whitespace and ordering. Copied lines remain valid JSON.
        let (head, tail) = line.split_at(open + 1);
        return format!("{head}\"timestamp_local\":\"{time}\",{tail}");
    }
    line.to_string()
}

fn search_matches(raw: &str, display: &str, search: &str) -> bool {
    raw.to_lowercase().contains(search) || display.to_lowercase().contains(search)
}

pub(super) fn filter(source: &str, filters: &Filters) -> Result<String, String> {
    filter_with_zone(source, filters, &Local)
}

pub(super) fn filter_with_zone<T: TimeZone>(
    source: &str,
    filters: &Filters,
    zone: &T,
) -> Result<String, String> {
    filters.validate()?;
    let mut result = String::new();
    for line in source.lines().take(MAX_LINES) {
        let (matches, display) = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(value) => (
                filters.matches_file(&value),
                file_display(line, &value, zone),
            ),
            Err(_) => (!filters.needs_metadata(), line.to_string()),
        };
        if matches && search_matches(line, &display, &filters.search) {
            if result.len() + display.len() + 2 > LIMIT {
                break;
            }
            result.push_str(&display);
            result.push_str("\r\n");
        }
    }
    Ok(result)
}

pub(super) fn filter_live(
    source: &str,
    entries: &[LogEntry],
    cursor: u64,
    filters: &Filters,
) -> Result<String, String> {
    filter_live_with_zone(source, entries, cursor, filters, &Local)
}

pub(super) fn filter_live_with_zone<T: TimeZone>(
    source: &str,
    entries: &[LogEntry],
    cursor: u64,
    filters: &Filters,
    zone: &T,
) -> Result<String, String> {
    filters.validate()?;
    // Sequence only associates the original Debug display with typed metadata;
    // never interpret status, level, error or timestamp from the display text.
    let entries: std::collections::BTreeMap<_, _> = entries
        .iter()
        .rev()
        .take(MAX_LINES)
        .map(|entry| (entry.sequence, entry))
        .collect();
    let mut result = String::new();
    for line in source.lines().take(MAX_LINES) {
        let mut display = line.to_string();
        let matches = if let Some(number) = sequence(line) {
            let Some(entry) = entries.get(&number) else {
                continue;
            };
            if entry.sequence <= cursor {
                continue;
            }
            if let Some(time) = display_time(entry.timestamp, zone) {
                display = format!("{time} | {line}");
            }
            let value =
                serde_json::to_value(entry).map_err(|_| "实时日志元数据序列化失败".to_string())?;
            filters.matches_file(&value)
        } else {
            // Preserve the controller's pre-initialization message, without
            // allowing untyped lines to satisfy any metadata filter.
            !filters.needs_metadata()
        };
        if matches && search_matches(line, &display, &filters.search) {
            if result.len() + display.len() + 2 > LIMIT {
                break;
            }
            result.push_str(&display);
            result.push_str("\r\n");
        }
    }
    Ok(result)
}
