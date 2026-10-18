//! UI 与控制线程之间的命令、响应和有界快照类型。
use std::{net::SocketAddr, path::PathBuf, sync::mpsc};

use crate::{
    config::{self, Config},
    history, logging,
    proxy::ProxyMetricsSnapshot,
    windows::{autostart, preferences},
};

pub(crate) enum Command {
    Load,
    #[allow(dead_code)] // 保留不启动代理的独立保存契约。
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
        mode: config::StorageMode,
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
    HistoryExportAll {
        query: history::Query,
        path: PathBuf,
        confirmed: bool,
    },
    HistorySetAlias {
        credential: history::CredentialId,
        alias: Option<history::CredentialAlias>,
    },
    HistoryStorage,
    HistoryConfigure {
        retention_days: u32,
        max_records: u32,
        confirmed: bool,
    },
    HistoryClear {
        confirmed: bool,
        remove_aliases: bool,
    },
    HistoryPolicy {
        options: history::Options,
        reply: mpsc::Sender<Result<history::PolicyReport, String>>,
    },
    SaveWindowGeometry {
        directory: PathBuf,
        geometry: preferences::WindowGeometry,
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
    #[allow(dead_code)] // 文件 UI 当前使用分页；保留有界尾读契约。
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

pub(crate) struct LogViewSnapshot {
    pub text: String,
    #[allow(dead_code)] // 日志 UI 的高级实时筛选在独立批次订阅此字段。
    pub entries: Vec<logging::LogEntry>,
    pub cursor: u64,
    pub gap: bool,
    pub status: logging::LogStatus,
    pub directory: Option<PathBuf>,
    pub logging: config::LoggingConfig,
}

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

pub(crate) enum AutostartResponse {
    State {
        status: autostart::Status,
        persisted: bool,
    },
    Error(String),
}
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

#[derive(Debug)]
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
    Storage {
        directory: PathBuf,
        stats: history::StorageStats,
    },
    AliasChanged {
        directory: PathBuf,
    },
    PolicyChanged {
        directory: PathBuf,
        report: history::PolicyReport,
    },
    ClearPreview {
        directory: PathBuf,
        records: u64,
        remove_aliases: bool,
    },
    Cleared {
        directory: PathBuf,
        remove_aliases: bool,
    },
    Error {
        directory: Option<PathBuf>,
        message: String,
    },
}

impl HistoryResponse {
    pub(crate) fn directory(&self) -> Option<PathBuf> {
        match self {
            Self::QueryResult { directory, .. }
            | Self::Exported { directory, .. }
            | Self::Counters { directory, .. }
            | Self::Storage { directory, .. }
            | Self::AliasChanged { directory }
            | Self::PolicyChanged { directory, .. }
            | Self::ClearPreview { directory, .. }
            | Self::Cleared { directory, .. } => Some(directory.clone()),
            Self::Error { directory, .. } => directory.clone(),
        }
    }
}

pub(crate) struct OverviewSnapshot {
    pub metrics: ProxyMetricsSnapshot,
    pub running: bool,
    pub listen: Option<SocketAddr>,
    pub upstream: Option<String>,
}

#[derive(Debug)]
pub(crate) enum WindowResponse {
    Loaded {
        directory: PathBuf,
        geometry: Option<preferences::WindowGeometry>,
    },
    Saved,
    Error(String),
}

pub(crate) struct Event {
    pub config: Option<Config>,
    pub directory: Option<PathBuf>,
    pub directory_locked: bool,
    pub running: bool,
    pub message: String,
}
