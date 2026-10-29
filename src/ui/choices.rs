//! Native Tab-backed segmented choices. Input and MSAA remain native; only the
//! header is painted. The compatibility messages cover fixed, Unicode lists,
//! not the mutable ComboBox API. All entry points belong to the HWND owner thread.
use super::theme::{Palette, Resolved, blend};
use crate::windows::{last_error, scale, wide};
use std::{
    cell::Cell,
    mem::size_of,
    ptr::{null, null_mut},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{Controls::*, HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

const PROPERTY: *const u16 = windows_sys::w!("AIRelay.Choices.v1");
const SUBCLASS_ID: usize = 0x4348_4f49;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
// Public commctrl.h helpers; their windows-sys module needs an additional feature.
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn GetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: *mut usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}
// Public TRACKMOUSEEVENT ABI; avoid requiring another windows-sys feature.
#[repr(C)]
struct MouseTracking {
    size: u32,
    flags: u32,
    hwnd: HWND,
    hover_time: u32,
}
const TRACK_LEAVE: u32 = 0x0000_0002;
const TRACK_CANCEL: u32 = 0x8000_0000;
#[link(name = "user32")]
unsafe extern "system" {
    fn GetFocus() -> HWND;
    fn TrackMouseEvent(event: *mut MouseTracking) -> i32;
}

#[derive(Clone, Copy)]
struct Appearance {
    kind: Resolved,
    palette: Palette,
    dpi: u32,
}
struct Choices {
    // Both the native Tab and this immutable list own copies of the item text.
    // The list supports ComboBox's unbounded GETLBTEXT contract without truncation.
    labels: Vec<Vec<u16>>,
    appearance: Cell<Option<Appearance>>,
    dpi: Cell<u32>,
    hovered: Cell<Option<usize>>,
    tracking_leave: Cell<bool>,
    attached: Cell<bool>,
}

fn item_size(width: i32, height: i32, count: usize, dpi: u32) -> (u16, u16) {
    let inset = scale(4, dpi);
    let width =
        (width.max(0).saturating_sub(inset) / count.max(1) as i32).clamp(1, u16::MAX as i32);
    let height = scale(30, dpi)
        .min(height.max(0))
        .saturating_sub(inset)
        .clamp(1, u16::MAX as i32);
    (width as u16, height as u16)
}
fn packed_size(width: u16, height: u16) -> LPARAM {
    (u32::from(width) | (u32::from(height) << 16)) as LPARAM
}
fn visible_rect(rect: RECT) -> Option<RECT> {
    (rect.right > rect.left && rect.bottom > rect.top).then_some(rect)
}
fn track_rect(bounds: RECT, items: RECT, dpi: u32) -> Option<RECT> {
    let margin = scale(1, dpi);
    visible_rect(RECT {
        left: items.left.saturating_sub(margin).max(bounds.left + margin),
        top: items.top.saturating_sub(margin).max(bounds.top + margin),
        right: items
            .right
            .saturating_add(margin)
            .min(bounds.right - margin),
        bottom: items
            .bottom
            .saturating_add(margin)
            .min(bounds.bottom - margin),
    })
}
fn surface_rect(item: RECT, track: RECT, dpi: u32) -> Option<RECT> {
    let gap = scale(1, dpi);
    let inset = scale(2, dpi);
    visible_rect(RECT {
        left: (item.left + gap).max(track.left + inset),
        top: track.top + inset,
        right: (item.right - gap).min(track.right - inset),
        bottom: track.bottom - inset,
    })
}
fn label_rect(surface: RECT, dpi: u32) -> RECT {
    // Keep narrow labels readable: the former 7 DIP padding cost 14 DIP per item.
    let padding = scale(2, dpi).min((surface.right - surface.left - 1).max(0) / 2);
    RECT {
        left: surface.left + padding,
        right: surface.right - padding,
        ..surface
    }
}
fn copy_label(label: &[u16], output: &mut [u16]) -> usize {
    if output.is_empty() {
        return 0;
    }
    let count = (label.len() - 1).min(output.len() - 1);
    output[..count].copy_from_slice(&label[..count]);
    output[count] = 0;
    count
}

/// Creates a hidden child; the page owns visibility, placement and font lifetime.
/// `parent` must be a live window on this thread. IDs must fit WM_COMMAND's low
/// word, and items must be nonempty and contain no embedded NUL. The list is fixed.
pub(super) fn create(parent: HWND, id: u32, items: &[&str]) -> Result<HWND, String> {
    if items.is_empty() || items.len() > u16::MAX as usize || id > u16::MAX as u32 {
        return Err("分段选择需要非空选项及 16 位控件 ID".into());
    }
    if items.iter().any(|item| item.contains('\0')) {
        return Err("分段选择文本不能包含 NUL".into());
    }
    let labels: Vec<_> = items.iter().map(|item| wide(item)).collect();
    // SAFETY: Window APIs validate the opaque parent handle. Creation/subclassing
    // is allowed only after confirming the parent belongs to the current thread.
    // Native insertion copies TCITEMW text before SendMessageW returns.
    unsafe {
        if parent.is_null() || GetWindowThreadProcessId(parent, null_mut()) != GetCurrentThreadId()
        {
            return Err("分段选择父窗口必须属于当前界面线程".into());
        }
        let init = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TAB_CLASSES,
        };
        if InitCommonControlsEx(&init) == 0 {
            return Err(last_error("初始化分段选择"));
        }
        let hwnd = CreateWindowExW(
            0,
            windows_sys::w!("SysTabControl32"),
            null(),
            WS_CHILD | WS_TABSTOP | WS_CLIPSIBLINGS | TCS_FIXEDWIDTH,
            0,
            0,
            0,
            0,
            parent,
            id as usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        if hwnd.is_null() {
            return Err(last_error("创建分段选择"));
        }
        for (index, label) in labels.iter().enumerate() {
            let item = TCITEMW {
                mask: TCIF_TEXT,
                pszText: label.as_ptr() as *mut u16,
                ..Default::default()
            };
            if SendMessageW(
                hwnd,
                TCM_INSERTITEMW,
                index,
                (&item as *const TCITEMW) as LPARAM,
            ) == -1
            {
                let error = last_error("添加分段选项");
                DestroyWindow(hwnd);
                return Err(error);
            }
        }
        let data = Rc::into_raw(Rc::new(Choices {
            labels,
            appearance: Cell::new(None),
            dpi: Cell::new(GetDpiForWindow(parent).max(96)),
            hovered: Cell::new(None),
            tracking_leave: Cell::new(false),
            attached: Cell::new(true),
        }));
        if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, data as usize) == 0 {
            let error = last_error("安装分段选择绘制");
            drop(Rc::from_raw(data));
            DestroyWindow(hwnd);
            return Err(error);
        }
        // Subclass now owns one Rc, including failures after this point.
        if SetPropW(hwnd, PROPERTY, data.cast_mut().cast()) == 0 {
            let error = last_error("标记分段选择");
            DestroyWindow(hwnd);
            return Err(error);
        }
        SendMessageW(hwnd, TCM_SETCURSEL, 0, 0);
        Ok(hwnd)
    }
}

