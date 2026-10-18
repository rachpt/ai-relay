//! Win32 设置与代理控制；所有磁盘和网络初始化由后台控制器执行。
mod choices;
mod control_theme;
mod date_picker;
mod files;
mod history;
mod history_chart;
mod logs;
mod overview;
mod overview_dashboard;
mod rounded;
mod scroll_theme;
mod settings;
mod status_bar;
mod tabs;
mod theme;
mod title_menu;
pub(crate) mod usage;
mod viewport;
use crate::app::instance::{Control, Signals};
use crate::app::{Options, controller::Controller};
use std::{
    sync::{Arc, atomic::Ordering, mpsc::Receiver},
    time::Instant,
};

use crate::windows::{Font, Icon, Menu, enable_dpi_awareness, last_error, scale, wide};
use std::{
    mem::size_of,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{COLOR_WINDOW, HBRUSH, UpdateWindow},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{NMHDR, TCN_SELCHANGE},
        HiDpi::{GetDpiForSystem, GetDpiForWindow},
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NIN_SELECT,
            NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
        },
        WindowsAndMessaging::*,
    },
};

// shellapi.h defines NIN_KEYSELECT as NIN_SELECT | NINF_KEY (0x1),
// but windows-sys 0.61 does not expose that composed constant.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const TRAY_MESSAGE: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
const SHOW_COMMAND: u32 = 1001;
const EXIT_COMMAND: u32 = 1002;
const WINDOW_CLASS: &str = "AIRelay.Window.v1";
const PLACEMENT_CHANGED: u32 = WM_APP + 21;
const OPEN_TITLE_MENU: u32 = WM_APP + 22;
// Client header identity strip: menu button, app icon + product name, then tabs.
const SS_ICON: u32 = 0x0000_0003;
const SS_CENTER: u32 = 0x0000_0001;
const SS_CENTERIMAGE: u32 = 0x0000_0200;
const SS_NOTIFY: u32 = 0x0000_0100;
const STM_SETICON: u32 = 0x0170;
const MENU_BUTTON: u32 = 2110;
const HEADER_HEIGHT: i32 = 44;
const BRAND_LEFT: i32 = 64;
const BRAND_ICON_SIZE: i32 = 32;
const BRAND_TEXT_LEFT: i32 = 104;
const BRAND_TEXT_WIDTH: i32 = 130;
const TAB_LEFT: i32 = 244;

/// Batches page-layout moves. Page layouts only establish document coordinates;
/// `Viewport::arrange` performs the final moved-and-painted pass. Suppressing
/// per-control repaint here avoids N synchronous paint/layout notifications.
pub(super) struct MoveBatch {
    entries: Vec<(HWND, i32, i32, i32, i32)>,
}
impl MoveBatch {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
    }
    pub(super) fn add(&mut self, hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
        self.entries.push((hwnd, x, y, width, height));
    }
    pub(super) fn commit(self) {
        if self.entries.is_empty() {
            return;
        }
        // SAFETY: All handles are live children on this UI thread. A failed batch
        // falls back to the same geometry with per-window SetWindowPos calls.
        unsafe {
            let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW;
            let mut hdwp = BeginDeferWindowPos(self.entries.len() as i32);
            if hdwp.is_null() {
                for &(hwnd, x, y, width, height) in &self.entries {
                    SetWindowPos(hwnd, null_mut(), x, y, width, height, flags);
                }
                return;
            }
            for &(hwnd, x, y, width, height) in &self.entries {
                hdwp = DeferWindowPos(hdwp, hwnd, null_mut(), x, y, width, height, flags);
                if hdwp.is_null() {
                    for &(hwnd, x, y, width, height) in &self.entries {
                        SetWindowPos(hwnd, null_mut(), x, y, width, height, flags);
                    }
                    return;
                }
            }
            EndDeferWindowPos(hdwp);
        }
    }
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetFocus(hwnd: HWND) -> HWND;
}

/// Batches visibility without painting intermediate document/stale coordinates.
/// The caller finishes with layout/arrange or refresh, which redraws the host tree.
pub(super) struct ShowBatch {
    entries: Vec<(HWND, bool)>,
}
impl ShowBatch {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
    }
    pub(super) fn add(&mut self, hwnd: HWND, visible: bool) {
        self.entries.push((hwnd, visible));
    }
    pub(super) fn commit(self) {
        // SAFETY: All handles are live children on this UI thread. The batch only
        // changes WS_VISIBLE; failure uses the same no-redraw flags per window.
        unsafe {
            let base = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW;
            let mut hdwp = BeginDeferWindowPos(self.entries.len().max(1) as i32);
            if hdwp.is_null() {
                for &(hwnd, visible) in &self.entries {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        0,
                        0,
                        0,
                        0,
                        base | if visible {
                            SWP_SHOWWINDOW
                        } else {
                            SWP_HIDEWINDOW
                        },
                    );
                }
                return;
            }
            for &(hwnd, visible) in &self.entries {
                if (GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE != 0) == visible {
                    continue;
                }
                hdwp = DeferWindowPos(
                    hdwp,
                    hwnd,
                    null_mut(),
                    0,
                    0,
                    0,
                    0,
                    base | if visible {
                        SWP_SHOWWINDOW
                    } else {
                        SWP_HIDEWINDOW
                    },
                );
                if hdwp.is_null() {
                    for &(hwnd, visible) in &self.entries {
                        SetWindowPos(
                            hwnd,
                            null_mut(),
                            0,
                            0,
                            0,
                            0,
                            base | if visible {
                                SWP_SHOWWINDOW
                            } else {
                                SWP_HIDEWINDOW
                            },
                        );
                    }
                    return;
                }
            }
            EndDeferWindowPos(hdwp);
        }
    }
}

