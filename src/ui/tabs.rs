//! 原生 Common Controls Tab；页面控件只隐藏，不销毁或重新填充。
use super::theme::{Palette, Resolved};
use super::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use windows_sys::Win32::{Graphics::Gdi::*, UI::Controls::*};

mod icons;
#[cfg(test)]
mod tests;

pub(super) const ID: u32 = 2100;
pub(super) const START: u32 = 2102;
pub(super) const STOP: u32 = 2103;
pub(super) const LABELS: [&str; 4] = ["概览", "统计分析", "实时日志", "设置"];

#[derive(Default)]
pub(super) struct Tabs {
    pub handle: HWND,
    pub active: usize,
    pub start: HWND,
    pub stop: HWND,
}
impl Tabs {
    pub fn create(&mut self, parent: HWND) -> Result<(), String> {
        let init = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TAB_CLASSES,
        };
        // SAFETY: Sized initialization structure and live parent; strings are copied synchronously.
        unsafe {
            if InitCommonControlsEx(&init) == 0 {
                return Err(last_error("初始化原生分页控件"));
            }
            self.handle = CreateWindowExW(
                0,
                wide("SysTabControl32").as_ptr(),
                null(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | TCS_FIXEDWIDTH,
                0,
                0,
                0,
                0,
                parent,
                ID as usize as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            if self.handle.is_null() {
                return Err(last_error("创建原生分页控件"));
            }
            for (index, title) in LABELS.iter().enumerate() {
                let mut title = wide(title);
                let item = TCITEMW {
                    mask: TCIF_TEXT,
                    pszText: title.as_mut_ptr(),
                    ..Default::default()
                };
                if SendMessageW(
                    self.handle,
                    TCM_INSERTITEMW,
                    index,
                    (&item as *const TCITEMW) as isize,
                ) == -1
                {
                    return Err(last_error("添加分页"));
                }
            }
            for (title, id, target) in [
                ("▶ 启动代理", START, &mut self.start),
                ("■ 停止代理", STOP, &mut self.stop),
            ] {
                *target = CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    wide(title).as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    0,
                    0,
                    0,
                    0,
                    parent,
                    id as usize as HMENU,
                    GetModuleHandleW(null()),
                    null(),
                );
                if target.is_null() {
                    return Err(last_error("创建概览操作"));
                }
            }
        }
        Ok(())
    }
    pub fn selection(&self) -> usize {
        // SAFETY: Live native Tab, no pointer arguments.
        unsafe { SendMessageW(self.handle, TCM_GETCURSEL, 0, 0).clamp(0, 3) as usize }
    }
    pub fn select(&mut self, index: usize) {
        self.active = index.min(3);
        // SAFETY: Live native Tab; setting selection does not emit TCN_SELCHANGE.
        unsafe {
            SendMessageW(self.handle, TCM_SETCURSEL, self.active, 0);
        }
    }
    pub fn set_font(&self, font: windows_sys::Win32::Graphics::Gdi::HFONT) {
        for control in [self.handle, self.start, self.stop] {
            // SAFETY: Live controls; State retains the font.
            unsafe {
                SendMessageW(control, WM_SETFONT, font as usize, 0);
            }
        }
    }
    pub fn layout(&self, dpi: u32, width: i32, left: i32) {
        // SAFETY: Live sibling controls; Tab occupies only the header, so it cannot cover page controls.
        unsafe {
            SendMessageW(
                self.handle,
                TCM_SETITEMSIZE,
                0,
                (scale(112, dpi) | (scale(36, dpi) << 16)) as isize,
            );
            MoveWindow(
                self.handle,
                left,
                0,
                (width - left - scale(24, dpi)).max(1),
                scale(44, dpi),
                1,
            );
        }
    }
    pub fn layout_actions(&self, dpi: u32, width: i32) {
        // SAFETY: Live overview-only children; positioned before that page's arrange pass.
        unsafe {
            MoveWindow(
                self.start,
                scale(24, dpi),
                scale(220, dpi),
                ((width - scale(48, dpi)) * 140 / 696).max(1),
                scale(36, dpi),
                1,
            );
            MoveWindow(
                self.stop,
                scale(24, dpi) + (width - scale(48, dpi)) * 148 / 696,
                scale(220, dpi),
                ((width - scale(48, dpi)) * 112 / 696).max(1),
                scale(36, dpi),
                1,
            );
        }
    }
}

