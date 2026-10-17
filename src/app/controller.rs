//! 有界 GUI 桥接；本线程独占 Runtime、配置目录与代理句柄。
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::Duration,
};

use super::{
    Options,
    instance::{Action, Control, Reply, Signals},
};
use crate::{
    config::{
        self, Config, ConfigError, DirectorySelection, DirectorySource, SecretStorage, StorageMode,
    },
    history::{self, HistoryHandle, HistoryService},
    logging::{self, LogEvent, LogHandle, LogOptions, LogService, LogStatus, RuntimeEvent},
    proxy::{self, ProxyError, ProxyHandle},
    usage::UsageStore,
    windows::{autostart, paths},
};

#[allow(dead_code)] // 日志命令由独立 UI 集成批次接入。
pub(crate) enum Command {
    Load,
    Save {
        config: Box<Config>,
        plaintext_confirmed: bool,
    },
    /// 兼容旧 UI；参数不参与启动，始终使用已保存会话。
    Start(Box<Config>),
    Restart,
    /// 仅由 GUI 风险确认产生，绑定当前目录和配置；授权只用于本次操作。
    ConfirmLanStart {
        directory: PathBuf,
        config: Box<Config>,
        plaintext_confirmed: bool,
        apply: bool,
    },
    SaveAndApply {
        config: Box<Config>,
        plaintext_confirmed: bool,
    },
    Rollback {
        confirmed: bool,
    },
    AutostartStatus,
    AutostartEnable {
        confirmed: bool,
    },
    AutostartDisable,
    ConfigExport,
    ConfigImportPreview {
        text: String,
    },
    ConfigImportConfirm {
        confirmed: bool,
        plaintext_confirmed: bool,
    },
    ConfigRestorePreview {
        backup: PathBuf,
    },
    ConfigRestoreConfirm {
        confirmed: bool,
    },
    ConfigMigrationPreview {
        mode: StorageMode,
    },
    ConfigMigrationConfirm {
        confirmed: bool,
    },
    OpenDataDirectory,
    OpenLogDirectory,
    HistoryQuery(history::Query),
    HistoryExport {
        query: history::Query,
        path: PathBuf,
        confirmed: bool,
    },
    HistoryCounters,
    HistorySnapshot {
        target: PathBuf,
        reply: mpsc::Sender<Result<(), String>>,
    },
    Stop,
    LogReadPage {
        name: String,
        cursor: logging::PageCursor,
    },
    LogReadFollow {
        name: String,
        cursor: logging::FileCursor,
    },
    LogListFiles,
    LogReadTail {
        name: String,
    },
    LogClearHistory {
        confirmed: bool,
    },
    LogClearAll {
        confirmed: bool,
    },
    /// 即时生效但不持久化；正常保存配置后才持久化。
    LogApplyOptions {
        logging: config::LoggingConfig,
    },
}

#[allow(dead_code)] // 独立日志 UI 的完整快照契约。
pub(crate) struct LogViewSnapshot {
    pub text: String,
    pub cursor: u64,
    pub gap: bool,
    pub status: LogStatus,
    pub directory: Option<PathBuf>,
    pub logging: config::LoggingConfig,
}

#[allow(dead_code)] // 独立日志 UI 的类型化响应契约。
pub(crate) enum LogResponse {
    Files(Vec<logging::LogFile>),
    Read {
        name: String,
        response: logging::ReadResponse,
    },
    FileChanged {
        name: String,
        change: logging::FileChange,
    },
    FileMissing {
        name: String,
    },
    Tail {
        name: String,
        tail: logging::TailResult,
    },
    ClearPreview {
        all: bool,
        files: usize,
        bytes: u64,
    },
    Cleared {
        all: bool,
        report: logging::ClearReport,
    },
    OptionsApplied {
        logging: config::LoggingConfig,
        persisted: bool,
    },
    Error(String),
}

#[allow(dead_code)]
pub(crate) enum AutostartResponse {
    State {
        status: autostart::Status,
        persisted: bool,
    },
    Error(String),
}
#[allow(dead_code)]
pub(crate) enum ConfigResponse {
    Export {
        text: String,
    },
    ImportPreview {
        config: Box<Config>,
    },
    Imported,
    RestorePreview {
        backup: PathBuf,
        target: PathBuf,
    },
    Restored {
        retained: Option<PathBuf>,
    },
    MigrationPreview {
        source: PathBuf,
        target: PathBuf,
        description: String,
    },
    Migrated {
        directory: PathBuf,
    },
    Error(String),
}

#[allow(dead_code)]
pub(crate) enum HistoryResponse {
    QueryResult {
        directory: PathBuf,
        query: history::Query,
        report: Box<history::QueryReport>,
        counters: history::Counters,
    },
    Exported {
        directory: PathBuf,
        path: PathBuf,
        rows: usize,
        page_only: bool,
        counters: history::Counters,
    },
    Counters {
        directory: PathBuf,
        counters: history::Counters,
    },
    Error {
        directory: Option<PathBuf>,
        message: String,
    },
}

type LogSource = Arc<Mutex<Option<(LogHandle, PathBuf, config::LoggingConfig)>>>;

pub(crate) struct Event {
    pub config: Option<Config>,
    pub directory: Option<PathBuf>,
    pub directory_locked: bool,
    pub running: bool,
    pub message: String,
}

pub(crate) struct Controller {
    commands: SyncSender<Command>,
    pub events: ResultQueue<Event>,
    pub usage_events: Receiver<String>,
    #[allow(dead_code)] // 等待日志 UI 订阅，不借用操作结果队列。
    pub log_snapshots: Receiver<LogViewSnapshot>,
    #[allow(dead_code)]
    pub log_events: ResultQueue<LogResponse>,
    #[allow(dead_code)] // UI 在独立批次订阅。
    pub autostart_events: ResultQueue<AutostartResponse>,
    #[allow(dead_code)]
    pub config_events: ResultQueue<ConfigResponse>,
    #[allow(dead_code)]
    pub history_events: ResultQueue<HistoryResponse>,
    exit: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}
