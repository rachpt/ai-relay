//! 历史服务的关联、策略同步和管理命令路由。
use std::sync::mpsc::TrySendError;

use crate::config::{self, ConfigError};

use super::super::{
    history::{HistoryOwner, history_options},
    types::{Command, HistoryResponse},
    util::{config_error, validation_error},
};
use super::Session;

impl Session {
    pub(crate) fn ensure_history(&mut self) -> Result<(), String> {
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
        let options =
            history_options(&self.saved.as_ref().ok_or("没有已保存的历史策略。")?.history);
        match HistoryOwner::start(
            selection.directory.clone(),
            self.history_events.clone(),
            options,
        ) {
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

    pub(crate) fn history_command(&mut self, command: Command) -> Result<(), String> {
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

    pub(crate) fn sync_history_policy(&mut self) -> Result<(), String> {
        self.ensure_history()?;
        let options =
            history_options(&self.saved.as_ref().ok_or("没有已保存的历史策略。")?.history);
        let owner = self.history.as_mut().ok_or("历史服务尚未初始化。")?;
        if owner.options != Some(options) {
            owner.configure(options)?;
        }
        Ok(())
    }

    pub(crate) fn configure_history(
        &mut self,
        retention_days: u32,
        max_records: u32,
        confirmed: bool,
    ) -> Result<HistoryResponse, String> {
        if !confirmed {
            return Err("历史策略可能立即删除记录，必须明确确认；未应用或保存。".into());
        }
        let mut config = self.saved.clone().ok_or("请先保存配置，再修改历史策略。")?;
        config.history = config::HistoryConfig {
            retention_days,
            max_records,
        };
        config
            .validate_for_save()
            .map_err(|error| validation_error(error.field))?;
        self.check_baseline()?;
        self.ensure_history()?;
        let owner = self.history.as_mut().ok_or("历史服务尚未初始化。")?;
        let report = owner.configure(history_options(&config.history))?;
        let directory = owner.directory.clone();
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
        result.map_err(|error| format!("历史策略和数据库清理已生效（删除 {} 条），但配置持久化未完整成功：{}；属于部分完成，代理未停止，未自动回滚，请查询存储并检查配置。", report.removed_records, config_error(&error)))?;
        Ok(HistoryResponse::PolicyChanged { directory, report })
    }
}