struct State {
    icon: Icon,
    font: Font,
    heading_font: Font,
    theme: theme::Theme,
    title_menu: title_menu::TitleMenu,
    labels: Vec<HWND>,
    status_label: HWND,
    menu_button: HWND,
    brand_icon: HWND,
    brand_label: HWND,
    viewport: viewport::Viewport,
    settings: settings::Settings,
    tabs: tabs::Tabs,
    overview: overview::Overview,
    preferences: overview::Preferences,
    logs: logs::Logs,
    history: history::History,
    controller: Controller,
    exiting: Option<Instant>,
    dpi: u32,
    page_initialized: bool,
    page_geometry: [bool; 4],
    page_positioned: [bool; 4],
    pages_prelaid: bool,
    layout_size: (i32, i32, u32),
    taskbar_created: u32,
    tray_present: bool,
    close_notice_shown: bool,
    error: Option<String>,
    signals: Arc<Signals>,
}

impl State {
    fn tray_data(&self, hwnd: HWND) -> NOTIFYICONDATAW {
        // SAFETY: All-zero is a valid initial NOTIFYICONDATAW; cbSize is set below.
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = TRAY_ID;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = TRAY_MESSAGE;
        data.hIcon = self.icon.handle;
        let tip = wide("AI Relay — 反向代理（状态见窗口）");
        data.szTip[..tip.len()].copy_from_slice(&tip);
        data
    }

