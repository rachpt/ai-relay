use super::*;
use crate::{
    config::ThemeMode,
    ui::{input_frame, scroll_theme, theme},
    windows::{Font, scale, wide},
};
use windows_sys::Win32::{
    Foundation::{POINT, RECT},
    Graphics::Gdi::*,
    System::Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources},
};

struct Fixture {
    parent: HWND,
    edit: HWND,
    _font: Font,
}
impl Fixture {
    fn new(dpi: u32) -> Self {
        Self::with_auto_hide(dpi, true)
    }
    fn with_auto_hide(dpi: u32, auto_hide: bool) -> Self {
        // SAFETY: Private hidden parent and native EDIT on this test thread.
        unsafe {
            let parent = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                null(),
                WS_POPUP,
                0,
                0,
                1600,
                1200,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                null(),
            );
            assert!(!parent.is_null());
            let edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                windows_sys::w!("EDIT"),
                null(),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | WS_BORDER
                    | WS_HSCROLL
                    | WS_VSCROLL
                    | (ES_MULTILINE | ES_READONLY | ES_AUTOHSCROLL | ES_AUTOVSCROLL) as u32,
                0,
                0,
                scale(696, dpi),
                scale(240, dpi),
                parent,
                super::super::CONTENT as usize as HMENU,
                std::ptr::null_mut(),
                null(),
            );
            assert!(!edit.is_null());
            let font = Font::new(dpi).unwrap();
            SendMessageW(edit, WM_SETFONT, font.0 as WPARAM, 0);
            if auto_hide {
                attach(edit).unwrap();
            }
            Self {
                parent,
                edit,
                _font: font,
            }
        }
    }
    unsafe fn text(&self, text: &str) {
        // SAFETY: Live same-thread EDIT copies the nul-terminated buffer.
        unsafe {
            assert_ne!(
                SendMessageW(self.edit, WM_SETTEXT, 0, wide(text).as_ptr() as LPARAM),
                0
            );
        }
    }
    unsafe fn size(&self, width: i32, height: i32) {
        // SAFETY: Same-thread native resize of our private control.
        unsafe {
            assert_ne!(MoveWindow(self.edit, 0, 0, width, height, 0), 0);
            dispatch_refreshes(self.edit);
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // SAFETY: Parent owns and destroys the EDIT before the retained font drops.
        unsafe {
            DestroyWindow(self.parent);
        }
    }
}

unsafe fn bars(hwnd: HWND, expected: (bool, bool)) {
    // SAFETY: Live test EDIT; stack output structures have the public SDK size.
    unsafe {
        dispatch_refreshes(hwnd);
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        for (axis, mask, object, visible) in [
            (SB_HORZ, WS_HSCROLL, OBJID_HSCROLL, expected.0),
            (SB_VERT, WS_VSCROLL, OBJID_VSCROLL, expected.1),
        ] {
            assert_eq!(style & mask != 0, visible, "axis {axis} native style");
            let mut bar = SCROLLBARINFO {
                cbSize: size_of::<SCROLLBARINFO>() as u32,
                ..Default::default()
            };
            assert_ne!(GetScrollBarInfo(hwnd, object, &mut bar), 0);
            assert_eq!(
                bar.rgstate[0] & STATE_SYSTEM_INVISIBLE == 0,
                visible,
                "axis {axis} MSAA visibility"
            );
            if visible {
                let mut info = SCROLLINFO {
                    cbSize: size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_RANGE | SIF_PAGE,
                    ..Default::default()
                };
                assert_ne!(GetScrollInfo(hwnd, axis, &mut info), 0);
                assert!(
                    !fits(info.nMin, info.nMax, info.nPage),
                    "visible axis must really overflow"
                );
            }
        }
    }
}

unsafe fn theme(fixture: &Fixture, kind: theme::Resolved, dpi: u32) {
    // SAFETY: Match the real native Edit frame/scroll theme order on our thread.
    unsafe {
        let p = theme::Theme::new(if kind == theme::Resolved::Dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        })
        .unwrap()
        .palette;
        SetWindowTheme(fixture.edit, null(), null());
        input_frame::apply(fixture.edit, kind, p, dpi);
        scroll_theme::apply(fixture.edit, kind, p, dpi);
    }
}

#[link(name = "comctl32")]
unsafe extern "system" {
    fn GetWindowSubclass(hwnd: HWND, proc: Proc, id: usize, data: *mut usize) -> i32;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn SetCapture(hwnd: HWND) -> HWND;
    fn ReleaseCapture() -> i32;
}

unsafe fn retained_state(hwnd: HWND) -> Option<State> {
    // SAFETY: Query only our proc/id on its owner thread, retaining its installed
    // Rc before any native forwarding or synchronous destruction can occur.
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), ID, &mut data) == 0 || data == 0 {
            return None;
        }
        let pointer = data as *const Cell<Presentation>;
        Rc::increment_strong_count(pointer);
        Some(Rc::from_raw(pointer))
    }
}

