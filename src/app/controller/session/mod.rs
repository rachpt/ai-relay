//! 后台会话状态：配置生命周期、代理启停和窗口偏好。
//!
//! `Session` 的状态所有权始终留在控制线程；本模块只把纯逻辑子域拆到
//! `history`、`config` 和 `logs` 子模块，不引入新的线程或锁。
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::{
    config::{
        self, Config, ConfigError, DirectorySelection, DirectorySource, SecretStorage, StorageMode,
    },
    logging::{LogService, RuntimeEvent},
    proxy::{self, ProxyHandle, ProxyMetrics},
    usage::UsageStore,
    windows::{paths, preferences},
};

use super::{
    history::HistoryOwner,
    logging::LogSource,
    results::ResultQueue,
    types::{Event, HistoryResponse, WindowResponse},
    util::{OverviewState, config_error, proxy_error, same_directory, validation_error},
};

mod config_ops;
mod history_ops;
mod log_ops;

pub(super) struct Session {
    pub(super) selection: Option<DirectorySelection>,
    pub(super) valid: bool,
    pub(super) baseline: Option<Config>,
    pub(super) saved: Option<Config>,
    pub(super) rollback: Option<Config>,
    pub(super) pending_import: Option<Config>,
    pub(super) pending_restore: Option<(PathBuf, Option<config::RestoreBaseline>, Config)>,
    pub(super) pending_migration: Option<crate::app::migration::Plan>,
    pub(super) stop_uncertain: bool,
    pub(super) locked: bool,
    pub(super) handle: Option<ProxyHandle>,
    pub(super) usage: Arc<UsageStore>,
    pub(super) metrics: Arc<ProxyMetrics>,
    pub(super) overview: OverviewState,
    pub(super) window_loaded: Option<PathBuf>,
    pub(super) grace: u64,
    pub(super) history: Option<HistoryOwner>,
    pub(super) history_failure: Option<String>,
    pub(super) history_events: ResultQueue<HistoryResponse>,
    pub(super) log: Option<LogService>,
    pub(super) log_directory: Option<PathBuf>,
    pub(super) log_source: LogSource,
    pub(super) logging: config::LoggingConfig,
    pub(super) clear_preview: Option<bool>,
}

impl Session {
    pub(super) fn event(&self, message: String, config: Option<Config>) -> Event {
        Event {
            config,
            directory: self.selection.as_ref().map(|s| s.directory.clone()),
            directory_locked: self.locked,
            running: self.handle.is_some(),
            message,
        }
    }