    fn add_tray(&mut self, hwnd: HWND) -> Result<(), String> {
        let mut data = self.tray_data(hwnd);
        // SAFETY: data has the correct size and references this live window/icon.
        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
            self.tray_present = false;
            return Err("无法添加托盘图标，请确认 Windows 桌面和 Explorer 正常运行。".into());
        }
        self.tray_present = true;
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: The tray icon was just added with this window/id pair.
        if unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) } == 0 {
            self.remove_tray(hwnd);
            return Err("无法设置托盘通知版本。".into());
        }
        Ok(())
    }

    fn remove_tray(&mut self, hwnd: HWND) {
        if self.tray_present {
            let data = self.tray_data(hwnd);
            // SAFETY: This window/id pair identifies only our tray icon.
            unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
            self.tray_present = false;
        }
    }

    fn layout(&mut self, hwnd: HWND, force: bool) {
        // Creation can deliver WM_SIZE before the page tree exists. Do not cache
        // a successful prelayout of null HWNDs before initial control creation.
        if self.viewport.handle.is_null() {
            return;
        }
        let mut rect = RECT::default();
        // SAFETY: hwnd is live and rect is a writable RECT.
        if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
            return;
        }
        let size = (rect.right, rect.bottom, self.dpi);
        if size != self.layout_size {
            self.layout_size = size;
            self.page_geometry = [false; 4];
            self.page_positioned = [false; 4];
        }
        let active = self.tabs.active;
        let virtual_height = viewport::page_height(self.dpi, rect.bottom, active);
        let session = active == 1 && self.history.session;
        if !force && !session && self.page_positioned[active] && self.page_geometry[active] {
            // Fast path for an already-positioned page: no header/control geometry
            // changed, only the viewport's active page and scroll metadata.
            self.viewport.refresh(self.dpi, virtual_height, active);
            return;
        }
        let (page_width, page_height) =
            self.viewport
                .layout(self.dpi, rect.right, rect.bottom, active);
        let margin = scale(24, self.dpi);
        let width = (page_width - margin * 2).max(1);
        if !self.pages_prelaid {
            // Pay the first native-layout cost once, while the window is still
            // hidden. Every page then only needs one arrange pass on first show.
            let heights = std::array::from_fn::<_, 4, _>(|page| {
                viewport::page_height(self.dpi, rect.bottom, page)
            });
            self.overview.layout(self.dpi, page_width, heights[0]);
            self.history.layout(self.dpi, page_width, heights[1]);
            self.logs.layout(self.dpi, page_width, heights[2]);
            self.settings.layout(self.dpi, width, heights[3]);
            self.page_geometry = [true; 4];
            self.page_positioned = [false; 4];
            self.pages_prelaid = true;
        }
        self.tabs
            .layout(self.dpi, page_width, scale(TAB_LEFT, self.dpi));
        let header = scale(HEADER_HEIGHT, self.dpi);
        let icon_size = scale(BRAND_ICON_SIZE, self.dpi);
        // SAFETY: Fixed header children are outside the scrolling viewport.
        unsafe {
            MoveWindow(
                self.menu_button,
                scale(24, self.dpi),
                0,
                scale(32, self.dpi),
                header,
                1,
            );
            MoveWindow(
                self.brand_icon,
                scale(BRAND_LEFT, self.dpi),
                (header - icon_size) / 2,
                icon_size,
                icon_size,
                1,
            );
            MoveWindow(
                self.brand_label,
                scale(BRAND_TEXT_LEFT, self.dpi),
                0,
                scale(BRAND_TEXT_WIDTH, self.dpi),
                header,
                1,
            );
        }
        if session {
            // The session view reuses the settings usage editor. Lay out both
            // owners at document coordinates, then position the visible union once.
            self.history.layout(self.dpi, page_width, page_height);
            self.page_geometry[1] = true;
            self.page_positioned[1] = false;
            self.settings.layout(self.dpi, width, page_height);
            self.page_geometry[3] = true;
            self.page_positioned[3] = false;
        } else if force || !self.page_geometry[active] {
            match active {
                1 => self.history.layout(self.dpi, page_width, page_height),
                2 => self.logs.layout(self.dpi, page_width, page_height),
                3 => self.settings.layout(self.dpi, width, page_height),
                _ => self.overview.layout(self.dpi, page_width, page_height),
            }
            self.page_geometry[active] = true;
            self.page_positioned[active] = false;
        }
        if session || !self.page_positioned[active] {
            if active == 0 && !self.page_positioned[0] {
                if let Some(title) = self.labels.first() {
                    // SAFETY: fixed overview heading reparented with the page; the
                    // following arrange pass moves it into the viewport coordinates.
                    unsafe {
                        MoveWindow(
                            *title,
                            margin,
                            scale(64, self.dpi),
                            width,
                            scale(28, self.dpi),
                            0,
                        );
                    }
                }
                self.tabs.layout_actions(self.dpi, page_width);
            }
            self.viewport.arrange(self.dpi, page_height, active);
            self.page_positioned[active] = true;
            if session {
                // Only the usage editor was visible from the settings owner; its
                // other controls remain at document coordinates for a later layout.
                self.page_positioned[3] = false;
                self.page_geometry[3] = false;
            }
        } else {
            self.viewport.refresh(self.dpi, page_height, active);
        }
        status_bar::layout(self.status_label, self.dpi, rect.right, rect.bottom);
        // WM_SIZE, category changes and first-show layouts all finish here, not
        // only interactive resize completion. Never paint while State is borrowed.
        viewport::redraw(hwnd);
    }

    fn repaint_all(&mut self, hwnd: HWND) {
        self.layout(hwnd, true);
    }

    fn show_page(&mut self, index: usize) {
        let previous = self.tabs.active;
        self.tabs.select(index);
        if !self.page_initialized {
            // First pass: hide every inactive page; later switches touch only the
            // outgoing and incoming page instead of all four control sets.
            self.page_initialized = true;
            self.settings.show_page(index);
            self.history.show(index == 1);
            self.logs.show(index == 2);
            self.overview.show(index == 0);
        } else if previous != index {
            match previous {
                0 => self.overview.show(false),
                1 => self.history.show(false),
                2 => self.logs.show(false),
                3 => self.settings.show_page(index),
                _ => {}
            }
            match index {
                0 => self.overview.show(true),
                1 => self.history.show(true),
                2 => self.logs.show(true),
                3 => self.settings.show_page(3),
                _ => {}
            }
        }
        self.settings
            .show_session(index == 1 && self.history.session);
        for (i, label) in self.labels.iter().enumerate() {
            // SAFETY: Live STATIC controls. Status stays visible on every page.
            unsafe {
                ShowWindow(
                    *label,
                    if i == 1 || index == 0 {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
            }
        }
        // SAFETY: Live controls. Move focus off the outgoing page before hiding its controls.
        unsafe {
            ShowWindow(self.tabs.start, if index == 0 { SW_SHOW } else { SW_HIDE });
            ShowWindow(self.tabs.stop, if index == 0 { SW_SHOW } else { SW_HIDE });
        }
        // Every page preserves its own scroll offset; draft controls are not recreated.
        unsafe {
            self.layout(GetParent(self.tabs.handle), false);
        }
    }

    fn update_dpi(&mut self, hwnd: HWND, dpi: u32) -> Result<(), String> {
        let font = Font::new(dpi)?;
        for label in &self.labels {
            // SAFETY: STATIC controls accept WM_SETFONT and do not call our parent
            // procedure for this message. Both the old and new fonts remain live.
            unsafe { SendMessageW(*label, WM_SETFONT, font.0 as WPARAM, 0) };
        }
        self.settings.set_font(font.0);
        self.tabs.set_font(font.0);
        self.overview.set_font(font.0);
        self.logs.set_font(font.0);
        self.history.set_font(font.0);
        let heading = Font::heading(dpi)?;
        if let Some(label) = self.labels.first() {
            // SAFETY: Live STATIC; both fonts remain alive until replacement completes.
            unsafe { SendMessageW(*label, WM_SETFONT, heading.0 as WPARAM, 0) };
        }
        if !self.brand_label.is_null() {
            // SAFETY: Live STATIC; same heading font lifetime as the first label.
            unsafe { SendMessageW(self.brand_label, WM_SETFONT, heading.0 as WPARAM, 0) };
        }
        if !self.menu_button.is_null() {
            // SAFETY: Live STATIC; same heading font lifetime as the brand label.
            unsafe { SendMessageW(self.menu_button, WM_SETFONT, heading.0 as WPARAM, 0) };
        }
        self.heading_font = heading;
        self.font = font;
        self.dpi = dpi;
        // Defer layout/painting until the exclusive font-update borrow ends.
        unsafe {
            PostMessageW(hwnd, WM_SIZE, 0, 0);
        }
        // SAFETY: Queue color/DPI refresh after replacing fonts; no reentrant
        // parent dispatch occurs while State is borrowed by this method.
        unsafe {
            PostMessageW(hwnd, theme::REFRESH, 0, 0);
        }
        Ok(())
    }
}

pub(crate) fn run(
    options: Options,
    ipc: Receiver<Control>,
    signals: Arc<Signals>,
) -> Result<(), String> {
    let background = options.background;
    enable_dpi_awareness()?;
    // SAFETY: Null requests this process's executable module.
    let instance = unsafe { GetModuleHandleW(null()) };
    if instance.is_null() {
        return Err(last_error("获取应用模块"));
    }
    let class_name = wide(WINDOW_CLASS);
    let title = wide("AI Relay 开发版");
    let taskbar_message = wide("TaskbarCreated");
    // SAFETY: The string is NUL-terminated and valid for the call.
    let taskbar_created = unsafe { RegisterWindowMessageW(taskbar_message.as_ptr()) };
    if taskbar_created == 0 {
        return Err(last_error("注册 Explorer 重启消息"));
    }
    // SAFETY: DPI awareness was established before window creation.
    let dpi = unsafe { GetDpiForSystem() }.max(96);
    let mut state = Box::new(State {
        icon: Icon::load()?,
        font: Font::new(dpi)?,
        heading_font: Font::heading(dpi)?,
        theme: theme::Theme::new(crate::config::ThemeMode::System)?,
        title_menu: title_menu::TitleMenu::default(),
        labels: Vec::new(),
        status_label: null_mut(),
        menu_button: null_mut(),
        brand_icon: null_mut(),
        brand_label: null_mut(),
        viewport: viewport::Viewport::default(),
        settings: settings::Settings::default(),
        tabs: tabs::Tabs::default(),
        overview: overview::Overview::default(),
        preferences: overview::Preferences::default(),
        logs: logs::Logs::default(),
        history: history::History::default(),
        controller: Controller::spawn(options, ipc, signals.clone())?,
        signals,
        exiting: None,
        dpi,
        page_initialized: false,
        page_geometry: [false; 4],
        page_positioned: [false; 4],
        pages_prelaid: false,
        layout_size: (0, 0, 0),
        taskbar_created,
        tray_present: false,
        close_notice_shown: false,
        error: None,
    });
    // SAFETY: IDC_ARROW is a shared system cursor resource.
    let cursor = unsafe { LoadCursorW(null_mut(), IDC_ARROW) };
    if cursor.is_null() {
        return Err(last_error("加载窗口光标"));
    }
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        hIcon: state.icon.handle,
        hIconSm: state.icon.handle,
        hCursor: cursor,
        hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
        lpszClassName: class_name.as_ptr(),
        ..Default::default()
    };
    // SAFETY: The callback has the required ABI; strings/icon remain live until
    // all windows are destroyed and the class is unregistered.
    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err(last_error("注册 AI Relay 窗口类"));
    }
    let class_guard = WindowClass {
        instance,
        name: class_name,
    };
    // SAFETY: The boxed State has a stable address through window destruction.
    // WM_NCCREATE stores this non-owning pointer; callbacks never free it.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_guard.name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(800, dpi),
            scale(680, dpi),
            null_mut(),
            null_mut(),
            instance,
            (&mut *state as *mut State).cast(),
        )
    };
    if hwnd.is_null() {
        return Err(last_error("创建 AI Relay 窗口"));
    }
    let window = Window(hwnd);
    let result = (|| {
        // SAFETY: hwnd is a live window owned by this thread.
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let texts = ["AI Relay 开发版", "代理已停止；正在后台加载配置。"];
        let static_class = wide("STATIC");
        for text in texts {
            let text = wide(text);
            // SAFETY: Strings are valid for this synchronous creation call. Parent
            // and module are live. STATIC copies the text; it does not retain it.
            let label = unsafe {
                CreateWindowExW(
                    0,
                    static_class.as_ptr(),
                    text.as_ptr(),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    null_mut(),
                    instance,
                    null(),
                )
            };
            if label.is_null() {
                return Err(last_error("创建状态文字控件"));
            }
            state.labels.push(label);
        }
        state.status_label = state.labels[1];
        let menu_text = wide("☰");
        // SAFETY: STATIC copies the glyph; the client-side button replaces the
        // caption-icon hamburger so WM_SETICON can keep the product icon.
        unsafe {
            state.menu_button = CreateWindowExW(
                0,
                static_class.as_ptr(),
                menu_text.as_ptr(),
                WS_CHILD | WS_VISIBLE | SS_CENTER | SS_CENTERIMAGE | SS_NOTIFY,
                scale(24, dpi),
                0,
                scale(32, dpi),
                scale(HEADER_HEIGHT, dpi),
                hwnd,
                MENU_BUTTON as usize as HMENU,
                instance,
                null(),
            );
        }
        if state.menu_button.is_null() {
            return Err(last_error("创建程序菜单按钮"));
        }
        // SAFETY: Live STATIC; State retains the heading font until DPI replacement.
        unsafe {
            SendMessageW(
                state.menu_button,
                WM_SETFONT,
                state.heading_font.0 as WPARAM,
                0,
            );
        }
        let icon_size = scale(BRAND_ICON_SIZE, dpi);
        // SAFETY: STATIC copies its text/icon reference synchronously; State owns
        // the HICON/font for the full lifetime of these child controls.
        unsafe {
            state.brand_icon = CreateWindowExW(
                0,
                static_class.as_ptr(),
                null(),
                WS_CHILD | WS_VISIBLE | SS_ICON | SS_CENTERIMAGE,
                scale(BRAND_LEFT, dpi),
                (scale(HEADER_HEIGHT, dpi) - icon_size) / 2,
                icon_size,
                icon_size,
                hwnd,
                null_mut(),
                instance,
                null(),
            );
        }
        if state.brand_icon.is_null() {
            return Err(last_error("创建程序图标"));
        }
        // SAFETY: Live STATIC and process-owned icon; the control does not destroy it.
        unsafe {
            SendMessageW(
                state.brand_icon,
                STM_SETICON,
                state.icon.handle as WPARAM,
                0,
            );
        }
        let brand_text = wide("AI Relay");
        // SAFETY: Static class copies the label; State owns the selected font.
        unsafe {
            state.brand_label = CreateWindowExW(
                0,
                static_class.as_ptr(),
                brand_text.as_ptr(),
                WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE,
                scale(BRAND_TEXT_LEFT, dpi),
                0,
                scale(BRAND_TEXT_WIDTH, dpi),
                scale(HEADER_HEIGHT, dpi),
                hwnd,
                null_mut(),
                instance,
                null(),
            );
        }
        if state.brand_label.is_null() {
            return Err(last_error("创建程序名称"));
        }
        // SAFETY: Live STATIC; State retains the heading font until DPI replacement.
        unsafe {
            SendMessageW(
                state.brand_label,
                WM_SETFONT,
                state.heading_font.0 as WPARAM,
                0,
            );
        }
        state.tabs.create(hwnd)?;
        // SAFETY: boxed State and native status STATIC outlive its subclass.
        unsafe {
            status_bar::attach(&mut *state)?;
            // The caption uses a hamburger glyph, but the taskbar must keep the
            // application icon. ICON_BIG is the taskbar/window-switcher image.
            SendMessageW(
                hwnd,
                WM_SETICON,
                ICON_BIG as usize,
                state.icon.handle as isize,
            );
        }
        state.overview.create(hwnd)?;
        state.settings.create(hwnd, instance)?;
        state.logs.create(hwnd, instance)?;
        state.history.create(hwnd)?;
        state.viewport.create(hwnd)?;
        let keep = vec![
            state.tabs.handle,
            state.status_label,
            state.menu_button,
            state.brand_icon,
            state.brand_label,
        ];
        state.viewport.adopt(hwnd, &keep)?;
        state.show_page(0);
        state.update_dpi(hwnd, dpi)?;
        // Clamp the initial default as well as saved geometry before first showing.
        if let Some(work) = overview::work_area(hwnd) {
            let mut rect = RECT::default();
            // SAFETY: Live HWND and writable RECT; no State borrow crosses resizing.
            unsafe {
                if GetWindowRect(hwnd, &mut rect) != 0 {
                    let width = (rect.right - rect.left).min(work.right - work.left).max(1);
                    let height = (rect.bottom - rect.top).min(work.bottom - work.top).max(1);
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        rect.left.clamp(work.left, work.right - width),
                        rect.top.clamp(work.top, work.bottom - height),
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
            }
        }
        // SAFETY: Owner thread and stable State; only native menu/icon messages
        // occur during attachment. Tracking starts later with no State borrow.
        unsafe {
            state.title_menu.set_install_caption_icon(false);
            state
                .title_menu
                .attach(hwnd, state.dpi, state.theme.kind, state.theme.palette)?;
            theme::refresh(hwnd, &mut *state);
        }
        // SAFETY: This live window owns timer 1; callbacks are dispatched on its UI thread.
        if unsafe { SetTimer(hwnd, 1, 100, None) } == 0 {
            return Err(last_error("创建后台事件轮询定时器"));
        }
        state.add_tray(hwnd)?;
        // SAFETY: Window is fully initialized. No State borrow is held while these
        // calls may synchronously dispatch messages to window_proc.
        unsafe {
            if !background {
                ShowWindow(hwnd, SW_SHOW);
                UpdateWindow(hwnd);
            }
        }
        let mut message = MSG::default();
        loop {
            // SAFETY: Writable MSG and no window filter; called on the owner thread.
            let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
            if result == -1 {
                return Err(last_error("读取窗口消息"));
            }
            if result == 0 {
                break;
            }
            // SAFETY: Owner-thread message loop; GetKeyState has no pointer arguments.
            // Handle Ctrl+Tab before IsDialogMessage so edits and combos cannot consume it.
            unsafe {
                #[link(name = "user32")]
                unsafe extern "system" {
                    fn GetKeyState(key: i32) -> i16;
                }
                let title_shortcut =
                    (message.wParam == 0x79 && GetKeyState(0x11) >= 0 && GetKeyState(0x10) >= 0)
                        || (message.wParam == 0x4d && (message.lParam as usize & (1 << 29)) != 0);
                if title_shortcut
                    && matches!(
                        message.message,
                        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP
                    )
                {
                    if matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN)
                        && (message.lParam as usize & (1 << 30)) == 0
                    {
                        PostMessageW(hwnd, OPEN_TITLE_MENU, 0, 0);
                    }
                    // Consume the paired key-up too, so DefWindowProc does not
                    // start a second SC_KEYMENU after our popup closes.
                    continue;
                }
                if message.message == WM_KEYDOWN && message.wParam == 9 && GetKeyState(0x11) < 0 {
                    let direction = if GetKeyState(0x10) < 0 { 3 } else { 1 };
                    SetFocus(state.tabs.handle);
                    state.show_page((state.tabs.active + direction) % 4);
                    continue;
                }
                if message.message == WM_KEYDOWN && message.wParam == 9 && GetKeyState(0x12) >= 0 {
                    // Read-only multiline controls may claim Tab themselves.
                    // Keep form traversal explicit; native date/arrow editing
                    // is untouched and the viewport reveals the new focus.
                    let next =
                        GetNextDlgTabItem(hwnd, message.hwnd, i32::from(GetKeyState(0x10) < 0));
                    if !next.is_null() {
                        SetFocus(next);
                        continue;
                    }
                }
            }
            // SAFETY: The MSG was initialized by GetMessageW on this thread.
            unsafe {
                if IsDialogMessageW(hwnd, &message) == 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        match state.error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    })();
    // Destroy HWND/children before State's fonts/icons and the class are released,
    // including every initialization-error and GetMessage-error return path.
    drop(window);
    drop(class_guard);
    drop(state);
    result
}

struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: This wrapper is UI-thread-local. The window may already have
        // been destroyed by Exit; IsWindow avoids a second destruction then.
        unsafe {
            if IsWindow(self.0) != 0 {
                DestroyWindow(self.0);
            }
        }
    }
}

