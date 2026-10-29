use super::*;
use crate::ui::tests::NativeState;
use windows_sys::Win32::{
    Foundation::SIZE,
    System::Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources},
};

struct Bitmap {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *const u32,
}
impl Bitmap {
    const WIDTH: i32 = 1800;
    const HEIGHT: i32 = 128;

    fn new() -> Self {
        // SAFETY: Private top-down 32-bit DIB and memory DC. Initialize the owner
        // before later assertions so partial allocation is also cleaned up.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            assert!(!dc.is_null());
            let mut target = Self {
                dc,
                bitmap: null_mut(),
                previous: null_mut(),
                bits: null(),
            };
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: Self::WIDTH,
                    biHeight: -Self::HEIGHT,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = null_mut();
            target.bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            assert!(!target.bitmap.is_null() && !bits.is_null());
            target.bits = bits.cast();
            target.previous = SelectObject(dc, target.bitmap);
            assert!(!target.previous.is_null() && target.previous as isize != -1);
            target
        }
    }

    fn pixels(&self, rect: RECT) -> Vec<u32> {
        assert!(rect.left >= 0 && rect.top >= 0);
        assert!(rect.right <= Self::WIDTH && rect.bottom <= Self::HEIGHT);
        assert!(rect.right >= rect.left && rect.bottom >= rect.top);
        // SAFETY: Flush GDI before reading the owned DIB; every index is checked
        // against its fixed allocation. Convert DIB RGB words to COLORREF.
        unsafe {
            GdiFlush();
            let bits = std::slice::from_raw_parts(self.bits, (Self::WIDTH * Self::HEIGHT) as usize);
            (rect.top..rect.bottom)
                .flat_map(|y| {
                    (rect.left..rect.right).map(move |x| {
                        let color = bits[(y * Self::WIDTH + x) as usize];
                        ((color >> 16) & 255) | (color & 0xff00) | ((color & 255) << 16)
                    })
                })
                .collect()
        }
    }

    fn text_reference(
        &self,
        text: &str,
        font: HFONT,
        rect: RECT,
        palette: theme::Palette,
        ellipsis: bool,
    ) {
        // SAFETY: The fixture retains font throughout this call. Only this private
        // DC is changed; its selections/attributes are restored before return.
        unsafe {
            let _saved = SavedDc::new(self.dc);
            let brush = GetStockObject(DC_BRUSH);
            assert!(!brush.is_null());
            SetDCBrushColor(self.dc, palette.background);
            FillRect(self.dc, &rect, brush);
            SelectObject(self.dc, font);
            SetBkMode(self.dc, TRANSPARENT as i32);
            SetTextColor(self.dc, palette.text);
            let mut rect = rect;
            DrawTextW(
                self.dc,
                wide(text).as_ptr(),
                -1,
                &mut rect,
                DT_SINGLELINE
                    | DT_VCENTER
                    | DT_NOPREFIX
                    | if ellipsis { DT_END_ELLIPSIS } else { 0 },
            );
        }
    }
}
impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: No DC guard outlives this owner. Deselect the owned DIB before
        // deleting it; if selection failed, it was never selected into this DC.
        unsafe {
            if !self.previous.is_null() && self.previous as isize != -1 {
                SelectObject(self.dc, self.previous);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            DeleteDC(self.dc);
        }
    }
}

struct SavedDc(HDC, i32);
impl SavedDc {
    unsafe fn new(dc: HDC) -> Self {
        // SAFETY: Caller keeps the test-owned DC alive until this guard drops.
        let level = unsafe { SaveDC(dc) };
        assert_ne!(level, 0);
        Self(dc, level)
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: Constructed only after successful SaveDC on a still-live DC.
        unsafe { RestoreDC(self.0, self.1) };
    }
}

struct Region(HRGN);
impl Region {
    fn new(rect: RECT) -> Self {
        // SAFETY: Scalar test coordinates; the returned HRGN is privately owned.
        let handle = unsafe { CreateRectRgn(rect.left, rect.top, rect.right, rect.bottom) };
        assert!(!handle.is_null());
        Self(handle)
    }
}
impl Drop for Region {
    fn drop(&mut self) {
        // SAFETY: Selecting a clip copies it, so this test always owns the HRGN.
        unsafe { DeleteObject(self.0) };
    }
}

