//! SVG-derived coverage masks; no runtime XML, disk I/O, or SVG drawing implementation.
use std::{ffi::c_void, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*};

const SIZES: [usize; 6] = [18, 27, 36, 48, 72, 144];
const ASSETS: [&[u8]; 4] = [
    include_bytes!("../../../assets/navigation/overview.mask"),
    include_bytes!("../../../assets/navigation/analytics.mask"),
    include_bytes!("../../../assets/navigation/logs.mask"),
    include_bytes!("../../../assets/navigation/settings.mask"),
];

// Public wingdi.h API; msimg32 is an inbox Windows 10/11 system DLL.
// Declared locally so this feature needs no additional Cargo feature/dependency.
#[link(name = "msimg32")]
unsafe extern "system" {
    fn AlphaBlend(
        destination: HDC,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        source: HDC,
        source_x: i32,
        source_y: i32,
        source_width: i32,
        source_height: i32,
        blend: BLENDFUNCTION,
    ) -> i32;
}

struct Mask<'a> {
    size: usize,
    coverage: &'a [u8],
}

fn read_frame(bytes: &[u8], requested: usize) -> Option<Mask<'_>> {
    if bytes.get(..4)? != b"NAV1" || !(18..=144).contains(&requested) {
        return None;
    }
    let mut offset = 4usize;
    let mut chosen = None;
    for size in SIZES {
        let header = bytes.get(offset..offset.checked_add(6)?)?;
        let dimension = u16::from_le_bytes(header[..2].try_into().ok()?) as usize;
        let length = u32::from_le_bytes(header[2..].try_into().ok()?) as usize;
        if dimension != size || length != size.checked_mul(size)? {
            return None;
        }
        offset = offset.checked_add(6)?;
        let end = offset.checked_add(length)?;
        let coverage = bytes.get(offset..end)?;
        if chosen.is_none() && size >= requested {
            chosen = Some(Mask { size, coverage });
        }
        offset = end;
    }
    if offset != bytes.len() {
        return None;
    }
    chosen
}

pub(super) fn icon_size(dpi: u32) -> i32 {
    ((18 * dpi.clamp(96, 768) + 48) / 96) as i32
}

fn tinted_pixels(mask: &Mask<'_>, size: usize, color: u32) -> Option<Vec<u8>> {
    if !(18..=144).contains(&size) || mask.coverage.len() != mask.size.checked_mul(mask.size)? {
        return None;
    }
    let mut pixels = vec![0u8; size.checked_mul(size)?.checked_mul(4)?];
    let channels = [(color >> 16) & 255, (color >> 8) & 255, color & 255];
    for y in 0..size {
        for x in 0..size {
            // Bilinear coverage resampling is done once per cache entry, not per paint.
            // Common DPI values use their exact generated frame without interpolation.
            let sx = ((x as f64 + 0.5) * mask.size as f64 / size as f64 - 0.5)
                .clamp(0.0, (mask.size - 1) as f64);
            let sy = ((y as f64 + 0.5) * mask.size as f64 / size as f64 - 0.5)
                .clamp(0.0, (mask.size - 1) as f64);
            let x0 = sx.floor() as usize;
            let y0 = sy.floor() as usize;
            let x1 = (x0 + 1).min(mask.size - 1);
            let y1 = (y0 + 1).min(mask.size - 1);
            let fx = sx - x0 as f64;
            let fy = sy - y0 as f64;
            let top = mask.coverage[y0 * mask.size + x0] as f64 * (1.0 - fx)
                + mask.coverage[y0 * mask.size + x1] as f64 * fx;
            let bottom = mask.coverage[y1 * mask.size + x0] as f64 * (1.0 - fx)
                + mask.coverage[y1 * mask.size + x1] as f64 * fx;
            let alpha = (top * (1.0 - fy) + bottom * fy).round() as u32;
            let pixel = &mut pixels[(y * size + x) * 4..][..4];
            for (output, channel) in pixel[..3].iter_mut().zip(channels) {
                *output = ((channel * alpha + 127) / 255) as u8;
            }
            pixel[3] = alpha as u8;
        }
    }
    Some(pixels)
}