struct WindowClass {
    instance: windows_sys::Win32::Foundation::HINSTANCE,
    name: Vec<u16>,
}
impl Drop for WindowClass {
    fn drop(&mut self) {
        // SAFETY: All windows of this owned class have been destroyed first.
        unsafe { UnregisterClassW(self.name.as_ptr(), self.instance) };
    }
}

fn show_window(hwnd: HWND) {
    // SAFETY: Called only for our live top-level window, outside State borrows.
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        let maximize = !state.is_null() && (*state).preferences.maximize_on_show;
        if !state.is_null() {
            (*state).preferences.maximize_on_show = false;
        }
        ShowWindow(
            hwnd,
            if maximize {
                SW_SHOWMAXIMIZED
            } else if IsIconic(hwnd) != 0 {
                SW_RESTORE
            } else {
                SW_SHOW
            },
        );
        if SetForegroundWindow(hwnd) == 0 {
            let info = FLASHWINFO {
                cbSize: size_of::<FLASHWINFO>() as u32,
                hwnd,
                dwFlags: FLASHW_TRAY,
                uCount: 3,
                dwTimeout: 0,
            };
            FlashWindowEx(&info);
        }
    }
}

fn show_error(hwnd: HWND, text: &str) {
    let text = wide(text);
    let title = wide("AI Relay 开发版");
    // SAFETY: Both strings live through this modal call; no State borrow is held
    // because MessageBox runs a nested message loop.
    unsafe { MessageBoxW(hwnd, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING) };
}

