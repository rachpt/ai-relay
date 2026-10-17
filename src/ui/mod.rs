//! Win32 设置与代理控制；所有磁盘和网络初始化由后台控制器执行。
mod files;
mod history;
mod logs;
mod settings;
mod tabs;
mod theme;
pub(crate) mod usage;
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

#[link(name = "user32")]
unsafe extern "system" {
    fn SetFocus(hwnd: HWND) -> HWND;
}

struct State {
    icon: Icon,
    font: Font,
    heading_font: Font,
    theme: theme::Theme,
    labels: Vec<HWND>,
    status_label: HWND,
    settings: settings::Settings,
    tabs: tabs::Tabs,
    logs: logs::Logs,
    history: history::History,
    controller: Controller,
    exiting: Option<Instant>,
    dpi: u32,
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

    fn layout(&self, hwnd: HWND) {
        let mut rect = RECT::default();
        // SAFETY: hwnd is live and rect is a writable RECT.
        if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
            return;
        }
        let margin = scale(24, self.dpi);
        let width = (rect.right - margin * 2).max(1);
        let rows = [
            (64, 28),
            (rect.bottom * 96 / self.dpi as i32 - 84, 76),
            (112, 64),
            (250, 64),
        ];
        for (label, (top, height)) in self.labels.iter().zip(rows) {
            // SAFETY: Each handle is a live child STATIC; this moves only the child,
            // not the parent, so it cannot recursively resize the parent.
            unsafe {
                MoveWindow(
                    *label,
                    margin,
                    scale(top, self.dpi),
                    width,
                    scale(height, self.dpi),
                    1,
                )
            };
        }
        self.tabs.layout(self.dpi, rect.right);
        self.settings.layout(self.dpi, width, rect.bottom);
        self.logs.layout(self.dpi, rect.right, rect.bottom);
        self.history.layout(self.dpi, rect.right, rect.bottom);
    }

    fn show_page(&mut self, index: usize) {
        self.tabs.select(index);
        self.settings.show_page(index);
        self.history.show(index == 1);
        self.settings
            .show_session(index == 1 && self.history.session);
        self.logs.show(index == 2);
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
    }

    fn update_dpi(&mut self, hwnd: HWND, dpi: u32) -> Result<(), String> {
        let font = Font::new(dpi)?;
        for label in &self.labels {
            // SAFETY: STATIC controls accept WM_SETFONT and do not call our parent
            // procedure for this message. Both the old and new fonts remain live.
            unsafe { SendMessageW(*label, WM_SETFONT, font.0 as WPARAM, 1) };
        }
        self.settings.set_font(font.0);
        self.tabs.set_font(font.0);
        self.logs.set_font(font.0);
        self.history.set_font(font.0);
        let heading = Font::heading(dpi)?;
        if let Some(label) = self.labels.first() {
            // SAFETY: Live STATIC; both fonts remain alive until replacement completes.
            unsafe { SendMessageW(*label, WM_SETFONT, heading.0 as WPARAM, 0) };
        }
        self.heading_font = heading;
        self.font = font;
        self.dpi = dpi;
        self.layout(hwnd);
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
        labels: Vec::new(),
        status_label: null_mut(),
        settings: settings::Settings::default(),
        tabs: tabs::Tabs::default(),
        logs: logs::Logs::default(),
        history: history::History::default(),
        controller: Controller::spawn(options, ipc, signals.clone())?,
        signals,
        exiting: None,
        dpi,
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
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(800, dpi),
            scale(900, dpi),
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
        let texts = [
            "AI Relay 开发版",
            "代理已停止；正在后台加载配置。",
            "关闭窗口隐藏到托盘，不停止代理；双击可恢复，右键可退出。\r\n保存并应用会受控重启；启动仅使用已保存配置，不使用草稿。",
            "DPAPI 仅当前用户；登录自启只由手动确认操作修改。\r\n目录迁移受后台安全检查限制；不修改系统代理或防火墙。",
        ];
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
        state.tabs.create(hwnd)?;
        state.settings.create(hwnd, instance)?;
        state.logs.create(hwnd, instance)?;
        state.history.create(hwnd)?;
        state.show_page(0);
        state.update_dpi(hwnd, dpi)?;
        // SAFETY: Stable boxed state; refresh holds no State reference across Win32 calls.
        unsafe { theme::refresh(hwnd, &mut *state) };
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
                if message.message == WM_KEYDOWN && message.wParam == 9 && GetKeyState(0x11) < 0 {
                    let direction = if GetKeyState(0x10) < 0 { 3 } else { 1 };
                    SetFocus(state.tabs.handle);
                    state.show_page((state.tabs.active + direction) % 4);
                    continue;
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
        ShowWindow(
            hwnd,
            if IsIconic(hwnd) != 0 {
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
    if message == WM_DRAWITEM && unsafe { history::draw(state, lparam) } {
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
            WM_SIZE => {
                (*state).layout(hwnd);
                0
            }
            WM_DPICHANGED => {
                let dpi = (wparam & 0xffff) as u32;
                let result = (*state).update_dpi(hwnd, dpi.max(96));
                if let Err(error) = result {
                    (*state).error = Some(error);
                    DestroyWindow(hwnd);
                } else {
                    // WM_DPICHANGED provides a RECT valid throughout this callback.
                    let rect = *(lparam as *const RECT);
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                0
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam as *mut MINMAXINFO);
                info.ptMinTrackSize.x = scale(760, (*state).dpi);
                info.ptMinTrackSize.y = scale(880, (*state).dpi);
                0
            }
            WM_CLOSE => {
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
                    if IsWindow(hwnd) != 0 && (*state).tray_present {
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
                if command == EXIT_COMMAND {
                    (*state).controller.request_exit();
                    let wait = (*state).settings.exit_wait_secs();
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
                    if (2300..=2324).contains(&command) {
                        history::command(hwnd, state, command);
                    } else if (2201..=2219).contains(&command) || command == 2222 || command == 2223
                    {
                        (*state).logs.handle(&(*state).controller, command);
                    } else {
                        settings::handle_command(hwnd, state, command);
                    }
                }
                0
            }
            WM_TIMER => {
                settings::poll(hwnd, state);
                logs::poll(hwnd, state);
                (*state).history.poll(&(*state).controller);
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