const SUBCLASS_ID: usize = 0x5441_4253;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
// Public commctrl.h helpers; no additional windows-sys feature is required.
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn GetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: *mut usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

#[repr(C)]
struct MouseTracking {
    size: u32,
    flags: u32,
    hwnd: HWND,
    hover_time: u32,
}
const TRACK_LEAVE: u32 = 0x0000_0002;
const TRACK_CANCEL: u32 = 0x8000_0000;
// Public user32 TRACKMOUSEEVENT ABI; Windows 10/11 both support these APIs.
#[link(name = "user32")]
unsafe extern "system" {
    fn GetFocus() -> HWND;
    fn TrackMouseEvent(event: *mut MouseTracking) -> i32;
}

#[derive(Clone, Copy)]
struct Appearance {
    palette: Palette,
    dpi: u32,
}

struct Presentation {
    appearance: Cell<Option<Appearance>>,
    hovered: Cell<Option<usize>>,
    tracking_leave: Cell<bool>,
    attached: Cell<bool>,
    revision: Cell<u64>,
    icons: RefCell<icons::Cache>,
}
impl Presentation {
    fn changed(&self) {
        self.revision.set(self.revision.get().wrapping_add(1));
    }
}

unsafe fn current(hwnd: HWND, data: &Presentation) -> bool {
    // SAFETY: Only our proc/id installs Presentation. A retained Rc keeps its
    // identity stable even when another subclass synchronously destroys the HWND.
    unsafe {
        let mut pointer = 0;
        data.attached.get()
            && GetWindowSubclass(hwnd, Some(tab_proc), SUBCLASS_ID, &mut pointer) != 0
            && std::ptr::eq(pointer as *const Presentation, data)
    }
}

unsafe fn presentation(hwnd: HWND) -> Option<Rc<Presentation>> {
    // SAFETY: Owner-thread lookup; the installed subclass owns one strong count.
    // Retain before calling any API which can reenter or remove the subclass.
    unsafe {
        let mut pointer = 0;
        if GetWindowSubclass(hwnd, Some(tab_proc), SUBCLASS_ID, &mut pointer) == 0 || pointer == 0 {
            return None;
        }
        let pointer = pointer as *const Presentation;
        Rc::increment_strong_count(pointer);
        Some(Rc::from_raw(pointer))
    }
}

unsafe fn cancel_tracking(hwnd: HWND) {
    // SAFETY: Only cancels leave tracking on the caller's owner-thread HWND.
    unsafe {
        let mut event = MouseTracking {
            size: size_of::<MouseTracking>() as u32,
            flags: TRACK_LEAVE | TRACK_CANCEL,
            hwnd,
            hover_time: 0,
        };
        TrackMouseEvent(&mut event);
    }
}

unsafe fn clear_hover(hwnd: HWND, data: &Presentation) {
    // SAFETY: A retained Presentation survives cancellation's synchronous calls;
    // clear Cells before user32 so no stale hover can be restored by reentry.
    unsafe {
        data.changed();
        let changed = data.hovered.replace(None).is_some();
        let tracking = data.tracking_leave.replace(false);
        if tracking && current(hwnd, data) {
            cancel_tracking(hwnd);
        }
        if changed && current(hwnd, data) {
            InvalidateRect(hwnd, null(), 0);
        }
    }
}