impl Controller {
    pub fn spawn(
        options: Options,
        ipc: Receiver<Control>,
        signals: Arc<Signals>,
    ) -> Result<Self, String> {
        let (commands, receiver) = mpsc::sync_channel(4);
        let events = ResultQueue::new(event_overflow);
        let event_receiver = events.clone();
        let (usage_events, usage_receiver) = mpsc::sync_channel(1);
        let (log_snapshots, log_snapshot_receiver) = mpsc::sync_channel(1);
        let log_events = ResultQueue::new(|_, count| LogResponse::Error(overflow_message(count)));
        let log_event_receiver = log_events.clone();
        let autostart_events =
            ResultQueue::new(|_, count| AutostartResponse::Error(overflow_message(count)));
        let config_events =
            ResultQueue::new(|_, count| ConfigResponse::Error(overflow_message(count)));
        let history_events =
            ResultQueue::new(|latest: &HistoryResponse, count| HistoryResponse::Error {
                directory: latest.directory(),
                message: overflow_message(count),
            });
        let action_events = (
            autostart_events.clone(),
            config_events.clone(),
            history_events.clone(),
        );
        let exit = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let worker_exit = exit.clone();
        let worker_finished = finished.clone();
        thread::Builder::new()
            .name("ai-relay-controller".into())
            .spawn(move || {
                struct Finished(Arc<AtomicBool>);
                impl Drop for Finished {
                    fn drop(&mut self) {
                        self.0.store(true, Ordering::Release);
                    }
                }
                let _finished = Finished(worker_finished);
                worker(
                    options,
                    receiver,
                    events,
                    usage_events,
                    worker_exit,
                    ipc,
                    signals,
                    (log_snapshots, log_events),
                    action_events,
                );
            })
            .map_err(|_| "无法创建后台控制线程。".to_string())?;
        Ok(Self {
            commands,
            events: event_receiver,
            usage_events: usage_receiver,
            log_snapshots: log_snapshot_receiver,
            log_events: log_event_receiver,
            autostart_events,
            config_events,
            history_events,
            exit,
            finished,
        })
    }

    pub fn send(&self, command: Command) -> Result<(), String> {
        if self.exit.load(Ordering::Acquire) {
            return Err("程序正在退出。".into());
        }
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(_) => "后台命令队列已满，请稍后重试。".into(),
                TrySendError::Disconnected(_) => "后台线程已结束，请退出并重新启动程序。".into(),
            })
    }
    pub fn request_exit(&self) {
        self.exit.store(true, Ordering::Release);
    }
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.request_exit();
    }
}

// 固定容量操作结果邮箱。锁内只有有界内存操作，不等待消费者；溢出通知
// 与计数独立于队列，不能被后续结果挤掉。先通知缺口，再读取保留的结果。
pub(crate) struct ResultQueue<T> {
    state: Arc<Mutex<ResultState<T>>>,
    overflow: fn(&T, u64) -> T,
}
struct ResultState<T> {
    entries: VecDeque<T>,
    dropped: u64,
}
impl<T> Clone for ResultQueue<T> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            overflow: self.overflow,
        }
    }
}
impl<T> ResultQueue<T> {
    fn new(overflow: fn(&T, u64) -> T) -> Self {
        Self {
            state: Arc::new(Mutex::new(ResultState {
                entries: VecDeque::with_capacity(8),
                dropped: 0,
            })),
            overflow,
        }
    }
    fn publish(&self, value: T) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.entries.len() == 8 {
            state.entries.pop_front();
            state.dropped = state.dropped.saturating_add(1);
        }
        state.entries.push_back(value);
    }
    pub fn try_recv(&self) -> Result<T, mpsc::TryRecvError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.dropped != 0
            && let Some(latest) = state.entries.back()
        {
            let notice = (self.overflow)(latest, state.dropped);
            state.dropped = 0;
            return Ok(notice);
        }
        state.entries.pop_front().ok_or(mpsc::TryRecvError::Empty)
    }
}
fn overflow_message(count: u64) -> String {
    format!(
        "后台结果队列溢出，{count} 条操作结果已丢失；操作可能已执行，请重新加载配置或查询状态，勿直接重试保存或删除。"
    )
}
fn event_overflow(latest: &Event, count: u64) -> Event {
    Event {
        config: None,
        directory: latest.directory.clone(),
        directory_locked: latest.directory_locked,
        running: latest.running,
        message: overflow_message(count),
    }
}
fn publish(sender: &ResultQueue<Event>, event: Event, _exit: &AtomicBool) {
    sender.publish(event);
}

struct Session {
    selection: Option<DirectorySelection>,
    valid: bool,
    baseline: Option<Config>,
    saved: Option<Config>,
    rollback: Option<Config>,
    pending_import: Option<Config>,
    pending_restore: Option<(PathBuf, Option<config::RestoreBaseline>, Config)>,
    pending_migration: Option<super::migration::Plan>,
    stop_uncertain: bool,
    locked: bool,
    handle: Option<ProxyHandle>,
    usage: Arc<UsageStore>,
    grace: u64,
    history: Option<HistoryOwner>,
    history_failure: Option<String>,
    history_events: ResultQueue<HistoryResponse>,
    log: Option<LogService>,
    log_directory: Option<PathBuf>,
    log_source: LogSource,
    logging: config::LoggingConfig,
    clear_preview: Option<bool>,
}
impl HistoryResponse {
    fn directory(&self) -> Option<PathBuf> {
        match self {
            Self::QueryResult { directory, .. }
            | Self::Exported { directory, .. }
            | Self::Counters { directory, .. } => Some(directory.clone()),
            Self::Error { directory, .. } => directory.clone(),
        }
    }
}

