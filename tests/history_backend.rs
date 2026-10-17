#![allow(dead_code)]
#[path = "../src/history/mod.rs"]
mod history;
#[allow(unused_imports)]
#[path = "../src/usage/mod.rs"]
mod usage;
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
        let p = std::env::temp_dir().join(format!(
            "relay-history-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn credential_filter_accepts_full_identifier_not_secret_or_prefix() {
    let id = "AB".repeat(32);
    assert_eq!(CredentialId::parse(&id).unwrap().as_str(), "ab".repeat(32));
    assert!(CredentialId::parse("synthetic-api-key").is_err());
    assert!(CredentialId::parse(&id[..12]).is_err());
    assert!(CredentialId::parse(&"z".repeat(64)).is_err());
}

fn record(h: &HistoryHandle) -> RequestRecord {
    RequestRecord {
        request_id: h.request_id(),
        started_ms: now_ms(),
        peer_ip: "127.0.0.1".parse().unwrap(),
        client: Some(h.credential("test-client-secret")),
        upstream: Some(h.credential("test-upstream-secret")),
        provider: Some(
            ProviderOrigin::new("https://user:password@example.invalid/private?key=secret")
                .unwrap(),
        ),
        requested_model: None,
        protocol: Protocol::OpenAi,
        interface: Interface::Responses,
        status: Some(200),
        outcome: Outcome::Success,
        latency_ms: 2,
        total_ms: 10,
        report: usage::UsageReport::default(),
    }
}
fn put(s: &HistoryService, r: RequestRecord) {
    assert_eq!(s.handle().try_record(r), TryRecord::Queued);
    s.barrier().unwrap();
}
#[test]
fn persistence_identity_and_metadata_privacy() {
    let t = Temp::new();
    let mut s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    let id = h.credential_id("test-client-secret");
    put(&s, record(&h));
    s.shutdown().unwrap();
    let s = HistoryService::start(&t.0).unwrap();
    assert_eq!(s.handle().credential_id("test-client-secret"), id);
    assert_ne!(s.handle().request_id(), h.request_id());
    let report = s.query(Query::default()).unwrap();
    assert_eq!(report.summary.requests, 1);
    assert_eq!(
        report.details[0].provider.as_ref().unwrap().as_str(),
        "https://example.invalid"
    );
    assert_eq!(report.summary.tokens.total_tokens, None);
    assert_eq!(report.summary.unknown, 1);
    let csv = report.export_csv();
    assert!(!csv.contains("127.0.0.1"));
    assert!(!csv.contains(&id));
    drop(s);
    for name in ["history.sqlite3", "history.sqlite3-wal"] {
        if let Ok(bytes) = fs::read(t.0.join(name)) {
            let text = String::from_utf8_lossy(&bytes);
            for secret in [
                "test-client-secret",
                "test-upstream-secret",
                "password",
                "private?key",
            ] {
                assert!(!text.contains(secret));
            }
        }
    }
}
#[test]
fn combined_filters_groups_dates_and_coverage() {
    let t = Temp::new();
    let s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    let day = now_ms() / 86_400_000 * 86_400_000;
    let mut a = record(&h);
    a.started_ms = day + 1;
    a.requested_model = Some(Model::new("requested-v1").unwrap());
    a.report.model = Some("actual-v1".into());
    a.report.usage.input_tokens = Some(0);
    a.report.usage.output_tokens = Some(3);
    a.report.usage.total_tokens = Some(3);
    a.report.parse_error = true;
    put(&s, a.clone());
    let mut b = record(&h);
    b.started_ms = day - 1;
    put(&s, b);
    let q = Query {
        time_from: Some(day),
        time_to: Some(day + 86_400_000),
        ip: Some(a.peer_ip),
        requested_model: a.requested_model.clone(),
        model: Some(Model::new("actual-v1").unwrap()),
        client: a.client.clone(),
        upstream: a.upstream.clone(),
        protocol: Some(a.protocol),
        provider: a.provider.clone(),
        status: a.status,
        outcome: Some(a.outcome),
        interface: Some(a.interface),
        group_by: vec![GroupBy::Day, GroupBy::Model, GroupBy::Credential],
        ..Query::default()
    };
    let r = s.query(q).unwrap();
    assert_eq!(r.summary.requests, 1);
    assert_eq!(r.summary.complete, 1);
    assert_eq!(r.summary.tokens.input_tokens, Some(0));
    assert_eq!(r.summary.coverage, [1, 1, 1, 0, 0, 0, 0]);
    assert_eq!(r.summary.parse_errors, 1);
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].keys[1], Some("actual-v1".into()));
    assert_eq!(r.trend[0].utc_day, day / 86_400_000);
    assert_eq!(r.trend[0].summary.requests, 1);
    let all = s
        .query(Query {
            group_by: vec![GroupBy::Model],
            ..Query::default()
        })
        .unwrap();
    assert_eq!(all.summary.unknown, 1);
    assert!(all.groups.iter().any(|g| g.keys == vec![None]));
    assert_eq!(all.details.len(), 2);
    assert!(
        s.query(Query {
            limit: 101,
            ..Query::default()
        })
        .is_err()
    );
    assert!(
        s.query(Query {
            offset: 100001,
            ..Query::default()
        })
        .is_err()
    );
    assert!(
        s.query(Query {
            group_by: vec![GroupBy::Day; 4],
            ..Query::default()
        })
        .is_err()
    );
}
#[test]
fn retention_and_duplicate_failures() {
    let t = Temp::new();
    let s = HistoryService::start_with_options(
        &t.0,
        Options {
            max_records: 2,
            retention_days: 1,
            ..Options::default()
        },
    )
    .unwrap();
    let h = s.handle();
    let mut old = record(&h);
    old.started_ms = now_ms() - 2 * 86_400_000;
    put(&s, old);
    for _ in 0..3 {
        put(&s, record(&h));
    }
    assert_eq!(s.query(Query::default()).unwrap().summary.requests, 2);
    let r = record(&h);
    put(&s, r.clone());
    put(&s, r);
    assert_eq!(h.counters().db_failures, 1);
    assert_eq!(s.query(Query::default()).unwrap().summary.requests, 2);
}
#[test]
fn bounded_queue_and_drop_drain() {
    let t = Temp::new();
    let s = HistoryService::start_with_options(
        &t.0,
        Options {
            queue_capacity: 1,
            ..Options::default()
        },
    )
    .unwrap();
    let h = s.handle();
    let lock = rusqlite::Connection::open(t.0.join("history.sqlite3")).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut full = false;
    for _ in 0..100 {
        if h.try_record(record(&h)) == TryRecord::Full {
            full = true;
            break;
        }
    }
    assert!(full);
    assert!(h.counters().queue_full > 0);
    lock.execute_batch("ROLLBACK").unwrap();
    drop(lock);
    drop(s);
    assert_eq!(h.try_record(record(&h)), TryRecord::Closed);
    let s = HistoryService::start(&t.0).unwrap();
    assert_eq!(
        s.query(Query::default()).unwrap().summary.requests,
        h.counters().written
    );
    assert!(h.counters().written > 0);
}
#[test]
fn migration_rejects_newer_or_unversioned_database() {
    for version in [0, 2] {
        let t = Temp::new();
        let mut s = HistoryService::start(&t.0).unwrap();
        s.shutdown().unwrap();
        let c = rusqlite::Connection::open(t.0.join("history.sqlite3")).unwrap();
        c.pragma_update(None, "user_version", version).unwrap();
        drop(c);
        assert!(HistoryService::start(&t.0).is_err());
    }
}
#[test]
fn input_safety_and_integer_unknown_semantics() {
    let t = Temp::new();
    let s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    assert!(Model::new("a\nsecret").is_err());
    assert!(ProviderOrigin::new("file:///private").is_err());
    let mut a = record(&h);
    a.report.usage.total_tokens = Some(u64::MAX);
    a.report.truncated = true;
    put(&s, a);
    let mut b = record(&h);
    b.report.usage.total_tokens = Some(1);
    put(&s, b);
    let r = s.query(Query::default()).unwrap();
    assert_eq!(r.summary.tokens.total_tokens, Some(u64::MAX));
    assert_eq!(r.summary.tokens.input_tokens, None);
    assert_eq!(r.summary.coverage[2], 2);
    assert_eq!(r.summary.partial, 2);
    assert_eq!(r.summary.truncated, 1);
    assert_eq!(r.summary.arithmetic_overflows, 1);
    assert!(HistoryService::start("relative").is_err());
}
#[test]
fn pagination_and_identity_fail_closed() {
    let t = Temp::new();
    let mut s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    for _ in 0..4 {
        put(&s, record(&h));
    }
    let first = s
        .query(Query {
            limit: 2,
            ..Query::default()
        })
        .unwrap();
    let second = s
        .query(Query {
            limit: 2,
            offset: 2,
            ..Query::default()
        })
        .unwrap();
    assert_eq!(first.summary.requests, 4);
    assert_eq!(second.details.len(), 2);
    assert!(
        first
            .details
            .iter()
            .all(|a| second.details.iter().all(|b| a.request_id != b.request_id))
    );
    s.shutdown().unwrap();
    fs::remove_file(t.0.join("history.identity")).unwrap();
    assert!(HistoryService::start(&t.0).is_err());
    assert!(!t.0.join("history.identity").exists());
}
#[test]
fn malformed_identity_is_never_overwritten() {
    let t = Temp::new();
    fs::write(t.0.join("history.identity"), b"invalid-test-identity").unwrap();
    assert!(HistoryService::start(&t.0).is_err());
    assert_eq!(
        fs::read(t.0.join("history.identity")).unwrap(),
        b"invalid-test-identity"
    );
}
#[test]
fn trend_is_bounded_to_366_utc_days() {
    let t = Temp::new();
    let s = HistoryService::start(&t.0).unwrap();
    put(&s, record(&s.handle()));
    let c = rusqlite::Connection::open(t.0.join("history.sqlite3")).unwrap();
    c.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<367) INSERT INTO records SELECT request_id || '-' || n.x, started_ms+n.x*86400000,ip,client,upstream,origin,requested_model,response_model,protocol,interface,status,outcome,latency_ms,total_ms,report FROM records CROSS JOIN n;").unwrap();
    let report = s.query(Query::default()).unwrap();
    assert_eq!(report.summary.requests, 368);
    assert_eq!(report.details.len(), 100);
    assert_eq!(report.trend.len(), 366);
    assert!(report.trend_truncated);
    assert!(
        report
            .trend
            .iter()
            .all(|p| p.summary.tokens.total_tokens.is_none())
    );
}
#[test]
fn snapshot_preserves_identity_and_is_independent() {
    let t = Temp::new();
    let s = HistoryService::start(t.0.join("source")).unwrap();
    let h = s.handle();
    put(&s, record(&h));
    let target = t.0.join("snapshot");
    s.snapshot_to(&target).unwrap();
    let copy = HistoryService::start(&target).unwrap();
    assert_eq!(
        copy.handle().credential_id("test-client-secret"),
        h.credential_id("test-client-secret")
    );
    assert_eq!(copy.query(Query::default()).unwrap().summary.requests, 1);
    put(&s, record(&h));
    assert_eq!(copy.query(Query::default()).unwrap().summary.requests, 1);
    assert!(s.snapshot_to(&target).is_err());
    assert_eq!(copy.query(Query::default()).unwrap().summary.requests, 1);
    assert!(s.snapshot_to("relative").is_err());
    s.clear().unwrap();
    assert_eq!(s.query(Query::default()).unwrap().summary.requests, 0);
    assert_eq!(
        s.handle().credential_id("test-client-secret"),
        copy.handle().credential_id("test-client-secret")
    );
    put(&s, record(&h));
    assert_eq!(s.query(Query::default()).unwrap().summary.requests, 1);
}
#[test]
fn close_timeout_is_not_proof_and_retry_drains_pending() {
    use std::time::{Duration, Instant};
    let t = Temp::new();
    let mut s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    let lock = rusqlite::Connection::open(t.0.join("history.sqlite3")).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(h.try_record(record(&h)), TryRecord::Queued);
    let start = Instant::now();
    assert!(
        s.flush_close(Duration::from_millis(20))
            .unwrap_err()
            .is_timeout()
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(h.try_record(record(&h)), TryRecord::Closed);
    lock.execute_batch("ROLLBACK").unwrap();
    drop(lock);
    s.flush_close(Duration::from_secs(3)).unwrap();
    s.flush_close(Duration::ZERO).unwrap();
    let reopened = HistoryService::start(&t.0).unwrap();
    assert_eq!(
        reopened.query(Query::default()).unwrap().summary.requests,
        1
    );
    assert_eq!(
        fs::metadata(t.0.join("history.sqlite3-wal"))
            .map(|m| m.len())
            .unwrap_or(0),
        0
    );
}
#[test]
fn concurrent_admission_drains_before_successful_close() {
    use std::{
        sync::{Arc, Barrier},
        time::Duration,
    };
    let t = Temp::new();
    let mut s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    let gate = Arc::new(Barrier::new(2));
    let thread_gate = gate.clone();
    let producer = h.clone();
    let task = std::thread::spawn(move || {
        thread_gate.wait();
        for _ in 0..1000 {
            if producer.try_record(record(&producer)) == TryRecord::Closed {
                break;
            }
        }
    });
    assert_eq!(h.try_record(record(&h)), TryRecord::Queued);
    gate.wait();
    s.flush_close(Duration::from_secs(5)).unwrap();
    task.join().unwrap();
    assert_eq!(h.counters().queued, h.counters().written);
    let reopened = HistoryService::start(&t.0).unwrap();
    assert_eq!(
        reopened.query(Query::default()).unwrap().summary.requests,
        h.counters().written
    );
}
#[test]
fn failed_queued_write_never_returns_successful_flush() {
    use std::time::Duration;
    let t = Temp::new();
    let mut s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    let r = record(&h);
    put(&s, r.clone());
    put(&s, r);
    assert!(s.flush_close(Duration::from_secs(3)).is_err());
    assert!(s.flush_close(Duration::from_millis(20)).is_err());
    assert_eq!(h.counters().db_failures, 1);
    let reopened = HistoryService::start(&t.0).unwrap();
    assert_eq!(
        reopened.query(Query::default()).unwrap().summary.requests,
        1
    );
}
#[test]
fn blocked_checkpoint_prevents_close_proof() {
    use std::time::Duration;
    let t = Temp::new();
    let mut s = HistoryService::start(&t.0).unwrap();
    let h = s.handle();
    put(&s, record(&h));
    let reader = rusqlite::Connection::open(t.0.join("history.sqlite3")).unwrap();
    reader
        .execute_batch("BEGIN; SELECT * FROM records;")
        .unwrap();
    put(&s, record(&h));
    assert!(matches!(
        s.flush_close(Duration::from_secs(4)),
        Err(Error::Busy)
    ));
    reader.execute_batch("ROLLBACK").unwrap();
    drop(reader);
    let reopened = HistoryService::start(&t.0).unwrap();
    assert_eq!(
        reopened.query(Query::default()).unwrap().summary.requests,
        2
    );
}
#[test]
fn sidecar_directory_is_rejected() {
    let t = Temp::new();
    fs::create_dir(t.0.join("history.sqlite3-wal")).unwrap();
    assert!(HistoryService::start(&t.0).is_err());
}
