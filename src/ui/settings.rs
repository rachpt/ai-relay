use super::*;
use crate::{
    app::controller::{AutostartResponse, Command, ConfigResponse},
    config::{Config, InjectionMode, SecretStorage, SecretString, StorageMode, ThemeMode},
};
use windows_sys::Win32::{Foundation::HINSTANCE, Graphics::Gdi::HFONT};

// winuser.h definitions not exposed by the currently enabled windows-sys features.
const EM_SETLIMITTEXT: u32 = 0x00c5;
const EM_GETSEL: u32 = 0x00b0;
const EM_SETSEL: u32 = 0x00b1;
const EM_LINESCROLL: u32 = 0x00b6;
const EM_GETFIRSTVISIBLELINE: u32 = 0x00ce;
#[link(name = "user32")]
unsafe extern "system" {
    pub(super) fn EnableWindow(hwnd: HWND, enable: i32) -> i32;
}

const SAVE: u32 = 2001;
pub(super) const START: u32 = 2002;
pub(super) const STOP: u32 = 2003;
const LOAD: u32 = 2004;
const STORAGE: u32 = 2010;
const USAGE: u32 = 2020;
const THEME: u32 = 2030;
const LOGIN: u32 = 2031;
const LOGIN_QUERY: u32 = 2032;
const ROLLBACK: u32 = 2033;
const EXPORT: u32 = 2034;
const IMPORT: u32 = 2035;
const RESTORE: u32 = 2036;
const DISCARD: u32 = 2037;
const CATEGORY: u32 = 2039;
const LOG_SETTINGS: u32 = 2062;

fn field_value(
    current: &toml::Value,
    field: &str,
    input: &str,
    checked: bool,
) -> Result<toml::Value, &'static str> {
    Ok(match current {
        toml::Value::Boolean(_) => toml::Value::Boolean(checked),
        toml::Value::Integer(_) => toml::Value::Integer(
            input
                .trim()
                .parse::<u32>()
                .map(i64::from)
                .map_err(|_| "请输入 0–4294967295 范围内的整数。")?,
        ),
        toml::Value::Array(_) => {
            let parsed: toml::Table = format!("value = {input}")
                .parse()
                .map_err(|_| "请输入 TOML 数组，例如 [443] 或 [\"127.0.0.1\"]。")?;
            if parsed.len() != 1 {
                return Err("只允许一个 TOML 数组，不能附加其他字段。");
            }
            let value = parsed
                .get("value")
                .filter(|v| v.is_array())
                .ok_or("请输入 TOML 数组。")?
                .clone();
            let valid = match field {
                "whitelist" => value.clone().try_into::<Vec<String>>().is_ok_and(|items| {
                    items.iter().all(|item| {
                        item.parse::<ipnet::IpNet>().is_ok()
                            || item.parse::<std::net::IpAddr>().is_ok()
                    })
                }),
                "connect_allowed_ports" => value.clone().try_into::<Vec<u16>>().is_ok(),
                "allowed_origins" => value.clone().try_into::<Vec<String>>().is_ok(),
                _ => false,
            };
            if !valid {
                return Err("数组元素无效；请检查 IP/CIDR、端口范围或带引号的 Origin。");
            }
            value
        }
        _ => toml::Value::String(input.to_owned()),
    })
}

#[derive(Default)]
pub(super) struct Settings {
    controls: Vec<(HWND, i32, i32, i32, i32)>,
    url: HWND,
    ip: HWND,
    port: HWND,
    key: HWND,
    injection: HWND,
    theme: HWND,
    storage: HWND,
    secrets: HWND,
    auto_start: HWND,
    directory: HWND,
    scope: Option<std::path::PathBuf>,
    usage: HWND,
    usage_text: String,
    pending_usage: Option<String>,
    config: Option<Config>,
    locked: bool,
    busy: bool,
    running: bool,
    storage_index: usize,
    login: HWND,
    login_status: HWND,
    file_task: Option<std::sync::mpsc::Receiver<super::files::Result>>,
    action_busy: bool,
    modal: bool,
    category: HWND,
    groups: Vec<(HWND, usize)>,
    fields: Vec<(HWND, &'static str, &'static str)>,
    page: usize,
    actual_login: Option<bool>,
    status_requested: bool,
}