unsafe fn dispatch_refreshes(hwnd: HWND) -> usize {
    // SAFETY: Dispatch only this EDIT's private scalar messages, with a hard bound
    // that fails instead of hiding a self-sustaining dirty/refresh loop.
    unsafe {
        for count in 0..16 {
            let mut message = MSG::default();
            if PeekMessageW(&mut message, hwnd, REFRESH, REFRESH, PM_REMOVE) == 0 {
                return count;
            }
            assert_eq!(message.wParam, 0);
            assert_eq!(
                message.lParam, 0,
                "posted refresh must not own an Rc pointer"
            );
            DispatchMessageW(&message);
            if IsWindow(hwnd) == 0 {
                return count + 1;
            }
        }
        panic!("scrollbar refresh did not settle after 16 private messages");
    }
}

unsafe fn viewport(hwnd: HWND) -> (LRESULT, i32) {
    // SAFETY: Live private native EDIT; no range or position is synthesized.
    unsafe {
        (
            SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, 0, 0),
            scroll_info(hwnd, SB_HORZ).unwrap().nPos,
        )
    }
}

fn packed(point: POINT) -> LPARAM {
    (u32::from(point.x as u16) | (u32::from(point.y as u16) << 16)) as LPARAM
}

#[derive(Clone, Copy, Debug)]
enum Press {
    Thumb,
    LineAfter,
    PageAfter,
}

#[derive(Default)]
struct TrackingProbe {
    native_entries: Cell<usize>,
    redraws: Cell<usize>,
    capture_changes: Cell<usize>,
}

