//! 设置页后台事件轮询：文件任务、自启、配置、状态和会话用量。
use super::*;
use crate::{
    app::controller::{AutostartResponse, Command, ConfigResponse},
    config::SecretStorage,
};

pub(super) fn proxy_status(running: bool, message: &str) -> Option<String> {
    // 启动时自动查询登录启动项，结果只属独立设置，不能覆盖代理启动失败的原因。
    if message.contains("独立")
        || message.contains("配置操作")
        || message.starts_with("登录自启操作")
    {
        return None;
    }
    Some(format!(
        "{}\r\n{}",
        if running {
            "代理状态：运行中"
        } else {
            "代理状态：已停止"
        },
        message
    ))
}

pub unsafe fn poll(hwnd: HWND, state: *mut State) {
    // SAFETY: Owner-thread raw State; no references cross modal preview confirmations.
    unsafe {
        if (*state).settings.modal {
            return;
        }
        let file_result = (*state)
            .settings
            .file_task
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(super::files::Result::Error("文件工作线程已结束。".into()))
                }
                Err(_) => None,
            });
        if let Some(result) = file_result {
            (*state).settings.file_task = None;
            let command = match result {
                super::files::Result::HistoryPath(_) => None,
                super::files::Result::Import(text) => Some(Command::ConfigImportPreview { text }),
                super::files::Result::Restore(backup) => {
                    Some(Command::ConfigRestorePreview { backup })
                }
                super::files::Result::Exported(path) => {
                    set_text(
                        (*state).status_label,
                        &format!("脱敏配置已导出：{}；未包含凭据", path.display()),
                    );
                    None
                }
                super::files::Result::Cancelled => {
                    set_text((*state).status_label, "文件操作已取消，未写入。");
                    None
                }
                super::files::Result::Error(error) => {
                    set_text((*state).status_label, &error);
                    None
                }
            };
            if let Some(command) = command {
                match (*state).controller.send(command) {
                    Ok(()) => (*state).settings.action_busy = true,
                    Err(error) => set_text((*state).status_label, &error),
                }
            }
            (*state).settings.enable();
        }
        while let Ok(response) = (*state).controller.autostart_events.try_recv() {
            (*state).settings.action_busy = false;
            match response {
                AutostartResponse::State { status, persisted } => {
                    use crate::windows::autostart::Status;
                    (*state).settings.actual_login = Some(status == Status::Enabled);
                    SendMessageW(
                        (*state).settings.login,
                        BM_SETCHECK,
                        usize::from(status == Status::Enabled),
                        0,
                    );
                    set_text(
                        (*state).settings.login_status,
                        &format!(
                            "系统实际自启：{}；配置持久化 {}",
                            match status {
                                Status::Enabled => "已启用",
                                Status::Disabled => "未启用",
                                Status::Lost => "路径失效（可手动确认修复）",
                                Status::Inconsistent => "与当前路径不一致／来源不明",
                            },
                            persisted
                        ),
                    );
                }
                AutostartResponse::Error(error) => {
                    (*state).settings.actual_login = None;
                    set_text(
                        (*state).settings.login_status,
                        &format!("自启操作失败／实际状态未知：{error}；请重新查询"),
                    );
                }
            }
            (*state).settings.enable();
        }
        if let Ok(response) = (*state).controller.config_events.try_recv() {
            (*state).settings.action_busy = false;
            let command = match response {
                ConfigResponse::Export { text } => {
                    (*state).settings.file(super::files::Task::Export(text));
                    None
                }
                ConfigResponse::ImportPreview { config } => {
                    (*state).settings.modal = true;
                    let accepted = confirm(
                        hwnd,
                        &format!(
                            "导入预览（凭据已隐藏）：版本 {}；监听 {}:{}；存储 {:?}；密钥保护 {:?}。\n覆盖当前已保存配置并重新加载（按自动启动代理设置运行）？不会变更系统自启。后台可能因运行中或存储限制拒绝。",
                            config.schema_version,
                            config.listen.address,
                            config.listen.port,
                            config.storage.mode,
                            config.storage.secrets
                        ),
                    );
                    let plaintext = config.storage.secrets != SecretStorage::Plaintext
                        || (accepted
                            && confirm(
                                hwnd,
                                "导入采用明文保存；原文件中的密钥与客户端令牌可能写入配置及备份。确认明文风险？",
                            ));
                    (*state).settings.modal = false;
                    Some(Command::ConfigImportConfirm {
                        confirmed: accepted && plaintext,
                        plaintext_confirmed: plaintext,
                    })
                }
                ConfigResponse::RestorePreview { backup, target } => {
                    (*state).settings.modal = true;
                    let accepted = confirm(
                        hwnd,
                        &format!(
                            "恢复预览：\n来源：{}\n目标：{}\n将替换配置并保留可恢复副本，随后重新加载（可能按配置启动代理）。备份可能包含旧凭据或明文秘密；不会修改系统 Startup。确认继续？",
                            backup.display(),
                            target.display()
                        ),
                    );
                    (*state).settings.modal = false;
                    Some(Command::ConfigRestoreConfirm {
                        confirmed: accepted,
                    })
                }
                ConfigResponse::MigrationPreview {
                    source,
                    target,
                    description,
                } => {
                    (*state).settings.modal = true;
                    let accepted = confirm(
                        hwnd,
                        &format!(
                            "{description}\n\n源：{}\n目标：{}",
                            source.display(),
                            target.display()
                        ),
                    );
                    (*state).settings.modal = false;
                    Some(Command::ConfigMigrationConfirm {
                        confirmed: accepted,
                    })
                }
                ConfigResponse::Migrated { directory } => {
                    (*state).settings.status_requested = false;
                    set_text(
                        (*state).status_label,
                        &format!(
                            "迁移已完成：{}；源目录和日志均保留。重新加载目标配置。",
                            directory.display()
                        ),
                    );
                    Some(Command::Load)
                }
                ConfigResponse::Imported => {
                    set_text(
                        (*state).status_label,
                        "配置已导入保存；重新加载已保存配置（按自动启动代理设置运行）。",
                    );
                    Some(Command::Load)
                }
                ConfigResponse::Restored { retained } => {
                    set_text(
                        (*state).status_label,
                        &format!(
                            "备份已恢复；保留副本：{}",
                            retained.map_or_else(|| "无".into(), |p| p.display().to_string())
                        ),
                    );
                    Some(Command::Load)
                }
                ConfigResponse::Error(error) => {
                    set_text(
                        (*state).status_label,
                        &format!("配置操作被拒绝／失败：{error}"),
                    );
                    None
                }
            };
            if (*state).exiting.is_none()
                && let Some(command) = command
            {
                match (*state).controller.send(command) {
                    Ok(()) => (*state).settings.action_busy = true,
                    Err(error) => set_text((*state).status_label, &error),
                }
            }
            (*state).settings.enable();
        }
        while let Ok(event) = (*state).controller.events.try_recv() {
            // Copy disjoint fields before borrowing Settings. EDIT notifications
            // are filtered before State access in window_proc; no modal calls here.
            let status_label = (*state).status_label;
            let exiting = (*state).exiting.is_some();
            let settings = &mut (*state).settings;
            if let Some(config) = event.config {
                if !event.message.contains("自启") {
                    (*state).logs.populate_options(&config.logging);
                    (*state).theme.mode = config.ui.theme;
                    settings.populate(config);
                } else if let Some(saved) = settings.config.as_mut() {
                    saved.system = config.system;
                }
            }
            if event.directory_locked && !settings.locked {
                settings.storage_index = selected(settings.storage);
            }
            settings.locked = event.directory_locked;
            settings.running = event.running;
            if !event.message.contains("自启") && !event.message.contains("配置操作") {
                settings.busy = false;
            }
            if !event.message.contains("独立")
                && !event.message.contains("配置操作")
                && !event.message.contains("自启")
            {
                settings.action_busy = false;
            }
            if let Some(directory) = event.directory {
                settings.scope = Some(directory.clone());
                (*state).history.scope(directory.clone());
                if !settings.status_requested && !exiting {
                    settings.status_requested =
                        (*state).controller.send(Command::AutostartStatus).is_ok();
                }
                set_text(
                    settings.directory,
                    &format!(
                        "实际数据目录：{}\r\n配置版本：1；切换存储模式请求后台迁移检查，拒绝时不会改文件。",
                        directory.display()
                    ),
                );
            }
            if let Some(status) = proxy_status(event.running, &event.message) {
                set_text(status_label, &status);
            }
            if exiting {
                settings.busy = true;
            }
            settings.enable();
        }
        // 独立快照不修改配置、busy/running 或操作结果；每次 tick 最多消费一条。
        if let Ok(text) = (*state).controller.usage_events.try_recv() {
            (*state).settings.pending_usage = Some(text);
        }
        if IsWindowVisible(hwnd) != 0
            && (*state).tabs.active == 1
            && (*state).history.session
            && let Some(text) = (*state).settings.pending_usage.take()
        {
            let settings = &mut (*state).settings;
            if settings.usage_text != text {
                // 保留用户在只读详情中的滚动位置与选区，避免刷新跳回顶部。
                let mut start = 0u32;
                let mut end = 0u32;
                SendMessageW(
                    settings.usage,
                    EM_GETSEL,
                    (&mut start as *mut u32) as usize,
                    (&mut end as *mut u32) as isize,
                );
                let first_line = SendMessageW(settings.usage, EM_GETFIRSTVISIBLELINE, 0, 0);
                set_text(settings.usage, &text);
                SendMessageW(settings.usage, EM_SETSEL, start as usize, end as isize);
                let current_line = SendMessageW(settings.usage, EM_GETFIRSTVISIBLELINE, 0, 0);
                SendMessageW(settings.usage, EM_LINESCROLL, 0, first_line - current_line);
                settings.usage_text = text;
            }
        }
        (*state).settings.sync_overview(&(*state).tabs);
        if (*state).controller.is_finished() && (*state).exiting.is_none() {
            (*state).settings.disable();
            (*state).settings.sync_overview(&(*state).tabs);
            set_text(
                (*state).status_label,
                "后台线程已结束；代理不可用，请退出并重新启动程序。",
            );
        }
    }
}