/// Returns only this component's data, never a native main Tab's State pointer.
unsafe fn marked(hwnd: HWND) -> *const Choices {
    // SAFETY: Same-thread HWND query; no property pointer is dereferenced until
    // it matches the data registered for our specific subclass procedure and ID.
    unsafe {
        let property = GetPropW(hwnd, PROPERTY);
        let mut data = 0;
        if !property.is_null()
            && GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) != 0
            && data == property as usize
        {
            property.cast()
        } else {
            null()
        }
    }
}

/// Call first in the parent's WM_NOTIFY handler. `l` must be zero or point to
/// that message's live NMHDR. Returns false for every unmarked native Tab.
pub(super) unsafe fn notify(l: LPARAM) -> bool {
    // SAFETY: Caller supplies WM_NOTIFY's NMHDR for the duration of this call.
    // Posting (rather than sending) avoids reentering the parent's mutable state.
    unsafe {
        if l == 0 {
            return false;
        }
        let header = &*(l as *const NMHDR);
        if header.code != TCN_SELCHANGE || marked(header.hwndFrom).is_null() {
            return false;
        }
        let hwnd = header.hwndFrom;
        let id = GetDlgCtrlID(hwnd) as u32;
        InvalidateRect(hwnd, null(), 0);
        PostMessageW(
            GetParent(hwnd),
            WM_COMMAND,
            (id | (CBN_SELCHANGE << 16)) as WPARAM,
            hwnd as LPARAM,
        );
        true
    }
}

/// UI thread only. Unmarked windows are ignored. Palette/DPI are copied and no
/// State borrow, font ownership or GDI allocation is retained by the subclass.
pub(super) unsafe fn apply(hwnd: HWND, kind: Resolved, palette: Palette, dpi: u32) {
    // SAFETY: Caller stays on the HWND owner thread; marked verifies provenance.
    // Retain the owner before sending messages that could destroy the subclass.
    unsafe {
        let data = marked(hwnd);
        if data.is_null() {
            return;
        }
        Rc::increment_strong_count(data);
        let data = Rc::from_raw(data);
        clear_hover(hwnd, &data);
        data.dpi.set(dpi.max(96));
        data.appearance.set(Some(Appearance {
            kind,
            palette,
            dpi: dpi.max(96),
        }));
        resize(hwnd, data.labels.len(), data.dpi.get());
        if data.attached.get() {
            InvalidateRect(hwnd, null(), 1);
        }
    }
}

unsafe fn resize(hwnd: HWND, count: usize, dpi: u32) {
    // SAFETY: Owner-thread Tab and writable local rectangle; SETITEMSIZE copies
    // scalar dimensions and does not resize the child or recursively send WM_SIZE.
    unsafe {
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect);
        let (width, height) = item_size(rect.right, rect.bottom, count, dpi);
        SendMessageW(hwnd, TCM_SETITEMSIZE, 0, packed_size(width, height));
    }
}

unsafe fn native_item_rect(hwnd: HWND, index: usize) -> Option<RECT> {
    // SAFETY: Owner-thread Tab; GETITEMRECT writes only this live stack rectangle.
    unsafe {
        let mut rect = RECT::default();
        if SendMessageW(
            hwnd,
            TCM_GETITEMRECT,
            index,
            (&mut rect as *mut RECT) as LPARAM,
        ) == 0
        {
            return None;
        }
        visible_rect(rect)
    }
}

unsafe fn clear_hover(hwnd: HWND, data: &Choices) {
    // SAFETY: Owner-thread HWND and retained subclass data. Cell values are copied
    // before calling user32; cancellation only concerns this HWND.
    unsafe {
        let changed = data.hovered.replace(None).is_some();
        let tracking = data.tracking_leave.replace(false);
        if tracking {
            let mut event = MouseTracking {
                size: size_of::<MouseTracking>() as u32,
                flags: TRACK_LEAVE | TRACK_CANCEL,
                hwnd,
                hover_time: 0,
            };
            TrackMouseEvent(&mut event);
        }
        if changed {
            InvalidateRect(hwnd, null(), 0);
        }
    }
}

unsafe fn update_hover(hwnd: HWND, data: &Choices, point: LPARAM) -> bool {
    // SAFETY: The caller retains this instance across synchronous hit testing.
    // Native rectangles, including signed coordinates, remain the hit-test source.
    unsafe {
        if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_DISABLED != 0 {
            clear_hover(hwnd, data);
            return true;
        }
        let mut hit = TCHITTESTINFO {
            pt: POINT {
                x: point as u16 as i16 as i32,
                y: ((point as u32 >> 16) as u16) as i16 as i32,
            },
            flags: 0,
        };
        let index = SendMessageW(
            hwnd,
            TCM_HITTEST,
            0,
            (&mut hit as *mut TCHITTESTINFO) as LPARAM,
        );
        // An outer subclass can destroy the Tab during HITTEST. The retained Rc
        // prevents allocation reuse; HWND validity alone cannot identify this owner.
        if !data.attached.get() || !std::ptr::eq(marked(hwnd), data) {
            return false;
        }
        if data
            .appearance
            .get()
            .is_none_or(|a| a.kind == Resolved::Contrast)
            || GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_DISABLED != 0
        {
            clear_hover(hwnd, data);
            return true;
        }
        let mut hovered =
            (index >= 0 && (index as usize) < data.labels.len()).then_some(index as usize);
        if hovered.is_some() && !data.tracking_leave.get() {
            let mut event = MouseTracking {
                size: size_of::<MouseTracking>() as u32,
                flags: TRACK_LEAVE,
                hwnd,
                hover_time: 0,
            };
            data.tracking_leave.set(TrackMouseEvent(&mut event) != 0);
            if !data.tracking_leave.get() {
                hovered = None;
            }
        }
        if data.hovered.replace(hovered) != hovered {
            InvalidateRect(hwnd, null(), 0);
        }
        true
    }
}

unsafe fn fill_round(dc: HDC, rect: RECT, color: u32, diameter: i32) {
    // SAFETY: draw selected the borrowed DC pen/brush into this live saved DC.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Colors {
    fill: u32,
    ink: u32,
}

