//! 登录自启、导入导出、恢复和目录迁移命令。
use crate::{
    config::{self, ConfigError, DirectorySelection, DirectorySource, StorageMode},
    windows::{autostart, paths},
};

use super::super::{
    types::{AutostartResponse, Command, ConfigResponse},
    util::config_error,
};
use super::Session;

impl Session {
    pub(crate) fn autostart_command(
        &mut self,
        command: Command,
    ) -> Result<AutostartResponse, String> {
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

    pub(crate) fn config_command(&mut self, command: Command) -> Result<ConfigResponse, String> {
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
                self.sync_history_policy()
                    .map_err(|error| format!("配置已恢复，但历史策略同步失败：{error}"))?;
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
                let plan = crate::app::migration::Plan::new(
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
                self.sync_history_policy()?;
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
