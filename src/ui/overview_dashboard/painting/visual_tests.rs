use super::*;
use crate::{app::controller::OverviewSnapshot, windows::Font};
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
        // SAFETY: Public system color indices require no HWND or theme changes.
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
}
impl Target {
    fn new() -> Self {
        // SAFETY: Private offscreen bitmap/DC; the previous bitmap is restored
        // before deletion. No window, desktop sampling, I/O or user data.
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let bitmap = CreateBitmap(1600, 900, 1, 32, std::ptr::null());
            assert!(!dc.is_null() && !bitmap.is_null());
            let previous = SelectObject(dc, bitmap);
            assert!(!previous.is_null());
            Self {
                dc,
                bitmap,
                previous,
            }
        }
    }
}
impl Drop for Target {
    fn drop(&mut self) {
        // SAFETY: Only this test owns the bitmap and memory DC.
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

#[test]
fn fluent_dashboard_colors_keep_neutral_cards_and_quieter_grids() {
    for kind in [Resolved::Light, Resolved::Dark] {
        let p = palette(kind);
        let colors = PaintColors::new(p, kind);
        assert!(contrast(p.text, p.surface) >= 7.0);
        assert!(contrast(colors.secondary, p.surface) >= 4.5);
        assert!(contrast(colors.secondary, colors.badge) >= 4.5);
        assert_ne!(p.background, p.surface);
        assert_ne!(colors.edge, p.surface);
        assert_ne!(colors.edge, colors.grid);
        assert_ne!(colors.badge, p.surface);
        for neutral in [colors.edge, colors.grid, colors.badge] {
            assert_eq!(neutral & 255, (neutral >> 8) & 255);
            assert_eq!(neutral & 255, (neutral >> 16) & 255);
        }
        for color in colors.series {
            assert!(contrast(color, p.surface) >= 3.0);
            assert!(contrast(color, p.surface) > contrast(colors.grid, p.surface));
        }
    }
    let p = palette(Resolved::Contrast);
    let colors = PaintColors::new(p, Resolved::Contrast);
    assert_eq!(colors.series, [p.text; 3]);
    assert_eq!(colors.secondary, p.text);
    assert_eq!(colors.edge, p.border);
    assert_eq!(colors.grid, p.border);
    assert_eq!(colors.badge, p.surface);
}

#[test]
fn cards_and_system_color_patterns_paint_without_color_only_distinctions() {
    let target = Target::new();
    // SAFETY: All drawing is confined to this private bitmap and saved DC.
    // The guard is dropped before the borrowed fonts are released.
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
                SelectObject(target.dc, GetStockObject(DC_PEN));
                SelectObject(target.dc, GetStockObject(DC_BRUSH));
                SetBkMode(target.dc, TRANSPARENT as i32);
                let canvas = Canvas {
                    dc: target.dc,
                    font: font.0,
                    heading: heading.0,
                    dpi,
                    palette: p,
                    kind,
                    colors: PaintColors::new(p, kind),
                };
                let px = |n| scale(n, dpi);
                let bounds = RECT {
                    left: px(11),
                    top: px(13),
                    right: px(11) + px(MIN_WIDTH_DIP),
                    bottom: px(13) + px(MIN_HEIGHT_DIP),
                };
                canvas.fill(bounds, p.background);
                canvas.cards(bounds, None);
                let gap = px(12);
                let cell = (bounds.right - bounds.left - gap * 3) / 4;
                for i in 0..4 {
                    let left = bounds.left + i * (cell + gap);
                    assert_eq!(
                        GetPixel(target.dc, left + cell / 2, bounds.top),
                        canvas.colors.edge
                    );
                    assert_eq!(
                        GetPixel(target.dc, left + px(8), bounds.top + px(106)),
                        p.surface
                    );
                    assert_eq!(
                        GetPixel(target.dc, left + px(15), bounds.top + px(15)),
                        canvas.colors.badge
                    );
                    assert_eq!(GetPixel(target.dc, left, bounds.top), p.background);
                }
                assert_eq!(
                    GetPixel(target.dc, bounds.left + cell + gap / 2, bounds.top + px(50)),
                    p.background
                );
                let left = bounds.left + px(16);
                let right = left + px(64);
                let y = bounds.top + px(150);
                canvas.fill(
                    RECT {
                        top: y - px(10),
                        bottom: y + px(10),
                        ..bounds
                    },
                    p.surface,
                );
                canvas.grid_line(left, right, y);
                assert_eq!(GetPixel(target.dc, left + px(1), y), canvas.colors.grid);
                assert_eq!(
                    GetPixel(target.dc, left + px(3), y),
                    if kind == Resolved::Contrast {
                        p.surface
                    } else {
                        canvas.colors.grid
                    }
                );
                if kind == Resolved::Contrast {
                    let y = y + px(24);
                    canvas.fill(
                        RECT {
                            top: y - px(10),
                            bottom: y + px(10),
                            ..bounds
                        },
                        p.surface,
                    );
                    canvas.series_line((left, y), (right, y), p.text, true);
                    assert_eq!(GetPixel(target.dc, left + px(4), y), p.text);
                    assert_eq!(GetPixel(target.dc, left + px(12), y), p.surface);
                    canvas.dot(left + px(96), y, p.text, false);
                    canvas.dot(left + px(120), y, p.text, true);
                    assert_eq!(GetPixel(target.dc, left + px(96), y), p.text);
                    assert_eq!(GetPixel(target.dc, left + px(120), y), p.surface);
                    assert_eq!(GetPixel(target.dc, left + px(118), y), p.text);
                }
                let before = DcState::capture(target.dc);
                for (width, height) in [(0, 1), (1, 0), (-4, 4), (4, -4)] {
                    canvas.panel(
                        canvas.rect(bounds, 0, 0, width, height),
                        p.surface,
                        p.border,
                        16,
                    );
                    assert_eq!(DcState::capture(target.dc), before);
                }
                for (width, height) in [(1, 1), (1, 3), (3, 1)] {
                    canvas.fill(bounds, p.background);
                    let tiny = canvas.rect(bounds, px(10), px(10), width, height);
                    canvas.panel(tiny, p.surface, p.border, 16);
                    assert_eq!(GetPixel(target.dc, tiny.left - 1, tiny.top), p.background);
                    assert_eq!(GetPixel(target.dc, tiny.right, tiny.top), p.background);
                    assert_eq!(GetPixel(target.dc, tiny.left, tiny.bottom), p.background);
                }
            }
        }
    }
}

