use super::*;
use crate::{config::ThemeMode, history::TrendPoint, ui::theme::Theme, windows::Font};
use windows_sys::Win32::{
    Foundation::POINT,
    Graphics::Gdi::*,
    System::Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources},
};

fn palette(kind: Resolved) -> Palette {
    match kind {
        Resolved::Light => Palette {
            background: 0xf3f3f3,
            surface: 0xffffff,
            text: 0x1a1a1a,
            disabled: 0x616161,
            accent: 0xb85f00,
            border: 0xe0e0e0,
            accent_surface: 0xf9f2ea,
        },
        Resolved::Dark => Palette {
            background: 0x202020,
            surface: 0x2b2b2b,
            text: 0xf5f5f5,
            disabled: 0xc5c5c5,
            accent: 0xffcd60,
            border: 0x454545,
            accent_surface: 0x473d2d,
        },
        // SAFETY: Public system color indices require no window or settings changes.
        Resolved::Contrast => unsafe {
            Palette {
                background: GetSysColor(COLOR_WINDOW),
                surface: GetSysColor(COLOR_WINDOW),
                text: GetSysColor(COLOR_WINDOWTEXT),
                disabled: GetSysColor(COLOR_GRAYTEXT),
                accent: GetSysColor(COLOR_HIGHLIGHT),
                border: GetSysColor(COLOR_WINDOWTEXT),
                accent_surface: GetSysColor(COLOR_HIGHLIGHT),
            }
        },
    }
}