/// Presentation only: no State pointer, borrowed font, input handler or layout
/// ownership. High contrast removes the subclass and restores native drawing.
pub(super) unsafe fn apply(hwnd: HWND, kind: Resolved, palette: Palette, dpi: u32) {
    // SAFETY: Called for our live UI-thread Tab. Installation transfers one Rc
    // to NCDESTROY/removal; local owners survive synchronous cancellation.
    unsafe {
        let existing = presentation(hwnd);
        if kind == Resolved::Contrast {
            if let Some(data) = existing {
                data.appearance.set(None);
                clear_hover(hwnd, &data);
                if current(hwnd, &data)
                    && data.appearance.get().is_none()
                    && RemoveWindowSubclass(hwnd, Some(tab_proc), SUBCLASS_ID) != 0
                {
                    data.attached.set(false);
                    drop(Rc::from_raw(Rc::as_ptr(&data)));
                }
            }
            return;
        }
        let appearance = Appearance {
            palette,
            dpi: dpi.clamp(96, 768),
        };
        let data = if let Some(data) = existing {
            data.appearance.set(Some(appearance));
            // Only GDI calls occur while drawing from this cache, never a window
            // message; try_borrow also permits a text-only reentrant paint fallback.
            if let Ok(mut cache) = data.icons.try_borrow_mut() {
                *cache = icons::Cache::default();
            }
            clear_hover(hwnd, &data);
            data
        } else {
            let data = Rc::new(Presentation {
                appearance: Cell::new(Some(appearance)),
                hovered: Cell::new(None),
                tracking_leave: Cell::new(false),
                attached: Cell::new(true),
                revision: Cell::new(0),
                icons: RefCell::new(icons::Cache::default()),
            });
            let pointer = Rc::into_raw(data.clone());
            if SetWindowSubclass(hwnd, Some(tab_proc), SUBCLASS_ID, pointer as usize) == 0 {
                drop(Rc::from_raw(pointer));
                return;
            }
            data
        };
        if current(hwnd, &data) {
            InvalidateRect(hwnd, null(), 1);
        }
    }
}

fn inset(rect: RECT, amount: i32) -> Option<RECT> {
    let rect = RECT {
        left: rect.left.saturating_add(amount),
        top: rect.top.saturating_add(amount),
        right: rect.right.saturating_sub(amount),
        bottom: rect.bottom.saturating_sub(amount),
    };
    (rect.right > rect.left && rect.bottom > rect.top).then_some(rect)
}

fn indicator_rect(surface: RECT, dpi: u32) -> Option<RECT> {
    let width = scale(24, dpi).min((surface.right - surface.left) - scale(8, dpi));
    let height = scale(3, dpi).min((surface.bottom - surface.top) - scale(2, dpi));
    if width <= 0 || height <= 0 {
        return None;
    }
    let left = surface.left + ((surface.right - surface.left) - width) / 2;
    let bottom = surface.bottom - scale(1, dpi);
    Some(RECT {
        left,
        top: bottom - height,
        right: left + width,
        bottom,
    })
}

