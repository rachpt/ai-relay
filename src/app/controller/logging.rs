//! 日志服务选项、错误文本和有界视图格式化；不访问 UI 或磁盘。
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::{
    config,
    logging::{self, LogHandle, LogOptions, LogStatus},
};

use super::types::{LogResponse, LogViewSnapshot};

pub(crate) type LogSource = Arc<Mutex<Option<(LogHandle, PathBuf, config::LoggingConfig)>>>;

pub(crate) fn log_options(config: &config::LoggingConfig) -> LogOptions {
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

pub(crate) fn log_error(error: logging::LogError) -> String {
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

pub(crate) fn log_read_response(name: String, response: logging::ReadResponse) -> LogResponse {
    match response {
        logging::ReadResponse::Changed(change) => LogResponse::FileChanged { name, change },
        logging::ReadResponse::Missing => LogResponse::FileMissing { name },
        response => LogResponse::Read { name, response },
    }
}

pub(crate) fn log_view(source: &LogSource) -> LogViewSnapshot {
    use std::fmt::Write;
    let source = source.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let Some((handle, directory, logging)) = source else {
        return LogViewSnapshot {
            text: "日志尚未初始化，未落盘；保存配置选定目录后启用。".into(),
            entries: Vec::new(),
            cursor: 0,
            gap: false,
            status: LogStatus::default(),
            directory: None,
            logging: config::LoggingConfig::default(),
        };
    };
    let snapshot = handle.snapshot(None);
    let mut text = String::new();
    let entries: Vec<_> = snapshot.entries.into_iter().rev().take(1000).collect();
    let entries: Vec<_> = entries.into_iter().rev().collect();
    // 文本和类型化视图都只保留最新 1000 条。
    for entry in &entries {
        let _ = writeln!(
            text,
            "{} #{} {:?}",
            entry.timestamp, entry.sequence, entry.event
        );
    }
    LogViewSnapshot {
        text,
        entries,
        cursor: snapshot.cursor,
        gap: snapshot.gap,
        status: snapshot.status,
        directory: Some(directory),
        logging,
    }
}