// 独立控制线程拥有阻塞服务，查询扫描和导出均不占 controller 或网络 runtime。
struct HistoryOwner {
    directory: PathBuf,
    handle: HistoryHandle,
    commands: SyncSender<Command>,
    worker: Option<thread::JoinHandle<()>>,
    stopped: Receiver<Result<(), String>>,
    stop_requested: bool,
}
impl HistoryOwner {
    fn start(directory: PathBuf, events: ResultQueue<HistoryResponse>) -> Result<Self, String> {
        let (commands, receiver) = mpsc::sync_channel(4);
        let (ready, started) = mpsc::sync_channel(1);
        let (finished, stopped) = mpsc::sync_channel(1);
        let scope = directory.clone();
        let worker = thread::Builder::new()
            .name("ai-relay-history-control".into())
            .spawn(move || {
                let mut service = match HistoryService::start(&scope) {
                    Ok(service) => service,
                    Err(error) => {
                        let _ = ready.send(Err(history_error(error)));
                        return;
                    }
                };
                if ready.send(Ok(service.handle())).is_err() {
                    return;
                }
                while let Ok(command) = receiver.recv() {
                    let result = match command {
                        Command::HistoryQuery(query) => service
                            .query(query.clone())
                            .map(|report| HistoryResponse::QueryResult {
                                directory: scope.clone(),
                                query,
                                report: Box::new(report),
                                counters: service.handle().counters(),
                            })
                            .map_err(history_error),
                        Command::HistoryExport {
                            query,
                            path,
                            confirmed,
                        } => export_history(&service, &scope, query, path, confirmed),
                        Command::HistorySnapshot { target, reply } => {
                            let _ = reply.send(service.snapshot_to(target).map_err(history_error));
                            continue;
                        }
                        Command::HistoryCounters => Ok(HistoryResponse::Counters {
                            directory: scope.clone(),
                            counters: service.handle().counters(),
                        }),
                        Command::Stop => break,
                        _ => Err("无效历史命令。".into()),
                    };
                    events.publish(result.unwrap_or_else(|message| HistoryResponse::Error {
                        directory: Some(scope.clone()),
                        message,
                    }));
                }
                let _ = finished.send(service.shutdown().map_err(history_error));
            })
            .map_err(|_| "无法创建历史控制线程。".to_string())?;
        match started.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(handle)) => Ok(Self {
                directory,
                handle,
                commands,
                worker: Some(worker),
                stopped,
                stop_requested: false,
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("历史初始化超过 5 秒或线程失败；未关联历史服务。".into()),
        }
    }
    fn snapshot(&self, target: PathBuf) -> Result<(), String> {
        let (reply, result) = mpsc::channel();
        self.commands
            .try_send(Command::HistorySnapshot { target, reply })
            .map_err(|_| "历史队列繁忙，未创建迁移快照。".to_string())?;
        result.recv_timeout(Duration::from_secs(30)).map_err(|_| {
            "快照30秒内未确认；后台可能仍在执行，禁止发布或重试覆盖暂存目录。".to_string()
        })?
    }
    fn shutdown(&mut self) -> Result<(), String> {
        if self.worker.is_none() {
            return Ok(());
        }
        if !self.stop_requested {
            self.commands.try_send(Command::Stop).map_err(|_| {
                "历史队列繁忙或已关闭；尚未确认排空/checkpoint，不得复制数据库。".to_string()
            })?;
            self.stop_requested = true;
        }
        self.stopped
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| {
                "历史关闭未在 5 秒内确认；不得复制数据库，后台可能仍在排空。".to_string()
            })??;
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "历史控制线程异常。".to_string())?;
        }
        Ok(())
    }
}

impl Drop for HistoryOwner {
    fn drop(&mut self) {
        // 曾等待超时则不再次阻塞退出；断开 sender 后后台自行排空，绝不宣称已 checkpoint。
        if !self.stop_requested {
            let _ = self.shutdown();
        }
    }
}

fn history_error(error: history::Error) -> String {
    match error {
        history::Error::Io(error) => {
            format!("历史读写失败（{:?}）；请检查数据目录和权限。", error.kind())
        }
        history::Error::Database(_) => "历史数据库操作失败；未清空或重建数据库。".into(),
        history::Error::Busy => "历史队列已满，请稍后查询；未自动重试。".into(),
        history::Error::Closed | history::Error::Worker => "历史服务已关闭或线程失败。".into(),
        history::Error::Invalid(_) => "历史配置、查询或文件身份无效；未静默替换历史身份。".into(),
    }
}

fn export_history(
    service: &HistoryService,
    directory: &std::path::Path,
    query: history::Query,
    path: PathBuf,
    confirmed: bool,
) -> Result<HistoryResponse, String> {
    use std::io::Write;
    if !confirmed {
        return Err("导出历史必须明确确认路径及当前查询页范围；未写入。".into());
    }
    if !path.is_absolute() {
        return Err("历史导出必须使用绝对路径。".into());
    }
    // 先完成查询再创建文件；create_new 拒绝现有文件、链接及数据库覆盖。
    let report = service.query(query).map_err(history_error)?;
    let text = report.export_csv();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("无法新建导出文件（{:?}）；已有目标绝不覆盖。", e.kind()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| {
            format!(
                "导出写入失败（{:?}）；部分文件保留于 {}，未报告成功。",
                e.kind(),
                path.display()
            )
        })?;
    Ok(HistoryResponse::Exported {
        directory: directory.to_path_buf(),
        path,
        rows: report.details.len(),
        page_only: true,
        counters: service.handle().counters(),
    })
}

fn open_directory(path: &std::path::Path) -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    if !path.is_absolute() || !path.is_dir() {
        return Err("目录不是已存在的绝对目录；未请求打开。".into());
    }
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err("目录含无效字符；未请求打开。".into());
    }
    wide.push(0);
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    // SAFETY: 两个字符串在同步调用期间有效且 NUL 结尾；无参数、无 CMD、无句柄所有权转移。
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        Err(format!("系统拒绝打开目录（ShellExecute 状态 {result}）。"))
    } else {
        Ok(format!(
            "系统已接受打开目录请求：{}；不代表资源管理器窗口已显示。",
            path.display()
        ))
    }
}

