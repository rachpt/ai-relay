use super::*;
use std::time::{Duration, Instant};

unsafe fn pump_for(duration: Duration) {
    // SAFETY: dispatch this test thread's real queue, including TimerProc calls.
    // Never fabricate timer IDs or force paint/print after navigation.
    unsafe {
        let until = Instant::now() + duration;
        loop {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if Instant::now() >= until {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

unsafe fn client_screen(hwnd: HWND) -> RECT {
    // SAFETY: owned HWND and writable local coordinates.
    unsafe {
        let mut rect = RECT::default();
        let mut origin = POINT::default();
        assert_ne!(GetClientRect(hwnd, &mut rect), 0);
        assert_ne!(ClientToScreen(hwnd, &mut origin), 0);
        RECT {
            left: origin.x,
            top: origin.y,
            right: origin.x + rect.right,
            bottom: origin.y + rect.bottom,
        }
    }
}

fn includes(outer: RECT, inner: RECT) -> bool {
    outer.left <= inner.left
        && outer.top <= inner.top
        && outer.right >= inner.right
        && outer.bottom >= inner.bottom
}

unsafe fn assert_popup_layout(calendar: HWND) {
    // SAFETY: real popup and its native parent; check both levels, not just the
    // child rect which can extend past a smaller clipping parent.
    unsafe {
        let mut minimum = RECT::default();
        assert_ne!(
            SendMessageW(
                calendar,
                MCM_GETMINREQRECT,
                0,
                (&mut minimum as *mut RECT) as LPARAM
            ),
            0
        );
        let client = client_screen(calendar);
        assert!(
            client.right - client.left
                >= minimum
                    .right
                    .max(SendMessageW(calendar, MCM_GETMAXTODAYWIDTH, 0, 0) as i32)
        );
        assert!(client.bottom - client.top >= minimum.bottom);
        let mut outer = RECT::default();
        GetWindowRect(calendar, &mut outer);
        if GetWindowLongPtrW(calendar, GWL_STYLE) as u32 & WS_CHILD != 0 {
            let popup = GetParent(calendar);
            assert!(
                includes(client_screen(popup), outer),
                "calendar clipped by popup host"
            );
            GetWindowRect(popup, &mut outer);
        }
        let work = crate::ui::overview::rect_work_area(&outer).unwrap();
        assert!(
            includes(work, outer),
            "popup extends outside monitor work area"
        );
    }
}

unsafe fn screen_calendar(calendar: HWND) -> Frame {
    // SAFETY: test-only bounded desktop read. No PrintWindow or calendar DC can
    // repair/mask the physical state being measured. Refuse occluded samples.
    unsafe {
        let rect = client_screen(calendar);
        let visible = || {
            for x in [rect.left + 3, (rect.left + rect.right) / 2, rect.right - 4] {
                for y in [rect.top + 3, (rect.top + rect.bottom) / 2, rect.bottom - 4] {
                    assert_eq!(
                        WindowFromPoint(POINT { x, y }),
                        calendar,
                        "calendar physical sample is occluded"
                    );
                }
            }
        };
        visible();
        let frame = Frame::new(rect.right - rect.left, rect.bottom - rect.top).unwrap();
        let screen = GetDC(null_mut());
        assert!(!screen.is_null());
        assert_ne!(
            BitBlt(
                frame.dc,
                0,
                0,
                frame.width,
                frame.height,
                screen,
                rect.left,
                rect.top,
                SRCCOPY
            ),
            0
        );
        ReleaseDC(null_mut(), screen);
        GdiFlush();
        visible();
        frame
    }
}

unsafe fn popup(picker: HWND) -> HWND {
    // SAFETY: create through native input, then execute the component's real
    // dropdown path (STATIC test hosts don't forward DTN_DROPDOWN themselves).
    unsafe {
        let mut info = DATETIMEPICKERINFO {
            cbSize: size_of::<DATETIMEPICKERINFO>() as u32,
            ..Default::default()
        };
        SendMessageW(
            picker,
            DTM_GETDATETIMEPICKERINFO,
            0,
            (&mut info as *mut DATETIMEPICKERINFO) as LPARAM,
        );
        let point = (((info.rcButton.top + info.rcButton.bottom) / 2) << 16)
            | ((info.rcButton.left + info.rcButton.right) / 2);
        SendMessageW(picker, WM_LBUTTONDOWN, 1, point as LPARAM);
        SendMessageW(picker, WM_LBUTTONUP, 0, point as LPARAM);
        let calendar = SendMessageW(picker, DTM_GETMONTHCAL, 0, 0) as HWND;
        assert!(!calendar.is_null());
        let notification = NMHDR {
            hwndFrom: picker,
            idFrom: GetDlgCtrlID(picker) as usize,
            code: DTN_DROPDOWN,
        };
        assert!(date_picker::notify(
            (&notification as *const NMHDR) as LPARAM
        ));
        SendMessageW(picker, super::super::super::REFRESH_POPUP, 0, 0);
        calendar
    }
}

unsafe fn hit_point(calendar: HWND, wanted: u32) -> POINT {
    // SAFETY: public native hit-testing, no guessed button coordinates.
    unsafe {
        let mut rect = RECT::default();
        GetClientRect(calendar, &mut rect);
        for y in 0..rect.bottom {
            for x in 0..rect.right {
                let mut hit = MCHITTESTINFO {
                    cbSize: size_of::<MCHITTESTINFO>() as u32,
                    pt: POINT { x, y },
                    ..Default::default()
                };
                let result = SendMessageW(
                    calendar,
                    MCM_HITTEST,
                    0,
                    (&mut hit as *mut MCHITTESTINFO) as LPARAM,
                ) as u32;
                if result == wanted {
                    return hit.pt;
                }
            }
        }
        panic!("missing native hit target {wanted:#x}");
    }
}

unsafe fn assert_grid_and_ink(calendar: HWND, frame: &Frame) {
    // SAFETY: inspect flushed native output and public hit targets; do not infer
    // text success from a dark blank bitmap. Every weekday column must have ink.
    unsafe {
        dark(frame);
        let mut columns = std::collections::BTreeMap::<i32, usize>::new();
        let mut weekdays = std::collections::BTreeMap::<i32, usize>::new();
        let mut row_ys = std::collections::BTreeSet::new();
        for y in 0..frame.height {
            for x in (0..frame.width).step_by(3) {
                let mut hit = MCHITTESTINFO {
                    cbSize: size_of::<MCHITTESTINFO>() as u32,
                    pt: POINT { x, y },
                    ..Default::default()
                };
                let part = SendMessageW(
                    calendar,
                    MCM_HITTEST,
                    0,
                    (&mut hit as *mut MCHITTESTINFO) as LPARAM,
                ) as u32;
                if part == MCHT_CALENDARDAY {
                    let ink = weekdays.entry(hit.iCol).or_default();
                    if luma(GetPixel(frame.dc, x, y)) > 120 {
                        *ink += 1;
                    }
                }
                if part == MCHT_CALENDARDATE {
                    let ink = columns.entry(hit.iCol).or_default();
                    if luma(GetPixel(frame.dc, x, y)) > 120 {
                        *ink += 1;
                    }
                    row_ys.insert(hit.iRow);
                }
            }
        }
        assert_eq!(weekdays.len(), 7, "native weekday header incomplete");
        assert!(
            weekdays.values().all(|ink| *ink > 3),
            "native weekday text invisible: {weekdays:?}"
        );
        assert_eq!(columns.len(), 7, "native seven-column grid incomplete");
        assert!(
            columns.values().all(|ink| *ink > 3),
            "native date ink missing in a weekday column: {columns:?}"
        );
        assert!(row_ys.len() >= 4);
        for (label, left, top, right, bottom) in [
            (
                "title",
                frame.width / 5,
                0,
                frame.width * 4 / 5,
                frame.height / 5,
            ),
            (
                "today footer",
                frame.width / 4,
                frame.height * 7 / 8,
                frame.width - 2,
                frame.height - 2,
            ),
        ] {
            let mut ink = 0;
            for y in top..bottom {
                for x in left..right {
                    if luma(GetPixel(frame.dc, x, y)) > 120 {
                        ink += 1;
                    }
                }
            }
            assert!(ink > 10, "missing native {label} foreground");
        }
        hit_point(calendar, MCHT_TITLEBTNNEXT);
        hit_point(calendar, MCHT_TITLEBTNPREV);
        hit_point(calendar, MCHT_TODAYLINK);
    }
}

#[test]
#[ignore = "requires an unobscured interactive desktop; run explicitly and serially"]
fn calendar_navigation_screen_stays_dark_without_print_or_forced_repaint() {
    let _visual_styles = VisualStyles::new();
    let host = Host::new();
    // SAFETY: private fixture; no data/network/clipboard/global theme changes.
    unsafe {
        let picker = date_picker::create(host.0, 2224, "开始（含）").unwrap();
        MoveWindow(picker, 24, 30, 336, 28, 0);
        SendMessageW(picker, WM_SETTEXT, 0, wide("2026-10-25").as_ptr() as LPARAM);
        SendMessageW(picker, DTM_SETRANGE, 0, 0);
        date_picker::apply(picker, Resolved::Dark, palette(), 96);
        SetWindowPos(host.0, HWND_TOPMOST, 80, 80, 700, 500, SWP_SHOWWINDOW);
        SetForegroundWindow(host.0);
        let calendar = popup(picker);
        // Wait only for the initial native/DWM presentation, without printing
        // or requesting a repaint. Navigation observations below never retry.
        let ready = Instant::now() + Duration::from_secs(2);
        loop {
            pump_for(Duration::from_millis(20));
            let frame = screen_calendar(calendar);
            let pixels =
                std::slice::from_raw_parts(frame.bits, (frame.width * frame.height) as usize);
            if pixels.iter().filter(|pixel| luma(**pixel) < 110).count() * 100 >= pixels.len() * 65
            {
                dark(&frame);
                break;
            }
            assert!(
                Instant::now() < ready,
                "initial native screen presentation stayed light"
            );
        }
        assert_popup_layout(calendar);
        let mut samples = Vec::new();
        for navigation in 0..3 {
            if navigation == 0 {
                SendMessageW(calendar, WM_KEYDOWN, 0x22, 0);
                SendMessageW(calendar, WM_KEYUP, 0x22, 0);
            } else {
                let point = hit_point(calendar, MCHT_TITLEBTNNEXT);
                let packed = ((point.y << 16) | point.x) as LPARAM;
                SendMessageW(calendar, WM_LBUTTONDOWN, 1, packed);
                SendMessageW(calendar, WM_LBUTTONUP, 0, packed);
            }
            let start = Instant::now();
            for millis in [0, 30, 100, 350, 1000] {
                pump_for(Duration::from_millis(millis).saturating_sub(start.elapsed()));
                samples.push(screen_calendar(calendar));
            }
            assert!(!active(calendar));
        }
        // Analyze AFTER the screen-only observation; no printing repairs frames.
        for frame in &samples {
            dark(frame);
        }
        assert_grid_and_ink(calendar, samples.last().unwrap());
        let mut selected = SYSTEMTIME::default();
        assert_ne!(
            SendMessageW(
                calendar,
                MCM_GETCURSEL,
                0,
                (&mut selected as *mut SYSTEMTIME) as LPARAM
            ),
            0
        );
        assert_eq!((selected.wYear, selected.wMonth), (2027, 1));
        SendMessageW(picker, DTM_CLOSEMONTHCAL, 0, 0);
        println!(
            "{} screen-only navigation frames remained dark with native content",
            samples.len()
        );
    }
}

#[test]
fn native_dark_popup_grid_footer_colors_and_theme_restore_at_three_fonts() {
    let _visual_styles = VisualStyles::new();
    let host = Host::new();
    // SAFETY: owned offscreen windows, font handles released only after controls.
    unsafe {
        ShowWindow(host.0, SW_SHOWNOACTIVATE);
        MoveWindow(host.0, -4000, -4000, 1600, 1200, 0);
        for id in [2224, 2225, 2302, 2303] {
            let picker = date_picker::create(host.0, id, "开始（含）").unwrap();
            SendMessageW(picker, WM_SETTEXT, 0, wide("2024-02-15").as_ptr() as LPARAM);
            SendMessageW(picker, DTM_SETRANGE, 0, 0);
            for dpi in [96, 144, 192] {
                let font = Font::new(dpi).unwrap();
                MoveWindow(picker, 0, 0, scale(336, dpi), scale(28, dpi), 0);
                SendMessageW(picker, WM_SETFONT, font.0 as WPARAM, 0);
                date_picker::apply(picker, Resolved::Dark, palette(), dpi);
                let calendar = popup(picker);
                assert!(!active(calendar));
                assert_eq!(GetWindowTheme(calendar), 0);
                assert_eq!(
                    SendMessageW(calendar, MCM_GETCOLOR, MCSC_MONTHBK as WPARAM, 0) as u32,
                    palette().surface
                );
                assert_popup_layout(calendar);
                let before = value(picker);
                let frame = paint(calendar, WM_PRINTCLIENT);
                assert_grid_and_ink(calendar, &frame);
                let mut rect = RECT::default();
                GetWindowRect(GetParent(calendar), &mut rect);
                for _ in 0..3 {
                    date_picker::apply(picker, Resolved::Dark, palette(), dpi);
                }
                let mut after = RECT::default();
                GetWindowRect(GetParent(calendar), &mut after);
                assert_eq!(
                    (rect.left, rect.top, rect.right, rect.bottom),
                    (after.left, after.top, after.right, after.bottom),
                    "popup layout grew on repeated apply"
                );
                for kind in [Resolved::Light, Resolved::Contrast] {
                    date_picker::apply(picker, kind, palette(), dpi);
                    assert!(!active(calendar));
                    assert_ne!(GetWindowTheme(calendar), 0);
                    assert_eq!(
                        SendMessageW(calendar, MCM_GETCOLOR, MCSC_MONTHBK as WPARAM, 0) as u32,
                        GetSysColor(COLOR_WINDOW)
                    );
                    assert_popup_layout(calendar);
                    assert_eq!(value(picker), before);
                    date_picker::apply(picker, Resolved::Dark, palette(), dpi);
                }
                SendMessageW(picker, DTM_CLOSEMONTHCAL, 0, 0);
                SendMessageW(picker, WM_SETFONT, 0, 0);
            }
            DestroyWindow(picker);
        }
    }
}

#[test]
fn native_popup_open_navigate_theme_close_resources_stay_bounded() {
    use windows_sys::Win32::System::Threading::{
        GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
    };
    let _visual_styles = VisualStyles::new();
    let host = Host::new();
    // SAFETY: own controls, no clipboard or app data. Warm the entire popup cycle
    // including themed fallback and native font before checking exact GDI counts.
    unsafe {
        let picker = date_picker::create(host.0, 2224, "开始（含）").unwrap();
        let font = Font::new(96).unwrap();
        SendMessageW(picker, WM_SETFONT, font.0 as WPARAM, 0);
        MoveWindow(picker, 0, 0, 336, 28, 0);
        MoveWindow(host.0, -4000, -4000, 1600, 1200, 0);
        ShowWindow(host.0, SW_SHOWNOACTIVATE);
        let once = || {
            SendMessageW(picker, WM_SETTEXT, 0, wide("2026-10-25").as_ptr() as LPARAM);
            date_picker::apply(picker, Resolved::Dark, palette(), 96);
            let calendar = popup(picker);
            SendMessageW(calendar, WM_KEYDOWN, 0x22, 0);
            SendMessageW(calendar, WM_KEYUP, 0x22, 0);
            drop(paint(calendar, WM_PRINTCLIENT));
            date_picker::apply(picker, Resolved::Light, palette(), 96);
            drop(paint(calendar, WM_PRINTCLIENT));
            SendMessageW(picker, DTM_CLOSEMONTHCAL, 0, 0);
            assert_eq!(IsWindow(calendar), 0);
            GdiFlush();
            GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS)
        };
        once();
        let before = once();
        for _ in 0..30 {
            assert_eq!(once(), before);
        }
        SendMessageW(picker, WM_SETFONT, 0, 0);
    }
}

#[test]
fn expanded_native_popup_fits_monitor_work_area_at_right_and_bottom_edge() {
    let _visual_styles = VisualStyles::new();
    let host = Host::new();
    // SAFETY: own popup near the current monitor edge; no desktop state changed.
    unsafe {
        let work = crate::ui::overview::work_area(host.0).unwrap();
        SetWindowPos(
            host.0,
            null_mut(),
            work.right - 350,
            work.bottom - 80,
            350,
            80,
            SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOZORDER,
        );
        let picker = date_picker::create(host.0, 2224, "开始（含）").unwrap();
        MoveWindow(picker, 0, 0, 336, 28, 0);
        SendMessageW(picker, WM_SETTEXT, 0, wide("2026-10-25").as_ptr() as LPARAM);
        date_picker::apply(picker, Resolved::Dark, palette(), 96);
        let calendar = popup(picker);
        assert_popup_layout(calendar);
        SendMessageW(picker, DTM_CLOSEMONTHCAL, 0, 0);
    }
}
