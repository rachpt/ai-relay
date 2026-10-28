use super::*;

struct ScrollPaintProbe {
    samples: Cell<usize>,
    native_track: Cell<u32>,
}

unsafe fn physical_track(hwnd: HWND, vertical: bool) -> u32 {
    // SAFETY: read only actual desktop pixels owned by this test window. Refuse
    // occluded samples rather than passing a final/offscreen frame as evidence.
    unsafe {
        let bar = native_bar(hwnd, vertical);
        let point = if vertical {
            POINT {
                x: bar.rcScrollBar.left + 1,
                y: (bar.rcScrollBar.top + bar.rcScrollBar.bottom) / 2,
            }
        } else {
            POINT {
                x: (bar.rcScrollBar.left + bar.rcScrollBar.right) / 2,
                y: bar.rcScrollBar.top + 1,
            }
        };
        assert_eq!(
            WindowFromPoint(point),
            hwnd,
            "physical scrollbar sample is occluded"
        );
        let dc = GetDC(null_mut());
        assert!(!dc.is_null());
        GdiFlush();
        let color = GetPixel(dc, point.x, point.y);
        ReleaseDC(null_mut(), dc);
        assert_eq!(
            WindowFromPoint(point),
            hwnd,
            "physical scrollbar ownership changed"
        );
        assert_ne!(color, CLR_INVALID);
        color
    }
}

unsafe extern "system" fn sample_native_scroll(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: the stack probe outlives the test-owned child. Sample BEFORE
    // returning to the dark overlay, never use a ListView DC to trigger a flush.
    unsafe {
        let result = DefSubclassProc(hwnd, message, w, l);
        if matches!(
            message,
            LVM_SCROLL | WM_VSCROLL | WM_HSCROLL | LVM_ENSUREVISIBLE | WM_MOUSEWHEEL
        ) {
            let probe = &*(data as *const ScrollPaintProbe);
            for vertical in [true, false] {
                let color = physical_track(hwnd, vertical);
                probe.native_track.set(color);
                assert_eq!(
                    color,
                    palette().background,
                    "native scroll exposed a light track before dark overlay"
                );
                probe.samples.set(probe.samples.get() + 1);
            }
        }
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(sample_native_scroll), id);
        }
        result
    }
}

unsafe fn report_fixture(parent: HWND) -> HWND {
    // SAFETY: regular report ListView like the history tables, synthetic text.
    unsafe {
        let init = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        };
        assert_ne!(InitCommonControlsEx(&init), 0);
        let hwnd = window(
            "SysListView32",
            WS_CHILD | WS_VISIBLE | WS_BORDER | LVS_REPORT,
            parent,
        );
        let empty = [0u16];
        SetWindowTheme(hwnd, empty.as_ptr(), empty.as_ptr());
        let mut column = LVCOLUMNW {
            mask: LVCF_WIDTH,
            cx: 1200,
            ..Default::default()
        };
        SendMessageW(
            hwnd,
            LVM_INSERTCOLUMNW,
            0,
            (&mut column as *mut LVCOLUMNW) as LPARAM,
        );
        for row in 0..100 {
            let mut item = LVITEMW {
                iItem: row,
                ..Default::default()
            };
            assert_eq!(
                SendMessageW(
                    hwnd,
                    LVM_INSERTITEMW,
                    0,
                    (&mut item as *mut LVITEMW) as LPARAM
                ),
                row as LRESULT
            );
        }
        hwnd
    }
}