unsafe extern "system" fn tracking_probe(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: The stack probe is declared before its fixture. The fixture destroys
    // the child first, including during unwinding. Never enter native modal NC
    // tracking if the dark subclass fails to consume the synthetic mouse input.
    unsafe {
        let probe = &*(data as *const TrackingProbe);
        match message {
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
                probe.native_entries.set(probe.native_entries.get() + 1);
                return 0;
            }
            WM_SETREDRAW => probe.redraws.set(probe.redraws.get() + 1),
            WM_CAPTURECHANGED => probe.capture_changes.set(probe.capture_changes.get() + 1),
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(tracking_probe), id);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

unsafe fn install_tracking_probe(hwnd: HWND, probe: &TrackingProbe) {
    // SAFETY: Install before applying Dark so the real dark adapter runs first;
    // the caller keeps the stack probe alive through child destruction.
    unsafe {
        assert_ne!(
            SetWindowSubclass(
                hwnd,
                Some(tracking_probe),
                ID + 1,
                probe as *const TrackingProbe as usize,
            ),
            0
        );
    }
}

unsafe fn press(hwnd: HWND, vertical: bool, part: Press) -> POINT {
    // SAFETY: Query native screen geometry and send NC input to our own child.
    // tracking_probe blocks native fallback so missing dark tracking cannot hang.
    unsafe {
        let mut bar = SCROLLBARINFO {
            cbSize: size_of::<SCROLLBARINFO>() as u32,
            ..Default::default()
        };
        assert_ne!(
            GetScrollBarInfo(
                hwnd,
                if vertical {
                    OBJID_VSCROLL
                } else {
                    OBJID_HSCROLL
                },
                &mut bar,
            ),
            0
        );
        let length = if vertical {
            bar.rcScrollBar.bottom - bar.rcScrollBar.top
        } else {
            bar.rcScrollBar.right - bar.rcScrollBar.left
        };
        let along = match part {
            Press::Thumb => (bar.xyThumbTop + bar.xyThumbBottom) / 2,
            Press::LineAfter => length - bar.dxyLineButton / 2 - 1,
            Press::PageAfter => (bar.xyThumbBottom + length - bar.dxyLineButton) / 2,
        };
        assert!(along >= bar.dxyLineButton || matches!(part, Press::LineAfter));
        let point = if vertical {
            POINT {
                x: (bar.rcScrollBar.left + bar.rcScrollBar.right) / 2,
                y: bar.rcScrollBar.top + along,
            }
        } else {
            POINT {
                x: bar.rcScrollBar.left + along,
                y: (bar.rcScrollBar.top + bar.rcScrollBar.bottom) / 2,
            }
        };
        SendMessageW(
            hwnd,
            WM_NCLBUTTONDOWN,
            if vertical { HTVSCROLL } else { HTHSCROLL } as WPARAM,
            packed(point),
        );
        assert_eq!(GetCapture(), hwnd, "dark {part:?} must own capture");
        point
    }
}

unsafe fn captured_mouse(hwnd: HWND, message: u32, screen: POINT) {
    // SAFETY: Captured mouse messages use client coordinates even above NC bars.
    unsafe {
        let mut point = screen;
        assert_ne!(ScreenToClient(hwnd, &mut point), 0);
        SendMessageW(
            hwnd,
            message,
            usize::from(message == WM_MOUSEMOVE),
            packed(point),
        );
    }
}

fn long_lines() -> String {
    (0..80)
        .map(|n| format!("{n:03} {}", "W".repeat(320)))
        .collect::<Vec<_>>()
        .join("\r\n")
}

#[test]
fn inclusive_ranges_do_not_overflow_and_invalid_ranges_do_not_hide() {
    assert!(fits(0, 0, 0));
    assert!(fits(0, 99, 100));
    assert!(!fits(0, 100, 100));
    assert!(!fits(3, 2, 100));
    assert!(!fits(i32::MIN, i32::MAX, u32::MAX));
    assert!(fits(1, i32::MAX, u32::MAX));
}

#[test]
fn native_text_resize_font_and_theme_transitions_hide_only_fitting_axes() {
    for dpi in [96, 144, 192] {
        let f = Fixture::new(dpi);
        // SAFETY: All controls/fonts/messages are local to this test thread.
        unsafe {
            let styles = GetWindowLongPtrW(f.edit, GWL_STYLE) as u32 & !(WS_HSCROLL | WS_VSCROLL);
            let extended = GetWindowLongPtrW(f.edit, GWL_EXSTYLE);
            for kind in [
                theme::Resolved::Light,
                theme::Resolved::Dark,
                theme::Resolved::Contrast,
                theme::Resolved::Light,
            ] {
                f.text("短日志 / Short log");
                theme(&f, kind, dpi); // Apply dark while BOTH bars are hidden.
                bars(f.edit, (false, false));
                f.text(&(0..80).map(|_| "短行").collect::<Vec<_>>().join("\r\n"));
                bars(f.edit, (false, true));
                f.text(&"W".repeat(320));
                bars(f.edit, (true, false));
                f.text(&long_lines());
                bars(f.edit, (true, true));
                f.size(scale(6000, dpi), scale(2400, dpi));
                bars(f.edit, (false, false));
                f.size(scale(696, dpi), scale(240, dpi));
                bars(f.edit, (true, true));
                f.text("");
                bars(f.edit, (false, false));
                f.text("再变短 / Short again");
                bars(f.edit, (false, false));
                assert_eq!(
                    GetWindowLongPtrW(f.edit, GWL_STYLE) as u32 & !(WS_HSCROLL | WS_VSCROLL),
                    styles
                );
                assert_eq!(GetWindowLongPtrW(f.edit, GWL_EXSTYLE), extended);
            }
            let bigger_font = Font::new(192).unwrap();
            f.text(&"W".repeat(30));
            f.size(696, 240);
            SendMessageW(f.edit, WM_SETFONT, f._font.0 as WPARAM, 0);
            // Font changes re-evaluate existing content, not just the next text update.
            SendMessageW(f.edit, WM_SETFONT, bigger_font.0 as WPARAM, 0);
            bars(f.edit, (true, false));
            SendMessageW(f.edit, WM_SETFONT, f._font.0 as WPARAM, 0);
        }
    }
}

#[test]
fn native_threshold_has_no_mutually_induced_bars_and_replacement_recomputes() {
    let f = Fixture::new(96);
    // SAFETY: Same-thread owned control, borrowed font/DC restored before release.
    unsafe {
        f.text("");
        let mut format = RECT::default();
        SendMessageW(f.edit, EM_GETRECT, 0, (&mut format as *mut RECT) as LPARAM);
        let dc = GetDC(f.edit);
        assert!(!dc.is_null());
        let previous = SelectObject(dc, f._font.0);
        let mut metrics = TEXTMETRICW::default();
        assert_ne!(GetTextMetricsW(dc, &mut metrics), 0);
        let line = "W".repeat(44);
        let text = wide(&line);
        let mut extent = windows_sys::Win32::Foundation::SIZE::default();
        assert_ne!(GetTextExtentPoint32W(dc, text.as_ptr(), 44, &mut extent), 0);
        SelectObject(dc, previous);
        ReleaseDC(f.edit, dc);
        // Less slack than a native bar's width: both bars would falsely look
        // necessary if the calculation started with both already displayed.
        let width = 696 - (format.right - format.left) + extent.cx + 4;
        let height = 240 - (format.bottom - format.top) + metrics.tmHeight * 10 + 4;
        f.size(width, height);
        let lines = (0..10)
            .map(|_| line.as_str())
            .collect::<Vec<_>>()
            .join("\r\n");
        f.text(&lines);
        bars(f.edit, (false, false));
        f.text(&long_lines());
        bars(f.edit, (true, true));
        f.text(&lines);
        bars(f.edit, (false, false));
        SendMessageW(f.edit, EM_SETSEL, 0, -1);
        let replacement = wide(&long_lines());
        SendMessageW(f.edit, EM_SETREADONLY, 0, 0);
        SendMessageW(f.edit, EM_REPLACESEL, 0, replacement.as_ptr() as LPARAM);
        SendMessageW(f.edit, EM_SETREADONLY, 1, 0);
        bars(f.edit, (true, true));
        SendMessageW(f.edit, EM_SETSEL, 0, -1);
        SendMessageW(f.edit, EM_SETREADONLY, 0, 0);
        SendMessageW(f.edit, EM_REPLACESEL, 0, wide("").as_ptr() as LPARAM);
        SendMessageW(f.edit, EM_SETREADONLY, 1, 0);
        bars(f.edit, (false, false));
    }
}

#[test]
fn native_visibility_redraw_selection_scroll_and_resources_survive_recalculation() {
    let f = Fixture::new(96);
    // SAFETY: Native test-only operations; no app data or global system settings.
    unsafe {
        f.text(&long_lines());
        theme(&f, theme::Resolved::Dark, 96);
        bars(f.edit, (true, true));
        SendMessageW(f.edit, EM_SETSEL, 10, 30);
        SendMessageW(f.edit, EM_LINESCROLL, 40, 20);
        let first = SendMessageW(f.edit, EM_GETFIRSTVISIBLELINE, 0, 0);
        let selection = SendMessageW(f.edit, EM_GETSEL, 0, 0);
        SendMessageW(f.edit, WM_SETFONT, f._font.0 as WPARAM, 0);
        assert_eq!(SendMessageW(f.edit, EM_GETFIRSTVISIBLELINE, 0, 0), first);
        assert_eq!(SendMessageW(f.edit, EM_GETSEL, 0, 0), selection);
        assert_ne!(GetWindowLongPtrW(f.edit, GWL_STYLE) as u32 & WS_VISIBLE, 0);
        SendMessageW(f.edit, WM_SETREDRAW, 0, 0);
        f.text("short while redraw disabled");
        bars(f.edit, (false, false));
        assert_eq!(GetWindowLongPtrW(f.edit, GWL_STYLE) as u32 & WS_VISIBLE, 0);
        SendMessageW(f.edit, WM_SETREDRAW, 1, 0);
        ShowWindow(f.edit, SW_HIDE);
        f.text(&long_lines());
        bars(f.edit, (true, true));
        assert_eq!(GetWindowLongPtrW(f.edit, GWL_STYLE) as u32 & WS_VISIBLE, 0);
        ShowWindow(f.edit, SW_SHOWNA);
        for _ in 0..10 {
            f.text("warm");
            f.text(&long_lines());
        }
        let before = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
        for _ in 0..40 {
            f.text("");
            bars(f.edit, (false, false));
            f.text(&long_lines());
            bars(f.edit, (true, true));
        }
        assert_eq!(GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS), before);
    }
}

