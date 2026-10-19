//! Presentation-only subclasses for native controls. No controller/State pointer
//! is retained; native controls still own values, input, notifications and MSAA.
//! Call apply for each child after SetWindowTheme; route WM_NOTIFY to custom_draw
//! before the generic button custom-draw handler (control style bits overlap).
use super::theme::{Palette, Resolved, blend};
use std::{mem::size_of, ptr::null};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    UI::{Controls::*, WindowsAndMessaging::*},
};

const SUBCLASS_ID: usize = 0x4354_484d;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn GetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: *mut usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}
// Documented winuser.h TRACKMOUSEEVENT ABI; keep the otherwise unused input
// feature out of the dependency list. Only leave tracking is requested.
#[repr(C)]
struct MouseTracking {
    size: u32,
    flags: u32,
    hwnd: HWND,
    hover_time: u32,
}
const TRACK_MOUSE_LEAVE: u32 = 0x0000_0002;

#[link(name = "user32")]
unsafe extern "system" {
    fn GetFocus() -> HWND;
    fn IsWindowEnabled(hwnd: HWND) -> i32;
    fn TrackMouseEvent(event: *mut MouseTracking) -> i32;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Combo,
    List,
    Header,
    Switch,
}

#[derive(Clone, Copy)]
struct Appearance {
    control: Control,
    kind: Resolved,
    palette: Palette,
    dpi: u32,
    hover: bool,
}

fn px(value: i32, dpi: u32) -> i32 {
    ((i64::from(value) * i64::from(dpi.clamp(96, 768)) + 48) / 96) as i32
}

fn identify(class: &str, style: u32) -> Option<Control> {
    match class {
        "ComboBox" if style & 3 == CBS_DROPDOWNLIST as u32 => Some(Control::Combo),
        "SysListView32" => Some(Control::List),
        "SysHeader32" => Some(Control::Header),
        "Button" if style & BS_TYPEMASK as u32 == BS_AUTOCHECKBOX as u32 => Some(Control::Switch),
        _ => None,
    }
}

unsafe fn class_name(hwnd: HWND) -> String {
    // SAFETY: caller supplies a live HWND; GetClassNameW bounds writes to buffer.
    unsafe {
        let mut buffer = [0u16; 64];
        let count = GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..count.max(0) as usize])
    }
}

unsafe fn appearance(hwnd: HWND) -> Option<Appearance> {
    // SAFETY: only this module installs this proc/id pair on the UI thread. Its
    // Box lives until detach/NCDESTROY. Copy before any reentrant window call.
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) == 0 || data == 0 {
            None
        } else {
            let value = *(data as *const Appearance);
            (value.kind != Resolved::Contrast).then_some(value)
        }
    }
}

unsafe fn set_hover(hwnd: HWND, hover: bool) {
    // SAFETY: owner-thread lookup/assignment only. No reference to the Box is
    // retained across invalidation or native processing of the mouse message.
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) != 0 && data != 0 {
            let mut value = *(data as *const Appearance);
            if value.hover != hover {
                value.hover = hover;
                *(data as *mut Appearance) = value;
                InvalidateRect(hwnd, null(), 0);
            }
        }
    }
}

unsafe fn attach(hwnd: HWND, mut value: Appearance) {
    // SAFETY: UI thread, live native control; each subclass owns exactly one Box.
    // Updating copies only colors/DPI, and no borrow spans a Win32 callback.
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) != 0 {
            if data != 0 {
                value.hover = (*(data as *const Appearance)).hover;
                *(data as *mut Appearance) = value;
            }
            return;
        }
        let data = Box::into_raw(Box::new(value));
        if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, data as usize) == 0 {
            drop(Box::from_raw(data));
        }
    }
}

unsafe fn detach(hwnd: HWND) {
    // SAFETY: same-thread lookup/removal; disable painting before removal. If it
    // fails, the installed callback still owns its Box but only forwards natively.
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) != 0 {
            if data != 0 {
                (*(data as *mut Appearance)).kind = Resolved::Contrast;
                (*(data as *mut Appearance)).hover = false;
            }
            if RemoveWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID) != 0 && data != 0 {
                drop(Box::from_raw(data as *mut Appearance));
            }
        }
    }
}

/// Apply after the parent's native theme selection, once for each child.
///
/// # Safety
/// `hwnd` must be a live control on the calling UI thread. Repeat on theme/DPI
/// changes. No parent State, palette reference or GDI object is retained.
pub(super) unsafe fn apply(hwnd: HWND, kind: Resolved, palette: Palette, dpi: u32) {
    // SAFETY: caller guarantees owner-thread access. Every message uses either a
    // scalar or a stack buffer valid for the synchronous SendMessageW call.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let Some(control) = identify(&class_name(hwnd), style) else {
            detach(hwnd);
            return;
        };
        let custom = kind != Resolved::Contrast;
        if custom {
            attach(
                hwnd,
                Appearance {
                    control,
                    kind,
                    palette,
                    dpi,
                    hover: false,
                },
            );
        } else {
            detach(hwnd);
        }
        if control == Control::List {
            // Themed v6 ListViews may ignore custom-draw background colors. Use
            // the documented unthemed painter in BOTH app themes; selection and
            // keyboard remain native. High contrast restores the native theme.
            let empty = [0u16];
            let theme = if custom { empty.as_ptr() } else { null() };
            SetWindowTheme(hwnd, theme, theme);
            let (background, text) = if custom {
                (palette.surface, palette.text)
            } else {
                (GetSysColor(COLOR_WINDOW), GetSysColor(COLOR_WINDOWTEXT))
            };
            SendMessageW(hwnd, LVM_SETBKCOLOR, 0, background as LPARAM);
            SendMessageW(hwnd, LVM_SETTEXTBKCOLOR, 0, background as LPARAM);
            SendMessageW(hwnd, LVM_SETTEXTCOLOR, 0, text as LPARAM);
            let header = SendMessageW(hwnd, LVM_GETHEADER, 0, 0) as HWND;
            if !header.is_null() {
                // Headers are grandchildren and may not be visited by the parent.
                SetWindowTheme(header, theme, theme);
                apply(header, kind, palette, dpi);
            }
        }
        InvalidateRect(hwnd, null(), 1);
    }
}