#[test]
#[ignore = "requires an unobscured interactive desktop; run explicitly and serially"]
fn report_scroll_never_exposes_native_track_before_dark_overlay() {
    // SAFETY: own visible HWND only, no application data, network or clipboard.
    unsafe {
        let probe = ScrollPaintProbe {
            samples: Cell::new(0),
            native_track: Cell::new(0),
        };
        let calls = Cell::new(0);
        let parent = Window(window("STATIC", WS_POPUP | WS_VISIBLE, null_mut()));
        let hwnd = report_fixture(parent.0);
        assert_ne!(
            SetWindowSubclass(
                hwnd,
                Some(sample_native_scroll),
                SUBCLASS_ID + 30,
                (&probe as *const ScrollPaintProbe) as usize
            ),
            0
        );
        tracking_probe(hwnd, &calls);
        apply(hwnd, Resolved::Dark, palette(), 96);
        SetWindowPos(parent.0, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        ShowWindow(parent.0, SW_SHOW);
        SetForegroundWindow(parent.0);
        RedrawWindow(
            parent.0,
            null(),
            null_mut(),
            RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
        let mut message = MSG::default();
        while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        windows_sys::Win32::Graphics::Dwm::DwmFlush();
        for vertical in [true, false] {
            assert_eq!(physical_track(hwnd, vertical), palette().background);
            let before = scroll(hwnd, if vertical { SB_VERT } else { SB_HORZ }).3;
            let mut point = press(hwnd, vertical, Part::Thumb);
            for _ in 0..8 {
                if vertical {
                    point.y += 3;
                } else {
                    point.x += 3;
                }
                captured_mouse(hwnd, WM_MOUSEMOVE, point);
                assert_eq!(GetCapture(), hwnd);
                for axis in [true, false] {
                    assert_eq!(physical_track(hwnd, axis), palette().background);
                }
            }
            assert!(scroll(hwnd, if vertical { SB_VERT } else { SB_HORZ }).3 > before);
            captured_mouse(hwnd, WM_LBUTTONUP, point);
            SendMessageW(
                hwnd,
                if vertical { WM_VSCROLL } else { WM_HSCROLL },
                SB_TOP as WPARAM,
                0,
            );
            let point = press(hwnd, vertical, Part::PageAfter);
            SendMessageW(hwnd, WM_TIMER, TRACK_TIMER, 0);
            assert_eq!(GetCapture(), hwnd);
            assert_eq!(physical_track(hwnd, vertical), palette().background);
            captured_mouse(hwnd, WM_LBUTTONUP, point);
        }
        SendMessageW(hwnd, LVM_ENSUREVISIBLE, 90, 0);
        SendMessageW(hwnd, WM_MOUSEWHEEL, (120u32 << 16) as WPARAM, 0);
        windows_sys::Win32::Graphics::Dwm::DwmFlush();
        for vertical in [true, false] {
            assert_eq!(physical_track(hwnd, vertical), palette().background);
        }
        assert!(probe.samples.get() >= 40);
        assert_eq!(calls.get(), 0);
        assert_ne!(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE, 0);
        assert!(GetPropW(hwnd, windows_sys::w!("SysSetRedraw")).is_null());
        println!(
            "{} intermediate physical track samples remained dark",
            probe.samples.get()
        );
    }
}

#[test]
fn report_compositing_is_local_owned_and_restored_in_light_and_contrast() {
    // SAFETY: hidden test windows; preserve styles installed by other owners.
    unsafe {
        let parent = Window(window("STATIC", WS_POPUP, null_mut()));
        let hwnd = report_fixture(parent.0);
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let native = [scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)];
        for kind in [Resolved::Light, Resolved::Contrast] {
            apply(hwnd, Resolved::Dark, palette(), 96);
            assert!(state(hwnd).unwrap().get().composited);
            assert_eq!(
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE),
                style | WS_EX_COMPOSITED as isize
            );
            assert_eq!(
                GetWindowLongPtrW(parent.0, GWL_EXSTYLE) as u32 & WS_EX_COMPOSITED,
                0
            );
            assert_eq!([scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)], native);
            apply(hwnd, kind, palette(), 96);
            assert_eq!(GetWindowLongPtrW(hwnd, GWL_EXSTYLE), style);
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_COMPOSITED as isize);
        apply(hwnd, Resolved::Dark, palette(), 96);
        assert!(!state(hwnd).unwrap().get().composited);
        apply(hwnd, Resolved::Light, palette(), 96);
        assert_ne!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_COMPOSITED,
            0
        );
    }
}

struct RegionProbe {
    count: Cell<usize>,
    original: HRGN,
    armed: Cell<bool>,
}
unsafe extern "system" fn inspect_nc_region(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: stack probe and borrowed original region outlive the child.
    unsafe {
        if message == WM_NCPAINT {
            let probe = &*(data as *const RegionProbe);
            if !probe.armed.get() {
                return DefSubclassProc(hwnd, message, w, l);
            }
            assert_ne!(w, 1);
            let region = w as HRGN;
            let v = native_bar(hwnd, true).rcScrollBar;
            let h = native_bar(hwnd, false).rcScrollBar;
            for (x, y) in [
                (v.left + 1, v.top + 20),
                (h.left + 20, h.top + 1),
                (v.left + 1, h.top + 1),
            ] {
                assert_eq!(PtInRegion(region, x, y), 0);
            }
            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect);
            assert_ne!(
                PtInRegion(region, rect.left, rect.top),
                0,
                "native border was removed"
            );
            if !probe.original.is_null() {
                assert_ne!(
                    PtInRegion(probe.original, v.left + 1, v.top + 20),
                    0,
                    "borrowed update region mutated"
                );
            }
            probe.count.set(probe.count.get() + 1);
        }
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(inspect_nc_region), id);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

#[test]
fn native_nc_region_excludes_both_bars_and_corner_but_preserves_border_and_borrowed_region() {
    // SAFETY: owned hidden controls/regions only, no screen pixel assumptions.
    unsafe {
        let mut probe = RegionProbe {
            count: Cell::new(0),
            original: null_mut(),
            armed: Cell::new(false),
        };
        let parent = Window(window("STATIC", WS_POPUP, null_mut()));
        let hwnd = report_fixture(parent.0);
        assert_ne!(
            SetWindowSubclass(
                hwnd,
                Some(inspect_nc_region),
                SUBCLASS_ID + 31,
                (&probe as *const RegionProbe) as usize
            ),
            0
        );
        apply(hwnd, Resolved::Dark, palette(), 96);
        probe.armed.set(true);
        SendMessageW(hwnd, WM_NCPAINT, 1, 0);
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect);
        probe.original = CreateRectRgnIndirect(&rect);
        SendMessageW(hwnd, WM_NCPAINT, probe.original as WPARAM, 0);
        assert_eq!(probe.count.get(), 2);
        RemoveWindowSubclass(hwnd, Some(inspect_nc_region), SUBCLASS_ID + 31);
        DeleteObject(probe.original);
    }
}

