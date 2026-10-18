//! Non-client scrollbar colors only; native controls retain scrolling, selection,
//! clipboard, hit testing, capture, keyboard handling and accessibility.
//!
//! Call `apply` on the owning UI thread after native/control theme selection and
//! again on theme/DPI changes. This module deliberately does not theme top-level
//! windows, page hosts, standalone scrollbars, ComboLBox popups or system menus.
//! PrintWindow implementations that omit WM_PRINT/PRF_NONCLIENT cannot include
//! this overlay. Native modal tracking/theme animations can briefly repaint over
//! it between messages; this is not a replacement scrollbar implementation.
#![cfg(windows)]

use super::theme::{Palette, Resolved};
use crate::windows::scale;
use std::{cell::Cell, mem::size_of, ptr::null, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    UI::{Controls::*, WindowsAndMessaging::*},
};

const SUBCLASS_ID: usize = 0x5343_5448;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn GetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: *mut usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

#[derive(Clone, Copy)]
struct Appearance {
    palette: Palette,
    dpi: u32,
    attached: bool,
    enabled: bool,
    painting: bool,
}
type State = Rc<Cell<Appearance>>;

fn supported(class: &str, style: u32) -> bool {
    class.eq_ignore_ascii_case("SysListView32")
        || (class.eq_ignore_ascii_case("Static") && style & WS_VSCROLL != 0)
        || (class.eq_ignore_ascii_case("Edit")
            && style & ES_MULTILINE as u32 != 0
            && style & (WS_VSCROLL | WS_HSCROLL) != 0)
}

unsafe fn supported_window(hwnd: HWND) -> bool {
    // SAFETY: public queries tolerate an invalid HWND; class writes are bounded.
    unsafe {
        if IsWindow(hwnd) == 0 || GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CHILD == 0 {
            return false;
        }
        let mut name = [0u16; 64];
        let count = GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32);
        count > 0
            && supported(
                &String::from_utf16_lossy(&name[..count as usize]),
                GetWindowLongPtrW(hwnd, GWL_STYLE) as u32,
            )
    }
}

unsafe fn state(hwnd: HWND) -> Option<State> {
    // SAFETY: only this proc/id installs data, owning one Rc until removal. The
    // lookup cannot call application code; retain before any native forwarding.
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
    // SAFETY: owner thread and retained Rc. Disable even if removal fails; in
    // that case the installed owner remains alive until a later removal/destroy.
    unsafe {
        let mut value = state.get();
        value.enabled = false;
        state.set(value);
        if value.attached && RemoveWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID) != 0 {
            value.attached = false;
            state.set(value);
            drop(Rc::from_raw(Rc::as_ptr(state)));
        }
    }
}

/// Color native non-client scrollbars, without changing styles or scroll state.
/// Light/high-contrast modes remove the subclass and request native frame paint.
/// Failed HWND/API/attachment queries leave native rendering in place.
///
/// # Safety
/// `hwnd` must belong to the calling UI thread. Call after native theme changes;
/// the caller must resolve high contrast to `Resolved::Contrast`. No application
/// state or borrowed palette is retained. Unsupported/invalid HWNDs are ignored.
pub(super) unsafe fn apply(hwnd: HWND, kind: Resolved, palette: Palette, dpi: u32) {
    // SAFETY: owner-thread contract; we only query native state and install/remove
    // our own subclass. RedrawWindow may reenter/destroy, so nothing is borrowed.
    unsafe {
        if IsWindow(hwnd) == 0 {
            return;
        }
        let previous = state(hwnd);
        if kind != Resolved::Dark || !supported_window(hwnd) {
            if let Some(previous) = previous {
                detach(hwnd, &previous);
                RedrawWindow(
                    hwnd,
                    null(),
                    std::ptr::null_mut(),
                    RDW_INVALIDATE | RDW_FRAME | RDW_UPDATENOW | RDW_NOCHILDREN,
                );
            }
            return;
        }
        let state = if let Some(state) = previous {
            let mut value = state.get();
            value.palette = palette;
            value.dpi = dpi.clamp(96, 768);
            value.enabled = true;
            state.set(value);
            state
        } else {
            let state = Rc::new(Cell::new(Appearance {
                palette,
                dpi: dpi.clamp(96, 768),
                attached: true,
                enabled: true,
                painting: false,
            }));
            let owner = Rc::into_raw(Rc::clone(&state));
            if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, owner as usize) == 0 {
                drop(Rc::from_raw(owner));
                return;
            }
            state
        };
        RedrawWindow(
            hwnd,
            null(),
            std::ptr::null_mut(),
            RDW_INVALIDATE | RDW_FRAME | RDW_UPDATENOW | RDW_NOCHILDREN,
        );
        overlay(hwnd, &state, std::ptr::null_mut());
    }
}

