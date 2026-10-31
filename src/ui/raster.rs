//! Bounded 4×4 CPU coverage for small, solid-color UI shapes, not text or whole
//! windows. Coordinates are local logical pixels; colors are Win32 COLORREFs.
//! Callers must retain their readable GDI painter when construction/paint fails.
#![cfg(windows)]

use std::mem::size_of;
use windows_sys::Win32::Graphics::Gdi::*;

const SAMPLES: usize = 16;
const OFFSETS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
const MAX_SIDE: i32 = 4096;
const MAX_PIXELS: usize = 65_536;
const MAX_COORD: f32 = 16_384.0;
const MAX_PRIMITIVES: usize = 64;
const MAX_VERTICES: usize = 32;
const MAX_EVALUATIONS: usize = 8 * 1024 * 1024;

/// Per-paint storage, at most 4 MiB of samples plus 256 KiB of output. No cache,
/// window, GDI object or borrowed DC is retained. Overlapping shapes are composed
/// at sample resolution, so an inset/border does not introduce a blended seam.
/// Invalid geometry or exhausted work budgets make `paint` return false.
pub(super) struct Raster {
    width: i32,
    height: i32,
    samples: Vec<[u32; SAMPLES]>,
    pixels: Vec<u32>,
    primitives: usize,
    evaluations: usize,
    valid: bool,
}