#[derive(Debug, PartialEq, Eq)]
struct DcState {
    colors: [u32; 4],
    objects: [HGDIOBJ; 4],
    modes: [i32; 5],
    align: u32,
    coordinates: [(i32, i32); 5],
    clip: (i32, i32, i32, i32, i32),
}
impl DcState {
    unsafe fn capture(dc: HDC) -> Self {
        // SAFETY: Test-owned live DC and writable stack outputs only.
        unsafe {
            let mut window = POINT::default();
            let mut viewport = POINT::default();
            let mut position = POINT::default();
            let mut window_extent = SIZE::default();
            let mut viewport_extent = SIZE::default();
            let mut clip = RECT::default();
            GetWindowOrgEx(dc, &mut window);
            GetViewportOrgEx(dc, &mut viewport);
            GetCurrentPositionEx(dc, &mut position);
            GetWindowExtEx(dc, &mut window_extent);
            GetViewportExtEx(dc, &mut viewport_extent);
            let clip_kind = GetClipBox(dc, &mut clip);
            Self {
                colors: [
                    GetTextColor(dc),
                    GetBkColor(dc),
                    GetDCBrushColor(dc),
                    GetDCPenColor(dc),
                ],
                objects: [OBJ_FONT, OBJ_PEN, OBJ_BRUSH, OBJ_BITMAP]
                    .map(|kind| GetCurrentObject(dc, kind as u32)),
                modes: [
                    GetBkMode(dc),
                    GetMapMode(dc),
                    GetROP2(dc),
                    GetPolyFillMode(dc),
                    GetStretchBltMode(dc),
                ],
                align: GetTextAlign(dc),
                coordinates: [
                    (window.x, window.y),
                    (viewport.x, viewport.y),
                    (window_extent.cx, window_extent.cy),
                    (viewport_extent.cx, viewport_extent.cy),
                    (position.x, position.y),
                ],
                clip: (clip_kind, clip.left, clip.top, clip.right, clip.bottom),
            }
        }
    }
}

