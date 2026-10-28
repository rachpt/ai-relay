//! Auto-hide only the log viewer's native scrollbars. EDIT owns all ranges and
//! scrolling; hiding a bar stops its range updates, so re-enable before querying.
use std::{cell::Cell, mem::size_of, ptr::null, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{RDW_FRAME, RDW_INVALIDATE, RDW_NOCHILDREN, RedrawWindow},
    UI::{Controls::*, WindowsAndMessaging::*},
};

const ID: usize = 0x4c53_4241;
const REFRESH: u32 = WM_APP + 92;
#[link(name = "user32")]
unsafe extern "system" {
    fn GetCapture() -> HWND;
}
type Proc = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: Proc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: Proc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

#[derive(Clone, Copy)]
struct Presentation {
    attached: bool,
    updating: bool,
    depth: u32,
    redraw: bool,
    dirty: bool,
    pending: bool,
    timer: usize,
    internal_redraw: Option<bool>,
    redraw_generation: u64,
    measuring_size: (i32, i32),
}
type State = Rc<Cell<Presentation>>;

/// Called once, only for the read-only multiline log content EDIT on its owner
/// thread. The subclass owns no log data or references into application State.
pub(super) unsafe fn attach(hwnd: HWND) -> Result<(), String> {
    // SAFETY: Caller owns this native EDIT. One Rc belongs to the subclass until
    // NCDESTROY; the local clone survives synchronous callbacks during setup.
    unsafe {
        let state = Rc::new(Cell::new(Presentation {
            attached: true,
            updating: false,
            depth: 0,
            redraw: true,
            dirty: true,
            pending: false,
            timer: 0,
            internal_redraw: None,
            redraw_generation: 0,
            measuring_size: (0, 0),
        }));
        let owner = Rc::into_raw(Rc::clone(&state));
        if SetWindowSubclass(hwnd, Some(procedure), ID, owner as usize) == 0 {
            drop(Rc::from_raw(owner));
            return Err(crate::windows::last_error("关联日志滚动条显隐"));
        }
        synchronize(hwnd, &state);
        Ok(())
    }
}

fn fits(min: i32, max: i32, page: u32) -> bool {
    max >= min && i64::from(max) - i64::from(min) < i64::from(page.max(1))
}

unsafe fn window_size(hwnd: HWND) -> (i32, i32) {
    // SAFETY: Owner-thread HWND; the output rectangle is a live stack buffer.
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect);
        (rect.right - rect.left, rect.bottom - rect.top)
    }
}

unsafe fn post_refresh(hwnd: HWND, state: &State) {
    // SAFETY: Copied scalar state; coalesce without retaining pointers in a message.
    unsafe {
        let mut value = state.get();
        if value.attached && !value.pending {
            value.pending = true;
            state.set(value);
            if PostMessageW(hwnd, REFRESH, 0, 0) == 0 {
                value = state.get();
                value.pending = false;
                state.set(value);
            }
        }
    }
}

unsafe fn redraw(hwnd: HWND, state: &State, enabled: bool) {
    // SAFETY: This next same-thread message consumes its marker before forwarding;
    // a nested external SETREDRAW remains distinguishable and increments generation.
    unsafe {
        let mut value = state.get();
        value.internal_redraw = Some(enabled);
        state.set(value);
        SendMessageW(hwnd, WM_SETREDRAW, usize::from(enabled), 0);
    }
}

unsafe fn scroll_info(hwnd: HWND, axis: i32) -> Option<SCROLLINFO> {
    // SAFETY: Same-thread query into a correctly sized stack buffer.
    unsafe {
        let mut info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            ..Default::default()
        };
        (GetScrollInfo(hwnd, axis, &mut info) != 0).then_some(info)
    }
}

