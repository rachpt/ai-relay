use super::filters::{date_boundary, date_range, parse_date};
use super::table::{alias_label, group_key, number, summary, timestamp};
use super::*;
use crate::history::{
    CredentialAlias, CredentialId, GroupBy, QueryReport, Summary, TimeZone, TrendGranularity,
};
use chrono::{Local, NaiveDate};
use std::path::PathBuf;

#[test]
fn idle_requires_no_query_file_chooser_or_confirmation() {
    let mut history = History::default();
    assert!(!history.busy());
    history.pending = Some(Pending {
        scope: None,
        generation: 0,
        key: "query".into(),
    });
    assert!(history.busy());
    history.pending = None;
    let (_sender, receiver) = std::sync::mpsc::channel();
    history.file = Some(receiver);
    assert!(history.busy());
    history.file = None;
    history.modal = true;
    assert!(history.busy());
    history.modal = false;
    assert!(!history.busy());
}
#[test]
fn native_time_zone_defaults_and_reset_follow_system_local() {
    assert_eq!(TimeZone::default(), TimeZone::Local);
    assert_eq!(Query::default().time_zone, TimeZone::Local);
    let mut native = super::super::tests::NativeState::new();
    native.state.show_page(1);
    assert_eq!(settings::selected(native.state.history.control(ZONE)), 1);
    assert_eq!(
        native.state.history.draft().unwrap().time_zone,
        TimeZone::Local
    );
    assert!(native.state.history.pending.is_none());

    native.command(ZONE, Some(0));
    assert_eq!(
        native.state.history.draft().unwrap().time_zone,
        TimeZone::Utc
    );
    assert!(native.state.history.pending.is_none());
    native.command(RESET, None);
    assert_eq!(settings::selected(native.state.history.control(ZONE)), 1);
    assert_eq!(
        native.state.history.draft().unwrap().time_zone,
        TimeZone::Local
    );
    assert!(native.state.history.pending.is_none());

    let ms = db::now_ms();
    let expected = chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S%.3f %:z")
        .to_string();
    assert_eq!(timestamp(ms, TimeZone::default()), expected);
}
#[test]
fn utc_dates() {
    assert_eq!(parse_date("1970-01-01", TimeZone::Utc).unwrap(), Some(0));
    assert!(parse_date("2025-02-29", TimeZone::Utc).is_err());
    for s in ["2024-02-29", "2026-10-17", "2000-01-01"] {
        assert!(
            timestamp(
                parse_date(s, TimeZone::Utc).unwrap().unwrap(),
                TimeZone::Utc
            )
            .starts_with(s)
        );
    }
}
#[test]
fn rejects_results_after_scope_changes_including_return_to_same_directory() {
    let a = PathBuf::from("scope-a");
    let b = PathBuf::from("scope-b");
    let pending = Pending {
        scope: Some(a.clone()),
        generation: 1,
        key: String::new(),
    };
    assert!(pending.matches_scope(&Some(a.clone()), 1, Some(&a)));
    assert!(!pending.matches_scope(&Some(b.clone()), 2, Some(&a)));
    assert!(!pending.matches_scope(&Some(a.clone()), 3, Some(&a)));
    assert!(!pending.matches_scope(&Some(a.clone()), 1, Some(&b)));
    assert!(pending.matches_scope(&Some(a.clone()), 1, None));
    assert!(!pending.matches_scope(&Some(a), 3, None));
}
#[test]
fn chart_and_results_fit_minimum_client_at_common_dpi() {
    for dpi in [96, 120, 144, 168, 192] {
        for extra in [0, 20, 60, 160] {
            let status_height = scale(status_layout::MIN_HEIGHT + extra, dpi);
            let bottom =
                viewport::content_bottom(dpi, scale(820, dpi) + status_height - scale(34, dpi));
            for advanced in [false, true] {
                let geometry = status_layout::Geometry::new(dpi, advanced, status_height);
                assert_eq!(
                    geometry.chart_top,
                    geometry.status_top + status_height + scale(8, dpi)
                );
                assert!(bottom - geometry.table_top >= scale(80, dpi));
            }
        }
    }
}
#[derive(Clone)]
struct Eastern2024;
impl chrono::TimeZone for Eastern2024 {
    type Offset = chrono::FixedOffset;
    fn from_offset(_: &Self::Offset) -> Self {
        Self
    }
    fn offset_from_local_date(&self, value: &NaiveDate) -> chrono::LocalResult<Self::Offset> {
        self.offset_from_local_datetime(&value.and_hms_opt(0, 0, 0).unwrap())
    }
    fn offset_from_local_datetime(
        &self,
        value: &chrono::NaiveDateTime,
    ) -> chrono::LocalResult<Self::Offset> {
        let standard = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        let daylight = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
        let spring = NaiveDate::from_ymd_opt(2024, 3, 10)
            .unwrap()
            .and_hms_opt(2, 0, 0)
            .unwrap();
        let fall = NaiveDate::from_ymd_opt(2024, 11, 3)
            .unwrap()
            .and_hms_opt(1, 0, 0)
            .unwrap();
        if *value >= spring && *value < spring + chrono::Duration::hours(1) {
            chrono::LocalResult::None
        } else if *value >= fall && *value < fall + chrono::Duration::hours(1) {
            chrono::LocalResult::Ambiguous(daylight, standard)
        } else if *value >= spring + chrono::Duration::hours(1) && *value < fall {
            chrono::LocalResult::Single(daylight)
        } else {
            chrono::LocalResult::Single(standard)
        }
    }
    fn offset_from_utc_date(&self, value: &NaiveDate) -> Self::Offset {
        self.offset_from_utc_datetime(&value.and_hms_opt(0, 0, 0).unwrap())
    }
    fn offset_from_utc_datetime(&self, value: &chrono::NaiveDateTime) -> Self::Offset {
        let ms = value.and_utc().timestamp_millis();
        let spring = chrono::DateTime::parse_from_rfc3339("2024-03-10T07:00:00Z")
            .unwrap()
            .timestamp_millis();
        let fall = chrono::DateTime::parse_from_rfc3339("2024-11-03T06:00:00Z")
            .unwrap()
            .timestamp_millis();
        chrono::FixedOffset::west_opt(if (spring..fall).contains(&ms) {
            4 * 3600
        } else {
            5 * 3600
        })
        .unwrap()
    }
}
#[test]
fn calendar_inputs_use_each_dates_dst_rules_not_current_offset() {
    let date = |month, day| NaiveDate::from_ymd_opt(2024, month, day).unwrap();
    for (month, day, hours) in [(3, 10, 23), (11, 3, 25)] {
        let start = date_boundary(date(month, day), &Eastern2024).unwrap();
        let end = date_boundary(date(month, day + 1), &Eastern2024).unwrap();
        assert_eq!(end - start, hours * 3_600_000);
    }
    let east = chrono::FixedOffset::east_opt(14 * 3600).unwrap();
    assert_eq!(
        date_boundary(date(3, 1), &east).unwrap(),
        chrono::DateTime::parse_from_rfc3339("2024-02-29T10:00:00Z")
            .unwrap()
            .timestamp_millis()
    );
    assert_eq!(parse_date("1970-01-01", TimeZone::Utc).unwrap(), Some(0));
    assert!(parse_date("2024-2-01", TimeZone::Utc).is_err());
    assert!(parse_date("2025-02-29", TimeZone::Local).is_err());
    let now = db::now_ms();
    let (start, end) = date_range(now, 1, TimeZone::Local).unwrap();
    assert!(start.unwrap() <= now && now < end.unwrap());
    assert_eq!(
        start.unwrap(),
        db::calendar_bucket_with_zone(now, TrendGranularity::Day, &Local)
            .unwrap()
            .bucket_start_ms
    );
}
#[test]
fn aliases_are_display_only_and_secret_like_labels_are_rejected() {
    let id = CredentialId::parse(&"ab".repeat(32)).unwrap();
    let report = QueryReport {
        aliases: vec![db::CredentialAliasEntry {
            credential: id.clone(),
            alias: CredentialAlias::new("研发客户端").unwrap(),
        }],
        summary: Summary::default(),
        groups: Vec::new(),
        details: Vec::new(),
        trend: Vec::new(),
        trend_truncated: false,
    };
    assert_eq!(alias_label(&report, Some(&id)), "研发客户端");
    let query = Query {
        time_zone: TimeZone::Utc,
        ..Query::default()
    };
    assert_eq!(group_key(Some("0"), &GroupBy::Day, &query), "1970-01-01");
    assert_eq!(
        group_key(Some("0"), &GroupBy::Hour, &query),
        "1970-01-01 00:00 +00:00"
    );
    assert_eq!(
        group_key(Some(id.as_str()), &GroupBy::Credential, &query),
        id.as_str()
    );
    assert_eq!(id.as_str(), "ab".repeat(32));
    assert!(CredentialAlias::new("sk-not-a-real-key").is_err());
    assert!(CredentialId::parse("研发客户端").is_err());
}
#[test]
fn native_query_results_preserve_hidden_views_and_cached_coordinates() {
    let mut native = super::super::tests::NativeState::new();
    native.state.show_page(1);
    let offset = native.scroll(SB_LINEDOWN);
    let directory = std::env::temp_dir().join("ai-relay-ui-query-scope");
    native.state.history.scope(directory.clone());
    for (session, selection, management, active) in [
        (false, 0, false, true),
        (false, 2, false, true),
        (false, 3, false, true),
        (true, 0, false, true),
        (false, 0, true, true),
        (false, 0, false, false),
    ] {
        native.command(VIEW, Some(0));
        native.command(TABLE_VIEW, Some(selection));
        if management != native.state.history.management {
            native.command(MANAGE, None);
        }
        native.command(VIEW, Some(usize::from(session)));
        let draft = native.state.history.control(2304);
        settings::set_text(draft, "127.0.0.2");
        if !active {
            native.state.show_page(0);
        }
        let table = native.state.history.control(TABLE);
        let before = native.rect(table);
        let query = Query::default();
        native.state.history.pending = Some(Pending {
            scope: Some(directory.clone()),
            generation: native.state.history.generation,
            key: format!("{query:?}"),
        });
        native
            .state
            .controller
            .history_events
            .publish(HistoryResponse::QueryResult {
                directory: directory.clone(),
                query,
                report: Box::new(QueryReport {
                    summary: Summary::default(),
                    groups: Vec::new(),
                    details: Vec::new(),
                    trend: Vec::new(),
                    trend_truncated: false,
                    aliases: Vec::new(),
                }),
                counters: db::Counters::default(),
            });
        native.state.history.poll(&native.state.controller);
        assert!(native.state.history.report.is_some());
        // Content population must not synchronously rearrange the page or reveal
        // hidden controls. The queued status change performs the final reflow.
        // SAFETY: Live test-owned child; inspect its own style because the
        // top-level fixture stays hidden during command/cache transitions.
        let visible = unsafe { GetWindowLongPtrW(table, GWL_STYLE) as u32 & WS_VISIBLE != 0 };
        assert_eq!(visible, active && !session && !management && selection <= 1);
        assert_eq!(native.rect(table).top, before.top);
        assert_eq!(settings::text(draft), "127.0.0.2");
        if active {
            assert_eq!(native.scroll(SB_ENDSCROLL), offset);
        }
        dispatch_status_layout(&native);
        let visible = unsafe { GetWindowLongPtrW(table, GWL_STYLE) as u32 & WS_VISIBLE != 0 };
        assert_eq!(visible, active && !session && !management && selection <= 1);
        assert_eq!(settings::text(draft), "127.0.0.2");
        native.state.show_page(0);
        native.state.show_page(1);
        native.command(VIEW, Some(0));
        if management {
            native.command(MANAGE, None);
        }
        native.command(TABLE_VIEW, Some(0));
        let status = native.rect(native.control(2325));
        assert_eq!(
            native.rect(table).top,
            status.bottom + scale(8 + 200 + 8 + 72, native.state.dpi)
        );
        assert_eq!(native.scroll(SB_ENDSCROLL), offset);
    }
}

pub(super) fn dispatch_status_layout(native: &super::super::tests::NativeState) -> usize {
    let mut count = 0;
    // SAFETY: Dispatch only the test-owned window's scalar status notifications;
    // no timers, background work, real input or other windows are pumped.
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(
            &mut message,
            native.hwnd,
            STATUS_CHANGED,
            STATUS_CHANGED,
            PM_REMOVE,
        ) != 0
        {
            assert!(count < 32, "status layout notification loop");
            DispatchMessageW(&message);
            count += 1;
        }
    }
    count
}

#[test]
fn unknown_not_zero() {
    assert!(summary(&Summary::default()).contains("总 未知"));
    assert_eq!(number(Some(0)), "0");
}