impl Raster {
    pub(super) fn new(width: i32, height: i32, background: u32) -> Option<Self> {
        if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
            return None;
        }
        let length = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?;
        if length > MAX_PIXELS {
            return None;
        }
        let mut samples = Vec::new();
        samples.try_reserve_exact(length).ok()?;
        samples.resize(length, [dib(background); SAMPLES]);
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(length).ok()?;
        pixels.resize(length, 0);
        Some(Self {
            width,
            height,
            samples,
            pixels,
            primitives: 0,
            evaluations: 0,
            valid: true,
        })
    }

    /// Filled rounded rectangle, with radius clamped to half its shorter side.
    /// A radius of zero gives a rectangle; zero-area shapes are harmless no-ops.
    pub(super) fn round_rect(&mut self, rect: [f32; 4], radius: f32, color: u32) {
        if !rect.into_iter().all(coordinate)
            || !length(radius)
            || rect[2] < rect[0]
            || rect[3] < rect[1]
        {
            self.valid = false;
            return;
        }
        let hx = (rect[2] - rect[0]) * 0.5;
        let hy = (rect[3] - rect[1]) * 0.5;
        let cx = (rect[0] + rect[2]) * 0.5;
        let cy = (rect[1] + rect[3]) * 0.5;
        let radius = radius.min(hx).min(hy);
        self.cover(rect, color, 1, |x, y| {
            let dx = ((x - cx).abs() - (hx - radius)).max(0.0);
            let dy = ((y - cy).abs() - (hy - radius)).max(0.0);
            dx * dx + dy * dy <= radius * radius
        });
    }

    pub(super) fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: u32) {
        if !coordinate(cx) || !coordinate(cy) || !length(radius) {
            self.valid = false;
            return;
        }
        self.cover(
            [cx - radius, cy - radius, cx + radius, cy + radius],
            color,
            1,
            |x, y| (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius,
        );
    }

    /// Round-capped segment; a zero-length segment is a circle of width/2.
    pub(super) fn line(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: u32) {
        if ![from.0, from.1, to.0, to.1].into_iter().all(coordinate) || !length(width) {
            self.valid = false;
            return;
        }
        if width == 0.0 {
            return;
        }
        let radius = width * 0.5;
        let dx = to.0 - from.0;
        let dy = to.1 - from.1;
        let squared = dx * dx + dy * dy;
        self.cover(
            [
                from.0.min(to.0) - radius,
                from.1.min(to.1) - radius,
                from.0.max(to.0) + radius,
                from.1.max(to.1) + radius,
            ],
            color,
            1,
            |x, y| {
                let t = if squared == 0.0 {
                    0.0
                } else {
                    (((x - from.0) * dx + (y - from.1) * dy) / squared).clamp(0.0, 1.0)
                };
                (x - from.0 - t * dx).powi(2) + (y - from.1 - t * dy).powi(2) <= radius * radius
            },
        );
    }

    /// Closed, even-odd filled polygon, at most 32 vertices. Points are consumed
    /// synchronously and never retained. No path or heap allocation is needed.
    pub(super) fn polygon(&mut self, points: &[(f32, f32)], color: u32) {
        if !(3..=MAX_VERTICES).contains(&points.len())
            || !points.iter().all(|&(x, y)| coordinate(x) && coordinate(y))
        {
            self.valid = false;
            return;
        }
        let mut rect = [points[0].0, points[0].1, points[0].0, points[0].1];
        for &(x, y) in points {
            rect[0] = rect[0].min(x);
            rect[1] = rect[1].min(y);
            rect[2] = rect[2].max(x);
            rect[3] = rect[3].max(y);
        }
        self.cover(rect, color, points.len(), |x, y| {
            let mut inside = false;
            let mut previous = points[points.len() - 1];
            for &next in points {
                if (next.1 > y) != (previous.1 > y)
                    && x < (previous.0 - next.0) * (y - next.1) / (previous.1 - next.1) + next.0
                {
                    inside = !inside;
                }
                previous = next;
            }
            inside
        });
    }

    fn cover(
        &mut self,
        rect: [f32; 4],
        color: u32,
        cost: usize,
        contains: impl Fn(f32, f32) -> bool,
    ) {
        if !self.valid {
            return;
        }
        self.primitives += 1;
        if self.primitives > MAX_PRIMITIVES {
            self.valid = false;
            return;
        }
        if rect[0] == rect[2] || rect[1] == rect[3] {
            return;
        }
        let left = rect[0].floor().clamp(0.0, self.width as f32) as usize;
        let top = rect[1].floor().clamp(0.0, self.height as f32) as usize;
        let right = rect[2].ceil().clamp(0.0, self.width as f32) as usize;
        let bottom = rect[3].ceil().clamp(0.0, self.height as f32) as usize;
        let work = (right - left)
            .checked_mul(bottom - top)
            .and_then(|n| n.checked_mul(SAMPLES))
            .and_then(|n| n.checked_mul(cost))
            .and_then(|n| n.checked_add(self.evaluations));
        let Some(work) = work.filter(|n| *n <= MAX_EVALUATIONS) else {
            self.valid = false;
            return;
        };
        self.evaluations = work;
        let color = dib(color);
        for y in top..bottom {
            for x in left..right {
                let samples = &mut self.samples[y * self.width as usize + x];
                for (sy, dy) in OFFSETS.into_iter().enumerate() {
                    for (sx, dx) in OFFSETS.into_iter().enumerate() {
                        if contains(x as f32 + dx, y as f32 + dy) {
                            samples[sy * 4 + sx] = color;
                        }
                    }
                }
            }
        }
    }

    fn resolve(&mut self) {
        for (pixel, samples) in self.pixels.iter_mut().zip(&self.samples) {
            *pixel = [0, 8, 16].into_iter().fold(0, |color, shift| {
                let sum: u32 = samples.iter().map(|value| (value >> shift) & 255).sum();
                color | (((sum + SAMPLES as u32 / 2) / SAMPLES as u32) << shift)
            });
        }
    }

    /// Present a top-down, opaque 32-bit DIB. The caller supplies the known solid
    /// background, so no destination readback or text re-rasterization is needed.
    /// Destination mapping/origin and complex clip are honored, never reset.
    ///
    /// # Safety
    /// `dc` is null or a live paint/borrowed print DC on the calling UI thread,
    /// and stays live until this call returns. False requests the GDI fallback.
    pub(super) unsafe fn paint(&mut self, dc: HDC, left: i32, top: i32) -> bool {
        if !self.valid
            || dc.is_null()
            || left.checked_add(self.width).is_none()
            || top.checked_add(self.height).is_none()
        {
            return false;
        }
        self.resolve();
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: self.width,
                biHeight: -self.height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: Checked, fallibly allocated buffer exactly matches this 32-bit
        // DIB. StretchDIBits borrows it synchronously and owns no GDI resources.
        // Save/restore encloses stretch mode only; caller's clip/mapping survives.
        unsafe {
            let saved = SaveDC(dc);
            if saved == 0 {
                return false;
            }
            let copied = if SetStretchBltMode(dc, COLORONCOLOR) == 0 {
                0
            } else {
                StretchDIBits(
                    dc,
                    left,
                    top,
                    self.width,
                    self.height,
                    0,
                    0,
                    self.width,
                    self.height,
                    self.pixels.as_ptr().cast(),
                    &info,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                )
            };
            let restored = RestoreDC(dc, saved) != 0;
            copied > 0 && restored
        }
    }
}

