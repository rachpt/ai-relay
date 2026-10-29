use super::*;
use std::{cell::Cell, ptr::null};
mod physical_tests {
    include!("chrome_physical_tests.rs");
}

fn inside(rect: RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}
unsafe fn outer_client(hwnd: HWND) -> (RECT, RECT) {
    // SAFETY: Actual native geometry, expressed in window-relative coordinates.
    unsafe {
        let mut outer = RECT::default();
        assert_ne!(GetWindowRect(hwnd, &mut outer), 0);
        let client = client_screen(hwnd);
        (
            outer,
            RECT {
                left: client.left - outer.left,
                top: client.top - outer.top,
                right: client.right - outer.left,
                bottom: client.bottom - outer.top,
            },
        )
    }
}
unsafe fn children(hwnd: HWND) -> Vec<(HWND, RECT)> {
    // SAFETY: Owner-thread queries only; no callback or cached foreign ownership.
    unsafe {
        let (outer, _) = outer_client(hwnd);
        let mut result = Vec::new();
        let mut child = GetWindow(hwnd, GW_CHILD);
        while !child.is_null() {
            let (rect, _) = outer_client(child);
            result.push((
                child,
                RECT {
                    left: rect.left - outer.left,
                    top: rect.top - outer.top,
                    right: rect.right - outer.left,
                    bottom: rect.bottom - outer.top,
                },
            ));
            child = GetWindow(child, GW_HWNDNEXT);
        }
        result
    }
}
unsafe fn light_nonclient(hwnd: HWND, raster: &Frame) -> usize {
    // SAFETY: Bounded native baseline; report child/host white NC independently.
    unsafe {
        let (_, client) = outer_client(hwnd);
        (0..raster.height)
            .flat_map(|y| (0..raster.width).map(move |x| (x, y)))
            .filter(|(x, y)| !inside(client, *x, *y) && luma(GetPixel(raster.dc, *x, *y)) > 150)
            .count()
    }
}
unsafe fn assert_frame_pixels(hwnd: HWND, raster: &Frame) {
    // SAFETY: Check the thin NC ring and host gutters independently, not a
    // mostly-dark whole bitmap that could hide the reported white border.
    unsafe {
        let (outer, client) = outer_client(hwnd);
        assert_eq!(
            (raster.width, raster.height),
            (outer.right - outer.left, outer.bottom - outer.top)
        );
        let host = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_POPUP != 0;
        let children = children(hwnd);
        for y in 0..raster.height {
            for x in 0..raster.width {
                if !inside(client, x, y) {
                    assert_eq!(
                        GetPixel(raster.dc, x, y),
                        palette().border,
                        "native NC bevel at {x},{y}"
                    );
                } else if host && !children.iter().any(|(_, rect)| inside(*rect, x, y)) {
                    assert_eq!(
                        GetPixel(raster.dc, x, y),
                        palette().surface,
                        "native DropDown gutter at {x},{y}"
                    );
                }
            }
        }
    }
}
pub(super) unsafe fn assert_printed_chrome(hwnd: HWND) {
    // SAFETY: Real native WM_PRINT, including the calendar inside DropDown.
    unsafe { assert_frame_pixels(hwnd, &paint(hwnd, WM_PRINT)) };
}
pub(super) unsafe fn assert_native_chrome_restored(hwnd: HWND, native: &Frame) {
    // SAFETY: Same native date/font; compare only outer NC and host gutters.
    unsafe {
        let current = paint(hwnd, WM_PRINT);
        let (_, client) = outer_client(hwnd);
        let host = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_POPUP != 0;
        let children = children(hwnd);
        assert_eq!(
            (current.width, current.height),
            (native.width, native.height)
        );
        for y in 0..current.height {
            for x in 0..current.width {
                if !inside(client, x, y)
                    || (host && !children.iter().any(|(_, rect)| inside(*rect, x, y)))
                {
                    assert_eq!(GetPixel(current.dc, x, y), GetPixel(native.dc, x, y));
                }
            }
        }
    }
}
pub(super) unsafe fn assert_screen_chrome(calendar: HWND) {
    // SAFETY: Physical observation only. Do not print, invalidate or repair a
    // navigation frame; reject occlusion rather than sampling another window.
    unsafe {
        for hwnd in [calendar, date_picker::popup::native_host(calendar).unwrap()] {
            let (outer, _) = outer_client(hwnd);
            for point in [
                POINT {
                    x: outer.left,
                    y: outer.top,
                },
                POINT {
                    x: outer.right - 1,
                    y: outer.bottom - 1,
                },
            ] {
                let hit = WindowFromPoint(point);
                assert!(
                    hit == hwnd || IsChild(hwnd, hit) != 0,
                    "popup chrome is occluded"
                );
            }
            let frame = Frame::new(outer.right - outer.left, outer.bottom - outer.top).unwrap();
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
                    outer.left,
                    outer.top,
                    SRCCOPY
                ),
                0
            );
            ReleaseDC(null_mut(), screen);
            GdiFlush();
            assert_frame_pixels(hwnd, &frame);
        }
    }
}
unsafe fn fixture(host: HWND) -> (HWND, HWND) {
    // SAFETY: Private offscreen native windows; no system/app settings change.
    unsafe {
        MoveWindow(host, -4000, -4000, 1600, 1200, 0);
        ShowWindow(host, SW_SHOWNOACTIVATE);
        let picker = date_picker::create(host, 2224, "开始（含）").unwrap();
        MoveWindow(picker, 0, 0, 336, 28, 0);
        SendMessageW(picker, WM_SETTEXT, 0, wide("2026-10-15").as_ptr() as LPARAM);
        date_picker::apply(picker, Resolved::Dark, palette(), 96);
        (picker, popup(picker))
    }
}

