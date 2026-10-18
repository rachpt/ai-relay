//! Presentation-only clipping for closed native controls. Call after native theme
//! selection, and repeat on theme/DPI changes; values and input remain native.
//! Edit 保留原生边框绘制，仅裁剪窗口外形，不保证像素级完整的圆角弧线。
#![cfg(windows)]

use super::theme::{self, Palette};
use std::{cell::Cell, ptr::null_mut, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

const SUBCLASS_ID: usize = 0x524f_554e;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn GetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: *mut usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Button,
    Edit { border: bool },
    Combo,
    Date,
}

fn identify(class: &str, style: u32) -> Option<Control> {
    if class.eq_ignore_ascii_case("Button") {
        match style & BS_TYPEMASK as u32 {
            n if n == BS_PUSHBUTTON as u32
                || n == BS_DEFPUSHBUTTON as u32
                || n == BS_OWNERDRAW as u32 =>
            {
                Some(Control::Button)
            }
            _ => None,
        }
    } else if class.eq_ignore_ascii_case("Edit") {
        (style & (ES_MULTILINE as u32 | WS_HSCROLL | WS_VSCROLL) == 0).then_some(Control::Edit {
            border: style & WS_BORDER != 0,
        })
    } else if class.eq_ignore_ascii_case("ComboBox") {
        (style & 3 == CBS_DROPDOWNLIST as u32).then_some(Control::Combo)
    } else if class.eq_ignore_ascii_case("SysDateTimePick32") {
        Some(Control::Date)
    } else {
        None
    }
}

fn px(dip: i32, dpi: u32) -> i32 {
    ((i64::from(dip) * i64::from(dpi.clamp(96, 768)) + 48) / 96) as i32
}

fn geometry(width: i32, height: i32, dpi: u32) -> (i32, i32, i32) {
    let width = width.max(0);
    let height = height.max(0);
    (width, height, px(12, dpi).min(width).min(height))
}

#[derive(Clone, Copy)]
struct Appearance {
    kind: theme::Resolved,
    palette: Palette,
    dpi: u32,
    control: Control,
    geometry: Option<(i32, i32, i32)>,
    busy: bool,
    attached: bool,
}
type State = Rc<Cell<Appearance>>;

unsafe fn control(hwnd: HWND) -> Option<Control> {
    // SAFETY: owner-thread live HWND; class name writes are bounded by the array.
    unsafe {
        let mut name = [0u16; 64];
        let count = GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32);
        identify(
            &String::from_utf16_lossy(&name[..count.max(0) as usize]),
            GetWindowLongPtrW(hwnd, GWL_STYLE) as u32,
        )
    }
}

unsafe fn state(hwnd: HWND) -> Option<State> {
    // SAFETY: only this module installs this proc/id, retaining one Rc until
    // removal. Clone before Win32 calls so synchronous destruction cannot free it.
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) == 0 || data == 0 {
            return None;
        }
        let pointer = data as *const Cell<Appearance>;
        Rc::increment_strong_count(pointer);
        Some(Rc::from_raw(pointer))
    }
}

unsafe fn detach(hwnd: HWND, state: &State) {
    // SAFETY: owner thread. Mark busy before SetWindowRgn, which may synchronously
    // send WINDOWPOSCHANGED/SIZE; never retain a Cell borrow across native calls.
    unsafe {
        let mut value = state.get();
        if !value.attached || value.busy {
            return;
        }
        value.busy = true;
        state.set(value);
        let cleared = SetWindowRgn(hwnd, null_mut(), 1) != 0;
        value = state.get();
        value.busy = false;
        if value.attached && cleared {
            value.geometry = None;
            if RemoveWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID) != 0 {
                value.attached = false;
                // Release only the subclass's owning reference, not our local Rc.
                drop(Rc::from_raw(Rc::as_ptr(state)));
            }
        }
        state.set(value);
    }
}