#[test]
fn dark_capture_survives_text_refresh_and_repeat_until_release() {
    // Private contract with scroll_theme's real tracking timer, not the auto-hide
    // timer. Explicit ticks keep this regression independent of wall-clock timing.
    const DARK_TRACK_TIMER: usize = 0x5343_5448;
    for vertical in [true, false] {
        for part in [Press::Thumb, Press::LineAfter, Press::PageAfter] {
            let probe = TrackingProbe::default();
            let f = Fixture::new(96);
            // SAFETY: Private hidden HWNDs; the earlier-declared probe outlives
            // destruction. No global input injection or native modal loop occurs.
            unsafe {
                f.text(&long_lines());
                install_tracking_probe(f.edit, &probe);
                theme(&f, theme::Resolved::Dark, 96);
                let retained = retained_state(f.edit).unwrap();
                let point = press(f.edit, vertical, part);
                let redraws = probe.redraws.get();
                let capture_changes = probe.capture_changes.get();
                f.text(&long_lines().replacen("000", "001", 1));
                assert_eq!(GetCapture(), f.edit, "text refresh canceled {part:?}");
                assert!(retained.get().dirty);
                assert!(!retained.get().updating);
                let timer = retained.get().timer;
                assert_ne!(timer, 0, "capture must arrange a bounded release recheck");
                for _ in 0..3 {
                    SendMessageW(f.edit, WM_TIMER, timer, 0);
                    assert_eq!(GetCapture(), f.edit);
                    assert!(retained.get().dirty);
                }
                assert_eq!(probe.redraws.get(), redraws);
                assert_eq!(probe.capture_changes.get(), capture_changes);
                let axis = if vertical { SB_VERT } else { SB_HORZ };
                let before = scroll_info(f.edit, axis).unwrap().nPos;
                let mut target = point;
                match part {
                    Press::Thumb => {
                        if vertical {
                            target.y += 35;
                        } else {
                            target.x += 45;
                        }
                        captured_mouse(f.edit, WM_MOUSEMOVE, target);
                    }
                    Press::LineAfter | Press::PageAfter => {
                        SendMessageW(f.edit, WM_TIMER, DARK_TRACK_TIMER, 0);
                    }
                }
                assert_eq!(GetCapture(), f.edit);
                assert!(
                    scroll_info(f.edit, axis).unwrap().nPos > before,
                    "{part:?} stopped driving the native axis after a text refresh"
                );
                if matches!(part, Press::LineAfter) {
                    SendMessageW(f.edit, WM_CANCELMODE, 0, 0);
                } else {
                    captured_mouse(f.edit, WM_LBUTTONUP, target);
                }
                assert_ne!(GetCapture(), f.edit);
                assert!(retained.get().pending);
                assert_eq!(dispatch_refreshes(f.edit), 1);
                assert!(!retained.get().dirty);
                assert!(!retained.get().pending);
                assert_eq!(retained.get().timer, 0);
                bars(f.edit, (true, true));
                assert_eq!(probe.native_entries.get(), 0);
            }
        }
    }
}