/// Handle WM_NOTIFY before theme's generic button NM_CUSTOMDRAW dispatch.
/// Recognized native list/header notifications are consumed even when detached,
/// preventing their style bits from being mistaken for button styles.
///
/// # Safety
/// `l` is zero or a valid WM_NOTIFY LPARAM on the sender's UI thread. For
/// NM_CUSTOMDRAW, the buffer must have the sender class's documented layout.
pub(super) unsafe fn custom_draw(l: LPARAM) -> Option<LRESULT> {
    // SAFETY: WM_NOTIFY guarantees the NMHDR prefix. Check class/code before
    // interpreting the larger list-view-specific notification or subclass data.
    unsafe {
        if l == 0 {
            return None;
        }
        let header = *(l as *const NMHDR);
        if header.code != NM_CUSTOMDRAW {
            return None;
        }
        let hwnd = header.hwndFrom;
        let class = class_name(hwnd);
        if class == "SysHeader32" {
            return Some(CDRF_DODEFAULT as LRESULT);
        }
        if class != "SysListView32" {
            return None;
        }
        let value = appearance(hwnd);
        // Cell postpaint adds only a frame. Copy before its reentrant ListView
        // queries, then recheck attachment in case a nested refresh chose native.
        let mut draw = *(l as *const NMLVCUSTOMDRAW);
        let cell_flags = super::cell_table::custom_draw(
            draw,
            value.map(|v| (v.palette.text, v.palette.accent_surface)),
        );
        if appearance(hwnd).is_none() {
            return Some(cell_flags as LRESULT);
        }
        match draw.nmcd.dwDrawStage {
            CDDS_PREPAINT => Some((CDRF_NOTIFYITEMDRAW | cell_flags) as LRESULT),
            stage if stage == CDDS_ITEMPREPAINT || stage == CDDS_ITEMPREPAINT | CDDS_SUBITEM => {
                // Native item state remains unchanged. Suppress only the temporary
                // drawing flags that would otherwise force a bright system fill.
                let state = SendMessageW(
                    hwnd,
                    LVM_GETITEMSTATE,
                    draw.nmcd.dwItemSpec,
                    LVIS_SELECTED as LPARAM,
                ) as u32;
                let hot = SendMessageW(hwnd, LVM_GETHOTITEM, 0, 0);
                let selected =
                    state & LVIS_SELECTED != 0 || draw.nmcd.uItemState & CDIS_SELECTED != 0;
                let hot = draw.nmcd.uItemState & CDIS_HOT != 0
                    || (hot >= 0 && hot as usize == draw.nmcd.dwItemSpec);
                let enabled = IsWindowEnabled(hwnd) != 0;
                let Some(value) = appearance(hwnd) else {
                    return Some(cell_flags as LRESULT);
                };
                draw.clrText = if enabled {
                    value.palette.text
                } else {
                    value.palette.disabled
                };
                draw.clrTextBk =
                    row_background(value.palette, draw.nmcd.dwItemSpec, selected, hot, enabled);
                draw.nmcd.uItemState &= !(CDIS_SELECTED | CDIS_HOT);
                // No reference into the notification spans SendMessageW above.
                (*(l as *mut NMLVCUSTOMDRAW)).clrText = draw.clrText;
                (*(l as *mut NMLVCUSTOMDRAW)).clrTextBk = draw.clrTextBk;
                (*(l as *mut NMLVCUSTOMDRAW)).nmcd.uItemState = draw.nmcd.uItemState;
                Some((CDRF_NEWFONT | CDRF_NOTIFYSUBITEMDRAW | cell_flags) as LRESULT)
            }
            _ => Some(cell_flags as LRESULT),
        }
    }
}

fn row_background(p: Palette, row: usize, selected: bool, hot: bool, enabled: bool) -> u32 {
    if selected {
        if enabled {
            p.accent_surface
        } else {
            blend(p.surface, p.accent_surface, 50)
        }
    } else if hot && enabled {
        p.hover()
    } else if row & 1 != 0 {
        blend(p.surface, p.subtle(), 50)
    } else {
        p.surface
    }
}

struct SavedDc {
    dc: HDC,
    index: i32,
}
impl SavedDc {
    unsafe fn new(dc: HDC) -> Option<Self> {
        // SAFETY: borrowed, live paint/print DC. Never mutate if SaveDC fails.
        let index = unsafe { SaveDC(dc) };
        (index != 0).then_some(Self { dc, index })
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: guard cannot outlive its borrowed DC; restores selections too.
        unsafe { RestoreDC(self.dc, self.index) };
    }
}

struct Pen(HPEN);
impl Drop for Pen {
    fn drop(&mut self) {
        // SAFETY: owned CreatePen result; drawing's SavedDc drops before this pen.
        unsafe { DeleteObject(self.0) };
    }
}

unsafe fn fill(dc: HDC, rect: &RECT, color: u32) {
    // SAFETY: caller has saved the live DC. DC_BRUSH is borrowed, never deleted.
    unsafe {
        SetDCBrushColor(dc, color);
        FillRect(dc, rect, GetStockObject(DC_BRUSH) as HBRUSH);
    }
}

unsafe fn chevron(dc: HDC, rect: &RECT, color: u32, dpi: u32, up: bool) {
    // SAFETY: all coordinates are scalar. The nested DC guard deselects the pen
    // before Pen::drop, including early returns, without deleting stock objects.
    unsafe {
        let raw = CreatePen(PS_SOLID, px(1, dpi), color);
        if raw.is_null() {
            return;
        }
        let pen = Pen(raw);
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        SelectObject(dc, pen.0 as HGDIOBJ);
        let x = (rect.left + rect.right) / 2;
        let y = (rect.top + rect.bottom) / 2;
        let radius = px(4, dpi).min((rect.right - rect.left).max(0) / 3);
        let offset = if up { -1 } else { 1 };
        let points = [
            POINT {
                x: x - radius,
                y: y - offset * radius / 2,
            },
            POINT {
                x,
                y: y + offset * radius / 2,
            },
            POINT {
                x: x + radius,
                y: y - offset * radius / 2,
            },
        ];
        Polyline(dc, points.as_ptr(), points.len() as i32);
    }
}