unsafe fn apply_geometry(hwnd: HWND, state: &State) {
    // SAFETY: the retained Rc survives reentrant destruction; attached is cleared
    // by NCDESTROY before forwarding. No raw state references cross Win32 calls.
    unsafe {
        let mut value = state.get();
        if !value.attached || value.busy {
            return;
        }
        let current = control(hwnd);
        if value.kind == theme::Resolved::Contrast || current.is_none() {
            detach(hwnd, state);
            return;
        }
        if let Some(current) = current {
            value.control = current;
            state.set(value);
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return;
        }
        // Combo's window rect is its CLOSED height, unlike CB_GETDROPPEDCONTROLRECT.
        let next = geometry(
            rect.right.saturating_sub(rect.left),
            rect.bottom.saturating_sub(rect.top),
            value.dpi,
        );
        if value.geometry == Some(next) {
            return;
        }
        // GDI's rounded rectangle excludes its final scanline/column. The extra
        // pixel keeps the region's bounding box equal to the native window size.
        let region = CreateRoundRectRgn(
            0,
            0,
            next.0.saturating_add(1),
            next.1.saturating_add(1),
            next.2,
            next.2,
        );
        if region.is_null() {
            return;
        }
        value.busy = true;
        state.set(value);
        let applied = SetWindowRgn(hwnd, region, 1) != 0;
        if !applied {
            // Success transfers ownership to Windows; failure leaves it with us.
            DeleteObject(region);
        }
        value = state.get();
        value.busy = false;
        if applied && value.attached {
            value.geometry = Some(next);
        }
        state.set(value);
        if value.attached && value.kind == theme::Resolved::Contrast {
            detach(hwnd, state);
        }
    }
}

/// Apply only to supported native controls; repeat after theme/DPI changes.
///
/// # Safety
/// `hwnd` must be a live native control on the calling UI thread. This module
/// owns its window region while attached; do not combine with another region owner.
pub(super) unsafe fn apply(hwnd: HWND, kind: theme::Resolved, palette: Palette, dpi: u32) {
    // SAFETY: caller guarantees owner-thread access; subclass data contains only
    // copied presentation traits. No application state or borrowed palette is kept.
    unsafe {
        let existing = state(hwnd);
        if kind == theme::Resolved::Contrast {
            if let Some(state) = existing {
                let mut value = state.get();
                value.kind = kind;
                state.set(value);
                detach(hwnd, &state);
            }
            return;
        }
        let Some(control) = control(hwnd) else {
            if let Some(state) = existing {
                detach(hwnd, &state);
            }
            return;
        };
        let state = if let Some(state) = existing {
            let mut value = state.get();
            value.kind = kind;
            value.palette = palette;
            value.dpi = dpi;
            value.control = control;
            state.set(value);
            state
        } else {
            let state = Rc::new(Cell::new(Appearance {
                kind,
                palette,
                dpi,
                control,
                geometry: None,
                busy: false,
                attached: true,
            }));
            let data = Rc::into_raw(state.clone());
            if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, data as usize) == 0 {
                drop(Rc::from_raw(data));
                return;
            }
            state
        };
        apply_geometry(hwnd, &state);
        if state.get().attached {
            RedrawWindow(
                hwnd,
                std::ptr::null(),
                null_mut(),
                RDW_INVALIDATE | RDW_FRAME,
            );
        }
    }
}

