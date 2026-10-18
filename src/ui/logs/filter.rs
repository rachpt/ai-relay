//! 日志筛选与有界文本处理：不访问窗口句柄或磁盘。
use super::*;

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

// Dates are whole UTC days; numeric endpoints remain inclusive UTC Unix seconds
// for parser/API callers. The GUI picker accepts only ISO dates or empty, not raw epoch.
pub(super) fn utc_bound(input: &str, end: bool) -> Result<Option<u64>, String> {
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
    let invalid = || "时间须为非负 UTC epoch 秒或 YYYY-MM-DD".to_string();
    let bytes = input.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return Err(invalid());
    }
    let number = |range: std::ops::Range<usize>| -> Result<u64, String> {
        let part = &bytes[range];
        if !part.iter().all(u8::is_ascii_digit) {
            return Err(invalid());
        }
        Ok(part.iter().fold(0, |n, c| n * 10 + u64::from(c - b'0')))
    };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let leap = |y: u64| y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if year < 1970 || !(1..=12).contains(&month) || day == 0 || day > months[month as usize - 1] {
        return Err(invalid());
    }
    let leap_days = |y: u64| y / 4 - y / 100 + y / 400;
    let days = (year - 1970) * 365 + leap_days(year - 1) - leap_days(1969)
        + months[..month as usize - 1].iter().sum::<u64>()
        + day
        - 1;
    Ok(Some(days * 86400 + if end { 86399 } else { 0 }))
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
            return Err("UTC 起始时间不得晚于结束时间".into());
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

pub(super) fn filter(source: &str, filters: &Filters) -> Result<String, String> {
    filters.validate()?;
    let mut result = String::new();
    for line in source.lines().take(MAX_LINES) {
        if !line.to_lowercase().contains(&filters.search) {
            continue;
        }
        let matches = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(value) => filters.matches_file(&value),
            Err(_) => !filters.needs_metadata(),
        };
        if matches {
            if result.len() + line.len() + 2 > LIMIT {
                break;
            }
            result.push_str(line);
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
        if !line.to_lowercase().contains(&filters.search) {
            continue;
        }
        let matches = if let Some(number) = sequence(line) {
            let Some(entry) = entries.get(&number) else {
                continue;
            };
            if entry.sequence <= cursor {
                continue;
            }
            let value =
                serde_json::to_value(entry).map_err(|_| "实时日志元数据序列化失败".to_string())?;
            filters.matches_file(&value)
        } else {
            // Preserve the controller's pre-initialization message, without
            // allowing untyped lines to satisfy any metadata filter.
            !filters.needs_metadata()
        };
        if matches {
            if result.len() + line.len() + 2 > LIMIT {
                break;
            }
            result.push_str(line);
            result.push_str("\r\n");
        }
    }
    Ok(result)
}
