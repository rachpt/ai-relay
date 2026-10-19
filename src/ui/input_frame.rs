//! Rectangular non-client frames for native Edit controls; never a client overlay.
//! Call after native theme selection and before rounded/scrollbar presentation.
//! Styles, geometry, regions, text, caret, selection, IME and input remain native.
//! High contrast restores the native theme and removes only this module's subclass.
#![cfg(windows)]

use super::theme::{Palette, Resolved, blend};
use crate::windows::scale;
use std::{
    cell::Cell,
    ptr::{null, null_mut},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::Threading::GetCurrentThreadId,
    UI::{
        Controls::{EM_SETRECT, EM_SETRECTNP, SetWindowTheme},
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::*,
    },
};

const SUBCLASS_ID: usize = 0x494e_4652;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn GetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: *mut usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}
// Documented user32 APIs whose bindings require an otherwise unused feature.
#[link(name = "user32")]
unsafe extern "system" {
    fn GetFocus() -> HWND;
    fn IsWindowEnabled(hwnd: HWND) -> i32;
}

#[derive(Clone, Copy)]
struct Appearance {
    palette: Palette,
    dpi: u32,
    attached: bool,
    active: bool,
    changing_theme: bool,
    painting: bool,
    printing: bool,
    redraw: bool,
    defer_paint: u32,
}
type State = Rc<Cell<Appearance>>;

struct PaintDeferral(State);
impl PaintDeferral {
    fn new(state: &State) -> Self {
        let mut value = state.get();
        value.defer_paint = value.defer_paint.saturating_add(1);
        state.set(value);
        Self(Rc::clone(state))
    }
}
impl Drop for PaintDeferral {
    fn drop(&mut self) {
        let mut value = self.0.get();
        value.defer_paint = value.defer_paint.saturating_sub(1);
        self.0.set(value);
    }
}

fn supported(class: &str, style: u32, extended: u32) -> bool {
    class.eq_ignore_ascii_case("Edit")
        && (style & WS_BORDER != 0 || extended & WS_EX_CLIENTEDGE != 0)
}

unsafe fn supported_window(hwnd: HWND) -> bool {
    // SAFETY: Owner-thread HWND queries; the native class name buffer is bounded.
    unsafe {
        let mut name = [0u16; 32];
        let count = GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32);
        count > 0
            && supported(
                &String::from_utf16_lossy(&name[..count as usize]),
                GetWindowLongPtrW(hwnd, GWL_STYLE) as u32,
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32,
            )
    }
}

unsafe fn state(hwnd: HWND) -> Option<State> {
    // SAFETY: Only this proc/id owns this data. Retain before any native call that
    // can send messages, detach the subclass, or synchronously destroy the HWND.
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

unsafe fn request_frame(hwnd: HWND) {
    // SAFETY: Attached owner-thread HWND; only queue NC damage, never paint in a
    // geometry/no-redraw transaction. Normalize both actual client corners, also
    // for mirrored windows, then express the outer bounds in client coordinates.
    unsafe {
        let mut window = RECT::default();
        let mut client = RECT::default();
        if GetWindowRect(hwnd, &mut window) == 0 || GetClientRect(hwnd, &mut client) == 0 {
            return;
        }
        let mut first = POINT {
            x: client.left,
            y: client.top,
        };
        let mut last = POINT {
            x: client.right,
            y: client.bottom,
        };
        if ClientToScreen(hwnd, &mut first) == 0 || ClientToScreen(hwnd, &mut last) == 0 {
            return;
        }
        let x = first.x.min(last.x);
        let y = first.y.min(last.y);
        let bounds = RECT {
            left: window.left.saturating_sub(x),
            top: window.top.saturating_sub(y),
            right: window.right.saturating_sub(x),
            bottom: window.bottom.saturating_sub(y),
        };
        // A null rectangle would invalidate the entire client. Four NC bands
        // preserve EM_SETRECTNP/SWP_NOREDRAW without allocating a window region.
        for rect in [
            RECT {
                bottom: client.top,
                ..bounds
            },
            RECT {
                top: client.bottom,
                ..bounds
            },
            RECT {
                right: client.left,
                top: client.top,
                bottom: client.bottom,
                ..bounds
            },
            RECT {
                left: client.right,
                top: client.top,
                bottom: client.bottom,
                ..bounds
            },
        ] {
            if rect.left < rect.right && rect.top < rect.bottom {
                RedrawWindow(
                    hwnd,
                    &rect,
                    null_mut(),
                    RDW_INVALIDATE | RDW_FRAME | RDW_NOCHILDREN,
                );
            }
        }
    }
}

unsafe fn synchronize_theme(hwnd: HWND, state: &State) {
    // SAFETY: Retained owner-thread Rc, with only copied Cell values across FFI.
    // SetWindowTheme sends WM_THEMECHANGED synchronously; nested apply/destroy
    // requests are observed before repaint/removal, never overwritten by old data.
    unsafe {
        let mut value = state.get();
        if !value.attached {
            return;
        }
        if value.changing_theme {
            return;
        }
        let custom = value.active;
        value.changing_theme = true;
        state.set(value);
        let empty = [0u16];
        let theme = if custom { empty.as_ptr() } else { null() };
        let result = SetWindowTheme(hwnd, theme, theme);

        value = state.get();
        value.changing_theme = false;
        state.set(value);
        if !value.attached {
            return;
        }
        if value.active != custom {
            synchronize_theme(hwnd, state);
            return;
        }
        if result < 0 {
            if custom {
                value.active = false;
                state.set(value);
                synchronize_theme(hwnd, state);
            }
            // Failed restoration leaves an inactive owner installed for retry or
            // NCDESTROY. In particular, do not claim ownership was removed.
            return;
        }
        request_frame(hwnd);
        value = state.get();
        if !value.attached {
            return;
        }
        if !custom && !value.active && RemoveWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID) != 0
        {
            value.attached = false;
            state.set(value);
            // Removal succeeded: release the installed owner, not the local Rc.
            // On failure the inactive owner must survive until retry/destruction.
            drop(Rc::from_raw(Rc::as_ptr(state)));
        }
    }
}

/// Apply a thin, rectangular native Edit frame without modifying styles/regions.
/// Native theme selection must precede this call. Unsupported handles are ignored.
///
/// # Safety
/// Call on the HWND's owner UI thread with high contrast resolved to `Contrast`.
/// `WM_PRINT`/`WM_PRINTCLIENT` callers retain their normal live-DC contracts. No
/// application state, asynchronous raw reference, brush or region is retained.
pub(super) unsafe fn apply(hwnd: HWND, kind: Resolved, palette: Palette, dpi: u32) {
    // SAFETY: Confirm the HWND's owner thread before querying subclass data. A
    // local Rc survives SetWindowTheme and any resulting synchronous callbacks.
    unsafe {
        if hwnd.is_null() || GetWindowThreadProcessId(hwnd, null_mut()) != GetCurrentThreadId() {
            return;
        }
        let previous = state(hwnd);
        let active = kind != Resolved::Contrast && supported_window(hwnd);
        let state = if let Some(state) = previous {
            let mut value = state.get();
            value.palette = palette;
            value.dpi = dpi.clamp(96, 768);
            value.active = active;
            state.set(value);
            state
        } else {
            if !active {
                return;
            }
            let state = Rc::new(Cell::new(Appearance {
                palette,
                dpi: dpi.clamp(96, 768),
                attached: true,
                active: true,
                changing_theme: false,
                painting: false,
                printing: false,
                redraw: true,
                defer_paint: 0,
            }));
            let owner = Rc::into_raw(Rc::clone(&state));
            if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, owner as usize) == 0 {
                drop(Rc::from_raw(owner));
                return;
            }
            state
        };
        synchronize_theme(hwnd, &state);
    }
}

struct SavedDc(HDC, i32);
impl SavedDc {
    unsafe fn new(dc: HDC) -> Option<Self> {
        // SAFETY: A live borrowed DC; no changes are permitted if SaveDC fails.
        let saved = unsafe { SaveDC(dc) };
        (saved != 0).then_some(Self(dc, saved))
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: This guard owns one successful save; its DC remains live.
        unsafe {
            RestoreDC(self.0, self.1);
        }
    }
}
struct WindowDc(HWND, HDC);
impl Drop for WindowDc {
    fn drop(&mut self) {
        // SAFETY: Paired with GetWindowDC; borrowed window DCs use ReleaseDC.
        unsafe {
            ReleaseDC(self.0, self.1);
        }
    }
}

