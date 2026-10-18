use super::*;
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;

/// Hidden, isolated native State: no tray, message pump, timer, network or
/// user configuration. The controller is stopped before any UI command runs.
pub(super) struct NativeState {
    pub(super) state: Box<State>,
    pub(super) hwnd: HWND,
    directory: std::path::PathBuf,
}

impl NativeState {
    pub(super) fn new() -> Self {
        use std::sync::{Once, atomic::AtomicU64, mpsc};
        static REGISTER: Once = Once::new();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "ai-relay-ui-state-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let signals = Arc::new(Signals::default());
        let (_sender, receiver) = mpsc::channel();
        let controller = Controller::spawn(
            Options {
                config_dir: Some(directory.clone()),
                background: true,
                ..Options::default()
            },
            receiver,
            signals.clone(),
        )
        .unwrap();
        controller.request_exit();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while !controller.is_finished() {
            assert!(Instant::now() < deadline, "isolated controller shutdown");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let state = Box::new(State {
            icon: Icon::load().unwrap(),
            font: Font::new(96).unwrap(),
            heading_font: Font::heading(96).unwrap(),
            theme: theme::Theme::new(crate::config::ThemeMode::Dark).unwrap(),
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
            controller,
            exiting: None,
            dpi: 96,
            page_initialized: false,
            page_geometry: [false; 4],
            page_positioned: [false; 4],
            pages_prelaid: false,
            layout_size: (0, 0, 0),
            taskbar_created: u32::MAX,
            tray_present: false,
            close_notice_shown: false,
            error: None,
            signals,
        });
        // SAFETY: Process-lifetime test class; each hidden HWND has a stable
        // boxed State owned on its creating thread, with no timer or modal UI.
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = windows_sys::w!("AIRelay.State.Tests");
            REGISTER.call_once(|| {
                let class = WNDCLASSW {
                    lpfnWndProc: Some(window_proc),
                    hInstance: instance,
                    lpszClassName: class,
                    ..Default::default()
                };
                assert_ne!(RegisterClassW(&class), 0);
            });
            let mut fixture = Self {
                state,
                hwnd: null_mut(),
                directory,
            };
            fixture.hwnd = CreateWindowExW(
                0,
                class,
                null(),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                -4000,
                -4000,
                800,
                680,
                null_mut(),
                null_mut(),
                instance,
                (&mut *fixture.state as *mut State).cast(),
            );
            assert!(!fixture.hwnd.is_null());
            let state = &mut fixture.state;
            for text in ["overview", "isolated status", "menu", "icon", "brand"] {
                let h = CreateWindowExW(
                    0,
                    windows_sys::w!("STATIC"),
                    wide(text).as_ptr(),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    1,
                    1,
                    fixture.hwnd,
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!h.is_null());
                state.labels.push(h);
            }
            state.status_label = state.labels[1];
            state.menu_button = state.labels[2];
            state.brand_icon = state.labels[3];
            state.brand_label = state.labels[4];
            state.labels.truncate(2);
            state.tabs.create(fixture.hwnd).unwrap();
            state.overview.create(fixture.hwnd).unwrap();
            state.settings.create(fixture.hwnd, instance).unwrap();
            state.logs.create(fixture.hwnd, instance).unwrap();
            state.history.create(fixture.hwnd).unwrap();
            state.viewport.create(fixture.hwnd).unwrap();
            state
                .viewport
                .adopt(
                    fixture.hwnd,
                    &[
                        state.tabs.handle,
                        state.status_label,
                        state.menu_button,
                        state.brand_icon,
                        state.brand_label,
                    ],
                )
                .unwrap();
            state.show_page(0);
            fixture
        }
    }

    pub(super) fn control(&self, id: u32) -> HWND {
        // SAFETY: All page controls are live direct viewport children.
        let child = unsafe { GetDlgItem(self.state.viewport.handle, id as i32) };
        assert!(!child.is_null(), "native control {id}");
        child
    }

    pub(super) fn command(&mut self, id: u32, selection: Option<usize>) {
        // SAFETY: Real scalar WM_COMMAND path; a stopped controller prevents
        // background I/O. No application borrow crosses native dispatch.
        unsafe {
            if let Some(selection) = selection {
                let control = GetDlgItem(self.state.viewport.handle, id as i32);
                assert!(!control.is_null());
                SendMessageW(control, CB_SETCURSEL, selection, 0);
            }
            SendMessageW(self.hwnd, WM_COMMAND, id as usize, 0);
        }
    }

    pub(super) fn scroll(&self, command: i32) -> i32 {
        // SAFETY: Live test-owned viewport; all arguments are copied scalars.
        unsafe {
            SendMessageW(self.state.viewport.handle, WM_VSCROLL, command as usize, 0);
            GetScrollPos(self.state.viewport.handle, SB_VERT)
        }
    }

    pub(super) fn rect(&self, child: HWND) -> RECT {
        let mut r = RECT::default();
        // SAFETY: Live test-owned HWNDs and writable local geometry.
        unsafe {
            assert_ne!(GetWindowRect(child, &mut r), 0);
            let mut p = POINT {
                x: r.left,
                y: r.top,
            };
            ScreenToClient(self.state.viewport.handle, &mut p);
            RECT {
                left: p.x,
                top: p.y,
                right: p.x + r.right - r.left,
                bottom: p.y + r.bottom - r.top,
            }
        }
    }
}

