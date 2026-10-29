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
            layout_size: (0, 0, 0, 0),
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
        assert_eq!(first.bottom - first.top, scale(240, dpi));
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

fn resize_client(native: &mut NativeState, width: i32, height: i32, dpi: u32) {
    native.state.dpi = dpi;
    // SAFETY: Hidden test-owned top-level. Compute its current native frame
    // rather than assuming title-bar metrics or changing the real monitor DPI.
    unsafe {
        let mut outer = RECT::default();
        let mut client = RECT::default();
        assert_ne!(GetWindowRect(native.hwnd, &mut outer), 0);
        assert_ne!(GetClientRect(native.hwnd, &mut client), 0);
        assert_ne!(
            SetWindowPos(
                native.hwnd,
                null_mut(),
                0,
                0,
                scale(width, dpi) + outer.right - outer.left - client.right,
                scale(height, dpi) + outer.bottom - outer.top - client.bottom,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            ),
            0
        );
    }
    native.state.layout(native.hwnd, true);
}

fn assert_bottom_padding(native: &NativeState, tail: u32) {
    // SAFETY: Live test-owned footer and viewport, actual full HWND rectangles;
    // no clipping/intersection can mask a tail behind the fixed status bar.
    unsafe {
        let mut host = RECT::default();
        GetClientRect(native.state.viewport.handle, &mut host);
        let footer = native.rect(native.state.status_label);
        let last = native.rect(native.control(tail));
        assert_eq!(footer.top, host.bottom, "viewport must meet the footer");
        assert_eq!(
            footer.top - last.bottom,
            scale(viewport::BOTTOM_PADDING, native.state.dpi),
            "page {} tail {tail} has an extra gap or clips the footer",
            native.state.tabs.active
        );
        assert!(last.top < host.bottom && last.bottom > 0 && last.bottom <= host.bottom);
    }
}

#[test]
fn native_stretched_pages_share_bottom_padding_at_short_tall_and_threshold_sizes() {
    let mut native = NativeState::new();
    for dpi in [96, 144, 192] {
        for height in [560, 680, 792, 819, 820, 848, 1000] {
            resize_client(&mut native, 1000, height, dpi);
            for (page, tail) in [(0, 2109), (1, 2337), (2, 2221)] {
                native.state.show_page(page);
                native.scroll(SB_BOTTOM);
                assert_bottom_padding(&native, tail);
                let offset = native.scroll(SB_ENDSCROLL);
                native.state.layout(native.hwnd, true);
                assert_bottom_padding(&native, tail);
                native.state.show_page(3);
                native.state.show_page(page);
                assert_eq!(native.scroll(SB_ENDSCROLL), offset);
                assert_bottom_padding(&native, tail);
            }
        }
    }
}

#[test]
fn native_history_subviews_keep_the_full_tail_above_the_footer() {
    let mut native = NativeState::new();
    for dpi in [96, 144, 192] {
        for height in [560, 820, 1000] {
            resize_client(&mut native, 1000, height, dpi);
            native.state.show_page(1);
            native.command(2300, Some(0));
            for advanced in [false, true] {
                if advanced {
                    native.command(2318, None);
                }
                for (view, tail) in [(0, 2337), (1, 2337), (2, 2330), (3, 2331)] {
                    native.command(2338, Some(view));
                    native.scroll(SB_BOTTOM);
                    assert_bottom_padding(&native, tail);
                }
            }
            native.command(2318, None);
            native.command(2339, None);
            native.scroll(SB_BOTTOM);
            assert_bottom_padding(&native, 2325);
            let storage = native.rect(native.control(2351));
            let status = native.rect(native.control(2325));
            assert_eq!(status.top - storage.bottom, scale(8, dpi));
            native.command(2339, None);
            native.command(2300, Some(1));
            native.scroll(SB_BOTTOM);
            assert_bottom_padding(&native, 2020);
            native.state.show_page(0);
            native.state.show_page(1);
            assert_bottom_padding(&native, 2020);
            native.command(2300, Some(0));
        }
    }
}

