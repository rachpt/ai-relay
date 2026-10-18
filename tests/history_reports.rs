#![allow(dead_code)]
#[allow(unused_imports)]
#[path = "../src/history/mod.rs"]
mod history;
#[allow(unused_imports)]
#[path = "../src/usage/mod.rs"]
mod usage;
use chrono::{Datelike, FixedOffset, LocalResult, NaiveDate, NaiveDateTime};
use history::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "relay-history-reports-{}-{}-{}",
            std::process::id(),
            now_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn ms(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp_millis()
}
fn record(h: &HistoryHandle, time: i64) -> RequestRecord {
    RequestRecord {
        request_id: h.request_id(),
        started_ms: time,
        peer_ip: "127.0.0.1".parse().unwrap(),
        client: Some(h.credential("synthetic-client-for-reports")),
        upstream: Some(h.credential("synthetic-upstream-for-reports")),
        provider: Some(ProviderOrigin::new("https://example.invalid").unwrap()),
        requested_model: None,
        protocol: Protocol::Unknown,
        interface: Interface::Unknown,
        status: Some(200),
        outcome: Outcome::Success,
        latency_ms: 1,
        total_ms: 3,
        report: usage::UsageReport::default(),
    }
}
fn put(s: &HistoryService, r: RequestRecord) {
    assert_eq!(s.handle().try_record(r), TryRecord::Queued);
    s.barrier().unwrap();
}
// An injectable historical zone: US Eastern's 2024 spring gap/fall fold. No
// timezone database, environment mutation, fixed-current-offset or clock change.
#[derive(Clone)]
struct Eastern2024;
impl chrono::TimeZone for Eastern2024 {
    type Offset = FixedOffset;
    fn from_offset(_: &Self::Offset) -> Self {
        Self
    }
    fn offset_from_local_date(&self, value: &NaiveDate) -> LocalResult<Self::Offset> {
        self.offset_from_local_datetime(&value.and_hms_opt(0, 0, 0).unwrap())
    }
    fn offset_from_local_datetime(&self, value: &NaiveDateTime) -> LocalResult<Self::Offset> {
        let standard = FixedOffset::west_opt(5 * 3600).unwrap();
        let daylight = FixedOffset::west_opt(4 * 3600).unwrap();
        let spring = NaiveDate::from_ymd_opt(2024, 3, 10)
            .unwrap()
            .and_hms_opt(2, 0, 0)
            .unwrap();
        let fall = NaiveDate::from_ymd_opt(2024, 11, 3)
            .unwrap()
            .and_hms_opt(1, 0, 0)
            .unwrap();
        if *value >= spring && *value < spring + chrono::Duration::hours(1) {
            LocalResult::None
        } else if *value >= fall && *value < fall + chrono::Duration::hours(1) {
            LocalResult::Ambiguous(daylight, standard)
        } else if *value >= spring + chrono::Duration::hours(1) && *value < fall {
            LocalResult::Single(daylight)
        } else {
            LocalResult::Single(standard)
        }
    }
    fn offset_from_utc_date(&self, value: &NaiveDate) -> Self::Offset {
        self.offset_from_utc_datetime(&value.and_hms_opt(0, 0, 0).unwrap())
    }
    fn offset_from_utc_datetime(&self, value: &NaiveDateTime) -> Self::Offset {
        if value.and_utc().timestamp_millis() >= ms("2024-03-10T07:00:00Z")
            && value.and_utc().timestamp_millis() < ms("2024-11-03T06:00:00Z")
        {
            FixedOffset::west_opt(4 * 3600).unwrap()
        } else {
            FixedOffset::west_opt(5 * 3600).unwrap()
        }
    }
}
#[derive(Clone)]
struct GapAtMidnight;
impl chrono::TimeZone for GapAtMidnight {
    type Offset = FixedOffset;
    fn from_offset(_: &Self::Offset) -> Self {
        Self
    }
    fn offset_from_local_date(&self, value: &NaiveDate) -> LocalResult<Self::Offset> {
        self.offset_from_local_datetime(&value.and_hms_opt(0, 0, 0).unwrap())
    }
    fn offset_from_local_datetime(&self, value: &NaiveDateTime) -> LocalResult<Self::Offset> {
        let boundary = NaiveDate::from_ymd_opt(2024, 6, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        if *value < boundary {
            LocalResult::Single(FixedOffset::east_opt(0).unwrap())
        } else if *value < boundary + chrono::Duration::hours(1) {
            LocalResult::None
        } else {
            LocalResult::Single(FixedOffset::east_opt(3600).unwrap())
        }
    }
    fn offset_from_utc_date(&self, value: &NaiveDate) -> Self::Offset {
        self.offset_from_utc_datetime(&value.and_hms_opt(0, 0, 0).unwrap())
    }
    fn offset_from_utc_datetime(&self, value: &NaiveDateTime) -> Self::Offset {
        FixedOffset::east_opt(
            if value.and_utc().timestamp_millis() >= ms("2024-06-01T00:00:00Z") {
                3600
            } else {
                0
            },
        )
        .unwrap()
    }
}
#[test]
fn missing_midnight_and_timestamp_extremes_do_not_panic() {
    for grain in [TrendGranularity::Day, TrendGranularity::Month] {
        let bucket =
            calendar_bucket_with_zone(ms("2024-06-01T12:00:00Z"), grain, &GapAtMidnight).unwrap();
        assert_eq!(bucket.bucket_start_ms, ms("2024-06-01T00:00:00Z"));
        assert_eq!(bucket.utc_day, ms("2024-06-01T00:00:00Z") / 86_400_000);
    }
    let east = FixedOffset::east_opt(14 * 3600).unwrap();
    assert!(
        calendar_bucket_with_zone(
            chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp_millis(),
            TrendGranularity::Day,
            &east
        )
        .is_err()
    );
}
#[test]
fn dst_gap_fold_day_lengths_and_month_use_historical_rules() {
    let bucket =
        |time: &str, grain| calendar_bucket_with_zone(ms(time), grain, &Eastern2024).unwrap();
    let before = bucket("2024-03-10T06:30:00Z", TrendGranularity::Hour);
    let after = bucket("2024-03-10T07:30:00Z", TrendGranularity::Hour);
    assert_eq!(before.label, "2024-03-10 01:00 -05:00");
    assert_eq!(after.label, "2024-03-10 03:00 -04:00");
    assert_eq!(after.bucket_start_ms - before.bucket_start_ms, 3_600_000);
    let first = bucket("2024-11-03T05:30:00Z", TrendGranularity::Hour);
    let repeated = bucket("2024-11-03T06:30:00Z", TrendGranularity::Hour);
    assert_eq!(first.label, "2024-11-03 01:00 -04:00");
    assert_eq!(repeated.label, "2024-11-03 01:00 -05:00");
    assert_eq!(repeated.bucket_start_ms - first.bucket_start_ms, 3_600_000);
    assert_eq!(first.utc_day, repeated.utc_day);
    let spring = bucket("2024-03-10T12:00:00Z", TrendGranularity::Day);
    let spring_next = bucket("2024-03-11T12:00:00Z", TrendGranularity::Day);
    assert_eq!(
        spring_next.bucket_start_ms - spring.bucket_start_ms,
        23 * 3_600_000
    );
    let fall = bucket("2024-11-03T12:00:00Z", TrendGranularity::Day);
    let fall_next = bucket("2024-11-04T12:00:00Z", TrendGranularity::Day);
    assert_eq!(
        fall_next.bucket_start_ms - fall.bucket_start_ms,
        25 * 3_600_000
    );
    let month = bucket("2024-11-20T12:00:00Z", TrendGranularity::Month);
    assert_eq!(month.label, "2024-11");
    assert_eq!(month.bucket_start_ms, ms("2024-11-01T04:00:00Z"));
    assert_eq!(month.utc_day, ms("2024-11-01T00:00:00Z") / 86_400_000);
    let offset = FixedOffset::east_opt(14 * 3600).unwrap();
    let next_month =
        calendar_bucket_with_zone(ms("2024-02-29T11:00:00Z"), TrendGranularity::Month, &offset)
            .unwrap();
    assert_eq!(next_month.label, "2024-03");
    assert_eq!(next_month.bucket_start_ms, ms("2024-02-29T10:00:00Z"));
    assert!(calendar_bucket_with_zone(i64::MAX, TrendGranularity::Day, &offset).is_err());
}
#[test]
fn default_query_uses_system_local_calendar_without_changing_stored_time() {
    assert_eq!(TimeZone::default(), TimeZone::Local);
    let query = Query::default();
    assert_eq!(query.time_zone, TimeZone::Local);
    let t = Temp::new();
    let s = HistoryService::start(&t.0).unwrap();
    let started_ms = now_ms();
    put(&s, record(&s.handle(), started_ms));
    let report = s
        .query(Query {
            group_by: vec![GroupBy::Day, GroupBy::Hour, GroupBy::Month],
            ..query
        })
        .unwrap();
    assert_eq!(report.details[0].started_ms, started_ms);
    let buckets: Vec<_> = [
        TrendGranularity::Day,
        TrendGranularity::Hour,
        TrendGranularity::Month,
    ]
    .into_iter()
    .map(|grain| calendar_bucket_with_zone(started_ms, grain, &chrono::Local).unwrap())
    .collect();
    assert_eq!(report.trend[0].bucket_start_ms, buckets[0].bucket_start_ms);
    assert_eq!(report.trend[0].label, buckets[0].label);
    assert_eq!(
        report.groups[0].keys,
        buckets
            .iter()
            .map(|b| Some(b.label.clone()))
            .collect::<Vec<_>>()
    );
}
#[test]
fn month_requested_model_protocol_and_unknown_coverage_agree() {
    let t = Temp::new();
    let s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    let start = chrono::Utc::now()
        .date_naive()
        .with_day(1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    let mut a = record(&h, start - 1);
    a.requested_model = Some(Model::new("requested-one").unwrap());
    a.report.model = Some("response-other".into());
    a.protocol = Protocol::OpenAi;
    a.report.usage.input_tokens = Some(0);
    put(&s, a);
    put(&s, record(&h, start));
    let q = Query {
        group_by: vec![GroupBy::Month, GroupBy::RequestedModel, GroupBy::Protocol],
        trend_granularity: TrendGranularity::Month,
        time_zone: TimeZone::Utc,
        ..Query::default()
    };
    let report = s.query(q).unwrap();
    assert_eq!(report.summary.requests, 2);
    assert_eq!(report.summary.tokens.input_tokens, Some(0));
    assert_eq!(report.summary.tokens.total_tokens, None);
    assert_eq!(report.summary.coverage, [1, 0, 0, 0, 0, 0, 0]);
    assert_eq!(report.trend.len(), 2);
    assert_eq!(report.trend[1].bucket_start_ms, start);
    assert_eq!(report.trend[1].utc_day, start / 86_400_000);
    assert_eq!(report.trend[1].summary.unknown, 1);
    assert_eq!(report.groups[1].keys[1], None);
    assert_eq!(report.groups[1].keys[2].as_deref(), Some("unknown"));
    let hours = s
        .query(Query {
            trend_granularity: TrendGranularity::Hour,
            time_zone: TimeZone::Utc,
            ..Query::default()
        })
        .unwrap();
    assert_eq!(hours.trend.len(), 2);
    let local = s
        .query(Query {
            time_zone: TimeZone::Local,
            trend_granularity: TrendGranularity::Month,
            ..Query::default()
        })
        .unwrap();
    for point in local.trend {
        let expected = calendar_bucket_with_zone(
            point.bucket_start_ms,
            TrendGranularity::Month,
            &chrono::Local,
        )
        .unwrap();
        assert_eq!(point.label, expected.label);
        assert_eq!(point.bucket_start_ms, expected.bucket_start_ms);
    }
}
#[test]
fn detail_and_group_sorting_preserve_u64_unknown_and_stable_paging() {
    let t = Temp::new();
    let s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    let time = now_ms();
    for (i, token) in [
        None,
        Some(0),
        Some(9),
        Some(i64::MAX as u64 + 1),
        Some(u64::MAX),
    ]
    .into_iter()
    .enumerate()
    {
        let mut r = record(&h, time + i as i64);
        r.requested_model = Some(Model::new(&format!("model-{i}")).unwrap());
        r.report.usage = usage::TokenUsage {
            input_tokens: token,
            output_tokens: token,
            total_tokens: token,
            cached_input_tokens: token,
            reasoning_tokens: token,
            cache_creation_input_tokens: token,
            cache_read_input_tokens: token,
        };
        put(&s, r);
    }
    for field in [
        TokenField::Input,
        TokenField::Output,
        TokenField::Total,
        TokenField::CachedInput,
        TokenField::Reasoning,
        TokenField::CacheCreationInput,
        TokenField::CacheReadInput,
    ] {
        for descending in [false, true] {
            let q = Query {
                detail_sort: if descending {
                    DetailSort::TokensDesc(field)
                } else {
                    DetailSort::TokensAsc(field)
                },
                group_sort: if descending {
                    GroupSort::TokensDesc(field)
                } else {
                    GroupSort::TokensAsc(field)
                },
                group_by: vec![GroupBy::RequestedModel],
                ..Query::default()
            };
            let r = s.query(q.clone()).unwrap();
            let values: Vec<_> = r
                .details
                .iter()
                .map(|v| v.report.usage.total_tokens)
                .collect();
            let expected = if descending {
                vec![
                    Some(u64::MAX),
                    Some(i64::MAX as u64 + 1),
                    Some(9),
                    Some(0),
                    None,
                ]
            } else {
                vec![
                    Some(0),
                    Some(9),
                    Some(i64::MAX as u64 + 1),
                    Some(u64::MAX),
                    None,
                ]
            };
            assert_eq!(values, expected);
            assert_eq!(
                r.groups
                    .iter()
                    .map(|g| g.summary.tokens.total_tokens)
                    .collect::<Vec<_>>(),
                expected
            );
            let page = s
                .query(Query {
                    limit: 2,
                    offset: 2,
                    ..q
                })
                .unwrap();
            assert_eq!(page.details[0].request_id, r.details[2].request_id);
            assert_eq!(page.groups[0].keys, r.groups[2].keys);
        }
    }
    let old = s
        .query(Query {
            detail_sort: DetailSort::TimeAsc,
            ..Query::default()
        })
        .unwrap();
    let new = s.query(Query::default()).unwrap();
    assert_eq!(old.details[0].started_ms, time);
    assert_eq!(new.details[0].started_ms, time + 4);
    let mut duplicate_group = record(&h, time + 5);
    duplicate_group.requested_model = Some(Model::new("model-0").unwrap());
    put(&s, duplicate_group);
    for (sort, first) in [
        (GroupSort::CountDesc, "model-0"),
        (GroupSort::CountAsc, "model-1"),
        (GroupSort::TimeAsc, "model-0"),
        (GroupSort::TimeDesc, "model-0"),
        (GroupSort::KeysDesc, "model-4"),
    ] {
        let r = s
            .query(Query {
                group_by: vec![GroupBy::RequestedModel],
                group_sort: sort,
                ..Query::default()
            })
            .unwrap();
        assert_eq!(r.groups[0].keys[0].as_deref(), Some(first));
    }
}
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => row.push(std::mem::take(&mut cell)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            _ => cell.push(c),
        }
    }
    assert!(!quoted);
    assert!(row.is_empty());
    assert!(cell.is_empty());
    rows
}
#[test]
fn full_csv_streams_all_filtered_rows_with_privacy_coverage_and_no_overwrite() {
    let t = Temp::new();
    let s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    s.set_alias(
        h.credential("synthetic-client-for-reports"),
        CredentialAlias::new(" =SUM(1,2)").unwrap(),
    )
    .unwrap();
    s.set_alias(
        h.credential("synthetic-upstream-for-reports"),
        CredentialAlias::new("中文\"别名").unwrap(),
    )
    .unwrap();
    let start = now_ms();
    for i in 0..137 {
        let mut r = record(&h, start + i);
        r.requested_model = Some(Model::new(if i < 128 { "@formula" } else { "other" }).unwrap());
        if i == 1 {
            r.report.usage.input_tokens = Some(0);
        }
        if i >= 2 {
            r.report.usage = usage::TokenUsage {
                input_tokens: Some(i as u64),
                output_tokens: Some(2),
                total_tokens: Some(i as u64 + 2),
                cached_input_tokens: Some(3),
                reasoning_tokens: Some(4),
                cache_creation_input_tokens: Some(5),
                cache_read_input_tokens: Some(6),
            };
            r.report.format = usage::ObservationFormat::Sse;
            r.report.truncated = true;
            r.report.parse_error = true;
            r.report.limit_exceeded = true;
            r.report.provider_error = true;
            r.report.arithmetic_overflow = true;
        }
        put(&s, r);
    }
    let path = t.0.join("full.csv");
    let query = Query {
        requested_model: Some(Model::new("@formula").unwrap()),
        limit: 1,
        offset: 1000,
        group_by: vec![GroupBy::Protocol],
        detail_sort: DetailSort::TimeAsc,
        ..Query::default()
    };
    let export = s.export_all(query.clone(), &path).unwrap();
    assert_eq!(export.rows, 128);
    assert_eq!(export.path, path);
    assert_eq!(export.bytes, fs::metadata(&path).unwrap().len());
    let text = fs::read_to_string(&path).unwrap();
    for private in [
        "127.0.0.1",
        h.credential("synthetic-client-for-reports").as_str(),
        h.credential("synthetic-upstream-for-reports").as_str(),
    ] {
        assert!(!text.contains(private));
    }
    let rows = parse_csv(&text);
    assert_eq!(rows.len(), 129);
    let column = |name| rows[0].iter().position(|c| c == name).unwrap();
    assert!(rows.iter().all(|row| row.len() == rows[0].len()));
    assert_eq!(rows[1][column("started_utc_ms")], start.to_string());
    assert_eq!(rows[1][column("requested_model")], "'@formula");
    assert_eq!(rows[1][column("client_alias")], "' =SUM(1,2)");
    assert_eq!(rows[1][column("upstream_alias")], "中文\"别名");
    assert_eq!(rows[1][column("usage_unknown")], "1");
    assert_eq!(rows[1][column("input_tokens")], "");
    assert_eq!(rows[1][column("input_coverage")], "0");
    assert_eq!(rows[2][column("input_tokens")], "0");
    assert_eq!(rows[2][column("input_coverage")], "1");
    assert_eq!(rows[2][column("usage_partial")], "1");
    for name in [
        "usage_complete",
        "truncated",
        "parse_error",
        "parse_limit_exceeded",
        "provider_error",
        "arithmetic_overflow",
        "cache_read_input_coverage",
    ] {
        assert_eq!(rows[3][column(name)], "1");
    }
    assert_eq!(rows[3][column("cache_read_input_tokens")], "6");
    let failure = s.export_all(query.clone(), &path).unwrap_err();
    assert_eq!(failure.file_state, ExportFileState::NotCreated);
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert_eq!(
        s.export_all(query, "relative.csv").unwrap_err().file_state,
        ExportFileState::NotCreated
    );
    let sorted = t.0.join("sorted.csv");
    s.export_all(
        Query {
            requested_model: Some(Model::new("@formula").unwrap()),
            detail_sort: DetailSort::TokensDesc(TokenField::Total),
            limit: 0,
            offset: u32::MAX,
            ..Query::default()
        },
        &sorted,
    )
    .unwrap();
    let sorted = parse_csv(&fs::read_to_string(sorted).unwrap());
    assert_eq!(sorted.len(), 129);
    assert_eq!(sorted[1][column("total_tokens")], "129");
}
#[test]
fn failed_export_explicitly_reports_incomplete_retained_file() {
    let t = Temp::new();
    let s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    let start = now_ms();
    put(&s, record(&h, start));
    put(&s, record(&h, start + 1));
    let db = rusqlite::Connection::open(t.0.join("history.sqlite3")).unwrap();
    db.execute(
        "UPDATE records SET report='invalid' WHERE started_ms=?",
        [start + 1],
    )
    .unwrap();
    let path = t.0.join("incomplete.csv");
    let error = s
        .export_all(
            Query {
                detail_sort: DetailSort::TimeAsc,
                ..Query::default()
            },
            &path,
        )
        .unwrap_err();
    assert_eq!(error.path, path);
    assert_eq!(error.rows_written, 1);
    assert_eq!(error.file_state, ExportFileState::Incomplete);
    assert!(path.is_file());
    assert_eq!(
        s.export_all(Query::default(), &path)
            .unwrap_err()
            .file_state,
        ExportFileState::NotCreated
    );
}
#[test]
fn aliases_validate_reopen_appear_in_reports_and_clear_only_with_opt_in() {
    for invalid in [
        "",
        "   ",
        "line\nname",
        "tab\tname",
        "\0",
        "name\u{202e}hidden",
        "prefix sk-synthetic-not-a-real-key",
        "Bearer synthetic",
        "sk_test_synthetic",
    ] {
        assert!(CredentialAlias::new(invalid).is_err(), "{invalid:?}");
    }
    assert!(CredentialAlias::new(&"字".repeat(64)).is_ok());
    assert!(CredentialAlias::new(&"字".repeat(65)).is_err());
    let t = Temp::new();
    let mut s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    let client = h.credential("synthetic-client-for-reports");
    let upstream = h.credential("synthetic-upstream-for-reports");
    s.set_alias(client.clone(), CredentialAlias::new("客户端甲").unwrap())
        .unwrap();
    s.set_alias(upstream.clone(), CredentialAlias::new("上游乙").unwrap())
        .unwrap();
    put(&s, record(&h, now_ms()));
    let r = s
        .query(Query {
            group_by: vec![GroupBy::Credential, GroupBy::UpstreamCredential],
            ..Query::default()
        })
        .unwrap();
    assert_eq!(r.aliases.len(), 2);
    assert_eq!(r.groups[0].keys[0].as_deref(), Some(client.as_str()));
    let snapshot = t.0.join("snapshot");
    s.snapshot_to(&snapshot).unwrap();
    let mut copy = HistoryService::start(&snapshot).unwrap();
    assert_eq!(copy.aliases().unwrap().len(), 2);
    copy.shutdown().unwrap();
    s.shutdown().unwrap();
    let mut s = HistoryService::start(&t.0).unwrap();
    assert_eq!(s.list_aliases().unwrap().len(), 2);
    s.set_alias(client.clone(), CredentialAlias::new("新显示名").unwrap())
        .unwrap();
    assert_eq!(s.aliases().unwrap().len(), 2);
    assert!(s.remove_alias(upstream.clone()).unwrap());
    assert!(!s.remove_alias(upstream).unwrap());
    s.clear().unwrap();
    assert_eq!(s.aliases().unwrap().len(), 1);
    assert_eq!(
        s.handle().credential("synthetic-client-for-reports"),
        client
    );
    s.clear_with_options(ClearOptions {
        remove_aliases: true,
    })
    .unwrap();
    assert!(s.aliases().unwrap().is_empty());
    assert_eq!(
        s.handle().credential("synthetic-client-for-reports"),
        client
    );
    put(&s, record(&s.handle(), now_ms()));
    s.shutdown().unwrap();
    let s = HistoryService::start(&t.0).unwrap();
    assert!(s.aliases().unwrap().is_empty());
    assert_eq!(s.query(Query::default()).unwrap().summary.requests, 1);
}
#[test]
fn configure_immediately_enforces_policy_and_updates_future_writes_stats() {
    let t = Temp::new();
    let s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    let time = now_ms();
    s.set_alias(
        h.credential("synthetic-client-for-reports"),
        CredentialAlias::new("保留的名称").unwrap(),
    )
    .unwrap();
    put(&s, record(&h, time - 3 * 86_400_000));
    for i in 0..4 {
        put(&s, record(&h, time + i));
    }
    let report = s
        .configure(Options {
            retention_days: 1,
            max_records: 2,
            ..Options::default()
        })
        .unwrap();
    assert_eq!(report.removed_records, 3);
    assert_eq!(report.checkpoint, CheckpointStatus::Complete);
    assert_eq!(report.storage.records, 2);
    assert_eq!(report.storage.aliases, 1);
    assert!(report.storage.database_bytes > 0);
    assert_eq!(report.storage.wal_bytes, 0);
    assert!(report.storage.total_bytes() >= report.storage.database_bytes);
    put(&s, record(&h, time - 2 * 86_400_000));
    put(&s, record(&h, time + 10));
    assert_eq!(s.query(Query::default()).unwrap().summary.requests, 2);
    for options in [
        Options {
            retention_days: 0,
            ..Options::default()
        },
        Options {
            max_records: 0,
            ..Options::default()
        },
        Options {
            queue_capacity: 2,
            ..Options::default()
        },
    ] {
        assert!(s.configure(options).is_err());
    }
    assert_eq!(s.stats().unwrap().options, report.options);
    s.configure(Options {
        max_records: 4,
        ..Options::default()
    })
    .unwrap();
    put(&s, record(&h, time - 2 * 86_400_000));
    assert_eq!(s.stats().unwrap().records, 3);
}
#[test]
fn busy_policy_checkpoint_reports_committed_deletion_without_rollback_claim() {
    let t = Temp::new();
    let s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    put(&s, record(&h, now_ms()));
    let reader = rusqlite::Connection::open(t.0.join("history.sqlite3")).unwrap();
    reader
        .execute_batch("BEGIN; SELECT * FROM records;")
        .unwrap();
    put(&s, record(&h, now_ms() + 1));
    let report = s
        .configure(Options {
            max_records: 1,
            ..Options::default()
        })
        .unwrap();
    assert_eq!(report.checkpoint, CheckpointStatus::Busy);
    assert_eq!(report.removed_records, 1);
    assert_eq!(s.stats().unwrap().options.max_records, 1);
    assert_eq!(s.query(Query::default()).unwrap().summary.requests, 1);
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        s.configure(report.options).unwrap().checkpoint,
        CheckpointStatus::Complete
    );
}
#[test]
fn v1_migration_preserves_records_identity_and_rejects_unknown_or_conflicting_schema() {
    let t = Temp::new();
    let mut s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    let credential = h.credential("synthetic-client-for-reports");
    put(&s, record(&h, now_ms()));
    s.shutdown().unwrap();
    let identity = fs::read(t.0.join("history.identity")).unwrap();
    let db = rusqlite::Connection::open(t.0.join("history.sqlite3")).unwrap();
    db.execute_batch("DROP TABLE credential_aliases; PRAGMA user_version=1;")
        .unwrap();
    drop(db);
    let mut s = HistoryService::start(&t.0).unwrap();
    assert_eq!(s.query(Query::default()).unwrap().summary.requests, 1);
    assert!(s.aliases().unwrap().is_empty());
    assert_eq!(
        s.handle().credential("synthetic-client-for-reports"),
        credential
    );
    s.set_alias(credential, CredentialAlias::new("迁移后名称").unwrap())
        .unwrap();
    s.shutdown().unwrap();
    assert_eq!(fs::read(t.0.join("history.identity")).unwrap(), identity);
    let db = rusqlite::Connection::open(t.0.join("history.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    db.execute_batch("PRAGMA user_version=1;").unwrap();
    drop(db);
    assert!(HistoryService::start(&t.0).is_err());
    let db = rusqlite::Connection::open(t.0.join("history.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM credential_aliases", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch("PRAGMA user_version=99;").unwrap();
    drop(db);
    assert!(HistoryService::start(&t.0).is_err());
    let db = rusqlite::Connection::open(t.0.join("history.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        99
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(fs::read(t.0.join("history.identity")).unwrap(), identity);
}