#[test]
fn native_popup_border_leaves_client_navigation_geometry_and_styles_unchanged() {
    let _styles = VisualStyles::new();
    let host = Host::new();
    // SAFETY: Owned native popup; compare classic six-color native rendering
    // without presentation to the adapted output, including unmodified arrows.
    unsafe {
        let (picker, calendar) = fixture(host.0);
        let popup_host = date_picker::popup::native_host(calendar).unwrap();
        date_picker::popup::apply(calendar, Resolved::Light, palette());
        let native = paint(calendar, WM_PRINT);
        let native_host = paint(popup_host, WM_PRINT);
        println!(
            "classic light NC pixels: calendar={}, DropDown={}",
            light_nonclient(calendar, &native),
            light_nonclient(popup_host, &native_host)
        );
        let snapshot = |hwnd| {
            let (outer, client) = outer_client(hwnd);
            (
                GetWindowLongPtrW(hwnd, GWL_STYLE),
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE),
                [
                    outer.left,
                    outer.top,
                    outer.right,
                    outer.bottom,
                    client.left,
                    client.top,
                    client.right,
                    client.bottom,
                ],
                children(hwnd)
                    .into_iter()
                    .map(|(hwnd, _)| hwnd)
                    .collect::<Vec<_>>(),
            )
        };
        let before = [calendar, popup_host].map(snapshot);
        let colors = [
            COLOR_BTNFACE,
            COLOR_BTNHIGHLIGHT,
            COLOR_BTNSHADOW,
            COLOR_WINDOW,
        ]
        .map(|part| GetSysColor(part));
        date_picker::popup::apply(calendar, Resolved::Dark, palette());
        assert_eq!([calendar, popup_host].map(snapshot), before);
        assert!(!date_picker::popup::active(host.0));
        assert!(!date_picker::popup::active(picker));
        let (_, client) = outer_client(calendar);
        let adapted = paint(calendar, WM_PRINT);
        for y in client.top..client.bottom {
            for x in client.left..client.right {
                assert_eq!(
                    GetPixel(adapted.dc, x, y),
                    GetPixel(native.dc, x, y),
                    "border overlay changed native calendar/navigation content"
                );
            }
        }
        let adapted_host = paint(popup_host, WM_PRINT);
        let alive = date_picker::popup::owner_alive(calendar).unwrap();
        for (_, rect) in children(popup_host) {
            for y in (rect.top + client.top)..(rect.top + client.bottom) {
                for x in (rect.left + client.left)..(rect.left + client.right) {
                    assert_eq!(
                        GetPixel(adapted_host.dc, x, y),
                        GetPixel(native_host.dc, x, y),
                        "host gutter overlay changed calendar content"
                    );
                }
            }
        }
        assert_printed_chrome(calendar);
        assert_printed_chrome(popup_host);
        assert_grid_and_ink(calendar, &paint(calendar, WM_PRINTCLIENT));

        let foreign = Host::new();
        let separate = CreateWindowExW(
            0,
            windows_sys::w!("SysMonthCal32"),
            null(),
            WS_CHILD,
            0,
            0,
            240,
            240,
            foreign.0,
            null_mut(),
            null_mut(),
            null(),
        );
        assert!(!separate.is_null());
        assert!(date_picker::popup::native_host(separate).is_none());
        date_picker::fit_popup_host(separate);
        assert!(!date_picker::popup::active(foreign.0));
        let selected = value(picker);
        for kind in [Resolved::Light, Resolved::Contrast] {
            let cycle_alive = date_picker::popup::owner_alive(calendar).unwrap();
            date_picker::apply(picker, kind, palette(), 96);
            assert!(!date_picker::popup::active(calendar));
            assert!(!date_picker::popup::active(popup_host));
            assert!(
                !alive() && !cycle_alive(),
                "removed border subclasses retained their owner"
            );
            assert_ne!(GetWindowTheme(calendar), 0);
            assert_eq!(value(picker), selected);
            assert_popup_layout(calendar);
            date_picker::apply(picker, Resolved::Dark, palette(), 96);
        }
        assert_eq!(
            [
                COLOR_BTNFACE,
                COLOR_BTNHIGHLIGHT,
                COLOR_BTNSHADOW,
                COLOR_WINDOW
            ]
            .map(|part| GetSysColor(part)),
            colors
        );
        SendMessageW(picker, DTM_CLOSEMONTHCAL, 0, 0);
    }
}

