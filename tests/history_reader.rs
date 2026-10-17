use ai_relay::{history::*, usage};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "relay-history-reader-{}-{}-{}",
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
fn record(h: &HistoryHandle) -> RequestRecord {
    RequestRecord {
        request_id: h.request_id(),
        started_ms: now_ms(),
        peer_ip: "127.0.0.1".parse().unwrap(),
        client: Some(h.credential("synthetic-reader-load")),
        upstream: None,
        provider: None,
        requested_model: Some(Model::new("live-request").unwrap()),
        protocol: Protocol::OpenAi,
        interface: Interface::Responses,
        status: Some(200),
        outcome: Outcome::Success,
        latency_ms: 1,
        total_ms: 2,
        report: usage::UsageReport {
            model: Some("live-response".into()),
            usage: usage::TokenUsage {
                total_tokens: Some(1),
                ..Default::default()
            },
            ..Default::default()
        },
    }
}
fn seed_100k(s: &HistoryService, path: &std::path::Path) {
    let h = s.handle();
    let mut r = record(&h);
    r.started_ms -= 100_000;
    assert_eq!(h.try_record(r), TryRecord::Queued);
    s.barrier().unwrap();
    let c = rusqlite::Connection::open(path.join("history.sqlite3")).unwrap();
    c.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<99999) INSERT INTO records SELECT request_id||'-'||n.x,started_ms+n.x,ip,printf('%064x',n.x),upstream,origin,printf('request-%06d',n.x),printf('response-%06d',n.x),protocol,interface,status,outcome,latency_ms,total_ms,report FROM records CROSS JOIN n;").unwrap();
}
fn produce(h: HistoryHandle, gate: Arc<Barrier>) -> (u64, u64, Duration) {
    gate.wait();
    // Start after the read request has crossed its writer barrier.
    thread::sleep(Duration::from_millis(100));
    let start = Instant::now();
    let mut queued = 0;
    let mut full = 0;
    for _ in 0..1500 {
        match h.try_record(record(&h)) {
            TryRecord::Queued => queued += 1,
            TryRecord::Full => full += 1,
            result => panic!("unexpected producer result: {result:?}"),
        }
        thread::sleep(Duration::from_millis(2));
    }
    (queued, full, start.elapsed())
}
#[test]
fn load_100k_three_groups_query_and_export_do_not_starve_2ms_writer() {
    let temp = Temp::new();
    let mut s = HistoryService::start(&temp.0).unwrap();
    seed_100k(&s, &temp.0);
    let gate = Arc::new(Barrier::new(2));
    let producer = s.handle();
    let thread_gate = gate.clone();
    let baseline = thread::spawn(move || produce(producer, thread_gate));
    gate.wait();
    let (queued, full, elapsed) = baseline.join().unwrap();
    s.barrier().unwrap();
    eprintln!(
        "baseline: queued={queued} full={full} producer_ms={} written={}",
        elapsed.as_millis(),
        s.handle().counters().written
    );
    assert_eq!(
        (queued, full),
        (1500, 0),
        "baseline writer throughput is insufficient"
    );
    let written_before = s.handle().counters().written;
    let gate = Arc::new(Barrier::new(2));
    let producer = s.handle();
    let thread_gate = gate.clone();
    let task = thread::spawn(move || produce(producer, thread_gate));
    gate.wait();
    let start = Instant::now();
    let report = s
        .query(Query {
            group_by: vec![GroupBy::RequestedModel, GroupBy::Model, GroupBy::Credential],
            detail_sort: DetailSort::TokensDesc(TokenField::Total),
            group_sort: GroupSort::TokensDesc(TokenField::Total),
            ..Query::default()
        })
        .unwrap();
    let query_elapsed = start.elapsed();
    let written_during = s.handle().counters().written - written_before;
    let (queued, full, elapsed) = task.join().unwrap();
    s.barrier().unwrap();
    eprintln!(
        "query: query_ms={} queued={queued} full={full} producer_ms={} written_before_query_return={written_during}",
        query_elapsed.as_millis(),
        elapsed.as_millis()
    );
    assert_eq!((queued, full), (1500, 0), "query starved the record writer");
    assert_eq!(s.handle().counters().written - written_before, 1500);
    assert!(written_during > 0, "writer made no progress during query");
    assert_eq!(report.summary.requests, 100_000);
    assert_eq!(report.details.len(), 100);
    assert_eq!(report.groups.len(), 100);
    assert!(report.trend.len() <= 366);
    assert!(query_elapsed < Duration::from_secs(60));
    let written_before = s.handle().counters().written;
    let gate = Arc::new(Barrier::new(2));
    let producer = s.handle();
    let thread_gate = gate.clone();
    let task = thread::spawn(move || produce(producer, thread_gate));
    gate.wait();
    let start = Instant::now();
    let export = s
        .export_all(
            Query {
                detail_sort: DetailSort::TokensDesc(TokenField::Total),
                ..Query::default()
            },
            temp.0.join("load.csv"),
        )
        .unwrap();
    let export_elapsed = start.elapsed();
    let written_during = s.handle().counters().written - written_before;
    let (queued, full, elapsed) = task.join().unwrap();
    s.barrier().unwrap();
    eprintln!(
        "export: export_ms={} rows={} bytes={} queued={queued} full={full} producer_ms={} written_before_export_return={written_during}",
        export_elapsed.as_millis(),
        export.rows,
        export.bytes,
        elapsed.as_millis()
    );
    assert_eq!(export.rows, 100_000);
    assert_eq!(
        (queued, full),
        (1500, 0),
        "CSV export starved the record writer"
    );
    assert_eq!(s.handle().counters().written - written_before, 1500);
    assert!(written_during > 0, "writer made no progress during export");
    assert_eq!(s.handle().counters().queue_full, 0);
    assert_eq!(s.handle().counters().db_failures, 0);
    s.shutdown().unwrap();
}
#[test]
fn retention_count_cache_reconciles_external_edits_and_policy_clear() {
    let temp = Temp::new();
    let mut s = HistoryService::start_with_options(
        &temp.0,
        Options {
            max_records: 3,
            ..Options::default()
        },
    )
    .unwrap();
    let h = s.handle();
    for _ in 0..3 {
        assert_eq!(h.try_record(record(&h)), TryRecord::Queued);
    }
    s.barrier().unwrap();
    let external = rusqlite::Connection::open(temp.0.join("history.sqlite3")).unwrap();
    external.execute_batch("INSERT INTO records SELECT request_id||'-external',started_ms-10000,ip,client,upstream,origin,requested_model,response_model,protocol,interface,status,outcome,latency_ms,total_ms,report FROM records;").unwrap();
    assert_eq!(h.try_record(record(&h)), TryRecord::Queued);
    s.barrier().unwrap();
    assert_eq!(s.query(Query::default()).unwrap().summary.requests, 3);
    external.execute_batch("DELETE FROM records;").unwrap();
    for _ in 0..3 {
        assert_eq!(h.try_record(record(&h)), TryRecord::Queued);
    }
    s.barrier().unwrap();
    assert_eq!(s.stats().unwrap().records, 3);
    s.configure(Options {
        max_records: 1,
        ..Options::default()
    })
    .unwrap();
    assert_eq!(h.try_record(record(&h)), TryRecord::Queued);
    s.barrier().unwrap();
    assert_eq!(s.stats().unwrap().records, 1);
    s.clear().unwrap();
    assert_eq!(h.try_record(record(&h)), TryRecord::Queued);
    s.barrier().unwrap();
    assert_eq!(s.stats().unwrap().records, 1);
    drop(external);
    assert_eq!(h.counters().db_failures, 0);
    s.shutdown().unwrap();
}
#[test]
fn read_barrier_includes_prior_queued_records_and_latest_committed_alias() {
    let temp = Temp::new();
    let mut s = HistoryService::start(&temp.0).unwrap();
    let h = s.handle();
    for _ in 0..100 {
        assert_eq!(h.try_record(record(&h)), TryRecord::Queued);
    }
    assert_eq!(s.query(Query::default()).unwrap().summary.requests, 100);
    let credential = h.credential("synthetic-reader-load");
    s.set_alias(
        credential.clone(),
        CredentialAlias::new("first label").unwrap(),
    )
    .unwrap();
    assert_eq!(
        s.query(Query::default()).unwrap().aliases[0].alias.as_str(),
        "first label"
    );
    s.set_alias(credential, CredentialAlias::new("current label").unwrap())
        .unwrap();
    assert_eq!(
        s.query(Query::default()).unwrap().aliases[0].alias.as_str(),
        "current label"
    );
    let summary = s
        .export_all(Query::default(), temp.0.join("ordered.csv"))
        .unwrap();
    assert_eq!(summary.rows, 100);
    let csv = fs::read_to_string(&summary.path).unwrap();
    assert!(csv.contains("current label"));
    assert!(!csv.contains("first label"));
    s.shutdown().unwrap();
}
