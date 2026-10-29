//! Only native calendar/DropDown nonclient bands and host gutters are adapted.
//! No child surfaces, style changes, calendar raster tint or input interception.
use super::{Palette, Resolved, SavedDc, high_contrast};
use std::{cell::Cell, ptr::null, ptr::null_mut, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::*,
};

const ID: usize = 0x4d43_5043;
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
struct Presentation {
    calendar: HWND,
    host: HWND,
    palette: Palette,
    enabled: bool,
    redraw: [bool; 2],
    defer: u32,
}
type State = Rc<Cell<Presentation>>;
unsafe fn state(hwnd: HWND) -> Option<State> {
    // SAFETY: Only this proc/id owns the retained Rc. Retain on the owner thread
    // before native dispatch can remove a subclass or synchronously destroy it.
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), ID, &mut data) == 0 || data == 0 {
            return None;
        }
        let pointer = data as *const Cell<Presentation>;
        Rc::increment_strong_count(pointer);
        Some(Rc::from_raw(pointer))
    }
}
unsafe fn owns(hwnd: HWND, value: &State) -> bool {
    // SAFETY: Compare installed owners, not potentially reused HWND values.
    unsafe { state(hwnd).is_some_and(|other| Rc::ptr_eq(&other, value)) }
}
unsafe fn attach(hwnd: HWND, value: &State) -> bool {
    // SAFETY: Live owner-thread window; each installed subclass owns one Rc.
    unsafe {
        if let Some(other) = state(hwnd) {
            return Rc::ptr_eq(&other, value);
        }
        let pointer = Rc::into_raw(Rc::clone(value));
        if SetWindowSubclass(hwnd, Some(procedure), ID, pointer as usize) == 0 {
            drop(Rc::from_raw(pointer));
            return false;
        }
        true
    }
}
unsafe fn detach(hwnd: HWND, value: &State) {
    // SAFETY: Release only a successfully removed installed reference. Failed
    // removal leaves an inactive owner for NCDESTROY, never a dangling pointer.
    unsafe {
        if owns(hwnd, value) && RemoveWindowSubclass(hwnd, Some(procedure), ID) != 0 {
            drop(Rc::from_raw(Rc::as_ptr(value)));
            RedrawWindow(
                hwnd,
                null(),
                null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_FRAME,
            );
        }
    }
}
pub(super) unsafe fn native_host(calendar: HWND) -> Option<HWND> {
    // SAFETY: Query only; never treat an arbitrary owner/app window as chrome.
    unsafe {
        if GetWindowThreadProcessId(calendar, null_mut()) != GetCurrentThreadId()
            || GetWindowLongPtrW(calendar, GWL_STYLE) as u32 & WS_CHILD == 0
        {
            return None;
        }
        let host = GetParent(calendar);
        let mut name = [0u16; 32];
        let count = GetClassNameW(host, name.as_mut_ptr(), name.len() as i32);
        (count > 0
            && String::from_utf16_lossy(&name[..count as usize]).eq_ignore_ascii_case("DropDown")
            && GetWindowThreadProcessId(host, null_mut()) == GetCurrentThreadId()
            && GetWindowLongPtrW(host, GWL_STYLE) as u32 & WS_POPUP != 0)
            .then_some(host)
    }
}
pub(super) unsafe fn apply(calendar: HWND, kind: Resolved, palette: Palette) {
    // SAFETY: This component's live native calendar, on its UI thread. No styles,
    // fonts, theme handles or app State are owned by this presentation module.
    unsafe {
        if GetWindowThreadProcessId(calendar, null_mut()) != GetCurrentThreadId() {
            return;
        }
        let enabled = kind == Resolved::Dark && !high_contrast();
        let existing = state(calendar);
        if !enabled && existing.is_none() {
            return;
        }
        let state = existing.unwrap_or_else(|| {
            Rc::new(Cell::new(Presentation {
                calendar,
                host: null_mut(),
                palette,
                enabled,
                redraw: [true; 2],
                defer: 0,
            }))
        });
        if !attach(calendar, &state) {
            return;
        }
        let mut value = state.get();
        value.enabled = enabled;
        value.palette = palette;
        state.set(value);
        if !enabled {
            detach(value.host, &state);
            detach(calendar, &state);
            return;
        }
        let host = native_host(calendar).unwrap_or(null_mut());
        if value.host != host {
            detach(value.host, &state);
            value.host = null_mut();
        }
        if !host.is_null() && attach(host, &state) {
            value.host = host;
        }
        state.set(value);
        for hwnd in [calendar, value.host] {
            if owns(hwnd, &state) {
                RedrawWindow(
                    hwnd,
                    null(),
                    null_mut(),
                    RDW_INVALIDATE | RDW_FRAME | RDW_NOCHILDREN,
                );
            }
        }
    }
}

