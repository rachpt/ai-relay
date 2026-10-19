mod startup;

use super::*;
use super::{
    history::{HistoryOwner, export_history, history_error, history_options},
    logging::log_view,
    results::overflow_message,
    session::{Session, check_secrets, start, start_authorized, stop},
    util::overview_snapshot,
};
use crate::{
    app::{
        Options,
        instance::{Action, Control, Signals},
    },
    config::{self, Config, DirectorySelection, DirectorySource, SecretStorage, StorageMode},
    history::{self as db, HistoryHandle, HistoryService},
    logging::{self, LogEvent, RuntimeEvent},
    proxy::ProxyMetrics,
    usage::UsageStore,
    windows::preferences,
};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, TrySendError},
    },
    thread,
    time::Duration,
};

fn history_response(queue: &ResultQueue<HistoryResponse>) -> HistoryResponse {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(response) = queue.try_recv() {
            return response;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "history response timed out"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn test_record(handle: &HistoryHandle) -> db::RequestRecord {
    db::RequestRecord {
        request_id: handle.request_id(),
        started_ms: db::now_ms(),
        peer_ip: "127.0.0.1".parse().unwrap(),
        client: None,
        upstream: None,
        provider: None,
        requested_model: None,
        protocol: db::Protocol::OpenAi,
        interface: db::Interface::Responses,
        status: Some(200),
        outcome: db::Outcome::Success,
        latency_ms: 1,
        total_ms: 1,
        report: crate::usage::UsageReport::default(),
    }
}

#[test]
fn history_owner_routes_alias_storage_export_and_scoped_clear() {
    let directory = std::env::temp_dir().join(format!(
        "relay-controller-history-management-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let events = ResultQueue::new(|latest: &HistoryResponse, count| HistoryResponse::Error {
        directory: latest.directory(),
        message: overflow_message(count),
    });
    let mut owner =
        HistoryOwner::start(directory.clone(), events.clone(), db::Options::default()).unwrap();
    let credential = owner.handle.credential("synthetic-controller-credential");
    owner
        .commands
        .send(Command::HistorySetAlias {
            credential: credential.clone(),
            alias: Some(db::CredentialAlias::new("测试客户端").unwrap()),
        })
        .unwrap();
    assert!(
        matches!(history_response(&events), HistoryResponse::AliasChanged { directory: scope } if scope == directory)
    );
    owner.commands.send(Command::HistoryStorage).unwrap();
    assert!(
        matches!(history_response(&events), HistoryResponse::Storage { stats, .. } if stats.records == 0 && stats.aliases == 1)
    );
    owner
        .commands
        .send(Command::HistoryClear {
            confirmed: true,
            remove_aliases: true,
        })
        .unwrap();
    assert!(matches!(
        history_response(&events),
        HistoryResponse::Error { .. }
    ));
    owner
        .commands
        .send(Command::HistoryClear {
            confirmed: false,
            remove_aliases: false,
        })
        .unwrap();
    assert!(matches!(
        history_response(&events),
        HistoryResponse::ClearPreview {
            records: 0,
            remove_aliases: false,
            ..
        }
    ));
    owner
        .commands
        .send(Command::HistoryClear {
            confirmed: true,
            remove_aliases: true,
        })
        .unwrap();
    assert!(matches!(
        history_response(&events),
        HistoryResponse::Error { .. }
    ));
    owner
        .commands
        .send(Command::HistoryClear {
            confirmed: true,
            remove_aliases: false,
        })
        .unwrap();
    assert!(matches!(
        history_response(&events),
        HistoryResponse::Error { .. }
    ));
    owner
        .commands
        .send(Command::HistoryClear {
            confirmed: false,
            remove_aliases: false,
        })
        .unwrap();
    assert!(matches!(
        history_response(&events),
        HistoryResponse::ClearPreview { .. }
    ));
    assert_eq!(
        owner.handle.try_record(test_record(&owner.handle)),
        db::TryRecord::Queued
    );
    owner.commands.send(Command::HistoryStorage).unwrap();
    assert!(
        matches!(history_response(&events), HistoryResponse::Storage { stats, .. } if stats.records == 1)
    );
    owner
        .commands
        .send(Command::HistoryClear {
            confirmed: true,
            remove_aliases: false,
        })
        .unwrap();
    assert!(
        matches!(history_response(&events), HistoryResponse::Error { message, .. } if message.contains("记录数"))
    );
    assert_eq!(
        owner.handle.try_record(test_record(&owner.handle)),
        db::TryRecord::Queued
    );
    let path = directory.join("all.csv");
    let query = db::Query {
        limit: 1,
        offset: 1,
        ..db::Query::default()
    };
    owner
        .commands
        .send(Command::HistoryExportAll {
            query: query.clone(),
            path: path.clone(),
            confirmed: false,
        })
        .unwrap();
    assert!(matches!(
        history_response(&events),
        HistoryResponse::Error { .. }
    ));
    assert!(!path.exists());
    owner
        .commands
        .send(Command::HistoryExportAll {
            query: query.clone(),
            path: path.clone(),
            confirmed: true,
        })
        .unwrap();
    assert!(matches!(
        history_response(&events),
        HistoryResponse::Exported {
            rows: 2,
            page_only: false,
            ..
        }
    ));
    let original = std::fs::read(&path).unwrap();
    owner
        .commands
        .send(Command::HistoryExportAll {
            query,
            path: path.clone(),
            confirmed: true,
        })
        .unwrap();
    assert!(matches!(
        history_response(&events),
        HistoryResponse::Error { .. }
    ));
    assert_eq!(std::fs::read(path).unwrap(), original);
    for remove_aliases in [false, true] {
        owner
            .commands
            .send(Command::HistoryClear {
                confirmed: false,
                remove_aliases,
            })
            .unwrap();
        assert!(matches!(
            history_response(&events),
            HistoryResponse::ClearPreview { .. }
        ));
        owner
            .commands
            .send(Command::HistoryClear {
                confirmed: true,
                remove_aliases,
            })
            .unwrap();
        assert!(
            matches!(history_response(&events), HistoryResponse::Cleared { remove_aliases: actual, .. } if actual == remove_aliases)
        );
        owner.commands.send(Command::HistoryStorage).unwrap();
        assert!(
            matches!(history_response(&events), HistoryResponse::Storage { stats, .. } if stats.records == 0 && stats.aliases == u64::from(!remove_aliases))
        );
    }
    assert_eq!(
        owner.handle.credential("synthetic-controller-credential"),
        credential
    );
    owner
        .commands
        .send(Command::HistorySetAlias {
            credential,
            alias: None,
        })
        .unwrap();
    assert!(matches!(
        history_response(&events),
        HistoryResponse::AliasChanged { .. }
    ));
    owner.shutdown().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn history_policy_persists_checked_and_reports_partial_save_failure() {
    let directory = std::env::temp_dir().join(format!(
        "relay-controller-history-policy-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let mut session = session_at(directory.clone());
    let mut config = Config::default();
    config.history.retention_days = 30;
    config.history.max_records = 50;
    session.save(&config, false).unwrap();
    assert_eq!(
        session.history.as_ref().unwrap().options,
        Some(history_options(&config.history))
    );
    assert!(session.configure_history(20, 25, false).is_err());
    assert!(session.configure_history(91, 25, true).is_err());
    assert!(
        matches!(session.configure_history(20, 25, true).unwrap(), HistoryResponse::PolicyChanged { report, .. } if report.options.retention_days == 20)
    );
    let path = directory.join("config.toml");
    assert_eq!(
        config::load_config(&path).unwrap().history.retention_days,
        20
    );
    let saved = session.saved.clone().unwrap();
    let mut external = saved.clone();
    external.listen.port = 8181;
    config::save_config_atomic(&path, &external).unwrap();
    assert!(
        session
            .configure_history(10, 10, true)
            .unwrap_err()
            .contains("外部")
    );
    assert_eq!(
        session
            .history
            .as_ref()
            .unwrap()
            .options
            .unwrap()
            .retention_days,
        20
    );
    config::save_config_atomic(&path, &saved).unwrap();
    let backup = directory.join("config.toml.bak");
    std::fs::remove_file(&backup).unwrap();
    std::fs::create_dir(&backup).unwrap();
    let error = session.configure_history(10, 10, true).unwrap_err();
    assert!(error.contains("数据库清理已生效") && error.contains("部分完成"));
    assert_eq!(session.saved.as_ref(), Some(&saved));
    assert_eq!(session.baseline.as_ref(), Some(&saved));
    assert_eq!(
        session
            .history
            .as_ref()
            .unwrap()
            .options
            .unwrap()
            .retention_days,
        10
    );
    session.sync_history_policy().unwrap();
    assert_eq!(
        session
            .history
            .as_ref()
            .unwrap()
            .options
            .unwrap()
            .retention_days,
        20
    );
    std::fs::remove_dir(backup).unwrap();
    drop(session);
    let mut reloaded = session_at(directory.clone());
    reloaded
        .load(&Options {
            config_dir: Some(directory.clone()),
            ..Options::default()
        })
        .unwrap();
    assert_eq!(
        reloaded.history.as_ref().unwrap().options,
        Some(history_options(&saved.history))
    );
    let backup = directory.join("config.toml.bak");
    let mut restored = saved.clone();
    restored.history.retention_days = 15;
    config::save_config_atomic(&backup, &restored).unwrap();
    reloaded
        .config_command(Command::ConfigRestorePreview { backup })
        .unwrap();
    reloaded
        .config_command(Command::ConfigRestoreConfirm { confirmed: true })
        .unwrap();
    assert_eq!(reloaded.saved.as_ref().unwrap().history, restored.history);
    assert_eq!(
        reloaded.history.as_ref().unwrap().options,
        Some(history_options(&restored.history))
    );
    let target = directory.join("migrated-scope");
    std::fs::create_dir(&target).unwrap();
    config::save_config_atomic(&target.join("config.toml"), &restored).unwrap();
    reloaded.selection.as_mut().unwrap().directory = target;
    reloaded.ensure_history().unwrap();
    assert_eq!(
        reloaded.history.as_ref().unwrap().options,
        Some(history_options(&restored.history))
    );
    drop(reloaded);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn window_preferences_are_loaded_once_scoped_and_respect_saved_opt_out() {
    let root = std::env::temp_dir().join(format!("relay-controller-window-{}", std::process::id()));
    let first = root.join("first");
    let second = root.join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let mut session = session_at(first.clone());
    let events = ResultQueue::new(|_, count| WindowResponse::Error(overflow_message(count)));
    session.load_window(&events);
    assert!(events.try_recv().is_err());
    session.save(&Config::default(), false).unwrap();
    let geometry = preferences::WindowGeometry {
        left: 10,
        top: 20,
        width: 800,
        height: 900,
        dpi: 96,
        maximized: false,
    };
    session.save_window(first.clone(), geometry).unwrap();
    session.load_window(&events);
    assert!(
        matches!(events.try_recv(), Ok(WindowResponse::Loaded { directory, geometry: Some(actual) }) if directory == first && actual == geometry)
    );
    session.load_window(&events);
    assert!(events.try_recv().is_err());
    let mut off = session.saved.clone().unwrap();
    off.ui.remember_window_geometry = false;
    session.save(&off, false).unwrap();
    assert!(session.save_window(first.clone(), geometry).is_err());
    assert_eq!(
        preferences::load(&first.join(preferences::FILE_NAME)).unwrap(),
        Some(geometry)
    );
    session.window_loaded = None;
    session.load_window(&events);
    assert!(matches!(
        events.try_recv(),
        Ok(WindowResponse::Loaded { geometry: None, .. })
    ));
    session.selection.as_mut().unwrap().directory = second.clone();
    assert!(
        session
            .save_window(first.clone(), geometry)
            .unwrap_err()
            .contains("目录已改变")
    );
    session.load_window(&events);
    assert!(
        matches!(events.try_recv(), Ok(WindowResponse::Loaded { directory, geometry: None }) if directory == second)
    );
    assert!(!second.join(preferences::FILE_NAME).exists());
    for _ in 0..12 {
        events.publish(WindowResponse::Saved);
    }
    assert!(
        matches!(events.try_recv(), Ok(WindowResponse::Error(error)) if error.contains("丢失"))
    );
    assert!(session.history_events.try_recv().is_err());
    drop(session);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_settings_cannot_reapply_old_retention_after_independent_policy_change() {
    let directory =
        std::env::temp_dir().join(format!("relay-stale-history-draft-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let mut session = session_at(directory.clone());
    let mut stale = Config::default();
    stale.history.max_records = 1;
    session.save(&stale, false).unwrap();
    session.configure_history(90, 100_000, true).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let exit = AtomicBool::new(false);
    assert!(session.save(&stale, false).unwrap_err().contains("旧草稿"));
    assert!(
        session
            .save_and_apply(&runtime, &exit, &stale, false)
            .unwrap_err()
            .contains("旧草稿")
    );
    assert_eq!(session.saved.as_ref().unwrap().history.max_records, 100_000);
    assert_eq!(
        config::load_config(&directory.join("config.toml"))
            .unwrap()
            .history
            .max_records,
        100_000
    );
    assert_eq!(
        session
            .history
            .as_ref()
            .unwrap()
            .options
            .unwrap()
            .max_records,
        100_000
    );
    drop(session);
    runtime.shutdown_timeout(Duration::from_secs(1));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn window_load_transient_sharing_failure_can_be_retried() {
    use std::os::windows::fs::OpenOptionsExt;
    let directory = std::env::temp_dir().join(format!("relay-window-retry-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let mut session = session_at(directory.clone());
    session.save(&Config::default(), false).unwrap();
    let path = directory.join(preferences::FILE_NAME);
    let geometry = preferences::WindowGeometry {
        left: 10,
        top: 20,
        width: 800,
        height: 900,
        dpi: 96,
        maximized: false,
    };
    preferences::save(&path, &geometry).unwrap();
    let events = ResultQueue::new(|_, count| WindowResponse::Error(overflow_message(count)));
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    session.load_window(&events);
    assert!(matches!(events.try_recv(), Ok(WindowResponse::Error(_))));
    assert!(session.window_loaded.is_none());
    drop(lock);
    session.load_window(&events);
    assert!(
        matches!(events.try_recv(), Ok(WindowResponse::Loaded { geometry: Some(value), .. }) if value == geometry)
    );
    assert_eq!(session.window_loaded.as_ref(), Some(&directory));
    drop(session);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn history_errors_do_not_expose_backend_payloads() {
    let raw = "synthetic-secret raw response body";
    for error in [
        db::Error::Invalid(raw),
        db::Error::Io(std::io::Error::other(raw)),
        db::Error::Database(rusqlite::Error::InvalidParameterName(raw.into())),
    ] {
        assert!(!history_error(error).contains(raw));
    }
}

#[test]
fn overview_snapshots_are_bounded_typed_and_track_actual_listener_state() {
    let state = Arc::new(Mutex::new((None, None)));
    let metrics = ProxyMetrics::new();
    let stopped = overview_snapshot(&state, &metrics);
    assert!(!stopped.running && stopped.listen.is_none() && stopped.upstream.is_none());
    let address: SocketAddr = "127.0.0.1:12345".parse().unwrap();
    *state.lock().unwrap() = (Some(address), Some("https://upstream.invalid/v1".into()));
    let (sender, receiver) = mpsc::sync_channel(1);
    sender
        .try_send(overview_snapshot(&state, &metrics))
        .unwrap_or_else(|_| panic!("empty queue"));
    assert!(matches!(
        sender.try_send(overview_snapshot(&state, &metrics)),
        Err(TrySendError::Full(_))
    ));
    let running = receiver.try_recv().unwrap();
    assert!(running.running);
    assert_eq!(running.listen, Some(address));
    assert_eq!(
        running.upstream.as_deref(),
        Some("https://upstream.invalid/v1")
    );
    *state.lock().unwrap() = (None, None);
    let stopped = overview_snapshot(&state, &metrics);
    assert!(!stopped.running && stopped.listen.is_none());
    assert!(stopped.metrics.uptime >= running.metrics.uptime);
}

#[test]
fn log_snapshot_exposes_only_latest_thousand_typed_entries() {
    let directory = std::env::temp_dir().join(format!(
        "relay-controller-typed-logs-{}",
        std::process::id()
    ));
    let mut session = session_at(directory.clone());
    assert!(log_view(&session.log_source).entries.is_empty());
    session
        .ensure_log(&config::LoggingConfig {
            file_enabled: false,
            recent_entries: 1500,
            ..config::LoggingConfig::default()
        })
        .unwrap();
    let log = session.log.as_ref().unwrap();
    for _ in 0..1100 {
        let _ = log.handle().try_log(LogEvent::Runtime {
            level: config::LogLevel::Info,
            event: RuntimeEvent::ConfigurationApplied,
            error: None,
        });
        log.barrier().unwrap();
    }
    let snapshot = log_view(&session.log_source);
    assert_eq!(snapshot.entries.len(), 1000);
    assert_eq!(snapshot.text.lines().count(), 1000);
    assert!(
        snapshot
            .entries
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    assert_eq!(snapshot.entries.last().unwrap().sequence, snapshot.cursor);
    drop(session);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn full_result_queues_report_loss_and_leave_ipc_stop_quit_responsive() {
    let directory = std::env::temp_dir().join(format!(
        "ai-relay-controller-overflow-{}",
        std::process::id()
    ));
    let (ipc, receiver) = mpsc::sync_channel(4);
    let signals = Arc::new(Signals::default());
    let controller = Controller::spawn(
        Options {
            config_dir: Some(directory),
            ..Options::default()
        },
        receiver,
        Arc::clone(&signals),
    )
    .unwrap();
    // FIFO sentinel proves that the worker has processed all preceding GUI
    // commands, while neither result queue is consumed.
    let sentinel = |action| {
        let (reply, received) = mpsc::sync_channel(1);
        ipc.send(Control {
            request: super::super::instance::Request {
                version: 1,
                action,
                directory: None,
            },
            reply,
        })
        .unwrap();
        assert!(received.recv_timeout(Duration::from_secs(5)).unwrap().ok);
    };
    for expected in 1..=12 {
        controller.send(Command::Stop).unwrap();
        controller.send(Command::LogListFiles).unwrap();
        // Wait for both GUI commands, not merely for an IPC status request
        // which has priority over GUI commands.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let processed = {
                let (entries, dropped) = controller.log_events.pending_len();
                entries as u64 + dropped
            };
            if processed >= expected {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "worker stalled publishing results"
            );
            thread::sleep(Duration::from_millis(10));
        }
        sentinel(Action::Status);
    }
    sentinel(Action::Stop);
    sentinel(Action::Quit);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !controller.is_finished() && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(controller.is_finished());
    assert!(signals.quit.load(Ordering::Acquire));
    assert!(
        controller
            .events
            .try_recv()
            .unwrap()
            .message
            .contains("丢失")
    );
    assert!(matches!(controller.log_events.try_recv(),
            Ok(LogResponse::Error(message)) if message.contains("丢失")));
}

#[test]
fn portable_commit_marks_directory_even_when_logs_fail() {
    let directory = std::env::temp_dir().join(format!(
        "ai-relay-controller-portable-failure-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("logs"), b"not a directory").unwrap();
    let mut session = session_at(directory.clone());
    session.selection.as_mut().unwrap().source = DirectorySource::Portable;
    let mut config = Config::default();
    config.storage.mode = StorageMode::Portable;
    let error = session.save(&config, false).unwrap_err();
    assert!(error.contains("配置已保存"));
    assert!(error.contains("日志"));
    assert!(directory.join("portable.flag").is_file());
    assert_eq!(
        config::load_config(&directory.join("config.toml")).unwrap(),
        config
    );
    assert_eq!(session.baseline, Some(config.clone()));
    assert!(session.locked);
    assert!(!session.selection.as_ref().unwrap().needs_initialization);
    // A post-commit log failure must not leave a stale/missing baseline.
    config.listen.port = 8081;
    assert!(session.save(&config, false).unwrap_err().contains("日志"));
    assert_eq!(
        config::load_config(&directory.join("config.toml")).unwrap(),
        config
    );
    drop(session);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn loaded_session_rejects_external_config_until_reload() {
    let directory = std::env::temp_dir().join(format!(
        "ai-relay-controller-baseline-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("config.toml");
    let first = Config::default();
    config::save_config_atomic(&path, &first).unwrap();
    let mut session = session_at(directory.clone());
    let options = Options {
        config_dir: Some(directory.clone()),
        ..Options::default()
    };
    session.load(&options).unwrap();
    let mut external = first.clone();
    external.listen.port = 8081;
    config::save_config_atomic(&path, &external).unwrap();
    assert!(session.save(&first, false).unwrap_err().contains("外部"));
    assert_eq!(config::load_config(&path).unwrap(), external);
    session.load(&options).unwrap();
    session.save(&first, false).unwrap();
    drop(session);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn saved_start_restart_and_external_apply_guard() {
    let directory =
        std::env::temp_dir().join(format!("ai-relay-controller-saved-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut saved = Config::default();
    saved.listen.port = port;
    saved.upstream.url = "http://127.0.0.1:9".into();
    saved.proxy.auto_start = false;
    saved.network.shutdown_grace_secs = 45;
    config::save_config_atomic(&directory.join("config.toml"), &saved).unwrap();
    let mut session = session_at(directory.clone());
    session
        .load(&Options {
            config_dir: Some(directory.clone()),
            ..Options::default()
        })
        .unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let exit = AtomicBool::new(false);
    assert!(
        start(&runtime, &mut session, &exit)
            .unwrap()
            .contains("已保存")
    );
    assert_eq!(session.grace, 45);
    assert_eq!(session.handle.as_ref().unwrap().local_addr().port(), port);
    assert!(Arc::ptr_eq(
        &session.metrics,
        &session.handle.as_ref().unwrap().metrics()
    ));
    assert_eq!(
        session.overview.lock().unwrap().0,
        Some(session.handle.as_ref().unwrap().local_addr())
    );
    assert!(matches!(
        session.configure_history(30, 5000, true).unwrap(),
        HistoryResponse::PolicyChanged { .. }
    ));
    assert_eq!(session.handle.as_ref().unwrap().local_addr().port(), port);
    let saved = session.saved.clone().unwrap();
    let metrics = Arc::clone(&session.metrics);
    let history_id = session
        .history
        .as_ref()
        .unwrap()
        .handle
        .credential_id("synthetic-restart");
    stop(&runtime, &mut session).unwrap();
    assert_eq!(*session.overview.lock().unwrap(), (None, None));
    start(&runtime, &mut session, &exit).unwrap();
    assert!(Arc::ptr_eq(
        &metrics,
        &session.handle.as_ref().unwrap().metrics()
    ));
    assert_eq!(
        history_id,
        session
            .history
            .as_ref()
            .unwrap()
            .handle
            .credential_id("synthetic-restart")
    );
    let mut external = saved.clone();
    external.proxy.auto_start = true;
    config::save_config_atomic(&directory.join("config.toml"), &external).unwrap();
    assert!(
        session
            .save_and_apply(&runtime, &exit, &saved, false)
            .unwrap_err()
            .contains("外部")
    );
    assert!(session.handle.is_some());
    assert_eq!(
        config::load_config(&directory.join("config.toml")).unwrap(),
        external
    );
    stop(&runtime, &mut session).unwrap();
    drop(session);
    runtime.shutdown_timeout(Duration::from_secs(2));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn lan_start_requires_explicit_authorization_on_each_attempt() {
    let directory =
        std::env::temp_dir().join(format!("ai-relay-lan-consent-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut saved = Config::default();
    saved.listen.address = "0.0.0.0".parse().unwrap();
    saved.listen.port = port;
    saved.upstream.url = "http://127.0.0.1:9".into();
    saved.proxy.auto_start = false;
    config::save_config_atomic(&directory.join("config.toml"), &saved).unwrap();
    let mut session = session_at(directory.clone());
    session
        .load(&Options {
            config_dir: Some(directory.clone()),
            ..Options::default()
        })
        .unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let exit = AtomicBool::new(false);
    assert!(start(&runtime, &mut session, &exit).is_err());
    assert!(session.handle.is_none());
    start_authorized(&runtime, &mut session, &exit, true).unwrap();
    assert!(
        session
            .handle
            .as_ref()
            .unwrap()
            .local_addr()
            .ip()
            .is_unspecified()
    );
    stop(&runtime, &mut session).unwrap();
    assert!(start(&runtime, &mut session, &exit).is_err());
    drop(session);
    runtime.shutdown_timeout(Duration::from_secs(2));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unconfirmed_actions_do_not_touch_startup_or_files() {
    let directory = std::env::temp_dir().join(format!(
        "ai-relay-controller-confirm-{}",
        std::process::id()
    ));
    let mut session = session_at(directory.clone());
    assert!(
        session
            .autostart_command(Command::AutostartEnable { confirmed: false })
            .is_err()
    );
    assert!(
        session
            .config_command(Command::ConfigRestoreConfirm { confirmed: false })
            .is_err()
    );
    assert!(
        session
            .config_command(Command::ConfigMigrationConfirm { confirmed: true })
            .is_err()
    );
    assert!(!directory.exists());
}

#[test]
fn history_scope_survives_restart_and_log_failure() {
    let directory = std::env::temp_dir().join(format!(
        "ai-relay-history-controller-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("logs"), b"not directory").unwrap();
    let mut session = session_at(directory.clone());
    let config = Config::default();
    assert!(session.save(&config, false).unwrap_err().contains("日志"));
    let handle = session.history.as_ref().unwrap().handle.clone();
    let credential = handle.credential_id("synthetic-credential");
    session.ensure_history().unwrap();
    assert_eq!(
        credential,
        session
            .history
            .as_ref()
            .unwrap()
            .handle
            .credential_id("synthetic-credential")
    );
    session
        .history_command(Command::HistoryQuery(db::Query::default()))
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(response) = session.history_events.try_recv() {
            match response {
                HistoryResponse::QueryResult {
                    directory: scope,
                    report,
                    ..
                } => {
                    assert_eq!(scope, directory);
                    assert_eq!(report.summary.requests, 0);
                    break;
                }
                _ => panic!("unexpected history response"),
            }
        }
        assert!(std::time::Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let mut owner = session.history.take().unwrap();
    owner.shutdown().unwrap();
    drop(owner);
    let service = HistoryService::start(&directory).unwrap();
    assert_eq!(
        credential,
        service.handle().credential_id("synthetic-credential")
    );
    let export = directory.join("page.csv");
    assert!(
        export_history(
            &service,
            &directory,
            db::Query::default(),
            export.clone(),
            false
        )
        .is_err()
    );
    assert!(!export.exists());
    assert!(matches!(
        export_history(
            &service,
            &directory,
            db::Query::default(),
            export.clone(),
            true
        )
        .unwrap(),
        HistoryResponse::Exported {
            rows: 0,
            page_only: true,
            ..
        }
    ));
    let original = std::fs::read(&export).unwrap();
    assert!(
        export_history(
            &service,
            &directory,
            db::Query::default(),
            export.clone(),
            true
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&export).unwrap(), original);
    drop(service);
    drop(session);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn plaintext_requires_explicit_mode_and_confirmation() {
    let mut config = Config::default();
    config.keys.upstream_api_key = config::SecretString::new("test-secret");
    assert!(check_secrets(&config, false).is_ok());
    config.storage.secrets = SecretStorage::Plaintext;
    assert!(check_secrets(&config, false).is_err());
    assert!(check_secrets(&config, true).is_ok());
    config.storage.secrets = SecretStorage::Dpapi;
    config.keys.upstream_api_key = config::SecretString::default();
    config.access.client_token = config::SecretString::new("test-token");
    assert!(check_secrets(&config, false).is_ok());
}

fn session_at(directory: PathBuf) -> Session {
    Session {
        selection: Some(DirectorySelection {
            directory,
            source: DirectorySource::Explicit,
            needs_initialization: true,
        }),
        valid: true,
        baseline: None,
        saved: None,
        rollback: None,
        pending_import: None,
        pending_restore: None,
        pending_migration: None,
        stop_uncertain: false,
        locked: true,
        handle: None,
        usage: Arc::new(UsageStore::new()),
        metrics: Arc::new(ProxyMetrics::new()),
        overview: Arc::new(Mutex::new((None, None))),
        window_loaded: None,
        grace: 10,
        history: None,
        history_failure: None,
        history_events: ResultQueue::new(|latest, count| HistoryResponse::Error {
            directory: latest.directory(),
            message: overflow_message(count),
        }),
        log: None,
        log_directory: None,
        log_source: Arc::new(Mutex::new(None)),
        logging: config::LoggingConfig::default(),
        clear_preview: None,
    }
}

#[test]
fn unselected_first_run_never_creates_log_directory() {
    let directory =
        std::env::temp_dir().join(format!("ai-relay-unselected-{}", std::process::id()));
    let mut session = session_at(directory.clone());
    session.locked = false;
    session.prepare_start_log(&Config::default()).unwrap();
    assert!(session.log.is_none());
    assert!(!directory.exists());
    assert!(session.log_command(Command::LogListFiles).is_err());
}

#[test]
fn log_preview_apply_and_directory_switch_are_scoped() {
    let root = std::env::temp_dir().join(format!("ai-relay-controller-log-{}", std::process::id()));
    let first = root.join("first");
    let second = root.join("second");
    let mut session = session_at(first.clone());
    session
        .ensure_log(&config::LoggingConfig::default())
        .unwrap();
    session.log.as_ref().unwrap().barrier().unwrap();
    std::fs::write(first.join("logs/ai-relay.1.log"), b"history").unwrap();
    std::fs::write(first.join("logs/other.log"), b"keep").unwrap();
    assert!(
        session
            .log_command(Command::LogClearHistory { confirmed: true })
            .is_err()
    );
    assert!(matches!(
        session
            .log_command(Command::LogClearHistory { confirmed: false })
            .unwrap(),
        LogResponse::ClearPreview {
            all: false,
            files: 1,
            bytes: 7
        }
    ));
    assert!(
        session
            .log_command(Command::LogClearAll { confirmed: true })
            .is_err()
    );
    session
        .log_command(Command::LogClearHistory { confirmed: false })
        .unwrap();
    assert!(matches!(
        session
            .log_command(Command::LogClearHistory { confirmed: true })
            .unwrap(),
        LogResponse::Cleared {
            report: logging::ClearReport {
                deleted_files: 1,
                ..
            },
            ..
        }
    ));
    assert!(first.join("logs/other.log").exists());
    let options = config::LoggingConfig {
        file_enabled: false,
        ..config::LoggingConfig::default()
    };
    assert!(matches!(
        session
            .log_command(Command::LogApplyOptions {
                logging: options.clone()
            })
            .unwrap(),
        LogResponse::OptionsApplied {
            persisted: false,
            ..
        }
    ));
    assert_eq!(session.logging, options);
    assert!(!first.join("config.toml").exists());
    session.selection.as_mut().unwrap().directory = second.clone();
    session.ensure_log(&options).unwrap();
    assert_eq!(session.log_directory, Some(second.join("logs")));
    assert_eq!(
        log_view(&session.log_source).directory,
        Some(second.join("logs"))
    );
    session
        .log
        .as_mut()
        .unwrap()
        .shutdown(Duration::from_secs(2))
        .unwrap();
    drop(session);
    std::fs::remove_dir_all(root).unwrap();
}