#[test]
fn capture_defers_hidden_axes_and_release_coalesces_final_visibility() {
    let f = Fixture::new(96);
    // SAFETY: Explicit capture belongs only to our hidden EDIT. Native destroy
    // releases it on panic; private refresh dispatch never pumps unrelated input.
    unsafe {
        let retained = retained_state(f.edit).unwrap();
        for (text, expected) in [
            (long_lines(), (true, true)),
            (String::new(), (false, false)),
        ] {
            SetCapture(f.edit);
            assert_eq!(GetCapture(), f.edit);
            f.text(&text);
            assert!(retained.get().dirty);
            let timer = retained.get().timer;
            assert_ne!(timer, 0);
            SendMessageW(f.edit, WM_TIMER, timer, 0);
            assert_eq!(GetCapture(), f.edit);
            assert!(retained.get().dirty);
            assert_ne!(ReleaseCapture(), 0);
            assert!(retained.get().pending);
            SendMessageW(f.edit, WM_CANCELMODE, 0, 0);
            SendMessageW(f.edit, WM_CANCELMODE, 0, 0);
            assert_eq!(
                dispatch_refreshes(f.edit),
                1,
                "release refreshes must coalesce"
            );
            bars(f.edit, expected);
            assert!(!retained.get().dirty);
            assert!(!retained.get().pending);
            assert_eq!(retained.get().timer, 0);
            // A queued tick from the canceled timer must not restart work.
            SendMessageW(f.edit, WM_TIMER, timer, 0);
            assert!(!retained.get().dirty);
            assert_eq!(retained.get().timer, 0);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ThemeViewport {
    first: LRESULT,
    horizontal: Option<(i32, i32, u32, i32)>,
    client: (i32, i32),
    outer: (i32, i32),
    bars: u32,
}

unsafe fn theme_viewport(hwnd: HWND) -> ThemeViewport {
    // SAFETY: Private test EDIT remains live during these synchronous queries.
    // The trace does not instrument EM_GETFIRSTVISIBLELINE, avoiding recursion.
    unsafe {
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client);
        ThemeViewport {
            first: SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, 0, 0),
            horizontal: scroll_info(hwnd, SB_HORZ)
                .map(|info| (info.nMin, info.nMax, info.nPage, info.nPos)),
            client: (client.right - client.left, client.bottom - client.top),
            outer: window_size(hwnd),
            bars: GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & (WS_HSCROLL | WS_VSCROLL),
        }
    }
}

#[derive(Debug)]
struct ThemeEvent {
    message: u32,
    w: WPARAM,
    l: LPARAM,
    before: ThemeViewport,
    native_after: ThemeViewport,
}
#[derive(Default)]
struct ThemeTrace {
    recording: Cell<bool>,
    events: std::cell::RefCell<Vec<ThemeEvent>>,
}

unsafe extern "system" fn native_theme_trace(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: The stack trace precedes its fixture and is installed BELOW auto
    // hide and the frame/dark subclasses. Its after snapshot is therefore native
    // processing's result BEFORE auto-hide can run its post-forward synchronize.
    // No RefCell borrow spans native forwarding; no assertions run in this FFI.
    unsafe {
        let trace = &*(data as *const ThemeTrace);
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(native_theme_trace), id);
        }
        let before = (trace.recording.get()
            && matches!(
                message,
                WM_THEMECHANGED | WM_SIZE | WM_SETREDRAW | WM_HSCROLL | EM_LINESCROLL
            ))
        .then(|| theme_viewport(hwnd));
        let result = DefSubclassProc(hwnd, message, w, l);
        if let Some(before) = before {
            let native_after = theme_viewport(hwnd);
            let mut events = trace.events.borrow_mut();
            if events.len() < 128 {
                events.push(ThemeEvent {
                    message,
                    w,
                    l,
                    before,
                    native_after,
                });
            }
        }
        result
    }
}

unsafe fn install_theme_trace(hwnd: HWND, trace: &ThemeTrace) {
    // SAFETY: Caller declares the trace before its fixture, retaining it until
    // all callbacks including destruction have finished.
    unsafe {
        assert_ne!(
            SetWindowSubclass(
                hwnd,
                Some(native_theme_trace),
                ID + 3,
                trace as *const ThemeTrace as usize,
            ),
            0
        );
    }
}