impl Session {
    fn ensure_history(&mut self) -> Result<(), String> {
        if let Some(error) = &self.history_failure {
            return Err(format!(
                "历史初始化曾失败：{error}；为防止并行打开或重建身份，请重启程序后再试。"
            ));
        }
        if !self.valid {
            return Err("配置未成功加载；未初始化或查询历史。".into());
        }
        let selection = self.selection.as_ref().ok_or("尚未确定配置目录。")?;
        if !self.locked || selection.needs_initialization {
            return Err("请先保存配置选定目录，再初始化历史。".into());
        }
        if let Some(owner) = &self.history {
            if owner.stop_requested {
                return Err("历史正在关闭或关闭未确认；请重启程序，未重新打开数据库。".into());
            }
            if owner.directory == selection.directory {
                return Ok(());
            }
        }
        // 不因日志失败改写历史 scope；旧服务必须确认关闭后才能关联新目录。
        if let Some(owner) = self.history.as_mut() {
            owner.shutdown()?;
        }
        self.history = None;
        match HistoryOwner::start(selection.directory.clone(), self.history_events.clone()) {
            Ok(owner) => self.history = Some(owner),
            Err(error) => {
                self.history_failure = Some(error.clone());
                self.history_events.publish(HistoryResponse::Error {
                    directory: Some(selection.directory.clone()),
                    message: error.clone(),
                });
                return Err(error);
            }
        }
        Ok(())
    }
    fn history_command(&mut self, command: Command) -> Result<(), String> {
        self.ensure_history()?;
        let owner = self.history.as_ref().ok_or("历史服务尚未初始化。")?;
        owner
            .commands
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(_) => "历史控制队列已满，未执行操作。".into(),
                TrySendError::Disconnected(_) => "历史控制线程已结束。".into(),
            })
    }

    fn log_runtime(&self, event: RuntimeEvent, failed: bool) {
        if let Some(log) = &self.log {
            let _ = log.handle().try_log(LogEvent::Runtime {
                level: if failed {
                    config::LogLevel::Error
                } else {
                    config::LogLevel::Info
                },
                event,
                error: failed.then_some(logging::ErrorCode::Internal),
            });
        }
    }

    fn ensure_log(&mut self, options: &config::LoggingConfig) -> Result<(), String> {
        let directory = self
            .selection
            .as_ref()
            .ok_or("尚未确定配置目录。")?
            .directory
            .join("logs");
        if self.log_directory.as_ref() != Some(&directory) {
            *self.log_source.lock().unwrap_or_else(|e| e.into_inner()) = None;
            self.log_directory = None;
            if let Some(mut old) = self.log.take() {
                old.shutdown(Duration::from_secs(2)).map_err(log_error)?;
            }
            self.clear_preview = None;
            self.log =
                Some(LogService::start(&directory, log_options(options)).map_err(log_error)?);
            self.log_directory = Some(directory.clone());
            self.log_runtime(RuntimeEvent::Started, false);
        } else if let Some(log) = &self.log {
            // configure 超时可能已经生效，不自动回滚或重试。
            log.configure(log_options(options)).map_err(log_error)?;
        }
        self.logging = options.clone();
        if let Some(log) = &self.log {
            *self.log_source.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((log.handle(), directory, options.clone()));
        }
        Ok(())
    }

    fn prepare_start_log(&mut self, config: &Config) -> Result<(), String> {
        if !self.locked {
            // 未保存且默认 standard 尚未被用户选定，不创建目录。
            self.logging = config.logging.clone();
            return Ok(());
        }
        self.ensure_log(&config.logging)
    }

    fn log_command(&mut self, command: Command) -> Result<LogResponse, String> {
        if let Command::LogApplyOptions { logging } = command {
            let check = Config {
                logging: logging.clone(),
                ..Config::default()
            };
            check
                .validate_for_save()
                .map_err(|e| validation_error(e.field))?;
            if self.log.is_none() {
                return Err("日志尚未初始化，未落盘；请先保存配置选定目录。".into());
            }
            self.ensure_log(&logging)?;
            self.clear_preview = None;
            self.log_runtime(RuntimeEvent::ConfigurationApplied, false);
            return Ok(LogResponse::OptionsApplied {
                logging,
                persisted: false,
            });
        }
        let log = self
            .log
            .as_ref()
            .ok_or("日志尚未初始化，未落盘；请先保存配置选定目录。")?;
        match command {
            Command::LogListFiles => log.list_files().map(LogResponse::Files).map_err(log_error),
            Command::LogReadPage { name, cursor } => {
                let response = log
                    .read_page(&name, cursor, logging::ReadBounds::default())
                    .map_err(log_error)?;
                Ok(log_read_response(name, response))
            }
            Command::LogReadFollow { name, cursor } => {
                let response = log
                    .read_follow(&name, cursor, logging::ReadBounds::default())
                    .map_err(log_error)?;
                Ok(log_read_response(name, response))
            }
            Command::LogReadTail { name } => {
                let tail = log
                    .read_tail(&name, logging::MAX_TAIL_BYTES, logging::MAX_TAIL_LINES)
                    .map_err(log_error)?;
                Ok(LogResponse::Tail { name, tail })
            }
            Command::LogClearHistory { confirmed } | Command::LogClearAll { confirmed } => {
                let all = matches!(command, Command::LogClearAll { .. });
                if !confirmed {
                    let files = log.list_files().map_err(log_error)?;
                    let files: Vec<_> = files
                        .into_iter()
                        .filter(|f| all || f.name != logging::CURRENT_FILE)
                        .collect();
                    self.clear_preview = Some(all);
                    return Ok(LogResponse::ClearPreview {
                        all,
                        files: files.len(),
                        bytes: files.iter().map(|f| f.bytes).sum(),
                    });
                }
                if self.clear_preview.take() != Some(all) {
                    return Err("必须先请求相同范围的清理预览并由用户确认；未执行删除。".into());
                }
                // worker 执行时再次验证严格文件名、目录祖先和链接边界。
                let report = if all {
                    log.clear_all()
                } else {
                    log.clear_history()
                }
                .map_err(log_error)?;
                Ok(LogResponse::Cleared { all, report })
            }
            _ => Err("无效日志命令。".into()),
        }
    }

    fn event(&self, message: String, config: Option<Config>) -> Event {
        Event {
            config,
            directory: self.selection.as_ref().map(|s| s.directory.clone()),
            directory_locked: self.locked,
            running: self.handle.is_some(),
            message,
        }
    }

    fn load(&mut self, options: &Options) -> Result<Config, String> {
        if self.handle.is_some() {
            return Err("请先停止代理，再重新加载配置。".into());
        }
        self.valid = false;
        self.baseline = None;
        self.saved = None;
        self.pending_import = None;
        self.pending_restore = None;
        self.pending_migration = None;
        self.rollback = None;
        self.locked = true;
        let selection = paths::discover_directory(options.config_dir.as_deref())?;
        self.locked =
            !selection.needs_initialization || selection.source != DirectorySource::Standard;
        self.selection = Some(selection.clone());
        let mut config = if selection.needs_initialization {
            Config::default()
        } else {
            config::load_config(&selection.config_path()).map_err(|e| config_error(&e))?
        };
        // 必须记录磁盘原值，不能记录下面按定位来源修正后的配置。
        self.baseline = (!selection.needs_initialization).then(|| config.clone());
        // 定位来源而非文件字段决定实际目录模式；显式目录保留文件模式。
        match selection.source {
            DirectorySource::Portable => config.storage.mode = StorageMode::Portable,
            DirectorySource::Standard => config.storage.mode = StorageMode::Standard,
            DirectorySource::Explicit => {}
        }
        self.valid = true;
        self.saved = (!selection.needs_initialization).then(|| config.clone());
        self.logging = config.logging.clone();
        if !selection.needs_initialization {
            self.ensure_history()?;
            self.ensure_log(&config.logging)?;
            self.log_runtime(RuntimeEvent::ConfigurationApplied, false);
        }
        Ok(config)
    }

    fn save(&mut self, config: &Config, confirmed: bool) -> Result<(), String> {
        if !self.valid {
            return Err("配置未成功加载，禁止覆盖；请修复原文件后重新加载。".into());
        }
        if self.handle.is_some() || self.stop_uncertain {
            return Err("请先确认代理已完全停止，再保存设置；保存不会隐式重启。".into());
        }
        if self
            .saved
            .as_ref()
            .map_or(config.system.login_auto_start, |saved| {
                saved.system != config.system
            })
        {
            return Err("普通保存不得改变登录自启；请使用独立自启命令并确认。".into());
        }
        check_secrets(config, confirmed)?;
        config
            .validate_for_save()
            .map_err(|e| validation_error(e.field))?;
        let selection = self.selection.as_mut().ok_or("尚未确定配置目录。")?;
        if self.locked && selection.source != DirectorySource::Explicit {
            let mode = if selection.source == DirectorySource::Portable {
                StorageMode::Portable
            } else {
                StorageMode::Standard
            };
            if config.storage.mode != mode {
                return Err("已有配置不能通过普通保存切换目录；请停止代理并使用迁移预览。".into());
            }
        }
        let mut target = selection.clone();
        if !self.locked {
            target.directory = match config.storage.mode {
                StorageMode::Standard => paths::standard_data_directory()?,
                StorageMode::Portable => paths::portable_data_directory()?,
            };
            target.source = if config.storage.mode == StorageMode::Portable {
                DirectorySource::Portable
            } else {
                DirectorySource::Standard
            };
            // 不覆盖尚未加载的另一份配置（包括损坏文件/重解析点）。
            match std::fs::symlink_metadata(target.config_path()) {
                Ok(_) => {
                    return Err(
                        "所选目录已有配置，请通过明确目录重新启动并加载，或使用迁移预览。".into(),
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("无法检查所选目录中的配置文件。".into()),
            }
        }
        std::fs::create_dir_all(&target.directory).map_err(|_| "无法创建数据目录，请检查权限。")?;
        let result = config::save_config_atomic_checked(
            &target.config_path(),
            config,
            self.baseline.as_ref(),
        );
        let committed =
            result.is_ok() || matches!(result, Err(ConfigError::BackupPublicationFailed { .. }));
        if committed {
            self.baseline = Some(config.clone());
            self.saved = Some(config.clone());
            target.needs_initialization = false;
            *selection = target;
            self.locked = true;
            let portable = selection.source == DirectorySource::Portable;
            let directory = selection.directory.clone();
            let mut failures = Vec::new();
            if let Err(e) = &result {
                failures.push(config_error(e));
            }
            if portable {
                // 标记不覆盖已有内容；失败必须说明配置已提交。
                match std::fs::OpenOptions::new().write(true).create_new(true).open(directory.join("portable.flag")) {
                    Ok(_) => {},
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
                    Err(_) => failures.push("配置已保存，但便携标记创建失败；下次请使用 --config-dir 指定此目录，勿直接重试覆盖。".into()),
                }
            }
            if let Err(error) = self.ensure_history() {
                failures.push(format!("配置已保存；{error}"));
            }
            match self.ensure_log(&config.logging) {
                Ok(()) => self.log_runtime(RuntimeEvent::ConfigurationApplied, false),
                Err(e) => failures.push(format!("配置已保存；{e}")),
            }
            return if failures.is_empty() {
                Ok(())
            } else {
                Err(failures.join("\n"))
            };
        }
        result.map_err(|e| config_error(&e))
    }
}

impl Session {
    fn check_baseline(&self) -> Result<(), String> {
        let path = self
            .selection
            .as_ref()
            .ok_or("尚未确定配置目录。")?
            .config_path();
        let current = config::inspect_restore_target(&path).map_err(|e| config_error(&e))?;
        match (&self.baseline, current) {
            (None, None) => Ok(()),
            (Some(expected), Some(_))
                if config::load_config(&path).map_err(|e| config_error(&e))? == *expected =>
            {
                Ok(())
            }
            _ => Err(config_error(&ConfigError::ExternalModification)),
        }
    }

    fn save_and_apply(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        exit: &AtomicBool,
        config: &Config,
        confirmed: bool,
    ) -> Result<String, String> {
        self.save_and_apply_authorized(runtime, exit, config, confirmed, false)
    }

    fn save_and_apply_authorized(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        exit: &AtomicBool,
        config: &Config,
        confirmed: bool,
        allow_lan: bool,
    ) -> Result<String, String> {
        // 外部改动与输入校验必须在停止当前服务之前完成，实际保存再次检查 baseline。
        self.check_baseline()?;
        check_secrets(config, confirmed)?;
        config
            .validate_for_start()
            .map_err(|e| validation_error(e.field))?;
        if !config.listen.address.is_loopback() && !allow_lan {
            return Err("非回环监听需要本次操作的风险确认；未停止、未保存。".into());
        }
        if self
            .saved
            .as_ref()
            .map_or(config.system.login_auto_start, |saved| {
                saved.system != config.system
            })
        {
            return Err("保存应用不得改变登录自启；请单独操作。".into());
        }
        let previous = self.saved.clone();
        let was_running = self.handle.is_some();
        stop(runtime, self)?;
        if let Err(error) = self.save(config, confirmed) {
            if self.saved.as_ref() == Some(config) {
                self.rollback = previous;
                return Err(format!(
                    "代理已停止；配置已提交但附属操作失败：{error}。未自动回滚，可明确确认回滚。"
                ));
            }
            let recovery = if was_running {
                start(runtime, self, exit)
            } else {
                Ok("原代理原本已停止。".into())
            };
            return Err(format!(
                "保存失败：{error}；旧配置未被主动覆盖；恢复运行结果：{}",
                recovery.unwrap_or_else(|e| e)
            ));
        }
        self.rollback = previous;
        start_authorized(runtime, self, exit, allow_lan).map(|message| format!("配置已保存并应用；{message}"))
            .map_err(|error| format!("配置已保存，但启动失败：{error}。代理未运行；未自动回滚，可由用户明确确认 Rollback。"))
    }

    fn autostart_command(&mut self, command: Command) -> Result<AutostartResponse, String> {
        let directory = self
            .selection
            .as_ref()
            .ok_or("尚未确定配置目录。")?
            .directory
            .clone();
        let exe = std::env::current_exe().map_err(|_| "无法解析当前程序绝对路径。")?;
        if matches!(command, Command::AutostartStatus) {
            return autostart::status(&exe, &directory).map(|status| AutostartResponse::State {
                status,
                persisted: false,
            });
        }
        let enable = matches!(command, Command::AutostartEnable { .. });
        if enable && !matches!(command, Command::AutostartEnable { confirmed: true }) {
            return Err("启用或修复登录自启必须由用户显式确认；未修改 Startup。".into());
        }
        self.check_baseline()?;
        let mut config = self.saved.clone().ok_or("请先保存配置，再修改登录自启。")?;
        let status = if enable {
            autostart::enable(&exe, &directory)
        } else {
            autostart::disable(&exe, &directory)
        }?;
        config.system.login_auto_start = enable;
        // 自启设置独立于网络核心配置，保存时不停止监听，也不再次确认未改变的秘密。
        let path = self
            .selection
            .as_ref()
            .ok_or("尚未确定配置目录。")?
            .config_path();
        let result = config::save_config_atomic_checked(&path, &config, self.baseline.as_ref());
        if result.is_ok() || matches!(result, Err(ConfigError::BackupPublicationFailed { .. })) {
            self.baseline = Some(config.clone());
            self.saved = Some(config);
        }
        result.map_err(|e| format!("系统自启已变更为 {status:?}，但配置持久化未完整成功：{}；请查询实际状态，不要盲目重试。", config_error(&e)))?;
        Ok(AutostartResponse::State {
            status,
            persisted: true,
        })
    }

    fn config_command(&mut self, command: Command) -> Result<ConfigResponse, String> {
        match command {
            Command::ConfigExport => {
                let saved = self.saved.as_ref().ok_or("没有已保存配置可导出。")?;
                let text = config::export_redacted_config(saved).map_err(|e| config_error(&e))?;
                Ok(ConfigResponse::Export { text })
            }
            Command::ConfigImportPreview { text } => {
                self.pending_import = None;
                let config = config::import_config(&text).map_err(|e| config_error(&e))?;
                self.pending_import = Some(config.clone());
                // 预览仅显示脱敏配置；确认时使用后台保留的原始配置。
                let safe = config::export_redacted_config(&config)
                    .and_then(|text| config::import_config(&text))
                    .map_err(|e| config_error(&e))?;
                Ok(ConfigResponse::ImportPreview {
                    config: Box::new(safe),
                })
            }
            Command::ConfigImportConfirm {
                confirmed,
                plaintext_confirmed,
            } => {
                if !confirmed {
                    return Err("导入必须先预览并明确确认覆盖；未保存。".into());
                }
                let imported = self.pending_import.take().ok_or("没有待确认的导入预览。")?;
                if self
                    .saved
                    .as_ref()
                    .is_some_and(|old| old.system != imported.system)
                    || imported.system.login_auto_start
                {
                    return Err("导入不得变更系统自启；请单独显式操作登录自启。".into());
                }
                self.save(&imported, plaintext_confirmed)?;
                Ok(ConfigResponse::Imported)
            }
            Command::ConfigRestorePreview { backup } => {
                self.pending_restore = None;
                if self.handle.is_some() {
                    return Err("请先停止代理，再预览恢复。".into());
                }
                let target = self
                    .selection
                    .as_ref()
                    .ok_or("尚未确定配置目录。")?
                    .config_path();
                let baseline =
                    config::inspect_restore_target(&target).map_err(|e| config_error(&e))?;
                let source = config::load_config(&backup).map_err(|e| config_error(&e))?;
                if self
                    .saved
                    .as_ref()
                    .is_some_and(|old| old.storage.mode != source.storage.mode)
                {
                    return Err("恢复不能隐式迁移存储模式；未写入。".into());
                }
                self.pending_restore = Some((backup.clone(), baseline, source));
                Ok(ConfigResponse::RestorePreview { backup, target })
            }
            Command::ConfigRestoreConfirm { confirmed } => {
                if !confirmed {
                    return Err("恢复必须先预览源与目标并明确确认；未写入。".into());
                }
                if self.handle.is_some() {
                    return Err("请先停止代理再恢复。".into());
                }
                let (backup, baseline, source) = self
                    .pending_restore
                    .take()
                    .ok_or("没有待确认的恢复预览。")?;
                let target = self
                    .selection
                    .as_ref()
                    .ok_or("尚未确定配置目录。")?
                    .config_path();
                let retained =
                    config::restore_backup_checked(&target, &backup, baseline.as_ref(), &source)
                        .map_err(|e| config_error(&e))?;
                self.saved = Some(source.clone());
                self.baseline = Some(source);
                self.valid = true;
                self.locked = true;
                if let Some(selection) = self.selection.as_mut() {
                    selection.needs_initialization = false;
                }
                Ok(ConfigResponse::Restored { retained })
            }
            Command::ConfigMigrationPreview { mode } => {
                self.pending_migration = None;
                if self.handle.is_some() || self.stop_uncertain {
                    return Err("请先完全停止代理再迁移。".into());
                }
                self.check_baseline()?;
                let source = self
                    .selection
                    .as_ref()
                    .ok_or("尚未确定目录。")?
                    .directory
                    .clone();
                let target = match mode {
                    StorageMode::Standard => paths::standard_data_directory()?,
                    StorageMode::Portable => paths::portable_data_directory()?,
                };
                let saved = self.saved.as_ref().ok_or("请先保存配置。")?;
                let plan = super::migration::Plan::new(
                    source.clone(),
                    target.clone(),
                    paths::portable_data_directory()?,
                    saved,
                    mode,
                )?;
                let description = plan.description();
                self.pending_migration = Some(plan);
                Ok(ConfigResponse::MigrationPreview {
                    source,
                    target,
                    description,
                })
            }
            Command::ConfigMigrationConfirm { confirmed } => {
                if !confirmed {
                    self.pending_migration = None;
                    return Err("迁移已取消，未改动目录或自启。".into());
                }
                if self.handle.is_some() || self.stop_uncertain {
                    return Err("代理状态已改变；请停止后重新预览。".into());
                }
                self.check_baseline()?;
                let plan = self.pending_migration.take().ok_or("没有待确认迁移。")?;
                if self
                    .selection
                    .as_ref()
                    .is_none_or(|s| s.directory != plan.source)
                    || self.saved.as_ref().is_none_or(|saved| {
                        let mut expected = plan.config.clone();
                        expected.storage.mode = saved.storage.mode;
                        *saved != expected
                    })
                {
                    return Err("预览后源配置或目录已变更，迁移授权失效。".into());
                }
                let exe = std::env::current_exe().map_err(|_| "无法解析EXE路径。")?;
                let startup = autostart::status(&exe, &plan.source)?;
                if plan.config.system.login_auto_start && startup != autostart::Status::Enabled {
                    return Err("自启配置与系统实际状态不一致；请先手动修复或关闭后重新迁移。源服务和数据未改变。".into());
                }
                let update = startup == autostart::Status::Enabled;
                self.ensure_history()?;
                let stage = plan.staging_path()?;
                self.history
                    .as_ref()
                    .ok_or("历史服务不可用。")?
                    .snapshot(stage.clone())?;
                self.history
                    .as_mut()
                    .ok_or("历史服务不可用。")?
                    .shutdown()?;
                self.history = None;
                let result = plan.publish(&stage, |directory| {
                    if update {
                        autostart::enable(&exe, directory).map(|_| ())
                    } else {
                        Ok(())
                    }
                });
                if let Err(error) = result {
                    let _ = self.ensure_history();
                    return Err(error);
                }
                self.selection = Some(DirectorySelection {
                    directory: plan.target.clone(),
                    source: if plan.config.storage.mode == StorageMode::Portable {
                        DirectorySource::Portable
                    } else {
                        DirectorySource::Standard
                    },
                    needs_initialization: false,
                });
                self.saved = Some(plan.config.clone());
                self.baseline = Some(plan.config.clone());
                self.locked = true;
                self.valid = true;
                self.rollback = None;
                self.history_failure = None;
                self.pending_import = None;
                self.pending_restore = None;
                self.clear_preview = None;
                let history = self.ensure_history();
                let log = self.ensure_log(&plan.config.logging);
                if history.is_err() || log.is_err() {
                    return Err(format!(
                        "迁移已提交至 {}，源目录保留；目标服务初始化失败：{} {}。下次用 --config-dir 指定目标，不要重复迁移。",
                        plan.target.display(),
                        history.err().unwrap_or_default(),
                        log.err().unwrap_or_default()
                    ));
                }
                Ok(ConfigResponse::Migrated {
                    directory: plan.target,
                })
            }
            _ => Err("无效配置操作。".into()),
        }
    }
}

fn start(
    runtime: &tokio::runtime::Runtime,
    session: &mut Session,
    exit: &AtomicBool,
) -> Result<String, String> {
    start_authorized(runtime, session, exit, false)
}

fn start_authorized(
    runtime: &tokio::runtime::Runtime,
    session: &mut Session,
    exit: &AtomicBool,
    allow_lan: bool,
) -> Result<String, String> {
    if !session.valid {
        return Err("配置未成功加载，禁止启动。".into());
    }
    if session.stop_uncertain {
        return Err("上次停止未确认完成；为避免重复监听，请退出并重新启动程序。".into());
    }
    if session.handle.is_some() {
        return Ok("代理已在运行；未重启或应用界面草稿。".into());
    }
    let config = session
        .saved
        .clone()
        .ok_or("没有已保存配置；请先保存设置。")?;
    if !config.listen.address.is_loopback() && !allow_lan {
        return Err("非回环监听需要 UI 本次风险确认；CLI/自动启动未执行。".into());
    }
    config
        .validate_for_start()
        .map_err(|e| validation_error(e.field))?;
    session.ensure_history()?;
    session.prepare_start_log(&config)?;
    session.grace = u64::from(config.network.shutdown_grace_secs);
    let handle = runtime.block_on(async {
        tokio::select! {
            result = proxy::start_with_telemetry(config, Arc::clone(&session.usage), session.log.as_ref().map(LogService::handle), session.history.as_ref().map(|owner| owner.handle.clone())) => result.map_err(|e| proxy_error(&e)),
            _ = async { while !exit.load(Ordering::Acquire) { tokio::time::sleep(Duration::from_millis(50)).await; } } => Err("程序退出，已取消启动。".into()),
            _ = tokio::time::sleep(Duration::from_secs(30)) => Err("代理初始化超过 30 秒，已取消启动。".into()),
        }
    })?;
    let message = format!(
        "代理运行中；实际监听 {}，使用已保存配置。",
        handle.local_addr()
    );
    session.handle = Some(handle);
    session.log_runtime(RuntimeEvent::ListenerStarted, false);
    Ok(message)
}

fn log_read_response(name: String, response: logging::ReadResponse) -> LogResponse {
    match response {
        logging::ReadResponse::Changed(change) => LogResponse::FileChanged { name, change },
        logging::ReadResponse::Missing => LogResponse::FileMissing { name },
        response => LogResponse::Read { name, response },
    }
}

fn check_secrets(config: &Config, confirmed: bool) -> Result<(), String> {
    let has_secrets =
        !config.keys.upstream_api_key.is_empty() || !config.access.client_token.is_empty();
    if has_secrets && config.storage.secrets == SecretStorage::Plaintext && !confirmed {
        return Err("保存非空密钥必须再次确认明文泄露风险。".into());
    }
    Ok(())
}

fn stop(runtime: &tokio::runtime::Runtime, session: &mut Session) -> Result<(), String> {
    let Some(handle) = session.handle.take() else {
        return if session.stop_uncertain {
            Err("上次停止未确认完成；请退出并重新启动程序，不能报告已完全停止。".into())
        } else {
            Ok(())
        };
    };
    let result = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(session.grace.saturating_add(2)),
            handle.shutdown(),
        )
        .await
        .map_err(|_| "停止等待超时，连接将在运行时退出时强制释放。".to_string())?
        .map_err(|e| proxy_error(&e))
    });
    session.stop_uncertain = result.is_err();
    result
}

