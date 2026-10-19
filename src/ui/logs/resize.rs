//! Native keyboard-focusable height grip. Only presentation and mouse capture
//! live here; the parent applies a bounded DIP height without touching log data.
use super::super::{
    theme::{Palette, Resolved},
    *,
};
use std::{cell::Cell, rc::Rc};
use windows_sys::Win32::Graphics::Gdi::*;

pub(super) const ID: u32 = 2230;
pub(in crate::ui) const CHANGED: u32 = WM_APP + 90;
pub(super) const DEFAULT_HEIGHT: i32 = 240;
const MIN_HEIGHT: i32 = 120;
const MAX_HEIGHT: i32 = 720;
const STEP: i32 = 24;
const SUBCLASS_ID: usize = 0x4c52_535a;
const CAPTION: &str = "拖动调整日志高度（↑↓调整，Home恢复默认）";
type Proc = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: Proc, id: usize, data: usize) -> i32;
    fn GetWindowSubclass(hwnd: HWND, proc: Proc, id: usize, data: *mut usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: Proc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn SetCapture(hwnd: HWND) -> HWND;
    fn GetCapture() -> HWND;
    fn ReleaseCapture() -> i32;
    fn GetFocus() -> HWND;
    fn SetFocus(hwnd: HWND) -> HWND;
}
#[derive(Clone, Copy)]
struct Drag {
    screen_y: i32,
    height: i32,
}
#[derive(Clone, Copy)]
struct Appearance {
    attached: bool,
    dpi: u32,
    height: i32,
    kind: Resolved,
    palette: Palette,
    drag: Option<Drag>,
    deferred_paint: u32,
}
type State = Rc<Cell<Appearance>>;

pub(super) fn clamp(height: i32) -> i32 {
    height.clamp(MIN_HEIGHT, MAX_HEIGHT)
}
fn dragged(start: Drag, screen_y: i32, dpi: u32) -> i32 {
    let delta =
        (i64::from(screen_y) - i64::from(start.screen_y)) * 96 / i64::from(dpi.clamp(96, 768));
    (i64::from(start.height) + delta).clamp(i64::from(MIN_HEIGHT), i64::from(MAX_HEIGHT)) as i32
}
unsafe fn state(hwnd: HWND) -> Option<State> {
    // SAFETY: Only this owner-thread proc/id owns the raw Rc; retain before FFI.
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
pub(super) fn create(
    parent: HWND,
    instance: windows_sys::Win32::Foundation::HINSTANCE,
) -> Result<HWND, String> {
    let theme = theme::Theme::new(crate::config::ThemeMode::System)?;
    // SAFETY: UI-thread native Button, copied caption and no borrowed app State.
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide(CAPTION).as_ptr(),
            WS_CHILD | WS_TABSTOP | BS_OWNERDRAW as u32,
            0,
            0,
            0,
            0,
            parent,
            ID as usize as HMENU,
            instance,
            null(),
        );
        if hwnd.is_null() {
            return Err(last_error("创建日志高度调整条"));
        }
        let value = Rc::new(Cell::new(Appearance {
            attached: true,
            dpi: 96,
            height: DEFAULT_HEIGHT,
            kind: theme.kind,
            palette: theme.palette,
            drag: None,
            deferred_paint: 0,
        }));
        let owner = Rc::into_raw(value);
        if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, owner as usize) == 0 {
            drop(Rc::from_raw(owner));
            DestroyWindow(hwnd);
            return Err(last_error("关联日志高度调整条"));
        }
        Ok(hwnd)
    }
}
unsafe fn end_drag(hwnd: HWND, state: &State) {
    // SAFETY: Clear before ReleaseCapture synchronously dispatches CAPTURECHANGED;
    // only release our grip's capture, never the text selection or another HWND.
    unsafe {
        let mut value = state.get();
        let active = value.drag.take().is_some();
        state.set(value);
        if active && GetCapture() == hwnd {
            ReleaseCapture();
        }
        if state.get().attached && state.get().deferred_paint == 0 {
            InvalidateRect(hwnd, null(), 0);
        }
    }
}
pub(in crate::ui) unsafe fn apply(hwnd: HWND, kind: Resolved, palette: Palette, dpi: u32) {
    // SAFETY: Live owner-thread child; retain local Rc through capture callbacks.
    unsafe {
        let Some(state) = state(hwnd) else { return };
        end_drag(hwnd, &state);
        let mut value = state.get();
        if !value.attached {
            return;
        }
        value.kind = kind;
        value.palette = palette;
        value.dpi = dpi.clamp(96, 768);
        state.set(value);
        InvalidateRect(hwnd, null(), 0);
    }
}
pub(in crate::ui) unsafe fn cancel(hwnd: HWND) {
    // SAFETY: Owner-thread grip; retain presentation state through capture release.
    unsafe {
        if let Some(state) = state(hwnd) {
            end_drag(hwnd, &state);
        }
    }
}