fn coordinate(value: f32) -> bool {
    value.is_finite() && value.abs() <= MAX_COORD
}

fn length(value: f32) -> bool {
    value.is_finite() && (0.0..=MAX_COORD).contains(&value)
}

fn dib(color: u32) -> u32 {
    ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixed(pixel: u32, background: u32, foreground: u32) -> bool {
        (1..SAMPLES as u32).any(|coverage| {
            let color = [0, 8, 16].into_iter().fold(0, |color, shift| {
                let a = (background >> shift) & 255;
                let b = (foreground >> shift) & 255;
                color | (((a * (16 - coverage) + b * coverage + 8) / 16) << shift)
            });
            pixel == color && pixel != background && pixel != foreground
        })
    }

    #[test]
    fn all_primitives_have_real_light_and_dark_coverage_pixels_at_common_dpi() {
        for dpi in [96, 144, 192] {
            let scale = dpi as f32 / 96.0;
            for (background, foreground) in [(0xf3f3f3, 0xb85f00), (0x202020, 0xffcd60)] {
                for shape in 0..4 {
                    let mut raster = Raster::new(40 * dpi / 96, 24 * dpi / 96, background).unwrap();
                    match shape {
                        0 => raster.round_rect(
                            [0.0, 0.0, 38.0 * scale, 20.0 * scale],
                            10.0 * scale,
                            foreground,
                        ),
                        1 => raster.circle(10.0 * scale, 10.0 * scale, 7.0 * scale, foreground),
                        2 => raster.line(
                            (3.0 * scale, 3.0 * scale),
                            (20.0 * scale, 17.0 * scale),
                            1.5 * scale,
                            foreground,
                        ),
                        _ => raster.polygon(
                            &[
                                (4.0 * scale, 3.0 * scale),
                                (4.0 * scale, 17.0 * scale),
                                (16.0 * scale, 10.0 * scale),
                            ],
                            foreground,
                        ),
                    }
                    assert!(raster.valid);
                    raster.resolve();
                    let pixels: Vec<_> = raster.pixels.iter().map(|pixel| dib(*pixel)).collect();
                    assert!(pixels.contains(&background));
                    assert!(pixels.contains(&foreground));
                    assert!(
                        pixels
                            .iter()
                            .any(|pixel| mixed(*pixel, background, foreground)),
                        "shape={shape}, dpi={dpi}, background={background:x}"
                    );
                }
            }
        }
    }

    #[test]
    fn sample_composition_has_no_border_seam_and_preserves_color_channels() {
        let mut raster = Raster::new(24, 24, 0x123456).unwrap();
        raster.circle(12.0, 12.0, 10.0, 0x2193e7);
        raster.circle(12.0, 12.0, 8.5, 0x123456);
        raster.circle(12.0, 12.0, 6.0, 0xe34172);
        raster.resolve();
        assert_eq!(dib(raster.pixels[0]), 0x123456);
        assert_eq!(dib(raster.pixels[12 * 24 + 12]), 0xe34172);
        assert!(raster.pixels.contains(&dib(0x2193e7)));
        // Repainting the same partially covered shape does not compound alpha.
        let mut repeated = Raster::new(24, 24, 0x123456).unwrap();
        for _ in 0..2 {
            repeated.circle(12.0, 12.0, 10.0, 0x2193e7);
            repeated.circle(12.0, 12.0, 8.5, 0x123456);
            repeated.circle(12.0, 12.0, 6.0, 0xe34172);
        }
        repeated.resolve();
        assert_eq!(raster.pixels, repeated.pixels);
    }

    struct Target {
        dc: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
    }
    impl Target {
        fn new() -> Self {
            // SAFETY: Test-owned hidden memory DC/bitmap; partial owners are
            // initialized before assertions so failures still release resources.
            unsafe {
                let dc = CreateCompatibleDC(std::ptr::null_mut());
                assert!(!dc.is_null());
                let mut target = Self {
                    dc,
                    bitmap: std::ptr::null_mut(),
                    previous: std::ptr::null_mut(),
                };
                target.bitmap = CreateBitmap(180, 120, 1, 32, std::ptr::null());
                assert!(!target.bitmap.is_null());
                target.previous = SelectObject(dc, target.bitmap);
                assert!(!target.previous.is_null() && target.previous as isize != -1);
                target
            }
        }
    }
    impl Drop for Target {
        fn drop(&mut self) {
            // SAFETY: Restore bitmap selection before deleting test-owned objects.
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

    #[test]
    fn dib_present_honors_mapping_complex_clip_restores_dc_and_has_no_gdi_growth() {
        use windows_sys::Win32::{
            Foundation::{POINT, RECT},
            System::Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources},
        };
        let target = Target::new();
        let mut raster = Raster::new(32, 24, 0x102030).unwrap();
        raster.round_rect([0.0, 0.0, 32.0, 24.0], 12.0, 0x3084d2);
        // SAFETY: Only this private DC is changed. No window is displayed and no
        // focus/capture/real desktop or user data is involved.
        unsafe {
            let dc = target.dc;
            SetDCBrushColor(dc, 0x795132);
            FillRect(
                dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: 180,
                    bottom: 120,
                },
                GetStockObject(DC_BRUSH),
            );
            SetTextColor(dc, 0x123456);
            SetBkColor(dc, 0x234567);
            SetDCPenColor(dc, 0x345678);
            SetBkMode(dc, TRANSPARENT as i32);
            SetStretchBltMode(dc, HALFTONE);
            SetROP2(dc, R2_XORPEN);
            SetPolyFillMode(dc, WINDING);
            SetMapMode(dc, MM_ANISOTROPIC);
            SetWindowExtEx(dc, 1, 1, std::ptr::null_mut());
            SetViewportExtEx(dc, 2, 2, std::ptr::null_mut());
            SetWindowOrgEx(dc, 3, 4, std::ptr::null_mut());
            SetViewportOrgEx(dc, 13, 17, std::ptr::null_mut());
            MoveToEx(dc, 7, 11, std::ptr::null_mut());
            let clip = CreateRectRgn(10, 10, 150, 110);
            let hole = CreateRectRgn(30, 30, 40, 40);
            let after = CreateRectRgn(0, 0, 0, 0);
            assert!(!clip.is_null() && !hole.is_null() && !after.is_null());
            assert_eq!(CombineRgn(clip, clip, hole, RGN_DIFF), COMPLEXREGION);
            assert_ne!(SelectClipRgn(dc, clip), ERROR);
            let snapshot = || {
                let mut window = POINT::default();
                let mut viewport = POINT::default();
                let mut position = POINT::default();
                GetWindowOrgEx(dc, &mut window);
                GetViewportOrgEx(dc, &mut viewport);
                GetCurrentPositionEx(dc, &mut position);
                (
                    [
                        GetTextColor(dc),
                        GetBkColor(dc),
                        GetDCBrushColor(dc),
                        GetDCPenColor(dc),
                    ],
                    [OBJ_FONT, OBJ_PEN, OBJ_BRUSH, OBJ_BITMAP]
                        .map(|kind| GetCurrentObject(dc, kind as u32)),
                    [
                        GetBkMode(dc),
                        GetMapMode(dc),
                        GetROP2(dc),
                        GetPolyFillMode(dc),
                        GetStretchBltMode(dc),
                    ],
                    [
                        (window.x, window.y),
                        (viewport.x, viewport.y),
                        (position.x, position.y),
                    ],
                )
            };
            let before = snapshot();
            let before_level = SaveDC(dc);
            assert_ne!(before_level, 0);
            assert!(raster.paint(dc, 10, 10));
            assert_eq!(snapshot(), before);
            assert_eq!(GetClipRgn(dc, after), 1);
            assert_ne!(EqualRgn(clip, after), 0);
            // Temporarily inspect physical pixels. The hole at (30,30)..(40,40)
            // must be untouched, while an interior mapped pixel must be ink.
            SelectClipRgn(dc, std::ptr::null_mut());
            SetMapMode(dc, MM_TEXT);
            SetWindowOrgEx(dc, 0, 0, std::ptr::null_mut());
            SetViewportOrgEx(dc, 0, 0, std::ptr::null_mut());
            assert_eq!(GetPixel(dc, 35, 35), 0x795132);
            assert_eq!(GetPixel(dc, 55, 50), 0x3084d2);
            assert_eq!(GetPixel(dc, 155, 115), 0x795132);
            assert_ne!(RestoreDC(dc, before_level), 0);
            assert_eq!(snapshot(), before);
            let process = GetCurrentProcess();
            let count = GetGuiResources(process, GR_GDIOBJECTS);
            for _ in 0..200 {
                assert!(raster.paint(dc, 10, 10));
            }
            assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), count);
            assert_eq!(snapshot(), before);
            assert_eq!(GetClipRgn(dc, after), 1);
            assert_ne!(EqualRgn(clip, after), 0);
            SelectClipRgn(dc, std::ptr::null_mut());
            DeleteObject(clip);
            DeleteObject(hole);
            DeleteObject(after);
        }
    }

    #[test]
    fn allocation_coordinates_vertices_and_work_are_bounded_and_fail_closed() {
        for (width, height) in [
            (0, 1),
            (-1, 1),
            (1, 0),
            (i32::MAX, 1),
            (1, MAX_SIDE + 1),
            (257, 256),
        ] {
            assert!(Raster::new(width, height, 0).is_none());
        }
        assert!(Raster::new(MAX_SIDE, 1, 0).is_some());
        assert!(Raster::new(256, 256, 0).is_some());
        let mut empty = Raster::new(16, 16, 0).unwrap();
        empty.round_rect([5.0, 0.0, 5.0, 16.0], 3.0, 0xffffff);
        empty.line((0.0, 0.0), (16.0, 16.0), 0.0, 0xffffff);
        empty.circle(8.0, 8.0, 0.0, 0xffffff);
        assert!(empty.valid);
        assert!(empty.samples.iter().flatten().all(|pixel| *pixel == 0));
        // SAFETY: Null DC is rejected even when the raster itself is valid.
        assert!(!unsafe { empty.paint(std::ptr::null_mut(), 0, 0) });
        let mut invalid = Raster::new(16, 16, 0).unwrap();
        invalid.circle(f32::NAN, 0.0, 1.0, 0xffffff);
        assert!(!invalid.valid);
        invalid.circle(8.0, 8.0, 4.0, 0xffffff);
        assert!(invalid.samples.iter().flatten().all(|pixel| *pixel == 0));
        // SAFETY: Null is explicitly accepted by the paint API and never used.
        assert!(!unsafe { invalid.paint(std::ptr::null_mut(), 0, 0) });
        for points in [&[(0.0, 0.0); 2][..], &[(0.0, 0.0); MAX_VERTICES + 1][..]] {
            let mut raster = Raster::new(16, 16, 0).unwrap();
            raster.polygon(points, 1);
            assert!(!raster.valid);
        }
        let mut raster = Raster::new(256, 256, 0).unwrap();
        for _ in 0..9 {
            raster.round_rect([0.0, 0.0, 256.0, 256.0], 4.0, 1);
        }
        assert!(!raster.valid);
        assert!(raster.evaluations <= MAX_EVALUATIONS);
        let mut raster = Raster::new(1, 1, 0).unwrap();
        for _ in 0..=MAX_PRIMITIVES {
            raster.circle(-100.0, -100.0, 1.0, 1);
        }
        assert!(!raster.valid);
        assert_eq!(raster.evaluations, 0);
    }
}
