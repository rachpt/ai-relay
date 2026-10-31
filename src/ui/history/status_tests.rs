use super::*;
use crate::ui::tests::NativeState;

fn resize(native: &mut NativeState, width: i32, height: i32, dpi: u32) {
    native.state.update_dpi(native.hwnd, dpi).unwrap();
    // SAFETY: Private same-thread hidden HWND. Set final client dimensions using
    // its actual native frame; font parameters do not claim physical monitor DPI.
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

fn measured(control: HWND, width: i32) -> i32 {
    // Independent measurement of the whole native value, without UI ellipsis.
    // SAFETY: Live native STATIC and font; memory DC state/resources are local.
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        assert!(!dc.is_null());
        let font = SendMessageW(control, WM_GETFONT, 0, 0) as HFONT;
        assert!(!font.is_null());
        let old = SelectObject(dc, font);
        let mut rect = RECT {
            right: width,
            ..Default::default()
        };
        let text = wide(&settings::text(control));
        assert!(
            DrawTextW(
                dc,
                text.as_ptr(),
                -1,
                &mut rect,
                DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX
            ) > 0
        );
        SelectObject(dc, old);
        DeleteDC(dc);
        rect.bottom
    }
}

fn layout_matches_text(native: &NativeState, advanced: bool) -> i32 {
    let dpi = native.state.dpi;
    let status = native.rect(native.control(2325));
    let height = status.bottom - status.top;
    let text_height = measured(native.control(2325), status.right - status.left);
    assert!(
        height >= text_height + scale(2, dpi),
        "status text is vertically clipped"
    );
    assert!(height >= scale(34, dpi));
    let sort = native.rect(native.control(DETAIL_SORT));
    if advanced {
        assert_eq!(sort.top, status.bottom + scale(8, dpi));
    } else {
        let chart = native.rect(native.control(CHART));
        assert_eq!(chart.top, status.bottom + scale(8, dpi));
        assert_eq!(chart.bottom - chart.top, scale(200, dpi));
        assert_eq!(sort.top, chart.bottom + scale(8, dpi));
    }
    for id in [GROUP_SORT, EXPORT_SCOPE] {
        assert_eq!(native.rect(native.control(id)).top, sort.top);
    }
    assert_eq!(
        native.rect(native.control(TABLE_VIEW)).top,
        sort.top + scale(36, dpi)
    );
    for id in [2330, 2331, TABLE] {
        let control = native.control(id);
        // Hidden result views retain document coordinates until revealed; only
        // visible controls participate in the viewport's native translation.
        let document_offset =
            if unsafe { GetWindowLongPtrW(control, GWL_STYLE) as u32 & WS_VISIBLE != 0 } {
                0
            } else {
                scale(HEADER_HEIGHT, dpi) + native.scroll(SB_ENDSCROLL)
            };
        assert_eq!(
            native.rect(control).top,
            sort.top + scale(72, dpi) + document_offset
        );
    }
    height
}

fn long_status() -> String {
    let mut report = db::QueryReport {
        summary: db::Summary {
            requests: 3058,
            ..Default::default()
        },
        groups: Vec::new(),
        details: Vec::new(),
        trend: Vec::new(),
        trend_truncated: false,
        aliases: Vec::new(),
    };
    let query = Query {
        limit: 50,
        ..Default::default()
    };
    let counters = db::Counters {
        queued: u64::MAX,
        written: 123456789,
        queue_full: 2,
        db_failures: 3,
        closed: 4,
    };
    report.summary.tokens.total_tokens = Some(496_840_000);
    status_text::query(&query, &report, counters)
}

#[test]
fn native_wrapped_status_uses_real_fonts_and_final_width_without_overlapping_results() {
    let mut native = NativeState::new();
    native.state.show_page(1);
    let status = native.control(2325);
    let draft = native.control(2304);
    settings::set_text(draft, "127.0.0.2");
    // SAFETY: Test-owned edit, scalar UTF-16 selection only.
    unsafe {
        SendMessageW(draft, 0x00b1, 1, 5);
    }
    let text = long_status();
    native.state.history.set_status(&text);
    assert!(super::tests::dispatch_status_layout(&native) > 0);
    for dpi in [96, 120, 144, 168, 192] {
        let mut widths = Vec::new();
        for width in [800, 1000, 1400, 800] {
            resize(&mut native, width, 680, dpi);
            for advanced in [false, true] {
                if native.state.history.advanced != advanced {
                    native.command(ADVANCED, None);
                }
                let height = layout_matches_text(&native, advanced);
                if !advanced {
                    widths.push(height);
                }
                let mut host = RECT::default();
                // SAFETY: Query only real scroll geometry and owned controls.
                unsafe {
                    GetClientRect(native.state.viewport.handle, &mut host);
                    assert_eq!(
                        GetWindowLongPtrW(status, GWL_STYLE) as u32 & SS_NOPREFIX,
                        SS_NOPREFIX
                    );
                }
                native.scroll(SB_BOTTOM);
                let tail = native.rect(native.control(TABLE));
                assert_eq!(
                    host.bottom - tail.bottom,
                    scale(viewport::BOTTOM_PADDING, dpi)
                );
                assert!(tail.bottom - tail.top >= scale(80, dpi));
                assert_eq!(settings::text(status), text);
                assert_eq!(settings::text(draft), "127.0.0.2");
                assert_eq!(
                    unsafe { SendMessageW(draft, 0x00b0, 0, 0) } as u32,
                    (5 << 16) | 1
                );
                let offset = native.scroll(SB_ENDSCROLL);
                let before = native.rect(native.control(TABLE)).top;
                native.state.show_page(0);
                native.state.show_page(1);
                assert_eq!(native.scroll(SB_ENDSCROLL), offset);
                assert_eq!(native.rect(native.control(TABLE)).top, before);
            }
        }
        assert!(
            widths[0] > widths[2],
            "narrow status must really wrap more at DPI {dpi}"
        );
        assert_eq!(
            widths[0], widths[3],
            "returning width changed status measurement"
        );
    }
}