#[test]
fn bottom_and_partial_character_right_positions_survive_font_and_theme_recalculation() {
    for dpi in [96, 144, 192] {
        let trace = ThemeTrace::default();
        let native_trace = ThemeTrace::default();
        let f = Fixture::with_auto_hide(dpi, false);
        let native = Fixture::with_auto_hide(dpi, false);
        // SAFETY: Native endpoints and observed EM_LINESCROLL steps are the
        // oracle, independent of the production font-unit calculation. The twin
        // has identical native/frame/dark configuration but never attaches auto
        // hide. Traces live longer than both private HWND trees.
        unsafe {
            install_theme_trace(f.edit, &trace);
            install_theme_trace(native.edit, &native_trace);
            attach(f.edit).unwrap();
            for kind in [
                theme::Resolved::Light,
                theme::Resolved::Dark,
                theme::Resolved::Contrast,
            ] {
                for fixture in [&f, &native] {
                    fixture.text(&long_lines());
                    theme(fixture, kind, dpi);
                }
                SendMessageW(f.edit, WM_HSCROLL, SB_LEFT as WPARAM, 0);
                let left = scroll_info(f.edit, SB_HORZ).unwrap().nPos;
                SendMessageW(f.edit, EM_LINESCROLL, 1, 0);
                let unit = scroll_info(f.edit, SB_HORZ).unwrap().nPos - left;
                assert!(unit > 0);
                // Force an endpoint outside the left-anchored character lattice;
                // otherwise a truncating restoration could accidentally pass.
                let mut partial_endpoint = false;
                for extra in 0..=unit {
                    for fixture in [&f, &native] {
                        fixture.size(scale(696, dpi) + extra, scale(240, dpi));
                    }
                    dispatch_refreshes(f.edit);
                    SendMessageW(f.edit, WM_HSCROLL, SB_RIGHT as WPARAM, 0);
                    let info = scroll_info(f.edit, SB_HORZ).unwrap();
                    if (info.nPos - info.nMin) % unit != 0 {
                        partial_endpoint = true;
                        break;
                    }
                }
                assert!(
                    partial_endpoint,
                    "fixture needs a partial-character right clamp"
                );
                for fixture in [&f, &native] {
                    SendMessageW(fixture.edit, EM_SETSEL, 10, 30);
                }
                let selection = SendMessageW(f.edit, EM_GETSEL, 0, 0);
                for back in [0isize, 1, 2, 5] {
                    for fixture in [&f, &native] {
                        SendMessageW(fixture.edit, WM_VSCROLL, SB_BOTTOM as WPARAM, 0);
                        SendMessageW(fixture.edit, WM_HSCROLL, SB_RIGHT as WPARAM, 0);
                        SendMessageW(fixture.edit, EM_LINESCROLL, (-back) as WPARAM, -back);
                    }
                    let expected = viewport(f.edit);
                    assert!(expected.0 > 20 && expected.1 > 100);
                    assert_eq!(theme_viewport(f.edit), theme_viewport(native.edit));
                    for iteration in 0..3 {
                        for fixture in [&f, &native] {
                            SendMessageW(fixture.edit, WM_SETFONT, fixture._font.0 as WPARAM, 0);
                        }
                        dispatch_refreshes(f.edit);
                        assert_eq!(
                            viewport(f.edit),
                            expected,
                            "same-font dpi={dpi} kind={kind:?} back={back} iteration={iteration}"
                        );
                        assert_eq!(viewport(native.edit), expected);
                        trace.events.borrow_mut().clear();
                        native_trace.events.borrow_mut().clear();
                        let before = theme_viewport(f.edit);
                        let native_before = theme_viewport(native.edit);
                        trace.recording.set(true);
                        native_trace.recording.set(true);
                        SendMessageW(f.edit, WM_THEMECHANGED, 0, 0);
                        SendMessageW(native.edit, WM_THEMECHANGED, 0, 0);
                        dispatch_refreshes(f.edit);
                        trace.recording.set(false);
                        native_trace.recording.set(false);
                        let after = theme_viewport(f.edit);
                        let native_after = theme_viewport(native.edit);
                        if viewport(f.edit) != expected || after != native_after {
                            eprintln!(
                                "THEME dpi={dpi} kind={kind:?} back={back} iteration={iteration} unit={unit} \
                                 expected={expected:?}\nauto before={before:?}\nauto after={after:?}\n\
                                 baseline before={native_before:?}\nbaseline after={native_after:?}"
                            );
                            for (name, events) in
                                [("auto", &trace.events), ("baseline", &native_trace.events)]
                            {
                                for event in events.borrow().iter() {
                                    eprintln!(
                                        "{name} msg={:#x} w={} l={} before={:?} native_after={:?}",
                                        event.message,
                                        event.w,
                                        event.l,
                                        event.before,
                                        event.native_after
                                    );
                                }
                            }
                        }
                        assert_eq!(
                            viewport(f.edit),
                            expected,
                            "theme clamp dpi={dpi} kind={kind:?} back={back} iteration={iteration}"
                        );
                        // Style bits alone miss a native bar restored during
                        // WM_SIZE without a matching NC calculation. Even when
                        // nPos does not clamp, compare client/page against the
                        // identical no-auto-hide control, not a guessed metric.
                        assert_eq!(
                            after, native_after,
                            "theme geometry/range differs from native baseline dpi={dpi} kind={kind:?} back={back} iteration={iteration}"
                        );
                        assert_eq!(viewport(native.edit), expected);
                        assert_eq!(SendMessageW(f.edit, EM_GETSEL, 0, 0), selection);
                        bars(f.edit, (true, true));
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum ReentryAction {
    ReplaceText,
    ReplaceTextAndDisableRedraw,
    ToggleRedraw,
    Destroy,
}

struct ReentryProbe {
    message: u32,
    w: Option<WPARAM>,
    action: ReentryAction,
    armed: Cell<bool>,
    fired: Cell<usize>,
    updating_seen: Cell<bool>,
    marker_consumed: Cell<bool>,
    survived_destroy: Cell<bool>,
    nc_destroys: Cell<usize>,
}
impl ReentryProbe {
    fn new(message: u32, w: Option<WPARAM>, action: ReentryAction) -> Self {
        Self {
            message,
            w,
            action,
            armed: Cell::new(true),
            fired: Cell::new(0),
            updating_seen: Cell::new(false),
            marker_consumed: Cell::new(false),
            survived_destroy: Cell::new(false),
            nc_destroys: Cell::new(0),
        }
    }
}

unsafe extern "system" fn reentry_probe(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: Stack probe outlives its fixture. Forward first so auto-hide has
    // consumed its internal-redraw marker before injecting an external message.
    // No assertion/panic is used inside this FFI callback.
    unsafe {
        let probe = &*(data as *const ReentryProbe);
        if message == WM_NCDESTROY {
            probe.nc_destroys.set(probe.nc_destroys.get() + 1);
            RemoveWindowSubclass(hwnd, Some(reentry_probe), id);
        }
        let trigger = message == probe.message
            && probe.w.is_none_or(|expected| expected == w)
            && probe.armed.replace(false);
        let result = DefSubclassProc(hwnd, message, w, l);
        if trigger {
            probe.fired.set(probe.fired.get() + 1);
            if let Some(retained) = retained_state(hwnd) {
                probe.updating_seen.set(retained.get().updating);
                probe
                    .marker_consumed
                    .set(retained.get().internal_redraw.is_none());
                match probe.action {
                    ReentryAction::ReplaceText | ReentryAction::ReplaceTextAndDisableRedraw => {
                        let replacement = wide(&long_lines());
                        SendMessageW(hwnd, WM_SETTEXT, 0, replacement.as_ptr() as LPARAM);
                        if matches!(probe.action, ReentryAction::ReplaceTextAndDisableRedraw) {
                            SendMessageW(hwnd, WM_SETREDRAW, 0, 0);
                        }
                    }
                    ReentryAction::ToggleRedraw => {
                        SendMessageW(hwnd, WM_SETREDRAW, 0, 0);
                        SendMessageW(hwnd, WM_SETREDRAW, 1, 0);
                    }
                    ReentryAction::Destroy => {
                        DestroyWindow(hwnd);
                        probe
                            .survived_destroy
                            .set(!retained.get().attached && Rc::strong_count(&retained) >= 3);
                    }
                }
            }
        }
        result
    }
}

unsafe fn install_reentry_probe(hwnd: HWND, probe: &ReentryProbe) {
    // SAFETY: Earlier-declared stack probe remains live until child destruction.
    unsafe {
        assert_ne!(
            SetWindowSubclass(
                hwnd,
                Some(reentry_probe),
                ID + 2,
                probe as *const ReentryProbe as usize,
            ),
            0
        );
    }
}

#[test]
fn nested_real_text_and_external_no_redraw_settle_once_without_overriding_the_caller() {
    for (trigger_w, disable) in [(0, false), (0, true), (1, false), (1, true)] {
        let probe = ReentryProbe::new(
            WM_SETREDRAW,
            Some(trigger_w),
            if disable {
                ReentryAction::ReplaceTextAndDisableRedraw
            } else {
                ReentryAction::ReplaceText
            },
        );
        let f = Fixture::new(96);
        // SAFETY: Private child and retained local Rc. Inject both before native
        // measurement and at final restoration, after the last range query.
        unsafe {
            f.text("short before reentry");
            let retained = retained_state(f.edit).unwrap();
            let generation = retained.get().redraw_generation;
            install_reentry_probe(f.edit, &probe);
            SendMessageW(f.edit, WM_SETFONT, f._font.0 as WPARAM, 0);
            assert_eq!(probe.fired.get(), 1);
            assert!(probe.updating_seen.get());
            assert!(probe.marker_consumed.get());
            assert!(retained.get().dirty);
            assert!(retained.get().pending);
            assert!(!retained.get().updating);
            assert_eq!(retained.get().redraw, !disable);
            assert_eq!(
                retained.get().redraw_generation,
                generation + u64::from(disable)
            );
            assert_eq!(
                SendMessageW(f.edit, WM_GETTEXTLENGTH, 0, 0),
                long_lines().len() as LRESULT
            );
            SendMessageW(f.edit, WM_CANCELMODE, 0, 0);
            SendMessageW(f.edit, WM_CANCELMODE, 0, 0);
            assert_eq!(dispatch_refreshes(f.edit), 1);
            assert!(!retained.get().dirty);
            assert!(!retained.get().pending);
            assert!(retained.get().internal_redraw.is_none());
            assert_eq!(retained.get().depth, 0);
            assert_eq!(retained.get().redraw, !disable);
            assert_eq!(
                GetWindowLongPtrW(f.edit, GWL_STYLE) as u32 & WS_VISIBLE != 0,
                !disable
            );
            bars(f.edit, (true, true));
            if disable {
                SendMessageW(f.edit, WM_SETREDRAW, 1, 0);
            }
        }
    }
}

#[test]
fn synchronous_destroy_during_measurement_or_redraw_releases_only_the_installed_owner() {
    for (message, w, via_post) in [
        (WM_GETTEXTLENGTH, None, false),
        (WM_SETREDRAW, Some(0), false),
        (WM_SETREDRAW, Some(1), false),
        (WM_GETTEXTLENGTH, None, true),
        (WM_SETREDRAW, Some(0), true),
        (WM_SETREDRAW, Some(1), true),
    ] {
        let probe = ReentryProbe::new(message, w, ReentryAction::Destroy);
        let f = Fixture::new(96);
        // SAFETY: The probe precedes the fixture; retain independently of all
        // callback frames, then prove they and the installed owner release once.
        unsafe {
            f.text(&long_lines());
            let retained = retained_state(f.edit).unwrap();
            assert_eq!(Rc::strong_count(&retained), 2);
            if via_post {
                SetCapture(f.edit);
                f.text(&long_lines());
                assert_ne!(retained.get().timer, 0);
                assert_ne!(ReleaseCapture(), 0);
                assert!(retained.get().pending);
            }
            install_reentry_probe(f.edit, &probe);
            if via_post {
                assert_eq!(dispatch_refreshes(f.edit), 1);
            } else {
                SendMessageW(f.edit, WM_SETFONT, f._font.0 as WPARAM, 0);
            }
            assert_eq!(probe.fired.get(), 1);
            assert!(probe.updating_seen.get());
            assert!(probe.survived_destroy.get());
            assert_eq!(probe.nc_destroys.get(), 1);
            assert_eq!(IsWindow(f.edit), 0);
            assert!(!retained.get().attached);
            assert_eq!(Rc::strong_count(&retained), 1);
            let weak = Rc::downgrade(&retained);
            drop(retained);
            assert!(weak.upgrade().is_none());
        }
    }
}

#[test]
fn redraw_generation_from_initial_scroll_query_is_not_treated_as_a_later_request() {
    let probe = ReentryProbe::new(EM_GETFIRSTVISIBLELINE, None, ReentryAction::ToggleRedraw);
    let f = Fixture::new(96);
    // SAFETY: Toggle external redraw during the first synchronous query, before
    // auto-hide starts suppressing paint. The final external request is TRUE.
    unsafe {
        f.text(&long_lines());
        let retained = retained_state(f.edit).unwrap();
        let generation = retained.get().redraw_generation;
        install_reentry_probe(f.edit, &probe);
        SendMessageW(f.edit, WM_SETFONT, f._font.0 as WPARAM, 0);
        assert_eq!(probe.fired.get(), 1);
        assert!(probe.updating_seen.get());
        assert_eq!(retained.get().redraw_generation, generation + 2);
        assert!(retained.get().redraw);
        assert!(retained.get().internal_redraw.is_none());
        assert!(!retained.get().updating);
        assert!(!retained.get().dirty);
        assert_ne!(
            GetWindowLongPtrW(f.edit, GWL_STYLE) as u32 & WS_VISIBLE,
            0,
            "an old generation must not strand the internal FALSE after external FALSE/TRUE"
        );
    }
}

#[test]
fn destroy_with_capture_timer_and_pending_refresh_does_not_retain_state() {
    let f = Fixture::new(96);
    // SAFETY: Only private HWND capture and scalar posted work; local Rc/Weak
    // observes ownership without dispatching messages to an already dead HWND.
    unsafe {
        let retained = retained_state(f.edit).unwrap();
        SetCapture(f.edit);
        f.text(&long_lines());
        assert_ne!(retained.get().timer, 0);
        assert!(retained.get().dirty);
        post_refresh(f.edit, &retained);
        assert!(retained.get().pending);
        assert_eq!(GetCapture(), f.edit);
        assert_eq!(Rc::strong_count(&retained), 2);
        DestroyWindow(f.edit);
        assert_eq!(IsWindow(f.edit), 0);
        assert_ne!(GetCapture(), f.edit);
        assert!(!retained.get().attached);
        assert_eq!(Rc::strong_count(&retained), 1);
        let weak = Rc::downgrade(&retained);
        drop(retained);
        assert!(
            weak.upgrade().is_none(),
            "timer/posted work must not own the state"
        );
    }
}

#[test]
fn dark_applied_while_both_axes_fit_tracks_later_overflow_without_reapplication() {
    let probe = TrackingProbe::default();
    let f = Fixture::new(96);
    // SAFETY: Guard native fallback before installing dark presentation. Actual
    // capture and native displacement prove tracking exists, not just styles.
    unsafe {
        f.text("fits");
        bars(f.edit, (false, false));
        install_tracking_probe(f.edit, &probe);
        theme(&f, theme::Resolved::Dark, 96);
        f.text(&long_lines());
        bars(f.edit, (true, true));
        for vertical in [true, false] {
            let mut point = press(f.edit, vertical, Press::Thumb);
            let axis = if vertical { SB_VERT } else { SB_HORZ };
            let before = scroll_info(f.edit, axis).unwrap().nPos;
            if vertical {
                point.y += 35;
            } else {
                point.x += 45;
            }
            captured_mouse(f.edit, WM_MOUSEMOVE, point);
            assert_eq!(GetCapture(), f.edit);
            assert!(scroll_info(f.edit, axis).unwrap().nPos > before);
            captured_mouse(f.edit, WM_LBUTTONUP, point);
            assert_ne!(GetCapture(), f.edit);
        }
        assert_eq!(probe.native_entries.get(), 0);
        for kind in [theme::Resolved::Light, theme::Resolved::Contrast] {
            theme(&f, kind, 96);
            SendMessageW(f.edit, WM_NCLBUTTONDOWN, HTVSCROLL as WPARAM, 0);
            assert_ne!(GetCapture(), f.edit);
        }
        assert_eq!(
            probe.native_entries.get(),
            2,
            "Light/Contrast must restore native input"
        );
    }
}