struct TintedIcon {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    color: u32,
}
impl TintedIcon {
    fn new(target: HDC, mask: &Mask<'_>, size: i32, color: u32) -> Option<Self> {
        let pixels = tinted_pixels(mask, size as usize, color)?;
        // SAFETY: target is the caller's live paint DC. The private memory DC/DIB
        // are UI-thread-owned, with a bounded top-down 32bpp allocation.
        unsafe {
            let dc = CreateCompatibleDC(target);
            if dc.is_null() {
                return None;
            }
            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = size;
            info.bmiHeader.biHeight = -size;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut c_void = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, bitmap);
            if previous.is_null() || previous as isize == -1 {
                DeleteObject(bitmap);
                DeleteDC(dc);
                return None;
            }
            // The DIB has exactly size * size * 4 bytes and pixels is that length.
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
            Some(Self {
                dc,
                bitmap,
                previous,
                color,
            })
        }
    }
    fn draw(&self, target: HDC, rect: &RECT, size: i32) -> bool {
        // SAFETY: Both DCs are live; the source DIB holds premultiplied BGRA.
        // Source and destination sizes match, so AlphaBlend does no scaling.
        unsafe {
            AlphaBlend(
                target,
                rect.left,
                rect.top,
                size,
                size,
                self.dc,
                0,
                0,
                size,
                size,
                BLENDFUNCTION {
                    BlendOp: 0, // AC_SRC_OVER
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: 1, // AC_SRC_ALPHA
                },
            ) != 0
        }
    }
}
impl Drop for TintedIcon {
    fn drop(&mut self) {
        // SAFETY: Restore the borrowed stock bitmap before deleting owned objects;
        // the memory DC and bitmap are never exposed or selected elsewhere.
        unsafe {
            let restored = SelectObject(self.dc, self.previous);
            let bitmap_deleted = DeleteObject(self.bitmap);
            let dc_deleted = DeleteDC(self.dc);
            #[cfg(test)]
            tests::record_release(self.dc, self.bitmap, restored, bitmap_deleted, dc_deleted);
            #[cfg(not(test))]
            let _ = (restored, bitmap_deleted, dc_deleted);
        }
    }
}