struct SavedDc(HDC, i32);
impl SavedDc {
    unsafe fn new(dc: HDC) -> Option<Self> {
        // SAFETY: borrowed live DC; a failed save means we must not modify it.
        let index = unsafe { SaveDC(dc) };
        (index != 0).then_some(Self(dc, index))
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: matching successful SaveDC; DC outlives the guard.
        unsafe {
            RestoreDC(self.0, self.1);
        }
    }
}
struct WindowDc(HWND, HDC);
impl Drop for WindowDc {
    fn drop(&mut self) {
        // SAFETY: paired with GetWindowDC, never DeleteDC on a window DC.
        unsafe {
            ReleaseDC(self.0, self.1);
        }
    }
}

fn valid(rect: RECT) -> bool {
    rect.left < rect.right && rect.top < rect.bottom
}
fn intersection(a: RECT, b: RECT) -> RECT {
    RECT {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    }
}
fn offset(rect: RECT, origin: POINT) -> RECT {
    RECT {
        left: rect.left - origin.x,
        top: rect.top - origin.y,
        right: rect.right - origin.x,
        bottom: rect.bottom - origin.y,
    }
}

#[derive(Clone, Copy)]
struct Bar {
    rect: RECT,
    vertical: bool,
    button: i32,
    thumb_top: i32,
    thumb_bottom: i32,
    states: [u32; 6],
    scrollable: bool,
}
impl Bar {
    fn length(self) -> i32 {
        if self.vertical {
            self.rect.bottom - self.rect.top
        } else {
            self.rect.right - self.rect.left
        }
    }
    fn segment(self, start: i32, end: i32) -> RECT {
        let mut rect = self.rect;
        if self.vertical {
            rect.top += start;
            rect.bottom = self.rect.top + end;
        } else {
            rect.left += start;
            rect.right = self.rect.left + end;
        }
        rect
    }
}

unsafe fn bar(hwnd: HWND, vertical: bool, origin: POINT, bounds: RECT) -> Option<Bar> {
    // SAFETY: only standard non-client object IDs (not OBJID_CLIENT, which can
    // dispatch SBM_GETSCROLLBARINFO). Both structures are correctly sized.
    unsafe {
        let mut info = SCROLLBARINFO {
            cbSize: size_of::<SCROLLBARINFO>() as u32,
            ..Default::default()
        };
        if GetScrollBarInfo(
            hwnd,
            if vertical {
                OBJID_VSCROLL
            } else {
                OBJID_HSCROLL
            },
            &mut info,
        ) == 0
            || info.rgstate[0] & (STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN) != 0
        {
            return None;
        }
        // rcScrollBar is in screen coordinates. Thumb offsets are relative to
        // the bar's top/left, NOT screen or window coordinates. dxyLineButton is
        // the native end-button extent, not the thumb length. The Learn member
        // descriptions are terse; native top/bottom tests below check both axes
        // at nonzero window origins rather than treating offsets as absolute.
        // https://learn.microsoft.com/windows/win32/api/winuser/ns-winuser-scrollbarinfo
        let rect = offset(info.rcScrollBar, origin);
        let clipped = intersection(rect, bounds);
        if !valid(rect)
            || clipped.left != rect.left
            || clipped.top != rect.top
            || clipped.right != rect.right
            || clipped.bottom != rect.bottom
        {
            return None;
        }
        let mut scroll = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS | SIF_TRACKPOS,
            ..Default::default()
        };
        if GetScrollInfo(hwnd, if vertical { SB_VERT } else { SB_HORZ }, &mut scroll) == 0 {
            return None;
        }
        let range = i64::from(scroll.nMax) - i64::from(scroll.nMin) + 1;
        let mut result = Bar {
            rect,
            vertical,
            button: info.dxyLineButton,
            thumb_top: info.xyThumbTop,
            thumb_bottom: info.xyThumbBottom,
            states: info.rgstate,
            scrollable: range > i64::from(scroll.nPage.max(1)),
        };
        result.button = result.button.clamp(0, result.length() / 2);
        Some(result)
    }
}