#[allow(clippy::too_many_arguments)] // 单个后台入口，包含独立 IPC 与 GUI 通道。
fn worker(
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
        grace: 10,
        history: None,
        history_failure: None,
        history_events: history_events.clone(),
        log: None,
        log_directory: None,
        log_source: Arc::new(Mutex::new(None)),
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
            Command::HistoryQuery(_) | Command::HistoryExport { .. } | Command::HistoryCounters
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
                if matches!(
                    options.command,
                    super::Command::Start | super::Command::Restart
                ) || (session.saved.is_some()
                    && config.proxy.auto_start
                    && !config.upstream.url.is_empty()
                    && config.listen.address.is_loopback())
                {
                    next = Some(Command::Start(Box::default()));
                }
                loaded = Some(config);
                "配置已加载；代理已停止。非回环监听需点击启动并确认风险。".to_string()
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
            Command::Start(_) => start(&runtime, &mut session, &exit),
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

fn log_options(config: &config::LoggingConfig) -> LogOptions {
    LogOptions {
        level: config.level,
        file_enabled: config.file_enabled,
        access_enabled: config.access_log,
        max_file_bytes: config.max_file_bytes,
        retained_files: config.retained_files,
        retention_days: Some(config.retention_days),
        recent_entries: config.recent_entries as usize,
        ..LogOptions::default()
    }
}

fn log_error(error: logging::LogError) -> String {
    match error {
        logging::LogError::Timeout => {
            "日志操作等待超时；已排队操作可能继续执行，并非取消。请查询状态，不要自动重试。".into()
        }
        logging::LogError::Io(e) => {
            format!("日志读写失败（{:?}）；请检查日志目录和权限。", e.kind())
        }
        other => format!("日志操作失败（{other:?}）；未自动重试。"),
    }
}

fn publish_log(sender: &ResultQueue<LogResponse>, response: LogResponse, _exit: &AtomicBool) {
    sender.publish(response);
}

fn log_view(source: &LogSource) -> LogViewSnapshot {
    use std::fmt::Write;
    let source = source.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let Some((handle, directory, logging)) = source else {
        return LogViewSnapshot {
            text: "日志尚未初始化，未落盘；保存配置选定目录后启用。".into(),
            cursor: 0,
            gap: false,
            status: LogStatus::default(),
            directory: None,
            logging: config::LoggingConfig::default(),
        };
    };
    let snapshot = handle.snapshot(None);
    let mut text = String::new();
    // 文本额外限制为最新 1000 条；完整缓存仍遵循 recent_entries。
    for entry in snapshot
        .entries
        .iter()
        .skip(snapshot.entries.len().saturating_sub(1000))
    {
        let _ = writeln!(
            text,
            "{} #{} {:?}",
            entry.timestamp, entry.sequence, entry.event
        );
    }
    LogViewSnapshot {
        text,
        cursor: snapshot.cursor,
        gap: snapshot.gap,
        status: snapshot.status,
        directory: Some(directory),
        logging,
    }
}

fn same_directory(left: &std::path::Path, right: &std::path::Path) -> bool {
    // Canonicalization runs only on controller, never GUI. Fail closed for missing
    // paths except identical absolute spelling (normal first-run directory).
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(a), Ok(b)) => a == b,
        _ => left == right,
    }
}

