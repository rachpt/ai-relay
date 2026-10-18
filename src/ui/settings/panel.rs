//! 设置页控件创建、分类布局与启用状态。
use super::*;
use crate::config::{Config, InjectionMode, SecretStorage, StorageMode};
use windows_sys::Win32::{Foundation::HINSTANCE, Graphics::Gdi::HFONT};

impl Settings {
    #[allow(clippy::too_many_arguments)]
    fn control(
        &mut self,
        parent: HWND,
        instance: HINSTANCE,
        class: &str,
        title: &str,
        style: u32,
        id: u32,
        row: (i32, i32, i32, i32),
    ) -> Result<HWND, String> {
        let class = wide(class);
        let title = wide(title);
        // SAFETY: Strings live for synchronous creation, parent/module are live; child ID is not a pointer.
        let control = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                0,
                0,
                0,
                0,
                parent,
                id as usize as HMENU,
                instance,
                null(),
            )
        };
        if control.is_null() {
            return Err(last_error("创建设置控件"));
        }
        self.controls.push((control, row.0, row.1, row.2, row.3));
        if String::from_utf16_lossy(&class[..class.len() - 1]) == "EDIT" {
            // SAFETY: Live EDIT; message limits user input without retaining a pointer.
            unsafe {
                SendMessageW(
                    control,
                    EM_SETLIMITTEXT,
                    if id == USAGE { 128 * 1024 } else { 16384 },
                    0,
                );
            }
        }
        Ok(control)
    }
    pub fn create(&mut self, hwnd: HWND, instance: HINSTANCE) -> Result<(), String> {
        for (caption, y) in [
            ("上游 URL", 120),
            ("监听 IP / 端口", 162),
            ("上游 Key", 204),
            ("密钥注入模式", 246),
            ("配置存储目录", 288),
            ("密钥保存方式", 330),
        ] {
            self.control(
                hwnd,
                instance,
                "STATIC",
                caption,
                0,
                0,
                (24, y + 4, 150, 28),
            )?;
        }
        let edit = WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32;
        self.url = self.control(hwnd, instance, "EDIT", "", edit, 0, (180, 120, 550, 30))?;
        self.ip = self.control(
            hwnd,
            instance,
            "EDIT",
            "127.0.0.1",
            edit,
            0,
            (180, 162, 360, 30),
        )?;
        self.port = self.control(hwnd, instance, "EDIT", "8080", edit, 0, (552, 162, 178, 30))?;
        self.key = self.control(
            hwnd,
            instance,
            "EDIT",
            "",
            edit | ES_PASSWORD as u32,
            0,
            (180, 204, 550, 30),
        )?;
        self.injection = self.control(
            hwnd,
            instance,
            "COMBOBOX",
            "",
            WS_TABSTOP | CBS_DROPDOWNLIST as u32 | WS_VSCROLL,
            0,
            (180, 246, 550, 180),
        )?;
        self.storage = self.control(
            hwnd,
            instance,
            "COMBOBOX",
            "",
            WS_TABSTOP | CBS_DROPDOWNLIST as u32 | WS_VSCROLL,
            STORAGE,
            (180, 288, 550, 180),
        )?;
        self.secrets = self.control(
            hwnd,
            instance,
            "COMBOBOX",
            "",
            WS_TABSTOP | CBS_DROPDOWNLIST as u32 | WS_VSCROLL,
            0,
            (180, 330, 550, 180),
        )?;
        for (control, items) in [
            (
                self.injection,
                vec![
                    "off — 不注入",
                    "always — 覆盖认证头",
                    "missing — 客户端无认证时注入",
                ],
            ),
            (
                self.storage,
                vec!["标准目录（推荐）", "软件目录（portable）"],
            ),
            (
                self.secrets,
                vec![
                    "DPAPI 当前用户保护（不保证跨机器／用户）",
                    "明文保存（需逐次确认风险）",
                ],
            ),
        ] {
            for item in items {
                let item = wide(item);
                // SAFETY: Live combo copies the NUL-terminated string synchronously.
                unsafe {
                    SendMessageW(control, CB_ADDSTRING, 0, item.as_ptr() as isize);
                }
            }
            select(control, 0);
        }
        self.auto_start = self.control(
            hwnd,
            instance,
            "BUTTON",
            "下次程序启动时自动启动代理",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            0,
            (180, 370, 550, 28),
        )?;
        self.directory = self.control(
            hwnd,
            instance,
            "STATIC",
            "数据目录：正在加载……",
            0,
            0,
            (24, 406, 706, 70),
        )?;
        for (caption, id, x) in [
            ("保存并应用", SAVE, 24),
            ("启动已保存配置", START, 188),
            ("停止代理", STOP, 400),
            ("重新加载", LOAD, 564),
        ] {
            self.control(
                hwnd,
                instance,
                "BUTTON",
                caption,
                WS_TABSTOP | BS_PUSHBUTTON as u32,
                id,
                (x, 486, if id == START { 196 } else { 148 }, 32),
            )?;
        }
        self.control(
            hwnd,
            instance,
            "STATIC",
            "外观主题",
            0,
            0,
            (24, 550, 150, 28),
        )?;
        self.theme = choices::create(hwnd, THEME, &["◐ 跟随系统", "☀ 浅色", "☾ 深色"])?;
        self.controls.push((self.theme, 180, 546, 550, 32));
        select(self.theme, 0);
        self.control(
            hwnd,
            instance,
            "STATIC",
            "主题立即预览；保存并应用后持久化。登录自启与配置导入导出位于“启动、存储与配置文件”分类。\n导出默认不含凭据；切换分类不会保存或丢失草稿。",
            0,
            0,
            (24, 590, 706, 48),
        )?;
        self.login = self.control(
            hwnd,
            instance,
            "BUTTON",
            "登录 Windows 后自动启动（当前用户，非系统服务）",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            LOGIN,
            (24, 646, 540, 28),
        )?;
        self.login_status = self.control(
            hwnd,
            instance,
            "STATIC",
            "系统实际自启状态：尚未查询（不会自动修改 Startup）",
            0,
            0,
            (24, 680, 706, 28),
        )?;
        for (caption, id, x, y, w) in [
            ("查询实际状态", LOGIN_QUERY, 576, 646, 144),
            ("回滚上次应用…", ROLLBACK, 24, 718, 164),
            ("导出脱敏配置…", EXPORT, 200, 718, 164),
            ("导入配置…", IMPORT, 376, 718, 164),
            ("恢复备份…", RESTORE, 552, 718, 164),
            ("放弃草稿", DISCARD, 24, 760, 164),
            ("打开数据目录", 2038, 200, 760, 340),
        ] {
            self.control(
                hwnd,
                instance,
                "BUTTON",
                caption,
                WS_TABSTOP,
                id,
                (x, y, w, 30),
            )?;
        }
        self.usage = self.control(
            hwnd,
            instance,
            "EDIT",
            "当前程序会话统计（非账单 / 未持久化）：等待后台快照……",
            WS_TABSTOP
                | WS_BORDER
                | WS_VSCROLL
                | WS_HSCROLL
                | (ES_READONLY | ES_MULTILINE | ES_AUTOVSCROLL | ES_AUTOHSCROLL) as u32,
            USAGE,
            (24, 620, 706, 200),
        )?;
        self.create_categories(hwnd, instance)?;
        self.busy = true;
        self.enable();
        Ok(())
    }
    pub fn show_page(&mut self, page: usize) {
        self.page = page;
        let mut shows = ShowBatch::with_capacity(self.controls.len());
        for &(control, ..) in &self.controls {
            let visible = if control == self.usage {
                false
            } else {
                page == 3
                    && self
                        .groups
                        .iter()
                        .find(|(h, _)| *h == control)
                        .is_none_or(|(_, category)| *category == selected(self.category))
            };
            shows.add(control, visible);
        }
        shows.commit();
    }
    pub fn show_session(&self, visible: bool) {
        // SAFETY: Live usage child; no modal dispatch.
        unsafe {
            ShowWindow(self.usage, if visible { SW_SHOW } else { SW_HIDE });
        }
    }
    pub(super) fn actions_y(&self) -> i32 {
        let category = selected(self.category);
        self.controls
            .iter()
            .filter(|(control, ..)| {
                self.groups
                    .iter()
                    .any(|(handle, group)| handle == control && *group == category)
            })
            // Combo rows reserve dropdown space, not the closed control's height.
            .map(|&(_, _, y, _, h)| y + if h == 180 { 30 } else { h })
            .max()
            .unwrap_or(142)
            + 24
    }
    pub fn layout(&self, dpi: u32, width: i32, height: i32) {
        let actions_y = self.actions_y();
        let mut moves = MoveBatch::with_capacity(self.controls.len());
        for &(control, x, y, w, h) in &self.controls {
            if control == self.usage {
                moves.add(
                    control,
                    scale(24, dpi),
                    scale(110, dpi),
                    width,
                    (height - scale(194, dpi)).max(scale(120, dpi)),
                );
                continue;
            }
            // SAFETY: Owned live child; querying its ID neither dispatches nor mutates state.
            let id = unsafe { GetDlgCtrlID(control) as u32 };
            let y = if matches!(id, SAVE | START | STOP | LOAD) {
                actions_y
            } else {
                y
            };
            let w = if control == self.port || w == 550 || w == 706 {
                (width - scale(x - 24, dpi)).max(1)
            } else {
                scale(w, dpi)
            };
            moves.add(control, scale(x, dpi), scale(y - 48, dpi), w, scale(h, dpi));
        }
        moves.commit();
    }
    pub fn set_font(&self, font: HFONT) {
        for &(control, ..) in &self.controls {
            // SAFETY: Font remains owned by parent State until replaced/destroyed.
            unsafe {
                SendMessageW(control, WM_SETFONT, font as usize, 0);
            }
        }
    }
    pub(super) fn enable(&self) {
        for &(control, ..) in &self.controls {
            // SAFETY: Owner-thread live controls, no State reentry from child EnableWindow.
            unsafe {
                let id = GetDlgCtrlID(control) as u32;
                let enabled = id == USAGE
                    || (id == THEME && !self.busy && self.config.is_some())
                    || (!self.busy
                        && !self.action_busy
                        && self.file_task.is_none()
                        && match id {
                            STOP => self.running,
                            START => self.config.is_some() && !self.running,
                            LOAD | RESTORE => !self.running && !self.action_busy,
                            LOGIN | LOGIN_QUERY | EXPORT | IMPORT | ROLLBACK => {
                                !self.action_busy && self.file_task.is_none()
                            }
                            2038 => self.locked,
                            LOG_SETTINGS => true,
                            _ => self.config.is_some(),
                        });
                EnableWindow(control, i32::from(enabled));
            }
        }
    }
    pub fn disable(&mut self) {
        self.busy = true;
        self.enable();
    }
    pub(super) fn populate(&mut self, config: Config) {
        select(
            self.theme,
            match config.ui.theme {
                ThemeMode::System => 0,
                ThemeMode::Light => 1,
                ThemeMode::Dark => 2,
            },
        );
        // SAFETY: Queue parent refresh after this Settings borrow has ended.
        unsafe {
            PostMessageW(GetParent(self.theme), theme::REFRESH, 0, 0);
        }
        set_text(self.url, &config.upstream.url);
        set_text(self.ip, &config.listen.address.to_string());
        set_text(self.port, &config.listen.port.to_string());
        set_text(self.key, config.keys.upstream_api_key.expose_secret());
        select(
            self.injection,
            match config.keys.injection_mode {
                InjectionMode::Off => 0,
                InjectionMode::Always => 1,
                InjectionMode::Missing => 2,
            },
        );
        self.storage_index = usize::from(config.storage.mode == StorageMode::Portable);
        select(self.storage, self.storage_index);
        select(
            self.secrets,
            usize::from(config.storage.secrets == SecretStorage::Plaintext),
        );
        // SAFETY: Live checkbox, no pointer arguments.
        unsafe {
            SendMessageW(
                self.auto_start,
                BM_SETCHECK,
                usize::from(config.proxy.auto_start),
                0,
            );
        }
        if let Ok(value) = toml::Value::try_from(&config) {
            for &(control, section, field) in &self.fields {
                if let Some(value) = value.get(section).and_then(|v| v.get(field)) {
                    if let Some(value) = value.as_bool() {
                        // SAFETY: Live checkbox, no pointer arguments.
                        unsafe {
                            SendMessageW(control, BM_SETCHECK, usize::from(value), 0);
                        }
                    } else {
                        set_text(
                            control,
                            value
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| value.to_string())
                                .as_str(),
                        );
                    }
                }
            }
        }
        self.config = Some(config);
    }
    pub fn create_categories(&mut self, hwnd: HWND, instance: HINSTANCE) -> Result<(), String> {
        // Keep original controls and IDs; only visibility and positions change.
        for &(control, _, y, _, _) in &self.controls {
            if control == self.usage {
                continue;
            }
            let category = if y <= 284 {
                Some(0)
            } else if (288..=476).contains(&y) || y >= 646 {
                Some(4)
            } else if (546..=638).contains(&y) {
                Some(5)
            } else {
                None
            };
            if let Some(category) = category {
                self.groups.push((control, category));
            }
        }
        self.category = choices::create(
            hwnd,
            CATEGORY,
            &[
                "上游监听",
                "访问控制",
                "代理CORS",
                "网络上限",
                "启动存储",
                "外观日志",
            ],
        )?;
        self.controls.push((self.category, 24, 108, 706, 34));
        select(self.category, 0);
        for row in &mut self.controls {
            if row.0 == self.category || row.0 == self.usage {
                continue;
            }
            if self
                .groups
                .iter()
                .any(|(h, group)| *h == row.0 && *group == 0)
            {
                row.2 += 40;
            }
            if self
                .groups
                .iter()
                .any(|(h, group)| *h == row.0 && *group == 4)
            {
                row.2 = match row.2 {
                    288..=476 => row.2 - 128,
                    646 => 360,
                    680 => 398,
                    718 => 436,
                    760 => 556,
                    other => other,
                };
            }
            if self
                .groups
                .iter()
                .any(|(h, group)| *h == row.0 && *group == 5)
            {
                row.2 -= 380;
            }
        }
        for (group, fields) in [
            (
                1,
                vec![
                    ("IP/CIDR 白名单（TOML 数组）", "access", "whitelist", false),
                    ("启用独立客户端认证", "access", "client_token_enabled", true),
                    ("客户端令牌", "access", "client_token", false),
                ],
            ),
            (
                2,
                vec![
                    ("启用 HTTP 正向代理", "proxy", "forward_enabled", true),
                    (
                        "正向请求改写至上游",
                        "proxy",
                        "rewrite_forward_to_upstream",
                        true,
                    ),
                    ("启用 CONNECT 隧道", "proxy", "connect_enabled", true),
                    (
                        "CONNECT 端口（如 [443]）",
                        "proxy",
                        "connect_allowed_ports",
                        false,
                    ),
                    ("启用 CORS", "cors", "enabled", true),
                    (
                        "Origin（TOML 字符串数组）",
                        "cors",
                        "allowed_origins",
                        false,
                    ),
                    ("CORS 允许凭据", "cors", "allow_credentials", true),
                ],
            ),
            (
                3,
                vec![
                    (
                        "建连超时（秒，非零）",
                        "network",
                        "connect_timeout_secs",
                        false,
                    ),
                    (
                        "响应头超时（秒，非零）",
                        "network",
                        "response_header_timeout_secs",
                        false,
                    ),
                    (
                        "响应体空闲（秒，0 禁用）",
                        "network",
                        "response_body_idle_timeout_secs",
                        false,
                    ),
                    (
                        "隧道空闲（秒，0 禁用）",
                        "network",
                        "tunnel_idle_timeout_secs",
                        false,
                    ),
                    (
                        "停止宽限（秒，非零）",
                        "network",
                        "shutdown_grace_secs",
                        false,
                    ),
                    (
                        "最大活动请求（非零）",
                        "network",
                        "max_active_requests",
                        false,
                    ),
                ],
            ),
        ] {
            for (index, (caption, section, field, checkbox)) in fields.into_iter().enumerate() {
                let y = 160 + index as i32 * 52;
                let label = self.control(
                    hwnd,
                    instance,
                    "STATIC",
                    caption,
                    0,
                    0,
                    (24, y + 4, 300, 32),
                )?;
                let control = self.control(
                    hwnd,
                    instance,
                    if checkbox { "BUTTON" } else { "EDIT" },
                    "",
                    WS_TABSTOP
                        | if checkbox {
                            BS_AUTOCHECKBOX as u32
                        } else {
                            WS_BORDER
                                | ES_AUTOHSCROLL as u32
                                | if field == "client_token" {
                                    ES_PASSWORD as u32
                                } else {
                                    0
                                }
                        },
                    2040 + self.fields.len() as u32,
                    (340, y, 380, 30),
                )?;
                self.groups.extend([(label, group), (control, group)]);
                self.fields.push((control, section, field));
            }
        }
        for (group, caption) in [
            (
                1,
                "空白名单不额外限制来源；回环免白名单但不免客户端认证。令牌独立于上游 Key，使用 Proxy-Authorization: Bearer，验证后不转发。",
            ),
            (
                2,
                "正向代理与 CONNECT 默认关闭。关闭改写时不向非可信目标注入上游 Key；CONNECT 不解密 TLS、不注入 Key。CORS 请限制 Origin，不能同时使用 * 和凭据许可。",
            ),
        ] {
            let y = if group == 1 { 324 } else { 548 };
            let note = self.control(hwnd, instance, "STATIC", caption, 0, 0, (24, y, 706, 84))?;
            self.groups.push((note, group));
        }
        let note = self.control(
            hwnd,
            instance,
            "STATIC",
            "窗口选项通过“保存并应用”生效；记忆位置只在实际数据目录加载后写入。取消“关闭到托盘”后，关闭窗口会停止代理并退出。日志完整设置使用下方入口。",
            0,
            0,
            (24, 398, 706, 84),
        )?;
        let shortcut = self.control(
            hwnd,
            instance,
            "BUTTON",
            "打开日志设置",
            WS_TABSTOP,
            LOG_SETTINGS,
            (24, 496, 220, 32),
        )?;
        self.groups.extend([(note, 5), (shortcut, 5)]);
        for (id, caption, field, y) in [
            (
                2063,
                "关闭窗口时隐藏到托盘（不停止代理）",
                "close_to_tray",
                286,
            ),
            (
                2064,
                "记忆窗口位置、尺寸与最大化状态",
                "remember_window_geometry",
                332,
            ),
        ] {
            let control = self.control(
                hwnd,
                instance,
                "BUTTON",
                caption,
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                id,
                (24, y, 706, 32),
            )?;
            self.groups.push((control, 5));
            self.fields.push((control, "ui", field));
        }
        Ok(())
    }
}