#[test]
fn native_settings_scroll_extent_tracks_category_instead_of_an_empty_page() {
    let mut native = NativeState::new();
    native.state.show_page(3);
    let draft = native.control(2040);
    settings::set_text(draft, "[\"127.0.0.1\"]");
    let assert_widths = |native: &NativeState, category: usize, dpi: u32| {
        let mut host = RECT::default();
        // SAFETY: Measure the actual host client after native scrollbar visibility settles.
        assert_ne!(
            unsafe { GetClientRect(native.state.viewport.handle, &mut host) },
            0
        );
        let right = host.right - scale(24, dpi);
        for (id, left, cap) in [(2039, 24, 864), (2030, 180, 456)] {
            if id == 2030 && category != 5 {
                continue;
            }
            let rect = native.rect(native.control(id));
            assert_eq!(rect.left, scale(left, dpi));
            assert_eq!(
                rect.right,
                right.min(scale(left, dpi) + scale(cap, dpi)),
                "category {category}, segment {id} has a stale or uncapped width"
            );
        }
        // This input is laid out for every category, even while hidden. Its
        // uncapped width still catches both scrollbar show/hide regressions.
        let input = native.rect(native.control(2010));
        assert_eq!(input.left, scale(180, dpi));
        assert_eq!(
            input.right, right,
            "category {category} input used the previous scrollbar width"
        );
    };
    for dpi in [96, 144, 192] {
        for (width, height) in [
            (1000, 560),
            (800, 560),
            (1000, 680),
            (800, 680),
            (1000, 820),
            (800, 820),
            (1000, 1000),
            (800, 1000),
        ] {
            resize_client(&mut native, width, height, dpi);
            for category in [2, 0, 4, 1, 5, 3] {
                native.command(2039, Some(category));
                assert_widths(&native, category, dpi);
                let offset = native.scroll(SB_BOTTOM);
                let last = native.rect(native.control(2001));
                let footer = native.rect(native.state.status_label);
                let bottom = scale([326, 416, 640, 464, 594, 536][category], dpi);
                let expected = (bottom + scale(status_bar::HEIGHT + viewport::BOTTOM_PADDING, dpi)
                    - scale(height, dpi))
                .max(0);
                assert_eq!(
                    offset, expected,
                    "category {category} has phantom scroll space"
                );
                if offset > 0 {
                    assert_bottom_padding(&native, 2001);
                } else {
                    assert!(footer.top - last.bottom >= scale(viewport::BOTTOM_PADDING, dpi));
                }
                native.state.show_page(0);
                native.state.show_page(3);
                assert_eq!(native.scroll(SB_ENDSCROLL), expected);
                assert_eq!(native.rect(native.control(2001)).bottom, last.bottom);
                assert_widths(&native, category, dpi);
                assert_eq!(settings::text(draft), "[\"127.0.0.1\"]");
            }
        }
    }
}