unsafe fn font(hwnd: HWND, dc: HDC) {
    // SAFETY: WM_GETFONT returns a borrowed font owned by the native UI. The
    // caller's SavedDc restores the previous font before the DC is released.
    unsafe {
        let handle = SendMessageW(hwnd, WM_GETFONT, 0, 0) as HGDIOBJ;
        if !handle.is_null() {
            SelectObject(dc, handle);
        }
        SetBkMode(dc, TRANSPARENT as i32);
    }
}

unsafe fn title(hwnd: HWND) -> Vec<u16> {
    // SAFETY: bounded local buffer; GetWindowTextW never exceeds supplied size.
    // Existing labels remain stored natively in full for WM_GETTEXT and MSAA.
    unsafe {
        let length = GetWindowTextLengthW(hwnd).clamp(0, 16_384) as usize;
        let mut buffer = vec![0u16; length + 1];
        let count = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        buffer.truncate(count.max(0) as usize);
        buffer
    }
}

fn combo_rects(client: RECT, dpi: u32) -> (RECT, RECT) {
    let width = (client.right - client.left).max(0);
    let height = (client.bottom - client.top).max(0);
    let edge = px(1, dpi).min(width / 2).min(height / 2);
    let arrow = RECT {
        left: (client.right - px(30, dpi)).max(client.left + edge),
        top: client.top + edge,
        right: client.right - edge,
        bottom: client.bottom - edge,
    };
    let left = (client.left + px(11, dpi)).min(arrow.left);
    let text = RECT {
        left,
        right: (arrow.left - px(3, dpi)).max(left),
        top: arrow.top,
        bottom: arrow.bottom,
    };
    (text, arrow)
}

fn combo_colors(p: Palette, enabled: bool, hover: bool, active: bool) -> (u32, u32, u32) {
    if !enabled {
        return (p.subtle(), blend(p.subtle(), p.border, 60), p.disabled);
    }
    let surface = if hover || active {
        blend(p.surface, p.hover(), 60)
    } else {
        p.surface
    };
    let border = if active {
        blend(p.border, p.accent, 80)
    } else if hover {
        blend(p.border, p.accent, 35)
    } else {
        blend(p.surface, p.border, 85)
    };
    let arrow = if active || hover {
        blend(p.disabled, p.accent, 65)
    } else {
        p.disabled
    };
    (surface, border, arrow)
}

unsafe fn draw_combo(hwnd: HWND, dc: HDC, value: Appearance) {
    // SAFETY: live control/DC, owner thread. Native paint has already validated
    // its update region. GetComboBoxInfo supplies public client-coordinate rects.
    unsafe {
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        let mut info = COMBOBOXINFO {
            cbSize: size_of::<COMBOBOXINFO>() as u32,
            ..Default::default()
        };
        if GetComboBoxInfo(hwnd, &mut info) == 0 {
            return;
        }
        let p = value.palette;
        let enabled = IsWindowEnabled(hwnd) != 0;
        let ink = if enabled { p.text } else { p.disabled };
        let focused = enabled && GetFocus() == hwnd;
        let opened = SendMessageW(hwnd, CB_GETDROPPEDSTATE, 0, 0) != 0;
        let (surface, border, arrow_ink) = combo_colors(p, enabled, value.hover, focused || opened);
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        // Owner-drawn combos may display icons/arbitrary application content.
        // Leave that native contract intact and cover only their arrow button.
        if style & (CBS_OWNERDRAWFIXED | CBS_OWNERDRAWVARIABLE) as u32 == 0 {
            let mut client = RECT::default();
            GetClientRect(hwnd, &mut client);
            super::rounded::surface(dc, &client, value.dpi, p.background, surface, border);
            font(hwnd, dc);
            SetTextColor(dc, ink);
            let text = title(hwnd);
            let (mut rect, arrow) = combo_rects(client, value.dpi);
            DrawTextW(
                dc,
                text.as_ptr(),
                text.len() as i32,
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            let ui = SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) as u32;
            if focused && ui & UISF_HIDEFOCUS == 0 {
                let mut focus = client;
                let inset = px(3, value.dpi);
                InflateRect(&mut focus, -inset, -inset);
                if focus.right > focus.left && focus.bottom > focus.top {
                    DrawFocusRect(dc, &focus);
                }
            }
            // Keep the arrow on the same rounded surface; a rectangular button
            // fill here would overwrite the right-hand arc and its border.
            chevron(dc, &arrow, arrow_ink, value.dpi, opened);
        } else {
            fill(dc, &info.rcButton, surface);
            chevron(dc, &info.rcButton, arrow_ink, value.dpi, opened);
        }
    }
}

fn switch_rects(client: RECT, dpi: u32, checked: bool, right: bool) -> (RECT, RECT, RECT) {
    let width = px(34, dpi).min((client.right - client.left).max(0));
    let height = px(18, dpi).min((client.bottom - client.top).max(0));
    let top = client.top + ((client.bottom - client.top) - height) / 2;
    let left = if right {
        client.right - width
    } else {
        client.left
    };
    let track = RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    };
    let inset = px(3, dpi).min(height / 3);
    let diameter = (height - 2 * inset).max(0);
    let knob_left = if checked {
        track.right - inset - diameter
    } else {
        left + inset
    };
    let knob = RECT {
        left: knob_left,
        top: top + inset,
        right: knob_left + diameter,
        bottom: top + inset + diameter,
    };
    let mut label = client;
    if right {
        label.right = (track.left - px(9, dpi)).max(label.left);
    } else {
        label.left = (track.right + px(9, dpi)).min(label.right);
    }
    (track, knob, label)
}

fn switch_colors(
    p: Palette,
    checked: bool,
    enabled: bool,
    hover: bool,
    pressed: bool,
) -> (u32, u32, u32) {
    let (mut track, mut border, mut knob) = if checked {
        (p.accent, p.accent, p.on_accent())
    } else {
        (
            blend(p.subtle(), p.border, 30),
            blend(p.subtle(), p.border, 65),
            p.disabled,
        )
    };
    if !enabled {
        track = blend(p.subtle(), track, 40);
        border = blend(track, p.border, 40);
        knob = blend(track, knob, 65);
    } else if hover || pressed {
        if checked {
            track = blend(track, p.on_accent(), if pressed { 15 } else { 8 });
            border = track;
        } else {
            track = blend(track, p.disabled, if pressed { 20 } else { 10 });
            border = blend(border, p.disabled, 35);
        }
    }
    (track, border, knob)
}

