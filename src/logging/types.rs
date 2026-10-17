use std::{io, net::IpAddr};

pub use crate::config::LogLevel;
use serde::Serialize;

pub const MAX_TAIL_BYTES: usize = 256 * 1024;
pub const MAX_TAIL_LINES: usize = 2000;
pub const CURRENT_FILE: &str = "ai-relay.current.log";

#[derive(Debug, Clone)]
pub struct LogOptions {
    pub level: LogLevel,
    pub file_enabled: bool,
    pub access_enabled: bool,
    pub max_file_bytes: u64,
    /// 包括当前文件。
    pub retained_files: u32,
    /// None 禁用按时间清理；Some(0) 无效。
    pub retention_days: Option<u32>,
    pub recent_entries: usize,
    pub queue_capacity: usize,
}
impl Default for LogOptions {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            file_enabled: true,
            access_enabled: true,
            max_file_bytes: 5 * 1024 * 1024,
            retained_files: 3,
            retention_days: Some(7),
            recent_entries: 1000,
            queue_capacity: 1024,
        }
    }
}
impl LogOptions {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.max_file_bytes < 1024
            || self.max_file_bytes > 1024 * 1024 * 1024
            || !(1..=100).contains(&self.retained_files)
            || !(1..=100_000).contains(&self.recent_entries)
            || !(1..=65_536).contains(&self.queue_capacity)
            || self.retention_days == Some(0)
        {
            return Err(LogError::InvalidOptions);
        }
        Ok(())
    }
}