unsafe fn restore_scroll(
    hwnd: HWND,
    state: &State,
    first: LRESULT,
    horizontal: Option<SCROLLINFO>,
) {
    // SAFETY: Native EDIT messages only. Temporary removal can clamp bottom/right
    // positions as the client grows. Restore using real line/character scrolling,
    // never by overwriting scrollbar range/page/position independently of text.
    unsafe {
        if !state.get().attached || state.get().dirty {
            return;
        }
        let now = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, 0, 0);
        if !state.get().attached || state.get().dirty {
            return;
        }
        if first != now {
            SendMessageW(hwnd, EM_LINESCROLL, 0, first.saturating_sub(now));
        }
        if !state.get().attached
            || state.get().dirty
            || GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_HSCROLL == 0
        {
            return;
        }
        let (Some(saved), Some(current)) = (horizontal, scroll_info(hwnd, SB_HORZ)) else {
            return;
        };
        let end = (i64::from(current.nMax) - i64::from(current.nPage.max(1)) + 1)
            .max(i64::from(current.nMin));
        let target = i64::from(saved.nPos).clamp(i64::from(current.nMin), end);
        if target == i64::from(current.nPos) {
            return;
        }
        let Some(unit) = super::super::scroll_theme::edit_scroll_unit(hwnd) else {
            return;
        };
        if !state.get().attached || state.get().dirty {
            return;
        }
        // Native character positions are anchored at either edge. Anchoring first
        // avoids losing a partial character at the rightmost native clamp.
        let anchor = if (target - i64::from(current.nMin)) % i64::from(unit) == 0 {
            Some(SB_LEFT)
        } else if (end - target) % i64::from(unit) == 0 {
            Some(SB_RIGHT)
        } else {
            None
        };
        if let Some(anchor) = anchor {
            SendMessageW(hwnd, WM_HSCROLL, anchor as WPARAM, 0);
        }
        if !state.get().attached || state.get().dirty {
            return;
        }
        if let Some(current) = scroll_info(hwnd, SB_HORZ) {
            let delta = (target - i64::from(current.nPos)) / i64::from(unit);
            SendMessageW(hwnd, EM_LINESCROLL, delta as WPARAM, 0);
        }
    }
}

unsafe fn show_bar(hwnd: HWND, state: &State, axis: i32, visible: bool, other_visible: bool) {
    // SAFETY: Same-thread EDIT. Classic theme may add the other style during
    // WM_SIZE without accounting for its NC width. Preserve our last decision
    // for that axis before querying page/range; forcing FRAMECHANGED instead
    // would make two unnecessary bars consume enough space to justify each other.
    unsafe {
        ShowScrollBar(hwnd, axis, i32::from(visible));
        if state.get().attached {
            let (other, mask) = if axis == SB_HORZ {
                (SB_VERT, WS_VSCROLL)
            } else {
                (SB_HORZ, WS_HSCROLL)
            };
            if (GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & mask != 0) != other_visible {
                ShowScrollBar(hwnd, other, i32::from(other_visible));
            }
        }
    }
}