fn tray_menu(hwnd: HWND) -> Result<(), String> {
    let menu = Menu::new()?;
    let show = wide("显示窗口(&S)");
    let exit = wide("退出(&X)");
    // SAFETY: menu is owned and the strings remain live through AppendMenuW.
    unsafe {
        if AppendMenuW(menu.0, MF_STRING, SHOW_COMMAND as usize, show.as_ptr()) == 0
            || AppendMenuW(menu.0, MF_SEPARATOR, 0, null()) == 0
            || AppendMenuW(menu.0, MF_STRING, EXIT_COMMAND as usize, exit.as_ptr()) == 0
        {
            return Err(last_error("添加托盘菜单项"));
        }
    }
    let mut point = POINT::default();
    // SAFETY: point is writable, hwnd is live, and no State borrow is held while
    // TrackPopupMenu runs its nested message loop. The menu lives until it returns.
    unsafe {
        if GetCursorPos(&mut point) == 0 {
            return Err(last_error("获取菜单位置"));
        }
        SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu.0,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        match command as u32 {
            SHOW_COMMAND => show_window(hwnd),
            EXIT_COMMAND => {
                PostMessageW(hwnd, WM_COMMAND, EXIT_COMMAND as usize, 0);
            }
            _ => {}
        }
    }
    Ok(())
}