#[test]
fn native_popup_print_flags_keep_real_children_and_unrequested_gutters_untouched() {
    let _styles = VisualStyles::new();
    let host = Host::new();
    // SAFETY: Private native EDIT makes the calendar's own CHILDREN path
    // non-vacuous. It is not a production overlay or a substitute calendar.
    unsafe {
        let (picker, calendar) = fixture(host.0);
        assert_grid_and_ink(calendar, &paint(calendar, WM_PRINTCLIENT));
        let child = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            windows_sys::w!("EDIT"),
            wide("Native child 123").as_ptr(),
            WS_CHILD | WS_VISIBLE | ES_READONLY as u32,
            19,
            45,
            118,
            24,
            calendar,
            null_mut(),
            null_mut(),
            null(),
        );
        assert!(!child.is_null());
        let popup_host = date_picker::popup::native_host(calendar).unwrap();
        for hwnd in [calendar, popup_host] {
            let (outer, client) = outer_client(hwnd);
            let descendants = children(hwnd);
            assert!(!descendants.is_empty());

            let native = paint(hwnd, WM_PRINT);
            let native_client = sentinel_target(native.width, native.height, 1, (0, 0));
            checked_print(hwnd, WM_PRINT, PRF_CLIENT | PRF_ERASEBKGND, &native_client);
            if hwnd == popup_host {
                assert!(
                    client.right - client.left > descendants[0].1.right - descendants[0].1.left
                        || client.bottom - client.top
                            > descendants[0].1.bottom - descendants[0].1.top,
                    "native host must expose a real gutter for sentinel assertions"
                );
            }

            for (zoom, offset) in [(1, (0, 0)), (2, (7, 9))] {
                // Render references at the same scale, rather than assuming
                // that scaled native font rasterization is pixel replication.
                let references: Vec<_> = descendants
                    .iter()
                    .map(|(child, rect)| {
                        let raster = sentinel_target(
                            rect.right - rect.left,
                            rect.bottom - rect.top,
                            zoom,
                            (0, 0),
                        );
                        {
                            // Match checked_print's input attributes: native
                            // calendar text alignment/colors affect its raster.
                            // Save/restore separately because the raw EDIT may
                            // change its borrowed DC, unlike our popup wrapper.
                            SetTextColor(raster.dc, 0x123456);
                            SetBkColor(raster.dc, 0x654321);
                            SetBkMode(raster.dc, OPAQUE as i32);
                            SetDCBrushColor(raster.dc, 0x010203);
                            SetDCPenColor(raster.dc, 0x030201);
                            SetTextAlign(raster.dc, TA_RIGHT | TA_BOTTOM);
                            MoveToEx(raster.dc, 7, 9, null_mut());
                            let _saved = SavedDc::new(raster.dc).unwrap();
                            SendMessageW(
                                *child,
                                WM_PRINT,
                                raster.dc as WPARAM,
                                (PRF_CLIENT | PRF_NONCLIENT | PRF_ERASEBKGND | PRF_CHILDREN)
                                    as LPARAM,
                            );
                        }
                        GdiFlush();
                        let mut ink = 0;
                        for y in 3 * zoom..(rect.bottom - rect.top - 3) * zoom {
                            for x in 3 * zoom..(rect.right - rect.left - 3) * zoom {
                                ink += usize::from(
                                    luma(*raster.bits.add((y * raster.width + x) as usize)) < 80,
                                );
                            }
                        }
                        assert!(ink > 10, "native child reference contains no real ink");
                        (*rect, raster)
                    })
                    .collect();
                for flags in [
                    0,
                    PRF_CHECKVISIBLE,
                    PRF_NONCLIENT,
                    PRF_CLIENT,
                    PRF_ERASEBKGND,
                    PRF_CHILDREN,
                    PRF_NONCLIENT | PRF_CHILDREN,
                    PRF_CLIENT | PRF_CHILDREN,
                    PRF_CLIENT | PRF_NONCLIENT | PRF_CHILDREN,
                ] {
                    let parent = sentinel_target(native.width, native.height, zoom, offset);
                    checked_print(hwnd, WM_PRINT, flags & !PRF_CHILDREN, &parent);
                    if zoom == 1 && flags & PRF_CLIENT != 0 && flags & PRF_NONCLIENT == 0 {
                        assert_print_pixels(&parent, &native_client, zoom, offset, |x, y| {
                            inside(client, x, y)
                        });
                    }
                    let target = sentinel_target(
                        outer.right - outer.left,
                        outer.bottom - outer.top,
                        zoom,
                        offset,
                    );
                    checked_print(hwnd, WM_PRINT, flags, &target);

                    let mut child_pixels = vec![0; descendants.len()];
                    let pixels = std::slice::from_raw_parts(
                        target.bits,
                        (target.width * target.height) as usize,
                    );
                    let parent_pixels = std::slice::from_raw_parts(
                        parent.bits,
                        (parent.width * parent.height) as usize,
                    );
                    for y in 0..target.height {
                        for x in 0..target.width {
                            let px = (x - offset.0).div_euclid(zoom);
                            let py = (y - offset.1).div_euclid(zoom);
                            let in_window =
                                px >= 0 && py >= 0 && px < native.width && py < native.height;
                            let child = references
                                .iter()
                                .position(|(rect, _)| inside(*rect, px, py));
                            let expected = if in_window
                                && inside(client, px, py)
                                && flags & PRF_CHILDREN != 0
                                && let Some(index) = child
                            {
                                let (rect, raster) = &references[index];
                                let rx = x - offset.0 - rect.left * zoom;
                                let ry = y - offset.1 - rect.top * zoom;
                                let color = *raster.bits.add((ry * raster.width + rx) as usize);
                                child_pixels[index] += usize::from(
                                    color & 0x00ff_ffff != dib(PRINT_SENTINEL) & 0x00ff_ffff,
                                );
                                color
                            } else if in_window
                                && !inside(client, px, py)
                                && flags & PRF_NONCLIENT != 0
                            {
                                dib(palette().border)
                            } else if in_window
                                && (if inside(client, px, py) {
                                    flags & (PRF_CLIENT | PRF_ERASEBKGND) != 0
                                } else {
                                    flags & PRF_NONCLIENT != 0
                                })
                            {
                                parent_pixels[(y * parent.width + x) as usize]
                            } else {
                                dib(PRINT_SENTINEL)
                            };

                            assert_eq!(
                                pixels[(y * target.width + x) as usize] & 0x00ff_ffff,
                                expected & 0x00ff_ffff,
                                "popup flags={flags:#x}, pixel={x},{y}, zoom={zoom}"
                            );
                        }
                    }
                    if flags & PRF_CHILDREN != 0 {
                        assert!(
                            child_pixels.iter().all(|count| *count > 100),
                            "requested native child content was clipped"
                        );
                    }
                }
            }
            // PRINTCLIENT zero flags still mean client pixels; CHILDREN adds
            // native descendants without shifting them by the parent's NC inset.
            for flags in [0, PRF_CHILDREN] {
                let target = sentinel_target(native.width, native.height, 1, (0, 0));
                checked_print(hwnd, WM_PRINTCLIENT, flags, &target);
                let parent = sentinel_target(native.width, native.height, 1, (0, 0));
                checked_print(hwnd, WM_PRINT, PRF_CLIENT | flags, &parent);
                for y in 0..target.height {
                    for x in 0..target.width {
                        let expected =
                            if x < client.right - client.left && y < client.bottom - client.top {
                                GetPixel(parent.dc, x + client.left, y + client.top)
                            } else {
                                PRINT_SENTINEL
                            };
                        assert_eq!(
                            GetPixel(target.dc, x, y),
                            expected,
                            "PRINTCLIENT client origin, flags={flags:#x}"
                        );
                    }
                }
            }
        }
        SendMessageW(picker, DTM_CLOSEMONTHCAL, 0, 0);
    }
}

