use super::*;

pub(super) unsafe fn synchronize(hwnd: HWND, state: &State) {
    // SAFETY: retain one presentation owner through synchronous style callbacks.
    // Nested theme requests update `enabled` but cannot remove this owner while
    // a pending style write can still commit. Reconcile the latest intent after.
    unsafe {
        let mut value = state.get();
        if value.changing_style || !value.attached {
            return;
        }
        value.changing_style = true;
        state.set(value);
        let _deferred = PaintDeferral::new(state);
        let mut pending = false;
        for _ in 0..4 {
            let enabled = state.get().enabled;
            if enabled {
                attach(hwnd, state);
            } else {
                restore(hwnd, state);
            }
            if !state.get().attached {
                break;
            }
            pending = enabled != state.get().enabled;
            if !pending {
                break;
            }
        }
        let mut value = state.get();
        value.changing_style = false;
        if pending && value.attached && !value.style_queued {
            value.style_queued = PostMessageW(hwnd, SYNC_STYLE, 0, 0) != 0;
        }
        state.set(value);
    }
}

unsafe fn attach(hwnd: HWND, state: &State) {
    // SAFETY: owner-thread known ListView; add only a documented buffering style.
    unsafe {
        if driver(hwnd) != Some(Driver::Report) || state.get().composited {
            return;
        }
        let class = GetClassLongPtrW(hwnd, GCL_STYLE) as u32;
        if class & (CS_OWNDC | CS_CLASSDC | CS_PARENTDC) != 0 {
            return;
        }
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if style as u32 & WS_EX_COMPOSITED == 0 {
            let _deferred = PaintDeferral::new(state);
            let mut value = state.get();
            value.composited = true;
            state.set(value);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_COMPOSITED as isize);
            // A successful zero-old-style return is ambiguous and callbacks can
            // change last-error. Verify the resulting style on the live instance.
            if state.get().attached
                && GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_COMPOSITED == 0
            {
                let mut value = state.get();
                value.composited = false;
                state.set(value);
            }
        }
    }
}

unsafe fn restore(hwnd: HWND, state: &State) {
    // SAFETY: remove only our own style before detaching the live owner-thread HWND.
    unsafe {
        let mut value = state.get();
        if value.composited && value.attached && IsWindow(hwnd) != 0 {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !(WS_EX_COMPOSITED as isize));
            if state.get().attached {
                value = state.get();
                value.composited =
                    GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_COMPOSITED != 0;
                state.set(value);
            }
        }
    }
}

pub(super) struct ScrollPaint(WindowDc);
impl ScrollPaint {
    pub(super) unsafe fn begin(hwnd: HWND, state: &State, message: u32, w: WPARAM) -> Option<Self> {
        // SAFETY: keep a short-lived composited DC until native drawing and the
        // dark overlay finish. No redraw/visibility/range state is changed.
        unsafe {
            let value = state.get();
            if !value.attached
                || !value.enabled
                || !value.redraw
                || value.defer_paint != 0
                || GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_COMPOSITED == 0
                || geometry_message(message)
                || !repaints_bar(message)
                // Keyboard context menus can run a modal loop. Only navigation
                // keys may retain a DC across the native keyboard dispatcher.
                || (message == WM_KEYDOWN && !matches!(w, 0x21..=0x28))
            {
                return None;
            }
            let dc = GetWindowDC(hwnd);
            (!dc.is_null()).then_some(Self(WindowDc(hwnd, dc)))
        }
    }
    pub(super) fn dc(&self) -> HDC {
        self.0.1
    }
}

struct Region(HRGN);
impl Drop for Region {
    fn drop(&mut self) {
        // SAFETY: only owned regions, never the message's borrowed update region.
        unsafe { DeleteObject(self.0) };
    }
}

/// Let native paint its border, but never the scrollbar strips we own. Returning
/// None is a readable native fallback if geometry/allocation fails.
pub(super) unsafe fn nonclient(hwnd: HWND, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
    // SAFETY: public geometry queries and owned GDI regions; no borrowed region
    // is modified or transferred. WM_NCPAINT uses screen-coordinate regions.
    unsafe {
        let mut window = RECT::default();
        if GetWindowRect(hwnd, &mut window) == 0 {
            return None;
        }
        let origin = POINT::default();
        let vertical = bar(hwnd, true, origin, window);
        let horizontal = bar(hwnd, false, origin, window);
        if vertical.is_none() && horizontal.is_none() {
            return None;
        }
        let region = Region(CreateRectRgnIndirect(&window));
        let subtract = Region(CreateRectRgn(0, 0, 0, 0));
        if region.0.is_null() || subtract.0.is_null() {
            return None;
        }
        if w != 1 && CombineRgn(region.0, region.0, w as HRGN, RGN_AND) == ERROR {
            return None;
        }
        let corner = vertical.zip(horizontal).map(|(v, h)| RECT {
            left: v.rect.left,
            right: v.rect.right,
            top: h.rect.top,
            bottom: h.rect.bottom,
        });
        for rect in [vertical.map(|b| b.rect), horizontal.map(|b| b.rect), corner]
            .into_iter()
            .flatten()
        {
            if SetRectRgn(subtract.0, rect.left, rect.top, rect.right, rect.bottom) == 0 {
                return None;
            }
            if CombineRgn(region.0, region.0, subtract.0, RGN_DIFF) == ERROR {
                return None;
            }
        }
        Some(DefSubclassProc(hwnd, WM_NCPAINT, region.0 as WPARAM, l))
    }
}