unsafe fn draw_switch(hwnd: HWND, dc: HDC, value: Appearance) {
    // SAFETY: native Button owns its caption/check/state and all input handling.
    // Only client painting changes; styles and BM_GET/SETCHECK are never replaced.
    unsafe {
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        let p = value.palette;
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client);
        fill(dc, &client, p.background);
        let enabled = IsWindowEnabled(hwnd) != 0;
        let checked = SendMessageW(hwnd, BM_GETCHECK, 0, 0) as u32 == BST_CHECKED;
        let pressed = SendMessageW(hwnd, BM_GETSTATE, 0, 0) as u32 & BST_PUSHED != 0;
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let (track, knob, mut label) =
            switch_rects(client, value.dpi, checked, style & BS_LEFTTEXT as u32 != 0);
        let (track_color, border, knob_color) =
            switch_colors(p, checked, enabled, value.hover, pressed);
        SelectObject(dc, GetStockObject(DC_BRUSH));
        SelectObject(dc, GetStockObject(DC_PEN));
        SetDCPenColor(dc, border);
        SetDCBrushColor(dc, track_color);
        let height = track.bottom - track.top;
        RoundRect(
            dc,
            track.left,
            track.top,
            track.right,
            track.bottom,
            height,
            height,
        );
        let ink = if enabled { p.text } else { p.disabled };
        SetDCPenColor(dc, knob_color);
        SetDCBrushColor(dc, knob_color);
        Ellipse(dc, knob.left, knob.top, knob.right, knob.bottom);
        font(hwnd, dc);
        SetTextColor(dc, ink);
        let ui = SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) as u32;
        let text = title(hwnd);
        let flags = if style & BS_MULTILINE as u32 != 0 {
            DT_WORDBREAK
        } else {
            DT_SINGLELINE | DT_VCENTER
        };
        DrawTextW(
            dc,
            text.as_ptr(),
            text.len() as i32,
            &mut label,
            flags
                | DT_END_ELLIPSIS
                | if ui & UISF_HIDEACCEL != 0 {
                    DT_HIDEPREFIX
                } else {
                    0
                },
        );
        if enabled && GetFocus() == hwnd && ui & UISF_HIDEFOCUS == 0 {
            let mut focus = client;
            InflateRect(&mut focus, -1, -1);
            DrawFocusRect(dc, &focus);
        }
    }
}