unsafe fn update_title_menu(hwnd: HWND, state: *mut State) {
    // SAFETY: Owner-thread State; IsWindowEnabled is a scalar native query, no
    // borrowed state crosses a menu tracking loop or controller operation.
    unsafe {
        let enabled = |id| {
            let handle = GetDlgItem((*state).viewport.handle, id);
            !handle.is_null() && settings::is_enabled(handle) != 0
        };
        let ready = (*state).exiting.is_none();
        let _ = title_menu::update(
            hwnd,
            title_menu::Availability {
                start: ready && enabled(tabs::START as i32),
                stop: ready && enabled(tabs::STOP as i32),
                restart: ready && (*state).settings.overview_available(),
                export: ready && enabled(2034),
                import: ready && enabled(2035),
                active_page: (*state).tabs.active,
            },
        );
    }
}

unsafe fn dispatch_title_menu(hwnd: HWND, state: *mut State, action: title_menu::Action) {
    use title_menu::Action;
    // SAFETY: Menus reuse existing commands and confirmation paths. No State
    // reference is held over a dialog, clipboard operation or async command.
    unsafe {
        if (*state).exiting.is_some() {
            return;
        }
        let command = match action {
            Action::OpenDir => overview::OPEN_DIRECTORY,
            Action::ExportConfig => 2034,
            Action::ImportConfig => 2035,
            Action::Exit => EXIT_COMMAND,
            Action::Start => tabs::START,
            Action::Stop => tabs::STOP,
            Action::Restart => overview::RESTART,
            Action::ViewOverview
            | Action::ViewHistory
            | Action::ViewLogs
            | Action::ViewSettings => {
                let page = match action {
                    Action::ViewOverview => 0,
                    Action::ViewHistory => 1,
                    Action::ViewLogs => 2,
                    _ => 3,
                };
                SetFocus((*state).tabs.handle);
                (*state).show_page(page);
                return;
            }
            Action::About => {
                let text = wide(concat!(
                    "AI Relay ",
                    env!("CARGO_PKG_VERSION"),
                    " · 开发预览\nRust + Win32 原生流式代理\n\n标题栏 ☰：文件 / 代理 / 视图 / 帮助\nF10 或 Alt+M 打开；Alt+Space 保留系统菜单。\n关闭窗口依照设置隐藏到托盘或退出。\n统计与流量观测不是 Provider 账单。"
                ));
                MessageBoxW(
                    hwnd,
                    text.as_ptr(),
                    wide("关于 AI Relay").as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
                return;
            }
        };
        // Check live availability again; a stale menu selection cannot bypass a
        // disabled command after background state changed during menu tracking.
        if action == Action::Restart && !(*state).settings.overview_available() {
            return;
        }
        if !matches!(action, Action::Exit | Action::OpenDir | Action::Restart) {
            let id = match action {
                Action::Start => tabs::START as i32,
                Action::Stop => tabs::STOP as i32,
                Action::Restart => overview::RESTART as i32,
                _ => command as i32,
            };
            let target = GetDlgItem((*state).viewport.handle, id);
            if target.is_null() || settings::is_enabled(target) == 0 {
                return;
            }
        }
        PostMessageW(hwnd, WM_COMMAND, command as usize, 0);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Win32 supplies a valid CREATESTRUCTW during WM_NCCREATE. The caller
    // retains the boxed State until after window destruction, on this same thread.
    if message == WM_NCCREATE {
        unsafe {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        return 1;
    }
    // EDIT updates/focus changes synchronously notify the parent while Settings
    // may be mutably borrowed. Ignore those messages before accessing State.
    if message == WM_COMMAND {
        let notification = wparam >> 16;
        if notification != 0 && notification != CBN_SELCHANGE as usize {
            return 0;
        }
    }
    // SAFETY: GWLP_USERDATA is either zero or our stable, non-owning State pointer.
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
    if state.is_null() {
        // SAFETY: Forward untouched Win32 callback arguments to the default handler.
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    // SAFETY: Valid WM_DRAWITEM payload, handled before theme's generic owner drawing.
    if message == WM_DRAWITEM
        && unsafe { history::draw(state, lparam) || overview::draw(state, lparam) }
    {
        return 1;
    }
    // SAFETY: Paint handlers access only copied values/raw fields, before mutable borrows.
    if let Some(result) = unsafe { theme::paint(state, message, wparam, lparam) } {
        return result;
    }
    // SAFETY: State lives on this UI thread. References below are deliberately
    // short-lived: no borrow crosses modal UI, main-window resizing, showing,
    // destruction, or default dispatch, all of which can reenter this procedure.
    unsafe {
        if message == (*state).taskbar_created {
            (*state).tray_present = false;
            let result = (*state).add_tray(hwnd);
            if let Err(error) = result {
                show_window(hwnd);
                show_error(hwnd, &error);
            }
            return 0;
        }
        match message {
            WM_SETTINGCHANGE | WM_THEMECHANGED => {
                PostMessageW(hwnd, theme::REFRESH, 0, 0);
                0
            }
            theme::REFRESH => {
                theme::refresh(hwnd, state);
                0
            }
            WM_NOTIFY => {
                if choices::notify(lparam) || date_picker::notify(lparam) {
                    return 0;
                }
                let notification = &*(lparam as *const NMHDR);
                if notification.hwndFrom == (*state).tabs.handle
                    && notification.code == TCN_SELCHANGE
                {
                    let page = (*state).tabs.selection();
                    SetFocus((*state).tabs.handle);
                    (*state).show_page(page);
                }
                0
            }
            WM_MOUSEWHEEL => SendMessageW((*state).viewport.handle, message, wparam, lparam),
            WM_SIZE => {
                (*state).layout(hwnd, false);
                0
            }
            WM_EXITSIZEMOVE | PLACEMENT_CHANGED => {
                (*state).preferences.changed();
                if message == WM_EXITSIZEMOVE {
                    (*state).repaint_all(hwnd);
                }
                overview::flush(hwnd, state, message == WM_EXITSIZEMOVE);
                0
            }
            WM_INITMENUPOPUP => {
                update_title_menu(hwnd, state);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam as u32 == HTSYSMENU => {
                // A menu-looking caption icon must not unexpectedly close the app.
                PostMessageW(hwnd, OPEN_TITLE_MENU, 0, 0);
                0
            }
            OPEN_TITLE_MENU => {
                update_title_menu(hwnd, state);
                if let Err(error) = title_menu::show(hwnd) {
                    show_error(hwnd, &error);
                }
                0
            }
            WM_SYSCOMMAND => {
                if let Some(action) = title_menu::action(wparam as u32) {
                    dispatch_title_menu(hwnd, state, action);
                    return 0;
                }
                if matches!(
                    wparam as u32 & 0xfff0,
                    SC_MAXIMIZE | SC_RESTORE | SC_MINIMIZE
                ) {
                    PostMessageW(hwnd, PLACEMENT_CHANGED, 0, 0);
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_DPICHANGED => {
                let dpi = (wparam & 0xffff) as u32;
                let result = (*state).update_dpi(hwnd, dpi.max(96));
                if let Err(error) = result {
                    (*state).error = Some(error);
                    DestroyWindow(hwnd);
                } else {
                    // WM_DPICHANGED provides a RECT valid throughout this callback.
                    let mut rect = *(lparam as *const RECT);
                    if let Some(work) = overview::rect_work_area(&rect) {
                        let width = (rect.right - rect.left).min(work.right - work.left).max(1);
                        let height = (rect.bottom - rect.top).min(work.bottom - work.top).max(1);
                        rect.left = rect.left.clamp(work.left, work.right - width);
                        rect.top = rect.top.clamp(work.top, work.bottom - height);
                        rect.right = rect.left + width;
                        rect.bottom = rect.top + height;
                    }
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    (*state).preferences.changed();
                }
                0
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam as *mut MINMAXINFO);
                let work = overview::work_area(hwnd);
                info.ptMinTrackSize.x =
                    scale(800, (*state).dpi).min(work.map_or(i32::MAX, |r| r.right - r.left));
                info.ptMinTrackSize.y =
                    scale(560, (*state).dpi).min(work.map_or(i32::MAX, |r| r.bottom - r.top));
                0
            }
            WM_CLOSE => {
                if (*state).exiting.is_some() {
                    return 0;
                }
                if !(*state).settings.close_to_tray() {
                    PostMessageW(hwnd, WM_COMMAND, EXIT_COMMAND as usize, 0);
                    return 0;
                }
                if !(*state).tray_present {
                    show_error(
                        hwnd,
                        "托盘图标不可用，窗口将保持显示。请重启 Explorer 后重试，或结束本开发版进程。",
                    );
                } else {
                    if !(*state).close_notice_shown {
                        (*state).close_notice_shown = true;
                        let text = wide(
                            "关闭窗口只会隐藏到托盘，正在运行的代理继续转发。双击托盘图标可恢复，右键菜单可停止并退出。",
                        );
                        let title = wide("AI Relay 开发版");
                        MessageBoxW(
                            hwnd,
                            text.as_ptr(),
                            title.as_ptr(),
                            MB_OK | MB_ICONINFORMATION,
                        );
                    }
                    // A nested message loop may have processed an Exit command.
                    if IsWindow(hwnd) != 0 && (*state).tray_present && (*state).exiting.is_none() {
                        overview::flush(hwnd, state, true);
                        ShowWindow(hwnd, SW_HIDE);
                    }
                }
                0
            }
            TRAY_MESSAGE => {
                let event = (lparam as u32) & 0xffff;
                let id = ((lparam as u32) >> 16) & 0xffff;
                if id == TRAY_ID {
                    match event {
                        WM_LBUTTONDBLCLK | NIN_KEYSELECT => show_window(hwnd),
                        WM_CONTEXTMENU => {
                            if let Err(error) = tray_menu(hwnd) {
                                show_error(hwnd, &error);
                            }
                        }
                        _ => {}
                    }
                }
                0
            }
            WM_COMMAND => {
                let command = (wparam & 0xffff) as u32;
                if command == MENU_BUTTON {
                    PostMessageW(hwnd, OPEN_TITLE_MENU, 0, 0);
                    return 0;
                }
                if command == EXIT_COMMAND && (*state).exiting.is_none() {
                    // Save before setting the controller's exit flag: send() rejects
                    // commands after request_exit. The timer waits for acknowledgement,
                    // bounded to two seconds, then begins ordinary graceful shutdown.
                    overview::flush(hwnd, state, true);
                    if (*state).preferences.needs_flush() {
                        (*state).preferences.exit_flush_deadline =
                            Some(Instant::now() + std::time::Duration::from_secs(2));
                    } else {
                        (*state).controller.request_exit();
                    }
                    let wait = (*state).settings.exit_wait_secs().saturating_add(2);
                    (*state).exiting.get_or_insert_with(|| {
                        Instant::now() + std::time::Duration::from_secs(wait)
                    });
                    (*state).settings.disable();
                    (*state).settings.sync_overview(&(*state).tabs);
                    settings::set_text(
                        (*state).status_label,
                        &format!(
                            "正在停止代理并退出（按已保存宽限期，最多等待 {wait} 秒；UI 安全上限 3600 秒）……"
                        ),
                    );
                } else if (*state).exiting.is_none() {
                    let command = match command {
                        tabs::START => settings::START,
                        tabs::STOP => settings::STOP,
                        other => other,
                    };
                    if (overview::RESTART..=overview::OPEN_DIRECTORY).contains(&command) {
                        overview::command(hwnd, state, command);
                    } else if (2300..=2399).contains(&command) {
                        history::command(hwnd, state, command);
                    } else if (2201..=2229).contains(&command) {
                        (*state).logs.handle(&(*state).controller, command);
                    } else {
                        settings::handle_command(hwnd, state, command);
                    }
                }
                0
            }
            WM_TIMER => {
                settings::poll(hwnd, state);
                overview::poll(hwnd, state);
                if let Some(deadline) = (*state).preferences.exit_flush_deadline
                    && (!(*state).preferences.needs_flush() || Instant::now() >= deadline)
                {
                    (*state).preferences.exit_flush_deadline = None;
                    (*state).controller.request_exit();
                }
                logs::poll(hwnd, state);
                (*state).history.poll(&(*state).controller);
                if let Some(policy) = (*state).history.take_applied_policy() {
                    (*state).settings.sync_history(policy);
                }
                let signals = Arc::clone(&(*state).signals);
                if signals.show.swap(false, Ordering::AcqRel) {
                    show_window(hwnd);
                }
                if signals.quit.swap(false, Ordering::AcqRel) {
                    PostMessageW(hwnd, WM_COMMAND, EXIT_COMMAND as usize, 0);
                }
                if let Some(deadline) = (*state).exiting
                    && ((*state).controller.is_finished() || Instant::now() >= deadline)
                {
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_QUERYENDSESSION => 1,
            WM_ENDSESSION => {
                if wparam != 0 {
                    // 系统会话结束不等待 GUI 消息或磁盘；进程退出是最终资源边界。
                    overview::flush(hwnd, state, true);
                    (*state).controller.request_exit();
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_DESTROY => {
                (*state).controller.request_exit();
                KillTimer(hwnd, 1);
                (*state).remove_tray(hwnd);
                PostQuitMessage(0);
                0
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests;