fn palette(kind: theme::Resolved) -> theme::Palette {
    match kind {
        theme::Resolved::Light => theme::Palette {
            background: 0xf3f3f3,
            surface: 0xffffff,
            text: 0x1a1a1a,
            disabled: 0x616161,
            accent: 0xb85f00,
            border: 0xe0e0e0,
            accent_surface: 0xf9f2ea,
        },
        theme::Resolved::Dark => theme::Palette {
            background: 0x202020,
            surface: 0x2b2b2b,
            text: 0xf5f5f5,
            disabled: 0xc5c5c5,
            accent: 0xffcd60,
            border: 0x454545,
            accent_surface: 0x473d2d,
        },
        // SAFETY: Public color indices; no system theme/contrast setting changes.
        theme::Resolved::Contrast => unsafe {
            theme::Palette {
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

fn resize(hwnd: HWND, width: i32, height: i32) {
    // SAFETY: Only the hidden fixture's STATIC is moved, without redraw or a pump.
    unsafe {
        assert_ne!(
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW
            ),
            0
        );
        let mut rect = RECT::default();
        assert_ne!(GetClientRect(hwnd, &mut rect), 0);
        assert_eq!(
            (rect.left, rect.top, rect.right, rect.bottom),
            (0, 0, width, height)
        );
    }
}

fn native_text(hwnd: HWND) -> String {
    // SAFETY: Owner-thread fixture STATIC, bounded synthetic text; WM_GETTEXT
    // copies into our writable buffer and must retain the complete native value.
    unsafe {
        let length = usize::try_from(SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0)).unwrap();
        assert!(length < 16_384);
        let mut text = vec![0; length + 1];
        let count = SendMessageW(hwnd, WM_GETTEXT, text.len(), text.as_mut_ptr() as LPARAM);
        assert_eq!(count as usize, length);
        String::from_utf16(&text[..length]).unwrap()
    }
}

fn paint(native: &mut NativeState, target: &Bitmap) {
    let hwnd = native.state.status_label;
    let text = native_text(hwnd);
    let before_clip = Region::new(RECT::default());
    let after_clip = Region::new(RECT::default());
    // SAFETY: The attached hidden STATIC, boxed State/fonts and private memory DC
    // stay alive. Call actual draw directly, without pumping or showing any GUI.
    unsafe {
        let before = DcState::capture(target.dc);
        let has_clip = GetClipRgn(target.dc, before_clip.0);
        assert_ne!(has_clip, -1);
        draw(hwnd, target.dc, &mut *native.state);
        assert_eq!(DcState::capture(target.dc), before);
        assert_eq!(GetClipRgn(target.dc, after_clip.0), has_clip);
        if has_clip == 1 {
            assert_ne!(EqualRgn(before_clip.0, after_clip.0), 0);
        }
    }
    assert_eq!(native_text(hwnd), text);
}

fn near(a: u32, b: u32) -> bool {
    [0, 8, 16]
        .into_iter()
        .all(|shift| (((a >> shift) & 255) as i32 - ((b >> shift) & 255) as i32).abs() <= 24)
}

#[test]
fn actual_draw_keeps_24_dip_icon_neutral_text_ellipsis_and_full_native_value() {
    let mut native = NativeState::new();
    let target = Bitmap::new();
    let reference = Bitmap::new();
    // SAFETY: NativeState owns the hidden child/boxed State until after all DCs
    // drop; attach installs the production WM_SETTEXT/tooltip/native text path.
    unsafe { attach(&mut *native.state).unwrap() };
    let hwnd = native.state.status_label;
    for dpi in [96, 144, 192] {
        native.state.update_dpi(native.hwnd, dpi).unwrap();
        let px = |n| scale(n, dpi);
        let width = px(360);
        let height = px(HEIGHT);
        resize(hwnd, width, height);
        for kind in [
            theme::Resolved::Light,
            theme::Resolved::Dark,
            theme::Resolved::Contrast,
        ] {
            // draw reads only the copied palette, not Theme's retained brushes.
            native.state.theme.kind = kind;
            native.state.theme.palette = palette(kind);
            let p = native.state.theme.palette;
            let band = RECT {
                left: 0,
                top: px(2),
                right: width,
                bottom: height - px(2),
            };
            let icon_rect = RECT {
                left: px(24),
                top: px(12),
                right: px(40),
                bottom: px(28),
            };
            let text_rect = RECT {
                left: px(48),
                top: 0,
                right: width - px(24),
                bottom: height,
            };
            let mut icons = Vec::new();
            for (label, expected_kind) in [
                ("错误", StatusKind::Error),
                ("启动中", StatusKind::Busy),
                ("运行中", StatusKind::Running),
                ("已停止", StatusKind::Stopped),
            ] {
                let text = format!("MMMMMMMM {label}");
                assert_eq!(status_kind(&text), expected_kind);
                settings::set_text(hwnd, &text);
                paint(&mut native, &target);
                assert_eq!(native_text(hwnd), text);
                assert!(
                    target
                        .pixels(RECT {
                            right: px(24),
                            ..band
                        })
                        .iter()
                        .all(|color| *color == p.background)
                );
                let icon_band = RECT {
                    left: px(24),
                    right: px(40),
                    ..band
                };
                let ink = target.pixels(icon_band);
                assert!(
                    ink.chunks_exact(px(16) as usize)
                        .any(|row| row[0] != p.background),
                    "icon must start at 24 DIP, not just somewhere in its box"
                );
                let icon = target.pixels(icon_rect);
                let colors = colors(expected_kind, kind, p);
                assert!(icon.iter().any(|color| near(*color, colors.icon)));
                assert!(icon.contains(&colors.badge));
                if kind == theme::Resolved::Contrast {
                    assert!(
                        icon.iter()
                            .all(|color| *color == p.text || *color == p.background)
                    );
                    assert!(
                        !icons.contains(&icon),
                        "system-color symbols must distinguish statuses"
                    );
                    icons.push(icon);
                }
                // A region-level ink check tolerates font/AA raster differences,
                // while semantic-colored text cannot meet this neutral core color.
                assert!(
                    target
                        .pixels(RECT {
                            left: text_rect.left,
                            right: text_rect.right,
                            ..band
                        })
                        .iter()
                        .any(|color| near(*color, p.text))
                );
                assert!(
                    target
                        .pixels(RECT {
                            left: text_rect.right,
                            ..band
                        })
                        .iter()
                        .all(|color| *color == p.background)
                );
            }
            let long = format!("错误：{}", "W".repeat(512));
            settings::set_text(hwnd, &long);
            paint(&mut native, &target);
            let tail = RECT {
                left: text_rect.right - px(40),
                right: text_rect.right,
                top: px(4),
                bottom: height - px(4),
            };
            let actual = target.pixels(tail);
            reference.text_reference(&long, native.state.font.0, text_rect, p, true);
            // Compare with same-run GDI layout, never a fixed character raster.
            assert_eq!(actual, reference.pixels(tail));
            assert!(actual.iter().any(|color| near(*color, p.text)));
            reference.text_reference(&long, native.state.font.0, text_rect, p, false);
            assert_ne!(
                actual,
                reference.pixels(tail),
                "ellipsis must differ from plain clipped text"
            );
            let multiline = format!(
                "{long}\r\n{}\r\n完整尾部 & Unicode 保留",
                "详细说明 ".repeat(256)
            );
            settings::set_text(hwnd, &multiline);
            paint(&mut native, &target);
            assert_eq!(native_text(hwnd), multiline);
            resize(hwnd, px(72), height);
            paint(&mut native, &target);
            assert!(
                target
                    .pixels(RECT {
                        left: px(48),
                        right: px(72),
                        ..band
                    })
                    .iter()
                    .all(|color| *color == p.background),
                "no room for text must take the early-return path"
            );
            resize(hwnd, width, height);
        }
    }
}

#[test]
fn normal_and_tiny_draw_restore_complex_dc_and_keep_repeated_gdi_usage_stable() {
    let mut native = NativeState::new();
    let target = Bitmap::new();
    // SAFETY: The hidden fixture and its stable State outlive the status subclass.
    unsafe { attach(&mut *native.state).unwrap() };
    let hwnd = native.state.status_label;
    let text = format!(
        "代理错误：{}\r\n{}\r\n完整尾部",
        "W".repeat(256),
        "详细说明 ".repeat(256)
    );
    settings::set_text(hwnd, &text);
    for dpi in [96, 144, 192] {
        native.state.update_dpi(native.hwnd, dpi).unwrap();
        let px = |n| scale(n, dpi);
        for kind in [
            theme::Resolved::Light,
            theme::Resolved::Dark,
            theme::Resolved::Contrast,
        ] {
            native.state.theme.kind = kind;
            native.state.theme.palette = palette(kind);
            // SAFETY: All mappings, stock selections and complex clips are private
            // to this memory DC. The guard restores them even if an assertion fails.
            unsafe {
                let _saved = SavedDc::new(target.dc);
                SelectObject(target.dc, GetStockObject(ANSI_FIXED_FONT));
                SelectObject(target.dc, GetStockObject(WHITE_PEN));
                SelectObject(target.dc, GetStockObject(WHITE_BRUSH));
                SetTextColor(target.dc, 0x123456);
                SetBkColor(target.dc, 0x654321);
                SetBkMode(target.dc, OPAQUE as i32);
                SetDCBrushColor(target.dc, 0x234567);
                SetDCPenColor(target.dc, 0x345678);
                SetTextAlign(target.dc, TA_RIGHT | TA_BASELINE);
                SetROP2(target.dc, R2_XORPEN);
                SetPolyFillMode(target.dc, WINDING);
                SetMapMode(target.dc, MM_ANISOTROPIC);
                SetWindowExtEx(target.dc, 5, 7, null_mut());
                SetViewportExtEx(target.dc, 10, 14, null_mut());
                SetWindowOrgEx(target.dc, 3, 4, null_mut());
                SetViewportOrgEx(target.dc, 11, 13, null_mut());
                MoveToEx(target.dc, 17, 19, null_mut());
                let clip = Region::new(RECT {
                    left: 5,
                    top: 7,
                    right: 1500,
                    bottom: 110,
                });
                let hole = Region::new(RECT {
                    left: 40,
                    top: 20,
                    right: 60,
                    bottom: 35,
                });
                assert_eq!(CombineRgn(clip.0, clip.0, hole.0, RGN_DIFF), COMPLEXREGION);
                assert_ne!(SelectClipRgn(target.dc, clip.0), ERROR);
                let before = DcState::capture(target.dc);
                for (width, height) in [
                    (0, 0),
                    (1, 1),
                    (48, HEIGHT),
                    (72, HEIGHT),
                    (73, HEIGHT),
                    (360, HEIGHT),
                ] {
                    resize(hwnd, px(width), px(height));
                    paint(&mut native, &target);
                    assert_eq!(DcState::capture(target.dc), before);
                }
                // Warm the two actual paths before process-wide GDI accounting;
                // this regression is intended for the main agent's serial run.
                for _ in 0..4 {
                    for width in [72, 360] {
                        resize(hwnd, px(width), px(HEIGHT));
                        paint(&mut native, &target);
                    }
                }
                let process = GetCurrentProcess();
                let count = GetGuiResources(process, GR_GDIOBJECTS);
                for _ in 0..30 {
                    for width in [72, 360] {
                        resize(hwnd, px(width), px(HEIGHT));
                        paint(&mut native, &target);
                    }
                }
                assert_eq!(DcState::capture(target.dc), before);
                assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), count);
            }
            assert_eq!(native_text(hwnd), text);
        }
    }
}