unsafe fn draw_header(hwnd: HWND, dc: HDC, value: Appearance) {
    // SAFETY: public synchronous HDM calls write into stack-owned buffers. Header
    // keeps column order/width/sort metadata, hit testing and accessibility.
    unsafe {
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        let p = value.palette;
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client);
        fill(dc, &client, p.subtle());
        font(hwnd, dc);
        SetTextColor(
            dc,
            if IsWindowEnabled(hwnd) != 0 {
                p.text
            } else {
                p.disabled
            },
        );
        let count = SendMessageW(hwnd, HDM_GETITEMCOUNT, 0, 0).clamp(0, 4096);
        for index in 0..count {
            let mut rect = RECT::default();
            if SendMessageW(
                hwnd,
                HDM_GETITEMRECT,
                index as WPARAM,
                (&mut rect as *mut RECT) as LPARAM,
            ) == 0
                || rect.right <= client.left
                || rect.left >= client.right
            {
                continue;
            }
            let mut buffer = [0u16; 1024];
            let mut item = HDITEMW {
                mask: HDI_TEXT | HDI_FORMAT,
                pszText: buffer.as_mut_ptr(),
                cchTextMax: buffer.len() as i32,
                ..Default::default()
            };
            if SendMessageW(
                hwnd,
                HDM_GETITEMW,
                index as WPARAM,
                (&mut item as *mut HDITEMW) as LPARAM,
            ) == 0
            {
                continue;
            }
            let Some(_cell) = SavedDc::new(dc) else {
                continue;
            };
            IntersectClipRect(dc, rect.left, rect.top, rect.right, rect.bottom);
            let mut text = rect;
            text.left += px(10, value.dpi);
            text.right = (text.right - px(10, value.dpi)).max(text.left);
            if item.fmt & (HDF_SORTUP | HDF_SORTDOWN) != 0 {
                let arrow = RECT {
                    left: (text.right - px(16, value.dpi)).max(text.left),
                    ..text
                };
                let ink = if IsWindowEnabled(hwnd) != 0 {
                    blend(p.disabled, p.accent, 60)
                } else {
                    p.disabled
                };
                chevron(dc, &arrow, ink, value.dpi, item.fmt & HDF_SORTUP != 0);
                text.right = arrow.left;
            }
            let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
            let align = match item.fmt & HDF_JUSTIFYMASK {
                HDF_CENTER => DT_CENTER,
                HDF_RIGHT => DT_RIGHT,
                _ => DT_LEFT,
            };
            DrawTextW(
                dc,
                buffer.as_ptr(),
                length as i32,
                &mut text,
                align | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
        }
        let line = RECT {
            top: (client.bottom - 1).max(client.top),
            ..client
        };
        fill(dc, &line, blend(p.subtle(), p.border, 65));
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
    // SAFETY: installed on the owner thread with one owned Appearance Box. Copy
    // before native calls, which can reenter theme refresh or destroy the child.
    unsafe {
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(procedure), id);
            let value = Box::from_raw(data as *mut Appearance);
            let result = DefSubclassProc(hwnd, message, w, l);
            drop(value);
            return result;
        }
        let value = *(data as *const Appearance);
        if value.kind == Resolved::Contrast {
            return DefSubclassProc(hwnd, message, w, l);
        }
        if matches!(value.control, Control::Combo | Control::Switch) {
            if message == WM_MOUSEMOVE && !value.hover && IsWindowEnabled(hwnd) != 0 {
                let mut tracking = MouseTracking {
                    size: size_of::<MouseTracking>() as u32,
                    flags: TRACK_MOUSE_LEAVE,
                    hwnd,
                    hover_time: 0,
                };
                if TrackMouseEvent(&mut tracking) != 0 {
                    set_hover(hwnd, true);
                }
            } else if message == WM_MOUSELEAVE
                || message == WM_CANCELMODE
                || (message == WM_ENABLE || message == WM_SHOWWINDOW) && w == 0
            {
                set_hover(hwnd, false);
            }
        }
        if message == WM_PAINT || message == WM_PRINTCLIENT {
            if value.control == Control::Combo {
                // Native WM_PAINT must run FIRST: it consumes/validates the update
                // region. BeginPaint before it would suppress its native painting.
                // Native PRINTCLIENT can also change the caller's DC; include
                // it inside the guard rather than saving only our overlay.
                let _print_dc = if message == WM_PRINTCLIENT && w != 0 {
                    SavedDc::new(w as HDC)
                } else {
                    None
                };
                let result = DefSubclassProc(hwnd, message, w, l);
                // Native painting may synchronously refresh the theme or destroy
                // the child. Do not draw a stale dark/light overlay after detach.
                let Some(value) = appearance(hwnd).filter(|v| v.control == Control::Combo) else {
                    return result;
                };
                let dc = if message == WM_PAINT {
                    GetDC(hwnd)
                } else {
                    w as HDC
                };
                if !dc.is_null() {
                    draw_combo(hwnd, dc, value);
                    if message == WM_PAINT {
                        ReleaseDC(hwnd, dc);
                    }
                }
                return result;
            }
            if value.control == Control::Switch || value.control == Control::Header {
                let mut paint = PAINTSTRUCT::default();
                let dc = if message == WM_PAINT {
                    BeginPaint(hwnd, &mut paint)
                } else {
                    w as HDC
                };
                if !dc.is_null() {
                    if value.control == Control::Switch {
                        draw_switch(hwnd, dc, value);
                    } else {
                        draw_header(hwnd, dc, value);
                    }
                }
                if message == WM_PAINT {
                    EndPaint(hwnd, &paint);
                }
                return 0;
            }
        }
        if message == WM_ERASEBKGND && matches!(value.control, Control::Switch | Control::Header) {
            return 1;
        }
        let result = DefSubclassProc(hwnd, message, w, l);
        let Some(value) = appearance(hwnd) else {
            return result;
        };
        if value.control == Control::List && message == LVM_INSERTCOLUMNW {
            // Some Common Controls versions create their header only on the
            // first column. Apply colors at that point, not just at theme refresh.
            apply(hwnd, value.kind, value.palette, value.dpi);
        }
        if matches!(
            message,
            WM_SETTEXT
                | WM_ENABLE
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_UPDATEUISTATE
                | BM_SETCHECK
                | BM_SETSTATE
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_KEYDOWN
                | WM_KEYUP
                | WM_CAPTURECHANGED
                | WM_STYLECHANGED
                | WM_THEMECHANGED
                | CB_SETCURSEL
                | CB_SHOWDROPDOWN
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

    fn light_palette() -> Palette {
        Palette {
            background: 0xffffff,
            surface: 0xffffff,
            text: 0x262220,
            disabled: 0x756d66,
            accent: 0xd77800,
            border: 0xe3ded9,
            accent_surface: 0xfcf0e5,
        }
    }

    #[test]
    fn combo_spacing_scales_without_overlapping_arrow_or_rounded_edge() {
        for dpi in [96, 144, 192, 288, 768] {
            let client = RECT {
                left: 5,
                top: 3,
                right: 5 + px(180, dpi),
                bottom: 3 + px(30, dpi),
            };
            let (text, arrow) = combo_rects(client, dpi);
            assert_eq!(text.left - client.left, px(11, dpi));
            assert_eq!(arrow.left - text.right, px(3, dpi));
            assert_eq!(client.right - arrow.right, px(1, dpi));
            assert!(arrow.left < arrow.right);
            assert!(text.top > client.top && text.bottom < client.bottom);
        }
        for width in [0, 1, 8, 20] {
            let client = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: 5,
            };
            let (text, arrow) = combo_rects(client, 192);
            assert!(text.left >= client.left && text.right >= text.left);
            assert!(text.right <= arrow.left && arrow.right <= client.right);
            assert!(arrow.left <= arrow.right && arrow.top <= arrow.bottom);
        }
    }

    #[test]
    fn app_themes_use_solid_checked_tracks_and_distinct_disabled_colors() {
        for p in [light_palette(), palette()] {
            let (track, border, knob) = switch_colors(p, true, true, false, false);
            assert_eq!(track, p.accent);
            assert_eq!(border, p.accent);
            assert_eq!(knob, p.on_accent());
            assert_ne!(knob, track);
            let off = switch_colors(p, false, true, false, false);
            assert_ne!(off.0, track);
            assert_eq!(off.2, p.disabled);
            for checked in [false, true] {
                let normal = switch_colors(p, checked, true, false, false);
                let hover = switch_colors(p, checked, true, true, false);
                let pressed = switch_colors(p, checked, true, false, true);
                let disabled = switch_colors(p, checked, false, false, false);
                assert_ne!(hover.0, normal.0);
                assert_ne!(pressed.0, hover.0);
                assert_eq!(hover.2, normal.2);
                assert_eq!(pressed.2, normal.2);
                assert_ne!(disabled.0, normal.0);
                assert_ne!(disabled.2, normal.2);
                assert_eq!(disabled, switch_colors(p, checked, false, true, true));
            }
            let normal = combo_colors(p, true, false, false);
            let hover = combo_colors(p, true, true, false);
            let focus = combo_colors(p, true, false, true);
            assert_eq!(normal.0, p.surface);
            assert_ne!(normal.0, hover.0);
            assert_ne!(normal.1, hover.1);
            assert_ne!(hover.1, focus.1);
            assert_eq!(combo_colors(p, false, false, false).2, p.disabled);
            assert_eq!(
                combo_colors(p, false, false, false),
                combo_colors(p, false, true, true)
            );
        }
    }

    #[test]
    fn table_colors_keep_selection_above_hover_and_subtle_stripes() {
        for p in [light_palette(), palette()] {
            assert_eq!(row_background(p, 0, false, false, true), p.surface);
            let stripe = blend(p.surface, p.subtle(), 50);
            assert_eq!(row_background(p, 1, false, false, true), stripe);
            assert_eq!(row_background(p, 3, false, false, true), stripe);
            assert_eq!(row_background(p, 1, false, true, true), p.hover());
            for row in [0, 1] {
                assert_eq!(row_background(p, row, true, false, true), p.accent_surface);
                assert_eq!(row_background(p, row, true, true, true), p.accent_surface);
                assert_eq!(
                    row_background(p, row, true, true, false),
                    blend(p.surface, p.accent_surface, 50)
                );
                assert_eq!(
                    row_background(p, row, false, true, false),
                    row_background(p, row, false, false, false)
                );
            }
        }
    }

    #[test]
    fn class_and_style_filter_do_not_confuse_other_native_controls() {
        assert!(identify("Edit", BS_AUTOCHECKBOX as u32).is_none());
        assert!(identify("ComboBox", CBS_DROPDOWN as u32).is_none());
        assert!(identify("Button", BS_AUTORADIOBUTTON as u32).is_none());
        assert!(identify("Button", BS_OWNERDRAW as u32).is_none());
        assert!(identify("Button", BS_AUTOCHECKBOX as u32 | WS_TABSTOP) == Some(Control::Switch));
        assert!(identify("SysListView32", LVS_REPORT | LVS_SINGLESEL) == Some(Control::List));
    }

    #[test]
    fn switch_geometry_scales_and_preserves_check_direction() {
        for dpi in [96, 144, 192, 288] {
            let client = RECT {
                left: 0,
                top: 0,
                right: px(240, dpi),
                bottom: px(32, dpi),
            };
            for right in [false, true] {
                let (track, off, label) = switch_rects(client, dpi, false, right);
                let (_, on, _) = switch_rects(client, dpi, true, right);
                assert_eq!(track.right - track.left, px(34, dpi));
                assert!(off.left >= track.left && on.right <= track.right);
                assert!(off.left < on.left);
                assert!(off.top >= track.top && on.bottom <= track.bottom);
                assert!(label.left >= client.left && label.right <= client.right);
                if right {
                    assert!(label.right < track.left);
                } else {
                    assert!(label.left > track.right);
                }
            }
        }
        let tiny = RECT {
            left: 0,
            top: 0,
            right: 8,
            bottom: 5,
        };
        let (track, knob, _) = switch_rects(tiny, 192, true, false);
        assert!(track.right <= tiny.right && track.bottom <= tiny.bottom);
        assert!(knob.left >= track.left && knob.right <= track.right);
    }

    unsafe fn window(class: &str, style: u32, parent: HWND) -> HWND {
        // SAFETY: test-owned native HWNDs, no external data or application State.
        unsafe {
            let class: Vec<u16> = class.encode_utf16().chain(Some(0)).collect();
            let caption: Vec<u16> = "Test &switch".encode_utf16().chain(Some(0)).collect();
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                caption.as_ptr(),
                style,
                0,
                0,
                360,
                40,
                parent,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                null(),
            );
            assert!(!hwnd.is_null());
            hwnd
        }
    }

    #[test]
    fn native_controls_keep_state_and_restore_native_rendering() {
        // SAFETY: all native windows are created/destroyed on this test thread;
        // test never starts the application, network, disk I/O or real user data.
        unsafe {
            let init = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            assert_ne!(InitCommonControlsEx(&init), 0);
            let parent = window("STATIC", WS_POPUP, std::ptr::null_mut());
            let switch = window(
                "BUTTON",
                WS_CHILD | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                parent,
            );
            let native_switch = window(
                "BUTTON",
                WS_CHILD | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                parent,
            );
            let combo = window("COMBOBOX", WS_CHILD | CBS_DROPDOWNLIST as u32, parent);
            let list = window("SysListView32", WS_CHILD | WS_TABSTOP | LVS_REPORT, parent);
            let p = palette();
            apply(list, Resolved::Light, light_palette(), 192);
            // Older common-controls versions create the header lazily. Inserting
            // its first column must not silently turn a light list/header dark.
            let mut column_text = [b'C' as u16, 0];
            let mut column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH,
                cx: 160,
                pszText: column_text.as_mut_ptr(),
                ..Default::default()
            };
            assert_eq!(
                SendMessageW(
                    list,
                    LVM_INSERTCOLUMNW,
                    0,
                    (&mut column as *mut LVCOLUMNW) as LPARAM
                ),
                0
            );
            let header = SendMessageW(list, LVM_GETHEADER, 0, 0) as HWND;
            assert!(!header.is_null());
            assert_eq!(appearance(list).unwrap().kind, Resolved::Light);
            assert_eq!(appearance(header).unwrap().kind, Resolved::Light);
            let mut row = LVITEMW {
                mask: LVIF_TEXT | LVIF_STATE,
                pszText: column_text.as_mut_ptr(),
                stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                state: LVIS_SELECTED | LVIS_FOCUSED,
                ..Default::default()
            };
            assert_eq!(
                SendMessageW(
                    list,
                    LVM_INSERTITEMW,
                    0,
                    (&mut row as *mut LVITEMW) as LPARAM
                ),
                0
            );
            row.iItem = 1;
            row.state = 0;
            assert_eq!(
                SendMessageW(
                    list,
                    LVM_INSERTITEMW,
                    0,
                    (&mut row as *mut LVITEMW) as LPARAM
                ),
                1
            );
            assert_eq!(
                SendMessageW(combo, CB_ADDSTRING, 0, column_text.as_ptr() as LPARAM),
                0
            );
            SendMessageW(combo, CB_SETCURSEL, 0, 0);
            let list_style = GetWindowLongPtrW(list, GWL_STYLE);
            let combo_style = GetWindowLongPtrW(combo, GWL_STYLE);
            let original_style = GetWindowLongPtrW(switch, GWL_STYLE);
            SendMessageW(switch, BM_SETCHECK, BST_CHECKED as WPARAM, 0);
            for kind in [
                Resolved::Dark,
                Resolved::Light,
                Resolved::Dark,
                Resolved::Contrast,
            ] {
                let colors = if kind == Resolved::Light {
                    light_palette()
                } else {
                    p
                };
                for hwnd in [switch, combo, list] {
                    apply(hwnd, kind, colors, 192);
                }
                assert_eq!(GetWindowLongPtrW(switch, GWL_STYLE), original_style);
                let scroll_styles = (WS_HSCROLL | WS_VSCROLL) as isize;
                assert_eq!(
                    GetWindowLongPtrW(list, GWL_STYLE) & !scroll_styles,
                    list_style & !scroll_styles
                );
                assert_eq!(GetWindowLongPtrW(combo, GWL_STYLE), combo_style);
                assert_eq!(
                    SendMessageW(switch, BM_GETCHECK, 0, 0),
                    BST_CHECKED as LRESULT
                );
                let expected = if kind != Resolved::Contrast {
                    colors.surface
                } else {
                    GetSysColor(COLOR_WINDOW)
                };
                assert_eq!(SendMessageW(list, LVM_GETBKCOLOR, 0, 0) as u32, expected);
                assert_eq!(
                    SendMessageW(list, LVM_GETTEXTBKCOLOR, 0, 0) as u32,
                    expected
                );
                for hwnd in [switch, combo, list] {
                    assert_eq!(appearance(hwnd).is_some(), kind != Resolved::Contrast);
                    if let Some(value) = appearance(hwnd) {
                        assert_eq!(value.kind, kind);
                    }
                }
                let header = SendMessageW(list, LVM_GETHEADER, 0, 0) as HWND;
                assert!(!header.is_null());
                assert_eq!(appearance(header).is_some(), kind != Resolved::Contrast);
                if kind == Resolved::Contrast {
                    for hwnd in [switch, combo, list, header] {
                        let mut data = 0;
                        assert_eq!(
                            GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data),
                            0
                        );
                    }
                }
                let expected_text = if kind != Resolved::Contrast {
                    colors.text
                } else {
                    GetSysColor(COLOR_WINDOWTEXT)
                };
                assert_eq!(
                    SendMessageW(list, LVM_GETTEXTCOLOR, 0, 0) as u32,
                    expected_text
                );
            }
            apply(switch, Resolved::Dark, p, 96);
            SendMessageW(switch, BM_CLICK, 0, 0);
            assert_eq!(
                SendMessageW(switch, BM_GETCHECK, 0, 0),
                BST_UNCHECKED as LRESULT
            );
            SendMessageW(switch, BM_CLICK, 0, 0);
            assert_eq!(
                SendMessageW(switch, BM_GETCHECK, 0, 0),
                BST_CHECKED as LRESULT
            );
            assert_eq!(SendMessageW(switch, WM_GETTEXTLENGTH, 0, 0), 12);
            for (kind, colors) in [(Resolved::Light, light_palette()), (Resolved::Dark, p)] {
                for left_text in [false, true] {
                    let style = if left_text {
                        original_style | BS_LEFTTEXT as isize
                    } else {
                        original_style
                    };
                    SetWindowLongPtrW(switch, GWL_STYLE, style);
                    SetWindowLongPtrW(native_switch, GWL_STYLE, style);
                    apply(switch, kind, colors, 96);
                    for hwnd in [native_switch, switch] {
                        SendMessageW(hwnd, BM_SETCHECK, BST_CHECKED as WPARAM, 0);
                    }
                    // Compare Space with an unmodified hidden native Button;
                    // this is not physical-input or screen-reader acceptance.
                    SendMessageW(native_switch, WM_KEYDOWN, 0x20, 1);
                    let pressed =
                        SendMessageW(native_switch, BM_GETSTATE, 0, 0) as u32 & BST_PUSHED;
                    SendMessageW(native_switch, WM_KEYUP, 0x20, 0xc000_0001u32 as LPARAM);
                    let checked = SendMessageW(native_switch, BM_GETCHECK, 0, 0);
                    SendMessageW(switch, WM_KEYDOWN, 0x20, 1);
                    assert_eq!(
                        SendMessageW(switch, BM_GETSTATE, 0, 0) as u32 & BST_PUSHED,
                        pressed
                    );
                    SendMessageW(switch, WM_KEYUP, 0x20, 0xc000_0001u32 as LPARAM);
                    assert_eq!(SendMessageW(switch, BM_GETCHECK, 0, 0), checked);
                    SendMessageW(switch, BM_SETCHECK, BST_CHECKED as WPARAM, 0);
                    assert_eq!(GetWindowLongPtrW(switch, GWL_STYLE), style);
                    set_hover(switch, true);
                    assert!(appearance(switch).unwrap().hover);
                    apply(switch, kind, colors, 144);
                    assert!(appearance(switch).unwrap().hover);
                    SendMessageW(switch, WM_MOUSELEAVE, 0, 0);
                    assert!(!appearance(switch).unwrap().hover);
                    set_hover(switch, true);
                    EnableWindow(switch, 0);
                    assert!(!appearance(switch).unwrap().hover);
                    assert_eq!(
                        SendMessageW(switch, BM_GETCHECK, 0, 0),
                        BST_CHECKED as LRESULT
                    );
                    assert_eq!(
                        GetWindowLongPtrW(switch, GWL_STYLE),
                        style | WS_DISABLED as isize
                    );
                    EnableWindow(switch, 1);
                }
            }
            SetWindowLongPtrW(switch, GWL_STYLE, original_style);
            SendMessageW(switch, BM_SETSTATE, 0, 0);
            for hwnd in [switch, combo, list] {
                apply(hwnd, Resolved::Dark, p, 96);
            }
            let mut draw = NMLVCUSTOMDRAW::default();
            draw.nmcd.hdr.hwndFrom = list;
            draw.nmcd.hdr.code = NM_CUSTOMDRAW;
            draw.nmcd.dwDrawStage = CDDS_PREPAINT;
            assert_eq!(
                custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM),
                Some(CDRF_NOTIFYITEMDRAW as LRESULT)
            );
            draw.nmcd.dwDrawStage = CDDS_ITEMPREPAINT;
            draw.nmcd.uItemState = CDIS_SELECTED | CDIS_FOCUS;
            assert_eq!(
                custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM),
                Some((CDRF_NEWFONT | CDRF_NOTIFYSUBITEMDRAW) as LRESULT)
            );
            assert_eq!(draw.clrTextBk, p.accent_surface);
            assert_ne!(draw.nmcd.uItemState & CDIS_FOCUS, 0);
            super::super::cell_table::install(list).unwrap();
            for (kind, colors) in [(Resolved::Light, light_palette()), (Resolved::Dark, p)] {
                apply(list, kind, colors, 96);
                draw.nmcd.dwDrawStage = CDDS_ITEMPREPAINT | CDDS_SUBITEM;
                draw.nmcd.dwItemSpec = 0;
                draw.nmcd.uItemState = CDIS_FOCUS;
                assert_eq!(
                    custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM),
                    Some((CDRF_NEWFONT | CDRF_NOTIFYSUBITEMDRAW | CDRF_NOTIFYPOSTPAINT) as LRESULT)
                );
                assert_eq!(draw.clrText, colors.text);
                assert_eq!(draw.clrTextBk, colors.accent_surface);
                assert_ne!(draw.nmcd.uItemState & CDIS_FOCUS, 0);
                draw.nmcd.dwItemSpec = 1;
                draw.nmcd.uItemState = 0;
                custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM);
                assert_eq!(draw.clrTextBk, blend(colors.surface, colors.subtle(), 50));
                draw.nmcd.uItemState = CDIS_HOT;
                custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM);
                assert_eq!(draw.clrTextBk, colors.hover());
                assert_eq!(draw.nmcd.uItemState & CDIS_HOT, 0);
                EnableWindow(list, 0);
                draw.nmcd.dwItemSpec = 0;
                custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM);
                assert_eq!(draw.clrText, colors.disabled);
                assert_eq!(
                    draw.clrTextBk,
                    blend(colors.surface, colors.accent_surface, 50)
                );
                EnableWindow(list, 1);
                assert_eq!(
                    SendMessageW(
                        list,
                        LVM_GETITEMSTATE,
                        0,
                        (LVIS_SELECTED | LVIS_FOCUSED) as LPARAM
                    ) as u32,
                    LVIS_SELECTED | LVIS_FOCUSED
                );
            }
            apply(list, Resolved::Contrast, p, 96);
            draw.nmcd.dwDrawStage = CDDS_ITEMPREPAINT | CDDS_SUBITEM;
            draw.nmcd.uItemState = CDIS_SELECTED | CDIS_FOCUS;
            draw.clrText = 0x123456;
            draw.clrTextBk = 0x654321;
            assert_eq!(
                custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM),
                Some(CDRF_NOTIFYPOSTPAINT as LRESULT)
            );
            assert_eq!(draw.clrText, 0x123456);
            assert_eq!(draw.clrTextBk, 0x654321);
            assert_eq!(draw.nmcd.uItemState, CDIS_SELECTED | CDIS_FOCUS);
            // Even native/high-contrast rendering must keep cell-table postpaint.
            draw.nmcd.dwDrawStage = CDDS_ITEMPOSTPAINT | CDDS_SUBITEM;
            assert_eq!(
                custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM),
                Some(CDRF_DODEFAULT as LRESULT)
            );
            apply(list, Resolved::Dark, p, 96);
            assert_eq!(SendMessageW(combo, CB_GETCURSEL, 0, 0), 0);
            draw.nmcd.hdr.hwndFrom = switch;
            assert_eq!(
                custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM),
                None
            );
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            assert!(!dc.is_null());
            let bitmap = CreateBitmap(360, 40, 1, 32, null());
            assert!(!bitmap.is_null());
            let old = SelectObject(dc, bitmap as HGDIOBJ);
            let header = SendMessageW(list, LVM_GETHEADER, 0, 0) as HWND;
            for (kind, colors) in [(Resolved::Light, light_palette()), (Resolved::Dark, p)] {
                for hwnd in [switch, combo, list] {
                    apply(hwnd, kind, colors, 96);
                }
                // Give the hidden header a deterministic print-only surface;
                // different Common Controls versions lay out hidden lists lazily.
                SetWindowPos(
                    header,
                    std::ptr::null_mut(),
                    0,
                    0,
                    360,
                    40,
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOMOVE,
                );
                for hwnd in [switch, combo, header] {
                    for enabled in [false, true] {
                        EnableWindow(hwnd, i32::from(enabled));
                        let color = SetTextColor(dc, 0x123456);
                        SendMessageW(hwnd, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                        assert_eq!(GetTextColor(dc), 0x123456);
                        SetTextColor(dc, color);
                        if hwnd == switch {
                            let mut client = RECT::default();
                            GetClientRect(switch, &mut client);
                            for checked in [false, true] {
                                SendMessageW(switch, BM_SETCHECK, WPARAM::from(checked), 0);
                                SendMessageW(
                                    switch,
                                    WM_PRINTCLIENT,
                                    dc as WPARAM,
                                    PRF_CLIENT as LPARAM,
                                );
                                let (track, knob, _) = switch_rects(client, 96, checked, false);
                                let (track_color, _, knob_color) =
                                    switch_colors(colors, checked, enabled, false, false);
                                assert_eq!(
                                    GetPixel(
                                        dc,
                                        (track.left + track.right) / 2,
                                        (track.top + track.bottom) / 2
                                    ),
                                    track_color
                                );
                                assert_eq!(
                                    GetPixel(
                                        dc,
                                        (knob.left + knob.right) / 2,
                                        (knob.top + knob.bottom) / 2
                                    ),
                                    knob_color
                                );
                            }
                        } else if hwnd == header {
                            let mut client = RECT::default();
                            GetClientRect(header, &mut client);
                            // The old right-hand column frame was visible even
                            // at the top; now only the bottom has a separator.
                            assert_eq!(GetPixel(dc, column.cx - 1, client.top), colors.subtle());
                            assert_eq!(
                                GetPixel(dc, column.cx - 1, client.bottom - 1),
                                blend(colors.subtle(), colors.border, 65)
                            );
                        }
                    }
                }
                // Both overlays must leave native WM_PAINT to consume the region.
                InvalidateRect(combo, null(), 1);
                SendMessageW(combo, WM_PAINT, 0, 0);
                assert_eq!(GetUpdateRect(combo, std::ptr::null_mut(), 0), 0);
            }
            use windows_sys::Win32::System::Threading::{
                GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
            };
            let process = GetCurrentProcess();
            let before = GetGuiResources(process, GR_GDIOBJECTS);
            for _ in 0..200 {
                for hwnd in [switch, combo, header] {
                    SendMessageW(hwnd, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                }
            }
            let after = GetGuiResources(process, GR_GDIOBJECTS);
            assert_eq!(
                before, after,
                "repeated control painting leaked GDI objects"
            );
            println!("GDI objects across 600 print passes: {before} -> {after}");
            SelectObject(dc, old);
            DeleteObject(bitmap as HGDIOBJ);
            DeleteDC(dc);
            DestroyWindow(parent);
        }
    }
}
