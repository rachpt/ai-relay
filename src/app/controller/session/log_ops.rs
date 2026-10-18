//! 日志服务的初始化、配置同步和后台操作路由。
use std::time::Duration;

use crate::{
    config::{self, LogLevel},
    logging::{self, LogEvent, LogService, RuntimeEvent},
};

use super::super::{
    logging::{log_error, log_options, log_read_response},
    types::{Command, LogResponse},
    util::validation_error,
};
use super::Session;

impl Session {
    pub(crate) fn log_runtime(&self, event: RuntimeEvent, failed: bool) {
        if let Some(log) = &self.log {
            let _ = log.handle().try_log(LogEvent::Runtime {
                level: if failed {
                    LogLevel::Error
                } else {
                    LogLevel::Info
                },
                event,
                error: failed.then_some(logging::ErrorCode::Internal),
            });
        }
    }

    pub(crate) fn ensure_log(&mut self, options: &config::LoggingConfig) -> Result<(), String> {
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

    pub(crate) fn prepare_start_log(&mut self, config: &config::Config) -> Result<(), String> {
        if !self.locked {
            // 未保存且默认 standard 尚未被用户选定，不创建目录。
            self.logging = config.logging.clone();
            return Ok(());
        }
        self.ensure_log(&config.logging)
    }

    pub(crate) fn log_command(&mut self, command: Command) -> Result<LogResponse, String> {
        if let Command::LogApplyOptions { logging } = command {
            let check = config::Config {
                logging: logging.clone(),
                ..config::Config::default()
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
}