unsafe fn synchronize(hwnd: HWND, state: &State) {
    // SAFETY: Owner-thread HWND; retain state and copy scalars across every FFI.
    // ShowScrollBar can synchronously resize/destroy the window. Reentrant SIZE
    // messages must not restart this bounded, native-range-only calculation.
    unsafe {
        let mut value = state.get();
        if !value.attached || value.updating || value.depth != 0 || !value.dirty {
            return;
        }
        if GetCapture() == hwnd {
            // Live refresh must not cancel a scrollbar drag/repeat or text
            // selection. Recheck after release; native range updates still run.
            if value.timer == 0 {
                value.timer = SetTimer(hwnd, ID, 50, None);
                state.set(value);
            }
            return;
        }
        if value.timer != 0 {
            KillTimer(hwnd, value.timer);
            value.timer = 0;
        }
        value.updating = true;
        value.dirty = false;
        value.measuring_size = window_size(hwnd);
        state.set(value);
        let first = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, 0, 0);
        if !state.get().attached {
            return;
        }
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let horizontal = if style & WS_HSCROLL != 0 {
            scroll_info(hwnd, SB_HORZ)
        } else {
            None
        };
        value = state.get();
        let suppress = value.redraw && style & WS_VISIBLE != 0;
        let generation = value.redraw_generation;
        if suppress {
            redraw(hwnd, state, false);
        }
        if state.get().attached {
            // Start without either bar: otherwise the two bars can consume just
            // enough space to make each other appear necessary for fitting text.
            ShowScrollBar(hwnd, SB_BOTH, 0);
        }
        if state.get().attached && SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0) != 0 {
            let mut visible = [false; 2];
            for axis in [SB_HORZ, SB_VERT, SB_HORZ] {
                if !state.get().attached {
                    break;
                }
                // Hidden axes hold stale ranges. Showing an axis makes EDIT
                // recalculate it using its font, text, margins and client area.
                let other_visible = visible[1 - axis as usize];
                show_bar(hwnd, state, axis, true, other_visible);
                if !state.get().attached {
                    break;
                }
                let mut info = SCROLLINFO {
                    cbSize: size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_RANGE | SIF_PAGE,
                    ..Default::default()
                };
                let needed = GetScrollInfo(hwnd, axis, &mut info) == 0
                    || !fits(info.nMin, info.nMax, info.nPage);
                if !needed {
                    show_bar(hwnd, state, axis, false, other_visible);
                }
                visible[axis as usize] = needed;
            }
        }
        restore_scroll(hwnd, state, first, horizontal);
        value = state.get();
        if suppress && value.attached && value.redraw && value.redraw_generation == generation {
            redraw(hwnd, state, true);
        }
        value = state.get();
        value.updating = false;
        state.set(value);
        if value.attached && value.dirty {
            post_refresh(hwnd, state);
        }
        if value.attached && value.redraw {
            // Queue one final paint; do not draw intermediate client/NC frames.
            RedrawWindow(
                hwnd,
                null(),
                std::ptr::null_mut(),
                RDW_INVALIDATE | RDW_FRAME | RDW_NOCHILDREN,
            );
        }
    }
}

fn changes_extent(message: u32) -> bool {
    matches!(
        message,
        WM_SIZE
            | WM_SETTEXT
            | WM_SETFONT
            | WM_THEMECHANGED
            | WM_SETTINGCHANGE
            | WM_DPICHANGED
            | WM_DPICHANGED_AFTERPARENT
            | EM_REPLACESEL
            | EM_SETMARGINS
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
    // SAFETY: Our owner-thread subclass owns one Rc. Retain BEFORE forwarding;
    // native processing or another subclass may synchronously destroy the EDIT.
    unsafe {
        let pointer = data as *const Cell<Presentation>;
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        let mut value = state.get();
        if message == WM_NCDESTROY {
            value.attached = false;
            state.set(value);
            if value.timer != 0 {
                KillTimer(hwnd, value.timer);
            }
            RemoveWindowSubclass(hwnd, Some(procedure), id);
            drop(Rc::from_raw(pointer));
            return DefSubclassProc(hwnd, message, w, l);
        }
        if message == REFRESH || (message == WM_TIMER && w == value.timer && value.timer != 0) {
            if message == REFRESH {
                value.pending = false;
                state.set(value);
            }
            synchronize(hwnd, &state);
            return 0;
        }
        if message == WM_SETREDRAW {
            if value.internal_redraw.take() != Some(w != 0) {
                value.redraw = w != 0;
                value.redraw_generation = value.redraw_generation.wrapping_add(1);
            }
            state.set(value);
        }
        let internal_size =
            value.updating && message == WM_SIZE && window_size(hwnd) == value.measuring_size;
        let changes = !internal_size && changes_extent(message);
        if changes {
            value.depth = value.depth.saturating_add(1);
            value.dirty = true;
            state.set(value);
        }
        let result = DefSubclassProc(hwnd, message, w, l);
        if changes {
            value = state.get();
            value.depth = value.depth.saturating_sub(1);
            state.set(value);
            if message == WM_SIZE {
                // A deferred window-position batch may still be committing other
                // children. Changing NC geometry here can overwrite its size.
                post_refresh(hwnd, &state);
            } else {
                synchronize(hwnd, &state);
            }
        } else if matches!(message, WM_CAPTURECHANGED | WM_LBUTTONUP | WM_CANCELMODE)
            && state.get().dirty
        {
            post_refresh(hwnd, &state);
        }
        result
    }
}

#[cfg(test)]
mod tests;