unsafe fn update_hover(hwnd: HWND, data: &Presentation, point: LPARAM) -> bool {
    // SAFETY: TCM_HITTEST is the only hit-test authority. Retained presentation
    // and revision checks prevent stale writes after destruction or theme reentry.
    unsafe {
        if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_DISABLED != 0 {
            clear_hover(hwnd, data);
            return current(hwnd, data);
        }
        let revision = data.revision.get();
        let mut hit = TCHITTESTINFO {
            pt: POINT {
                x: point as u16 as i16 as i32,
                y: (point as u32 >> 16) as u16 as i16 as i32,
            },
            flags: 0,
        };
        let index = SendMessageW(
            hwnd,
            TCM_HITTEST,
            0,
            (&mut hit as *mut TCHITTESTINFO) as LPARAM,
        );
        if !current(hwnd, data) {
            return false;
        }
        if data.revision.get() != revision || data.appearance.get().is_none() {
            return true;
        }
        let mut hovered = (index >= 0 && (index as usize) < LABELS.len()).then_some(index as usize);
        if hovered.is_some() && !data.tracking_leave.get() {
            let mut event = MouseTracking {
                size: size_of::<MouseTracking>() as u32,
                flags: TRACK_LEAVE,
                hwnd,
                hover_time: 0,
            };
            data.tracking_leave.set(true);
            let tracked = TrackMouseEvent(&mut event) != 0;
            if !current(hwnd, data) {
                return false;
            }
            if data.revision.get() != revision {
                if tracked && !data.tracking_leave.get() {
                    cancel_tracking(hwnd);
                }
                return current(hwnd, data);
            }
            data.tracking_leave.set(tracked);
            if !tracked {
                hovered = None;
            }
        }
        if data.hovered.replace(hovered) != hovered {
            data.changed();
            InvalidateRect(hwnd, null(), 0);
        }
        true
    }
}

struct Snapshot {
    appearance: Appearance,
    bounds: RECT,
    items: [Option<RECT>; 4],
    selected: LRESULT,
    focused: LRESULT,
    font: HFONT,
    enabled: bool,
    show_focus: bool,
    hovered: Option<usize>,
}

unsafe fn snapshot(hwnd: HWND, data: &Presentation) -> Option<Snapshot> {
    // SAFETY: Queries can reenter other subclasses. Copy every result and reject
    // the entire snapshot after any removal/theme/geometry/font change; no GDI
    // object or mutable application/cache reference crosses a native message.
    unsafe {
        let appearance = data.appearance.get()?;
        let revision = data.revision.get();
        let valid = || current(hwnd, data) && data.revision.get() == revision;
        let mut bounds = RECT::default();
        if GetClientRect(hwnd, &mut bounds) == 0 {
            return None;
        }
        let selected = SendMessageW(hwnd, TCM_GETCURSEL, 0, 0);
        if !valid() {
            return None;
        }
        let focused = SendMessageW(hwnd, TCM_GETCURFOCUS, 0, 0);
        if !valid() {
            return None;
        }
        let font = SendMessageW(hwnd, WM_GETFONT, 0, 0) as HFONT;
        if !valid() {
            return None;
        }
        let ui_state = SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0);
        if !valid() {
            return None;
        }
        let mut items = [None; 4];
        for (index, item) in items.iter_mut().enumerate() {
            let mut rect = RECT::default();
            let found = SendMessageW(
                hwnd,
                TCM_GETITEMRECT,
                index,
                (&mut rect as *mut RECT) as LPARAM,
            );
            if !valid() {
                return None;
            }
            if found != 0 && rect.right > rect.left && rect.bottom > rect.top {
                *item = Some(rect);
            }
        }
        Some(Snapshot {
            appearance,
            bounds,
            items,
            selected,
            focused,
            font,
            enabled: GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_DISABLED == 0,
            show_focus: GetFocus() == hwnd && ui_state & UISF_HIDEFOCUS as isize == 0,
            hovered: data.hovered.get(),
        })
    }
}

struct SavedDc(HDC, i32);
impl SavedDc {
    unsafe fn new(dc: HDC) -> Option<Self> {
        // SAFETY: Borrowed paint/print DC; failure means no custom drawing at all.
        #[cfg(test)]
        if tests::FAIL_SAVE.with(Cell::get) {
            return None;
        }
        let saved = unsafe { SaveDC(dc) };
        (saved != 0).then_some(Self(dc, saved))
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: Owns exactly one successful SaveDC on the still-borrowed DC.
        unsafe { RestoreDC(self.0, self.1) };
    }
}

unsafe fn fill_round(dc: HDC, rect: RECT, color: u32, diameter: i32) {
    // SAFETY: draw selects the stock DC brush/pen into its saved, clipped DC.
    unsafe {
        SetDCBrushColor(dc, color);
        SetDCPenColor(dc, color);
        let diameter = diameter
            .min(rect.right - rect.left)
            .min(rect.bottom - rect.top);
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            diameter,
            diameter,
        );
    }
}

