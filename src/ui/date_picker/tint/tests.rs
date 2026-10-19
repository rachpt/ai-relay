use super::*;
use crate::{
    config::ThemeMode,
    ui::date_picker,
    windows::{Font, scale, wide},
};
use windows_sys::Win32::{
    Foundation::SYSTEMTIME,
    UI::{Controls::*, HiDpi::*},
};

#[link(name = "user32")]
unsafe extern "system" {
    fn SetFocus(hwnd: HWND) -> HWND;
    fn GetFocus() -> HWND;
    fn EnableWindow(hwnd: HWND, enabled: i32) -> i32;
}

fn palette() -> Palette {
    crate::ui::theme::Theme::new(ThemeMode::Dark)
        .unwrap()
        .palette
}
fn luma(color: u32) -> u32 {
    (color & 255) * 299 / 1000
        + ((color >> 8) & 255) * 587 / 1000
        + ((color >> 16) & 255) * 114 / 1000
}
fn contrast(a: u32, b: u32) -> f64 {
    fn linear(color: u32) -> f64 {
        [0.2126, 0.7152, 0.0722]
            .iter()
            .enumerate()
            .map(|(index, weight)| {
                let value = ((color >> (index * 8)) & 255) as f64 / 255.0;
                weight
                    * if value <= 0.04045 {
                        value / 12.92
                    } else {
                        ((value + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum()
    }
    let (a, b) = (linear(a), linear(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
#[test]
fn raster_ramps_keep_native_white_black_gray_and_blue_states_readable() {
    let p = palette();
    let colors = Colors::new(p);
    assert_eq!(colors.pixel(0xffffffff), dib(p.surface));
    assert_eq!(colors.pixel(0xff000000), dib(p.text));
    let disabled = colors.pixel(0xff808080);
    assert!(contrast(disabled, dib(p.surface)) >= 4.5);
    assert!(contrast(colors.pixel(0xff000000), dib(p.surface)) >= 7.0);
    assert!(contrast(colors.pixel(0xffffffff), colors.pixel(0xff0078d7)) >= 4.5);
    for ramp in [&colors.neutral, &colors.accent] {
        for pair in ramp.windows(2) {
            assert!(luma(pair[0]) >= luma(pair[1]));
        }
        for pixel in ramp {
            assert_eq!(*pixel >> 24, 255);
        }
    }
}

struct Host(HWND);
impl Host {
    fn new() -> Self {
        // SAFETY: Hidden native test window; no app config, network or modal UI.
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                std::ptr::null(),
                WS_POPUP | WS_CLIPCHILDREN,
                0,
                0,
                1600,
                1200,
                null_mut(),
                null_mut(),
                null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            Self(hwnd)
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.0) };
    }
}
unsafe fn paint(hwnd: HWND, message: u32) -> Frame {
    // SAFETY: Test-owned HWND/DC. WM_PRINT payload uses the correct borrowed HDC.
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect);
        let target = Frame::new(rect.right - rect.left, rect.bottom - rect.top).unwrap();
        SetTextColor(target.dc, 0x123456);
        SetBkColor(target.dc, 0x654321);
        SetBkMode(target.dc, OPAQUE as i32);
        SetDCBrushColor(target.dc, 0x010203);
        let font = GetCurrentObject(target.dc, OBJ_FONT as u32);
        let pen = GetCurrentObject(target.dc, OBJ_PEN as u32);
        SendMessageW(
            hwnd,
            message,
            target.dc as WPARAM,
            (PRF_CLIENT | PRF_ERASEBKGND | PRF_NONCLIENT | PRF_CHILDREN) as LPARAM,
        );
        assert_eq!(GetTextColor(target.dc), 0x123456);
        assert_eq!(GetBkColor(target.dc), 0x654321);
        assert_eq!(GetBkMode(target.dc), OPAQUE as i32);
        assert_eq!(GetDCBrushColor(target.dc), 0x010203);
        assert_eq!(GetCurrentObject(target.dc, OBJ_FONT as u32), font);
        assert_eq!(GetCurrentObject(target.dc, OBJ_PEN as u32), pen);
        GdiFlush();
        target
    }
}
unsafe fn dark(frame: &Frame) {
    // SAFETY: Test-owned flushed top-down DIB and checked pixel count.
    unsafe {
        let pixels = std::slice::from_raw_parts(frame.bits, (frame.width * frame.height) as usize);

        let count = pixels.iter().filter(|pixel| luma(**pixel) < 110).count();
        assert!(
            count * 100 >= pixels.len() * 65,
            "native raster remains light/blank: dark={count}, pixels={}",
            pixels.len()
        );
        assert!(
            pixels.iter().any(|pixel| luma(*pixel) > 120),
            "native content disappeared"
        );
    }
}
unsafe fn value(hwnd: HWND) -> String {
    // SAFETY: Writable sized WM_GETTEXT buffer for test-owned picker.
    unsafe {
        let mut buffer = [0u16; 11];
        let n = GetWindowTextW(hwnd, buffer.as_mut_ptr(), 11);
        String::from_utf16(&buffer[..n as usize]).unwrap()
    }
}
#[test]
fn native_picker_print_focus_disable_none_and_theme_fallback_preserve_value_styles_and_dc() {
    let host = Host::new();
    let p = palette();
    // SAFETY: Isolated native controls; values/fonts live until host destruction.
    unsafe {
        let old_dpi = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let picker = date_picker::create(host.0, 2225, "结束（含）").unwrap();
        let styles = (
            GetWindowLongPtrW(picker, GWL_STYLE),
            GetWindowLongPtrW(picker, GWL_EXSTYLE),
        );
        for dpi in [96, 144, 192] {
            let font = Font::new(dpi).unwrap();
            MoveWindow(picker, 0, 0, scale(336, dpi), scale(28, dpi), 0);
            SendMessageW(picker, WM_SETFONT, font.0 as WPARAM, 0);
            assert_ne!(
                SendMessageW(picker, WM_SETTEXT, 0, wide("2024-02-29").as_ptr() as LPARAM),
                0
            );
            date_picker::apply(picker, Resolved::Dark, p, dpi);
            assert!(active(picker));
            for enabled in [true, false, true] {
                EnableWindow(picker, i32::from(enabled));
                for message in [WM_PRINTCLIENT, WM_PRINT] {
                    let frame = paint(picker, message);
                    dark(&frame);
                }
                assert_eq!(value(picker), "2024-02-29");
            }
            // Native focus/selected-field rendering uses the same raster path.
            ShowWindow(host.0, SW_SHOWNOACTIVATE);
            SetFocus(picker);
            assert_eq!(GetFocus(), picker);
            let frame = paint(picker, WM_PRINTCLIENT);
            dark(&frame);
            drop(frame);
            SendMessageW(picker, WM_KEYDOWN, 0x27, 0);
            SendMessageW(picker, WM_KEYUP, 0x27, 0);
            assert_eq!(value(picker), "2024-02-29");
            assert_ne!(
                SendMessageW(picker, WM_SETTEXT, 0, wide("").as_ptr() as LPARAM),
                0
            );
            let frame = paint(picker, WM_PRINTCLIENT);
            dark(&frame);
            drop(frame);
            assert_eq!(value(picker), "");
            for kind in [Resolved::Light, Resolved::Contrast] {
                date_picker::apply(picker, kind, p, dpi);
                assert!(!active(picker));
                assert_eq!(GetWindowLongPtrW(picker, GWL_STYLE), styles.0);
                assert_eq!(GetWindowLongPtrW(picker, GWL_EXSTYLE), styles.1);
                assert_eq!(value(picker), "");
            }
            SetFocus(null_mut());
            ShowWindow(host.0, SW_HIDE);
            SendMessageW(picker, WM_SETFONT, 0, 0);
        }
        if !old_dpi.is_null() {
            SetThreadDpiAwarenessContext(old_dpi);
        }
    }
}

struct VisualStyles {
    handle: windows_sys::Win32::Foundation::HANDLE,
    cookie: usize,
}
impl VisualStyles {
    fn new() -> Self {
        #[repr(C)]
        struct Context {
            size: u32,
            flags: u32,
            source: *const u16,
            architecture: u16,
            language: u16,
            directory: *const u16,
            resource: *const u16,
            application: *const u16,
            module: windows_sys::Win32::Foundation::HMODULE,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateActCtxW(context: *const Context) -> windows_sys::Win32::Foundation::HANDLE;
            fn ActivateActCtx(
                handle: windows_sys::Win32::Foundation::HANDLE,
                cookie: *mut usize,
            ) -> i32;
        }
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ai-relay.manifest");
        let source: Vec<u16> = path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let context = Context {
            size: size_of::<Context>() as u32,
            flags: 0,
            source: source.as_ptr(),
            architecture: 0,
            language: 0,
            directory: std::ptr::null(),
            resource: std::ptr::null(),
            application: std::ptr::null(),
            module: null_mut(),
        };
        // SAFETY: Test-only documented activation-context ABI. Rust test binaries
        // do not activate the application's Common Controls v6 manifest by default.
        unsafe {
            let handle = CreateActCtxW(&context);
            assert_ne!(handle as isize, -1);
            let mut cookie = 0;
            assert_ne!(ActivateActCtx(handle, &mut cookie), 0);
            Self { handle, cookie }
        }
    }
}
impl Drop for VisualStyles {
    fn drop(&mut self) {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn DeactivateActCtx(flags: u32, cookie: usize) -> i32;
            fn ReleaseActCtx(handle: windows_sys::Win32::Foundation::HANDLE);
        }
        // SAFETY: The activation cookie is active on this test thread; all
        // controls were destroyed before this guard releases the context.
        unsafe {
            DeactivateActCtx(0, self.cookie);
            ReleaseActCtx(self.handle);
        }
    }
}

const PRINT_SENTINEL: u32 = 0x00c0_1dc0;

struct PrintRegion(HRGN);
impl PrintRegion {
    unsafe fn new() -> Self {
        // SAFETY: Private scratch region, owned until Drop on this test thread.
        let region = unsafe { CreateRectRgn(0, 0, 0, 0) };
        assert!(!region.is_null());
        Self(region)
    }
}
impl Drop for PrintRegion {
    fn drop(&mut self) {
        // SAFETY: Only this guard owns the scratch region.
        unsafe { DeleteObject(self.0) };
    }
}

unsafe fn bordered_picker(parent: HWND, font: HFONT) -> HWND {
    // SAFETY: Private picker and borrowed font outliving its parent. Request a
    // real native nonclient inset so opposite-area assertions cannot be vacuous.
    unsafe {
        let picker = date_picker::create(parent, 2302, "开始（含）").unwrap();
        let style = GetWindowLongPtrW(picker, GWL_STYLE) as u32;
        let extended = GetWindowLongPtrW(picker, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(picker, GWL_STYLE, (style | WS_BORDER) as isize);
        SetWindowLongPtrW(picker, GWL_EXSTYLE, (extended | WS_EX_CLIENTEDGE) as isize);
        assert_ne!(
            SetWindowPos(
                picker,
                null_mut(),
                19,
                23,
                336,
                40,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW | SWP_FRAMECHANGED,
            ),
            0
        );
        SendMessageW(picker, WM_SETFONT, font as WPARAM, 0);
        assert_ne!(
            SendMessageW(picker, WM_SETTEXT, 0, wide("2024-02-29").as_ptr() as LPARAM),
            0
        );
        picker
    }
}

unsafe fn print_rects(hwnd: HWND) -> (RECT, RECT) {
    // SAFETY: Read actual native geometry independently of the tint helper.
    unsafe {
        let mut outer = RECT::default();
        let mut client = RECT::default();
        let mut origin = POINT::default();
        assert_ne!(GetWindowRect(hwnd, &mut outer), 0);
        assert_ne!(GetClientRect(hwnd, &mut client), 0);
        assert_ne!(ClientToScreen(hwnd, &mut origin), 0);
        let window = RECT {
            right: outer.right - outer.left,
            bottom: outer.bottom - outer.top,
            ..Default::default()
        };
        let client = RECT {
            left: origin.x - outer.left,
            top: origin.y - outer.top,
            right: origin.x - outer.left + client.right,
            bottom: origin.y - outer.top + client.bottom,
        };
        assert!(client.left > 0 && client.top > 0);
        assert!(client.right < window.right && client.bottom < window.bottom);
        assert!(client.right > client.left && client.bottom > client.top);
        (window, client)
    }
}

unsafe fn checked_print(hwnd: HWND, message: u32, flags: i32, target: &Frame) {
    // SAFETY: Test-owned HWND and memory DC, with no application State or pump.
    // Check the complete caller clip (including holes), mapping and selections.
    unsafe {
        SetTextColor(target.dc, 0x123456);
        SetBkColor(target.dc, 0x654321);
        SetBkMode(target.dc, OPAQUE as i32);
        SetDCBrushColor(target.dc, 0x010203);
        SetDCPenColor(target.dc, 0x030201);
        SetTextAlign(target.dc, TA_RIGHT | TA_BOTTOM);
        MoveToEx(target.dc, 7, 9, null_mut());
        let coordinates = || {
            let mut window_origin = POINT::default();
            let mut viewport_origin = POINT::default();
            let mut window_extent = windows_sys::Win32::Foundation::SIZE::default();
            let mut viewport_extent = windows_sys::Win32::Foundation::SIZE::default();
            let mut position = POINT::default();
            assert_ne!(GetWindowOrgEx(target.dc, &mut window_origin), 0);
            assert_ne!(GetViewportOrgEx(target.dc, &mut viewport_origin), 0);
            assert_ne!(GetWindowExtEx(target.dc, &mut window_extent), 0);
            assert_ne!(GetViewportExtEx(target.dc, &mut viewport_extent), 0);
            assert_ne!(GetCurrentPositionEx(target.dc, &mut position), 0);
            (
                GetMapMode(target.dc),
                GetStretchBltMode(target.dc),
                [
                    (window_origin.x, window_origin.y),
                    (viewport_origin.x, viewport_origin.y),
                    (window_extent.cx, window_extent.cy),
                    (viewport_extent.cx, viewport_extent.cy),
                    (position.x, position.y),
                ],
            )
        };
        let drawing = || {
            (
                GetTextColor(target.dc),
                GetBkColor(target.dc),
                GetBkMode(target.dc),
                GetDCBrushColor(target.dc),
                GetDCPenColor(target.dc),
                GetTextAlign(target.dc),
                [OBJ_FONT, OBJ_PEN, OBJ_BRUSH, OBJ_BITMAP]
                    .map(|kind| GetCurrentObject(target.dc, kind as u32)),
            )
        };
        let before_coordinates = coordinates();
        let before_drawing = drawing();
        let before_clip = PrintRegion::new();
        let has_clip = GetClipRgn(target.dc, before_clip.0);
        assert_ne!(has_clip, -1);
        SendMessageW(hwnd, message, target.dc as WPARAM, flags as LPARAM);
        assert_eq!(coordinates(), before_coordinates);
        assert_eq!(drawing(), before_drawing);
        let after_clip = PrintRegion::new();
        assert_eq!(GetClipRgn(target.dc, after_clip.0), has_clip);
        if has_clip == 1 {
            assert_ne!(EqualRgn(before_clip.0, after_clip.0), 0);
        }
        GdiFlush();
    }
}

unsafe fn sentinel_target(width: i32, height: i32, zoom: i32, offset: (i32, i32)) -> Frame {
    // SAFETY: Bounded test DIB; only its own DC receives mapping changes.
    unsafe {
        let target =
            Frame::new(width * zoom + offset.0 + 13, height * zoom + offset.1 + 11).unwrap();
        GdiFlush();
        std::slice::from_raw_parts_mut(target.bits, (target.width * target.height) as usize)
            .fill(dib(PRINT_SENTINEL));
        if zoom != 1 || offset != (0, 0) {
            assert_ne!(SetMapMode(target.dc, MM_ANISOTROPIC), 0);
            assert_ne!(SetWindowExtEx(target.dc, 1, 1, null_mut()), 0);
            assert_ne!(SetViewportExtEx(target.dc, zoom, zoom, null_mut()), 0);
            assert_ne!(SetWindowOrgEx(target.dc, 5, 7, null_mut()), 0);
            assert_ne!(
                SetViewportOrgEx(
                    target.dc,
                    offset.0 + 5 * zoom,
                    offset.1 + 7 * zoom,
                    null_mut()
                ),
                0
            );
        }
        SetStretchBltMode(target.dc, COLORONCOLOR);
        target
    }
}

unsafe fn assert_print_pixels(
    target: &Frame,
    native: &Frame,
    zoom: i32,
    offset: (i32, i32),
    included: impl Fn(i32, i32) -> bool,
) {
    // SAFETY: Both flushed DIBs are private and their dimensions bound all reads.
    unsafe {
        GdiFlush();
        let pixels =
            std::slice::from_raw_parts(target.bits, (target.width * target.height) as usize);
        let source =
            std::slice::from_raw_parts(native.bits, (native.width * native.height) as usize);
        let mut changed = 0;
        for y in 0..target.height {
            for x in 0..target.width {
                let sx = (x - offset.0).div_euclid(zoom);
                let sy = (y - offset.1).div_euclid(zoom);
                let pixel = pixels[(y * target.width + x) as usize];
                if sx >= 0 && sy >= 0 && sx < native.width && sy < native.height && included(sx, sy)
                {
                    let expected = source[(sy * native.width + sx) as usize];
                    assert_eq!(pixel & 0x00ff_ffff, expected & 0x00ff_ffff, "pixel {x},{y}");
                    changed +=
                        usize::from(pixel & 0x00ff_ffff != dib(PRINT_SENTINEL) & 0x00ff_ffff);
                } else {
                    assert_eq!(pixel, dib(PRINT_SENTINEL), "forbidden write at {x},{y}");
                }
            }
        }
        assert!(changed > 0, "requested native print area was not painted");
    }
}

#[test]
fn native_print_zero_flags_and_checkvisible_preserve_every_sentinel_pixel() {
    let _visual_styles = VisualStyles::new();
    let font = Font::new(96).unwrap();
    let host = Host::new();
    // SAFETY: Private native windows, moved offscreen before the visibility cases.
    unsafe {
        MoveWindow(host.0, -4000, -4000, 1600, 1200, 0);
        let picker = bordered_picker(host.0, font.0);
        date_picker::apply(picker, Resolved::Dark, palette(), 96);
        assert!(active(picker));
        let (window, _) = print_rects(picker);
        for (parent_visible, child_visible) in [(false, true), (true, false), (true, true)] {
            ShowWindow(
                host.0,
                if parent_visible {
                    SW_SHOWNOACTIVATE
                } else {
                    SW_HIDE
                },
            );
            ShowWindow(
                picker,
                if child_visible {
                    SW_SHOWNOACTIVATE
                } else {
                    SW_HIDE
                },
            );
            let visible = IsWindowVisible(picker) != 0;
            assert_eq!(visible, parent_visible && child_visible);
            let full = PRF_CHECKVISIBLE | PRF_CLIENT | PRF_NONCLIENT | PRF_ERASEBKGND;
            for flags in [0, PRF_CHECKVISIBLE, full] {
                let target = sentinel_target(window.right, window.bottom, 1, (0, 0));
                checked_print(picker, WM_PRINT, flags, &target);
                let pixels = std::slice::from_raw_parts(
                    target.bits,
                    (target.width * target.height) as usize,
                );
                if flags == full && visible {
                    assert!(pixels.iter().any(|pixel| *pixel != dib(PRINT_SENTINEL)));
                } else {
                    assert!(pixels.iter().all(|pixel| *pixel == dib(PRINT_SENTINEL)));
                }
            }
        }
        assert_eq!(value(picker), "2024-02-29");
    }
}

#[test]
fn native_print_parts_keep_opposite_area_and_restore_mapped_clipped_dc() {
    let _visual_styles = VisualStyles::new();
    let font = Font::new(96).unwrap();
    let host = Host::new();
    let p = palette();
    // SAFETY: Private native picker/DIBs. Native references use the same live
    // HWND without tint; no synthetic painter or assumed border thickness.
    unsafe {
        let picker = bordered_picker(host.0, font.0);
        for flags in [
            PRF_NONCLIENT,
            PRF_CLIENT,
            PRF_CLIENT | PRF_NONCLIENT,
            PRF_CLIENT | PRF_NONCLIENT | PRF_ERASEBKGND,
        ] {
            date_picker::apply(picker, Resolved::Light, p, 96);
            let (window, client) = print_rects(picker);
            let native = Frame::new(window.right, window.bottom).unwrap();
            {
                let _saved = SavedDc::new(native.dc).unwrap();
                SendMessageW(picker, WM_PRINT, native.dc as WPARAM, flags as LPARAM);
            }
            native.recolor(&Colors::new(p));
            if flags & (PRF_CLIENT | PRF_NONCLIENT) == (PRF_CLIENT | PRF_NONCLIENT) {
                dark(&native);
            }
            date_picker::apply(picker, Resolved::Dark, p, 96);
            assert!(active(picker));
            for (zoom, offset) in [(1, (0, 0)), (1, (17, 13)), (2, (23, 19))] {
                for clipped in [false, true] {
                    let target = sentinel_target(window.right, window.bottom, zoom, offset);
                    if clipped {
                        assert_ne!(
                            IntersectClipRect(
                                target.dc,
                                -2,
                                -2,
                                window.right + 2,
                                window.bottom + 2
                            ),
                            ERROR
                        );
                        assert_ne!(
                            ExcludeClipRect(
                                target.dc,
                                window.right / 2,
                                0,
                                window.right / 2 + 5,
                                window.bottom
                            ),
                            ERROR
                        );
                    }
                    checked_print(picker, WM_PRINT, flags, &target);
                    assert_print_pixels(&target, &native, zoom, offset, |x, y| {
                        let in_client = x >= client.left
                            && x < client.right
                            && y >= client.top
                            && y < client.bottom;
                        let in_hole = clipped && x >= window.right / 2 && x < window.right / 2 + 5;
                        !in_hole
                            && if in_client {
                                flags & PRF_CLIENT != 0
                            } else {
                                flags & PRF_NONCLIENT != 0
                            }
                    });
                }
            }
        }
        assert_eq!(value(picker), "2024-02-29");
    }
}

#[test]
fn native_printclient_keeps_zero_flag_client_origin_semantics() {
    let _visual_styles = VisualStyles::new();
    let font = Font::new(96).unwrap();
    let host = Host::new();
    let p = palette();
    // SAFETY: Hidden private native picker; PRINTCLIENT is not WM_PRINT and
    // must neither require PRF_CLIENT nor add the window's nonclient offset.
    unsafe {
        let picker = bordered_picker(host.0, font.0);
        for flags in [0, PRF_CLIENT] {
            date_picker::apply(picker, Resolved::Light, p, 96);
            let (_, client) = print_rects(picker);
            let native =
                Frame::new(client.right - client.left, client.bottom - client.top).unwrap();
            {
                let _saved = SavedDc::new(native.dc).unwrap();
                SendMessageW(picker, WM_PRINTCLIENT, native.dc as WPARAM, flags as LPARAM);
            }
            native.recolor(&Colors::new(p));
            if flags & PRF_CLIENT != 0 {
                dark(&native);
            }
            date_picker::apply(picker, Resolved::Dark, p, 96);
            assert!(active(picker));
            for (zoom, offset) in [(1, (0, 0)), (1, (17, 13)), (2, (23, 19))] {
                let target = sentinel_target(native.width, native.height, zoom, offset);
                checked_print(picker, WM_PRINTCLIENT, flags, &target);
                assert_print_pixels(&target, &native, zoom, offset, |_, _| true);
            }
        }
        assert_eq!(value(picker), "2024-02-29");
    }
}

#[test]
fn themed_month_calendar_minimum_grid_navigation_and_print_remain_native_at_three_fonts() {
    let _visual_styles = VisualStyles::new();
    let host = Host::new();
    let p = palette();
    // SAFETY: Test-only month calendar on its creating thread, no popup app data.
    unsafe {
        let init = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_DATE_CLASSES,
        };
        assert_ne!(InitCommonControlsEx(&init), 0);
        let picker = date_picker::create(host.0, 2302, "开始（含）").unwrap();
        ShowWindow(host.0, SW_SHOWNOACTIVATE);
        for dpi in [96, 144, 192] {
            let font = Font::new(dpi).unwrap();
            MoveWindow(picker, 0, 0, scale(336, dpi), scale(28, dpi), 0);
            SendMessageW(picker, WM_SETFONT, font.0 as WPARAM, 0);
            SendMessageW(picker, WM_SETTEXT, 0, wide("2024-02-15").as_ptr() as LPARAM);
            SendMessageW(picker, DTM_SETRANGE, 0, 0);
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
            let x = (info.rcButton.left + info.rcButton.right) / 2;
            let y = (info.rcButton.top + info.rcButton.bottom) / 2;
            let point = ((y << 16) | x) as LPARAM;
            SendMessageW(picker, WM_LBUTTONDOWN, 1, point);
            SendMessageW(picker, WM_LBUTTONUP, 0, point);
            let calendar = SendMessageW(picker, DTM_GETMONTHCAL, 0, 0) as HWND;
            assert!(!calendar.is_null(), "native picker did not create calendar");
            SendMessageW(calendar, WM_SETFONT, font.0 as WPARAM, 1);
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
            let mut outer = RECT::default();
            let mut client = RECT::default();
            GetWindowRect(calendar, &mut outer);
            GetClientRect(calendar, &mut client);
            MoveWindow(
                calendar,
                0,
                0,
                minimum.right + (outer.right - outer.left - client.right).max(0),
                minimum.bottom + (outer.bottom - outer.top - client.bottom).max(0),
                0,
            );
            let mut minimum_before = minimum;
            let date = SYSTEMTIME {
                wYear: 2024,
                wMonth: 2,
                wDay: 15,
                ..Default::default()
            };
            assert_ne!(
                SendMessageW(
                    calendar,
                    MCM_SETCURSEL,
                    0,
                    (&date as *const SYSTEMTIME) as LPARAM
                ),
                0
            );

            apply(calendar, Resolved::Dark, p);
            SendMessageW(
                calendar,
                MCM_GETMINREQRECT,
                0,
                (&mut minimum as *mut RECT) as LPARAM,
            );
            assert_eq!(
                (minimum.right, minimum.bottom),
                (minimum_before.right, minimum_before.bottom)
            );
            ShowWindow(host.0, SW_SHOWNOACTIVATE);
            UpdateWindow(calendar);
            let frame = paint(calendar, WM_PRINTCLIENT);
            dark(&frame);
            drop(frame);
            SendMessageW(calendar, WM_KEYDOWN, 0x22, 0);
            SendMessageW(calendar, WM_KEYUP, 0x22, 0);
            SendMessageW(calendar, WM_KEYDOWN, 0x27, 0);
            SendMessageW(calendar, WM_KEYUP, 0x27, 0);
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
            assert_eq!(
                (selected.wYear, selected.wMonth, selected.wDay),
                (2024, 3, 16)
            );
            // Let the native month-transition animation process its bounded
            // timer messages before requesting a complete settled raster.
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(350);
            while std::time::Instant::now() < deadline {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let frame = paint(calendar, WM_PRINT);
            dark(&frame);
            drop(frame);
            apply(calendar, Resolved::Contrast, p);
            assert!(!active(calendar));
            SendMessageW(
                calendar,
                MCM_GETMINREQRECT,
                0,
                (&mut minimum_before as *mut RECT) as LPARAM,
            );
            assert_eq!(
                (minimum.right, minimum.bottom),
                (minimum_before.right, minimum_before.bottom)
            );
            SendMessageW(picker, DTM_CLOSEMONTHCAL, 0, 0);
            SendMessageW(picker, WM_SETFONT, 0, 0);
        }
    }
}

#[test]
fn warmed_native_raster_resources_and_redraw_suppression_are_bounded() {
    use windows_sys::Win32::System::Threading::{
        GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
    };
    let host = Host::new();
    let picker = date_picker::create(host.0, 2302, "开始（含）").unwrap();
    let p = palette();
    // SAFETY: Test-owned live HWNDs and resource counters on the current process.
    unsafe {
        SendMessageW(picker, WM_SETTEXT, 0, wide("2024-02-29").as_ptr() as LPARAM);
        date_picker::apply(picker, Resolved::Dark, p, 96);
        drop(paint(picker, WM_PRINTCLIENT));
        date_picker::apply(picker, Resolved::Light, p, 96);
        let baseline = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
        for _ in 0..30 {
            date_picker::apply(picker, Resolved::Dark, p, 96);
            drop(paint(picker, WM_PRINTCLIENT));
            SendMessageW(picker, WM_SETREDRAW, 0, 0);
            assert!(!state(picker).unwrap().borrow().redraw);
            SendMessageW(picker, WM_SETREDRAW, 1, 0);
            assert!(state(picker).unwrap().borrow().redraw);
            date_picker::apply(picker, Resolved::Light, p, 96);
        }
        assert_eq!(
            GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS),
            baseline
        );
        assert!(Frame::new(0, 10).is_none());
        assert!(Frame::new(-1, 10).is_none());
        assert!(Frame::new(16384, 16384).is_none());
        date_picker::apply(picker, Resolved::Dark, p, 96);
        let retained = state(picker).unwrap();
        DestroyWindow(picker);
        assert!(!retained.borrow().attached);
        assert_eq!(value(host.0), "");
    }
}