fn contrast(a: u32, b: u32) -> f64 {
    let luminance = |color: u32| {
        [0.2126, 0.7152, 0.0722]
            .into_iter()
            .enumerate()
            .map(|(i, weight)| {
                let channel = f64::from((color >> (i * 8)) & 255) / 255.0;
                weight
                    * if channel <= 0.04045 {
                        channel / 12.92
                    } else {
                        ((channel + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum::<f64>()
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

struct Target {
    dc: HDC,
    bitmap: HGDIOBJ,
    previous: HGDIOBJ,
    bits: *const u32,
}
impl Target {
    fn new() -> Self {
        // SAFETY: Test-owned top-down 32-bit DIB; bits has exactly 1600 * 900
        // pixels and stays valid until bitmap deletion. Nothing is shown onscreen.
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            assert!(!dc.is_null());
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: 1600,
                    biHeight: -900,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            assert!(!bitmap.is_null() && !bits.is_null());
            let previous = SelectObject(dc, bitmap);
            assert!(!previous.is_null());
            Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
            }
        }
    }

    fn pixels(&self) -> Vec<u32> {
        // SAFETY: Flush drawing before reading the live, fixed-size owned DIB.
        unsafe {
            GdiFlush();
            std::slice::from_raw_parts(self.bits, 1600 * 900).to_vec()
        }
    }
}
impl Drop for Target {
    fn drop(&mut self) {
        // SAFETY: Restore the original bitmap before deleting our DIB and DC.
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct DcState {
    colors: [u32; 4],
    objects: [HGDIOBJ; 4],
    modes: [i32; 4],
    text_align: u32,
    viewport: (i32, i32),
    window: (i32, i32),
    position: (i32, i32),
    clip: (i32, i32, i32, i32, i32),
}
impl DcState {
    unsafe fn capture(dc: HDC) -> Self {
        // SAFETY: The test-owned memory DC remains live throughout capture.
        unsafe {
            let mut viewport = POINT::default();
            let mut window = POINT::default();
            let mut position = POINT::default();
            let mut clip = RECT::default();
            GetViewportOrgEx(dc, &mut viewport);
            GetWindowOrgEx(dc, &mut window);
            GetCurrentPositionEx(dc, &mut position);
            let clip_kind = GetClipBox(dc, &mut clip);
            Self {
                colors: [
                    GetTextColor(dc),
                    GetBkColor(dc),
                    GetDCBrushColor(dc),
                    GetDCPenColor(dc),
                ],
                objects: [OBJ_FONT, OBJ_BRUSH, OBJ_PEN, OBJ_BITMAP]
                    .map(|kind| GetCurrentObject(dc, kind as u32)),
                modes: [
                    GetBkMode(dc),
                    GetMapMode(dc),
                    GetROP2(dc),
                    GetPolyFillMode(dc),
                ],
                text_align: GetTextAlign(dc),
                viewport: (viewport.x, viewport.y),
                window: (window.x, window.y),
                position: (position.x, position.y),
                clip: (clip_kind, clip.left, clip.top, clip.right, clip.bottom),
            }
        }
    }
}

fn report() -> QueryReport {
    let mut summary = Summary {
        requests: 4,
        complete: 2,
        partial: 1,
        unknown: 1,
        ..Default::default()
    };
    summary.tokens.total_tokens = Some(36);
    summary.tokens.input_tokens = Some(24);
    summary.tokens.output_tokens = Some(12);
    summary.coverage[..3].fill(3);
    QueryReport {
        summary,
        groups: vec![],
        details: vec![],
        trend: [Some(24), Some(12), Some(0), None]
            .into_iter()
            .enumerate()
            .map(|(i, value)| {
                let mut summary = Summary {
                    requests: 2,
                    ..Default::default()
                };
                summary.tokens.total_tokens = value;
                summary.coverage[2] = if i == 1 {
                    1
                } else if value.is_some() {
                    2
                } else {
                    0
                };
                TrendPoint {
                    utc_day: 20744,
                    bucket_start_ms: i as i64 * 3_600_000,
                    label: format!("2026-10-18 {i:02}:00 +00:00"),
                    summary,
                }
            })
            .collect(),
        trend_truncated: false,
        aliases: vec![],
    }
}

#[test]
fn summary_cards_keep_neutral_layers_padding_and_readable_text_at_common_dpi() {
    let target = Target::new();
    // SAFETY: Private memory drawing only; guards restore selected fonts before drop.
    unsafe {
        for dpi in [96, 144, 192] {
            let font = Font::new(dpi).unwrap();
            let heading = Font::heading(dpi).unwrap();
            for kind in [Resolved::Light, Resolved::Dark, Resolved::Contrast] {
                let p = palette(kind);
                let level = SaveDC(target.dc);
                assert_ne!(level, 0);
                let _saved = SavedDc {
                    dc: target.dc,
                    level,
                };
                SetBkMode(target.dc, TRANSPARENT as i32);
                let px = |n| scale(n, dpi);
                let bounds = RECT {
                    left: px(11),
                    top: px(13),
                    right: px(707),
                    bottom: px(213),
                };
                let canvas = Canvas {
                    dc: target.dc,
                    font: font.0,
                    heading: heading.0,
                    palette: p,
                    contrast: kind == Resolved::Contrast,
                    bounds,
                    dpi,
                };
                if kind != Resolved::Contrast {
                    assert_ne!(p.surface, p.background);
                    assert_ne!(canvas.edge(), p.surface);
                    assert!(contrast(p.text, p.surface) >= 7.0);
                    assert!(contrast(canvas.secondary(), p.surface) >= 4.5);
                    assert!(contrast(canvas.secondary(), p.background) >= 4.5);
                    assert_eq!(canvas.edge() & 255, (canvas.edge() >> 8) & 255);
                    assert_eq!(canvas.edge() & 255, (canvas.edge() >> 16) & 255);
                } else {
                    assert_eq!(canvas.secondary(), p.text);
                    assert_eq!(canvas.edge(), p.border);
                }
                let width = bounds.right - bounds.left - px(16);
                for (report, pending) in [(None, false), (None, true), (Some(&report()), false)] {
                    canvas.fill(bounds, p.background, false);
                    canvas.summaries(report, pending, width);
                    for i in 0..4 {
                        let left = bounds.left + px(8) + width * i / 4;
                        let right = bounds.left + px(8) + width * (i + 1) / 4 - px(6);
                        assert_eq!(
                            GetPixel(target.dc, (left + right) / 2, bounds.top + px(2)),
                            canvas.edge()
                        );
                        assert_eq!(
                            GetPixel(target.dc, left + px(4), bounds.top + px(36)),
                            p.surface
                        );
                        assert_eq!(GetPixel(target.dc, left, bounds.top + px(2)), p.background);
                        assert_eq!(
                            GetPixel(target.dc, right + px(2), bounds.top + px(36)),
                            p.background
                        );
                        let content_width = right - left - px(18);
                        let title = [
                            "请求",
                            "已报告输入 Token",
                            "已报告输出 Token",
                            "已报告总 Token",
                        ][i as usize];
                        assert!(canvas.text_width(title) <= content_width);
                    }
                }
                let before = DcState::capture(target.dc);
                for (width, height) in [(0, 1), (1, 0), (-4, 4), (4, -4)] {
                    canvas.panel(
                        canvas.rect(0, 0, width, height),
                        p.surface,
                        p.border,
                        px(16),
                    );
                    assert_eq!(DcState::capture(target.dc), before);
                }
                for (width, height) in [(1, 1), (1, 3), (3, 1)] {
                    canvas.fill(bounds, p.background, false);
                    let tiny = canvas.rect(px(10), px(10), width, height);
                    canvas.panel(tiny, p.surface, p.border, px(16));
                    assert_eq!(GetPixel(target.dc, tiny.left - 1, tiny.top), p.background);
                    assert_eq!(GetPixel(target.dc, tiny.right, tiny.top), p.background);
                    assert_eq!(GetPixel(target.dc, tiny.left, tiny.bottom), p.background);
                }
            }
        }
    }
}

#[test]
fn trend_distinguishes_unknown_zero_and_partial_coverage_in_system_colors() {
    let target = Target::new();
    let report = report();
    assert_eq!(bucket_value(&report.trend[0].summary), "24");
    assert_eq!(bucket_value(&report.trend[1].summary), "12*");
    assert_eq!(bucket_value(&report.trend[2].summary), "0");
    assert_eq!(bucket_value(&report.trend[3].summary), "未知");
    // SAFETY: The DC and fonts belong to the test; no HWND or query is created.
    unsafe {
        for dpi in [96, 144, 192] {
            let font = Font::new(dpi).unwrap();
            let heading = Font::heading(dpi).unwrap();
            for kind in [Resolved::Light, Resolved::Dark, Resolved::Contrast] {
                let p = palette(kind);
                let level = SaveDC(target.dc);
                assert_ne!(level, 0);
                let _saved = SavedDc {
                    dc: target.dc,
                    level,
                };
                SetBkMode(target.dc, TRANSPARENT as i32);
                let px = |n| scale(n, dpi);
                let bounds = RECT {
                    left: px(11),
                    top: px(13),
                    right: px(707),
                    bottom: px(213),
                };
                let canvas = Canvas {
                    dc: target.dc,
                    font: font.0,
                    heading: heading.0,
                    palette: p,
                    contrast: kind == Resolved::Contrast,
                    bounds,
                    dpi,
                };
                canvas.fill(bounds, p.background, false);
                let x = px(8);
                let cell = px(112);
                canvas.trend(&report, x, cell * 4);
                let base = bounds.top + px(154);
                let left = bounds.left + x;
                let color = if kind == Resolved::Contrast {
                    p.text
                } else {
                    p.accent
                };
                assert_eq!(GetPixel(target.dc, left + cell / 2, base - px(2)), color);
                assert_eq!(
                    GetPixel(target.dc, left + cell + cell / 4, base - px(4)),
                    color
                );
                assert_eq!(
                    GetPixel(target.dc, left + cell + cell / 2, base - px(4)),
                    if kind == Resolved::Contrast {
                        p.background
                    } else {
                        color
                    }
                );
                assert_eq!(
                    GetPixel(target.dc, left + 2 * cell + cell / 4, base - 1),
                    p.text
                );
                assert_eq!(
                    GetPixel(target.dc, left + 3 * cell + cell / 4, base - 1),
                    p.background
                );
                assert_eq!(
                    GetPixel(target.dc, left + 3 * cell + cell / 4, base - px(24)),
                    p.background
                );
            }
        }
    }
}

#[test]
fn pending_never_paints_stale_report_and_draw_restores_dc_and_gdi() {
    let target = Target::new();
    let report = report();
    let empty = QueryReport {
        summary: Summary::default(),
        groups: vec![],
        details: vec![],
        trend: vec![],
        trend_truncated: false,
        aliases: vec![],
    };
    // SAFETY: All handles are test-owned. Theme brush handles are not used by this
    // renderer, so overriding its copied palette/kind exercises system colors too.
    unsafe {
        for dpi in [96, 144, 192] {
            let font = Font::new(dpi).unwrap();
            let heading = Font::heading(dpi).unwrap();
            for kind in [Resolved::Light, Resolved::Dark, Resolved::Contrast] {
                let mut theme = Theme::new(ThemeMode::Light).unwrap();
                theme.kind = kind;
                theme.palette = palette(kind);
                let item = DRAWITEMSTRUCT {
                    hDC: target.dc,
                    rcItem: RECT {
                        left: scale(11, dpi),
                        top: scale(13, dpi),
                        right: scale(707, dpi),
                        bottom: scale(213, dpi),
                    },
                    ..Default::default()
                };
                draw(&item, dpi, font.0, heading.0, &theme, None, true);
                let pending = target.pixels();
                draw(&item, dpi, font.0, heading.0, &theme, Some(&report), true);
                assert!(
                    pending == target.pixels(),
                    "pending must not expose stale report pixels"
                );
                let level = SaveDC(target.dc);
                assert_ne!(level, 0);
                let _saved = SavedDc {
                    dc: target.dc,
                    level,
                };
                SelectObject(target.dc, GetStockObject(ANSI_FIXED_FONT));
                SelectObject(target.dc, GetStockObject(WHITE_BRUSH));
                SelectObject(target.dc, GetStockObject(WHITE_PEN));
                SetTextColor(target.dc, 0x123456);
                SetBkColor(target.dc, 0x654321);
                SetBkMode(target.dc, OPAQUE as i32);
                SetDCBrushColor(target.dc, 0x234567);
                SetDCPenColor(target.dc, 0x345678);
                SetTextAlign(target.dc, TA_RIGHT | TA_BASELINE);
                SetROP2(target.dc, R2_XORPEN);
                SetPolyFillMode(target.dc, WINDING);
                SetWindowOrgEx(target.dc, 2, 3, std::ptr::null_mut());
                SetViewportOrgEx(target.dc, 5, 7, std::ptr::null_mut());
                MoveToEx(target.dc, 31, 37, std::ptr::null_mut());
                IntersectClipRect(target.dc, 1, 1, 1500, 800);
                let before = DcState::capture(target.dc);
                for (width, height) in [(696, 200), (1, 1), (8, 3), (0, 8), (8, 0)] {
                    let item = DRAWITEMSTRUCT {
                        hDC: target.dc,
                        rcItem: RECT {
                            left: 11,
                            top: 13,
                            right: 11 + scale(width, dpi),
                            bottom: 13 + scale(height, dpi),
                        },
                        ..Default::default()
                    };
                    for (report, pending) in [
                        (None, false),
                        (None, true),
                        (Some(&empty), false),
                        (Some(&report), false),
                        (Some(&report), true),
                    ] {
                        draw(&item, dpi, font.0, heading.0, &theme, report, pending);
                        assert_eq!(DcState::capture(target.dc), before);
                    }
                }
                for (font, heading) in [
                    (std::ptr::null_mut(), heading.0),
                    (font.0, std::ptr::null_mut()),
                ] {
                    draw(&item, dpi, font, heading, &theme, Some(&report), false);
                    assert_eq!(DcState::capture(target.dc), before);
                }
                let process = GetCurrentProcess();
                let count = GetGuiResources(process, GR_GDIOBJECTS);
                for _ in 0..30 {
                    draw(&item, dpi, font.0, heading.0, &theme, Some(&report), false);
                }
                assert_eq!(DcState::capture(target.dc), before);
                assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), count);
            }
        }
    }
}