macro_rules! labels {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
    };
}
labels!(RuntimeEvent {
    Started,
    Stopped,
    ConfigurationApplied,
    ListenerStarted,
    ListenerStopped,
    OperationFailed
});
labels!(ErrorCode {
    Io,
    PermissionDenied,
    DiskFull,
    Timeout,
    Cancelled,
    Connection,
    Tls,
    Protocol,
    InvalidConfiguration,
    ResourceLimit,
    Internal
});
labels!(Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
    Connect,
    Other
});
labels!(Endpoint {
    ChatCompletions,
    Responses,
    Messages,
    Models,
    Embeddings,
    Health,
    Redacted
});
impl Endpoint {
    /// 仅匹配完整白名单路径；query、URL、动态路径均不保留。
    pub fn from_path(path: &str) -> Self {
        match path {
            "/v1/chat/completions" => Self::ChatCompletions,
            "/v1/responses" => Self::Responses,
            "/v1/messages" => Self::Messages,
            "/v1/models" => Self::Models,
            "/v1/embeddings" => Self::Embeddings,
            "/health" => Self::Health,
            _ => Self::Redacted,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct AccessMetadata {
    pub request_id: Option<u64>,
    pub ip: Option<IpAddr>,
    pub method: Option<Method>,
    pub endpoint: Option<Endpoint>,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
    pub first_byte_ms: Option<u64>,
    pub error: Option<ErrorCode>,
}
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "category", rename_all = "snake_case")]
pub enum LogEvent {
    Runtime {
        level: LogLevel,
        event: RuntimeEvent,
        error: Option<ErrorCode>,
    },
    Access {
        level: LogLevel,
        metadata: AccessMetadata,
    },
}
impl LogEvent {
    pub fn level(&self) -> LogLevel {
        match self {
            Self::Runtime { level, .. } | Self::Access { level, .. } => *level,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub sequence: u64,
    pub timestamp: u64,
    pub event: LogEvent,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryLogResult {
    Queued,
    Full,
    Closed,
}
#[derive(Debug, Clone, Default)]
pub struct LogStatus {
    pub dropped: u64,
    pub disk_errors: u64,
    pub last_disk_error: Option<io::ErrorKind>,
    pub last_persisted_sequence: Option<u64>,
    pub worker_stopped: bool,
}
#[derive(Debug, Clone)]
pub struct LogSnapshot {
    pub entries: Vec<LogEntry>,
    /// 将此值作为下一次 after_sequence；清空视图只需保存此 cursor。
    pub cursor: u64,
    pub gap: bool,
    pub status: LogStatus,
}
#[derive(Debug, Clone)]
pub struct LogFile {
    pub name: String,
    pub bytes: u64,
    pub modified_epoch: Option<u64>,
}
#[derive(Debug, Clone)]
pub struct TailResult {
    pub text: String,
    pub file_bytes: u64,
    pub truncated: bool,
}
/// 单次磁盘读取及解码后文本的上限；与旧 tail 使用相同硬限制。
#[derive(Debug, Clone, Copy)]
pub struct ReadBounds {
    pub max_bytes: usize,
    pub max_lines: usize,
}
impl ReadBounds {
    pub(crate) fn validate(self) -> Result<()> {
        if !(1..=MAX_TAIL_BYTES).contains(&self.max_bytes)
            || !(1..=MAX_TAIL_LINES).contains(&self.max_lines)
        {
            return Err(LogError::InvalidTailBounds);
        }
        Ok(())
    }
}
impl Default for ReadBounds {
    fn default() -> Self {
        Self {
            max_bytes: MAX_TAIL_BYTES,
            max_lines: MAX_TAIL_LINES,
        }
    }
}

/// 只能由读取结果产生；不包含调用方可控制的路径或偏移。
#[derive(Debug, Clone)]
pub struct FileCursor {
    pub(crate) index: u32,
    pub(crate) identity: [u64; 3],
    pub(crate) offset: u64,
    pub(crate) observed_len: u64,
    pub(crate) partial_line: bool,
}
#[derive(Debug, Clone)]
pub enum PageCursor {
    Tail,
    Before(FileCursor),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChange {
    Replaced,
    Truncated,
}
#[derive(Debug, Clone)]
pub enum ReadResponse {
    Page(LogPage),
    /// 不自动切换文件或重置偏移；调用方可重新请求 Tail。
    Changed(FileChange),
    Missing,
}
#[derive(Debug, Clone)]
pub struct LogPage {
    pub text: String,
    pub file_bytes: u64,
    /// 历史分页使用；None 表示已到文件开头。
    pub before: Option<FileCursor>,
    /// 从本页结束处跟随；历史页不会跳至文件末尾。
    pub follow: FileCursor,
    /// 页面之前/之后是否还有磁盘字节（不表示内容损失）。
    pub has_more: bool,
    /// 含不完整行片段，或有损解码膨胀导致输出被裁剪。
    /// 游标按原始字节推进；输出裁剪的内容不会在后续页补发。
    pub truncated: bool,
    pub utf8_loss: bool,
}

#[derive(Debug, Clone)]
pub struct FileFailure {
    pub name: String,
    pub kind: io::ErrorKind,
}
#[derive(Debug, Clone, Default)]
pub struct ClearReport {
    pub deleted_files: u32,
    pub deleted_bytes: u64,
    pub errors: Vec<FileFailure>,
}
#[derive(Debug)]
pub enum LogError {
    Io(io::Error),
    InvalidDirectory,
    UnsafePath,
    InvalidName,
    InvalidOptions,
    InvalidTailBounds,
    InvalidCursor,
    WrongThread,
    QueueFull,
    Closed,
    Timeout,
}
impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for LogError {}
impl From<io::Error> for LogError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
pub type Result<T> = std::result::Result<T, LogError>;

pub(crate) fn enabled(event: LogEvent, options: &LogOptions) -> bool {
    fn rank(level: LogLevel) -> u8 {
        match level {
            LogLevel::Trace => 0,
            LogLevel::Debug => 1,
            LogLevel::Info => 2,
            LogLevel::Warn => 3,
            LogLevel::Error => 4,
            LogLevel::Off => 5,
        }
    }
    options.level != LogLevel::Off
        && event.level() != LogLevel::Off
        && rank(event.level()) >= rank(options.level)
        && (options.access_enabled || !matches!(event, LogEvent::Access { .. }))
}
