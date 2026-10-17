use ai_relay::logging::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ai-relay-logging-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn logs(&self) -> PathBuf {
        self.0.join("data").join("logs")
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn event() -> LogEvent {
    LogEvent::Runtime {
        level: LogLevel::Info,
        event: RuntimeEvent::Started,
        error: None,
    }
}
fn stop(service: &mut LogService) {
    service.shutdown(Duration::from_secs(2)).unwrap();
}
fn emit(service: &LogService, count: usize) {
    let handle = service.handle();
    for _ in 0..count {
        assert_eq!(handle.try_log(event()), TryLogResult::Queued);
        service.barrier().unwrap();
    }
}
#[test]
fn rotation_clear_and_resume_preserve_unowned_files() {
    let temp = Temp::new();
    fs::write(temp.0.join("old.log"), b"legacy").unwrap();
    let mut service = LogService::start(
        temp.logs(),
        LogOptions {
            max_file_bytes: 1024,
            ..Default::default()
        },
    )
    .unwrap();
    fs::write(temp.logs().join("other.log"), b"keep").unwrap();
    emit(&service, 60);
    let files = service.list_files().unwrap();
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|file| file.bytes <= 1024));
    let history_bytes: u64 = files
        .iter()
        .filter(|f| f.name != CURRENT_FILE)
        .map(|f| f.bytes)
        .sum();
    let report = service.clear_history().unwrap();
    assert_eq!(report.deleted_files, 2);
    assert_eq!(report.deleted_bytes, history_bytes);
    assert!(report.errors.is_empty());
    assert_eq!(service.list_files().unwrap().len(), 1);
    let report = service.clear_all().unwrap();
    assert_eq!(report.deleted_files, 1);
    assert!(report.errors.is_empty());
    assert_eq!(service.list_files().unwrap()[0].bytes, 0);
    emit(&service, 1);
    assert!(service.list_files().unwrap()[0].bytes > 0);
    assert_eq!(fs::read(temp.0.join("old.log")).unwrap(), b"legacy");
    assert_eq!(fs::read(temp.logs().join("other.log")).unwrap(), b"keep");
    stop(&mut service);
}
#[test]
fn bounded_snapshot_cursor_and_independent_switches() {
    let temp = Temp::new();
    let mut options = LogOptions {
        file_enabled: false,
        access_enabled: false,
        recent_entries: 3,
        ..Default::default()
    };
    let mut service = LogService::start(temp.logs(), options.clone()).unwrap();
    let handle = service.handle();
    emit(&service, 5);
    assert!(service.list_files().unwrap().is_empty());
    let snapshot = handle.snapshot(Some(0));
    assert_eq!(snapshot.entries.len(), 3);
    assert!(snapshot.gap);
    assert_eq!(snapshot.cursor, 5);
    assert!(handle.snapshot(Some(snapshot.cursor)).entries.is_empty());
    handle.try_log(LogEvent::Access {
        level: LogLevel::Error,
        metadata: AccessMetadata::default(),
    });
    service.barrier().unwrap();
    assert_eq!(handle.snapshot(None).cursor, 5);
    options.file_enabled = true;
    options.level = LogLevel::Error;
    service.configure(options.clone()).unwrap();
    emit(&service, 1);
    assert_eq!(handle.snapshot(None).cursor, 5);
    handle.try_log(LogEvent::Runtime {
        level: LogLevel::Error,
        event: RuntimeEvent::OperationFailed,
        error: Some(ErrorCode::Tls),
    });
    service.barrier().unwrap();
    assert!(service.list_files().unwrap()[0].bytes > 0);
    options.recent_entries = 1;
    service.configure(options).unwrap();
    assert_eq!(handle.snapshot(None).entries.len(), 1);
    stop(&mut service);
    assert_eq!(handle.try_log(event()), TryLogResult::Closed);
}
#[test]
fn tail_is_bounded_and_rejects_traversal() {
    let temp = Temp::new();
    let mut service = LogService::start(
        temp.logs(),
        LogOptions {
            file_enabled: false,
            ..Default::default()
        },
    )
    .unwrap();
    fs::write(temp.logs().join(CURRENT_FILE), b"one\ntwo\nthree\nfour\n").unwrap();
    assert_eq!(
        service.read_tail(CURRENT_FILE, 1024, 2).unwrap().text,
        "three\nfour\n"
    );
    assert!(service.read_tail(CURRENT_FILE, 8, 10).unwrap().truncated);
    for name in [
        "../old.log",
        "ai-relay.01.log",
        "AI-RELAY.1.log",
        "ai-relay.100.log",
        "ai-relay.1.log:stream",
        "a/ai-relay.1.log",
    ] {
        assert!(matches!(
            service.read_tail(name, 10, 1),
            Err(LogError::InvalidName)
        ));
    }
    assert!(matches!(
        service.read_tail(CURRENT_FILE, MAX_TAIL_BYTES + 1, 1),
        Err(LogError::InvalidTailBounds)
    ));
    assert!(matches!(
        service.read_tail(CURRENT_FILE, 1, MAX_TAIL_LINES + 1),
        Err(LogError::InvalidTailBounds)
    ));
    fs::write(
        temp.logs().join(CURRENT_FILE),
        vec![0xff; MAX_TAIL_BYTES * 3],
    )
    .unwrap();
    let result = service.read_tail(CURRENT_FILE, MAX_TAIL_BYTES, 10).unwrap();
    assert!(result.text.len() <= MAX_TAIL_BYTES);
    assert!(result.truncated);
    fs::write(temp.logs().join(CURRENT_FILE), vec![0xff; 1024]).unwrap();
    assert!(
        service
            .read_tail(CURRENT_FILE, 1024, 10)
            .unwrap()
            .text
            .len()
            <= 1024
    );
    stop(&mut service);
}
#[test]
fn typed_events_do_not_retain_sensitive_paths() {
    let temp = Temp::new();
    let mut service = LogService::start(temp.logs(), LogOptions::default()).unwrap();
    let secret = "synthetic-secret-query-body-key";
    for path in [
        format!("/v1/chat/completions?token={secret}"),
        format!("https://user:{secret}@example.invalid/v1/messages"),
        format!("/unknown/{secret}"),
    ] {
        assert_eq!(Endpoint::from_path(&path), Endpoint::Redacted);
        service.handle().try_log(LogEvent::Access {
            level: LogLevel::Info,
            metadata: AccessMetadata {
                endpoint: Some(Endpoint::from_path(&path)),
                request_id: Some(42),
                ..Default::default()
            },
        });
    }
    service.barrier().unwrap();
    let text = service
        .read_tail(CURRENT_FILE, MAX_TAIL_BYTES, 100)
        .unwrap()
        .text;
    assert!(!text.contains(secret));
    assert!(text.contains("redacted"));
    assert!(!format!("{:?}", service.handle().snapshot(None)).contains(secret));
    assert_eq!(Endpoint::from_path("/v1/messages"), Endpoint::Messages);
    for line in text.lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
    stop(&mut service);
}
#[test]
fn disk_failure_is_visible_but_memory_continues_and_clear_reports_errors() {
    let temp = Temp::new();
    fs::create_dir_all(temp.logs().join(CURRENT_FILE)).unwrap();
    let mut service = LogService::start(temp.logs(), LogOptions::default()).unwrap();
    emit(&service, 3);
    let snapshot = service.handle().snapshot(None);
    assert_eq!(snapshot.entries.len(), 3);
    assert!(snapshot.status.disk_errors >= 4);
    assert!(snapshot.status.last_persisted_sequence.is_none());
    let report = service.clear_all().unwrap();
    assert!(report.errors.iter().any(|error| error.name == CURRENT_FILE));
    fs::remove_dir(temp.logs().join(CURRENT_FILE)).unwrap();
    emit(&service, 1);
    assert_eq!(
        service
            .handle()
            .snapshot(None)
            .status
            .last_persisted_sequence,
        Some(4)
    );
    stop(&mut service);
}
#[test]
fn retention_and_count_are_applied_on_start_and_configuration() {
    let temp = Temp::new();
    fs::create_dir_all(temp.logs()).unwrap();
    let old = temp.logs().join("ai-relay.1.log");
    fs::write(&old, b"old").unwrap();
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(86400)),
        )
        .unwrap();
    fs::copy(&old, temp.logs().join(CURRENT_FILE)).unwrap();
    fs::File::options()
        .write(true)
        .open(temp.logs().join(CURRENT_FILE))
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(86400)),
        )
        .unwrap();
    fs::write(temp.logs().join("ai-relay.2.log"), b"new").unwrap();
    let mut service = LogService::start(temp.logs(), LogOptions::default()).unwrap();
    assert!(!old.exists());
    assert_eq!(
        fs::metadata(temp.logs().join(CURRENT_FILE)).unwrap().len(),
        0
    );
    assert!(temp.logs().join("ai-relay.2.log").exists());
    service
        .configure(LogOptions {
            retained_files: 1,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(service.list_files().unwrap().len(), 1);
    service
        .configure(LogOptions {
            retained_files: 1,
            max_file_bytes: 1024,
            ..Default::default()
        })
        .unwrap();
    emit(&service, 30);
    let files = service.list_files().unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].bytes <= 1024);
    stop(&mut service);
}
#[test]
fn reject_relative_and_linked_directories_and_hardlinked_files() {
    assert!(matches!(
        LogService::start("relative/logs", LogOptions::default()),
        Err(LogError::InvalidDirectory)
    ));
    let temp = Temp::new();
    let outside = temp.0.join("outside");
    fs::write(&outside, b"must not append").unwrap();
    fs::create_dir_all(temp.logs()).unwrap();
    fs::hard_link(&outside, temp.logs().join(CURRENT_FILE)).unwrap();
    let mut service = LogService::start(temp.logs(), LogOptions::default()).unwrap();
    emit(&service, 1);
    assert!(service.handle().snapshot(None).status.disk_errors > 0);
    assert_eq!(fs::read(&outside).unwrap(), b"must not append");
    stop(&mut service);
}
#[cfg(windows)]
#[test]
fn reject_junction_and_share_current_for_read_and_delete() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = Temp::new();
    let mut service = LogService::start(temp.logs(), LogOptions::default()).unwrap();
    emit(&service, 1);
    let reader = fs::OpenOptions::new()
        .read(true)
        .share_mode(7)
        .open(temp.logs().join(CURRENT_FILE))
        .unwrap();
    let report = service.clear_all().unwrap();
    assert!(report.errors.is_empty());
    emit(&service, 1);
    drop(reader);
    stop(&mut service);
    let target = temp.0.join("target");
    fs::create_dir(&target).unwrap();
    let junction = temp.0.join("junction");
    let output = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&target)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(matches!(
        LogService::start(junction.join("logs"), LogOptions::default()),
        Err(LogError::UnsafePath)
    ));
    assert!(!target.join("logs").exists());
    fs::remove_dir(junction).unwrap();
}