#[test]
fn short_constant_contrast_series_retains_visible_square_samples() {
    let target = Target::new();
    let font = Font::new(96).unwrap();
    let heading = Font::heading(96).unwrap();
    let p = palette(Resolved::Contrast);
    let mut timeline = Timeline::default();
    for second in 0..=120 {
        timeline.record(&ProxyMetricsSnapshot {
            uptime: Duration::from_secs(second),
            total_requests: second * 5,
            errors: second * 3,
            ..Default::default()
        });
    }
    // SAFETY: Only a private memory bitmap is painted. At this width each
    // one-second segment is four pixels; eight-way native dashes may degenerate.
    unsafe {
        let level = SaveDC(target.dc);
        let _saved = SavedDc {
            dc: target.dc,
            level,
        };
        SelectObject(target.dc, GetStockObject(DC_PEN));
        SelectObject(target.dc, GetStockObject(DC_BRUSH));
        SetBkMode(target.dc, TRANSPARENT as i32);
        let canvas = Canvas {
            dc: target.dc,
            font: font.0,
            heading: heading.0,
            dpi: 96,
            palette: p,
            kind: Resolved::Contrast,
            colors: PaintColors::new(p, Resolved::Contrast),
        };
        let panel = RECT {
            left: 0,
            top: 0,
            right: 560,
            bottom: 300,
        };
        let layout = ChartLayout::new(panel, 96, [50; 2], 112, false);
        assert_eq!(layout.plot.right - layout.plot.left, 480);
        canvas.chart(panel, &timeline, false, true);
        let x = layout.plot.left + 240;
        let axis = RateScale::new(5.0, false);
        let y = layout.plot.bottom
            - ((layout.plot.bottom - layout.plot.top) as f64 * 3.0 / axis.maximum).round() as i32;
        assert_eq!(GetPixel(target.dc, x - 2, y), p.text);
        assert_eq!(GetPixel(target.dc, x, y), p.surface);
    }
}

unsafe fn pixels(target: &Target) -> Vec<u32> {
    // SAFETY: Temporarily deselect the test-owned bitmap for GetDIBits, then
    // restore it. The writable buffer exactly matches the requested 32-bit DIB.
    unsafe {
        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = 1600;
        info.bmiHeader.biHeight = -900;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut pixels = vec![0_u32; 1600 * 900];
        SelectObject(target.dc, target.previous);
        let rows = GetDIBits(
            target.dc,
            target.bitmap,
            0,
            900,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        );
        SelectObject(target.dc, target.bitmap);
        assert_eq!(rows, 900);
        for pixel in &mut pixels {
            *pixel &= 0x00ff_ffff;
        }
        pixels
    }
}

