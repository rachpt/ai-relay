//! 后台控制线程主循环：消费 GUI/IPC 命令，排序启动快照任务并负责退出清理。
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    time::Duration,
};

use crate::{config, logging::RuntimeEvent, proxy::ProxyMetrics, usage::UsageStore};

use super::super::{
    Options,
    instance::{Action, Control, Reply, Signals},
};
use super::{
    logging::log_view,
    results::{ResultQueue, publish, publish_log},
    session::{Session, start, start_authorized, stop},
    types::{
        AutostartResponse, Command, ConfigResponse, Event, HistoryResponse, LogResponse,
        LogViewSnapshot, OverviewSnapshot, WindowResponse,
    },
    util::{open_directory, overview_snapshot, same_directory},
};

#[allow(clippy::too_many_arguments)] // 单个后台入口，包含独立 IPC 与 GUI 通道。
pub(crate) fn worker(
    mut options: Options,
    commands: Receiver<Command>,
    events: ResultQueue<Event>,
    usage_events: SyncSender<String>,
    exit: Arc<AtomicBool>,
    ipc: Receiver<Control>,
    signals: Arc<Signals>,
    logs: (SyncSender<LogViewSnapshot>, ResultQueue<LogResponse>),
    actions: (
        ResultQueue<AutostartResponse>,
        ResultQueue<ConfigResponse>,
        ResultQueue<HistoryResponse>,
    ),
    overview_events: SyncSender<OverviewSnapshot>,
    window_events: ResultQueue<WindowResponse>,
) {
    let (autostart_events, config_events, history_events) = actions;
    let (log_snapshots, log_events) = logs;
    let mut session = Session {
        selection: None,
        valid: false,
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
        overview: Arc::new(std::sync::Mutex::new((None, None))),
        window_loaded: None,
        grace: 10,
        history: None,
        history_failure: None,
        history_events: history_events.clone(),
        log: None,
        log_directory: None,
        log_source: Arc::new(std::sync::Mutex::new(None)),
        logging: config::LoggingConfig::default(),
        clear_preview: None,
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            publish(
                &events,
                session.event("后台网络运行时初始化失败，请重启程序。".into(), None),
                &exit,
            );
            return;
        }
    };
    let overview = Arc::clone(&session.overview);
    let metrics = Arc::clone(&session.metrics);
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let snapshot = overview_snapshot(&overview, &metrics);
            if matches!(
                overview_events.try_send(snapshot),
                Err(TrySendError::Disconnected(_))
            ) {
                break;
            }
        }
    });
    let log_source = Arc::clone(&session.log_source);
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut pending = None;
        loop {
            interval.tick().await;
            // GUI 未消费时重用已格式化的快照，不重复大量格式化。
            let view = pending.take().unwrap_or_else(|| log_view(&log_source));
            match log_snapshots.try_send(view) {
                Ok(()) => {}
                Err(TrySendError::Full(view)) => pending = Some(view),
                Err(TrySendError::Disconnected(_)) => break,
            }
        }
    });
    let usage = Arc::clone(&session.usage);
    // 独立有界通道：GUI 不消费时丢弃快照，不挤占命令结果或等待 GUI。
    // 此任务也在控制线程等待启动/停止时运行；锁与格式化均不进入界面线程。
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let text = crate::ui::usage::render(&usage.snapshot());
            if matches!(
                usage_events.try_send(text),
                Err(TrySendError::Disconnected(_))
            ) {
                break;
            }
        }
    });
    let mut next = Some(Command::Load);
    let mut ipc_stop = None;
    loop {
        if exit.load(Ordering::Acquire) {
            break;
        }
        if next.is_none()
            && let Ok(control) = ipc.try_recv()
        {
            let running = session.handle.is_some();
            let directory_matches = control.request.directory.as_ref().is_none_or(|requested| {
                session
                    .selection
                    .as_ref()
                    .is_some_and(|s| same_directory(&s.directory, &PathBuf::from(requested)))
            });
            if !directory_matches {
                let _ = control.reply.try_send(Reply::new(
                    false,
                    running,
                    "显式配置目录与主实例不同或尚未确定；请先退出原实例。",
                ));
                continue;
            }
            let reply = match control.request.action {
                Action::Show => {
                    signals.show.store(true, Ordering::Release);
                    Reply::new(true, running, "已请求显示主窗口。")
                }
                Action::Status => Reply::new(
                    !session.stop_uncertain,
                    running,
                    if session.stop_uncertain {
                        "上次停止等待失败，连接清理状态未确认；请退出并重新启动程序。"
                    } else if running {
                        "主实例运行中；代理正在监听。"
                    } else {
                        "主实例运行中；代理已停止。"
                    },
                ),
                Action::Quit => {
                    exit.store(true, Ordering::Release);
                    signals.quit.store(true, Ordering::Release);
                    Reply::new(true, running, "已接受优雅退出请求；不代表进程已退出。")
                }
                Action::Start | Action::Restart => {
                    next = Some(if control.request.action == Action::Restart {
                        Command::Restart
                    } else {
                        Command::Start(Box::default())
                    });
                    ipc_stop = Some(control.reply);
                    continue;
                }
                Action::Stop => {
                    ipc_stop = Some(control.reply);
                    next = Some(Command::Stop);
                    continue;
                }
            };
            let _ = control.reply.try_send(reply);
            continue;
        }
        let command = match next.take() {
            Some(command) => command,
            None => match commands.recv_timeout(Duration::from_millis(50)) {
                Ok(command) => command,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
        };
        if let Command::SaveWindowGeometry {
            directory,
            geometry,
        } = command
        {
            window_events.publish(
                session
                    .save_window(directory, geometry)
                    .unwrap_or_else(WindowResponse::Error),
            );
            continue;
        }
        if let Command::HistoryConfigure {
            retention_days,
            max_records,
            confirmed,
        } = command
        {
            let response = session.configure_history(retention_days, max_records, confirmed);
            let message = match &response {
                Ok(_) => {
                    "历史策略独立保存并应用；网络配置与界面草稿未重新加载，代理未停止。".to_string()
                }
                Err(error) => format!("历史策略独立操作失败：{error}"),
            };
            history_events.publish(response.unwrap_or_else(|message| {
                HistoryResponse::Error {
                    directory: session
                        .selection
                        .as_ref()
                        .map(|selection| selection.directory.clone()),
                    message,
                }
            }));
            publish(&events, session.event(message, None), &exit);
            continue;
        }
        if matches!(
            &command,
            Command::LogListFiles
                | Command::LogReadTail { .. }
                | Command::LogReadPage { .. }
                | Command::LogReadFollow { .. }
                | Command::LogClearHistory { .. }
                | Command::LogClearAll { .. }
                | Command::LogApplyOptions { .. }
        ) {
            let response = session
                .log_command(command)
                .unwrap_or_else(LogResponse::Error);
            publish_log(&log_events, response, &exit);
            continue;
        }
        if matches!(
            &command,
            Command::HistoryQuery(_)
                | Command::HistoryExport { .. }
                | Command::HistoryExportAll { .. }
                | Command::HistorySetAlias { .. }
                | Command::HistoryStorage
                | Command::HistoryClear { .. }
                | Command::HistoryCounters
        ) {
            if let Err(message) = session.history_command(command) {
                history_events.publish(HistoryResponse::Error {
                    directory: session.selection.as_ref().map(|s| s.directory.clone()),
                    message,
                });
            }
            continue;
        }
        let mut loaded = None;
        let result = match command {
            Command::OpenDataDirectory | Command::OpenLogDirectory => {
                let directory = if matches!(command, Command::OpenLogDirectory) {
                    session.log_directory.clone()
                } else {
                    session
                        .selection
                        .as_ref()
                        .filter(|_| session.locked)
                        .map(|s| s.directory.clone())
                };
                directory
                    .ok_or_else(|| "尚未选定或初始化目录；未打开。".to_string())
                    .and_then(|path| open_directory(&path))
            }
            Command::Load => session.load(&options).map(|config| {
                let message = if matches!(
                    options.command,
                    super::super::Command::Start | super::super::Command::Restart
                ) {
                    next = Some(Command::Start(Box::default()));
                    "配置已加载；正在按命令启动已保存配置……"
                } else if session.saved.is_none() {
                    "首次打开：尚无已保存配置；请先完成设置并保存。"
                } else if !config.proxy.auto_start {
                    "配置已加载；已关闭打开软件后自动启动代理，可手动启动。"
                } else if config.upstream.url.is_empty() {
                    "配置已加载；上游未配置，请先填写并保存有效配置。"
                } else if !config.listen.address.is_loopback() {
                    "配置已加载；非回环监听需要手动启动并确认风险，未自动启动。"
                } else {
                    next = Some(Command::Start(Box::default()));
                    "配置已加载；正在使用已保存配置自动启动代理……"
                };
                loaded = Some(config);
                message.to_string()
            }),
            Command::Save {
                config,
                plaintext_confirmed,
            } => session
                .save(&config, plaintext_confirmed)
                .map(|()| "设置已保存；代理已停止，点击启动应用已保存配置。".into()),
            Command::ConfirmLanStart {
                directory,
                config,
                plaintext_confirmed,
                apply,
            } => {
                let scoped = session
                    .selection
                    .as_ref()
                    .is_some_and(|s| same_directory(&s.directory, &directory));
                if !scoped {
                    Err("配置目录已改变；风险确认失效，请重新确认。".into())
                } else if let Err(error) = session.check_baseline() {
                    Err(error)
                } else if apply {
                    let result = session.save_and_apply_authorized(
                        &runtime,
                        &exit,
                        &config,
                        plaintext_confirmed,
                        true,
                    );
                    loaded = session.saved.clone();
                    result
                } else if session.saved.as_ref() != Some(config.as_ref()) {
                    Err("已保存配置已改变；风险确认失效，不使用草稿启动。".into())
                } else {
                    start_authorized(&runtime, &mut session, &exit, true)
                }
            }
            Command::Start(draft) => {
                drop(draft); // 兼容参数必须丢弃；只能启动已保存配置。
                start(&runtime, &mut session, &exit)
            }
            Command::Restart => {
                stop(&runtime, &mut session).and_then(|()| start(&runtime, &mut session, &exit))
            }
            Command::SaveAndApply {
                config,
                plaintext_confirmed,
            } => {
                let result = session.save_and_apply(&runtime, &exit, &config, plaintext_confirmed);
                loaded = session.saved.clone();
                result
            }
            Command::Rollback { confirmed } => {
                if !confirmed {
                    Err("回滚会覆盖当前已保存配置并重启；必须由用户明确确认。".into())
                } else if let Some(previous) = session.rollback.clone() {
                    let result = session.save_and_apply(&runtime, &exit, &previous, true);
                    if result.is_ok() {
                        session.rollback = None;
                    }
                    loaded = session.saved.clone();
                    result
                } else {
                    Err("没有可回滚的配置。".into())
                }
            }
            Command::AutostartStatus
            | Command::AutostartEnable { .. }
            | Command::AutostartDisable => {
                let result = session.autostart_command(command);
                let message = match &result {
                    Ok(_) => "登录自启操作完成；实际状态见独立自启事件。".to_string(),
                    Err(error) => format!("登录自启操作失败：{error}"),
                };
                autostart_events.publish(result.unwrap_or_else(AutostartResponse::Error));
                publish(
                    &events,
                    session.event(message, session.saved.clone()),
                    &exit,
                );
                continue;
            }
            Command::ConfigExport
            | Command::ConfigImportPreview { .. }
            | Command::ConfigImportConfirm { .. }
            | Command::ConfigRestorePreview { .. }
            | Command::ConfigRestoreConfirm { .. }
            | Command::ConfigMigrationPreview { .. }
            | Command::ConfigMigrationConfirm { .. } => {
                let migrating = matches!(
                    &command,
                    Command::ConfigMigrationConfirm { confirmed: true }
                );
                let response = session.config_command(command);
                if migrating && let Some(selection) = &session.selection {
                    options.config_dir = Some(selection.directory.clone());
                }
                let message = match &response {
                    Ok(_) => "配置操作完成；预览、导出或恢复详情见独立配置事件。".to_string(),
                    Err(error) => format!("配置操作失败：{error}"),
                };
                session.load_window(&window_events);
                config_events.publish(response.unwrap_or_else(ConfigResponse::Error));
                let config = migrating.then(|| session.saved.clone()).flatten();
                publish(&events, session.event(message, config), &exit);
                continue;
            }

            Command::Stop => {
                publish(&events, session.event("正在停止代理……".into(), None), &exit);
                stop(&runtime, &mut session).map(|()| {
                    session.log_runtime(RuntimeEvent::ListenerStopped, false);
                    "代理已停止。".into()
                })
            }
            _ => unreachable!("special commands are handled separately"),
        };
        session.load_window(&window_events);
        if let Some(reply) = ipc_stop.take() {
            let _ = reply.try_send(Reply::new(
                result.is_ok(),
                session.handle.is_some(),
                result
                    .as_ref()
                    .map(String::as_str)
                    .unwrap_or_else(|e| e.as_str()),
            ));
        }
        if result.is_err() {
            session.log_runtime(RuntimeEvent::OperationFailed, true);
            signals.show.store(true, Ordering::Release);
        }
        publish(
            &events,
            session.event(
                result.unwrap_or_else(|error| format!("操作失败：{error}")),
                loaded,
            ),
            &exit,
        );
    }
    // 不再处理队列中的保存/启动，避免退出后产生新的持久化或监听操作。
    let _ = stop(&runtime, &mut session);
    session.log_runtime(RuntimeEvent::Stopped, false);
    if let Some(log) = session.log.as_mut() {
        let _ = log.shutdown(Duration::from_secs(2));
    }
    if let Some(mut history) = session.history.take()
        && let Err(message) = history.shutdown()
    {
        history_events.publish(HistoryResponse::Error {
            directory: Some(history.directory.clone()),
            message,
        });
    }
    runtime.shutdown_timeout(Duration::from_secs(2));
}
