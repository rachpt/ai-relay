//! 设置页状态、控件句柄和公共文本/确认辅助。
//! 控件创建在 `panel`，草稿解析在 `draft`，命令和事件分别在 `commands`/`events`。
use super::*;
use crate::config::{Config, ThemeMode};

mod commands;
mod draft;
mod events;
mod layout;
mod numeric;
mod panel;
mod secret;
#[cfg(test)]
mod tests;

pub(super) use commands::handle_command;
pub(super) use events::poll;
pub(super) use secret::draw;

// winuser.h definitions not exposed by the currently enabled windows-sys features.
const EM_SETLIMITTEXT: u32 = 0x00c5;
const EM_GETSEL: u32 = 0x00b0;
const EM_SETSEL: u32 = 0x00b1;
const EM_LINESCROLL: u32 = 0x00b6;
const EM_GETFIRSTVISIBLELINE: u32 = 0x00ce;
#[link(name = "user32")]
unsafe extern "system" {
    pub(super) fn EnableWindow(hwnd: HWND, enable: i32) -> i32;
    #[link_name = "IsWindowEnabled"]
    pub(super) fn is_enabled(hwnd: HWND) -> i32;
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
const AUTO_START: u32 = 2065;
const KEY_VISIBILITY: u32 = 2066;

#[derive(Default)]
pub(super) struct Settings {
    controls: Vec<(HWND, i32, i32, i32, i32)>,
    url: HWND,
    ip: HWND,
    port: HWND,
    key: HWND,
    key_visibility: HWND,
    base_labels: [HWND; 6],
    numbers: Vec<numeric::NumberControl>,
    field_rows: Vec<(HWND, HWND)>,
    layout_width: std::cell::Cell<i32>,
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
    // SAFETY: Live COMBOBOX or fixed Choices control and valid item index; no pointers.
    unsafe { SendMessageW(hwnd, CB_SETCURSEL, index, 0) };
}
pub(super) fn selected(hwnd: HWND) -> usize {
    // SAFETY: Live COMBOBOX or fixed Choices control; no pointer arguments.
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
    pub fn data_directory_available(&self) -> bool {
        self.locked
            && self.scope.is_some()
            && !self.busy
            && !self.action_busy
            && self.file_task.is_none()
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
}