#[test]
fn native_log_height_grip_preserves_content_selection_and_manual_dip_height() {
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCapture() -> HWND;
    }
    let mut native = NativeState::new();
    native.state.show_page(2);
    let editor = native.control(2200);
    let grip = native.control(2230);
    let text = (0..90)
        .map(|n| format!("synthetic {n} 日志文字\r\n"))
        .collect::<String>();
    settings::set_text(editor, &text);
    // SAFETY: Only hidden private HWNDs, local scalar messages. No clipboard,
    // live log service or user configuration is touched by this isolated fixture.
    unsafe {
        SendMessageW(editor, 0xb1, 3, 17);
        for dpi in [96, 144, 192] {
            resize_client(&mut native, 1000, 1000, dpi);
            let selected = SendMessageW(editor, 0xb0, 0, 0);
            let start = native.state.logs.viewer_height();
            let mut point = POINT {
                x: scale(50, dpi),
                y: scale(6, dpi),
            };
            let packed = |p: POINT| ((p.x as u16 as u32) | ((p.y as u16 as u32) << 16)) as LPARAM;
            SendMessageW(grip, WM_LBUTTONDOWN, 1, packed(point));
            assert_eq!(GetCapture(), grip);
            ClientToScreen(grip, &mut point);
            point.y += scale(48, dpi);
            for _ in 0..2 {
                let mut local = point;
                ScreenToClient(grip, &mut local);
                SendMessageW(grip, WM_MOUSEMOVE, 1, packed(local));
                assert_eq!(GetCapture(), grip);
                assert_eq!(native.state.logs.viewer_height(), start + 48);
                assert_eq!(
                    native.rect(editor).bottom - native.rect(editor).top,
                    scale(start + 48, dpi)
                );
            }
            SendMessageW(grip, WM_KEYDOWN, 0x1b, 0);
            assert_ne!(GetCapture(), grip);
            assert_eq!(native.state.logs.viewer_height(), start);
            for _ in 0..40 {
                SendMessageW(grip, WM_KEYDOWN, 0x28, 0);
            }
            assert_eq!(native.state.logs.viewer_height(), 720);
            native.scroll(SB_BOTTOM);
            assert_bottom_padding(&native, 2221);
            for _ in 0..40 {
                SendMessageW(grip, WM_KEYDOWN, 0x26, 0);
            }
            assert_eq!(native.state.logs.viewer_height(), 120);
            native.state.show_page(0);
            native.state.show_page(2);
            assert_eq!(native.state.logs.viewer_height(), 120);
            SendMessageW(grip, WM_KEYDOWN, 0x24, 0);
            assert_eq!(native.state.logs.viewer_height(), 240);
            resize_client(&mut native, 1000, 560, dpi);
            assert_eq!(
                native.rect(editor).bottom - native.rect(editor).top,
                scale(240, dpi)
            );
            assert_eq!(settings::text(editor), text);
            assert_eq!(SendMessageW(editor, 0xb0, 0, 0), selected);
            assert_ne!(
                GetWindowLongPtrW(editor, GWL_STYLE) as u32 & ES_READONLY as u32,
                0
            );
        }
    }
}

fn dispatch_focus_reveals(host: HWND) -> usize {
    let mut count = 0;
    let mut message = MSG::default();
    // SAFETY: Only drain this test viewport's copied-scalar reveal messages;
    // no timer, theme refresh, user input or other window messages are pumped.
    unsafe {
        while PeekMessageW(&mut message, host, WM_APP + 80, WM_APP + 80, PM_REMOVE) != 0 {
            assert!(count < 32, "focus revelation must not loop");
            DispatchMessageW(&message);
            count += 1;
        }
    }
    count
}