unsafe fn draw(dc: HDC, data: &Presentation, snapshot: Snapshot) {
    // SAFETY: All HWND queries finished before drawing; these are only GDI calls.
    // No State pointer is retained. The guard restores all caller DC state, even
    // for print DCs with nondefault colors, mapping, clip, font, brush or pen.
    unsafe {
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        let p = snapshot.appearance.palette;
        let dpi = snapshot.appearance.dpi;
        IntersectClipRect(
            dc,
            snapshot.bounds.left,
            snapshot.bounds.top,
            snapshot.bounds.right,
            snapshot.bounds.bottom,
        );
        SelectObject(dc, GetStockObject(DC_BRUSH));
        SelectObject(dc, GetStockObject(DC_PEN));
        SetDCBrushColor(dc, p.background);
        FillRect(dc, &snapshot.bounds, GetStockObject(DC_BRUSH));
        SelectObject(
            dc,
            if snapshot.font.is_null() {
                GetStockObject(DEFAULT_GUI_FONT)
            } else {
                snapshot.font
            },
        );
        SetBkMode(dc, TRANSPARENT as i32);
        for (index, title) in LABELS.iter().enumerate() {
            let Some(rect) = snapshot.items[index] else {
                continue;
            };
            let active = snapshot.selected == index as isize;
            let hot = snapshot.enabled && snapshot.hovered == Some(index);
            if let Some(surface) = inset(rect, scale(3, dpi)) {
                if active || hot {
                    fill_round(
                        dc,
                        surface,
                        if active { p.surface } else { p.hover() },
                        scale(8, dpi),
                    );
                }
                if active && let Some(indicator) = indicator_rect(surface, dpi) {
                    fill_round(
                        dc,
                        indicator,
                        if snapshot.enabled {
                            p.accent
                        } else {
                            p.disabled
                        },
                        scale(3, dpi),
                    );
                }
            }
            let color = if snapshot.enabled && (active || hot) {
                p.text
            } else {
                p.disabled
            };
            SetTextColor(dc, color);
            let title = wide(title);
            let mut measured = RECT::default();
            DrawTextW(
                dc,
                title.as_ptr(),
                -1,
                &mut measured,
                DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
            );
            let size = icons::icon_size(dpi);
            let gap = scale(6, dpi);
            let text_width = measured.right - measured.left;
            let content_width = size + gap + text_width;
            let mut text_rect = rect;
            // Artwork insets never change native item rectangles or hit targets.
            // Insufficient space/resources retain the full centered text fallback.
            if content_width <= rect.right - rect.left - scale(8, dpi)
                && size <= rect.bottom - rect.top
            {
                let left = rect.left + (rect.right - rect.left - content_width) / 2;
                let top = rect.top + (rect.bottom - rect.top - size) / 2;
                let icon = RECT {
                    left,
                    top,
                    right: left + size,
                    bottom: top + size,
                };
                let drawn = data
                    .icons
                    .try_borrow_mut()
                    .is_ok_and(|mut cache| cache.draw(index, dc, &icon, dpi, color));
                if drawn {
                    text_rect.left = left + size + gap;
                    text_rect.right = text_rect.left + text_width;
                }
            }
            DrawTextW(
                dc,
                title.as_ptr(),
                -1,
                &mut text_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
            if snapshot.enabled
                && snapshot.show_focus
                && snapshot.focused == index as isize
                && let Some(focus) = inset(rect, scale(1, dpi))
            {
                SetTextColor(dc, p.text);
                SetBkColor(dc, p.background);
                DrawFocusRect(dc, &focus);
            }
        }
    }
}

unsafe extern "system" fn tab_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    pointer: usize,
) -> LRESULT {
    // SAFETY: Subclass owns one Rc on the UI thread; retain before dispatch so
    // nested NCDESTROY cannot free its identity. Native input, TCM, accessibility
    // and notification messages are always forwarded unchanged.
    unsafe {
        let pointer = pointer as *const Presentation;
        Rc::increment_strong_count(pointer);
        let data = Rc::from_raw(pointer);
        if message == WM_NCDESTROY {
            if data.attached.replace(false) {
                data.appearance.set(None);
                data.hovered.set(None);
                data.tracking_leave.set(false);
                data.changed();
                RemoveWindowSubclass(hwnd, Some(tab_proc), id);
                drop(Rc::from_raw(pointer));
            }
            return DefSubclassProc(hwnd, message, w, l);
        }
        if data.appearance.get().is_some() {
            if message == WM_PRINT {
                if l == 0 || (l & PRF_CHECKVISIBLE as LPARAM != 0 && IsWindowVisible(hwnd) == 0) {
                    return 0;
                }
                let Some(_saved) = SavedDc::new(w as HDC) else {
                    return 0;
                };
                // Preserve native non-client/child print routing as well as the
                // caller's DC around any native WM_PRINTCLIENT dispatch.
                return DefSubclassProc(hwnd, message, w, l);
            }

            if message == WM_PAINT || message == WM_PRINTCLIENT {
                if message == WM_PRINTCLIENT
                    && (l & PRF_CLIENT as LPARAM == 0
                        || (l & PRF_CHECKVISIBLE as LPARAM != 0 && IsWindowVisible(hwnd) == 0))
                {
                    return 0;
                }
                let mut paint = PAINTSTRUCT::default();
                let dc = if message == WM_PAINT {
                    BeginPaint(hwnd, &mut paint)
                } else {
                    w as HDC
                };
                if !dc.is_null()
                    && let Some(snapshot) = snapshot(hwnd, &data)
                {
                    draw(dc, &data, snapshot);
                }
                if message == WM_PAINT {
                    EndPaint(hwnd, &paint);
                }
                return 0;
            }
            if message == WM_ERASEBKGND {
                if let Some(appearance) = data.appearance.get()
                    && let Some(_saved) = SavedDc::new(w as HDC)
                {
                    let mut bounds = RECT::default();
                    GetClientRect(hwnd, &mut bounds);
                    SetDCBrushColor(w as HDC, appearance.palette.background);
                    FillRect(w as HDC, &bounds, GetStockObject(DC_BRUSH));
                    return 1;
                }
                return 0;
            }
        }
        // Native input is dispatched exactly once, before passive presentation
        // work: even cancellation/theme reentry cannot swallow a font/key/message.
        let result = DefSubclassProc(hwnd, message, w, l);
        if !current(hwnd, &data) {
            return result;
        }
        match message {
            WM_MOUSELEAVE => {
                data.tracking_leave.set(false);
                clear_hover(hwnd, &data);
            }
            WM_CANCELMODE => clear_hover(hwnd, &data),
            WM_ENABLE | WM_SHOWWINDOW if w == 0 => clear_hover(hwnd, &data),
            WM_SIZE
            | WM_SETFONT
            | WM_STYLECHANGED
            | WM_THEMECHANGED
            | WM_DPICHANGED_AFTERPARENT
            | TCM_SETITEMSIZE
            | TCM_SETPADDING => clear_hover(hwnd, &data),
            WM_MOUSEMOVE if data.appearance.get().is_some() && !update_hover(hwnd, &data, l) => {
                return result;
            }
            _ => {}
        }
        if matches!(
            message,
            WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_UPDATEUISTATE
                | WM_ENABLE
                | TCM_SETCURSEL
                | TCM_SETCURFOCUS
        ) && current(hwnd, &data)
        {
            data.changed();
            InvalidateRect(hwnd, null(), 0);
        }
        result
    }
}