fn validation_error(field: &str) -> String {
    format!("配置校验失败（字段 {field}）；请检查 URL、端口、密钥及注入策略。")
}
fn config_error(error: &ConfigError) -> String {
    match error {
        ConfigError::InvalidConfig(e) if e.field == "protected_secrets" => "DPAPI 密钥无法解密或密文格式无效；请使用原 Windows 用户，或重新输入密钥。未降级为明文。".into(),
        ConfigError::InvalidConfig(e) => validation_error(e.field),
        ConfigError::DpapiNotImplemented => "当前平台无法使用 Windows DPAPI，未降级为明文。".into(),
        ConfigError::BackupPublicationFailed { recovery_path, .. } => format!(
            "配置已保存，但备份发布失败；旧配置保留于 {}，请检查后再操作。",
            recovery_path.display()
        ),
        ConfigError::AtomicReplacementFailed {
            replacement_path,
            recovery_path,
            ..
        } => format!(
            "原子替换失败，请检查保留文件 {} 和 {}；不要直接覆盖。",
            replacement_path.display(),
            recovery_path.display()
        ),
        ConfigError::InvalidToml | ConfigError::InvalidUtf8 => {
            "配置格式或版本不受支持；原文件不会被覆盖，请手工修复或从备份恢复。".into()
        }
        ConfigError::TooLarge => "配置超过 1 MiB 上限。".into(),
        ConfigError::AbsolutePathRequired => "配置路径必须为绝对路径。".into(),
        ConfigError::Serialization => "配置序列化失败，未保存。".into(),
        ConfigError::ExternalModification => {
            "配置文件在加载后已被外部修改、创建或删除；未保存，请重新加载并确认修改。".into()
        }
        ConfigError::Io { kind, .. } => {
            format!("配置读写失败（{kind:?}），请检查显示的数据目录及权限。")
        }
    }
}
fn proxy_error(error: &ProxyError) -> String {
    match error {
        ProxyError::InvalidConfig(e) => validation_error(e.field),
        ProxyError::UnsupportedFeature => "请求的代理扩展尚不受支持，请检查配置。".into(),
        ProxyError::InvalidUpstream => "上游 URL 无效。".into(),
        ProxyError::InvalidCredential => "密钥无法用于 HTTP 认证头，请检查密钥格式。".into(),
        ProxyError::TlsRoots => "无法加载系统 TLS 信任根，未关闭证书校验。".into(),
        ProxyError::Bind => "无法监听指定 IP 和端口，请检查地址、端口占用及权限。".into(),
        ProxyError::Listener | ProxyError::Task => "代理监听或后台任务异常，请停止后重试。".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                    let state = controller.log_events.state.lock().unwrap();
                    state.entries.len() as u64 + state.dropped
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
        let history_id = session
            .history
            .as_ref()
            .unwrap()
            .handle
            .credential_id("synthetic-restart");
        stop(&runtime, &mut session).unwrap();
        start(&runtime, &mut session, &exit).unwrap();
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
            .history_command(Command::HistoryQuery(history::Query::default()))
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
                history::Query::default(),
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
                history::Query::default(),
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
                history::Query::default(),
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
        let root =
            std::env::temp_dir().join(format!("ai-relay-controller-log-{}", std::process::id()));
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
}