#[test]
fn native_log_keyboard_height_keeps_the_focused_grip_inside_short_viewports() {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetFocus() -> HWND;
    }
    let mut native = NativeState::new();
    native.state.show_page(2);
    let grip = native.control(2230);
    let editor = native.control(2200);
    let host = native.state.viewport.handle;
    settings::set_text(
        editor,
        "synthetic log text preserved during keyboard resizing",
    );
    // SAFETY: Test-owned offscreen window; show without activation so actual
    // focus/visibility can be checked. No clipboard, GUI timers or live worker.
    unsafe {
        ShowWindow(native.hwnd, SW_SHOWNOACTIVATE);
        SendMessageW(editor, 0xb1, 2, 9);
        let selection = SendMessageW(editor, 0xb0, 0, 0);
        for dpi in [96, 144, 192] {
            for height in [560, 680] {
                resize_client(&mut native, 1000, height, dpi);
                SendMessageW(grip, WM_KEYDOWN, 0x24, 0);
                native.scroll(SB_TOP);
                SetFocus(grip);
                viewport::reveal(grip);
                dispatch_focus_reveals(host);
                for key in std::iter::repeat_n(0x28, 25)
                    .chain(std::iter::repeat_n(0x26, 27))
                    .chain([0x24, 0x28, 0x26])
                {
                    assert_eq!(GetFocus(), grip);
                    let offset = GetScrollPos(host, SB_VERT);
                    SendMessageW(grip, WM_KEYDOWN, key, 0);
                    // Layout is synchronous, but scrolling must wait until its
                    // State borrow ends and the queued reveal is dispatched.
                    let mut info = SCROLLINFO {
                        cbSize: size_of::<SCROLLINFO>() as u32,
                        fMask: SIF_RANGE | SIF_PAGE,
                        ..Default::default()
                    };
                    GetScrollInfo(host, SB_VERT, &mut info);
                    let limit = (info.nMax - info.nPage as i32 + 1).max(0);
                    assert_eq!(GetScrollPos(host, SB_VERT), offset.min(limit));
                    assert!(dispatch_focus_reveals(host) > 0);
                    let mut bounds = RECT::default();
                    GetClientRect(host, &mut bounds);
                    let rect = native.rect(grip);
                    assert!(
                        rect.top >= 0 && rect.bottom <= bounds.bottom,
                        "DPI {dpi}, height {height}, key {key}: grip {}..{} outside 0..{}",
                        rect.top,
                        rect.bottom,
                        bounds.bottom
                    );
                    assert_eq!(GetFocus(), grip);
                    assert_eq!(SendMessageW(editor, 0xb0, 0, 0), selection);
                    assert_eq!(
                        settings::text(editor),
                        "synthetic log text preserved during keyboard resizing"
                    );
                }
            }
        }
    }
}

#[test]
fn native_app_edits_have_real_nonclient_frames_without_clipping_input() {
    use windows_sys::Win32::Graphics::Gdi::*;
    let mut native = NativeState::new();
    native.state.show_page(3);
    // SAFETY: All HWNDs, memory DC/bitmap and scratch region are test-owned;
    // the isolated controller is stopped and no real user settings are loaded.
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        let bitmap = CreateBitmap(1024, 256, 1, 32, null());
        assert!(!dc.is_null() && !bitmap.is_null());
        let old = SelectObject(dc, bitmap);
        let region = CreateRectRgn(0, 0, 0, 0);
        assert!(!region.is_null());
        let mut edits = Vec::new();
        let mut child = GetWindow(native.state.viewport.handle, GW_CHILD);
        while !child.is_null() {
            let mut class = [0u16; 16];
            let count = GetClassNameW(child, class.as_mut_ptr(), class.len() as i32);
            if String::from_utf16_lossy(&class[..count.max(0) as usize]) == "Edit" {
                edits.push(child);
            }
            child = GetWindow(child, GW_HWNDNEXT);
        }
        assert!(edits.len() >= 30);
        for child in edits {
            assert_ne!(
                GetWindowLongPtrW(child, GWL_EXSTYLE) as u32 & WS_EX_CLIENTEDGE,
                0
            );
            let style = GetWindowLongPtrW(child, GWL_STYLE);
            let extended = GetWindowLongPtrW(child, GWL_EXSTYLE);
            let selection = SendMessageW(child, 0xb0, 0, 0);
            for mode in [
                crate::config::ThemeMode::Light,
                crate::config::ThemeMode::Dark,
            ] {
                native.state.theme.mode = mode;
                theme::refresh(native.hwnd, &mut *native.state);
                assert_eq!(GetWindowLongPtrW(child, GWL_STYLE), style);
                assert_eq!(GetWindowLongPtrW(child, GWL_EXSTYLE), extended);
                assert_eq!(SendMessageW(child, 0xb0, 0, 0), selection);
                assert_eq!(GetWindowRgn(child, region), 0);
                SendMessageW(child, WM_PRINT, dc as WPARAM, PRF_NONCLIENT as LPARAM);
                let palette = native.state.theme.palette;
                let border = if settings::is_enabled(child) != 0 {
                    palette.border
                } else {
                    theme::blend(palette.border, palette.surface, 50)
                };
                assert_eq!(GetPixel(dc, 0, 0), border);
            }
        }
        DeleteObject(region);
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
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