unsafe fn fill(dc: HDC, rect: RECT, color: u32) {
    // SAFETY: valid DC/rectangle; stock brush is borrowed and restored by SavedDc.
    unsafe {
        if valid(rect) {
            SetDCBrushColor(dc, color);
            FillRect(dc, &rect, GetStockObject(DC_BRUSH) as HBRUSH);
        }
    }
}
fn component_color(states: u32, scrollable: bool, palette: Palette) -> u32 {
    if !scrollable || states & STATE_SYSTEM_UNAVAILABLE != 0 {
        palette.border
    } else if states & (STATE_SYSTEM_PRESSED | STATE_SYSTEM_HOTTRACKED) != 0 {
        palette.accent
    } else {
        palette.disabled
    }
}
unsafe fn capsule(dc: HDC, rect: RECT, color: u32) {
    // SAFETY: caller clips to the bar; stock brush/pen are borrowed and restored.
    unsafe {
        let brush = GetStockObject(DC_BRUSH);
        let pen = GetStockObject(NULL_PEN);
        if brush.is_null() || pen.is_null() || rect.right <= rect.left || rect.bottom <= rect.top {
            return;
        }
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);
        SetDCBrushColor(dc, color);
        let radius = ((rect.right - rect.left).min(rect.bottom - rect.top) / 2).max(1);
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
    }
}

fn slim_thumb(bar: Bar, dpi: u32) -> Option<RECT> {
    let hidden = STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN;
    if bar.states[3] & hidden != 0 || !bar.scrollable {
        return None;
    }
    let top = bar.thumb_top.max(bar.button);
    let bottom = bar.thumb_bottom.min(bar.length() - bar.button);
    if top >= bottom {
        return None;
    }
    let mut thumb = bar.segment(top, bottom);
    let thickness = scale(6, dpi).max(3);
    if bar.vertical {
        let available = (thumb.right - thumb.left).max(1);
        let thickness = thickness.min((available - 2).max(1));
        let center = (thumb.left + thumb.right) / 2;
        thumb.left = center - thickness / 2;
        thumb.right = thumb.left + thickness;
    } else {
        let available = (thumb.bottom - thumb.top).max(1);
        let thickness = thickness.min((available - 2).max(1));
        let center = (thumb.top + thumb.bottom) / 2;
        thumb.top = center - thickness / 2;
        thumb.bottom = thumb.top + thickness;
    }
    Some(thumb)
}

unsafe fn draw_bar(dc: HDC, bar: Bar, appearance: Appearance) {
    // SAFETY: saved clip is narrowed to the exact native bar rectangle. No pixels
    // outside it (including the native client edge) are intentionally modified.
    unsafe {
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        if IntersectClipRect(
            dc,
            bar.rect.left,
            bar.rect.top,
            bar.rect.right,
            bar.rect.bottom,
        ) == ERROR
        {
            return;
        }
        let p = appearance.palette;
        // Blend the native 17-DIP scrollbar strip into the page and paint only a
        // slim capsule thumb. Native hit testing, arrows, wheel and keyboard remain.
        fill(dc, bar.rect, p.background);
        let hidden = STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN;
        if bar.states[0] & hidden != 0 {
            return;
        }
        if let Some(thumb) = slim_thumb(bar, appearance.dpi) {
            capsule(
                dc,
                thumb,
                component_color(bar.states[0] | bar.states[3], bar.scrollable, p),
            );
        }
    }
}

unsafe fn draw(hwnd: HWND, dc: HDC, appearance: Appearance) {
    // SAFETY: live native child and DC in window coordinates. Query only; do not
    // synthesize scroll geometry or call SetScrollInfo/ShowScrollBar/style APIs.
    unsafe {
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        let mut window = RECT::default();
        let mut client = RECT::default();
        let mut client_origin = POINT::default();
        if GetWindowRect(hwnd, &mut window) == 0
            || GetClientRect(hwnd, &mut client) == 0
            || ClientToScreen(hwnd, &mut client_origin) == 0
        {
            return;
        }
        let origin = POINT {
            x: window.left,
            y: window.top,
        };
        let bounds = offset(window, origin);
        let client = RECT {
            left: client_origin.x - origin.x,
            top: client_origin.y - origin.y,
            right: client_origin.x - origin.x + client.right,
            bottom: client_origin.y - origin.y + client.bottom,
        };
        if ExcludeClipRect(dc, client.left, client.top, client.right, client.bottom) == ERROR {
            return;
        }
        let vertical = bar(hwnd, true, origin, bounds);
        let horizontal = bar(hwnd, false, origin, bounds);
        if let Some(bar) = vertical {
            draw_bar(dc, bar, appearance);
        }
        if let Some(bar) = horizontal {
            draw_bar(dc, bar, appearance);
        }
        if let (Some(v), Some(h)) = (vertical, horizontal) {
            // Intersection of the extended scrollbar strips, not of their
            // (normally disjoint) rectangles. Works with left-hand scrollbars.
            let corner = intersection(
                RECT {
                    left: v.rect.left,
                    right: v.rect.right,
                    top: h.rect.top,
                    bottom: h.rect.bottom,
                },
                bounds,
            );
            fill(dc, corner, appearance.palette.surface);
        }
    }
}