pub(super) unsafe fn set_height(hwnd: HWND, height: i32, dpi: u32) {
    // SAFETY: Owner-thread lookup; no references survive a native message.
    unsafe {
        if let Some(state) = state(hwnd) {
            let mut value = state.get();
            value.height = clamp(height);
            value.dpi = dpi.clamp(96, 768);
            state.set(value);
        }
    }
}
unsafe fn change(hwnd: HWND, state: &State, height: i32) {
    // SAFETY: Store only a bounded scalar; reparenting places the grip below the
    // viewport. The root validates this fixed ID and never accepts a pointer.
    unsafe {
        let mut value = state.get();
        let height = clamp(height);
        if !value.attached || value.height == height {
            return;
        }
        value.height = height;
        state.set(value);
        let root = GetAncestor(hwnd, GA_ROOT);
        SendMessageW(root, CHANGED, height as WPARAM, ID as LPARAM);
    }
}
pub(in crate::ui) unsafe fn draw(l: LPARAM) -> bool {
    // SAFETY: A live WM_DRAWITEM payload; validate the control's own state/ID
    // before accessing its borrowed DC. All stock objects are restored, not freed.
    unsafe {
        let item = &*(l as *const windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT);
        if item.CtlID != ID {
            return false;
        }
        let Some(state) = state(item.hwndItem) else {
            return false;
        };
        let value = state.get();
        let dc = item.hDC;
        let saved = SaveDC(dc);
        if saved == 0 {
            return true;
        }
        let p = value.palette;
        SelectObject(dc, GetStockObject(DC_BRUSH));
        SetDCBrushColor(dc, p.background);
        FillRect(dc, &item.rcItem, GetStockObject(DC_BRUSH));
        let center = (item.rcItem.left + item.rcItem.right) / 2;
        let middle = (item.rcItem.top + item.rcItem.bottom) / 2;
        let width = scale(44, value.dpi).min(item.rcItem.right - item.rcItem.left);
        let thickness = scale(3, value.dpi).max(1);
        let color = if value.kind == Resolved::Contrast {
            p.text
        } else if value.drag.is_some() || GetFocus() == item.hwndItem {
            p.accent
        } else {
            p.disabled
        };
        SelectObject(dc, GetStockObject(DC_PEN));
        SetDCPenColor(dc, color);
        SetDCBrushColor(dc, color);
        RoundRect(
            dc,
            center - width / 2,
            middle - thickness / 2,
            center + width / 2,
            middle + thickness - thickness / 2,
            thickness,
            thickness,
        );
        if item.itemState
            & (windows_sys::Win32::UI::Controls::ODS_FOCUS
                | windows_sys::Win32::UI::Controls::ODS_NOFOCUSRECT)
            == windows_sys::Win32::UI::Controls::ODS_FOCUS
        {
            let mut focus = item.rcItem;
            InflateRect(&mut focus, -scale(1, value.dpi), -scale(1, value.dpi));
            SetBkColor(dc, p.background);
            DrawFocusRect(dc, &focus);
        }
        RestoreDC(dc, saved);
        true
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
    // SAFETY: Installed on this UI-thread grip with one owning Rc. Retain before
    // every forwarding/native call; parent layout can move this control reentrantly.
    unsafe {
        let pointer = data as *const Cell<Appearance>;
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        if message == WM_NCDESTROY {
            end_drag(hwnd, &state);
            let mut value = state.get();
            value.attached = false;
            state.set(value);
            RemoveWindowSubclass(hwnd, Some(procedure), id);
            drop(Rc::from_raw(pointer));
            return DefSubclassProc(hwnd, message, w, l);
        }
        match message {
            WM_SETCURSOR => {
                SetCursor(LoadCursorW(null_mut(), IDC_SIZENS));
                return 1;
            }
            WM_LBUTTONDOWN => {
                SetFocus(hwnd);
                if !state.get().attached {
                    return 0;
                }
                let mut point = POINT {
                    x: l as u16 as i16 as i32,
                    y: (l as u32 >> 16) as u16 as i16 as i32,
                };
                if ClientToScreen(hwnd, &mut point) == 0 {
                    return 0;
                }
                let mut value = state.get();
                value.drag = Some(Drag {
                    screen_y: point.y,
                    height: value.height,
                });
                state.set(value);
                SetCapture(hwnd);
                if GetCapture() != hwnd {
                    end_drag(hwnd, &state);
                }
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            WM_MOUSEMOVE if state.get().drag.is_some() => {
                if GetCapture() != hwnd {
                    end_drag(hwnd, &state);
                    return 0;
                }
                let mut point = POINT {
                    x: l as u16 as i16 as i32,
                    y: (l as u32 >> 16) as u16 as i16 as i32,
                };
                if ClientToScreen(hwnd, &mut point) != 0 {
                    let value = state.get();
                    if let Some(start) = value.drag {
                        change(hwnd, &state, dragged(start, point.y, value.dpi));
                    }
                }
                return 0;
            }
            WM_LBUTTONUP => {
                end_drag(hwnd, &state);
                return 0;
            }
            WM_CANCELMODE | WM_CAPTURECHANGED | WM_KILLFOCUS => {
                end_drag(hwnd, &state);
            }
            WM_SHOWWINDOW | WM_ENABLE if w == 0 => {
                end_drag(hwnd, &state);
            }
            WM_KEYDOWN if w == 0x1b => {
                let start = state.get().drag;
                end_drag(hwnd, &state);
                if let Some(start) = start {
                    change(hwnd, &state, start.height);
                }
                return 0;
            }
            WM_KEYDOWN if matches!(w, 0x26 | 0x28 | 0x24) => {
                end_drag(hwnd, &state);
                let value = state.get();
                let height = match w {
                    0x26 => value.height - STEP,
                    0x28 => value.height + STEP,
                    _ => DEFAULT_HEIGHT,
                };
                change(hwnd, &state, height);
                // Keyboard growth moves the focused grip without a new SETFOCUS.
                // Reveal only after layout; mouse drags retain their screen anchor.
                if state.get().attached && GetFocus() == hwnd {
                    viewport::reveal(hwnd);
                }
                return 0;
            }
            WM_GETDLGCODE => {
                if w == 0x1b && state.get().drag.is_some() {
                    return DLGC_WANTALLKEYS as LRESULT;
                }
                return DLGC_WANTARROWS as LRESULT;
            }
            _ => {}
        }
        if matches!(
            message,
            WM_WINDOWPOSCHANGING | WM_WINDOWPOSCHANGED | WM_SIZE | WM_SETFONT
        ) {
            let mut value = state.get();
            value.deferred_paint = value.deferred_paint.saturating_add(1);
            state.set(value);
            let result = DefSubclassProc(hwnd, message, w, l);
            let mut value = state.get();
            value.deferred_paint = value.deferred_paint.saturating_sub(1);
            state.set(value);
            return result;
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn height_and_screen_delta_are_bounded_in_dip() {
        for dpi in [96, 144, 192] {
            let start = Drag {
                height: DEFAULT_HEIGHT,
                screen_y: 700,
            };
            assert_eq!(dragged(start, 700 + scale(48, dpi), dpi), 288);
            assert_eq!(dragged(start, 700 - scale(48, dpi), dpi), 192);
            assert_eq!(dragged(start, i32::MAX, dpi), MAX_HEIGHT);
            assert_eq!(dragged(start, i32::MIN, dpi), MIN_HEIGHT);
        }
        assert_eq!(clamp(-1), MIN_HEIGHT);
        assert_eq!(clamp(999), MAX_HEIGHT);
    }
}