pub(super) fn set_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    // SAFETY: Called on the owner thread with a live control; SetWindowText copies text.
    unsafe { SetWindowTextW(hwnd, text.as_ptr()) };
}
pub(super) fn text(hwnd: HWND) -> String {
    // Edit lengths are bounded at creation; avoid unbounded allocation even if changed externally.
    // SAFETY: The control is live and buffer is writable with the supplied capacity.
    unsafe {
        let length = GetWindowTextLengthW(hwnd).clamp(0, 16384) as usize;
        let mut buffer = vec![0; length + 1];
        let count = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..count.max(0) as usize])
    }
}
fn select(hwnd: HWND, index: usize) {
    // SAFETY: Live COMBOBOX and valid item index, no pointer arguments.
    unsafe { SendMessageW(hwnd, CB_SETCURSEL, index, 0) };
}
pub(super) fn selected(hwnd: HWND) -> usize {
    // SAFETY: Live COMBOBOX, no pointer arguments.
    unsafe { SendMessageW(hwnd, CB_GETCURSEL, 0, 0).max(0) as usize }
}
pub(super) fn confirm(hwnd: HWND, message: &str) -> bool {
    let message = wide(message);
    let title = wide("AI Relay 风险确认");
    // SAFETY: Strings and parent live through this modal call; caller holds no State reference.
    unsafe {
        MessageBoxW(
            hwnd,
            message.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        ) == IDYES
    }
}
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
            (24, 646, 550, 28),
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
            ("查询实际状态", LOGIN_QUERY, 576, 646, 148),
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
        for &(control, ..) in &self.controls {
            // SAFETY: Live children; hidden controls are excluded from dialog navigation.
            unsafe {
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
                ShowWindow(control, if visible { SW_SHOW } else { SW_HIDE });
            }
        }
    }
    pub fn show_session(&self, visible: bool) {
        // SAFETY: Live usage child; no modal dispatch.
        unsafe {
            ShowWindow(self.usage, if visible { SW_SHOW } else { SW_HIDE });
        }
    }
    pub fn has_config(&self) -> bool {
        self.config.is_some()
    }
    pub fn scope(&self) -> Option<&std::path::PathBuf> {
        self.scope.as_ref()
    }
    pub fn close_to_tray(&self) -> bool {
        self.config
            .as_ref()
            .is_none_or(|config| config.ui.close_to_tray)
    }
    pub fn remember_geometry(&self) -> bool {
        self.config
            .as_ref()
            .is_some_and(|config| config.ui.remember_window_geometry)
    }
    pub fn overview_available(&self) -> bool {
        !self.busy && !self.action_busy && self.file_task.is_none() && self.config.is_some()
    }
    pub fn sync_overview(&self, tabs: &tabs::Tabs) {
        // SAFETY: Live UI-owned buttons, matching the settings commands' availability.
        unsafe {
            EnableWindow(
                tabs.start,
                i32::from(
                    !self.busy
                        && !self.action_busy
                        && self.file_task.is_none()
                        && !self.running
                        && self.config.is_some(),
                ),
            );
            EnableWindow(
                tabs.stop,
                i32::from(!self.busy && !self.action_busy && self.running),
            );
        }
    }
    pub fn layout(&self, dpi: u32, width: i32, height: i32) {
        for &(control, x, y, w, h) in &self.controls {
            if control == self.usage {
                // SAFETY: Live EDIT, kept intact across page switches.
                unsafe {
                    MoveWindow(
                        control,
                        scale(24, dpi),
                        scale(110, dpi),
                        width,
                        (height - scale(176, dpi)).max(scale(120, dpi)),
                        1,
                    );
                }
                continue;
            }
            let w = if w == 550 || w == 706 {
                (width - scale(x - 24, dpi)).max(1)
            } else {
                scale(w, dpi)
            };
            // SAFETY: These are owned live child controls; moving them does not resize the parent.
            unsafe {
                MoveWindow(
                    control,
                    scale(x, dpi),
                    scale(y - 48, dpi),
                    w,
                    scale(h, dpi),
                    1,
                );
            }
        }
    }
    pub fn set_font(&self, font: HFONT) {
        for &(control, ..) in &self.controls {
            // SAFETY: Font remains owned by parent State until replaced/destroyed.
            unsafe {
                SendMessageW(control, WM_SETFONT, font as usize, 1);
            }
        }
    }
    fn enable(&self) {
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
    pub fn sync_history(&mut self, options: crate::history::Options) {
        if let Some(config) = &mut self.config {
            config.history.retention_days = options.retention_days;
            config.history.max_records = options.max_records;
        }
    }
    pub fn sync_logging(&mut self, logging: crate::config::LoggingConfig) {
        if let Some(config) = &mut self.config {
            config.logging = logging;
        }
    }
    fn populate(&mut self, config: Config) {
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
    fn create_categories(&mut self, hwnd: HWND, instance: HINSTANCE) -> Result<(), String> {
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
        self.category = self.control(
            hwnd,
            instance,
            "COMBOBOX",
            "",
            WS_TABSTOP | CBS_DROPDOWNLIST as u32,
            CATEGORY,
            (180, 108, 550, 220),
        )?;
        for caption in [
            "上游与监听",
            "访问控制",
            "代理模式与 CORS",
            "网络与资源上限",
            "启动、存储与配置文件",
            "外观与日志",
        ] {
            // SAFETY: Native combo copies each terminated string synchronously.
            unsafe {
                SendMessageW(
                    self.category,
                    CB_ADDSTRING,
                    0,
                    wide(caption).as_ptr() as isize,
                );
            }
        }
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
            // Common actions stay below all category content.
            if row.2 == 486 {
                row.2 = 700;
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
            let note = self.control(hwnd, instance, "STATIC", caption, 0, 0, (24, 548, 706, 84))?;
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
    pub fn exit_wait_secs(&self) -> u64 {
        // Includes controller shutdown/log flush overhead; an extreme imported grace is bounded.
        self.config
            .as_ref()
            .map_or(36, |c| {
                u64::from(c.network.shutdown_grace_secs).saturating_add(26)
            })
            .clamp(36, 3600)
    }
    fn file(&mut self, task: super::files::Task) {
        match super::files::spawn(task) {
            Ok(receiver) => self.file_task = Some(receiver),
            Err(error) => set_text(self.directory, &error),
        }
        self.enable();
    }
    pub fn theme_mode(&self) -> ThemeMode {
        match selected(self.theme) {
            1 => ThemeMode::Light,
            2 => ThemeMode::Dark,
            _ => ThemeMode::System,
        }
    }
    fn draft(&self) -> Result<Config, String> {
        let mut config = self.config.clone().ok_or("配置未加载，不能操作。")?;
        config.ui.theme = self.theme_mode();
        config.upstream.url = text(self.url).trim().to_owned();
        config.listen.address = text(self.ip)
            .trim()
            .parse()
            .map_err(|_| "监听 IP 无效，请输入 IPv4 或 IPv6 地址。")?;
        config.listen.port = text(self.port)
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or("监听端口必须在 1–65535 之间。")?;
        config.keys.upstream_api_key = SecretString::new(text(self.key));
        config.keys.injection_mode = match selected(self.injection) {
            1 => InjectionMode::Always,
            2 => InjectionMode::Missing,
            _ => InjectionMode::Off,
        };
        config.storage.mode = if selected(self.storage) == 1 {
            StorageMode::Portable
        } else {
            StorageMode::Standard
        };
        config.storage.secrets = if selected(self.secrets) == 1 {
            SecretStorage::Plaintext
        } else {
            SecretStorage::Dpapi
        };
        // SAFETY: Live checkbox with no pointer arguments.
        config.proxy.auto_start = unsafe { SendMessageW(self.auto_start, BM_GETCHECK, 0, 0) != 0 };
        let mut value = toml::Value::try_from(&config).map_err(|_| "无法准备设置草稿。")?;
        for &(control, section, field) in &self.fields {
            let target = value
                .get_mut(section)
                .and_then(|v| v.get_mut(field))
                .ok_or("未知设置字段。")?;
            // SAFETY: Live native child; only checkboxes interpret BM_GETCHECK.
            let checked = unsafe { SendMessageW(control, BM_GETCHECK, 0, 0) != 0 };
            *target = field_value(target, field, &text(control), checked)
                .map_err(|error| format!("{section}.{field}：{error}"))?;
        }
        config = value
            .try_into()
            .map_err(|_| "设置类型或范围无效；请检查当前及其他分类的输入。")?;
        config
            .validate_for_save()
            .map_err(|e| format!("配置校验失败（字段 {}），请检查输入及注入模式。", e.field))?;
        Ok(config)
    }
}

// 原始指针避免在确认弹窗的嵌套消息循环期间持有 State 引用。
pub(super) unsafe fn handle_command(hwnd: HWND, state: *mut State, id: u32) {
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

pub(super) unsafe fn poll(hwnd: HWND, state: *mut State) {
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
            if !event.message.contains("独立") && !event.message.contains("配置操作") {
                set_text(
                    status_label,
                    &format!(
                        "{}\r\n{}",
                        if event.running {
                            "代理状态：运行中"
                        } else {
                            "代理状态：已停止"
                        },
                        event.message
                    ),
                );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_and_protocol_fields_round_trip_without_losing_secrets() {
        let config = Config::default();
        let mut value = toml::Value::try_from(&config).unwrap();
        for (section, field, input, checked) in [
            (
                "access",
                "whitelist",
                "[\"127.0.0.1\", \"::1\", \"192.0.2.0/24\"]",
                false,
            ),
            ("access", "client_token_enabled", "", true),
            (
                "access",
                "client_token",
                "synthetic-local-client-token",
                false,
            ),
            ("proxy", "forward_enabled", "", true),
            ("proxy", "rewrite_forward_to_upstream", "", false),
            ("proxy", "connect_enabled", "", true),
            ("proxy", "connect_allowed_ports", "[443, 8443]", false),
            ("cors", "enabled", "", true),
            (
                "cors",
                "allowed_origins",
                "[\"https://client.invalid\"]",
                false,
            ),
            ("cors", "allow_credentials", "", true),
            ("network", "max_active_requests", "17", false),
        ] {
            let target = value.get_mut(section).unwrap().get_mut(field).unwrap();
            *target = field_value(target, field, input, checked).unwrap();
        }
        let actual: Config = value.try_into().unwrap();
        actual.validate_for_save().unwrap();
        assert_eq!(actual.access.whitelist.len(), 3);
        assert!(actual.access.client_token_enabled);
        assert_eq!(
            actual.access.client_token.expose_secret(),
            "synthetic-local-client-token"
        );
        assert!(actual.proxy.forward_enabled && actual.proxy.connect_enabled);
        assert!(!actual.proxy.rewrite_forward_to_upstream);
        assert_eq!(actual.proxy.connect_allowed_ports, [443, 8443]);
        assert!(actual.cors.enabled && actual.cors.allow_credentials);
        assert_eq!(actual.network.max_active_requests, 17);
    }

    #[test]
    fn window_checkboxes_round_trip_without_changing_proxy_preferences() {
        let original = Config::default();
        let mut value = toml::Value::try_from(&original).unwrap();
        for field in ["close_to_tray", "remember_window_geometry"] {
            let target = value.get_mut("ui").unwrap().get_mut(field).unwrap();
            *target = field_value(target, field, "", false).unwrap();
        }
        let actual: Config = value.try_into().unwrap();
        assert!(!actual.ui.close_to_tray);
        assert!(!actual.ui.remember_window_geometry);
        for section in ["proxy", "listen", "upstream", "network", "access", "cors"] {
            let before = toml::Value::try_from(&original).unwrap();
            let after = toml::Value::try_from(&actual).unwrap();
            assert_eq!(before.get(section), after.get(section));
        }
    }

    #[test]
    fn invalid_field_input_is_rejected_without_echoing_the_input() {
        let array = toml::Value::Array(vec![]);
        for (field, input) in [
            ("whitelist", "[\"synthetic-private-value\"]"),
            ("connect_allowed_ports", "[65536]"),
            ("connect_allowed_ports", "[\"443\"]"),
            ("allowed_origins", "[1]"),
            ("allowed_origins", "[]\nextra = 'synthetic-private-value'"),
        ] {
            let error = field_value(&array, field, input, false).unwrap_err();
            assert!(!error.contains("synthetic-private-value"));
        }
        let integer = toml::Value::Integer(1);
        for input in ["-1", "4294967296", "1.5", ""] {
            assert!(field_value(&integer, "max_active_requests", input, false).is_err());
        }
    }
}