struct Deferral(State);
impl Deferral {
    fn new(state: &State) -> Self {
        let mut value = state.get();
        value.defer = value.defer.saturating_add(1);
        state.set(value);
        Self(Rc::clone(state))
    }
}
impl Drop for Deferral {
    fn drop(&mut self) {
        let mut value = self.0.get();
        value.defer = value.defer.saturating_sub(1);
        self.0.set(value);
    }
}

struct Geometry {
    outer: RECT,
    client: RECT,
}
unsafe fn geometry(hwnd: HWND) -> Option<Geometry> {
    // SAFETY: Measure both native client corners, including mirrored windows.
    unsafe {
        let mut outer = RECT::default();
        let mut client = RECT::default();
        if GetWindowRect(hwnd, &mut outer) == 0 || GetClientRect(hwnd, &mut client) == 0 {
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
        Some(Geometry {
            outer,
            client: RECT {
                left: first.x.min(last.x) - outer.left,
                top: first.y.min(last.y) - outer.top,
                right: first.x.max(last.x) - outer.left,
                bottom: first.y.max(last.y) - outer.top,
            },
        })
    }
}
unsafe fn frame(hwnd: HWND, state: &State, dc: HDC, nc: bool, client: bool, offset: POINT) {
    // SAFETY: Draw only measured NC and native host gutters; exclude every direct
    // child. No calendar client, arrows, dates, title or footer are overwritten.
    unsafe {
        let value = state.get();
        if !owns(hwnd, state) || !value.enabled || high_contrast() {
            return;
        }
        let Some(g) = geometry(hwnd) else { return };
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        let area = RECT {
            left: g.client.left + offset.x,
            top: g.client.top + offset.y,
            right: g.client.right + offset.x,
            bottom: g.client.bottom + offset.y,
        };
        let bounds = RECT {
            left: offset.x,
            top: offset.y,
            right: g.outer.right - g.outer.left + offset.x,
            bottom: g.outer.bottom - g.outer.top + offset.y,
        };
        if IntersectClipRect(dc, bounds.left, bounds.top, bounds.right, bounds.bottom) == ERROR {
            return;
        }
        if nc {
            let Some(_nc) = SavedDc::new(dc) else { return };
            if ExcludeClipRect(dc, area.left, area.top, area.right, area.bottom) != ERROR {
                SetDCBrushColor(dc, value.palette.border);
                FillRect(dc, &bounds, GetStockObject(DC_BRUSH));
            }
        }
        if client && hwnd == value.host && native_host(value.calendar) == Some(hwnd) {
            if IntersectClipRect(dc, area.left, area.top, area.right, area.bottom) == ERROR {
                return;
            }
            let mut child = GetWindow(hwnd, GW_CHILD);
            while !child.is_null() {
                let mut rect = RECT::default();
                if GetWindowRect(child, &mut rect) == 0
                    || ExcludeClipRect(
                        dc,
                        rect.left - g.outer.left + offset.x,
                        rect.top - g.outer.top + offset.y,
                        rect.right - g.outer.left + offset.x,
                        rect.bottom - g.outer.top + offset.y,
                    ) == ERROR
                {
                    return;
                }
                child = GetWindow(child, GW_HWNDNEXT);
            }
            SetDCBrushColor(dc, value.palette.surface);
            FillRect(dc, &area, GetStockObject(DC_BRUSH));
        }
    }
}
unsafe fn origin(dc: HDC, point: POINT) -> bool {
    // SAFETY: Borrowed saved DC. Convert a logical displacement rather than
    // adding logical pixels to a device-coordinate viewport (scaled print DC).
    unsafe {
        let mut points = [POINT::default(), point];
        LPtoDP(dc, points.as_mut_ptr(), 2) != 0
            && OffsetViewportOrgEx(
                dc,
                points[1].x - points[0].x,
                points[1].y - points[0].y,
                null_mut(),
            ) != 0
    }
}
unsafe fn print_children(hwnd: HWND, state: &State, dc: HDC, client: RECT, outer: RECT) {
    // SAFETY: Public WM_PRINT traversal of actual visible direct children. Each
    // child gets its full native content in its own coordinate system, outside
    // the parent's NC-only clip. LPtoDP preserves the caller's mapping/scale.
    unsafe {
        let mut child = GetWindow(hwnd, GW_CHILD);
        while !child.is_null() && owns(hwnd, state) && GetParent(child) == hwnd {
            let mut rect = RECT::default();
            if GetWindowLongPtrW(child, GWL_STYLE) as u32 & WS_VISIBLE != 0
                && GetWindowRect(child, &mut rect) != 0
            {
                let Some(_saved) = SavedDc::new(dc) else {
                    return;
                };
                let x = rect.left - outer.left;
                let y = rect.top - outer.top;
                if IntersectClipRect(dc, client.left, client.top, client.right, client.bottom)
                    != ERROR
                    && IntersectClipRect(dc, x, y, rect.right - outer.left, rect.bottom - outer.top)
                        != ERROR
                    && origin(dc, POINT { x, y })
                {
                    SendMessageW(
                        child,
                        WM_PRINT,
                        dc as WPARAM,
                        (PRF_CLIENT | PRF_NONCLIENT | PRF_ERASEBKGND | PRF_CHILDREN) as LPARAM,
                    );
                }
            }
            // Native child printing may destroy it (or its parent); do not
            // follow a stale sibling handle captured before that dispatch.
            if !owns(hwnd, state) || GetParent(child) != hwnd {
                break;
            }
            child = GetWindow(child, GW_HWNDNEXT);
        }
    }
}
unsafe fn print(hwnd: HWND, state: &State, message: u32, dc: HDC, flags: LPARAM) -> LRESULT {
    // SAFETY: Saved caller DC and retained state. Native monthcal can paint client
    // pixels even for NC-only WM_PRINT: constrain only the parent pass, then
    // restore that clip before drawing requested children. Never tint the raster.
    unsafe {
        let Some(g) = geometry(hwnd) else { return 0 };
        let direct = message == WM_PRINTCLIENT;
        let client = direct || flags & (PRF_CLIENT | PRF_ERASEBKGND) as LPARAM != 0;
        let nc = !direct && flags & PRF_NONCLIENT as LPARAM != 0;
        let offset = if direct {
            POINT {
                x: -g.client.left,
                y: -g.client.top,
            }
        } else {
            POINT::default()
        };
        let mut result = 0;
        if client || nc {
            {
                let Some(_native) = SavedDc::new(dc) else {
                    return 0;
                };
                let r = if client && !nc {
                    g.client
                } else {
                    RECT {
                        left: 0,
                        top: 0,
                        right: g.outer.right - g.outer.left,
                        bottom: g.outer.bottom - g.outer.top,
                    }
                };
                if IntersectClipRect(
                    dc,
                    r.left + offset.x,
                    r.top + offset.y,
                    r.right + offset.x,
                    r.bottom + offset.y,
                ) == ERROR
                    || (nc
                        && !client
                        && ExcludeClipRect(
                            dc,
                            g.client.left,
                            g.client.top,
                            g.client.right,
                            g.client.bottom,
                        ) == ERROR)
                {
                    return 0;
                }
                result = DefSubclassProc(
                    hwnd,
                    message,
                    dc as WPARAM,
                    flags & !((PRF_CHILDREN | PRF_OWNED) as LPARAM),
                );
            }
            if !owns(hwnd, state) {
                return result;
            }
            frame(hwnd, state, dc, nc, client, offset);
        }
        if owns(hwnd, state) && flags & PRF_CHILDREN as LPARAM != 0 {
            let Some(_children) = SavedDc::new(dc) else {
                return result;
            };
            if direct && !origin(dc, offset) {
                return result;
            }
            print_children(hwnd, state, dc, g.client, g.outer);
        }
        if owns(hwnd, state) && !direct && flags & PRF_OWNED as LPARAM != 0 {
            DefWindowProcW(
                hwnd,
                WM_PRINT,
                dc as WPARAM,
                flags & (PRF_OWNED | PRF_CHECKVISIBLE) as LPARAM,
            );
        }
        result
    }
}
fn geometry_message(message: u32) -> bool {
    matches!(
        message,
        WM_SIZE
            | WM_MOVE
            | WM_WINDOWPOSCHANGING
            | WM_WINDOWPOSCHANGED
            | WM_NCCALCSIZE
            | WM_STYLECHANGING
            | WM_STYLECHANGED
            | WM_SETFONT
            | WM_THEMECHANGED
            | WM_DPICHANGED
            | WM_DPICHANGED_AFTERPARENT
            | WM_SHOWWINDOW
    )
}
unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: Retain the installed Rc before any dispatch; nested removal or
    // destruction cannot free this invocation. Recheck identity before drawing.
    unsafe {
        let pointer = data as *const Cell<Presentation>;
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        if message == WM_NCDESTROY {
            let installed = owns(hwnd, &state);
            RemoveWindowSubclass(hwnd, Some(procedure), ID);
            let mut value = state.get();
            let host = if hwnd == value.calendar {
                value.host
            } else {
                null_mut()
            };
            if hwnd == value.calendar {
                value.calendar = null_mut();
                value.enabled = false;
            }
            if hwnd == value.host || !host.is_null() {
                value.host = null_mut();
            }
            state.set(value);
            detach(host, &state);
            if installed {
                drop(Rc::from_raw(pointer));
            }
            return DefSubclassProc(hwnd, message, w, l);
        }
        if message == WM_SETREDRAW {
            let mut value = state.get();
            value.redraw[usize::from(hwnd == value.host)] = w != 0;
            state.set(value);
        }
        if matches!(message, WM_PRINT | WM_PRINTCLIENT) {
            if w == 0
                || (message == WM_PRINT
                    && (l == 0
                        || (l & PRF_CHECKVISIBLE as LPARAM != 0 && IsWindowVisible(hwnd) == 0)))
            {
                return 0;
            }
            let Some(_saved) = SavedDc::new(w as HDC) else {
                return 0;
            };
            let _deferred = Deferral::new(&state);
            return print(hwnd, &state, message, w as HDC, l);
        }
        let geometry = geometry_message(message);
        let no_redraw = message == WM_NCACTIVATE && l == -1;
        let deferred = (geometry || no_redraw).then(|| Deferral::new(&state));
        let result = DefSubclassProc(hwnd, message, w, l);
        drop(deferred);
        let value = state.get();
        if !geometry
            && !no_redraw
            && value.enabled
            && value.defer == 0
            && value.redraw[usize::from(hwnd == value.host)]
            && owns(hwnd, &state)
            && !high_contrast()
            && IsWindowVisible(hwnd) != 0
            && matches!(message, WM_NCPAINT | WM_NCACTIVATE | WM_PAINT)
        {
            #[cfg(test)]
            SCREEN_PASSES.with(|count| count.set(count.get() + 1));
            let dc = GetWindowDC(hwnd);
            if !dc.is_null() {
                frame(hwnd, &state, dc, true, true, POINT::default());
                ReleaseDC(hwnd, dc);
            }
        }
        result
    }
}

#[cfg(test)]
std::thread_local! { static SCREEN_PASSES: Cell<usize> = const { Cell::new(0) }; }
#[cfg(test)]
pub(super) fn screen_passes() -> usize {
    SCREEN_PASSES.with(Cell::get)
}
#[cfg(test)]
pub(super) unsafe fn owner_alive(hwnd: HWND) -> Option<impl Fn() -> bool> {
    // SAFETY: Weak observation only; cannot retain or resurrect a removed owner.
    unsafe {
        let owner = Rc::downgrade(&state(hwnd)?);
        Some(move || owner.strong_count() != 0)
    }
}
#[cfg(test)]
pub(super) unsafe fn active(hwnd: HWND) -> bool {
    // SAFETY: Test-only owner-thread lookup of our installed owner.
    unsafe { state(hwnd).is_some_and(|state| state.get().enabled) }
}