#[derive(Clone, Copy)]
struct Frame {
    bounds: RECT,
    client: RECT,
    inset: i32,
}
impl Frame {
    fn new(bounds: RECT, client: RECT) -> Option<Self> {
        if bounds.left >= bounds.right
            || bounds.top >= bounds.bottom
            || client.left > client.right
            || client.top > client.bottom
            || client.left < bounds.left
            || client.top < bounds.top
            || client.right > bounds.right
            || client.bottom > bounds.bottom
        {
            return None;
        }
        // A conservative complete ring cannot reach a scrollbar, even with RTL
        // bars. Never invent a border from a style bit or force a zero inset to 1.
        let inset = (client.left - bounds.left)
            .min(client.top - bounds.top)
            .min(bounds.right - client.right)
            .min(bounds.bottom - client.bottom);
        (inset > 0).then_some(Self {
            bounds,
            client,
            inset,
        })
    }
}

unsafe fn frame(hwnd: HWND) -> Option<Frame> {
    // SAFETY: Queries only. Convert BOTH actual client corners to screen/window
    // coordinates, normalize mirroring, and preserve even a one-pixel client.
    unsafe {
        let mut window = RECT::default();
        let mut client = RECT::default();
        if GetWindowRect(hwnd, &mut window) == 0 || GetClientRect(hwnd, &mut client) == 0 {
            return None;
        }
        let mut first = POINT {
            x: client.left,
            y: client.top,
        };
        let mut last = POINT {
            x: client.right,
            y: client.bottom,
        };
        if ClientToScreen(hwnd, &mut first) == 0 || ClientToScreen(hwnd, &mut last) == 0 {
            return None;
        }
        Frame::new(
            RECT {
                left: 0,
                top: 0,
                right: window.right.saturating_sub(window.left),
                bottom: window.bottom.saturating_sub(window.top),
            },
            RECT {
                left: first.x.min(last.x).saturating_sub(window.left),
                top: first.y.min(last.y).saturating_sub(window.top),
                right: first.x.max(last.x).saturating_sub(window.left),
                bottom: first.y.max(last.y).saturating_sub(window.top),
            },
        )
    }
}

fn border_color(palette: Palette, enabled: bool, focused: bool) -> u32 {
    if !enabled {
        blend(palette.border, palette.surface, 50)
    } else if focused {
        palette.accent
    } else {
        palette.border
    }
}

unsafe fn ring(dc: HDC, bounds: RECT, width: i32, color: u32) {
    // SAFETY: Live saved DC with the ACTUAL client already excluded. Stock brush
    // is borrowed, and four rectangular bands avoid pen endpoint ambiguities.
    unsafe {
        SetDCBrushColor(dc, color);
        for rect in [
            RECT {
                bottom: bounds.top + width,
                ..bounds
            },
            RECT {
                top: bounds.bottom - width,
                ..bounds
            },
            RECT {
                right: bounds.left + width,
                top: bounds.top + width,
                bottom: bounds.bottom - width,
                ..bounds
            },
            RECT {
                left: bounds.right - width,
                top: bounds.top + width,
                bottom: bounds.bottom - width,
                ..bounds
            },
        ] {
            if rect.left < rect.right && rect.top < rect.bottom {
                FillRect(dc, &rect, GetStockObject(DC_BRUSH));
            }
        }
    }
}

unsafe fn draw(dc: HDC, frame: Frame, value: Appearance, enabled: bool, focused: bool) {
    // SAFETY: Window-coordinate live DC. Clip to the outer rectangle and exclude
    // the entire real client BEFORE ANY drawing, irrespective of client size.
    unsafe {
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        let bounds = frame.bounds;
        let client = frame.client;
        if IntersectClipRect(dc, bounds.left, bounds.top, bounds.right, bounds.bottom) == ERROR
            || ExcludeClipRect(dc, client.left, client.top, client.right, client.bottom) == ERROR
        {
            return;
        }
        // Erase only the native frame's bevel, not the remaining non-client area
        // (notably scrollbars). One DIP of that existing ring is the visible edge.
        ring(dc, bounds, frame.inset, value.palette.surface);
        ring(
            dc,
            bounds,
            scale(1, value.dpi).min(frame.inset),
            border_color(value.palette, enabled, focused),
        );
    }
}

#[cfg(test)]
std::thread_local! {
    static SCREEN_PASSES: Cell<usize> = const { Cell::new(0) };
    static SCREEN_DRAWS: Cell<usize> = const { Cell::new(0) };
}

unsafe fn overlay(hwnd: HWND, state: &State, print_dc: HDC) {
    // SAFETY: Retained Rc guards reentrant detach/destruction; all native queries
    // use copied values. The painting flag only suppresses our own recursive pass.
    unsafe {
        let mut value = state.get();
        if !value.attached
            || !value.active
            || value.changing_theme
            || value.painting
            || (print_dc.is_null()
                && (value.printing
                    || !value.redraw
                    || value.defer_paint != 0
                    || IsWindowVisible(hwnd) == 0))
            || !supported_window(hwnd)
        {
            return;
        }
        value.painting = true;
        state.set(value);
        let window_dc = if print_dc.is_null() {
            #[cfg(test)]
            SCREEN_PASSES.with(|count| count.set(count.get() + 1));
            let dc = GetWindowDC(hwnd);
            (!dc.is_null()).then_some(WindowDc(hwnd, dc))
        } else {
            None
        };
        let dc = window_dc.as_ref().map_or(print_dc, |window_dc| window_dc.1);
        let current = state.get();
        // Query the actual client AFTER acquiring the window DC, never using
        // cached insets across native messages or a possible size change.
        if !dc.is_null()
            && current.attached
            && current.active
            && !current.changing_theme
            && (!print_dc.is_null()
                || (current.redraw
                    && current.defer_paint == 0
                    && !current.printing
                    && IsWindowVisible(hwnd) != 0))
            && let Some(frame) = frame(hwnd)
        {
            draw(
                dc,
                frame,
                current,
                IsWindowEnabled(hwnd) != 0,
                GetFocus() == hwnd,
            );
            #[cfg(test)]
            if print_dc.is_null() {
                SCREEN_DRAWS.with(|count| count.set(count.get() + 1));
            }
        }
        drop(window_dc);
        // A nested callback may have updated the palette or removed the owner.
        value = state.get();
        value.painting = false;
        state.set(value);
    }
}

