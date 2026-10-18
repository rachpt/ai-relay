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
            DTM_GETRANGE, DTM_GETSYSTEMTIME, DTM_SETSYSTEMTIME, DTS_SHOWNONE, GDT_NONE, GDT_VALID,
            GDTR_MAX, GDTR_MIN,
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