    pub(super) fn load(&mut self, options: &crate::app::Options) -> Result<Config, String> {
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
            self.sync_history_policy()?;
            self.ensure_log(&config.logging)?;
            self.log_runtime(RuntimeEvent::ConfigurationApplied, false);
        }
        Ok(config)
    }

    fn check_history_draft(&self, config: &Config) -> Result<(), String> {
        if self
            .saved
            .as_ref()
            .is_some_and(|saved| saved.history != config.history)
        {
            return Err("历史保留策略已独立变更；旧草稿不能覆盖或触发清理。请重新加载后保留网络修改，策略请在历史管理中单独确认。".into());
        }
        Ok(())
    }

    pub(super) fn save(&mut self, config: &Config, confirmed: bool) -> Result<(), String> {
        self.check_history_draft(config)?;
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
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(directory.join("portable.flag"))
                {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => failures.push(
                        "配置已保存，但便携标记创建失败；下次请使用 --config-dir 指定此目录，勿直接重试覆盖。"
                            .into(),
                    ),
                }
            }
            if let Err(error) = self.sync_history_policy() {
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

    pub(super) fn check_baseline(&self) -> Result<(), String> {
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

    pub(super) fn save_and_apply(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        exit: &AtomicBool,
        config: &Config,
        confirmed: bool,
    ) -> Result<String, String> {
        self.save_and_apply_authorized(runtime, exit, config, confirmed, false)
    }

    pub(super) fn save_and_apply_authorized(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        exit: &AtomicBool,
        config: &Config,
        confirmed: bool,
        allow_lan: bool,
    ) -> Result<String, String> {
        // Independently confirmed retention changes may be newer than an already
        // queued GUI draft; reject it before stopping the currently running proxy.
        self.check_history_draft(config)?;
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

    pub(super) fn load_window(&mut self, events: &ResultQueue<WindowResponse>) {
        let Some(selection) = self.selection.as_ref().filter(|selection| {
            self.valid && self.locked && !selection.needs_initialization && self.saved.is_some()
        }) else {
            return;
        };
        if self.window_loaded.as_ref() == Some(&selection.directory) {
            return;
        }
        let directory = selection.directory.clone();
        let geometry = if self
            .saved
            .as_ref()
            .is_some_and(|config| config.ui.remember_window_geometry)
        {
            preferences::load(&directory.join(preferences::FILE_NAME))
        } else {
            Ok(None)
        };
        events.publish(match geometry {
            Ok(geometry) => {
                // Cache only successful loads. A transient sharing/permission failure
                // must remain retryable through an explicit reload of this directory.
                self.window_loaded = Some(directory.clone());
                WindowResponse::Loaded {
                    directory,
                    geometry,
                }
            }
            Err(error) => WindowResponse::Error(format!(
                "窗口偏好读取失败（{:?}）；未覆盖原文件。",
                error.kind()
            )),
        });
    }

    pub(super) fn save_window(
        &self,
        directory: PathBuf,
        geometry: preferences::WindowGeometry,
    ) -> Result<WindowResponse, String> {
        let selection = self
            .selection
            .as_ref()
            .filter(|selection| self.valid && self.locked && !selection.needs_initialization)
            .ok_or("尚未选定并保存数据目录，未写入窗口偏好。")?;
        if !directory.is_absolute() || !same_directory(&selection.directory, &directory) {
            return Err("数据目录已改变；旧窗口偏好保存请求已拒绝。".into());
        }
        if !self
            .saved
            .as_ref()
            .is_some_and(|config| config.ui.remember_window_geometry)
        {
            return Err("已保存配置已关闭窗口记忆；保留旧文件但不写入。".into());
        }
        preferences::save(&selection.directory.join(preferences::FILE_NAME), &geometry)
            .map_err(|error| format!("窗口偏好保存失败（{:?}）；未报告成功。", error.kind()))?;
        Ok(WindowResponse::Saved)
    }
}

pub(super) fn check_secrets(config: &Config, confirmed: bool) -> Result<(), String> {
    let has_secrets =
        !config.keys.upstream_api_key.is_empty() || !config.access.client_token.is_empty();
    if has_secrets && config.storage.secrets == SecretStorage::Plaintext && !confirmed {
        return Err("保存非空密钥必须再次确认明文泄露风险。".into());
    }
    Ok(())
}

pub(super) fn stop(runtime: &tokio::runtime::Runtime, session: &mut Session) -> Result<(), String> {
    let Some(handle) = session.handle.take() else {
        return if session.stop_uncertain {
            Err("上次停止未确认完成；请退出并重新启动程序，不能报告已完全停止。".into())
        } else {
            Ok(())
        };
    };
    *session
        .overview
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = (None, None);
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

pub(super) fn start(
    runtime: &tokio::runtime::Runtime,
    session: &mut Session,
    exit: &AtomicBool,
) -> Result<String, String> {
    start_authorized(runtime, session, exit, false)
}

pub(super) fn start_authorized(
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
            result = proxy::start_with_metrics(config.clone(), Arc::clone(&session.usage), session.log.as_ref().map(LogService::handle), session.history.as_ref().map(|owner| owner.handle.clone()), Arc::clone(&session.metrics)) => result.map_err(|e| proxy_error(&e)),
            _ = async { while !exit.load(Ordering::Acquire) { tokio::time::sleep(Duration::from_millis(50)).await; } } => Err("程序退出，已取消启动。".into()),
            _ = tokio::time::sleep(Duration::from_secs(30)) => Err("代理初始化超过 30 秒，已取消启动。".into()),
        }
    })?;
    let message = format!(
        "代理运行中；实际监听 {}，使用已保存配置。",
        handle.local_addr()
    );
    *session
        .overview
        .lock()
        .unwrap_or_else(|error| error.into_inner()) =
        (Some(handle.local_addr()), Some(config.upstream.url));
    session.handle = Some(handle);
    session.log_runtime(RuntimeEvent::ListenerStarted, false);
    Ok(message)
}