#[test]
fn buffered_frame_matches_direct_pixels_with_offsets_clipping_and_bounded_allocation() {
    let buffered = Target::new();
    let direct = Target::new();
    let mut timeline = Timeline::default();
    timeline.record(&ProxyMetricsSnapshot::default());
    let snapshot = OverviewSnapshot {
        metrics: ProxyMetricsSnapshot {
            uptime: Duration::from_secs(1),
            total_requests: 12,
            errors: 3,
            upload_body_bytes: 134_554,
            download_body_bytes: 6 * 1024 * 1024,
            ..Default::default()
        },
        running: true,
        listen: None,
        upstream: None,
    };
    timeline.record(&snapshot.metrics);
    // SAFETY: Isolated memory DCs, stock objects, and borrowed test fonts. No
    // window/desktop access; pixel equality includes outside-clip sentinels.
    unsafe {
        let count = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
        assert!(Frame::new(buffered.dc, 0, 1).is_none());
        assert!(Frame::new(buffered.dc, 1, -1).is_none());
        assert!(Frame::new(buffered.dc, i32::MAX, i32::MAX).is_none());
        assert_eq!(GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS), count);
        for dpi in [96, 192] {
            let font = Font::new(dpi).unwrap();
            let heading = Font::heading(dpi).unwrap();
            for kind in [Resolved::Light, Resolved::Dark, Resolved::Contrast] {
                for target in [&buffered, &direct] {
                    SelectClipRgn(target.dc, std::ptr::null_mut());
                    SetWindowOrgEx(target.dc, 0, 0, std::ptr::null_mut());
                    SetViewportOrgEx(target.dc, 0, 0, std::ptr::null_mut());
                    SetDCBrushColor(target.dc, 0xdc00dc);
                    FillRect(
                        target.dc,
                        &RECT {
                            left: 0,
                            top: 0,
                            right: 1600,
                            bottom: 900,
                        },
                        GetStockObject(DC_BRUSH),
                    );
                    SetWindowOrgEx(target.dc, 2, 3, std::ptr::null_mut());
                    SetViewportOrgEx(target.dc, 5, 7, std::ptr::null_mut());
                    IntersectClipRect(target.dc, 17, 19, 1300, 680);
                }
                let item = DRAWITEMSTRUCT {
                    hDC: buffered.dc,
                    rcItem: RECT {
                        left: 11,
                        top: 13,
                        right: 11 + scale(MIN_WIDTH_DIP, dpi),
                        bottom: 13 + scale(MIN_HEIGHT_DIP, dpi),
                    },
                    ..Default::default()
                };
                draw(
                    &item,
                    dpi,
                    font.0,
                    heading.0,
                    palette(kind),
                    kind,
                    Some(&snapshot),
                    &timeline,
                );
                assert!(paint(
                    &DRAWITEMSTRUCT {
                        hDC: direct.dc,
                        ..item
                    },
                    dpi,
                    font.0,
                    heading.0,
                    palette(kind),
                    kind,
                    Some(&snapshot),
                    &timeline
                ));
                GdiFlush();
                assert_eq!(
                    pixels(&buffered),
                    pixels(&direct),
                    "buffered offset/clip mismatch {kind:?} dpi={dpi}"
                );
            }
        }
    }
}

#[test]
fn full_dashboard_and_tiny_fallback_restore_dc_and_gdi_resources() {
    let target = Target::new();
    let metrics = ProxyMetricsSnapshot {
        uptime: Duration::from_secs(1),
        total_requests: 8,
        errors: 2,
        upload_body_bytes: 100,
        download_body_bytes: 200,
        ..Default::default()
    };
    let snapshot = OverviewSnapshot {
        metrics,
        running: true,
        listen: None,
        upstream: None,
    };
    let mut timeline = Timeline::default();
    timeline.record(&ProxyMetricsSnapshot::default());
    timeline.record(&metrics);
    // SAFETY: Offscreen borrowed handles only. No controls or application state
    // are created. Stock objects are never deleted; draw must restore every exit.
    unsafe {
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
        for dpi in [96, 144, 192] {
            let font = Font::new(dpi).unwrap();
            let heading = Font::heading(dpi).unwrap();
            for kind in [Resolved::Light, Resolved::Dark, Resolved::Contrast] {
                for (width, height) in [(696, 350), (695, 350), (696, 349), (1, 1), (0, 8), (8, 0)]
                {
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
                    for snapshot in [None, Some(&snapshot)] {
                        draw(
                            &item,
                            dpi,
                            font.0,
                            heading.0,
                            palette(kind),
                            kind,
                            snapshot,
                            &timeline,
                        );
                        assert_eq!(DcState::capture(target.dc), before);
                    }
                }
                let item = DRAWITEMSTRUCT {
                    hDC: target.dc,
                    rcItem: RECT {
                        left: 11,
                        top: 13,
                        right: 11 + scale(696, dpi),
                        bottom: 13 + scale(350, dpi),
                    },
                    ..Default::default()
                };
                let process = GetCurrentProcess();
                let count = GetGuiResources(process, GR_GDIOBJECTS);
                for _ in 0..30 {
                    draw(
                        &item,
                        dpi,
                        font.0,
                        heading.0,
                        palette(kind),
                        kind,
                        Some(&snapshot),
                        &timeline,
                    );
                }
                assert_eq!(DcState::capture(target.dc), before);
                assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), count);
            }
        }
    }
}