#[test]
fn composited_report_never_bypasses_no_redraw_and_does_not_hold_dc_for_menu_keys() {
    // SAFETY: scratch hidden windows. No real menu or clipboard is invoked.
    unsafe {
        let parent = Window(window("STATIC", WS_POPUP, null_mut()));
        let hwnd = report_fixture(parent.0);
        apply(hwnd, Resolved::Dark, palette(), 96);
        let retained = state(hwnd).unwrap();
        assert!(paint::ScrollPaint::begin(hwnd, &retained, WM_KEYDOWN, 0x5d).is_none());
        assert!(paint::ScrollPaint::begin(hwnd, &retained, WM_KEYDOWN, 0x79).is_none());
        assert!(paint::ScrollPaint::begin(hwnd, &retained, WM_SIZE, 0).is_none());
        let dc = GetWindowDC(hwnd);
        let before = SCREEN_PASSES.with(Cell::get);
        SendMessageW(hwnd, WM_SETREDRAW, 0, 0);
        overlay_dc(hwnd, &retained, dc, false);
        assert_eq!(SCREEN_PASSES.with(Cell::get), before);
        assert!(paint::ScrollPaint::begin(hwnd, &retained, LVM_SCROLL, 0).is_none());
        SendMessageW(hwnd, WM_SETREDRAW, 1, 0);
        ReleaseDC(hwnd, dc);
    }
}

struct ThemeProbe {
    message: u32,
    target: Resolved,
    destroy: bool,
    armed: Cell<bool>,
    fired: Cell<bool>,
}
unsafe extern "system" fn theme_during_style(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: one-shot probe lives longer than its owned HWND; never borrow
    // application state while invoking synchronous theme/destruction callbacks.
    unsafe {
        let probe = &*(data as *const ThemeProbe);
        if message == probe.message && w == GWL_EXSTYLE as WPARAM && probe.armed.replace(false) {
            probe.fired.set(true);
            if probe.destroy {
                DestroyWindow(hwnd);
                return 0;
            }
            apply(hwnd, probe.target, palette(), 96);
        }
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(theme_during_style), id);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

#[test]
fn reentrant_style_theme_requests_keep_latest_intent_and_owned_style() {
    // SAFETY: hidden local HWNDs, one-shot style notifications on owner thread.
    unsafe {
        for message in [WM_STYLECHANGING, WM_STYLECHANGED] {
            for target in [Resolved::Light, Resolved::Contrast, Resolved::Dark] {
                for destroy in [false, true] {
                    let probe = ThemeProbe {
                        message,
                        target,
                        destroy,
                        armed: Cell::new(false),
                        fired: Cell::new(false),
                    };
                    let parent = Window(window("STATIC", WS_POPUP, null_mut()));
                    let hwnd = report_fixture(parent.0);
                    assert_ne!(
                        SetWindowSubclass(
                            hwnd,
                            Some(theme_during_style),
                            SUBCLASS_ID + 32,
                            (&probe as *const ThemeProbe) as usize
                        ),
                        0
                    );
                    if target == Resolved::Dark {
                        apply(hwnd, Resolved::Dark, palette(), 96);
                    }
                    probe.armed.set(true);
                    apply(
                        hwnd,
                        if target == Resolved::Dark {
                            Resolved::Light
                        } else {
                            Resolved::Dark
                        },
                        palette(),
                        96,
                    );
                    assert!(probe.fired.get());
                    if destroy {
                        assert_eq!(IsWindow(hwnd), 0);
                        continue;
                    }
                    let retained = state(hwnd);
                    assert_eq!(retained.is_some(), target == Resolved::Dark);
                    assert_eq!(
                        GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_COMPOSITED != 0,
                        target == Resolved::Dark
                    );
                    if let Some(retained) = retained {
                        assert!(retained.get().enabled && retained.get().composited);
                        assert!(!retained.get().changing_style);
                    }
                }
            }
        }
    }
}

#[test]
fn report_composited_scroll_and_theme_cycles_do_not_leak_gdi() {
    // SAFETY: test-owned hidden report and balanced transient DC/region resources.
    unsafe {
        let parent = Window(window("STATIC", WS_POPUP, null_mut()));
        let hwnd = report_fixture(parent.0);
        let once = || {
            apply(hwnd, Resolved::Dark, palette(), 96);
            for _ in 0..4 {
                SendMessageW(hwnd, LVM_SCROLL, 8, 20);
                SendMessageW(hwnd, WM_NCPAINT, 1, 0);
            }
            SendMessageW(hwnd, WM_VSCROLL, SB_TOP as WPARAM, 0);
            apply(hwnd, Resolved::Light, palette(), 96);
            GdiFlush();
            GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS)
        };
        once();
        let before = once();
        for _ in 0..100 {
            assert_eq!(once(), before);
        }
    }
}
