//! 设置页命令路由：保存/启动/停止/导入导出等确认与后台命令发送。
use super::*;
use crate::{
    app::controller::Command,
    config::{SecretStorage, StorageMode},
};

pub unsafe fn handle_command(hwnd: HWND, state: *mut State, id: u32) {
    // SAFETY: Caller supplies the stable UI-owned State pointer; no references cross modal calls.
    unsafe {
        if id == 2038 {
            if let Err(error) = (*state).controller.send(Command::OpenDataDirectory) {
                show_error(hwnd, &error);
            }
            return;
        }
        if id == LOG_SETTINGS {
            SetFocus((*state).tabs.handle);
            (*state).show_page(2);
            return;
        }
        if id == CATEGORY {
            let page = (*state).settings.page;
            (*state).settings.show_page(page);
            (*state).layout(hwnd, true);
            return;
        }
        if id == THEME {
            (*state).theme.mode = (*state).settings.theme_mode();
            PostMessageW(hwnd, theme::REFRESH, 0, 0);
            return;
        }
        if (*state).settings.modal {
            return;
        }
        if id == STORAGE && (*state).settings.locked {
            if (*state).settings.busy
                || (*state).settings.action_busy
                || (*state).settings.file_task.is_some()
            {
                select((*state).settings.storage, (*state).settings.storage_index);
                return;
            }
            select((*state).settings.storage, (*state).settings.storage_index);
            if (*state).settings.config.is_some() {
                let mode = if (*state).settings.storage_index == 0 {
                    StorageMode::Portable
                } else {
                    StorageMode::Standard
                };
                match (*state)
                    .controller
                    .send(Command::ConfigMigrationPreview { mode })
                {
                    Ok(()) => (*state).settings.action_busy = true,
                    Err(error) => show_error(hwnd, &error),
                }
            }
            (*state).settings.enable();
            (*state).settings.sync_overview(&(*state).tabs);
            return;
        }
        if [
            LOGIN,
            LOGIN_QUERY,
            ROLLBACK,
            EXPORT,
            IMPORT,
            RESTORE,
            DISCARD,
        ]
        .contains(&id)
        {
            if (*state).settings.busy
                || (*state).settings.action_busy
                || (*state).settings.file_task.is_some()
            {
                return;
            }
            let command = match id {
                DISCARD => {
                    if let Some(config) = (*state).settings.config.clone() {
                        (*state).settings.populate(config);
                    }
                    return;
                }
                IMPORT | RESTORE => {
                    (*state).settings.file(if id == IMPORT {
                        super::files::Task::Import
                    } else {
                        super::files::Task::Restore
                    });
                    return;
                }
                EXPORT => Command::ConfigExport,
                LOGIN_QUERY => Command::AutostartStatus,
                LOGIN => {
                    let enable = SendMessageW((*state).settings.login, BM_GETCHECK, 0, 0) != 0;
                    (*state).settings.modal = true;
                    let accepted = confirm(
                        hwnd,
                        if enable {
                            "确认启用／修复当前用户登录自启？将修改系统 Startup 快捷方式，使用当前程序与已保存配置路径，并后台运行。"
                        } else {
                            "确认关闭本应用可识别的登录自启快捷方式？不删除来源不明的启动项。"
                        },
                    );
                    (*state).settings.modal = false;
                    // Restore verified state while awaiting the actual system result.
                    SendMessageW(
                        (*state).settings.login,
                        BM_SETCHECK,
                        usize::from((*state).settings.actual_login == Some(true)),
                        0,
                    );
                    if !accepted || (*state).exiting.is_some() {
                        return;
                    }
                    if enable {
                        Command::AutostartEnable { confirmed: true }
                    } else {
                        Command::AutostartDisable
                    }
                }
                _ => {
                    (*state).settings.modal = true;
                    let accepted = confirm(
                        hwnd,
                        "回滚将覆盖当前已保存配置并尝试重新启动代理。当前草稿不会保存；旧凭据也可能恢复。确认执行？",
                    );
                    (*state).settings.modal = false;
                    if !accepted || (*state).exiting.is_some() {
                        return;
                    }
                    Command::Rollback { confirmed: true }
                }
            };
            match (*state).controller.send(command) {
                Ok(()) => (*state).settings.action_busy = true,
                Err(error) => show_error(hwnd, &error),
            }
            (*state).settings.enable();
            return;
        }
        if ![SAVE, START, STOP, LOAD].contains(&id)
            || (*state).settings.busy
            || (*state).settings.action_busy
            || (*state).settings.file_task.is_some()
        {
            return;
        }
        struct ModalGuard(*mut State);
        impl Drop for ModalGuard {
            fn drop(&mut self) {
                // SAFETY: State stays alive until the command callback returns.
                unsafe {
                    (*self.0).settings.modal = false;
                }
            }
        }
        (*state).settings.modal = true;
        let _modal = ModalGuard(state);
        let command = if id == STOP {
            Command::Stop
        } else if id == LOAD {
            Command::Load
        } else if id == START {
            let Some(config) = (*state).settings.config.clone() else {
                return;
            };
            if !config.listen.address.is_loopback()
                && !confirm(
                    hwnd,
                    "已保存配置为非回环监听，可能向局域网或公网开放代理。请配置白名单、防火墙及独立客户端认证。确认本次启动已保存配置？此授权不保存，不供 CLI 或下次自动启动复用。",
                )
            {
                return;
            }
            if (*state).exiting.is_some() {
                return;
            }
            if !config.listen.address.is_loopback() {
                let Some(directory) = (*state).settings.scope.clone() else {
                    show_error(hwnd, "当前数据目录尚未确定，不能确认局域网启动。");
                    return;
                };
                Command::ConfirmLanStart {
                    directory,
                    config: Box::new(config),
                    plaintext_confirmed: false,
                    apply: false,
                }
            } else {
                Command::Start(Box::new(config))
            }
        } else {
            let config = match (*state).settings.draft() {
                Ok(config) => config,
                Err(error) => {
                    show_error(hwnd, &error);
                    return;
                }
            };
            if !config.listen.address.is_loopback()
                && !confirm(
                    hwnd,
                    "非回环监听可能向局域网或公网开放代理。请确认白名单、防火墙及独立客户端认证。确认保存并在本次操作中启用非回环监听？以后自动启动或 CLI 不会复用此授权。",
                )
            {
                return;
            }
            if (*state).exiting.is_some() {
                return;
            }
            if (config.proxy.forward_enabled || config.proxy.connect_enabled || config.cors.enabled)
                && !confirm(
                    hwnd,
                    "正向代理／CONNECT／CORS 可能扩大网络访问范围或跨域访问权限。请检查白名单、独立客户端认证、CONNECT 允许端口及 Origin。关闭正向改写后将访问客户端指定目标；CONNECT 不解密 TLS。确认保存并应用？",
                )
            {
                return;
            }
            if id == SAVE {
                let has_secrets = !config.keys.upstream_api_key.is_empty()
                    || !config.access.client_token.is_empty();
                let confirmed = !has_secrets
                    || config.storage.secrets != SecretStorage::Plaintext
                    || confirm(
                        hwnd,
                        "密钥和客户端令牌将以明文写入配置及后续备份。能够读取这些文件的人可使用凭据。请勿分享或提交仓库。确认明文保存？",
                    );
                if !confirmed || (*state).exiting.is_some() {
                    return;
                }
                if !config.listen.address.is_loopback() {
                    let Some(directory) = (*state).settings.scope.clone() else {
                        show_error(hwnd, "当前数据目录尚未确定，不能确认保存应用。");
                        return;
                    };
                    Command::ConfirmLanStart {
                        directory,
                        config: Box::new(config),
                        plaintext_confirmed: confirmed,
                        apply: true,
                    }
                } else {
                    Command::SaveAndApply {
                        config: Box::new(config),
                        plaintext_confirmed: confirmed,
                    }
                }
            } else {
                Command::Start(Box::new(config))
            }
        };
        match (*state).controller.send(command) {
            Ok(()) => {
                (*state).settings.disable();
                (*state).settings.sync_overview(&(*state).tabs);
                set_text(
                    (*state).status_label,
                    match id {
                        START => "代理启动中：后台初始化 TLS 与监听……",
                        STOP => "代理停止中：等待请求结束……",
                        SAVE => "后台保存并应用中（运行中将受控重启；失败后可明确确认回滚）……",
                        _ => "后台重新加载配置中……",
                    },
                );
            }
            Err(error) => show_error(hwnd, &error),
        }
    }
}