/// Paint a small rounded native-control surface without retaining GDI objects.
/// # Safety
/// `dc` is a live paint DC; the caller supplies its client-coordinate rectangle.
pub(super) unsafe fn surface(
    dc: HDC,
    rect: &RECT,
    dpi: u32,
    background: u32,
    fill: u32,
    border: u32,
) {
    // SAFETY: SaveDC protects all selected stock objects and their per-DC colors.
    // The stock pen/brush are borrowed and never deleted or retained.
    unsafe {
        let saved = SaveDC(dc);
        if saved == 0 {
            return;
        }
        SelectObject(dc, GetStockObject(DC_BRUSH));
        SelectObject(dc, GetStockObject(DC_PEN));
        SetDCBrushColor(dc, background);
        FillRect(dc, rect, GetStockObject(DC_BRUSH));
        SetDCBrushColor(dc, fill);
        SetDCPenColor(dc, border);
        let (_, _, diameter) = geometry(rect.right - rect.left, rect.bottom - rect.top, dpi);
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            diameter,
            diameter,
        );
        RestoreDC(dc, saved);
    }
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: installed on the owner thread with one owning Rc. Retain a local
    // reference before forwarding, since another subclass can destroy the HWND.
    unsafe {
        let pointer = data as *const Cell<Appearance>;
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        if message == WM_NCDESTROY {
            let mut value = state.get();
            value.attached = false;
            state.set(value);
            RemoveWindowSubclass(hwnd, Some(procedure), id);
            drop(Rc::from_raw(pointer));
            // No HWND access after forwarding destruction; OS releases its region.
            return DefSubclassProc(hwnd, message, w, l);
        }
        let result = DefSubclassProc(hwnd, message, w, l);
        if !state.get().attached {
            return result;
        }
        if matches!(
            message,
            WM_SIZE | WM_STYLECHANGED | WM_DPICHANGED | WM_DPICHANGED_AFTERPARENT
        ) {
            if matches!(message, WM_DPICHANGED | WM_DPICHANGED_AFTERPARENT) {
                let mut value = state.get();
                value.dpi = if message == WM_DPICHANGED {
                    (w & 0xffff) as u32
                } else {
                    GetDpiForWindow(hwnd)
                };
                state.set(value);
            }
            apply_geometry(hwnd, &state);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::{
        System::Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources},
        UI::Controls::*,
    };

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetFocus() -> HWND;
        fn SetFocus(hwnd: HWND) -> HWND;
        fn IsWindowEnabled(hwnd: HWND) -> i32;
    }

    fn palette() -> Palette {
        Palette {
            background: 0xffffff,
            surface: 0x322c29,
            text: 0xf5f2ee,
            disabled: 0xb2aaa2,
            accent: 0xffc078,
            border: 0x574e47,
            accent_surface: 0x543e29,
        }
    }

    #[test]
    fn geometry_scales_and_clamps() {
        for (dpi, diameter) in [(96, 12), (144, 18), (192, 24)] {
            assert_eq!(geometry(240, 32, dpi), (240, 32, diameter));
            assert_eq!(geometry(3, 2, dpi), (3, 2, 2));
            assert_eq!(geometry(-2, 0, dpi), (0, 0, 0));
        }
    }

    #[test]
    fn only_supported_native_classes_and_styles() {
        for style in [BS_PUSHBUTTON, BS_DEFPUSHBUTTON, BS_OWNERDRAW] {
            assert_eq!(identify("BUTTON", style as u32), Some(Control::Button));
        }
        for style in [BS_CHECKBOX, BS_AUTOCHECKBOX, BS_RADIOBUTTON, BS_GROUPBOX] {
            assert!(identify("Button", style as u32).is_none());
        }
        assert_eq!(
            identify("Edit", WS_BORDER),
            Some(Control::Edit { border: true })
        );
        for style in [ES_MULTILINE as u32, WS_VSCROLL, WS_HSCROLL] {
            assert!(identify("Edit", style | WS_BORDER).is_none());
        }
        assert_eq!(
            identify("ComboBox", CBS_DROPDOWNLIST as u32),
            Some(Control::Combo)
        );
        assert!(identify("ComboBox", CBS_DROPDOWN as u32).is_none());
        assert_eq!(identify("SysDateTimePick32", 0), Some(Control::Date));
        for class in [
            "SysListView32",
            "SysTabControl32",
            "SysMonthCal32",
            "ComboLBox",
        ] {
            assert!(identify(class, 0).is_none());
        }
    }

    unsafe fn window(class: &str, style: u32, parent: HWND) -> HWND {
        // SAFETY: test-owned native windows, all created on this test thread.
        unsafe {
            let class: Vec<u16> = class.encode_utf16().chain(Some(0)).collect();
            let text: Vec<u16> = "Native value".encode_utf16().chain(Some(0)).collect();
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                text.as_ptr(),
                style,
                100,
                100,
                240,
                80,
                parent,
                null_mut(),
                null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            hwnd
        }
    }

    unsafe fn region_bounds(hwnd: HWND) -> Option<RECT> {
        // SAFETY: owned scratch region is deleted on both result paths.
        unsafe {
            let region = CreateRectRgn(0, 0, 0, 0);
            assert!(!region.is_null());
            let mut rect = RECT::default();
            let result = GetWindowRgn(hwnd, region);
            if result != ERROR {
                GetRgnBox(region, &mut rect);
            }
            DeleteObject(region);
            (result != ERROR).then_some(rect)
        }
    }

    #[test]
    fn native_values_focus_visibility_resize_contrast_and_gdi() {
        // SAFETY: temporary native windows, no application data, network or disk.
        unsafe {
            let init = INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_DATE_CLASSES,
            };
            assert_ne!(InitCommonControlsEx(&init), 0);
            let parent = window("STATIC", WS_POPUP | WS_VISIBLE, null_mut());
            let edit = window(
                "EDIT",
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP,
                parent,
            );
            let combo = window("COMBOBOX", WS_CHILD | CBS_DROPDOWNLIST as u32, parent);
            let button = window(
                "BUTTON",
                WS_CHILD | WS_VISIBLE | BS_OWNERDRAW as u32,
                parent,
            );
            let date = window("SysDateTimePick32", WS_CHILD | DTS_SHORTDATEFORMAT, parent);
            let controls = [edit, combo, button, date];
            let label = [b'A' as u16, 0];
            SendMessageW(combo, CB_ADDSTRING, 0, label.as_ptr() as LPARAM);
            SendMessageW(combo, CB_SETCURSEL, 0, 0);
            SendMessageW(edit, EM_SETSEL, 2, 5);
            SendMessageW(button, BM_SETSTATE, 1, 0);
            SetFocus(edit);
            assert_eq!(GetFocus(), edit);
            let styles = controls.map(|hwnd| GetWindowLongPtrW(hwnd, GWL_STYLE));
            let extended = controls.map(|hwnd| GetWindowLongPtrW(hwnd, GWL_EXSTYLE));
            let visible = controls.map(|hwnd| IsWindowVisible(hwnd));
            let enabled = controls.map(|hwnd| IsWindowEnabled(hwnd));
            let button_state = SendMessageW(button, BM_GETSTATE, 0, 0);
            let margins = SendMessageW(edit, EM_GETMARGINS, 0, 0);
            let mut content = RECT::default();
            SendMessageW(edit, EM_GETRECT, 0, (&mut content as *mut RECT) as LPARAM);
            let mut dropped = RECT::default();
            SendMessageW(
                combo,
                CB_GETDROPPEDCONTROLRECT,
                0,
                (&mut dropped as *mut RECT) as LPARAM,
            );
            let mut combo_rect = RECT::default();
            assert_ne!(GetWindowRect(combo, &mut combo_rect), 0);
            assert!(dropped.bottom - dropped.top > combo_rect.bottom - combo_rect.top);
            let mut date_before = windows_sys::Win32::Foundation::SYSTEMTIME::default();
            let date_result = SendMessageW(
                date,
                DTM_GETSYSTEMTIME,
                0,
                (&mut date_before as *mut _) as LPARAM,
            );
            let p = palette();
            for dpi in [96, 144, 192] {
                for kind in [
                    theme::Resolved::Light,
                    theme::Resolved::Dark,
                    theme::Resolved::Contrast,
                ] {
                    for hwnd in controls {
                        apply(hwnd, kind, p, dpi);
                        if kind == theme::Resolved::Contrast {
                            assert!(state(hwnd).is_none());
                            assert!(region_bounds(hwnd).is_none());
                        } else {
                            let bounds = region_bounds(hwnd).expect("rounded window region");
                            let mut native = RECT::default();
                            assert_ne!(GetWindowRect(hwnd, &mut native), 0);
                            assert_eq!((bounds.left, bounds.top), (0, 0));
                            assert_eq!(
                                (bounds.right, bounds.bottom),
                                (native.right - native.left, native.bottom - native.top)
                            );
                        }
                    }
                    assert_eq!(controls.map(|h| GetWindowLongPtrW(h, GWL_STYLE)), styles);
                    assert_eq!(
                        controls.map(|h| GetWindowLongPtrW(h, GWL_EXSTYLE)),
                        extended
                    );
                    assert_eq!(controls.map(|h| IsWindowVisible(h)), visible);
                    assert_eq!(controls.map(|h| IsWindowEnabled(h)), enabled);
                    assert_eq!(GetFocus(), edit);
                    assert_eq!(SendMessageW(combo, CB_GETCURSEL, 0, 0), 0);
                    assert_eq!(SendMessageW(edit, EM_GETSEL, 0, 0), 2 | (5 << 16));
                    assert_eq!(SendMessageW(button, BM_GETSTATE, 0, 0), button_state);
                    assert_eq!(SendMessageW(edit, EM_GETMARGINS, 0, 0), margins);
                    let mut current = RECT::default();
                    SendMessageW(edit, EM_GETRECT, 0, (&mut current as *mut RECT) as LPARAM);
                    assert_eq!(
                        (current.left, current.top, current.right, current.bottom),
                        (content.left, content.top, content.right, content.bottom)
                    );
                    let mut text = [0u16; 32];
                    let len = GetWindowTextW(edit, text.as_mut_ptr(), text.len() as i32);
                    assert_eq!(
                        String::from_utf16_lossy(&text[..len as usize]),
                        "Native value"
                    );
                }
            }
            let mut date_after = windows_sys::Win32::Foundation::SYSTEMTIME::default();
            assert_eq!(
                SendMessageW(
                    date,
                    DTM_GETSYSTEMTIME,
                    0,
                    (&mut date_after as *mut _) as LPARAM
                ),
                date_result
            );
            assert_eq!(
                (date_before.wYear, date_before.wMonth, date_before.wDay),
                (date_after.wYear, date_after.wMonth, date_after.wDay)
            );

            apply(edit, theme::Resolved::Dark, p, 96);
            assert_ne!(MoveWindow(edit, 0, 0, 210, 36, 0), 0);
            let resized = region_bounds(edit).unwrap();
            assert_eq!((resized.right, resized.bottom), (210, 36));
            apply(edit, theme::Resolved::Dark, p, 192);
            assert_eq!(state(edit).unwrap().get().geometry, Some((210, 36, 24)));
            SendMessageW(edit, WM_DPICHANGED, 144 | (144 << 16), 0);
            assert_eq!(state(edit).unwrap().get().geometry, Some((210, 36, 18)));
            SendMessageW(edit, WM_DPICHANGED_AFTERPARENT, 0, 0);
            assert_eq!(state(edit).unwrap().get().dpi, GetDpiForWindow(edit));
            SendMessageW(edit, WM_NCPAINT, 1, 0);
            // Warm native theme/DC caches before resource accounting.
            for _ in 0..20 {
                for hwnd in controls {
                    apply(hwnd, theme::Resolved::Dark, p, 96);
                }
                SendMessageW(edit, WM_NCPAINT, 1, 0);
            }
            let process = GetCurrentProcess();
            let before = GetGuiResources(process, GR_GDIOBJECTS);
            for index in 0..600 {
                let kind = if index % 3 == 0 {
                    theme::Resolved::Contrast
                } else if index % 3 == 1 {
                    theme::Resolved::Light
                } else {
                    theme::Resolved::Dark
                };
                for hwnd in controls {
                    apply(hwnd, kind, p, [96, 144, 192][index % 3]);
                }
                MoveWindow(edit, 0, 0, 210 + (index % 2) as i32, 36, 0);
                SendMessageW(edit, WM_NCPAINT, 1, 0);
            }
            for hwnd in controls {
                apply(hwnd, theme::Resolved::Dark, p, 96);
            }
            let after = GetGuiResources(process, GR_GDIOBJECTS);
            assert_eq!(
                before, after,
                "repeated apply/resize/NC paint leaked GDI objects"
            );
            println!("GDI objects over 600 apply cycles: {before} -> {after}");

            // A later incompatible style must restore the native rectangular edge.
            SetWindowLongPtrW(
                edit,
                GWL_STYLE,
                (styles[0] as u32 | ES_MULTILINE as u32) as isize,
            );
            assert!(state(edit).is_none());
            assert!(region_bounds(edit).is_none());
            // Retain a reference to prove destruction marks data dead before freeing.
            let retained = state(combo).unwrap();
            assert_ne!(DestroyWindow(parent), 0);
            assert!(!retained.get().attached);
        }
    }
}