unsafe fn overlay(hwnd: HWND, state: &State, print_dc: HDC) {
    // SAFETY: retained Rc survives synchronous detach/destruction. No Cell borrow
    // spans FFI, and painting suppresses recursive overlays, not native messages.
    unsafe {
        let mut value = state.get();
        if !value.attached || !value.enabled || value.painting || !supported_window(hwnd) {
            return;
        }
        value.painting = true;
        state.set(value);
        if print_dc.is_null() {
            let dc = GetWindowDC(hwnd);
            if !dc.is_null() {
                let _dc = WindowDc(hwnd, dc);
                draw(hwnd, dc, value);
            }
        } else {
            draw(hwnd, print_dc, value);
        }
        // Do not overwrite colors/enabled/attached changed by a nested callback.
        value = state.get();
        value.painting = false;
        state.set(value);
    }
}

fn repaints_bar(message: u32) -> bool {
    matches!(
        message,
        WM_NCPAINT
            | WM_PAINT
            | WM_VSCROLL
            | WM_HSCROLL
            | WM_SIZE
            | WM_WINDOWPOSCHANGED
            | WM_MOUSEWHEEL
            | WM_MOUSEHWHEEL
            | WM_NCMOUSEMOVE
            | WM_NCMOUSEHOVER
            | WM_NCMOUSELEAVE
            | WM_NCLBUTTONDOWN
            | WM_NCLBUTTONUP
            | WM_NCLBUTTONDBLCLK
            | WM_MOUSEMOVE
            | WM_MOUSELEAVE
            | WM_LBUTTONUP
            | WM_CAPTURECHANGED
            | WM_CANCELMODE
            | WM_TIMER
            | WM_ENABLE
            | WM_SETFOCUS
            | WM_KILLFOCUS
            | WM_KEYDOWN
            | WM_KEYUP
            | WM_CHAR
            | WM_SETTEXT
            | WM_CUT
            | WM_PASTE
            | WM_CLEAR
            | WM_UNDO
            | WM_STYLECHANGED
            | WM_THEMECHANGED
            | WM_SYSCOLORCHANGE
            | WM_DPICHANGED
            | WM_DPICHANGED_AFTERPARENT
            | WM_SHOWWINDOW
            | EM_SETSEL
            | EM_REPLACESEL
            | EM_LINESCROLL
            | EM_SCROLL
            | EM_SCROLLCARET
            | LVM_SCROLL
            | LVM_ENSUREVISIBLE
            | LVM_INSERTITEMW
            | LVM_DELETEITEM
            | LVM_DELETEALLITEMS
            | LVM_SETITEMCOUNT
            | LVM_INSERTCOLUMNW
            | LVM_DELETECOLUMN
            | LVM_SETCOLUMNWIDTH
    )
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: subclass owns one Rc, retained BEFORE default processing; another
    // subclass/parent notification may synchronously destroy or untheme the HWND.
    unsafe {
        let pointer = data as *const Cell<Appearance>;
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        if message == WM_NCDESTROY {
            let mut value = state.get();
            value.attached = false;
            value.enabled = false;
            state.set(value);
            RemoveWindowSubclass(hwnd, Some(procedure), id);
            drop(Rc::from_raw(pointer));
            return DefSubclassProc(hwnd, message, w, l);
        }
        // Native WM_PRINT can change its caller's DC. Restore its original
        // window-coordinate mapping/clip before our non-client-only pass, then
        // draw() separately saves/restores all overlay changes.
        let print = message == WM_PRINT && w != 0 && l & PRF_NONCLIENT as LPARAM != 0;
        let saved = if print { SavedDc::new(w as HDC) } else { None };
        let can_print = saved.is_some();
        let result = DefSubclassProc(hwnd, message, w, l);
        drop(saved);
        if !state.get().attached || IsWindow(hwnd) == 0 {
            return result;
        }
        if print {
            if can_print && (l & PRF_CHECKVISIBLE as LPARAM == 0 || IsWindowVisible(hwnd) != 0) {
                overlay(hwnd, &state, w as HDC);
            }
        } else if repaints_bar(message) {
            overlay(hwnd, &state, std::ptr::null_mut());
        }
        // WM_PRINTCLIENT is intentionally forwarded without any NC overlay.
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null_mut;
    use windows_sys::Win32::System::Threading::{
        GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
    };

    // Public SDK APIs behind unused windows-sys feature gates.
    #[link(name = "user32")]
    unsafe extern "system" {
        fn PrintWindow(hwnd: HWND, dc: HDC, flags: u32) -> i32;
        fn EnableScrollBar(hwnd: HWND, flags: u32, arrows: u32) -> i32;
        fn GetFocus() -> HWND;
        fn SetFocus(hwnd: HWND) -> HWND;
    }

    fn palette() -> Palette {
        Palette {
            background: 0x24201e,
            surface: 0x322c29,
            text: 0xf5f2ee,
            disabled: 0xb2aaa2,
            accent: 0xffc078,
            border: 0x574e47,
            accent_surface: 0x543e29,
        }
    }
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: test-owned HWND, children are destroyed with their parent.
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    unsafe fn window(class: &str, style: u32, parent: HWND) -> HWND {
        // SAFETY: temporary native windows on this thread; no application state.
        unsafe {
            let class: Vec<u16> = class.encode_utf16().chain(Some(0)).collect();
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                style,
                if parent.is_null() { 173 } else { 7 },
                if parent.is_null() { 127 } else { 11 },
                if parent.is_null() { 700 } else { 360 },
                if parent.is_null() { 500 } else { 240 },
                parent,
                null_mut(),
                null_mut(),
                null(),
            );
            assert!(!hwnd.is_null());
            hwnd
        }
    }
    unsafe fn edit(parent: HWND) -> HWND {
        // SAFETY: synchronous messages copy text; no clipboard is modified.
        unsafe {
            let hwnd = window(
                "EDIT",
                WS_CHILD
                    | WS_VISIBLE
                    | WS_BORDER
                    | WS_VSCROLL
                    | WS_HSCROLL
                    | (ES_MULTILINE | ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_READONLY) as u32,
                parent,
            );
            let text = (0..160)
                .map(|n| format!("Line {n:03} {}\r\n", "native selection ".repeat(18)))
                .collect::<String>();
            let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            SendMessageW(hwnd, WM_SETTEXT, 0, text.as_ptr() as LPARAM);
            SendMessageW(hwnd, EM_SETSEL, 7, 29);
            hwnd
        }
    }
    unsafe fn scroll(hwnd: HWND, axis: i32) -> (i32, i32, u32, i32, i32) {
        // SAFETY: correctly sized output, standard non-client scrollbar.
        unsafe {
            let mut value = SCROLLINFO {
                cbSize: size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..Default::default()
            };
            assert_ne!(GetScrollInfo(hwnd, axis, &mut value), 0);
            (
                value.nMin,
                value.nMax,
                value.nPage,
                value.nPos,
                value.nTrackPos,
            )
        }
    }
    unsafe fn native_bar(hwnd: HWND, vertical: bool) -> SCROLLBARINFO {
        // SAFETY: correctly sized output, public standard bar object ID.
        unsafe {
            let mut value = SCROLLBARINFO {
                cbSize: size_of::<SCROLLBARINFO>() as u32,
                ..Default::default()
            };
            assert_ne!(
                GetScrollBarInfo(
                    hwnd,
                    if vertical {
                        OBJID_VSCROLL
                    } else {
                        OBJID_HSCROLL
                    },
                    &mut value
                ),
                0
            );
            value
        }
    }
    unsafe fn selection(hwnd: HWND) -> (u32, u32) {
        // SAFETY: stack outputs live for synchronous EM_GETSEL.
        unsafe {
            let (mut start, mut end) = (0u32, 0u32);
            SendMessageW(
                hwnd,
                EM_GETSEL,
                (&mut start as *mut u32) as WPARAM,
                (&mut end as *mut u32) as LPARAM,
            );
            (start, end)
        }
    }
    struct BitmapDc {
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
    }
    impl BitmapDc {
        unsafe fn new() -> Self {
            // SAFETY: scratch bitmap/DC, restored and released by Drop.
            unsafe {
                let screen = GetDC(null_mut());
                let dc = CreateCompatibleDC(screen);
                let bitmap = CreateCompatibleBitmap(screen, 800, 600);
                ReleaseDC(null_mut(), screen);
                assert!(!dc.is_null() && !bitmap.is_null());
                let old = SelectObject(dc, bitmap as HGDIOBJ);
                Self { dc, bitmap, old }
            }
        }
    }
    impl Drop for BitmapDc {
        fn drop(&mut self) {
            // SAFETY: unselect before deleting the owned bitmap/DC.
            unsafe {
                SelectObject(self.dc, self.old);
                DeleteObject(self.bitmap as HGDIOBJ);
                DeleteDC(self.dc);
            }
        }
    }

    #[test]
    fn class_filter_and_native_state_colors() {
        assert!(supported("EDIT", ES_MULTILINE as u32 | WS_VSCROLL));
        assert!(supported("Edit", ES_MULTILINE as u32 | WS_HSCROLL));
        assert!(supported("SysListView32", 0));
        for (class, style) in [
            ("Edit", WS_VSCROLL),
            ("Edit", ES_MULTILINE as u32),
            ("ComboLBox", WS_VSCROLL),
            ("SCROLLBAR", 0),
            ("AIRelay.PageViewport", WS_VSCROLL),
        ] {
            assert!(!supported(class, style));
        }
        let p = palette();
        assert_eq!(component_color(STATE_SYSTEM_UNAVAILABLE, true, p), p.border);
        assert_eq!(component_color(STATE_SYSTEM_PRESSED, true, p), p.accent);
        assert_eq!(component_color(0, false, p), p.border);
    }

    #[test]
    fn native_edit_state_detach_print_and_600_gdi_passes() {
        // SAFETY: test owns all windows/DCs; no user data, network or clipboard.
        unsafe {
            let parent = Window(window("STATIC", WS_POPUP | WS_VISIBLE, null_mut()));
            let hwnd = edit(parent.0);
            let bitmap = BitmapDc::new();
            let p = palette();
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let text_len = SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0);
            SetFocus(hwnd);
            let focus = GetFocus();
            let sel = selection(hwnd);
            let before = [scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)];
            for dpi in [96, 144, 192] {
                for kind in [
                    Resolved::Dark,
                    Resolved::Light,
                    Resolved::Dark,
                    Resolved::Contrast,
                ] {
                    apply(hwnd, kind, p, dpi);
                    assert_eq!(state(hwnd).is_some(), kind == Resolved::Dark);
                    assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), style);
                    assert_eq!(GetWindowLongPtrW(hwnd, GWL_EXSTYLE), ex_style);
                    assert_eq!([scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)], before);
                    assert_eq!(selection(hwnd), sel);
                    assert_eq!(GetFocus(), focus);
                    assert_eq!(SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0), text_len);
                }
            }
            apply(hwnd, Resolved::Dark, p, 96);
            let info = native_bar(hwnd, true);
            assert_eq!(
                info.rgstate[0] & (STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN),
                0
            );
            let mut wr = RECT::default();
            GetWindowRect(hwnd, &mut wr);
            let x = info.rcScrollBar.left - wr.left + 1;
            let y = info.rcScrollBar.top - wr.top + info.dxyLineButton + 2;
            let sentinel = 0x123456;
            for kind in [
                Resolved::Dark,
                Resolved::Light,
                Resolved::Contrast,
                Resolved::Dark,
            ] {
                apply(hwnd, kind, p, 96);
                SetPixel(bitmap.dc, x, y, sentinel);
                let printed = PrintWindow(hwnd, bitmap.dc, 0);
                if printed == 0 {
                    println!(
                        "PrintWindow unavailable for test child (error {}); testing explicit WM_PRINT/PRF_NONCLIENT instead",
                        windows_sys::Win32::Foundation::GetLastError()
                    );
                    SendMessageW(
                        hwnd,
                        WM_PRINT,
                        bitmap.dc as WPARAM,
                        (PRF_NONCLIENT | PRF_CLIENT) as LPARAM,
                    );
                }
                if kind == Resolved::Dark {
                    assert_eq!(
                        GetPixel(bitmap.dc, x, y),
                        p.background,
                        "PrintWindow missed NC overlay"
                    );
                } else {
                    assert_ne!(
                        GetPixel(bitmap.dc, x, y),
                        p.background,
                        "native frame was not restored"
                    );
                }
            }
            EnableScrollBar(hwnd, SB_BOTH as u32, ESB_DISABLE_BOTH);
            let disabled = native_bar(hwnd, true);
            assert_ne!(
                (disabled.rgstate[0] | disabled.rgstate[3]) & STATE_SYSTEM_UNAVAILABLE,
                0
            );
            SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
            let thumb_x = (info.rcScrollBar.left + info.rcScrollBar.right) / 2 - wr.left;
            let thumb_y =
                info.rcScrollBar.top - wr.top + (info.xyThumbTop + info.xyThumbBottom) / 2;
            assert_eq!(GetPixel(bitmap.dc, thumb_x, thumb_y), p.border);
            EnableScrollBar(hwnd, SB_BOTH as u32, ESB_ENABLE_BOTH);
            for message in [WM_PRINT, WM_PRINTCLIENT] {
                SetPixel(bitmap.dc, x, y, sentinel);
                let saved = SaveDC(bitmap.dc);
                // Isolate the NC part of this caller's bitmap for PRINTCLIENT.
                IntersectClipRect(bitmap.dc, x, y, x + 1, y + 1);
                SendMessageW(hwnd, message, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
                RestoreDC(bitmap.dc, saved);
                assert_eq!(
                    GetPixel(bitmap.dc, x, y),
                    if message == WM_PRINT {
                        p.background
                    } else {
                        sentinel
                    }
                );
            }
            let h = native_bar(hwnd, false);
            let cx = info.rcScrollBar.left - wr.left + 1;
            let cy = h.rcScrollBar.top - wr.top + 1;
            SendMessageW(
                hwnd,
                WM_PRINT,
                bitmap.dc as WPARAM,
                (PRF_NONCLIENT | PRF_CLIENT) as LPARAM,
            );
            assert_eq!(
                GetPixel(bitmap.dc, cx, cy),
                p.surface,
                "corner remains white"
            );
            let old_color = SetTextColor(bitmap.dc, sentinel);
            let process = GetCurrentProcess();
            let before_gdi = GetGuiResources(process, GR_GDIOBJECTS);
            for n in 0..600 {
                SendMessageW(
                    hwnd,
                    WM_PRINT,
                    bitmap.dc as WPARAM,
                    (PRF_NONCLIENT | PRF_CLIENT) as LPARAM,
                );
                if n % 3 == 0 {
                    apply(hwnd, Resolved::Light, p, 96);
                    apply(hwnd, Resolved::Dark, p, 96);
                }
            }
            let after_gdi = GetGuiResources(process, GR_GDIOBJECTS);
            assert_eq!(
                before_gdi, after_gdi,
                "600 paint/theme passes leaked GDI resources"
            );
            assert_eq!(GetTextColor(bitmap.dc), sentinel);
            SetTextColor(bitmap.dc, old_color);
            assert_eq!([scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)], before);
            assert_eq!(selection(hwnd), sel);
            println!("600 native scrollbar print/theme passes: GDI {before_gdi} -> {after_gdi}");
            InvalidateRect(hwnd, null(), 0);
            SendMessageW(hwnd, WM_PAINT, 0, 0);
            assert_eq!(GetUpdateRect(hwnd, null_mut(), 0), 0);
            let dc = GetWindowDC(hwnd);
            assert_eq!(
                GetPixel(dc, x, y),
                p.background,
                "WM_PAINT overwrote the NC overlay"
            );
            ReleaseDC(hwnd, dc);
            let retained = state(hwnd).unwrap();
            DestroyWindow(hwnd);
            assert!(!retained.get().attached);
            apply(null_mut(), Resolved::Dark, p, 96);
            apply(hwnd, Resolved::Dark, p, 96);
        }
    }

    #[test]
    fn native_bar_offsets_follow_both_axes_and_native_scroll_input() {
        // SAFETY: test-owned Edit, standard native scroll messages only.
        unsafe {
            let parent = Window(window("STATIC", WS_POPUP | WS_VISIBLE, null_mut()));
            let hwnd = edit(parent.0);
            apply(hwnd, Resolved::Dark, palette(), 192);
            let sel = selection(hwnd);
            for (vertical, message) in [(true, WM_VSCROLL), (false, WM_HSCROLL)] {
                SendMessageW(hwnd, message, SB_TOP as WPARAM, 0);
                let top = native_bar(hwnd, vertical);
                let length = if vertical {
                    top.rcScrollBar.bottom - top.rcScrollBar.top
                } else {
                    top.rcScrollBar.right - top.rcScrollBar.left
                };
                assert!(top.rcScrollBar.top > 0 && top.rcScrollBar.left > 0);
                assert!(
                    (top.xyThumbTop - top.dxyLineButton).abs() <= 1,
                    "thumb start is bar-relative"
                );
                assert!(top.xyThumbBottom > top.xyThumbTop && top.xyThumbBottom <= length);
                SendMessageW(hwnd, message, SB_BOTTOM as WPARAM, 0);
                let bottom = native_bar(hwnd, vertical);
                assert!(bottom.xyThumbTop > top.xyThumbTop);
                assert!(
                    (bottom.xyThumbBottom - (length - bottom.dxyLineButton)).abs() <= 1,
                    "thumb end is bar-relative"
                );
            }
            assert_eq!(selection(hwnd), sel);
            let before = scroll(hwnd, SB_VERT).3;
            SendMessageW(hwnd, WM_MOUSEWHEEL, (120u32 << 16) as WPARAM, 0);
            assert!(scroll(hwnd, SB_VERT).3 < before);
            let current = scroll(hwnd, SB_VERT);
            apply(hwnd, Resolved::Light, palette(), 96);
            assert_eq!(scroll(hwnd, SB_VERT), current);
        }
    }

    unsafe extern "system" fn destroy_during_default(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: test-only subclass; data selects a synchronous lifetime probe.
        unsafe {
            if message == WM_SETTEXT {
                if data == 1 {
                    DestroyWindow(hwnd);
                    return 0;
                }
                apply(hwnd, Resolved::Light, palette(), 96);
            }
            if message == WM_NCDESTROY {
                RemoveWindowSubclass(hwnd, Some(destroy_during_default), id);
            }
            DefSubclassProc(hwnd, message, w, l)
        }
    }

    #[test]
    fn reentrant_native_destroy_and_detach_keep_state_alive() {
        // SAFETY: private native children and test-only lifetime probes.
        unsafe {
            let parent = Window(window("STATIC", WS_POPUP, null_mut()));
            for action in [1, 2] {
                let hwnd = edit(parent.0);
                assert_ne!(
                    SetWindowSubclass(hwnd, Some(destroy_during_default), SUBCLASS_ID + 1, action),
                    0
                );
                apply(hwnd, Resolved::Dark, palette(), 96);
                let retained = state(hwnd).unwrap();
                let text = [b'N' as u16, 0];
                SendMessageW(hwnd, WM_SETTEXT, 0, text.as_ptr() as LPARAM);
                assert!(!retained.get().attached);
                if action == 2 {
                    assert!(state(hwnd).is_none());
                    assert_eq!(SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0), 1);
                } else {
                    assert_eq!(IsWindow(hwnd), 0);
                }
            }
        }
    }

    #[test]
    fn native_listview_and_invisible_bars() {
        // SAFETY: test-owned common controls, no application/controller state.
        unsafe {
            let init = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            assert_ne!(InitCommonControlsEx(&init), 0);
            let parent = Window(window("STATIC", WS_POPUP | WS_VISIBLE, null_mut()));
            let hwnd = window(
                "SysListView32",
                WS_CHILD | WS_VISIBLE | LVS_REPORT,
                parent.0,
            );
            let mut column = LVCOLUMNW {
                mask: LVCF_WIDTH,
                cx: 1000,
                ..Default::default()
            };
            SendMessageW(
                hwnd,
                LVM_INSERTCOLUMNW,
                0,
                (&mut column as *mut LVCOLUMNW) as LPARAM,
            );
            for i in 0..100 {
                let mut item = LVITEMW {
                    iItem: i,
                    ..Default::default()
                };
                assert_eq!(
                    SendMessageW(
                        hwnd,
                        LVM_INSERTITEMW,
                        0,
                        (&mut item as *mut LVITEMW) as LPARAM
                    ),
                    i as LRESULT
                );
            }
            let before = [scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)];
            apply(hwnd, Resolved::Dark, palette(), 144);
            assert!(state(hwnd).is_some());
            assert_eq!([scroll(hwnd, SB_VERT), scroll(hwnd, SB_HORZ)], before);
            apply(hwnd, Resolved::Contrast, palette(), 144);
            assert!(state(hwnd).is_none());
            let edit = window(
                "EDIT",
                WS_CHILD | ES_MULTILINE as u32 | WS_VSCROLL,
                parent.0,
            );
            // A hidden HWND is not necessarily marked OFFSCREEN by MSAA. Hide
            // the actual bar explicitly in the fixture, not in production code.
            assert_ne!(ShowScrollBar(edit, SB_VERT, 0), 0);
            let info = native_bar(edit, true);
            assert_ne!(
                info.rgstate[0] & (STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN),
                0
            );
            let origin = POINT::default();
            let bounds = RECT {
                left: 0,
                top: 0,
                right: 10000,
                bottom: 10000,
            };
            assert!(bar(edit, true, origin, bounds).is_none());
            apply(parent.0, Resolved::Dark, palette(), 96);
            assert!(state(parent.0).is_none());
        }
    }

    #[test]
    fn slim_thumb_is_narrow_centered_and_keeps_native_track() {
        let bar = |vertical| Bar {
            rect: RECT {
                left: 0,
                top: 0,
                right: 200,
                bottom: 200,
            },
            vertical,
            button: 17,
            thumb_top: 30,
            thumb_bottom: 90,
            states: [0; 6],
            scrollable: true,
        };
        let vertical = slim_thumb(bar(true), 96).unwrap();
        assert_eq!(vertical.right - vertical.left, scale(6, 96));
        assert_eq!((vertical.left + vertical.right) / 2, 100);
        assert_eq!(vertical.top, 30);
        let horizontal = slim_thumb(
            Bar {
                thumb_top: 80,
                thumb_bottom: 120,
                ..bar(false)
            },
            192,
        )
        .unwrap();
        assert_eq!(horizontal.bottom - horizontal.top, scale(6, 192));
        assert_eq!((horizontal.top + horizontal.bottom) / 2, 100);
        assert_eq!(horizontal.left, 80);
        assert_eq!(horizontal.right, 120);
        let mut hidden = bar(true);
        hidden.states[3] = STATE_SYSTEM_INVISIBLE;
        assert!(slim_thumb(hidden, 96).is_none());
        let mut fixed = bar(true);
        fixed.scrollable = false;
        assert!(slim_thumb(fixed, 96).is_none());
    }
}