#[test]
fn native_status_change_reflows_after_borrow_and_preserves_hidden_pages_and_scroll() {
    let mut native = NativeState::new();
    native.state.show_page(1);
    resize(&mut native, 800, 680, 96);
    native.scroll(SB_LINEDOWN);
    let offset = native.scroll(SB_ENDSCROLL);
    let before = native.rect(native.control(TABLE)).top;
    native.state.history.set_status(&long_status());
    assert_eq!(
        native.rect(native.control(TABLE)).top,
        before,
        "setter must not rearrange under History borrow"
    );
    assert!(super::tests::dispatch_status_layout(&native) > 0);
    assert!(native.rect(native.control(TABLE)).top > before);
    assert_eq!(native.scroll(SB_ENDSCROLL), offset);
    layout_matches_text(&native, false);
    for view in [0, 1, 2, 3] {
        native.command(TABLE_VIEW, Some(view));
        layout_matches_text(&native, false);
    }
    native.state.show_page(0);
    let overview_before = native.rect(native.control(2109));
    native.state.history.set_status(&format!(
        "历史不可用／操作失败：{} & details",
        "较长的中文错误说明；".repeat(12)
    ));
    assert!(super::tests::dispatch_status_layout(&native) > 0);
    assert!(!native.state.page_geometry[1]);
    let overview_after = native.rect(native.control(2109));
    assert_eq!(
        (overview_before.top, overview_before.bottom),
        (overview_after.top, overview_after.bottom)
    );
    native.state.show_page(1);
    layout_matches_text(&native, false);
    native.state.history.set_status("已完成");
    super::tests::dispatch_status_layout(&native);
    assert_eq!(layout_matches_text(&native, false), 34);
    assert_eq!(
        native.rect(native.control(CHART)).top,
        312 - HEADER_HEIGHT - native.scroll(SB_ENDSCROLL)
    );
    assert_eq!(
        native.rect(native.control(2331)).top,
        592 - HEADER_HEIGHT - native.scroll(SB_ENDSCROLL)
    );
}

#[test]
fn native_management_status_and_session_keep_independent_tails_and_gdi_resources() {
    use windows_sys::Win32::System::Threading::{
        GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
    };
    let mut native = NativeState::new();
    native.state.show_page(1);
    resize(&mut native, 800, 680, 96);
    native.state.history.management = true;
    native.state.history.show(true);
    native.state.layout(native.hwnd, true);
    native.state.history.set_status(&long_status());
    super::tests::dispatch_status_layout(&native);
    native.scroll(SB_BOTTOM);
    let status = native.rect(native.control(2325));
    let storage = native.rect(native.control(STORAGE_TEXT));
    let mut host = RECT::default();
    // SAFETY: Local status/host and process GDI count, no desktop input.
    unsafe {
        GetClientRect(native.state.viewport.handle, &mut host);
    }
    assert!(
        status.bottom - status.top >= measured(native.control(2325), status.right - status.left)
    );
    assert_eq!(status.top - storage.bottom, 8);
    assert_eq!(host.bottom - status.bottom, viewport::BOTTOM_PADDING);
    native.command(VIEW, Some(1));
    assert_eq!(native.state.history.page_height(96, 680, 784), 820);
    native.scroll(SB_BOTTOM);
    assert_eq!(
        host.bottom - native.rect(native.control(2020)).bottom,
        viewport::BOTTOM_PADDING
    );
    let status_handle = native.control(2325);
    status_layout::height(status_handle, 96, 696);
    let baseline = unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
    for _ in 0..100 {
        assert!(status_layout::height(status_handle, 96, 696) >= 34);
    }
    assert_eq!(
        unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) },
        baseline
    );
}