impl Drop for NativeState {
    fn drop(&mut self) {
        // SAFETY: Destroy before fonts/State drop. Detaching avoids posting
        // WM_QUIT from production WM_DESTROY into the test runner's thread.
        unsafe {
            if !self.hwnd.is_null() {
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                DestroyWindow(self.hwnd);
            }
        }
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

#[test]
fn native_initial_log_cache_uses_its_own_minimum_height() {
    let mut native = NativeState::new();
    for dpi in [96, 144, 192] {
        native.state.dpi = dpi;
        native.state.pages_prelaid = false;
        native.state.show_page(0);
        assert!(native.state.page_geometry.iter().all(|valid| *valid));
        native.state.show_page(2);
        // SAFETY: A live direct child of this test's viewport.
        let editor = unsafe { GetDlgItem(native.state.viewport.handle, logs::CONTENT as i32) };
        assert!(!editor.is_null());
        let first = native.rect(editor);
        assert_eq!(first.bottom - first.top, scale(96, dpi));
        native.state.layout(native.hwnd, true);
        let forced = native.rect(editor);
        assert_eq!(first.top, forced.top);
        assert_eq!(first.bottom, forced.bottom);
        native.state.show_page(3);
        native.state.show_page(2);
        let cached = native.rect(editor);
        assert_eq!(cached.top, first.top);
        assert_eq!(cached.bottom, first.bottom);
    }
}

#[test]
fn native_size_category_and_cached_tab_changes_damage_visible_children() {
    use windows_sys::Win32::Graphics::Gdi::{GetUpdateRect, ValidateRect};
    let mut native = NativeState::new();
    native.state.show_page(3);
    let save = native.control(2001);
    // SAFETY: Off-screen test window only; no app launch or event loop.
    unsafe {
        ShowWindow(native.hwnd, SW_SHOWNOACTIVATE);
    }
    for category in [1, 4, 0] {
        // SAFETY: Owner-thread live child, clear prior damage to test this
        // transaction, rather than accepting leftovers from initial show.
        unsafe {
            ValidateRect(save, null());
        }
        native.command(2039, Some(category));
        assert_ne!(unsafe { GetUpdateRect(save, null_mut(), 0) }, 0);
    }
    // A programmatic WM_SIZE must work without a later WM_EXITSIZEMOVE.
    unsafe {
        ValidateRect(save, null());
        SetWindowPos(
            native.hwnd,
            null_mut(),
            0,
            0,
            820,
            700,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        assert_ne!(GetUpdateRect(save, null_mut(), 0), 0);
    }
    native.state.show_page(0);
    unsafe {
        ValidateRect(save, null());
    }
    native.state.show_page(3);
    assert!(native.state.page_positioned[3] && native.state.page_geometry[3]);
    assert_ne!(unsafe { GetUpdateRect(save, null_mut(), 0) }, 0);
}

#[test]
fn native_history_commands_rearrange_revealed_controls_without_drift() {
    let mut native = NativeState::new();
    native.state.show_page(1);
    let offset = native.scroll(SB_LINEDOWN);
    assert!(offset > 0);
    let view = native.control(history::VIEW);
    let top = native.rect(view).top;
    let draft = native.control(2304);
    settings::set_text(draft, "127.0.0.2");
    for _ in 0..3 {
        native.command(2300, Some(1));
        assert!(native.state.history.session);
        assert!(native.state.page_positioned[1]);
        assert!(!native.state.page_geometry[3]);
        // SAFETY: Live usage editor owned by this fixture's viewport.
        let usage = native.rect(unsafe { GetDlgItem(native.state.viewport.handle, 2020) });
        assert_eq!(usage.top, 110 - HEADER_HEIGHT - offset);
        native.command(2300, Some(0));
        assert!(!native.state.history.session);
        for selection in [2, 3, 0, 1, 2] {
            native.command(2338, Some(selection));
            let result = native.control(match selection {
                2 => 2330,
                3 => 2331,
                _ => 2337,
            });
            assert_eq!(native.rect(result).top, 592 - HEADER_HEIGHT - offset);
            assert_eq!(native.rect(view).top, top);
            native.state.show_page(0);
            native.state.show_page(1);
            assert_eq!(native.rect(result).top, 592 - HEADER_HEIGHT - offset);
            assert_eq!(native.scroll(SB_ENDSCROLL), offset);
        }
        native.command(2301, Some(4));
        let date = native.control(2302);
        assert_eq!(native.rect(date).top, 136 - HEADER_HEIGHT - offset);
        native.command(2317, None);
        // SAFETY: Child remains live; its own visibility is meaningful with
        // the top-level fixture hidden.
        assert_eq!(
            unsafe { GetWindowLongPtrW(date, GWL_STYLE) as u32 & WS_VISIBLE },
            0
        );
        native.command(2301, Some(4));
        assert_eq!(native.rect(date).top, 136 - HEADER_HEIGHT - offset);
        settings::set_text(draft, "127.0.0.2");
        for command in [2318, 2318, 2339, 2339] {
            native.command(command, None);
            assert_eq!(native.rect(view).top, top);
            assert_eq!(native.scroll(SB_ENDSCROLL), offset);
            assert_eq!(settings::text(draft), "127.0.0.2");
        }
    }
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn brand_header_fits_before_all_four_tabs_at_minimum_width() {
    assert!(BRAND_LEFT + BRAND_ICON_SIZE <= BRAND_TEXT_LEFT);
    assert!(BRAND_TEXT_LEFT + BRAND_TEXT_WIDTH <= TAB_LEFT);
    assert!(TAB_LEFT + 4 * 112 <= 800 - 24);
    assert!(BRAND_ICON_SIZE <= HEADER_HEIGHT);
}