fn colors(p: Palette, kind: Resolved, active: bool, enabled: bool, hot: bool) -> Colors {
    let track = p.subtle();
    // A dark selected segment needs elevation above the neutral track, not blue ink.
    let raised = if kind == Resolved::Dark {
        blend(p.surface, p.text, 8)
    } else {
        p.surface
    };
    if !enabled {
        return Colors {
            fill: if active {
                blend(track, raised, 55)
            } else {
                track
            },
            ink: p.disabled,
        };
    }
    Colors {
        fill: if active && hot {
            blend(raised, p.text, 3)
        } else if active {
            raised
        } else if hot {
            p.hover()
        } else {
            track
        },
        ink: if active {
            p.text
        } else {
            blend(p.text, p.disabled, 50)
        },
    }
}

struct SavedDc(HDC, i32);
impl SavedDc {
    unsafe fn new(dc: HDC) -> Option<Self> {
        // SAFETY: Borrowed paint/print DC; leave it untouched if saving fails.
        let index = unsafe { SaveDC(dc) };
        (index != 0).then_some(Self(dc, index))
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: This guard owns one saved state and cannot outlive the borrowed DC.
        unsafe { RestoreDC(self.0, self.1) };
    }
}

unsafe fn draw(hwnd: HWND, dc: HDC, data: &Choices, appearance: Appearance) {
    // SAFETY: Paint/print supplies a live DC. Choices outlives this callback;
    // no mutable reference crosses native messages. Stock GDI objects are borrowed,
    // and SaveDC/RestoreDC restores selections/colors before this function returns.
    unsafe {
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        let palette = appearance.palette;
        let mut bounds = RECT::default();
        GetClientRect(hwnd, &mut bounds);
        SelectObject(dc, GetStockObject(DC_BRUSH));
        SelectObject(dc, GetStockObject(DC_PEN));
        SetDCBrushColor(dc, palette.background);
        FillRect(dc, &bounds, GetStockObject(DC_BRUSH));
        let count = data.labels.len();
        let mut items: Option<RECT> = None;
        for index in 0..count {
            if let Some(rect) = native_item_rect(hwnd, index) {
                items = Some(match items {
                    Some(items) => RECT {
                        left: items.left.min(rect.left),
                        top: items.top.min(rect.top),
                        right: items.right.max(rect.right),
                        bottom: items.bottom.max(rect.bottom),
                    },
                    None => rect,
                });
            }
        }
        let Some(track) = items.and_then(|items| track_rect(bounds, items, appearance.dpi)) else {
            return;
        };
        let track_color = palette.subtle();
        fill_round(dc, track, track_color, scale(12, appearance.dpi));
        let font = SendMessageW(hwnd, WM_GETFONT, 0, 0) as HGDIOBJ;
        SelectObject(
            dc,
            if font.is_null() {
                GetStockObject(DEFAULT_GUI_FONT)
            } else {
                font
            },
        );
        SetBkMode(dc, TRANSPARENT as i32);
        let selected = SendMessageW(hwnd, TCM_GETCURSEL, 0, 0);
        let focused = SendMessageW(hwnd, TCM_GETCURFOCUS, 0, 0);
        let enabled = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_DISABLED == 0;
        let show_focus = enabled
            && GetFocus() == hwnd
            && SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) & UISF_HIDEFOCUS as isize == 0;
        let hovered = data.hovered.get();
        for index in 0..count {
            let Some(mut rect) = native_item_rect(hwnd, index)
                .and_then(|item| surface_rect(item, track, appearance.dpi))
            else {
                continue;
            };
            // Inset only the artwork; native rectangles still define hit testing.
            let active = selected == index as isize;
            let hot = enabled && hovered == Some(index);
            let c = colors(palette, appearance.kind, active, enabled, hot);
            let diameter = scale(8, appearance.dpi);
            if active && enabled {
                let offset = scale(1, appearance.dpi);
                let shadow = RECT {
                    top: rect.top + offset,
                    bottom: (rect.bottom + offset).min(track.bottom - offset),
                    ..rect
                };
                fill_round(dc, shadow, blend(track_color, 0, 10), diameter);
            }
            if active || hot {
                fill_round(dc, rect, c.fill, diameter);
            }
            SetTextColor(dc, c.ink);
            let mut text = label_rect(rect, appearance.dpi);
            if text.right > text.left {
                let label = &data.labels[index];
                DrawTextW(
                    dc,
                    label.as_ptr(),
                    (label.len() - 1) as i32,
                    &mut text,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
            }
            if show_focus && focused == index as isize {
                InflateRect(
                    &mut rect,
                    -scale(2, appearance.dpi),
                    -scale(2, appearance.dpi),
                );
                if rect.right > rect.left && rect.bottom > rect.top {
                    // Focus stays separate from selection and hover, including when
                    // native keyboard navigation focuses an unselected item.
                    SetTextColor(dc, palette.text);
                    SetBkColor(dc, c.fill);
                    DrawFocusRect(dc, &rect);
                }
            }
        }
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
    // SAFETY: The installed owner holds one Rc. Retain it before synchronous
    // dispatch so nested NCDESTROY cannot free this callback's data or reuse its
    // identity. Cell access never holds an exclusive borrow across native calls.
    // Pointer-bearing messages retain their documented Win32 caller contracts.
    unsafe {
        let pointer = data as *const Choices;
        Rc::increment_strong_count(pointer);
        let data = Rc::from_raw(pointer);
        match message {
            WM_NCDESTROY => {
                if data.attached.replace(false) {
                    clear_hover(hwnd, &data);
                    RemovePropW(hwnd, PROPERTY);
                    RemoveWindowSubclass(hwnd, Some(procedure), id);
                    drop(Rc::from_raw(pointer));
                }
                return DefSubclassProc(hwnd, message, w, l);
            }
            CB_GETCURSEL => return SendMessageW(hwnd, TCM_GETCURSEL, 0, 0),
            CB_SETCURSEL => {
                // TCM_SETCURSEL returns the OLD selection; ComboBox returns the
                // NEW index on success. Invalid positive indices preserve selection.
                if w == usize::MAX || w < data.labels.len() {
                    SendMessageW(hwnd, TCM_SETCURSEL, w, 0);
                    InvalidateRect(hwnd, null(), 0);
                    return SendMessageW(hwnd, TCM_GETCURSEL, 0, 0);
                }
                return CB_ERR as LRESULT;
            }
            CB_GETCOUNT => return data.labels.len() as LRESULT,
            CB_GETLBTEXTLEN | CB_GETLBTEXT => {
                let labels = &data.labels;
                let Some(label) = labels.get(w) else {
                    return CB_ERR as LRESULT;
                };
                let count = label.len() - 1;
                if message == CB_GETLBTEXT {
                    if l == 0 {
                        return CB_ERR as LRESULT;
                    }
                    // Like native CB_GETLBTEXT, caller must allocate LEN + 1 UTF-16 units.
                    std::ptr::copy_nonoverlapping(label.as_ptr(), l as *mut u16, count + 1);
                }
                return count as LRESULT;
            }
            WM_GETTEXTLENGTH | WM_GETTEXT => {
                let selected = SendMessageW(hwnd, TCM_GETCURSEL, 0, 0);
                let labels = &data.labels;
                let label = labels
                    .get(selected as usize)
                    .map_or(&[0][..], Vec::as_slice);
                if message == WM_GETTEXTLENGTH {
                    return (label.len() - 1) as LRESULT;
                }
                if w == 0 || l == 0 {
                    return 0;
                }
                let output = std::slice::from_raw_parts_mut(l as *mut u16, w);
                return copy_label(label, output) as LRESULT;
            }
            WM_SIZE | WM_SETFONT => {
                clear_hover(hwnd, &data);
                // Native stores the borrowed HFONT; caller keeps it alive, as for any Tab.
                let result = DefSubclassProc(hwnd, message, w, l);
                if !data.attached.get() {
                    return result;
                }
                resize(hwnd, data.labels.len(), data.dpi.get());
                InvalidateRect(hwnd, null(), 0);
                return result;
            }
            WM_MOUSELEAVE => {
                data.tracking_leave.set(false);
                clear_hover(hwnd, &data);
            }
            WM_CANCELMODE => clear_hover(hwnd, &data),
            WM_ENABLE | WM_SHOWWINDOW if w == 0 => clear_hover(hwnd, &data),
            _ => {}
        }
        let appearance = data.appearance.get();
        if let Some(appearance) = appearance.filter(|a| a.kind != Resolved::Contrast) {
            if message == WM_PRINT {
                // WM_PRINT's caller lends a live DC until dispatch returns.
                // Preserve the entire native pass, not just our PRINTCLIENT
                // artwork; the retained Rc also survives nested destruction.
                let Some(_saved) = SavedDc::new(w as HDC) else {
                    return 0;
                };
                return DefSubclassProc(hwnd, message, w, l);
            }
            if message == WM_MOUSEMOVE && !update_hover(hwnd, &data, l) {
                return 0;
            }
            if message == WM_PAINT || message == WM_PRINTCLIENT {
                let mut paint = PAINTSTRUCT::default();
                let dc = if message == WM_PAINT {
                    BeginPaint(hwnd, &mut paint)
                } else {
                    w as HDC
                };
                if !dc.is_null() {
                    draw(hwnd, dc, &data, appearance);
                }
                if message == WM_PAINT {
                    EndPaint(hwnd, &paint);
                }
                return 0;
            }
            if message == WM_ERASEBKGND {
                // WM_PAINT fills the entire header; suppress the native page border.
                return 1;
            }
        }
        let result = DefSubclassProc(hwnd, message, w, l);
        if !data.attached.get() {
            return result;
        }
        if matches!(
            message,
            TCM_SETCURSEL
                | TCM_SETCURFOCUS
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_KEYDOWN
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_ENABLE
                | WM_UPDATEUISTATE
        ) {
            InvalidateRect(hwnd, null(), 0);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnableWindow(hwnd: HWND, enabled: i32) -> i32;
    }

    #[test]
    fn equal_items_fit_common_dpi_and_narrow_clients() {
        for dpi in [96, 144, 192] {
            for count in [1, 2, 3, 5] {
                let width = scale(400, dpi);
                let height = scale(30, dpi);
                let (item_width, item_height) = item_size(width, height, count, dpi);
                assert!(i32::from(item_width) * count as i32 + scale(4, dpi) <= width);
                assert_eq!(i32::from(item_height), scale(26, dpi));
                assert_eq!(
                    packed_size(item_width, item_height) as u32 & 0xffff,
                    u32::from(item_width)
                );
            }
        }
        assert_eq!(item_size(0, 0, 0, 96), (1, 1));
        assert_eq!(item_size(2, 2, 5, 192), (1, 1));
        assert_eq!(item_size(i32::MAX, i32::MAX, 1, 96).0, u16::MAX);
    }

    #[test]
    fn compact_artwork_preserves_label_space_and_stays_inside_track() {
        for dpi in [96, 144, 192] {
            for height in [24, 28, 30, 32, 36] {
                let bounds = RECT {
                    right: scale(180, dpi),
                    bottom: scale(height, dpi),
                    ..RECT::default()
                };
                let items = RECT {
                    left: scale(2, dpi),
                    top: scale(2, dpi),
                    right: bounds.right - scale(2, dpi),
                    bottom: scale(height.min(30) - 2, dpi),
                };
                let track = track_rect(bounds, items, dpi).unwrap();
                assert!(track.left >= bounds.left + scale(1, dpi));
                assert!(track.top >= bounds.top + scale(1, dpi));
                assert!(track.right <= bounds.right - scale(1, dpi));
                assert!(track.bottom <= bounds.bottom - scale(1, dpi));
                let first = RECT {
                    right: items.left + (items.right - items.left) / 3,
                    ..items
                };
                let surface = surface_rect(first, track, dpi).unwrap();
                assert!(surface.left >= first.left);
                assert!(surface.right <= first.right);
                assert_eq!(surface.top, track.top + scale(2, dpi));
                assert_eq!(surface.bottom, track.bottom - scale(2, dpi));
                let text = label_rect(surface, dpi);
                assert_eq!(
                    text.right - text.left,
                    surface.right - surface.left - scale(4, dpi)
                );
                assert!(text.bottom - text.top >= scale(17, dpi));
            }
        }
        assert!(track_rect(RECT::default(), RECT::default(), 96).is_none());
        let tiny = RECT {
            right: 2,
            bottom: 2,
            ..RECT::default()
        };
        assert!(surface_rect(tiny, tiny, 96).is_none());
        let narrow = RECT {
            right: 1,
            bottom: 20,
            ..RECT::default()
        };
        assert_eq!(label_rect(narrow, 192).right, 1);
    }

    #[test]
    fn unicode_copy_obeys_capacity_and_terminates() {
        let label = wide("本地🌏");
        assert_eq!(label.len() - 1, 4);
        assert_eq!(copy_label(&label, &mut []), 0);
        let mut only_nul = [99];
        assert_eq!(copy_label(&label, &mut only_nul), 0);
        assert_eq!(only_nul, [0]);
        let mut short = [99; 3];
        assert_eq!(copy_label(&label, &mut short), 2);
        assert_eq!(&short, &[label[0], label[1], 0]);
        let mut full = [99; 8];
        assert_eq!(copy_label(&label, &mut full), 4);
        assert_eq!(&full[..5], label.as_slice());
        assert_eq!(full[5], 99);
    }

    struct Host(HWND);
    impl Host {
        fn new() -> Self {
            // SAFETY: Creates a hidden native test parent on this test's thread.
            unsafe {
                let hwnd = CreateWindowExW(
                    0,
                    windows_sys::w!("STATIC"),
                    null(),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    640,
                    480,
                    null_mut(),
                    null_mut(),
                    GetModuleHandleW(null()),
                    null(),
                );
                assert!(!hwnd.is_null());
                assert_ne!(SetWindowSubclass(hwnd, Some(host_proc), 1, 0), 0);
                Self(hwnd)
            }
        }
    }
    impl Drop for Host {
        fn drop(&mut self) {
            // SAFETY: This test owns its parent and children on the creating thread.
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    unsafe extern "system" fn host_proc(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        id: usize,
        _data: usize,
    ) -> LRESULT {
        // SAFETY: Test subclass receives valid native messages on the owner thread.
        unsafe {
            if message == WM_NOTIFY && notify(l) {
                return 0;
            }
            if message == WM_NCDESTROY {
                RemoveWindowSubclass(hwnd, Some(host_proc), id);
            }
            DefSubclassProc(hwnd, message, w, l)
        }
    }
    fn palette() -> Palette {
        Palette {
            background: 0xf3f3f3,
            surface: 0xffffff,
            text: 0x1a1a1a,
            disabled: 0x616161,
            accent: 0xb85f00,
            border: 0xe0e0e0,
            accent_surface: 0xf9f2ea,
        }
    }

    fn dark_palette() -> Palette {
        Palette {
            background: 0x202020,
            surface: 0x2b2b2b,
            text: 0xf5f5f5,
            disabled: 0xc5c5c5,
            accent: 0xffcd60,
            border: 0x454545,
            accent_surface: 0x473d2d,
        }
    }

    fn luminance(color: u32) -> f64 {
        [0.2126, 0.7152, 0.0722]
            .into_iter()
            .enumerate()
            .map(|(index, weight)| {
                let channel = f64::from(color >> (index * 8) & 255) / 255.0;
                weight
                    * if channel <= 0.04045 {
                        channel / 12.92
                    } else {
                        ((channel + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum()
    }

    fn assert_choice_contrast(p: Palette, kind: Resolved) {
        for active in [false, true] {
            for enabled in [false, true] {
                for hot in [false, true] {
                    let c = colors(p, kind, active, enabled, hot);
                    let ink = luminance(c.ink);
                    let fill = luminance(c.fill);
                    let ratio = (ink.max(fill) + 0.05) / (ink.min(fill) + 0.05);
                    assert!(
                        ratio >= 4.5,
                        "{kind:?} active={active}, enabled={enabled}, hot={hot}: {ratio}"
                    );
                    if !enabled {
                        assert_eq!(c, colors(p, kind, active, false, !hot));
                        assert_ne!(c.ink, colors(p, kind, active, true, hot).ink);
                    }
                }
            }
        }
    }

    #[test]
    fn raised_selection_is_neutral_and_all_label_states_remain_readable() {
        for (kind, p) in [
            (Resolved::Light, palette()),
            (Resolved::Dark, dark_palette()),
        ] {
            assert_choice_contrast(p, kind);
            let selected = colors(p, kind, true, true, false);
            assert_eq!(selected.ink, p.text);
            assert!(luminance(selected.fill) > luminance(p.subtle()));
            for active in [false, true] {
                for enabled in [false, true] {
                    for hot in [false, true] {
                        let c = colors(p, kind, active, enabled, hot);
                        for color in [c.fill, c.ink] {
                            assert_eq!(color & 255, color >> 8 & 255);
                            assert_eq!(color & 255, color >> 16 & 255);
                        }
                    }
                }
            }
        }
        for mode in [
            crate::config::ThemeMode::Light,
            crate::config::ThemeMode::Dark,
        ] {
            let theme = crate::ui::theme::Theme::new(mode).unwrap();
            if theme.kind != Resolved::Contrast {
                assert_choice_contrast(theme.palette, theme.kind);
            }
        }
    }

    #[test]
    fn native_selection_combo_results_text_font_and_marker() {
        let host = Host::new();
        let hwnd = create(host.0, 42, &["全部", "本地🌏", "UTC"]).unwrap();
        // SAFETY: All HWNDs/buffers belong to this test thread; stock font is borrowed.
        unsafe {
            assert_eq!(GetParent(hwnd), host.0);
            assert_eq!(GetDlgCtrlID(hwnd), 42);
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            assert_eq!(style & WS_VISIBLE, 0);
            assert_ne!(style & WS_TABSTOP, 0);
            assert!(!marked(hwnd).is_null());
            assert_eq!(SendMessageW(hwnd, CB_GETCOUNT, 0, 0), 3);
            assert_eq!(SendMessageW(hwnd, CB_GETCURSEL, 0, 0), 0);
            assert_eq!(SendMessageW(hwnd, CB_SETCURSEL, 1, 0), 1);
            assert_eq!(SendMessageW(hwnd, CB_GETCURSEL, 0, 0), 1);
            assert_eq!(SendMessageW(hwnd, CB_SETCURSEL, 99, 0), -1);
            assert_eq!(SendMessageW(hwnd, CB_GETCURSEL, 0, 0), 1);
            assert_eq!(SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0), 4);
            let mut text = [0u16; 16];
            assert_eq!(
                SendMessageW(hwnd, WM_GETTEXT, text.len(), text.as_mut_ptr() as LPARAM),
                4
            );
            assert_eq!(String::from_utf16_lossy(&text[..4]), "本地🌏");
            assert_eq!(SendMessageW(hwnd, CB_GETLBTEXTLEN, 1, 0), 4);
            assert_eq!(
                SendMessageW(hwnd, CB_GETLBTEXT, 1, text.as_mut_ptr() as LPARAM),
                4
            );
            assert_eq!(SendMessageW(hwnd, CB_GETLBTEXTLEN, 99, 0), -1);
            // Native TCM return semantics are not rewritten by the bridge.
            assert_eq!(SendMessageW(hwnd, TCM_SETCURSEL, 2, 0), 1);
            assert_eq!(SendMessageW(hwnd, CB_SETCURSEL, usize::MAX, 0), -1);
            assert_eq!(SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0), 0);
            assert_eq!(SendMessageW(hwnd, CB_SETCURSEL, 0, 0), 0);
            let font = GetStockObject(DEFAULT_GUI_FONT);
            SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 0);
            assert_eq!(SendMessageW(hwnd, WM_GETFONT, 0, 0), font as LRESULT);
            MoveWindow(hwnd, 0, 0, 390, 30, 0);
            for kind in [Resolved::Light, Resolved::Dark, Resolved::Contrast] {
                apply(
                    hwnd,
                    kind,
                    if kind == Resolved::Dark {
                        dark_palette()
                    } else {
                        palette()
                    },
                    96,
                );
                assert_eq!(SendMessageW(hwnd, CB_GETCURSEL, 0, 0), 0);
                assert_eq!(SendMessageW(hwnd, WM_GETFONT, 0, 0), font as LRESULT);
            }
            // Other controls, including the native main Tab, are never interpreted as Choices.
            apply(host.0, Resolved::Dark, dark_palette(), 192);
            assert!(marked(host.0).is_null());
            assert_eq!(DestroyWindow(hwnd), 1);
            assert!(marked(hwnd).is_null());
        }
    }

    #[test]
    fn native_input_layout_and_print_at_common_dpi() {
        let host = Host::new();
        let hwnd = create(host.0, 91, &["全部", "本地", "UTC"]).unwrap();
        // SAFETY: Hidden owner-thread controls and an owned memory DC/bitmap. The
        // bitmap is deselected before deletion; the stock font/pen/brush are borrowed.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            assert!(!dc.is_null());
            let bitmap = CreateBitmap(800, 80, 1, 32, null());
            assert!(!bitmap.is_null());
            let previous = SelectObject(dc, bitmap);
            for dpi in [96, 144, 192] {
                MoveWindow(hwnd, 0, 0, scale(390, dpi), scale(30, dpi), 0);
                apply(hwnd, Resolved::Light, palette(), dpi);
                let mut second = RECT::default();
                assert_ne!(
                    SendMessageW(
                        hwnd,
                        TCM_GETITEMRECT,
                        1,
                        (&mut second as *mut RECT) as LPARAM
                    ),
                    0
                );
                let point = packed_size(
                    ((second.left + second.right) / 2) as u16,
                    ((second.top + second.bottom) / 2) as u16,
                );
                SendMessageW(hwnd, WM_LBUTTONDOWN, 1, point);
                SendMessageW(hwnd, WM_LBUTTONUP, 0, point);
                assert_eq!(SendMessageW(hwnd, CB_GETCURSEL, 0, 0), 1);
                SendMessageW(hwnd, WM_KEYDOWN, 0x27, 0); // VK_RIGHT, native Tab input.
                assert_eq!(SendMessageW(hwnd, CB_GETCURSEL, 0, 0), 2);
                SendMessageW(hwnd, WM_KEYDOWN, 0x25, 0); // VK_LEFT.
                assert_eq!(SendMessageW(hwnd, CB_GETCURSEL, 0, 0), 1);
                for kind in [Resolved::Light, Resolved::Dark, Resolved::Contrast] {
                    apply(
                        hwnd,
                        kind,
                        if kind == Resolved::Dark {
                            dark_palette()
                        } else {
                            palette()
                        },
                        dpi,
                    );
                    assert_eq!(
                        SendMessageW(hwnd, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM),
                        0
                    );
                }
                apply(hwnd, Resolved::Light, palette(), dpi);
                SendMessageW(hwnd, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                assert_eq!(GetPixel(dc, 0, 0), palette().background);
                assert_eq!(SendMessageW(hwnd, CB_SETCURSEL, 0, 0), 0);
            }
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }

    #[test]
    fn native_print_uses_one_track_and_preserves_hover_selection_and_dc() {
        let host = Host::new();
        let hwnd = create(host.0, 92, &["全部", "本地", "UTC"]).unwrap();
        let dark = dark_palette();
        // SAFETY: Owner-thread hidden HWNDs and an owned memory DC/bitmap; native
        // messages receive live buffers, and the bitmap is deselected before deletion.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            assert!(!dc.is_null());
            let bitmap = CreateBitmap(800, 80, 1, 32, null());
            assert!(!bitmap.is_null());
            let previous = SelectObject(dc, bitmap);
            let font = GetStockObject(DEFAULT_GUI_FONT);
            SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 0);
            for dpi in [96, 144, 192] {
                MoveWindow(hwnd, 0, 0, scale(390, dpi), scale(30, dpi), 0);
                for (kind, palette) in [(Resolved::Light, palette()), (Resolved::Dark, dark)] {
                    apply(hwnd, kind, palette, dpi);
                    assert_eq!(SendMessageW(hwnd, CB_SETCURSEL, 0, 0), 0);
                    let focus = SendMessageW(hwnd, TCM_GETCURFOCUS, 0, 0);
                    let first = native_item_rect(hwnd, 0).unwrap();
                    let second = native_item_rect(hwnd, 1).unwrap();
                    let third = native_item_rect(hwnd, 2).unwrap();
                    let mut bounds = RECT::default();
                    GetClientRect(hwnd, &mut bounds);
                    let track = track_rect(
                        bounds,
                        RECT {
                            left: first.left.min(second.left).min(third.left),
                            top: first.top.min(second.top).min(third.top),
                            right: first.right.max(second.right).max(third.right),
                            bottom: first.bottom.max(second.bottom).max(third.bottom),
                        },
                        dpi,
                    )
                    .unwrap();
                    let active = surface_rect(first, track, dpi).unwrap();
                    let inactive = surface_rect(second, track, dpi).unwrap();
                    let middle = (track.top + track.bottom) / 2;
                    let sample_x = inactive.left + scale(3, dpi);
                    SetTextColor(dc, 0x123456);
                    SetBkColor(dc, 0x654321);
                    SetBkMode(dc, OPAQUE as i32);
                    SetDCBrushColor(dc, 0x234567);
                    SetDCPenColor(dc, 0x345678);
                    let old_font = GetCurrentObject(dc, OBJ_FONT as u32);
                    SendMessageW(hwnd, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                    assert_eq!(GetTextColor(dc), 0x123456);
                    assert_eq!(GetBkColor(dc), 0x654321);
                    assert_eq!(GetBkMode(dc), OPAQUE as i32);
                    assert_eq!(GetDCBrushColor(dc), 0x234567);
                    assert_eq!(GetDCPenColor(dc), 0x345678);
                    assert_eq!(GetCurrentObject(dc, OBJ_FONT as u32), old_font);
                    let old_pen = GetCurrentObject(dc, OBJ_PEN as u32);
                    let old_brush = GetCurrentObject(dc, OBJ_BRUSH as u32);
                    let old_map = GetMapMode(dc);
                    let mut old_clip = RECT::default();
                    let clip_kind = GetClipBox(dc, &mut old_clip);
                    let mut old_viewport = POINT::default();
                    let mut old_window = POINT::default();
                    GetViewportOrgEx(dc, &mut old_viewport);
                    GetWindowOrgEx(dc, &mut old_window);
                    SendMessageW(hwnd, WM_PRINT, dc as WPARAM, PRF_CLIENT as LPARAM);
                    assert_eq!(GetTextColor(dc), 0x123456);
                    assert_eq!(GetBkColor(dc), 0x654321);
                    assert_eq!(GetBkMode(dc), OPAQUE as i32);
                    assert_eq!(GetDCBrushColor(dc), 0x234567);
                    assert_eq!(GetDCPenColor(dc), 0x345678);
                    assert_eq!(GetCurrentObject(dc, OBJ_FONT as u32), old_font);
                    assert_eq!(GetCurrentObject(dc, OBJ_PEN as u32), old_pen);
                    assert_eq!(GetCurrentObject(dc, OBJ_BRUSH as u32), old_brush);
                    assert_eq!(GetMapMode(dc), old_map);
                    let mut clip = RECT::default();
                    assert_eq!(GetClipBox(dc, &mut clip), clip_kind);
                    assert_eq!(
                        (clip.left, clip.top, clip.right, clip.bottom),
                        (old_clip.left, old_clip.top, old_clip.right, old_clip.bottom)
                    );
                    let mut origin = POINT::default();
                    GetViewportOrgEx(dc, &mut origin);
                    assert_eq!((origin.x, origin.y), (old_viewport.x, old_viewport.y));
                    GetWindowOrgEx(dc, &mut origin);
                    assert_eq!((origin.x, origin.y), (old_window.x, old_window.y));
                    SendMessageW(hwnd, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                    assert_eq!(GetPixel(dc, 0, 0), palette.background);
                    // Inactive items and their shared boundary are the same track,
                    // not individually stroked buttons. Active edges are neutral.
                    assert_eq!(GetPixel(dc, sample_x, middle), palette.subtle());
                    assert_eq!(
                        GetPixel(dc, (second.right + third.left) / 2, middle),
                        palette.subtle()
                    );
                    assert_eq!(
                        GetPixel(dc, (active.left + active.right) / 2, active.top),
                        colors(palette, kind, true, true, false).fill
                    );
                    assert_eq!(
                        GetPixel(dc, (active.left + active.right) / 2, active.bottom),
                        blend(palette.subtle(), 0, 10)
                    );
                    let point = packed_size(
                        ((second.left + second.right) / 2) as u16,
                        ((second.top + second.bottom) / 2) as u16,
                    );
                    SendMessageW(hwnd, WM_MOUSEMOVE, 0, point);
                    assert_eq!((*marked(hwnd)).hovered.get(), Some(1));
                    assert!((*marked(hwnd)).tracking_leave.get());
                    assert_eq!(SendMessageW(hwnd, CB_GETCURSEL, 0, 0), 0);
                    assert_eq!(SendMessageW(hwnd, TCM_GETCURFOCUS, 0, 0), focus);
                    SendMessageW(hwnd, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                    assert_eq!(GetPixel(dc, sample_x, middle), palette.hover());
                    for (index, before) in [first, second, third].into_iter().enumerate() {
                        let after = native_item_rect(hwnd, index).unwrap();
                        assert_eq!(
                            (after.left, after.top, after.right, after.bottom),
                            (before.left, before.top, before.right, before.bottom),
                        );
                    }
                    SendMessageW(hwnd, WM_MOUSELEAVE, 0, 0);
                    assert_eq!((*marked(hwnd)).hovered.get(), None);
                    assert!(!(*marked(hwnd)).tracking_leave.get());
                    SendMessageW(hwnd, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                    assert_eq!(GetPixel(dc, sample_x, middle), palette.subtle());
                    SendMessageW(hwnd, WM_MOUSEMOVE, 0, point);
                    SendMessageW(hwnd, WM_MOUSEMOVE, 0, packed_size(u16::MAX, u16::MAX));
                    assert_eq!((*marked(hwnd)).hovered.get(), None);
                    SendMessageW(hwnd, WM_MOUSEMOVE, 0, point);
                    EnableWindow(hwnd, 0);
                    assert_eq!((*marked(hwnd)).hovered.get(), None);
                    assert!(!(*marked(hwnd)).tracking_leave.get());
                    SendMessageW(hwnd, WM_MOUSEMOVE, 0, point);
                    assert_eq!((*marked(hwnd)).hovered.get(), None);
                    SendMessageW(hwnd, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                    assert_eq!(GetPixel(dc, sample_x, middle), palette.subtle());
                    assert_eq!(
                        GetPixel(dc, (active.left + active.right) / 2, active.top),
                        colors(palette, kind, true, false, false).fill
                    );
                    EnableWindow(hwnd, 1);
                    SendMessageW(hwnd, WM_MOUSEMOVE, 0, point);
                    apply(hwnd, Resolved::Contrast, palette, dpi);
                    assert_eq!((*marked(hwnd)).hovered.get(), None);
                    assert!(!(*marked(hwnd)).tracking_leave.get());
                    SendMessageW(hwnd, WM_MOUSEMOVE, 0, point);
                    assert_eq!((*marked(hwnd)).hovered.get(), None);
                    assert_eq!(SendMessageW(hwnd, CB_GETCURSEL, 0, 0), 0);
                    let mut command = MSG::default();
                    assert_eq!(
                        PeekMessageW(&mut command, host.0, WM_COMMAND, WM_COMMAND, PM_REMOVE),
                        0
                    );
                }
            }
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }

    struct HoverDestroyProbe {
        armed: Cell<bool>,
        hits: Cell<usize>,
        removed: Cell<bool>,
        destroyed: Cell<bool>,
        retained_after_destroy: Cell<usize>,
        original: std::rc::Weak<Choices>,
        replacement: Cell<HWND>,
    }

    unsafe extern "system" fn destroy_during_hit_test(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: Installed outside Choices on a private hidden Tab. The probe
        // outlives its host; disarm/remove before real synchronous destruction.
        // Never unwind assertions through this native callback.
        unsafe {
            let probe = &*(data as *const HoverDestroyProbe);
            if message == TCM_HITTEST && probe.armed.replace(false) {
                probe.hits.set(probe.hits.get() + 1);
                probe
                    .removed
                    .set(RemoveWindowSubclass(hwnd, Some(destroy_during_hit_test), id) != 0);
                let parent = GetParent(hwnd);
                probe.destroyed.set(DestroyWindow(hwnd) != 0);
                probe
                    .retained_after_destroy
                    .set(probe.original.strong_count());
                // Allocate another real Choices before the suspended hover query
                // returns. Rc retention keeps the old instance identity distinct,
                // even if Windows reuses its HWND for this replacement.
                if let Ok(replacement) = create(parent, 94, &["替代", "保留"]) {
                    apply(replacement, Resolved::Light, palette(), 96);
                    probe.replacement.set(replacement);
                }
                // A valid index makes a missing lifetime check follow the old
                // labels/hover path, rather than accidentally avoiding its UAF.
                return 0;
            }
            if message == WM_NCDESTROY {
                RemoveWindowSubclass(hwnd, Some(destroy_during_hit_test), id);
            }
            DefSubclassProc(hwnd, message, w, l)
        }
    }

    #[test]
    fn native_hover_hit_test_destruction_keeps_inflight_owner_alive() {
        for kind in [Resolved::Light, Resolved::Dark] {
            let mut probe = HoverDestroyProbe {
                armed: Cell::new(true),
                hits: Cell::new(0),
                removed: Cell::new(false),
                destroyed: Cell::new(false),
                retained_after_destroy: Cell::new(0),
                original: std::rc::Weak::new(),
                replacement: Cell::new(null_mut()),
            };
            // Declared after probe so panic-time host destruction is also safe.
            let host = Host::new();
            let hwnd = create(host.0, 93, &["全部", "本地", "UTC"]).unwrap();
            // SAFETY: Same-thread hidden native windows and stack-lived probe;
            // only a Weak is kept by the test while the real messages run.
            unsafe {
                MoveWindow(hwnd, 0, 0, 390, 30, 0);
                apply(
                    hwnd,
                    kind,
                    if kind == Resolved::Dark {
                        dark_palette()
                    } else {
                        palette()
                    },
                    96,
                );
                let pointer = marked(hwnd);
                assert!(!pointer.is_null());
                Rc::increment_strong_count(pointer);
                let retained = Rc::from_raw(pointer);
                probe.original = Rc::downgrade(&retained);
                drop(retained);
                assert_eq!(probe.original.strong_count(), 1);
                assert_ne!(
                    SetWindowSubclass(
                        hwnd,
                        Some(destroy_during_hit_test),
                        SUBCLASS_ID + 1,
                        &probe as *const HoverDestroyProbe as usize,
                    ),
                    0
                );
                assert_eq!(SendMessageW(hwnd, WM_MOUSEMOVE, 0, 10 | (10 << 16)), 0);
                assert_eq!(probe.hits.get(), 1);
                assert!(probe.removed.get());
                assert!(probe.destroyed.get());
                assert_eq!(
                    probe.retained_after_destroy.get(),
                    1,
                    "the in-flight mouse callback must retain the destroyed owner"
                );
                assert!(probe.original.upgrade().is_none(), "destroyed owner leaked");
                let replacement = probe.replacement.get();
                assert!(!replacement.is_null());
                let replacement_data = marked(replacement);
                assert!(!replacement_data.is_null());
                assert_ne!(replacement_data, pointer);
                assert!((*replacement_data).attached.get());
                assert_eq!((*replacement_data).hovered.get(), None);
                assert!(!(*replacement_data).tracking_leave.get());
                assert_eq!(SendMessageW(replacement, CB_GETCURSEL, 0, 0), 0);
                let mut command = MSG::default();
                assert_eq!(
                    PeekMessageW(&mut command, host.0, WM_COMMAND, WM_COMMAND, PM_REMOVE),
                    0
                );
            }
        }
    }

    #[test]
    fn rejects_invalid_creation_without_creating_children() {
        let host = Host::new();
        assert!(create(host.0, 1, &[]).is_err());
        assert!(create(host.0, 65536, &["选项"]).is_err());
        assert!(create(host.0, 1, &["内嵌\0字符"]).is_err());
        assert!(create(null_mut(), 1, &["选项"]).is_err());
        // SAFETY: The live parent belongs to this thread; rejected calls create nothing.
        unsafe {
            assert!(GetWindow(host.0, GW_CHILD).is_null());
        }
    }

    #[test]
    fn native_notification_posts_original_command_and_ignores_main_tab() {
        let host = Host::new();
        let hwnd = create(host.0, 73, &["会话", "历史"]).unwrap();
        // SAFETY: Same-thread windows and live stack NMHDR/MSG for synchronous dispatch.
        unsafe {
            let mut message = MSG::default();
            assert_eq!(
                PeekMessageW(&mut message, host.0, WM_COMMAND, WM_COMMAND, PM_REMOVE),
                0
            );
            SendMessageW(hwnd, CB_SETCURSEL, 1, 0);
            // Programmatic selection must not emit CBN_SELCHANGE.
            assert_eq!(
                PeekMessageW(&mut message, host.0, WM_COMMAND, WM_COMMAND, PM_REMOVE),
                0
            );
            let header = NMHDR {
                hwndFrom: hwnd,
                idFrom: 73,
                code: TCN_SELCHANGE,
            };
            assert_eq!(
                SendMessageW(host.0, WM_NOTIFY, 73, (&header as *const NMHDR) as LPARAM),
                0
            );
            assert_ne!(
                PeekMessageW(&mut message, host.0, WM_COMMAND, WM_COMMAND, PM_REMOVE),
                0
            );
            assert_eq!(message.wParam, 73 | ((CBN_SELCHANGE as usize) << 16));
            assert_eq!(message.lParam, hwnd as LPARAM);
            assert!(!notify(0));
            let other = CreateWindowExW(
                0,
                windows_sys::w!("SysTabControl32"),
                null(),
                WS_CHILD | TCS_FIXEDWIDTH,
                0,
                0,
                100,
                30,
                host.0,
                74usize as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!other.is_null());
            apply(other, Resolved::Dark, dark_palette(), 192);
            assert!(marked(other).is_null());
            let unrelated = NMHDR {
                hwndFrom: other,
                idFrom: 74,
                code: TCN_SELCHANGE,
            };
            assert!(!notify((&unrelated as *const NMHDR) as LPARAM));
            let changing = NMHDR {
                hwndFrom: hwnd,
                idFrom: 73,
                code: TCN_SELCHANGING,
            };
            assert!(!notify((&changing as *const NMHDR) as LPARAM));
        }
    }
}
