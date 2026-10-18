//! 日志页面仅消费后台返回的文本和状态，不创建 LogService、不访问磁盘。
//! 设置即时应用不代表持久化；正常保存配置才写盘。
use super::*;
use crate::{
    app::controller::{Command, LogResponse, LogViewSnapshot},
    config::{LogLevel, LoggingConfig},
    logging::{FileCursor, LogEntry, LogFile, PageCursor, ReadResponse},
};
use windows_sys::Win32::{Foundation::HINSTANCE, Graphics::Gdi::HFONT};

const MAX_LINES: usize = 1000;

fn bounded_file_text(first: &str, second: &str, keep_end: bool) -> String {
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
struct Filters {
    search: String,
    level: usize,
    start: Option<u64>,
    end: Option<u64>,
    category: usize,
    status: Option<(u16, u16)>,
    errors_only: bool,
}

// Dates are whole UTC days; numeric endpoints remain inclusive UTC Unix seconds
// for parser/API callers. The GUI picker accepts only ISO dates or empty, not raw epoch.
fn utc_bound(input: &str, end: bool) -> Result<Option<u64>, String> {
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

fn status_range(input: &str) -> Result<Option<(u16, u16)>, String> {
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
    fn validate(&self) -> Result<(), String> {
        if self
            .start
            .zip(self.end)
            .is_some_and(|(start, end)| start > end)
        {
            return Err("UTC 起始时间不得晚于结束时间".into());
        }
        Ok(())
    }
    fn needs_metadata(&self) -> bool {
        self.level != 0
            || self.start.is_some()
            || self.end.is_some()
            || self.category != 0
            || self.status.is_some()
            || self.errors_only
    }
    fn matches_file(&self, value: &serde_json::Value) -> bool {
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

fn filter(source: &str, filters: &Filters) -> Result<String, String> {
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

fn filter_live(
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

// No State references may cross confirmation dialogs, which run nested message loops.
pub(super) unsafe fn poll(hwnd: HWND, state: *mut State) {
    // SAFETY: Stable owner-thread State pointer; borrows end before every modal call.
    unsafe {
        if let Ok(snapshot) = (*state).controller.log_snapshots.try_recv() {
            let logs = &mut (*state).logs;
            if !logs.initialized && snapshot.directory.is_some() {
                logs.populate_options(&snapshot.logging);
            }
            let first = snapshot.entries.first().map(|entry| entry.sequence);
            if logs.displayed_cursor != 0
                && first.is_some_and(|n| n > logs.displayed_cursor.saturating_add(1))
            {
                logs.gap = true;
            }
            logs.gap |= snapshot.gap;
            logs.initialized = snapshot.directory.is_some() && !snapshot.status.worker_stopped;
            logs.snapshot = Some(snapshot);
        }
        // Bounded result queue; one result per tick also limits modal confirmations.
        if let Ok(response) = (*state).controller.log_events.try_recv() {
            if let LogResponse::ClearPreview { all, files, bytes } = response {
                let message = format!(
                    "将删除{}：{files} 个文件 / {bytes} bytes。\n仅限当前日志目录内本应用管理的日志；不删除配置、密钥、统计或旧版日志。\n后台执行时范围可能因轮转变化。确认继续？",
                    if all {
                        "全部应用日志（含当前文件）"
                    } else {
                        "已关闭的历史日志（不含当前文件）"
                    }
                );
                let accepted = settings::confirm(hwnd, &message);
                let accepted = accepted
                    && (!all
                        || settings::confirm(
                            hwnd,
                            "再次确认清空全部应用日志？当前日志将由后台关闭／重开，新日志仍可继续产生。此操作不可撤销。",
                        ));
                if (*state).exiting.is_none() && accepted {
                    let command = if all {
                        Command::LogClearAll { confirmed: true }
                    } else {
                        Command::LogClearHistory { confirmed: true }
                    };
                    (*state).logs.send(&(*state).controller, command);
                } else {
                    (*state).logs.pending = false;
                    (*state).logs.message = "已取消删除，未发送执行命令".into();
                }
            } else if let Some(logging) = (*state).logs.response(response) {
                (*state).settings.sync_logging(logging);
            }
        }
        (*state).logs.visible = (*state).tabs.active == 2 && IsWindowVisible(hwnd) != 0;
        if (*state).exiting.is_none() {
            (*state).logs.tick(&(*state).controller);
        }
        (*state).logs.enable();
        (*state).logs.render();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_cursor_and_filters_are_independent() {
        let entries = live_fixture();
        let text = debug_text(&entries);
        let all = Filters::default();
        assert!(
            !filter_live(&text, &entries, 1, &all)
                .unwrap()
                .contains("Started")
        );
        let errors = Filters {
            search: "failed".into(),
            level: 5,
            ..all.clone()
        };
        assert!(
            filter_live(&text, &entries, 1, &errors)
                .unwrap()
                .contains("#2")
        );
        assert!(
            filter_live(
                &text,
                &entries,
                0,
                &Filters {
                    level: 1,
                    ..all.clone()
                }
            )
            .unwrap()
            .is_empty()
        );
        assert!(filter_live(&text, &entries, 6, &all).unwrap().is_empty());
    }
    #[test]
    fn file_levels_use_json_not_incidental_text() {
        let text = "{\"sequence\":1,\"event\":{\"level\":\"info\",\"error\":\"error\"}}\n";
        let filters = Filters {
            level: 5,
            ..Filters::default()
        };
        assert!(filter(text, &filters).unwrap().is_empty());
        assert!(
            filter(
                text,
                &Filters {
                    search: "sequence".into(),
                    level: 3,
                    ..filters.clone()
                }
            )
            .unwrap()
            .contains("sequence")
        );
        assert!(filter("invalid json", &filters).unwrap().is_empty());
    }
    #[test]
    fn file_append_and_prepend_are_utf8_bounded() {
        let first = "早期记录\n".repeat(LIMIT / 4);
        let last = "最新记录\n".repeat(LIMIT / 4);
        let appended = bounded_file_text(&first, &last, true);
        let prepended = bounded_file_text(&first, &last, false);
        assert!(appended.len() <= LIMIT && prepended.len() <= LIMIT);
        assert!(appended.ends_with("最新记录\n"));
        assert!(prepended.starts_with("早期记录\n"));
    }

    #[test]
    fn utc_dates_and_epoch_are_explicit_and_validated() {
        assert_eq!(utc_bound("1970-01-01", false).unwrap(), Some(0));
        assert_eq!(utc_bound("1970-01-01", true).unwrap(), Some(86399));
        assert_eq!(utc_bound("2000-03-01", false).unwrap(), Some(951868800));
        assert_eq!(utc_bound("2024-02-29", false).unwrap(), Some(1709164800));
        assert_eq!(utc_bound(" 123 ", true).unwrap(), Some(123));
        assert_eq!(utc_bound("", false).unwrap(), None);
        for invalid in [
            "1969-12-31",
            "2023-02-29",
            "2100-02-29",
            "2024-00-01",
            "2024-13-01",
            "2024-01-00",
            "2024-04-31",
            "2024-1-01",
            "2024-01-01T00:00Z",
            "-1",
            "18446744073709551616",
            "中文日期输入",
        ] {
            assert!(utc_bound(invalid, false).is_err(), "{invalid}");
        }
        let filters = Filters {
            start: Some(2),
            end: Some(1),
            ..Filters::default()
        };
        assert!(filter("", &filters).unwrap_err().contains("不得晚于"));
    }

    #[test]
    fn utc_date_filters_include_both_days_in_file_and_live_views() {
        let start = utc_bound("2024-02-28", false).unwrap().unwrap();
        let end = utc_bound("2024-02-29", true).unwrap().unwrap();
        let filters = Filters {
            start: Some(start),
            end: Some(end),
            ..Filters::default()
        };
        let entry = live_fixture()[0].clone();
        let entries: Vec<_> = [start - 1, start, end - 43200, end, end + 1]
            .into_iter()
            .enumerate()
            .map(|(i, timestamp)| LogEntry {
                sequence: i as u64 + 1,
                timestamp,
                ..entry.clone()
            })
            .collect();
        let file = entries
            .iter()
            .map(|entry| serde_json::to_string(entry).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let live = debug_text(&entries);
        let file_output = filter(&file, &filters).unwrap();
        let live_output = filter_live(&live, &entries, 0, &filters).unwrap();
        assert_eq!(file_output.lines().count(), 3);
        assert_eq!(live_output.lines().count(), 3);
        for sequence in 1..=5 {
            let included = (2..=4).contains(&sequence);
            assert_eq!(
                file_output.contains(&format!("\"sequence\":{sequence},")),
                included
            );
            assert_eq!(live_output.contains(&format!("#{sequence} ")), included);
        }
        assert_eq!(end - start, 2 * 86400 - 1);
        let final_day = Filters {
            start: utc_bound("2024-02-29", false).unwrap(),
            ..filters
        };
        assert_eq!(filter(&file, &final_day).unwrap().lines().count(), 2);
        assert_eq!(
            filter_live(&live, &entries, 0, &final_day)
                .unwrap()
                .lines()
                .count(),
            2
        );
    }

    #[test]
    fn status_syntax_is_validated_not_silently_empty() {
        for (input, expected) in [
            ("200", (200, 200)),
            ("2xx", (200, 299)),
            ("5XX", (500, 599)),
            (" 400 - 499 ", (400, 499)),
        ] {
            assert_eq!(status_range(input).unwrap(), Some(expected));
        }
        assert_eq!(status_range(" ").unwrap(), None);
        for invalid in [
            "99",
            "600",
            "0xx",
            "6xx",
            "20xx",
            "500-400",
            "200-",
            "200,201",
            "status:200",
            "２００",
        ] {
            assert!(status_range(invalid).is_err(), "{invalid}");
        }
    }

    fn fixture() -> String {
        [
            r#"{"sequence":1,"timestamp":100,"event":{"category":"runtime","level":"info","event":"started","error":null},"text":"status: 503 error timeout"}"#,
            r#"{"sequence":2,"timestamp":101,"event":{"category":"access","level":"info","metadata":{"status":200,"error":null}},"text":"500 error"}"#,
            r#"{"sequence":3,"timestamp":102,"event":{"category":"access","level":"warn","metadata":{"status":503,"error":null}}}"#,
            r#"{"sequence":4,"timestamp":103,"event":{"category":"access","level":"info","metadata":{"status":null,"error":"timeout"}}}"#,
            r#"{"sequence":5,"timestamp":104,"event":{"category":"runtime","level":"error","event":"operation_failed","error":"io"}}"#,
            r#"{"sequence":6,"timestamp":105,"event":{"category":"access","level":"info","metadata":{"status":200,"error":null}}}"#,
        ].join("\n")
    }

    #[test]
    fn structured_metadata_and_all_conditions_are_anded() {
        let source = fixture();
        let filters = Filters {
            start: Some(102),
            end: Some(102),
            category: 2,
            status: Some((500, 599)),
            errors_only: true,
            level: 4,
            search: "metadata".into(),
        };
        let output = filter(&source, &filters).unwrap();
        assert_eq!(output.lines().count(), 1);
        assert!(output.contains("\"sequence\":3"));
        assert!(
            filter(
                &source,
                &Filters {
                    search: "started".into(),
                    ..filters.clone()
                }
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            filter(
                &source,
                &Filters {
                    level: 3,
                    ..filters.clone()
                }
            )
            .unwrap()
            .is_empty()
        );
        let success = filter(
            &source,
            &Filters {
                status: Some((200, 200)),
                ..Filters::default()
            },
        )
        .unwrap();
        assert_eq!(success.lines().count(), 2);
        assert!(!success.contains("started"));
        let errors = filter(
            &source,
            &Filters {
                errors_only: true,
                ..Filters::default()
            },
        )
        .unwrap();
        assert_eq!(errors.lines().count(), 3);
        assert!(!errors.contains("\"sequence\":1"));
        assert!(!errors.contains("\"sequence\":2"));
        let runtime = filter(
            &source,
            &Filters {
                category: 1,
                ..Filters::default()
            },
        )
        .unwrap();
        assert_eq!(runtime.lines().count(), 2);
    }

    #[test]
    fn missing_malformed_and_incidental_fields_never_match_metadata() {
        let text = ["invalid json", r#"{"text":"status 500 error","timestamp":"100","event":{"category":"runtime","level":"info","status":500,"metadata":{"status":500},"error":"not-an-error-code"}}"#,
            r#"{"event":{"category":"access","level":"info","metadata":{"status":"500","error":null}}}"#].join("\n");
        for filters in [
            Filters {
                status: Some((500, 599)),
                ..Filters::default()
            },
            Filters {
                errors_only: true,
                ..Filters::default()
            },
            Filters {
                start: Some(0),
                ..Filters::default()
            },
        ] {
            assert!(filter(&text, &filters).unwrap().is_empty());
        }
        assert_eq!(
            filter(&text, &Filters::default()).unwrap().lines().count(),
            3
        );
    }

    #[test]
    fn live_metadata_is_typed_and_keeps_original_debug_lines() {
        let entries = live_fixture();
        // Deliberately misleading display text must affect text search only.
        let text = debug_text(&entries)
            .lines()
            .map(|line| format!("{line} status: Some(503) level: Error error: Some(Timeout)\n"))
            .collect::<String>();
        let all = Filters::default();
        let status = Filters {
            status: Some((500, 599)),
            ..all.clone()
        };
        let output = filter_live(&text, &entries, 0, &status).unwrap();
        assert_eq!(output.lines().count(), 1);
        assert!(output.contains("#4 Access"));
        assert!(output.contains("status: Some(503) level: Error"));
        let errors = filter_live(
            &text,
            &entries,
            0,
            &Filters {
                errors_only: true,
                ..all.clone()
            },
        )
        .unwrap();
        assert_eq!(errors.lines().count(), 4);
        assert!(!errors.contains("#1 "));
        assert!(!errors.contains("#3 "));
        let combined = Filters {
            start: Some(103),
            end: Some(103),
            category: 2,
            status: Some((503, 503)),
            errors_only: true,
            level: 4,
            search: "access".into(),
        };
        assert_eq!(filter_live(&text, &entries, 0, &combined).unwrap(), output);
        assert!(
            filter_live(&text, &entries, 4, &combined)
                .unwrap()
                .is_empty()
        );
        for mismatch in [
            Filters {
                category: 1,
                ..combined.clone()
            },
            Filters {
                level: 3,
                ..combined.clone()
            },
            Filters {
                search: "started".into(),
                ..combined.clone()
            },
            Filters {
                end: Some(102),
                start: None,
                ..combined.clone()
            },
        ] {
            assert!(
                filter_live(&text, &entries, 0, &mismatch)
                    .unwrap()
                    .is_empty()
            );
        }
        let runtime = filter_live(
            &text,
            &entries,
            0,
            &Filters {
                category: 1,
                ..all.clone()
            },
        )
        .unwrap();
        assert_eq!(runtime.lines().count(), 2);
        assert!(runtime.contains("#1 Runtime") && runtime.contains("#2 Runtime"));
        let success = filter_live(
            &text,
            &entries,
            0,
            &Filters {
                status: Some((200, 299)),
                ..all
            },
        )
        .unwrap();
        assert_eq!(success.lines().count(), 1);
        assert!(success.contains("#3 Access"));
        assert!(filter_live(&text, &[], 0, &status).unwrap().is_empty());
    }

    fn live_fixture() -> Vec<LogEntry> {
        use crate::logging::{AccessMetadata, ErrorCode, LogEvent, RuntimeEvent};
        [
            LogEvent::Runtime {
                level: LogLevel::Info,
                event: RuntimeEvent::Started,
                error: None,
            },
            LogEvent::Runtime {
                level: LogLevel::Error,
                event: RuntimeEvent::OperationFailed,
                error: Some(ErrorCode::Io),
            },
            LogEvent::Access {
                level: LogLevel::Info,
                metadata: AccessMetadata {
                    status: Some(200),
                    ..AccessMetadata::default()
                },
            },
            LogEvent::Access {
                level: LogLevel::Warn,
                metadata: AccessMetadata {
                    status: Some(503),
                    ..AccessMetadata::default()
                },
            },
            LogEvent::Access {
                level: LogLevel::Info,
                metadata: AccessMetadata {
                    error: Some(ErrorCode::Timeout),
                    ..AccessMetadata::default()
                },
            },
            LogEvent::Access {
                level: LogLevel::Error,
                metadata: AccessMetadata::default(),
            },
        ]
        .into_iter()
        .enumerate()
        .map(|(i, event)| LogEntry {
            sequence: i as u64 + 1,
            timestamp: 100 + i as u64,
            event,
        })
        .collect()
    }

    fn debug_text(entries: &[LogEntry]) -> String {
        entries
            .iter()
            .map(|entry| {
                format!(
                    "{} #{} {:?}\n",
                    entry.timestamp, entry.sequence, entry.event
                )
            })
            .collect()
    }

    fn live_snapshot(entries: Vec<LogEntry>) -> LogViewSnapshot {
        LogViewSnapshot {
            text: debug_text(&entries),
            cursor: entries.last().map_or(0, |entry| entry.sequence),
            entries,
            gap: false,
            status: Default::default(),
            directory: None,
            logging: Default::default(),
        }
    }

    #[test]
    fn file_cache_keeps_direction_and_complete_line_bounds() {
        let first = (0..1000).map(|n| format!("old-{n}\n")).collect::<String>();
        let last = (0..1000).map(|n| format!("new-{n}\n")).collect::<String>();
        assert_eq!(bounded_file_text(&first, &last, true), last);
        assert_eq!(bounded_file_text(&first, &last, false), first);
    }

    #[test]
    fn native_controls_layout_filters_pause_and_invalid_preservation() {
        use windows_sys::Win32::{
            Foundation::SYSTEMTIME,
            UI::Controls::{
                DTM_GETRANGE, DTM_GETSYSTEMTIME, DTM_SETSYSTEMTIME, DTS_SHOWNONE, GDT_NONE,
                GDT_VALID, GDTR_MAX, GDTR_MIN,
            },
        };

        struct Parent(HWND);
        impl Drop for Parent {
            fn drop(&mut self) {
                // SAFETY: This test owns the parent and all its child controls.
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        // SAFETY: No application singleton/controller is created. All windows are
        // owned, used and destroyed on this test thread, with no disk/network IO.
        unsafe {
            let instance = GetModuleHandleW(null());
            let parent = Parent(CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("logs test").as_ptr(),
                WS_POPUP,
                0,
                0,
                1800,
                2200,
                null_mut(),
                null_mut(),
                instance,
                null(),
            ));
            assert!(!parent.0.is_null());
            let mut logs = Logs::default();
            logs.create(parent.0, instance).unwrap();
            for (id, x) in [(2224, 24), (2225, 372)] {
                let picker = logs.control(id);
                assert!(!picker.is_null());
                assert_eq!(GetDlgCtrlID(picker), id as i32);
                assert_eq!(GetParent(picker), parent.0);
                assert!(logs.controls.contains(&(picker, x, 208, 336, 28)));
                let mut class = [0u16; 32];
                let count = GetClassNameW(picker, class.as_mut_ptr(), class.len() as i32);
                assert_eq!(
                    String::from_utf16(&class[..count as usize]).unwrap(),
                    "SysDateTimePick32"
                );
                let style = GetWindowLongPtrW(picker, GWL_STYLE) as u32;
                assert_ne!(style & DTS_SHOWNONE, 0);
                assert_ne!(style & WS_TABSTOP, 0);
                assert_eq!(settings::text(picker), "");
                let mut native = SYSTEMTIME::default();
                assert_eq!(
                    SendMessageW(
                        picker,
                        DTM_GETSYSTEMTIME,
                        0,
                        (&mut native as *mut SYSTEMTIME) as LPARAM,
                    ),
                    GDT_NONE as LRESULT
                );
                let mut range = [SYSTEMTIME::default(); 2];
                assert_eq!(
                    SendMessageW(picker, DTM_GETRANGE, 0, range.as_mut_ptr() as LPARAM),
                    (GDTR_MIN | GDTR_MAX) as LRESULT
                );
                assert_eq!(
                    (range[0].wYear, range[0].wMonth, range[0].wDay),
                    (1970, 1, 1)
                );
                assert_eq!(
                    (range[1].wYear, range[1].wMonth, range[1].wDay),
                    (9999, 12, 31)
                );
                for date in ["1970-01-01", "2024-02-29", "9999-12-31"] {
                    assert_ne!(
                        SendMessageW(picker, WM_SETTEXT, 0, wide(date).as_ptr() as LPARAM),
                        0
                    );
                    assert_eq!(settings::text(picker), date);
                    let filters = logs.filters().unwrap();
                    assert_eq!(
                        if id == 2224 {
                            filters.start
                        } else {
                            filters.end
                        },
                        utc_bound(date, id == 2225).unwrap()
                    );
                }
                // Strict picker writes reject raw epoch and malformed dates;
                // they preserve the valid value instead of creating an invalid draft.
                for invalid in [
                    "102",
                    "1969-12-31",
                    "10000-01-01",
                    "2023-02-29",
                    "2024-2-29",
                    "2024-02-29extra",
                    " ",
                ] {
                    assert_eq!(
                        SendMessageW(picker, WM_SETTEXT, 0, wide(invalid).as_ptr() as LPARAM),
                        0,
                        "{id}: {invalid}"
                    );
                    assert_eq!(settings::text(picker), "9999-12-31");
                }
                // Calendar/keyboard changes use native SYSTEMTIME; even a
                // non-midnight value still maps to a whole UTC day, not local time.
                native = SYSTEMTIME {
                    wYear: 2024,
                    wMonth: 2,
                    wDay: 29,
                    wHour: 18,
                    ..Default::default()
                };
                assert_ne!(
                    SendMessageW(
                        picker,
                        DTM_SETSYSTEMTIME,
                        GDT_VALID as WPARAM,
                        (&native as *const SYSTEMTIME) as LPARAM,
                    ),
                    0
                );
                assert_eq!(settings::text(picker), "2024-02-29");
                let filters = logs.filters().unwrap();
                assert_eq!(
                    if id == 2224 {
                        filters.start
                    } else {
                        filters.end
                    },
                    Some(1709164800 + if id == 2225 { 86399 } else { 0 })
                );
                assert_eq!(
                    if id == 2224 {
                        filters.end
                    } else {
                        filters.start
                    },
                    None
                );
                // Unchecking the native control also removes only this bound.
                assert_ne!(
                    SendMessageW(picker, DTM_SETSYSTEMTIME, GDT_NONE as WPARAM, 0),
                    0
                );
                assert_eq!(settings::text(picker), "");
                assert_eq!(logs.filters().unwrap(), Filters::default());
                assert_ne!(
                    SendMessageW(picker, WM_SETTEXT, 0, wide("2024-02-29").as_ptr() as LPARAM),
                    0
                );
                assert_ne!(
                    SendMessageW(picker, WM_SETTEXT, 0, wide("").as_ptr() as LPARAM),
                    0
                );
                assert_eq!(settings::text(picker), "");
                assert_eq!(logs.filters().unwrap(), Filters::default());
                assert_eq!(
                    SendMessageW(
                        picker,
                        DTM_GETSYSTEMTIME,
                        0,
                        (&mut native as *mut SYSTEMTIME) as LPARAM,
                    ),
                    GDT_NONE as LRESULT
                );
            }
            let category = logs.control(2226);
            assert!(logs.controls.contains(&(category, 24, 244, 156, 28)));
            assert_eq!(SendMessageW(category, CB_GETCOUNT, 0, 0), 3);
            for (index, label) in ["全部", "运行", "访问"].into_iter().enumerate() {
                assert_eq!(
                    SendMessageW(category, CB_SETCURSEL, index, 0),
                    index as LRESULT
                );
                assert_eq!(SendMessageW(category, CB_GETCURSEL, 0, 0), index as LRESULT);
                assert_eq!(settings::text(category), label);
                assert_eq!(logs.filters().unwrap().category, index);
            }
            SendMessageW(category, CB_SETCURSEL, 0, 0);
            logs.visible = true;
            logs.file_mode = true;
            logs.paused = true;
            logs.file_text = fixture();
            logs.before = Some(FileCursor {
                index: 1,
                identity: [1, 2, 3],
                offset: 10,
                observed_len: 100,
                partial_line: false,
            });
            logs.follow = logs.before.clone();
            for dpi in [96, 144, 192] {
                for (width, height) in [(744, 820), (784, 840), (1100, 1000)] {
                    logs.layout(dpi, scale(width, dpi), scale(height, dpi));
                    for (i, &(control, _, _, _, _)) in logs.controls.iter().enumerate() {
                        let mut rect = RECT::default();
                        assert_ne!(GetWindowRect(control, &mut rect), 0);
                        let id = GetDlgCtrlID(control);
                        if [2208, 2209].contains(&id) {
                            rect.bottom = rect.bottom.min(rect.top + scale(30, dpi));
                        }
                        assert!(
                            rect.bottom > rect.top && rect.right > rect.left,
                            "empty {id}"
                        );
                        assert!(
                            rect.bottom <= scale(height - 84, dpi),
                            "bottom clipping {id} at {dpi}"
                        );
                        assert!(
                            rect.right <= scale(width - 24, dpi),
                            "right clipping {id} at {dpi}"
                        );
                        for &(other, ..) in &logs.controls[i + 1..] {
                            let mut other_rect = RECT::default();
                            assert_ne!(GetWindowRect(other, &mut other_rect), 0);
                            let other_id = GetDlgCtrlID(other);
                            if [2208, 2209].contains(&other_id) {
                                other_rect.bottom =
                                    other_rect.bottom.min(other_rect.top + scale(30, dpi));
                            }
                            assert!(
                                !(rect.left < other_rect.right
                                    && rect.right > other_rect.left
                                    && rect.top < other_rect.bottom
                                    && rect.bottom > other_rect.top),
                                "overlap {id} / {other_id} at {dpi}"
                            );
                        }
                    }
                }
            }
            logs.render();
            assert_eq!(logs.rendered.lines().count(), 6);
            settings::set_text(logs.control(2224), "1970-01-01");
            settings::set_text(logs.control(2225), "1970-01-01");
            settings::set_text(logs.control(2227), "5xx");
            SendMessageW(logs.control(2226), CB_SETCURSEL, 2, 0);
            SendMessageW(logs.control(2208), CB_SETCURSEL, 4, 0);
            SendMessageW(logs.control(2228), BM_SETCHECK, 1, 0);
            settings::set_text(logs.control(2207), "metadata");
            logs.render();
            assert_eq!(logs.rendered.lines().count(), 1);
            assert!(logs.rendered.contains("\"sequence\":3"));
            let valid = logs.rendered.clone();
            for id in [2224, 2225] {
                for invalid in ["102", "2023-02-29"] {
                    assert_eq!(
                        SendMessageW(
                            logs.control(id),
                            WM_SETTEXT,
                            0,
                            wide(invalid).as_ptr() as LPARAM
                        ),
                        0
                    );
                    assert_eq!(settings::text(logs.control(id)), "1970-01-01");
                    logs.render();
                    assert_eq!(logs.rendered, valid);
                    assert!(!settings::text(logs.control(2220)).contains("筛选无效"));
                }
            }
            settings::set_text(logs.control(2227), "6xx");
            logs.render();
            assert_eq!(logs.rendered, valid);
            assert!(settings::text(logs.control(2220)).contains("筛选无效"));
            assert!(settings::text(logs.control(2229)).contains("保留原显示"));
            assert_eq!(logs.file_text, fixture());
            assert_eq!(logs.before.as_ref().unwrap().offset, 10);
            assert_eq!(logs.follow.as_ref().unwrap().offset, 10);
            assert!(logs.paused);

            // Switch to live, then advance the backend while paused. Both the
            // display text and metadata must remain from the frozen batch.
            for id in [2207, 2224, 2225, 2227] {
                settings::set_text(logs.control(id), "");
            }
            SendMessageW(logs.control(2226), CB_SETCURSEL, 0, 0);
            SendMessageW(logs.control(2208), CB_SETCURSEL, 0, 0);
            SendMessageW(logs.control(2228), BM_SETCHECK, 0, 0);
            logs.file_mode = false;
            logs.paused = false;
            logs.snapshot = Some(live_snapshot(live_fixture()));
            logs.render();
            assert_eq!(logs.rendered.lines().count(), 6);
            let frozen_text = logs.live_text.clone();
            logs.paused = true;
            let mut newer = live_fixture();
            for entry in &mut newer {
                entry.sequence += 10;
                entry.timestamp += 86400;
            }
            logs.snapshot = Some(live_snapshot(newer));
            settings::set_text(logs.control(2224), "1970-01-01");
            settings::set_text(logs.control(2225), "1970-01-01");
            settings::set_text(logs.control(2227), "5xx");
            SendMessageW(logs.control(2226), CB_SETCURSEL, 2, 0);
            SendMessageW(logs.control(2208), CB_SETCURSEL, 4, 0);
            SendMessageW(logs.control(2228), BM_SETCHECK, 1, 0);
            logs.render();
            assert_eq!(logs.rendered.lines().count(), 1);
            assert!(logs.rendered.contains("103 #4 Access"));
            assert_eq!(logs.live_text, frozen_text);
            assert_eq!(logs.live_entries[3].timestamp, 103);
            let valid = logs.rendered.clone();
            settings::set_text(logs.control(2224), "1970-01-02");
            logs.render();
            assert_eq!(logs.rendered, valid);
            assert!(settings::text(logs.control(2220)).contains("不得晚于"));
            assert_eq!(logs.displayed_cursor, 6);

            // Clear uses the latest typed sequence cursor, but does not alter
            // either file cursor. Resuming cannot resurrect old live entries.
            logs.clear_view();
            assert_eq!(logs.clear_cursor, 16);
            assert!(logs.live_entries.is_empty() && logs.live_text.is_empty());
            assert_eq!(logs.before.as_ref().unwrap().offset, 10);
            assert_eq!(logs.follow.as_ref().unwrap().offset, 10);
            for id in [2224, 2225, 2227] {
                settings::set_text(logs.control(id), "");
            }
            SendMessageW(logs.control(2226), CB_SETCURSEL, 0, 0);
            SendMessageW(logs.control(2208), CB_SETCURSEL, 0, 0);
            SendMessageW(logs.control(2228), BM_SETCHECK, 0, 0);
            logs.paused = false;
            logs.render();
            assert!(logs.rendered.is_empty());
            let mut next = live_fixture()[0].clone();
            next.sequence = 17;
            logs.snapshot = Some(live_snapshot(vec![next]));
            logs.render();
            assert!(logs.rendered.contains("#17 Runtime"));
            assert_eq!(logs.before.as_ref().unwrap().offset, 10);
            assert_eq!(logs.follow.as_ref().unwrap().offset, 10);
        }
    }

    #[test]
    fn live_rendering_is_bounded_and_untyped_messages_cannot_match_metadata() {
        let entry = live_fixture()[0].clone();
        let entries: Vec<_> = (1..=2000)
            .map(|sequence| LogEntry {
                sequence,
                ..entry.clone()
            })
            .collect();
        let text = debug_text(&entries[1000..]);
        let rendered = filter_live(&text, &entries, 0, &Filters::default()).unwrap();
        assert_eq!(rendered.lines().count(), MAX_LINES);
        assert!(rendered.len() <= LIMIT);
        let untyped = "日志尚未初始化";
        assert_eq!(
            filter_live(untyped, &[], 0, &Filters::default()).unwrap(),
            format!("{untyped}\r\n")
        );
        assert!(
            filter_live(
                untyped,
                &[],
                0,
                &Filters {
                    category: 1,
                    ..Filters::default()
                }
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn rendered_view_is_bounded() {
        let text = "a".repeat(1000) + "\n";
        assert!(
            filter(&text.repeat(1000), &Filters::default())
                .unwrap()
                .len()
                <= LIMIT
        );
        let many = "x\n".repeat(4000);
        assert_eq!(
            filter(&many, &Filters::default()).unwrap().lines().count(),
            MAX_LINES
        );
        assert_eq!(
            bounded_file_text("", &many, true).lines().count(),
            MAX_LINES
        );
    }
}
pub(super) const CONTENT: u32 = 2200;
const LIMIT: usize = 256 * 1024;
#[derive(Default)]
pub(super) struct Logs {
    controls: Vec<(HWND, i32, i32, i32, i32)>,
    content: HWND,
    snapshot: Option<LogViewSnapshot>,
    live_text: String,
    live_entries: Vec<LogEntry>,
    files: Vec<LogFile>,
    file_text: String,
    file_name: Option<String>,
    file_mode: bool,
    before: Option<FileCursor>,
    follow: Option<FileCursor>,
    read_kind: Option<u32>,
    last_follow: Option<Instant>,
    paused: bool,
    clear_cursor: u64,
    displayed_cursor: u64,
    gap: bool,
    pending: bool,
    initialized: bool,
    visible: bool,
    rendered: String,
    message: String,
    detail: String,
}
impl Logs {
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        parent: HWND,
        instance: HINSTANCE,
        class: &str,
        title: &str,
        style: u32,
        id: u32,
        row: (i32, i32, i32, i32),
    ) -> Result<HWND, String> {
        // SAFETY: Live parent/module; strings are copied during creation; id is a child identifier.
        let control = unsafe {
            CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(title).as_ptr(),
                WS_CHILD | style,
                0,
                0,
                0,
                0,
                parent,
                id as usize as HMENU,
                instance,
                null(),
            )
        };
        if control.is_null() {
            return Err(last_error("创建日志控件"));
        }
        self.controls.push((control, row.0, row.1, row.2, row.3));
        Ok(control)
    }
    pub fn create(&mut self, parent: HWND, instance: HINSTANCE) -> Result<(), String> {
        self.add(
            parent,
            instance,
            "STATIC",
            "等待后台日志状态；未初始化时请先正常保存配置。",
            0,
            2220,
            (24, 64, 700, 40),
        )?;
        let button = WS_TABSTOP;
        for (title, id, x, w) in [
            ("实时视图", 2201, 24, 100),
            ("暂停显示", 2202, 132, 100),
            ("文件视图", 2203, 240, 100),
            ("清空视图", 2204, 348, 100),
            ("↻ 刷新文件列表", 2205, 456, 132),
            ("读取尾部", 2206, 596, 124),
        ] {
            self.add(
                parent,
                instance,
                "BUTTON",
                title,
                button,
                id,
                (x, 108, w, 30),
            )?;
        }
        self.add(
            parent,
            instance,
            "STATIC",
            "搜索 / 级别 / 文件",
            0,
            0,
            (24, 150, 160, 26),
        )?;
        self.add(
            parent,
            instance,
            "EDIT",
            "",
            WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            2207,
            (184, 146, 210, 30),
        )?;
        for (id, x, w, items) in [
            (
                2208,
                404,
                104,
                vec!["全部级别", "trace", "debug", "info", "warn", "error"],
            ),
            (2209, 518, 202, vec!["尚未加载文件列表"]),
        ] {
            let combo = self.add(
                parent,
                instance,
                "COMBOBOX",
                "",
                WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32,
                id,
                (x, 146, w, 180),
            )?;
            // SAFETY: Live COMBOBOX copies each string; selection index exists.
            unsafe {
                for item in items {
                    SendMessageW(combo, CB_ADDSTRING, 0, wide(item).as_ptr() as isize);
                }
                SendMessageW(combo, CB_SETCURSEL, 0, 0);
            }
        }
        self.add(
            parent,
            instance,
            "STATIC",
            "UTC 日期：开始（左）/ 结束（右），含首尾整日；取消勾选不限",
            0,
            0,
            (24, 182, 696, 22),
        )?;
        for (id, x, label) in [(2224, 24, "开始（含）"), (2225, 372, "结束（含）")] {
            let picker = date_picker::create(parent, id, label)?;
            self.controls.push((picker, x, 208, 336, 28));
        }
        let category = choices::create(parent, 2226, &["全部", "运行", "访问"])?;
        self.controls.push((category, 24, 244, 156, 28));
        self.add(
            parent,
            instance,
            "STATIC",
            "状态码 / 2xx / 400-499",
            0,
            0,
            (192, 248, 208, 24),
        )?;
        let status = self.add(
            parent,
            instance,
            "EDIT",
            "",
            WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            2227,
            (404, 244, 128, 28),
        )?;
        // SAFETY: Live EDIT; no retained pointers.
        unsafe {
            SendMessageW(status, 0x00c5, 32, 0);
        }
        self.add(
            parent,
            instance,
            "BUTTON",
            "仅错误（AND）",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            2228,
            (544, 244, 176, 28),
        )?;
        self.add(parent, instance, "STATIC", "", 0, 2229, (24, 280, 696, 40))?;
        self.content = self.add(parent, instance, "EDIT",
            "等待后台快照。只读内容支持选中及 Ctrl+C。\r\n文件视图每秒异步跟随追加，支持向前分页；暂停时不请求追加。",
            WS_TABSTOP | WS_BORDER | WS_VSCROLL | WS_HSCROLL |
                (ES_READONLY | ES_MULTILINE | ES_AUTOVSCROLL | ES_AUTOHSCROLL) as u32,
            CONTENT, (24, 324, 696, 96))?;
        // SAFETY: Live EDIT; fixed input limit, no retained pointers.
        unsafe {
            SendMessageW(self.content, 0x00c5, LIMIT, 0);
        }
        for (title, id, x, w) in [
            ("删除历史日志…", 2210, 24, 180),
            ("清空全部应用日志…", 2211, 216, 220),
            ("← 向前一页", 2222, 448, 124),
            ("打开日志目录", 2223, 584, 136),
        ] {
            self.add(
                parent,
                instance,
                "BUTTON",
                title,
                button,
                id,
                (x, 432, w, 30),
            )?;
        }
        self.add(
            parent,
            instance,
            "STATIC",
            "暂停不停止记录；向前分页会暂停跟随。打开目录由后台执行，不阻塞界面。",
            0,
            0,
            (24, 468, 696, 30),
        )?;
        self.add(
            parent,
            instance,
            "STATIC",
            "日志设置（暂放本页）：应用仅即时生效，正常保存设置后才持久化",
            0,
            0,
            (24, 504, 696, 26),
        )?;
        for (title, id, x) in [("文件日志", 2212, 24), ("访问日志", 2213, 180)] {
            self.add(
                parent,
                instance,
                "BUTTON",
                title,
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                id,
                (x, 538, 144, 28),
            )?;
        }
        self.add(
            parent,
            instance,
            "STATIC",
            "级别 / 单文件 MiB / 文件数（含当前）/ 保留天数 / 缓存条数",
            0,
            0,
            (24, 578, 696, 26),
        )?;
        for (id, title, x, w) in [
            (2214, "info", 24, 104),
            (2215, "5", 140, 104),
            (2216, "3", 256, 104),
            (2217, "7", 372, 104),
            (2218, "1000", 488, 104),
        ] {
            self.add(
                parent,
                instance,
                "EDIT",
                title,
                WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
                id,
                (x, 612, w, 30),
            )?;
        }
        self.add(
            parent,
            instance,
            "BUTTON",
            "应用日志设置",
            button,
            2219,
            (24, 656, 180, 30),
        )?;
        self.add(
            parent,
            instance,
            "EDIT",
            "目录、占用、落盘错误、丢弃及缓存缺口：等待后台状态。",
            WS_TABSTOP
                | WS_BORDER
                | WS_VSCROLL
                | (ES_READONLY | ES_MULTILINE | ES_AUTOVSCROLL) as u32,
            2221,
            (24, 698, 696, 46),
        )?;

        self.enable();
        Ok(())
    }
    fn control(&self, id: u32) -> HWND {
        self.controls
            .iter()
            .find(|(h, ..)| {
                // SAFETY: All handles are live child controls on this thread.
                unsafe { GetDlgCtrlID(*h) == id as i32 }
            })
            .map_or(null_mut(), |(h, ..)| *h)
    }
    pub fn populate_options(&mut self, options: &LoggingConfig) {
        for (id, value) in [
            (2214, format!("{:?}", options.level).to_lowercase()),
            (2215, (options.max_file_bytes / (1024 * 1024)).to_string()),
            (2216, options.retained_files.to_string()),
            (2217, options.retention_days.to_string()),
            (2218, options.recent_entries.to_string()),
        ] {
            settings::set_text(self.control(id), &value);
        }
        // SAFETY: Live checkbox controls, no pointers retained.
        unsafe {
            SendMessageW(
                self.control(2212),
                BM_SETCHECK,
                usize::from(options.file_enabled),
                0,
            );
            SendMessageW(
                self.control(2213),
                BM_SETCHECK,
                usize::from(options.access_log),
                0,
            );
        }
    }
    fn draft(&self) -> Result<LoggingConfig, String> {
        let number = |id| {
            settings::text(self.control(id))
                .trim()
                .parse::<u32>()
                .map_err(|_| "日志设置数值无效。".to_string())
        };
        let level = match settings::text(self.control(2214))
            .trim()
            .to_lowercase()
            .as_str()
        {
            "trace" => LogLevel::Trace,
            "debug" => LogLevel::Debug,
            "info" => LogLevel::Info,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            "off" => LogLevel::Off,
            _ => return Err("日志级别须为 trace/debug/info/warn/error/off。".into()),
        };
        // SAFETY: Live checkboxes; messages have no pointer arguments.
        let (file_enabled, access_log) = unsafe {
            (
                SendMessageW(self.control(2212), BM_GETCHECK, 0, 0) != 0,
                SendMessageW(self.control(2213), BM_GETCHECK, 0, 0) != 0,
            )
        };
        Ok(LoggingConfig {
            level,
            file_enabled,
            access_log,
            max_file_bytes: u64::from(number(2215)?) * 1024 * 1024,
            retained_files: number(2216)?,
            retention_days: number(2217)?,
            recent_entries: number(2218)?,
        })
    }
    fn enable(&self) {
        for &(control, ..) in &self.controls {
            // SAFETY: Owner-thread child controls; no modal UI or State access from notifications.
            unsafe {
                let id = GetDlgCtrlID(control);
                if (2201..=2219).contains(&id) || id == 2222 || id == 2223 {
                    let enabled = match id {
                        2201 | 2202 | 2204 | 2207 | 2208 => true,
                        2209 => !self.pending && !self.files.is_empty(),
                        2206 => !self.pending && !self.files.is_empty() && self.initialized,
                        2222 => !self.pending && self.file_mode && self.before.is_some(),
                        2223 => self.initialized,
                        _ => !self.pending && self.initialized,
                    };
                    settings::EnableWindow(control, i32::from(enabled));
                }
            }
        }
    }
    fn send(&mut self, controller: &Controller, command: Command) {
        match controller.send(command) {
            Ok(()) => {
                self.pending = true;
                self.message = "日志后台操作中……".into();
            }
            Err(error) => self.message = error,
        }
        self.enable();
    }
    fn selected_file(&self) -> Option<String> {
        self.files
            .get(settings::selected(self.control(2209)))
            .map(|f| f.name.clone())
    }
    // IDs 2224–2228 are sampled by the existing UI timer, so no parent dispatch
    // expansion or nested message-loop callbacks are required.
    fn filters(&self) -> Result<Filters, String> {
        let start = utc_bound(&settings::text(self.control(2224)), false)?;
        let end = utc_bound(&settings::text(self.control(2225)), true)?;
        let status = status_range(&settings::text(self.control(2227)))?;
        // SAFETY: Live checkbox owned by this page; message contains no pointers.
        let errors_only = unsafe { SendMessageW(self.control(2228), BM_GETCHECK, 0, 0) != 0 };
        Ok(Filters {
            search: settings::text(self.control(2207)).to_lowercase(),
            level: settings::selected(self.control(2208)),
            start,
            end,
            status,
            errors_only,
            category: settings::selected(self.control(2226)),
        })
    }
    fn clear_view(&mut self) {
        self.clear_cursor = self.snapshot.as_ref().map_or(0, |s| s.cursor);
        self.file_text.clear();
        self.live_text.clear();
        self.live_entries.clear();
        self.rendered.clear();
        settings::set_text(self.content, "");
        self.message = "已清空视图，未删除文件".into();
    }
    fn render(&mut self) {
        if !self.visible {
            return;
        }
        if !self.file_mode && !self.paused {
            self.live_text = self
                .snapshot
                .as_ref()
                .map_or_else(String::new, |s| bounded_file_text("", &s.text, true));
            self.live_entries = self.snapshot.as_ref().map_or_else(Vec::new, |s| {
                s.entries[s.entries.len().saturating_sub(MAX_LINES)..].to_vec()
            });
        }
        let result = self.filters().and_then(|filters| {
            if self.file_mode {
                filter(&self.file_text, &filters)
            } else {
                filter_live(
                    &self.live_text,
                    &self.live_entries,
                    self.clear_cursor,
                    &filters,
                )
            }
        });
        let error = result
            .as_ref()
            .err()
            .map(|e| format!("筛选无效：{e}；保留原显示"));
        let hint = error.as_deref().unwrap_or("所有条件 AND；仅错误 = error 级别、错误码或 HTTP 4xx/5xx。最多显示 1000 行 / 256 KiB；文件筛选仅作用于已加载窗口。");
        if settings::text(self.control(2229)) != hint {
            settings::set_text(self.control(2229), hint);
        }
        if let Ok(filtered) = result
            && filtered != self.rendered
        {
            // SAFETY: Live readonly EDIT. Preserve selection and vertical viewport; never force follow.
            unsafe {
                let mut start = 0u32;
                let mut end = 0u32;
                SendMessageW(
                    self.content,
                    0xB0,
                    (&mut start as *mut u32) as usize,
                    (&mut end as *mut u32) as isize,
                );
                let line = SendMessageW(self.content, 0xCE, 0, 0);
                settings::set_text(self.content, &filtered);
                SendMessageW(self.content, 0xB1, start as usize, end as isize);
                let current = SendMessageW(self.content, 0xCE, 0, 0);
                SendMessageW(self.content, 0xB6, 0, line - current);
            }
            self.rendered = filtered;
        }
        let snapshot = self.snapshot.as_ref();
        if !self.paused && !self.file_mode && error.is_none() {
            self.displayed_cursor = snapshot.map_or(0, |s| s.cursor);
        }
        let status = format!(
            "{}；{}；{}",
            if self.file_mode {
                if self.paused {
                    "文件分页／跟随已暂停"
                } else {
                    "文件每秒自动跟随（不强制滚动）"
                }
            } else if self.paused {
                "实时已暂停显示（后台继续记录）"
            } else {
                "实时视图（保留选区，不自动滚动）"
            },
            if self.gap {
                "缓存缺口"
            } else {
                "有界缓存"
            },
            error.as_deref().unwrap_or(&self.message)
        );
        if settings::text(self.control(2220)) != status {
            settings::set_text(self.control(2220), &status);
        }
        let detail = snapshot.map_or_else(|| "等待日志快照".into(), |s| format!(
            "目录：{}；文件列表 {} 个 / {} bytes\r\n丢弃 {}；磁盘错误 {} ({:?})；已落盘 {:?} / cursor {}；停止 {}；{}",
            s.directory.as_ref().map_or_else(|| "未初始化".into(), |p| p.display().to_string()),
            self.files.len(), self.files.iter().map(|f| f.bytes).sum::<u64>(), s.status.dropped,
            s.status.disk_errors, s.status.last_disk_error, s.status.last_persisted_sequence,
            s.cursor, s.status.worker_stopped, self.detail));
        if settings::text(self.control(2221)) != detail {
            settings::set_text(self.control(2221), &detail);
        }
    }
    pub fn show(&mut self, visible: bool) {
        self.visible = visible;
        for &(control, ..) in &self.controls {
            // SAFETY: Live owned children; hiding also removes their keyboard navigation.
            unsafe {
                ShowWindow(control, if visible { SW_SHOW } else { SW_HIDE });
            }
        }
        self.render();
    }
    fn response(&mut self, response: LogResponse) -> Option<LoggingConfig> {
        self.pending = false;
        match response {
            LogResponse::Files(files) => {
                let selected = self.selected_file();
                self.files = files;
                // SAFETY: Live COMBOBOX; string copies complete synchronously.
                unsafe {
                    let combo = self.control(2209);
                    SendMessageW(combo, CB_RESETCONTENT, 0, 0);
                    for file in &self.files {
                        SendMessageW(combo, CB_ADDSTRING, 0, wide(&file.name).as_ptr() as isize);
                    }
                    let index = self
                        .files
                        .iter()
                        .position(|f| Some(&f.name) == selected.as_ref())
                        .unwrap_or(0);
                    SendMessageW(combo, CB_SETCURSEL, index, 0);
                }
                self.message = format!("已刷新 {} 个文件；选中文件后读取尾部", self.files.len());
            }
            LogResponse::Read { name, response } => {
                if self.file_name.as_ref() == Some(&name) {
                    match response {
                        ReadResponse::Page(page) => {
                            let kind = self.read_kind.take().unwrap_or(2206);
                            if kind == 2222 {
                                self.file_text =
                                    bounded_file_text(&page.text, &self.file_text, false);
                                self.before = page.before;
                            } else if kind == 2224 {
                                self.file_text =
                                    bounded_file_text(&self.file_text, &page.text, true);
                                self.follow = Some(page.follow);
                            } else {
                                self.file_text = bounded_file_text("", &page.text, true);
                                self.before = page.before;
                                self.follow = Some(page.follow);
                            }
                            self.message = format!(
                                "{name} / {} bytes；更多 {}；片段裁剪 {}；UTF-8 损失 {}",
                                page.file_bytes, page.has_more, page.truncated, page.utf8_loss
                            );
                        }
                        ReadResponse::Changed(change) => self.file_changed(format!(
                            "文件身份／长度变化：{change:?}；旧游标已清除，请重新读取尾部"
                        )),
                        ReadResponse::Missing => {
                            self.file_changed("文件已删除／缺失；旧游标已清除，请刷新列表".into())
                        }
                    }
                }
            }
            LogResponse::FileChanged { name, change } => {
                if self.file_name.as_ref() == Some(&name) {
                    self.file_changed(format!(
                        "{name} 已变化：{change:?}；停止跟随，请重新读取尾部"
                    ));
                }
            }
            LogResponse::FileMissing { name } => {
                if self.file_name.as_ref() == Some(&name) {
                    self.file_changed(format!("{name} 已缺失；停止跟随，请刷新列表"));
                }
            }
            LogResponse::Tail { name, tail } => {
                if self.file_name.as_ref() == Some(&name) {
                    self.file_text = bounded_file_text("", &tail.text, true);
                    self.message = format!(
                        "{} / {} bytes；尾部截断 {}；未自动跟随",
                        name, tail.file_bytes, tail.truncated
                    );
                }
            }
            LogResponse::Cleared { all, report } => {
                self.file_text.clear();
                self.file_name = None;
                self.before = None;
                self.follow = None;
                self.message = format!(
                    "{}：已删除 {} 文件 / {} bytes；失败 {} 项；允许新日志继续产生",
                    if all {
                        "全部应用日志清理"
                    } else {
                        "历史清理"
                    },
                    report.deleted_files,
                    report.deleted_bytes,
                    report.errors.len()
                );
                self.detail = report
                    .errors
                    .iter()
                    .map(|f| format!("{}: {:?}", f.name, f.kind))
                    .collect::<Vec<_>>()
                    .join("；");
                self.files.clear();
                // SAFETY: Live COMBOBOX; deletion invalidates all file selections.
                unsafe {
                    SendMessageW(self.control(2209), CB_RESETCONTENT, 0, 0);
                }
            }
            LogResponse::OptionsApplied { logging, persisted } => {
                self.message = if persisted {
                    "日志设置已持久化"
                } else {
                    "日志设置已应用，未持久化；请到设置页正常保存"
                }
                .into();
                self.enable();
                return Some(logging);
            }
            LogResponse::Error(error) => {
                if self.read_kind.take().is_some() {
                    self.paused = true;
                    settings::set_text(self.control(2202), "恢复显示");
                }
                self.message = format!("日志操作失败：{error}；读取跟随已暂停，可手动重试");
            }
            LogResponse::ClearPreview { .. } => {}
        }
        self.enable();
        None
    }
    pub fn handle(&mut self, controller: &Controller, id: u32) {
        if self.pending && ![2201, 2202, 2204, 2207, 2208].contains(&id) {
            return;
        }
        match id {
            2201 => {
                self.file_mode = false;
                self.paused = false;
                settings::set_text(self.control(2202), "暂停显示");
            }
            2202 => {
                self.paused = !self.paused;
                settings::set_text(
                    self.control(2202),
                    if self.paused {
                        "恢复显示"
                    } else {
                        "暂停显示"
                    },
                );
            }
            2204 => self.clear_view(),
            2203 | 2205 => self.send(controller, Command::LogListFiles),
            2206 | 2209 => {
                if let Some(name) = self.selected_file() {
                    self.file_mode = true;
                    self.file_name = Some(name.clone());
                    self.file_text.clear();
                    self.before = None;
                    self.follow = None;
                    self.paused = false;
                    self.read_kind = Some(2206);
                    self.last_follow = Some(Instant::now());
                    settings::set_text(self.control(2202), "暂停显示");
                    self.send(
                        controller,
                        Command::LogReadPage {
                            name,
                            cursor: PageCursor::Tail,
                        },
                    );
                }
            }
            2222 => {
                if let (Some(name), Some(cursor)) = (self.file_name.clone(), self.before.clone()) {
                    self.paused = true;
                    settings::set_text(self.control(2202), "恢复显示");
                    self.read_kind = Some(2222);
                    self.send(
                        controller,
                        Command::LogReadPage {
                            name,
                            cursor: PageCursor::Before(cursor),
                        },
                    );
                }
            }
            2223 => {
                if let Err(error) = controller.send(Command::OpenLogDirectory) {
                    self.message = error;
                }
            }
            2210 => self.send(controller, Command::LogClearHistory { confirmed: false }),
            2211 => self.send(controller, Command::LogClearAll { confirmed: false }),
            2219 => match self.draft() {
                Ok(logging) => self.send(controller, Command::LogApplyOptions { logging }),
                Err(error) => self.message = error,
            },
            _ => {}
        }
        self.render();
    }
    fn file_changed(&mut self, message: String) {
        self.before = None;
        self.follow = None;
        self.read_kind = None;
        self.paused = true;
        settings::set_text(self.control(2202), "恢复显示");
        self.message = message;
    }
    fn tick(&mut self, controller: &Controller) {
        if !self.visible
            || !self.file_mode
            || self.paused
            || self.pending
            || self
                .last_follow
                .is_some_and(|last| last.elapsed().as_secs() < 1)
        {
            return;
        }
        if let (Some(name), Some(cursor)) = (self.file_name.clone(), self.follow.clone()) {
            self.last_follow = Some(Instant::now());
            self.read_kind = Some(2224);
            self.send(controller, Command::LogReadFollow { name, cursor });
        }
    }
    pub fn set_font(&self, font: HFONT) {
        for &(control, ..) in &self.controls {
            // SAFETY: State owns the live font until controls are destroyed or updated.
            unsafe {
                SendMessageW(control, WM_SETFONT, font as usize, 0);
            }
        }
    }
    pub fn layout(&self, dpi: u32, width: i32, height: i32) {
        let extra = (height - scale(848, dpi)).max(-scale(32, dpi));
        for &(control, x, y, w, h) in &self.controls {
            let wide = w == 696 || w == 700;
            let y = scale(y, dpi) + if y >= 432 { extra } else { 0 };
            let h = scale(h, dpi) + if control == self.content { extra } else { 0 };
            // SAFETY: Live owned children, no parent resizing or disk operations.
            unsafe {
                MoveWindow(
                    control,
                    scale(x, dpi),
                    y,
                    if wide {
                        (width - scale(48, dpi)).max(1)
                    } else {
                        scale(w, dpi)
                    },
                    h,
                    1,
                );
            }
        }
    }
}