fn geometry_message(message: u32) -> bool {
    matches!(
        message,
        WM_MOVE
            | WM_SIZE
            | WM_NCCALCSIZE
            | WM_WINDOWPOSCHANGING
            | WM_WINDOWPOSCHANGED
            | WM_STYLECHANGING
            | WM_STYLECHANGED
            | WM_THEMECHANGED
            | WM_SYSCOLORCHANGE
            | WM_SETTINGCHANGE
            | WM_SHOWWINDOW
            | WM_SETFONT
            | WM_DPICHANGED
            | WM_DPICHANGED_BEFOREPARENT
            | WM_DPICHANGED_AFTERPARENT
            | WM_SETREDRAW
            | EM_SETRECT
            | EM_SETRECTNP
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
    // SAFETY: This owner-thread subclass owns one Rc. Retain it before forwarding
    // ANY message: native processing/other subclasses may destroy or detach us.
    unsafe {
        let pointer = data as *const Cell<Appearance>;
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        if message == WM_NCDESTROY {
            let mut value = state.get();
            if value.attached {
                value.attached = false;
                value.active = false;
                state.set(value);
                RemoveWindowSubclass(hwnd, Some(procedure), id);
                // NCDESTROY ends the installed owner's lifetime even if removal
                // fails; no HWND query or overlay follows native destruction.
                drop(Rc::from_raw(pointer));
            }
            return DefSubclassProc(hwnd, message, w, l);
        }
        // Retain the depth through native forwarding AND theme synchronization:
        // either can reenter WM_PAINT/NCPAINT before geometry has settled. The
        // NCACTIVATE -1 contract also forbids nested screen overlays.
        let geometry = geometry_message(message);
        let no_activate_redraw = message == WM_NCACTIVATE && l == -1;
        let _deferred = (geometry || no_activate_redraw).then(|| PaintDeferral::new(&state));
        if message == WM_SETREDRAW {
            let mut value = state.get();
            value.redraw = w != 0;
            state.set(value);
        }
        let printing = message == WM_PRINT || message == WM_PRINTCLIENT;
        let saved = if printing && w != 0 {
            SavedDc::new(w as HDC)
        } else {
            None
        };
        if printing && saved.is_none() {
            // Do not pass an unprotected/invalid caller DC to native printing.
            return 0;
        }
        let was_printing = state.get().printing;
        if printing {
            let mut value = state.get();
            value.printing = true;
            state.set(value);
        }
        // In particular, native NCPAINT/NCACTIVATE always run BEFORE our frame.
        let result = DefSubclassProc(hwnd, message, w, l);
        drop(saved);
        if printing {
            let mut value = state.get();
            value.printing = was_printing;
            state.set(value);
        }
        if !state.get().attached || no_activate_redraw {
            return result;
        }
        if printing {
            if message == WM_PRINT
                && l & PRF_NONCLIENT as LPARAM != 0
                && (l & PRF_CHECKVISIBLE as LPARAM == 0 || IsWindowVisible(hwnd) != 0)
            {
                // Native printing's mapping/clip has been restored; overlay uses
                // the caller's window-coordinate mapping, then restores it again.
                overlay(hwnd, &state, w as HDC);
            }
            return result;
        }
        if message == WM_THEMECHANGED {
            if !state.get().changing_theme {
                synchronize_theme(hwnd, &state);
            }
            return result;
        }
        if message == WM_STYLECHANGED && !supported_window(hwnd) {
            let mut value = state.get();
            value.active = false;
            state.set(value);
            synchronize_theme(hwnd, &state);
            return result;
        }
        if matches!(message, WM_DPICHANGED | WM_DPICHANGED_AFTERPARENT) {
            let mut value = state.get();
            value.dpi = if message == WM_DPICHANGED {
                (w & 0xffff) as u32
            } else {
                GetDpiForWindow(hwnd)
            }
            .clamp(96, 768);
            state.set(value);
        }
        if state.get().active && !state.get().changing_theme {
            let refresh = matches!(message, WM_SETFOCUS | WM_KILLFOCUS | WM_ENABLE);
            if geometry || refresh {
                request_frame(hwnd);
            }
            if !geometry && (refresh || matches!(message, WM_NCPAINT | WM_NCACTIVATE | WM_PAINT)) {
                overlay(hwnd, &state, null_mut());
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::MoveBatch;
    use super::*;
    use windows_sys::Win32::{
        System::Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources},
        UI::Controls::*,
    };

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetFocus(hwnd: HWND) -> HWND;
        fn EnableWindow(hwnd: HWND, enabled: i32) -> i32;
    }

    fn palette(kind: Resolved) -> Palette {
        if kind == Resolved::Light {
            Palette {
                background: 0xffffff,
                surface: 0xfafafa,
                text: 0x262220,
                disabled: 0x756d66,
                accent: 0xd77800,
                border: 0xe3ded9,
                accent_surface: 0xfcf0e5,
            }
        } else {
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
    }

    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: Only private test-thread windows; parent owns its children.
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    unsafe fn window(class: &str, extended: u32, style: u32, parent: HWND) -> HWND {
        // SAFETY: Isolated native windows, no application data or I/O. Creation
        // is hidden; only dedicated offscreen fixtures opt into showing a parent.
        unsafe {
            let edit = class.eq_ignore_ascii_case("Edit");
            let class: Vec<u16> = class.encode_utf16().chain(Some(0)).collect();
            let text: Vec<u16> = "原生 Edit 😀".encode_utf16().chain(Some(0)).collect();
            let hwnd = CreateWindowExW(
                extended,
                class.as_ptr(),
                text.as_ptr(),
                style,
                13,
                17,
                240,
                60,
                parent,
                null_mut(),
                null_mut(),
                null(),
            );
            assert!(!hwnd.is_null());
            // Single-line native Edit can consume WS_BORDER during creation,
            // leaving no frame at all. Establish a REAL WS_BORDER test fixture
            // before taking snapshots; production apply must never add it.
            if edit
                && style & WS_BORDER != 0
                && GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_BORDER == 0
            {
                let actual = GetWindowLongPtrW(hwnd, GWL_STYLE);
                SetWindowLongPtrW(hwnd, GWL_STYLE, actual | WS_BORDER as isize);
                assert_ne!(
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED
                    ),
                    0
                );
            }
            hwnd
        }
    }

    struct BitmapDc {
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
    }
    impl BitmapDc {
        unsafe fn new() -> Self {
            // SAFETY: Owned memory DC/bitmap; never capture the user's screen.
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let bitmap = CreateBitmap(800, 240, 1, 32, null());
                assert!(!dc.is_null() && !bitmap.is_null());
                let old = SelectObject(dc, bitmap);
                Self { dc, bitmap, old }
            }
        }
    }
    impl Drop for BitmapDc {
        fn drop(&mut self) {
            // SAFETY: Deselect before deleting the owned bitmap and memory DC.
            unsafe {
                SelectObject(self.dc, self.old);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }

    fn rect_tuple(rect: RECT) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }
    unsafe fn no_region(hwnd: HWND, scratch: HRGN) {
        // SAFETY: The test owns the scratch region; never sets the window region.
        unsafe {
            assert_eq!(GetWindowRgn(hwnd, scratch), ERROR);
        }
    }
    unsafe fn fill(dc: HDC, rect: RECT, color: u32) {
        // SAFETY: Test-owned live DC; preserve its brush state around the sentinel.
        unsafe {
            let _saved = SavedDc::new(dc).unwrap();
            SetDCBrushColor(dc, color);
            FillRect(dc, &rect, GetStockObject(DC_BRUSH));
        }
    }
    unsafe fn assert_client(dc: HDC, rect: RECT, color: u32) {
        // SAFETY: Pixels lie within this test's selected bitmap, not a window DC.
        unsafe {
            for y in rect.top..rect.bottom {
                for x in rect.left..rect.right {
                    assert_eq!(GetPixel(dc, x, y), color, "client pixel ({x}, {y}) changed");
                }
            }
        }
    }
    #[derive(Debug, PartialEq, Eq)]
    struct NativeState {
        style: isize,
        extended: isize,
        text: Vec<u16>,
        selection: LRESULT,
        margins: LRESULT,
        password: LRESULT,
        limit: LRESULT,
        dialog_code: LRESULT,
    }
    unsafe fn snapshot(hwnd: HWND) -> NativeState {
        // SAFETY: Live hidden Edit, sized text buffer; synchronous scalar messages.
        unsafe {
            let length = GetWindowTextLengthW(hwnd);
            let mut text = vec![0u16; length as usize + 1];
            assert_eq!(
                GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32),
                length
            );
            NativeState {
                style: GetWindowLongPtrW(hwnd, GWL_STYLE),
                extended: GetWindowLongPtrW(hwnd, GWL_EXSTYLE),
                text,
                selection: SendMessageW(hwnd, EM_GETSEL, 0, 0),
                margins: SendMessageW(hwnd, EM_GETMARGINS, 0, 0),
                password: SendMessageW(hwnd, EM_GETPASSWORDCHAR, 0, 0),
                limit: SendMessageW(hwnd, EM_GETLIMITTEXT, 0, 0),
                dialog_code: SendMessageW(hwnd, WM_GETDLGCODE, 0, 0),
            }
        }
    }

    fn screen_activity() -> (usize, usize) {
        (SCREEN_PASSES.with(Cell::get), SCREEN_DRAWS.with(Cell::get))
    }

    unsafe fn clear_damage(hwnd: HWND) {
        // SAFETY: Private test HWND; clear both client and NC damage so a later
        // UpdateWindow cannot pass using stale creation/theme/show invalidation.
        unsafe {
            assert_ne!(
                RedrawWindow(
                    hwnd,
                    null(),
                    null_mut(),
                    RDW_VALIDATE | RDW_NOFRAME | RDW_NOERASE | RDW_NOINTERNALPAINT | RDW_NOCHILDREN,
                ),
                0
            );
            assert_eq!(GetUpdateRect(hwnd, null_mut(), 0), 0);
        }
    }

    unsafe fn offscreen_parent() -> Window {
        // SAFETY: Test-owned popup outside the virtual desktop; never activate it
        // or sample screen pixels. The caller may show it without exposing UI.
        unsafe {
            let parent = Window(window(
                "STATIC",
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                WS_POPUP | WS_CLIPCHILDREN,
                null_mut(),
            ));
            assert_ne!(
                SetWindowPos(
                    parent.0,
                    null_mut(),
                    GetSystemMetrics(SM_XVIRTUALSCREEN).saturating_sub(1024),
                    GetSystemMetrics(SM_YVIRTUALSCREEN).saturating_sub(1024),
                    512,
                    320,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
                ),
                0
            );
            parent
        }
    }

    unsafe fn clientedge_edit(parent: HWND) -> HWND {
        // SAFETY: Establish the real NC area at creation, not by changing styles
        // in apply. Multiline Edit also exercises native EM_SETRECT/SETRECTNP.
        unsafe {
            let hwnd = window(
                "EDIT",
                WS_EX_CLIENTEDGE,
                WS_CHILD | WS_VISIBLE | ES_MULTILINE as u32,
                parent,
            );
            assert_ne!(
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_CLIENTEDGE,
                0
            );
            assert!(supported_window(hwnd));
            assert!(frame(hwnd).is_some());
            hwnd
        }
    }

    unsafe fn framed_window(shown: bool) -> (Window, HWND) {
        // SAFETY: Isolated HWNDs, no application state, message loop or I/O.
        unsafe {
            let parent = offscreen_parent();
            let hwnd = clientedge_edit(parent.0);
            apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
            assert!(state(hwnd).unwrap().get().active);
            assert!(frame(hwnd).is_some());
            if shown {
                ShowWindow(parent.0, SW_SHOWNOACTIVATE);
            }
            assert_eq!(IsWindowVisible(hwnd) != 0, shown);
            let mut bounds = RECT::default();
            assert_ne!(GetWindowRect(parent.0, &mut bounds), 0);
            assert!(bounds.right <= GetSystemMetrics(SM_XVIRTUALSCREEN));
            assert!(bounds.bottom <= GetSystemMetrics(SM_YVIRTUALSCREEN));
            clear_damage(hwnd);
            (parent, hwnd)
        }
    }

    unsafe fn native_window() -> (Window, HWND) {
        // SAFETY: Match the framed fixture's real NC geometry and native theme,
        // without installing input_frame or loading application state.
        unsafe {
            let parent = offscreen_parent();
            let hwnd = native_clientedge_edit(parent.0);
            ShowWindow(parent.0, SW_SHOWNOACTIVATE);
            assert_ne!(IsWindowVisible(hwnd), 0);
            clear_damage(hwnd);
            (parent, hwnd)
        }
    }

    unsafe fn native_clientedge_edit(parent: HWND) -> HWND {
        // SAFETY: Private child and synchronous public theme API; no subclass is
        // installed, so client invalidation comes only from the native control.
        unsafe {
            let hwnd = clientedge_edit(parent);
            let empty = [0u16];
            assert!(SetWindowTheme(hwnd, empty.as_ptr(), empty.as_ptr()) >= 0);
            assert!(state(hwnd).is_none());
            assert!(frame(hwnd).is_some());
            hwnd
        }
    }

    struct Region(HRGN);
    impl Region {
        unsafe fn new() -> Self {
            // SAFETY: Test-owned initially empty region, released by Drop.
            let region = unsafe { CreateRectRgn(0, 0, 0, 0) };
            assert!(!region.is_null());
            Self(region)
        }
    }
    impl Drop for Region {
        fn drop(&mut self) {
            // SAFETY: This region is owned, not selected or assigned to an HWND.
            unsafe {
                DeleteObject(self.0);
            }
        }
    }

    unsafe fn client_damage(hwnd: HWND) -> Region {
        // SAFETY: Only inspect the actual update region; its bounding rectangle
        // can enclose the client even when four disjoint NC bands exclude it.
        unsafe {
            let update = Region::new();
            assert_ne!(GetUpdateRgn(hwnd, update.0, 0), ERROR);
            let mut client = RECT::default();
            assert_ne!(GetClientRect(hwnd, &mut client), 0);
            let clip = Region::new();
            assert_ne!(
                SetRectRgn(clip.0, client.left, client.top, client.right, client.bottom),
                0
            );
            assert_ne!(CombineRgn(update.0, update.0, clip.0, RGN_AND), ERROR);
            update
        }
    }

    unsafe fn assert_no_client_damage(hwnd: HWND) {
        // SAFETY: Owned scratch region and live test HWND; no invalidation or
        // validation here can conceal client damage introduced by the operation.
        unsafe {
            let damage = client_damage(hwnd);
            let mut actual = RECT::default();
            let kind = GetRgnBox(damage.0, &mut actual);
            let mut bounds = RECT::default();
            let bounding_update = GetUpdateRect(hwnd, &mut bounds, 0);
            assert_eq!(
                kind,
                NULLREGION,
                "NC damage reached the real client: region={:?}, GetUpdateRect={bounding_update} {:?}",
                rect_tuple(actual),
                rect_tuple(bounds)
            );
        }
    }

    unsafe fn assert_native_client_damage(hwnd: HWND, native: HWND) {
        // SAFETY: Matched live fixtures with the same operations and native
        // theme. Compare exact client regions, not just bounding boxes/booleans.
        unsafe {
            assert!(state(native).is_none());
            let mut actual_client = RECT::default();
            let mut native_client = RECT::default();
            assert_ne!(GetClientRect(hwnd, &mut actual_client), 0);
            assert_ne!(GetClientRect(native, &mut native_client), 0);
            assert_eq!(rect_tuple(actual_client), rect_tuple(native_client));
            let actual = client_damage(hwnd);
            let expected = client_damage(native);
            let mut actual_bounds = RECT::default();
            let mut expected_bounds = RECT::default();
            GetRgnBox(actual.0, &mut actual_bounds);
            GetRgnBox(expected.0, &mut expected_bounds);
            assert_ne!(
                EqualRgn(actual.0, expected.0),
                0,
                "presentation changed native client damage: actual={:?}, native={:?}",
                rect_tuple(actual_bounds),
                rect_tuple(expected_bounds)
            );
        }
    }

    unsafe fn update_deferred_frame(hwnd: HWND, native: Option<HWND>) {
        // SAFETY: Visible offscreen Edit with queued frame damage. Native resize
        // or redraw enabling may invalidate its client; allow only the exact
        // un-subclassed control's region, never erase/validate unexpected damage.
        unsafe {
            assert_ne!(IsWindowVisible(hwnd), 0);
            assert!(frame(hwnd).is_some());
            if let Some(native) = native {
                assert_native_client_damage(hwnd, native);
            } else {
                assert_no_client_damage(hwnd);
            }
            let before = screen_activity();
            UpdateWindow(hwnd);
            let after = screen_activity();
            assert!(
                after.0 > before.0 && after.1 > before.1,
                "UpdateWindow missed the deferred frame"
            );
            assert_eq!(state(hwnd).unwrap().get().defer_paint, 0);
            if let Some(native) = native {
                UpdateWindow(native);
            }
        }
    }

    #[test]
    fn frame_invalidation_preserves_the_native_client_update_region() {
        // SAFETY: Matched private offscreen Edit controls; the native baseline
        // has identical geometry/theme, but never installs input_frame's subclass.
        unsafe {
            let (_parent, hwnd) = framed_window(true);
            let (_native_parent, native) = native_window();
            for dirty in [false, true] {
                clear_damage(hwnd);
                clear_damage(native);
                if dirty {
                    let rect = RECT {
                        left: 4,
                        top: 5,
                        right: 18,
                        bottom: 19,
                    };
                    assert_ne!(InvalidateRect(hwnd, &rect, 0), 0);
                    assert_ne!(InvalidateRect(native, &rect, 0), 0);
                }
                let before = screen_activity();
                request_frame(hwnd);
                assert_eq!(screen_activity(), before);
                assert_native_client_damage(hwnd, native);
                if !dirty {
                    assert_no_client_damage(hwnd);
                    update_deferred_frame(hwnd, None);
                } else {
                    UpdateWindow(hwnd);
                }
            }
        }
    }

    struct NcActivateProbe {
        armed: Cell<bool>,
        nested_paint: Cell<bool>,
        calls: Cell<usize>,
        w: Cell<WPARAM>,
        l: Cell<LPARAM>,
        result: Cell<LRESULT>,
    }

    unsafe extern "system" fn ncactivate_probe(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: Lower subclass on a test-owned offscreen Edit. Its stack probe
        // outlives the HWND. Capture the actual forwarded payload/native return;
        // optional nested paints exercise the entire no-redraw dispatch interval.
        unsafe {
            let probe = &*(data as *const NcActivateProbe);
            if message == WM_NCACTIVATE && probe.armed.replace(false) {
                probe.calls.set(probe.calls.get() + 1);
                probe.w.set(w);
                probe.l.set(l);
                if probe.nested_paint.get() {
                    SendMessageW(hwnd, WM_NCPAINT, 1, 0);
                    SendMessageW(hwnd, WM_PAINT, 0, 0);
                }
                let result = DefSubclassProc(hwnd, message, w, l);
                probe.result.set(result);
                return result;
            }
            if message == WM_NCDESTROY {
                RemoveWindowSubclass(hwnd, Some(ncactivate_probe), id);
            }
            DefSubclassProc(hwnd, message, w, l)
        }
    }

    #[test]
    fn ncactivate_minus_one_preserves_native_dispatch_without_screen_overlays() {
        // SAFETY: Visible private HWNDs live outside the virtual desktop, never
        // activate or capture focus. Painting is observed by owner-thread counters,
        // not a screenshot; borrowed printing uses only an owned memory bitmap.
        unsafe {
            let probe = NcActivateProbe {
                armed: Cell::new(false),
                nested_paint: Cell::new(false),
                calls: Cell::new(0),
                w: Cell::new(0),
                l: Cell::new(0),
                result: Cell::new(0),
            };
            let parent = offscreen_parent();
            let hwnd = clientedge_edit(parent.0);
            assert_ne!(
                SetWindowSubclass(
                    hwnd,
                    Some(ncactivate_probe),
                    SUBCLASS_ID + 3,
                    &probe as *const NcActivateProbe as usize,
                ),
                0
            );
            apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
            ShowWindow(parent.0, SW_SHOWNOACTIVATE);
            assert_ne!(IsWindowVisible(hwnd), 0);
            let bitmap = BitmapDc::new();
            let original = snapshot(hwnd);
            for kind in [Resolved::Light, Resolved::Dark] {
                apply(hwnd, kind, palette(kind), 96);
                for nested_paint in [false, true] {
                    for active in [0, 1] {
                        clear_damage(hwnd);
                        probe.nested_paint.set(nested_paint);
                        probe.calls.set(0);
                        probe.armed.set(true);
                        let before = screen_activity();
                        let result = SendMessageW(hwnd, WM_NCACTIVATE, active, -1);
                        assert_eq!(probe.calls.get(), 1);
                        assert_eq!((probe.w.get(), probe.l.get()), (active, -1));
                        assert_eq!(result, probe.result.get());
                        assert_eq!(
                            screen_activity(),
                            before,
                            "WM_NCACTIVATE(-1) overlaid: {kind:?}, active={active}, nested={nested_paint}"
                        );
                        let value = state(hwnd).unwrap().get();
                        assert!(value.attached && value.active && value.redraw);
                        assert_eq!(value.defer_paint, 0);
                        assert_eq!(snapshot(hwnd), original);
                    }
                }
                // No latch: ordinary activation and later native paint still
                // overlay, while NC printing remains isolated to its borrowed DC.
                probe.nested_paint.set(false);
                probe.calls.set(0);
                probe.armed.set(true);
                let before = screen_activity();
                let result = SendMessageW(hwnd, WM_NCACTIVATE, 1, 0);
                assert_eq!(probe.calls.get(), 1);
                assert_eq!((probe.w.get(), probe.l.get()), (1, 0));
                assert_eq!(result, probe.result.get());
                let after = screen_activity();
                assert!(after.0 > before.0 && after.1 > before.1);
                let bounds = frame(hwnd).unwrap().bounds;
                fill(bitmap.dc, bounds, 0x123456);
                let before = screen_activity();
                SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
                assert_eq!(screen_activity(), before);
                assert_eq!(GetPixel(bitmap.dc, 0, 0), palette(kind).border);
                SendMessageW(hwnd, WM_NCPAINT, 1, 0);
                let after = screen_activity();
                assert!(after.0 > before.0 && after.1 > before.1);
            }
        }
    }

    #[test]
    fn hidden_geometry_moves_and_redraw_never_start_a_screen_overlay() {
        // SAFETY: The child has a real NC frame, but its parent stays hidden.
        unsafe {
            let (_parent, hwnd) = framed_window(false);
            let before = screen_activity();
            for n in 0..8 {
                assert_ne!(
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        31,
                        29,
                        241 + n,
                        61 + n,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW
                    ),
                    0
                );
                let mut moves = MoveBatch::with_capacity(1);
                moves.add(hwnd, 17, 19, 260 + n, 70 + n);
                moves.commit();
                SendMessageW(hwnd, WM_THEMECHANGED, 0, 0);
                SendMessageW(hwnd, WM_DPICHANGED_AFTERPARENT, 0, 0);
                SendMessageW(hwnd, WM_NCPAINT, 1, 0);
                SendMessageW(hwnd, WM_PAINT, 0, 0);
            }
            SendMessageW(hwnd, WM_SETREDRAW, 0, 0);
            SendMessageW(hwnd, WM_NCPAINT, 1, 0);
            SendMessageW(hwnd, WM_SETREDRAW, 1, 0);
            UpdateWindow(hwnd);
            assert_eq!(screen_activity(), before);
            assert_eq!(state(hwnd).unwrap().get().defer_paint, 0);
        }
    }

    #[test]
    fn visible_no_redraw_windowpos_preserves_native_client_damage_until_updatewindow() {
        // SAFETY: Shown outside the virtual desktop; counters observe this
        // thread's overlays, not screenshots after the intermediate frame.
        unsafe {
            let (_parent, hwnd) = framed_window(true);
            let (_native_parent, native) = native_window();
            for n in 0..8 {
                clear_damage(hwnd);
                clear_damage(native);
                let before = screen_activity();
                for edit in [native, hwnd] {
                    assert_ne!(
                        SetWindowPos(
                            edit,
                            null_mut(),
                            31 + n,
                            29,
                            241 + n,
                            61 + n,
                            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW
                        ),
                        0
                    );
                }
                assert_eq!(
                    screen_activity(),
                    before,
                    "SWP_NOREDRAW painted an intermediate frame"
                );
                update_deferred_frame(hwnd, Some(native));
            }
        }
    }

    #[test]
    fn visible_move_batch_defers_all_edit_overlays_until_updatewindow() {
        // SAFETY: Both children belong to one private offscreen parent. Use the
        // actual layout batch, which also keeps SWP_NOREDRAW on its fallback.
        unsafe {
            let (parent, first) = framed_window(true);
            let second = clientedge_edit(parent.0);
            apply(second, Resolved::Dark, palette(Resolved::Dark), 96);
            assert!(state(second).unwrap().get().active);
            assert_ne!(IsWindowVisible(second), 0);
            let (native_parent, native_first) = native_window();
            let native_second = native_clientedge_edit(native_parent.0);
            for n in 0..8 {
                for edit in [first, second, native_first, native_second] {
                    clear_damage(edit);
                }
                let before = screen_activity();
                for (first, second) in [(native_first, native_second), (first, second)] {
                    let mut moves = MoveBatch::with_capacity(2);
                    moves.add(first, 17, 19, 250 + n, 70 + n);
                    moves.add(second, 17, 110, 260 + n, 80 + n);
                    moves.commit();
                }
                assert_eq!(
                    screen_activity(),
                    before,
                    "MoveBatch painted before the layout completed"
                );
                update_deferred_frame(first, Some(native_first));
                update_deferred_frame(second, Some(native_second));
            }
        }
    }

    #[test]
    fn setrectnp_queues_a_frame_without_creating_client_damage() {
        // SAFETY: The native multiline Edit owns the formatting rectangle; our
        // presentation must not turn its no-paint operation into a client paint.
        unsafe {
            let (_parent, hwnd) = framed_window(true);
            let mut rect = RECT::default();
            assert_ne!(GetClientRect(hwnd, &mut rect), 0);
            rect.left += 3;
            rect.top += 3;
            rect.right -= 3;
            rect.bottom -= 3;
            let before = screen_activity();
            SendMessageW(hwnd, EM_SETRECTNP, 0, &rect as *const RECT as LPARAM);
            assert_eq!(screen_activity(), before);
            update_deferred_frame(hwnd, None);
        }
    }

    #[test]
    fn setredraw_blocks_screen_overlays_but_not_protected_nonclient_printing() {
        // SAFETY: Offscreen private HWND and owned memory DC. Borrowed printing
        // must still work while both the redraw gate and a geometry depth apply.
        unsafe {
            let (_parent, hwnd) = framed_window(true);
            let (_native_parent, native) = native_window();
            let retained = state(hwnd).unwrap();
            let bitmap = BitmapDc::new();
            let before = screen_activity();
            SendMessageW(native, WM_SETREDRAW, 0, 0);
            SendMessageW(hwnd, WM_SETREDRAW, 0, 0);
            assert!(!retained.get().redraw);
            assert_eq!(retained.get().defer_paint, 0);
            SendMessageW(hwnd, WM_NCPAINT, 1, 0);
            SendMessageW(hwnd, WM_PAINT, 0, 0);
            assert_eq!(screen_activity(), before);
            let deferred = PaintDeferral::new(&retained);
            assert_ne!(
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    31,
                    29,
                    251,
                    71,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW
                ),
                0
            );
            for message in [
                WM_PAINT,
                WM_NCPAINT,
                WM_NCACTIVATE,
                WM_THEMECHANGED,
                WM_DPICHANGED_AFTERPARENT,
            ] {
                SendMessageW(hwnd, message, usize::from(message != WM_PAINT), 0);
            }
            apply(hwnd, Resolved::Light, palette(Resolved::Light), 144);
            let native_frame = frame(hwnd).unwrap();
            let sentinel = 0x123456;
            fill(bitmap.dc, native_frame.bounds, sentinel);
            SetTextColor(bitmap.dc, 0x234567);
            SetBkColor(bitmap.dc, 0x345678);
            SetDCBrushColor(bitmap.dc, 0x456789);
            let _clip = SavedDc::new(bitmap.dc).unwrap();
            IntersectClipRect(bitmap.dc, 0, 0, 1, 1);
            SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
            assert_eq!(GetPixel(bitmap.dc, 0, 0), palette(Resolved::Light).border);
            assert_eq!(GetTextColor(bitmap.dc), 0x234567);
            assert_eq!(GetBkColor(bitmap.dc), 0x345678);
            assert_eq!(GetDCBrushColor(bitmap.dc), 0x456789);
            let mut clip = RECT::default();
            GetClipBox(bitmap.dc, &mut clip);
            assert_eq!(rect_tuple(clip), (0, 0, 1, 1));
            assert!(!retained.get().redraw);
            assert_eq!(retained.get().defer_paint, 1);
            assert_eq!(
                screen_activity(),
                before,
                "no-redraw/print reached the screen overlay"
            );
            drop(deferred);
            assert_eq!(retained.get().defer_paint, 0);
            SendMessageW(hwnd, WM_NCPAINT, 1, 0);
            SendMessageW(hwnd, WM_PAINT, 0, 0);
            assert_eq!(screen_activity(), before);
            assert_ne!(
                SetWindowPos(
                    native,
                    null_mut(),
                    31,
                    29,
                    251,
                    71,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW
                ),
                0
            );
            // Reset both while disabled: only native reenabling and our NC
            // request can contribute to the regions compared below.
            clear_damage(native);
            clear_damage(hwnd);
            SendMessageW(native, WM_SETREDRAW, 1, 0);
            SendMessageW(hwnd, WM_SETREDRAW, 1, 0);
            assert!(retained.get().redraw);
            assert_eq!(
                screen_activity(),
                before,
                "reenabling redraw painted synchronously"
            );
            update_deferred_frame(hwnd, Some(native));
        }
    }

    struct ReentrantPaintProbe {
        message: Cell<u32>,
        armed: Cell<bool>,
        calls: Cell<usize>,
        depth_before: Cell<u32>,
        depth_after: Cell<u32>,
        screen_before: Cell<(usize, usize)>,
        screen_after: Cell<(usize, usize)>,
    }

    unsafe extern "system" fn paint_during_geometry(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: This lower subclass borrows a test-local probe that outlives
        // the parent HWND. Reentry is disarmed before any nested native message.
        unsafe {
            let probe = &*(data as *const ReentrantPaintProbe);
            if message == probe.message.get()
                && probe.armed.replace(false)
                && let Some(retained) = state(hwnd)
            {
                probe.calls.set(probe.calls.get() + 1);
                probe.depth_before.set(retained.get().defer_paint);
                probe.screen_before.set(screen_activity());
                SendMessageW(hwnd, WM_PAINT, 0, 0);
                SendMessageW(hwnd, WM_NCPAINT, 1, 0);
                // Nested geometry must not clear the outer depth. Theme sync
                // also used to overlay immediately after its synchronous call.
                SendMessageW(
                    hwnd,
                    WM_SETFONT,
                    SendMessageW(hwnd, WM_GETFONT, 0, 0) as WPARAM,
                    0,
                );
                SendMessageW(hwnd, WM_THEMECHANGED, 0, 0);
                apply(hwnd, Resolved::Light, palette(Resolved::Light), 144);
                probe.depth_after.set(retained.get().defer_paint);
                SendMessageW(hwnd, WM_NCACTIVATE, 1, 0);
                SendMessageW(hwnd, WM_NCPAINT, 1, 0);
                SendMessageW(hwnd, WM_PAINT, 0, 0);
                probe.screen_after.set(screen_activity());
            }
            if message == WM_NCDESTROY {
                RemoveWindowSubclass(hwnd, Some(paint_during_geometry), id);
            }
            DefSubclassProc(hwnd, message, w, l)
        }
    }

    unsafe fn geometry_notification(hwnd: HWND, message: u32) {
        // SAFETY: Use live HWND/DC/font and correctly typed synchronous payloads;
        // synthetic notifications keep this fixture's styles/visibility intact.
        unsafe {
            let mut bounds = RECT::default();
            assert_ne!(GetWindowRect(hwnd, &mut bounds), 0);
            match message {
                WM_WINDOWPOSCHANGING | WM_WINDOWPOSCHANGED => {
                    assert_ne!(
                        SetWindowPos(
                            hwnd,
                            null_mut(),
                            31,
                            29,
                            bounds.right - bounds.left + 1,
                            bounds.bottom - bounds.top + 1,
                            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW
                        ),
                        0
                    );
                }
                WM_STYLECHANGING | WM_STYLECHANGED => {
                    let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
                    let mut change = STYLESTRUCT {
                        styleOld: style,
                        styleNew: style,
                    };
                    SendMessageW(
                        hwnd,
                        message,
                        GWL_STYLE as WPARAM,
                        &mut change as *mut STYLESTRUCT as LPARAM,
                    );
                }
                WM_NCCALCSIZE => {
                    SendMessageW(hwnd, message, 0, &mut bounds as *mut RECT as LPARAM);
                }
                WM_SETFONT => {
                    SendMessageW(hwnd, message, GetStockObject(DEFAULT_GUI_FONT) as WPARAM, 1);
                }
                WM_SHOWWINDOW | WM_SETREDRAW => {
                    SendMessageW(hwnd, message, 1, 0);
                }
                WM_DPICHANGED => {
                    SendMessageW(
                        hwnd,
                        message,
                        144 | (144 << 16),
                        &bounds as *const RECT as LPARAM,
                    );
                }
                EM_SETRECT | EM_SETRECTNP => {
                    let mut rect = RECT::default();
                    GetClientRect(hwnd, &mut rect);
                    rect.left += 3;
                    rect.top += 3;
                    rect.right -= 3;
                    rect.bottom -= 3;
                    SendMessageW(hwnd, message, 0, &rect as *const RECT as LPARAM);
                }
                _ => {
                    SendMessageW(hwnd, message, 0, 0);
                }
            }
        }
    }

    #[test]
    fn geometry_notifications_defer_reentrant_paints_and_theme_sync_without_latching() {
        // SAFETY: Declare the probe before its parent so every child callback,
        // including panic-time destruction, runs while its probe is still live.
        unsafe {
            let probe = ReentrantPaintProbe {
                message: Cell::new(0),
                armed: Cell::new(false),
                calls: Cell::new(0),
                depth_before: Cell::new(0),
                depth_after: Cell::new(0),
                screen_before: Cell::new((0, 0)),
                screen_after: Cell::new((0, 0)),
            };
            let parent = offscreen_parent();
            let hwnd = clientedge_edit(parent.0);
            assert_ne!(
                SetWindowSubclass(
                    hwnd,
                    Some(paint_during_geometry),
                    SUBCLASS_ID + 2,
                    &probe as *const ReentrantPaintProbe as usize
                ),
                0
            );
            apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
            ShowWindow(parent.0, SW_SHOWNOACTIVATE);
            assert_ne!(IsWindowVisible(hwnd), 0);
            assert!(frame(hwnd).is_some());
            for message in [
                WM_MOVE,
                WM_SIZE,
                WM_NCCALCSIZE,
                WM_WINDOWPOSCHANGING,
                WM_WINDOWPOSCHANGED,
                WM_STYLECHANGING,
                WM_STYLECHANGED,
                WM_THEMECHANGED,
                WM_SYSCOLORCHANGE,
                WM_SETTINGCHANGE,
                WM_SHOWWINDOW,
                WM_SETFONT,
                WM_DPICHANGED,
                WM_DPICHANGED_BEFOREPARENT,
                WM_DPICHANGED_AFTERPARENT,
                WM_SETREDRAW,
                EM_SETRECT,
                EM_SETRECTNP,
            ] {
                clear_damage(hwnd);
                probe.message.set(message);
                probe.armed.set(true);
                probe.calls.set(0);
                let before = screen_activity();
                geometry_notification(hwnd, message);
                assert_eq!(
                    probe.calls.get(),
                    1,
                    "message {message:#x} missed the lower subclass"
                );
                assert!(
                    probe.depth_before.get() > 0,
                    "message {message:#x} forwarded without a guard"
                );
                assert_eq!(
                    probe.depth_after.get(),
                    probe.depth_before.get(),
                    "nested geometry lost depth for {message:#x}"
                );
                assert_eq!(probe.screen_before.get(), before);
                assert_eq!(
                    probe.screen_after.get(),
                    before,
                    "reentrant paint/theme overlay for {message:#x}"
                );
                assert_eq!(
                    screen_activity(),
                    before,
                    "geometry overlaid on return for {message:#x}"
                );
                assert_eq!(state(hwnd).unwrap().get().defer_paint, 0);
            }
            // Native paints still overlay normally once all short-lived guards
            // are gone, and a fresh NC-only invalidation reaches UpdateWindow.
            for message in [WM_NCPAINT, WM_PAINT] {
                let before = screen_activity();
                SendMessageW(hwnd, message, usize::from(message == WM_NCPAINT), 0);
                let after = screen_activity();
                assert!(after.0 > before.0 && after.1 > before.1);
            }
            clear_damage(hwnd);
            let before = screen_activity();
            request_frame(hwnd);
            assert_eq!(screen_activity(), before);
            update_deferred_frame(hwnd, None);
        }
    }

    #[test]
    fn only_bordered_edits_and_real_nonclient_rings_are_supported() {
        for (style, extended) in [
            (WS_BORDER, 0),
            (WS_BORDER | ES_PASSWORD as u32, 0),
            (ES_MULTILINE as u32 | WS_VSCROLL, WS_EX_CLIENTEDGE),
            (0, WS_EX_CLIENTEDGE),
        ] {
            assert!(supported("EDIT", style, extended));
            assert!(supported("Edit", style, extended));
        }
        for (class, style, extended) in [
            ("Edit", 0, 0),
            ("Edit", WS_VSCROLL, 0),
            ("RichEdit50W", WS_BORDER, WS_EX_CLIENTEDGE),
            ("Static", WS_BORDER, 0),
            ("ComboBox", WS_BORDER, WS_EX_CLIENTEDGE),
        ] {
            assert!(!supported(class, style, extended));
        }
        let bounds = RECT {
            right: 12,
            bottom: 10,
            ..RECT::default()
        };
        let client = RECT {
            left: 2,
            top: 2,
            right: 10,
            bottom: 8,
        };
        assert_eq!(Frame::new(bounds, client).unwrap().inset, 2);
        assert!(Frame::new(bounds, bounds).is_none());
        assert!(Frame::new(bounds, RECT { top: 0, ..client }).is_none());
        assert!(Frame::new(bounds, RECT { left: -1, ..client }).is_none());
        assert!(Frame::new(RECT::default(), client).is_none());
        let tiny = Frame::new(
            bounds,
            RECT {
                left: 5,
                top: 4,
                right: 6,
                bottom: 5,
            },
        )
        .unwrap();
        assert_eq!(tiny.inset, 4);
        for kind in [Resolved::Light, Resolved::Dark] {
            let p = palette(kind);
            assert_eq!(border_color(p, true, false), p.border);
            assert_eq!(border_color(p, true, true), p.accent);
            assert_eq!(border_color(p, false, true), border_color(p, false, false));
        }
    }

    #[test]
    fn hidden_edits_keep_native_state_geometry_and_no_region_across_themes_and_dpi() {
        // SAFETY: All HWNDs/regions belong to this test. No top-level is shown,
        // no clipboard, application data, I/O or system setting is touched.
        unsafe {
            let parent = Window(window("STATIC", 0, WS_POPUP, null_mut()));
            let other = window("EDIT", 0, WS_CHILD | WS_VISIBLE | WS_BORDER, parent.0);
            let region = CreateRectRgn(0, 0, 0, 0);
            assert!(!region.is_null());
            for (style, extended) in [
                (WS_BORDER | ES_AUTOHSCROLL as u32, 0),
                (ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32, WS_EX_CLIENTEDGE),
                (
                    WS_BORDER | ES_MULTILINE as u32 | WS_VSCROLL | WS_HSCROLL,
                    WS_EX_CLIENTEDGE,
                ),
                (WS_BORDER | ES_MULTILINE as u32 | ES_READONLY as u32, 0),
            ] {
                let hwnd = window(
                    "EDIT",
                    extended,
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | style,
                    parent.0,
                );
                SendMessageW(hwnd, EM_SETSEL, 2, 6);
                SendMessageW(
                    hwnd,
                    EM_SETMARGINS,
                    (EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize,
                    3 | (7 << 16),
                );
                SendMessageW(hwnd, EM_SETLIMITTEXT, 128, 0);
                if style & ES_PASSWORD as u32 != 0 {
                    SendMessageW(hwnd, EM_SETPASSWORDCHAR, b'*' as usize, 0);
                }
                let before = snapshot(hwnd);
                assert!(supported_window(hwnd));
                assert!(frame(hwnd).is_some());
                let mut bounds = RECT::default();
                GetWindowRect(hwnd, &mut bounds);
                no_region(hwnd, region);
                SetFocus(hwnd);
                let focus = GetFocus();
                assert_eq!(focus, hwnd);
                for dpi in [96, 144, 192] {
                    for kind in [
                        Resolved::Light,
                        Resolved::Dark,
                        Resolved::Contrast,
                        Resolved::Light,
                    ] {
                        apply(hwnd, kind, palette(kind), dpi);
                        assert_eq!(
                            state(hwnd).is_some(),
                            kind != Resolved::Contrast,
                            "kind={kind:?} dpi={dpi} requested style={style:#x} extended={extended:#x} actual style={:#x} extended={:#x} live={}",
                            GetWindowLongPtrW(hwnd, GWL_STYLE),
                            GetWindowLongPtrW(hwnd, GWL_EXSTYLE),
                            IsWindow(hwnd)
                        );
                        assert_eq!(snapshot(hwnd), before);
                        assert_eq!(GetFocus(), focus);
                        no_region(hwnd, region);
                        let mut after = RECT::default();
                        GetWindowRect(hwnd, &mut after);
                        assert_eq!(rect_tuple(after), rect_tuple(bounds));
                        for message in [WM_NCPAINT, WM_NCACTIVATE] {
                            SendMessageW(hwnd, message, 1, 0);
                        }
                        assert_eq!(snapshot(hwnd), before);
                        assert_eq!(GetFocus(), focus);
                    }
                }
                SetFocus(other);
                let focus = GetFocus();
                assert_eq!(focus, other);
                apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 192);
                assert_eq!(GetFocus(), focus);
                assert_eq!(snapshot(hwnd), before);
                EnableWindow(hwnd, 0);
                let disabled = snapshot(hwnd);
                apply(hwnd, Resolved::Light, palette(Resolved::Light), 96);
                assert_eq!(snapshot(hwnd), disabled);
                EnableWindow(hwnd, 1);
                assert_eq!(snapshot(hwnd), before);
                MoveWindow(hwnd, 13, 17, 211, 32, 0);
                for message in [WM_DPICHANGED, WM_DPICHANGED_AFTERPARENT] {
                    SendMessageW(hwnd, message, 144 | (144 << 16), 0);
                    no_region(hwnd, region);
                }
                assert_eq!(snapshot(hwnd), before);
            }
            let borderless = window("EDIT", 0, WS_CHILD | WS_VISIBLE | WS_TABSTOP, parent.0);
            let label = window("STATIC", 0, WS_CHILD | WS_BORDER, parent.0);
            for hwnd in [borderless, label, null_mut()] {
                apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
                assert!(state(hwnd).is_none());
            }
            DeleteObject(region);
        }
    }

    #[test]
    fn frame_pixels_exclude_even_tiny_clients_and_preserve_borrowed_print_dcs() {
        // SAFETY: Only private hidden Edit and memory DC; direct draw tests isolate
        // our pass from native client printing. No screen pixels are sampled.
        unsafe {
            let parent = Window(window("STATIC", 0, WS_POPUP, null_mut()));
            let hwnd = window(
                "EDIT",
                WS_EX_CLIENTEDGE,
                WS_CHILD | WS_VISIBLE | WS_BORDER,
                parent.0,
            );
            let bitmap = BitmapDc::new();
            let sentinel = 0x123456;
            for dpi in [96, 144, 192] {
                for kind in [Resolved::Light, Resolved::Dark] {
                    let p = palette(kind);
                    apply(hwnd, kind, p, dpi);
                    let state = state(hwnd).unwrap();
                    let value = state.get();
                    let native = frame(hwnd).unwrap();
                    for (enabled, focused) in [(true, false), (true, true), (false, true)] {
                        fill(bitmap.dc, native.bounds, sentinel);
                        draw(bitmap.dc, native, value, enabled, focused);
                        assert_client(bitmap.dc, native.client, sentinel);
                        assert_eq!(GetPixel(bitmap.dc, 0, 0), border_color(p, enabled, focused));
                    }
                    let tiny = Frame::new(
                        RECT {
                            right: 9,
                            bottom: 9,
                            ..RECT::default()
                        },
                        RECT {
                            left: 4,
                            top: 4,
                            right: 5,
                            bottom: 5,
                        },
                    )
                    .unwrap();
                    fill(bitmap.dc, tiny.bounds, sentinel);
                    draw(bitmap.dc, tiny, value, true, true);
                    assert_eq!(GetPixel(bitmap.dc, 4, 4), sentinel);
                    fill(bitmap.dc, native.bounds, sentinel);
                    let _clip = SavedDc::new(bitmap.dc).unwrap();
                    // An NC-only WM_PRINT must not touch its real client. A
                    // client-only clip also proves there is no fallback overlay.
                    IntersectClipRect(
                        bitmap.dc,
                        native.client.left,
                        native.client.top,
                        native.client.right,
                        native.client.bottom,
                    );
                    SetTextColor(bitmap.dc, 0x234567);
                    SetBkColor(bitmap.dc, 0x345678);
                    SetBkMode(bitmap.dc, TRANSPARENT as i32);
                    SetDCBrushColor(bitmap.dc, 0x456789);
                    let brush = GetCurrentObject(bitmap.dc, OBJ_BRUSH as u32);
                    SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
                    assert_client(bitmap.dc, native.client, sentinel);
                    assert_eq!(GetTextColor(bitmap.dc), 0x234567);
                    assert_eq!(GetBkColor(bitmap.dc), 0x345678);
                    assert_eq!(GetBkMode(bitmap.dc), TRANSPARENT as i32);
                    assert_eq!(GetDCBrushColor(bitmap.dc), 0x456789);
                    assert_eq!(GetCurrentObject(bitmap.dc, OBJ_BRUSH as u32), brush);
                    let mut clip = RECT::default();
                    GetClipBox(bitmap.dc, &mut clip);
                    assert_eq!(rect_tuple(clip), rect_tuple(native.client));
                }
            }
            apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
            let native = frame(hwnd).unwrap();
            SetFocus(hwnd);
            assert_eq!(GetFocus(), hwnd);
            SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
            assert_eq!(GetPixel(bitmap.dc, 0, 0), palette(Resolved::Dark).accent);
            let other = window("EDIT", 0, WS_CHILD | WS_VISIBLE | WS_BORDER, parent.0);
            SetFocus(other);
            assert_eq!(GetFocus(), other);
            SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
            assert_eq!(GetPixel(bitmap.dc, 0, 0), palette(Resolved::Dark).border);
            EnableWindow(hwnd, 0);
            SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
            assert_eq!(
                GetPixel(bitmap.dc, 0, 0),
                border_color(palette(Resolved::Dark), false, false)
            );
            EnableWindow(hwnd, 1);
            // WM_PRINTCLIENT uses CLIENT coordinates. Sample beyond its actual
            // client extent, not at (0, 0), which native printing may validly fill.
            let sample = (native.bounds.right - 1, native.bounds.bottom - 1);
            for (message, flags, expected) in [
                (
                    WM_PRINT,
                    PRF_NONCLIENT,
                    Some(palette(Resolved::Dark).border),
                ),
                (WM_PRINT, PRF_NONCLIENT | PRF_CHECKVISIBLE, None),
                (WM_PRINT, 0, None),
                (WM_PRINTCLIENT, PRF_CLIENT, None),
            ] {
                fill(bitmap.dc, native.bounds, sentinel);
                let _clip = SavedDc::new(bitmap.dc).unwrap();
                IntersectClipRect(bitmap.dc, sample.0, sample.1, sample.0 + 1, sample.1 + 1);
                SendMessageW(hwnd, message, bitmap.dc as WPARAM, flags as LPARAM);
                assert_eq!(
                    GetPixel(bitmap.dc, sample.0, sample.1),
                    expected.unwrap_or(sentinel)
                );
            }
            // A translated caller mapping is restored around native printing and
            // used by our pass; the caller's clip, origin and colors survive.
            fill(
                bitmap.dc,
                RECT {
                    right: 800,
                    bottom: 240,
                    ..RECT::default()
                },
                sentinel,
            );
            let _saved = SavedDc::new(bitmap.dc).unwrap();
            SetViewportOrgEx(bitmap.dc, 7, 9, null_mut());
            IntersectClipRect(bitmap.dc, 0, 0, 1, 1);
            SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
            assert_eq!(GetPixel(bitmap.dc, 0, 0), palette(Resolved::Dark).border);
            let mut origin = POINT::default();
            GetViewportOrgEx(bitmap.dc, &mut origin);
            assert_eq!((origin.x, origin.y), (7, 9));
        }
    }

    #[test]
    fn repeated_frame_theme_and_print_passes_do_not_allocate_gdi_objects() {
        // SAFETY: Private hidden windows, one owned bitmap/DC and scratch region.
        // The resource baseline is taken only after warming native theme caches.
        unsafe {
            let parent = Window(window("STATIC", 0, WS_POPUP, null_mut()));
            let hwnd = window(
                "EDIT",
                WS_EX_CLIENTEDGE,
                WS_CHILD | WS_VISIBLE | WS_BORDER,
                parent.0,
            );
            let bitmap = BitmapDc::new();
            let region = CreateRectRgn(0, 0, 0, 0);
            assert!(!region.is_null());
            let p = palette(Resolved::Dark);
            // Warm PAINTING in every mode/size, not just theme transitions. Native
            // themed NC paint/print lazily creates caches that Dark never uses.
            for n in 0..24 {
                let kind = [Resolved::Light, Resolved::Dark, Resolved::Contrast][n % 3];
                apply(hwnd, kind, palette(kind), [96, 144, 192][n % 3]);
                MoveWindow(hwnd, 13, 17, 240 + (n % 2) as i32, 60, 0);
                SendMessageW(hwnd, WM_NCPAINT, 1, 0);
                SendMessageW(hwnd, WM_NCACTIVATE, 1, 0);
                SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
                no_region(hwnd, region);
            }
            apply(hwnd, Resolved::Dark, p, 192);
            let process = GetCurrentProcess();
            let before = GetGuiResources(process, GR_GDIOBJECTS);
            for n in 0..600 {
                let kind = [Resolved::Light, Resolved::Dark, Resolved::Contrast][n % 3];
                apply(hwnd, kind, palette(kind), [96, 144, 192][n % 3]);
                MoveWindow(hwnd, 13, 17, 240 + (n % 2) as i32, 60, 0);
                SendMessageW(hwnd, WM_NCPAINT, 1, 0);
                SendMessageW(hwnd, WM_NCACTIVATE, 1, 0);
                SendMessageW(hwnd, WM_PRINT, bitmap.dc as WPARAM, PRF_NONCLIENT as LPARAM);
                no_region(hwnd, region);
            }
            apply(hwnd, Resolved::Dark, p, 192);
            let after = GetGuiResources(process, GR_GDIOBJECTS);
            assert_eq!(
                before, after,
                "600 frame/theme/print passes leaked GDI objects"
            );
            DeleteObject(region);
        }
    }

    unsafe extern "system" fn lifetime_probe(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        id: usize,
        action: usize,
    ) -> LRESULT {
        // SAFETY: Private test subclass installed below input_frame. It simulates
        // synchronous destruction/detach while our callback is forwarding.
        unsafe {
            if message == WM_SETTEXT {
                match action {
                    1 => {
                        DestroyWindow(hwnd);
                        return 0;
                    }
                    2 => apply(hwnd, Resolved::Contrast, palette(Resolved::Dark), 96),
                    _ => {}
                }
            }
            if message == WM_THEMECHANGED && action == 3 {
                DestroyWindow(hwnd);
                return 0;
            }
            if message == WM_NCDESTROY {
                RemoveWindowSubclass(hwnd, Some(lifetime_probe), id);
            }
            DefSubclassProc(hwnd, message, w, l)
        }
    }

    #[test]
    fn synchronous_detach_and_destroy_keep_rc_alive_and_inactive_owners_are_safe() {
        // SAFETY: Isolated test-thread HWNDs; retained Rc proves lifetime after
        // a lower subclass destroys/removes us during native message forwarding.
        unsafe {
            let parent = Window(window("STATIC", 0, WS_POPUP, null_mut()));
            for action in [1, 2, 3] {
                let hwnd = window("EDIT", 0, WS_CHILD | WS_BORDER, parent.0);
                assert_ne!(
                    SetWindowSubclass(hwnd, Some(lifetime_probe), SUBCLASS_ID + 1, action),
                    0
                );
                if action == 3 {
                    apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
                    assert_eq!(IsWindow(hwnd), 0);
                    continue;
                }
                apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
                let retained = state(hwnd).unwrap();
                let text = [b'N' as u16, 0];
                SendMessageW(hwnd, WM_SETTEXT, 0, text.as_ptr() as LPARAM);
                assert!(!retained.get().attached);
                assert!(!retained.get().active);
                assert_eq!(Rc::strong_count(&retained), 1);
                if action == 2 {
                    assert!(state(hwnd).is_none());
                    assert_eq!(SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0), 1);
                } else {
                    assert_eq!(IsWindow(hwnd), 0);
                }
            }
            let hwnd = window("EDIT", 0, WS_CHILD | WS_BORDER, parent.0);
            apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
            let retained = state(hwnd).unwrap();
            // Reproduce the safe state left when RemoveWindowSubclass fails:
            // the installed owner remains retained, but must never paint.
            let mut value = retained.get();
            value.active = false;
            retained.set(value);
            let bitmap = BitmapDc::new();
            fill(
                bitmap.dc,
                RECT {
                    right: 240,
                    bottom: 60,
                    ..RECT::default()
                },
                0x123456,
            );
            overlay(hwnd, &retained, bitmap.dc);
            assert_eq!(GetPixel(bitmap.dc, 0, 0), 0x123456);
            assert_eq!(Rc::strong_count(&retained), 2);
            DestroyWindow(hwnd);
            assert!(!retained.get().attached);
            assert_eq!(Rc::strong_count(&retained), 1);
            apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
        }
    }
}