const PROBE_ID: usize = 0x4d43_5453;
#[derive(Clone, Copy)]
enum Action {
    Restore,
    Destroy,
    NoRedraw,
}
struct Probe {
    picker: HWND,
    action: Action,
    armed: Cell<bool>,
}
unsafe extern "system" fn reenter(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: Test owns Probe until dispatch/window teardown completes. Never
    // forward through a window after the probe has synchronously destroyed it.
    unsafe {
        let probe = &*(data as *const Probe);
        let trigger = if matches!(probe.action, Action::NoRedraw) {
            WM_NCACTIVATE
        } else {
            WM_PRINT
        };
        if message == trigger && probe.armed.replace(false) {
            match probe.action {
                Action::Restore => date_picker::popup::apply(hwnd, Resolved::Light, palette()),
                Action::Destroy => {
                    DestroyWindow(probe.picker);
                    return 0;
                }
                Action::NoRedraw => {
                    assert_eq!(l, -1);
                    SendMessageW(hwnd, WM_NCPAINT, 1, 0);
                    SendMessageW(hwnd, WM_PAINT, 0, 0);
                    return 0;
                }
            }
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}
#[test]
fn native_popup_border_survives_nested_restore_destroy_and_no_redraw() {
    let _styles = VisualStyles::new();
    let host = Host::new();
    // SAFETY: Only fixture HWNDs and local subclass state; no global changes.
    unsafe {
        for action in [Action::Restore, Action::Destroy, Action::NoRedraw] {
            let (picker, calendar) = fixture(host.0);
            date_picker::popup::apply(calendar, Resolved::Light, palette());
            let probe = Box::new(Probe {
                picker,
                action,
                armed: Cell::new(true),
            });
            assert_ne!(
                SetWindowSubclass(
                    calendar,
                    Some(reenter),
                    PROBE_ID,
                    (&*probe as *const Probe) as usize
                ),
                0
            );
            date_picker::popup::apply(calendar, Resolved::Dark, palette());
            let alive = date_picker::popup::owner_alive(calendar).unwrap();
            let passes = date_picker::popup::screen_passes();
            if matches!(action, Action::NoRedraw) {
                SendMessageW(calendar, WM_NCACTIVATE, 1, -1);
            } else {
                drop(paint(
                    date_picker::popup::native_host(calendar).unwrap(),
                    WM_PRINT,
                ));
            }
            assert!(!probe.armed.get());
            assert_eq!(date_picker::popup::screen_passes(), passes);
            if matches!(action, Action::Destroy) {
                assert_eq!(IsWindow(calendar), 0);
            } else {
                RemoveWindowSubclass(calendar, Some(reenter), PROBE_ID);
                DestroyWindow(picker);
            }
            assert!(!alive(), "border owner outlived native popup teardown");
        }
    }
}