#[derive(Default)]
pub(super) struct Cache {
    size: i32,
    entries: [Option<TintedIcon>; 8],
    next: [usize; 4],
}
impl Cache {
    pub(super) fn draw(
        &mut self,
        index: usize,
        dc: HDC,
        rect: &RECT,
        dpi: u32,
        color: u32,
    ) -> bool {
        let size = icon_size(dpi);
        let Some(asset) = ASSETS.get(index) else {
            return false;
        };
        if dc.is_null() || rect.right - rect.left < size || rect.bottom - rect.top < size {
            return false;
        }
        if self.size != size {
            self.entries = Default::default();
            self.next = Default::default();
            self.size = size;
        }
        let pair = &mut self.entries[index * 2..index * 2 + 2];
        if let Some(icon) = pair.iter().flatten().find(|icon| icon.color == color) {
            return icon.draw(dc, rect, size);
        }
        let Some(mask) = read_frame(asset, size as usize) else {
            return false;
        };
        let Some(icon) = TintedIcon::new(dc, &mask, size, color) else {
            return false;
        };
        let slot = self.next[index];
        pair[slot] = Some(icon);
        self.next[index] = 1 - slot;
        pair[slot]
            .as_ref()
            .is_some_and(|icon| icon.draw(dc, rect, size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Debug)]
    struct Release {
        dc: HDC,
        bitmap: HBITMAP,
        restored: HGDIOBJ,
        bitmap_deleted: i32,
        dc_deleted: i32,
    }

    thread_local! {
        // Opt-in and thread-local: unrelated/parallel UI tests cannot affect this
        // audit, unlike process-wide GetGuiResources or stale handle queries.
        static RELEASES: RefCell<Option<Vec<Release>>> = const { RefCell::new(None) };
    }

    pub(super) fn record_release(
        dc: HDC,
        bitmap: HBITMAP,
        restored: HGDIOBJ,
        bitmap_deleted: i32,
        dc_deleted: i32,
    ) {
        RELEASES.with_borrow_mut(|releases| {
            if let Some(releases) = releases {
                releases.push(Release {
                    dc,
                    bitmap,
                    restored,
                    bitmap_deleted,
                    dc_deleted,
                });
            }
        });
    }

    struct ReleaseAudit;
    impl ReleaseAudit {
        fn start() -> Self {
            RELEASES.with_borrow_mut(|releases| {
                assert!(releases.is_none());
                *releases = Some(Vec::new());
            });
            Self
        }
        fn assert_released(&self, expected: &[(HDC, HBITMAP)]) {
            RELEASES.with_borrow_mut(|releases| {
                let releases = releases.as_mut().unwrap();
                assert_eq!(
                    releases.len(),
                    expected.len(),
                    "unexpected destructor count"
                );
                for (release, &(dc, bitmap)) in releases.iter().zip(expected) {
                    assert_eq!((release.dc, release.bitmap), (dc, bitmap));
                    assert_eq!(release.restored, bitmap, "owned bitmap was not selected");
                    assert_ne!(
                        release.bitmap_deleted, 0,
                        "DeleteObject failed: {release:?}"
                    );
                    assert_ne!(release.dc_deleted, 0, "DeleteDC failed: {release:?}");
                }
                releases.clear();
            });
        }
    }
    impl Drop for ReleaseAudit {
        fn drop(&mut self) {
            RELEASES.with_borrow_mut(|releases| *releases = None);
        }
    }

    #[test]
    fn embedded_frames_have_padding_and_antialiased_artwork() {
        for asset in ASSETS {
            for size in SIZES {
                let mask = read_frame(asset, size).unwrap();
                assert_eq!(mask.size, size);
                assert_eq!(mask.coverage.len(), size * size);
                let white = tinted_pixels(&mask, size, 0x00ffffff).unwrap();
                for (pixel, &coverage) in white.as_chunks::<4>().0.iter().zip(mask.coverage) {
                    assert_eq!(pixel, &[coverage; 4]);
                }
                assert!(mask.coverage.iter().any(|&a| a >= 240));
                assert!(mask.coverage.iter().any(|&a| a > 0 && a < 240));
                for i in 0..size {
                    assert_eq!(mask.coverage[i], 0);
                    assert_eq!(mask.coverage[(size - 1) * size + i], 0);
                    assert_eq!(mask.coverage[i * size], 0);
                    assert_eq!(mask.coverage[i * size + size - 1], 0);
                }
            }
        }
    }

    #[test]
    fn corrupt_masks_fail_closed() {
        let asset = ASSETS[0];
        for length in [0, 3, 4, 9, 200, asset.len() - 1] {
            assert!(read_frame(&asset[..length], 18).is_none());
        }
        for index in [0, 4, 6] {
            let mut corrupt = asset.to_vec();
            corrupt[index] ^= 255;
            assert!(read_frame(&corrupt, 18).is_none());
        }
        let mut trailing = asset.to_vec();
        trailing.push(0);
        assert!(read_frame(&trailing, 18).is_none());
        assert!(read_frame(asset, 145).is_none());
    }

    #[test]
    fn tint_is_premultiplied_bgra_at_common_and_intermediate_dpi() {
        for dpi in [0, 96, 120, 144, 168, 192, 240, 256, 384, 768, u32::MAX] {
            let size = icon_size(dpi) as usize;
            assert!((18..=144).contains(&size));
            let mask = read_frame(ASSETS[2], size).unwrap();
            for color in [0x00d77800, 0x00ffc078, 0x00262220, 0x00f5f2ee] {
                let pixels = tinted_pixels(&mask, size, color).unwrap();
                assert_eq!(pixels.len(), size * size * 4);
                for pixel in pixels.as_chunks::<4>().0 {
                    let alpha = pixel[3] as u32;
                    for (channel, shift) in [16, 8, 0].into_iter().enumerate() {
                        assert_eq!(
                            pixel[channel] as u32,
                            (((color >> shift) & 255) * alpha + 127) / 255
                        );
                        assert!(pixel[channel] <= pixel[3]);
                    }
                }
            }
        }
        for (dpi, size) in [
            (96, 18),
            (144, 27),
            (192, 36),
            (256, 48),
            (384, 72),
            (768, 144),
        ] {
            assert_eq!(icon_size(dpi), size);
        }
    }

    #[test]
    fn cache_reuses_entries_and_releases_gdi_objects_on_replacement_and_drop() {
        const CHILD: &str = "AI_RELAY_ICON_GDI_AUDIT_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // Run only this test in a fresh test-harness process, so exact process
            // GDI counts remain meaningful even in a parallel all-UI test run.
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "ui::tabs::icons::tests::cache_reuses_entries_and_releases_gdi_objects_on_replacement_and_drop",
                    "--test-threads=1",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"),
                "isolated GDI audit did not run exactly one passing test: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        use windows_sys::Win32::System::Threading::{
            GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
        };
        // SAFETY: Only private memory DCs are used; no windows are launched. The
        // child harness runs this one test, with no competing GDI allocations.
        unsafe {
            let process = GetCurrentProcess();
            // Warm up GDI before taking the baseline.
            let warmup = CreateCompatibleDC(null_mut());
            assert!(!warmup.is_null());
            assert_ne!(DeleteDC(warmup), 0);
            assert_ne!(GdiFlush(), 0);
            let baseline = GetGuiResources(process, GR_GDIOBJECTS);
            let target = CreateCompatibleDC(null_mut());
            assert!(!target.is_null());
            assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), baseline + 1);
            let audit = ReleaseAudit::start();
            let mut cache = Cache::default();
            let rect = RECT {
                left: 0,
                top: 0,
                right: 144,
                bottom: 144,
            };
            for round in 0..24 {
                for dpi in [96, 144, 192, 256, 384, 768] {
                    for index in 0..4 {
                        // The default 1x1 destination bitmap is sufficient for testing
                        // source ownership; the blit is clipped, not a GUI acceptance test.
                        cache.draw(index, target, &rect, dpi, 0x00262220 + round);
                        cache.draw(index, target, &rect, dpi, 0x00d77800 + round);
                    }
                    assert_eq!(cache.entries.iter().flatten().count(), 8);
                    assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), baseline + 17);
                    let handles: Vec<_> = cache
                        .entries
                        .iter()
                        .flatten()
                        .map(|icon| (icon.dc, icon.bitmap))
                        .collect();
                    for &(dc, bitmap) in &handles {
                        assert_eq!(GetObjectType(dc), OBJ_MEMDC as u32);
                        assert_eq!(GetObjectType(bitmap), OBJ_BITMAP as u32);
                        assert_eq!(GetCurrentObject(dc, OBJ_BITMAP as u32), bitmap);
                    }
                    cache.draw(0, target, &rect, dpi, 0x00262220 + round);
                    let hit = cache.entries[0].as_ref().unwrap();
                    assert_eq!((hit.dc, hit.bitmap), handles[0]);
                    audit.assert_released(&[]);
                    // A stale HDC query is not proof of ownership: on Windows,
                    // GetObjectType may return OBJ_DC after successful DeleteDC,
                    // even without any intervening allocation. Audit the actual
                    // deselection/deletion results plus exact process GDI counts.
                    cache.draw(0, target, &rect, dpi, 0x00f5f2ee + round);
                    audit.assert_released(&handles[..1]);
                    assert_ne!(GdiFlush(), 0);
                    assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), baseline + 17);
                    let retired: Vec<_> = cache
                        .entries
                        .iter()
                        .flatten()
                        .map(|icon| (icon.dc, icon.bitmap))
                        .collect();
                    cache.entries = Default::default();
                    cache.next = Default::default();
                    audit.assert_released(&retired);
                    assert_ne!(GdiFlush(), 0);
                    assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), baseline + 1);
                }
            }
            assert!(!cache.draw(4, target, &rect, 96, 0));
            cache.draw(0, target, &rect, 96, 0x00262220);
            let icon = cache.entries.iter().flatten().next().unwrap();
            let old = (icon.dc, icon.bitmap);
            cache.draw(0, target, &rect, 144, 0x00262220);
            audit.assert_released(&[old]);
            assert_ne!(GdiFlush(), 0);
            assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), baseline + 3);
            assert_eq!(cache.size, 27);
            assert_eq!(cache.entries.iter().flatten().count(), 1);

            let icon = cache.entries.iter().flatten().next().unwrap();
            let (dc, bitmap) = (icon.dc, icon.bitmap);
            drop(cache);
            audit.assert_released(&[(dc, bitmap)]);
            assert_ne!(GdiFlush(), 0);
            assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), baseline + 1);
            assert_ne!(DeleteDC(target), 0);
            assert_ne!(GdiFlush(), 0);
            assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), baseline);
        }
    }
}
