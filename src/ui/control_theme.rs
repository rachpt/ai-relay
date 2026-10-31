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
    pointer: Option<(i32, i32)>,
    press_origin: Option<(i32, i32)>,
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
                let previous = *(data as *const Appearance);
                value.hover = previous.hover;
                value.pointer = previous.pointer;
                value.press_origin = previous.press_origin;
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
                    pointer: None,
                    press_origin: None,
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
        blend(p.surface, p.background, 40)
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

fn chevron_points(rect: RECT, dpi: u32, up: bool) -> Option<[POINT; 3]> {
    let width = (rect.right - rect.left).max(0);
    let height = (rect.bottom - rect.top).max(0);
    if width < 3 || height < 3 {
        return None;
    }
    let x = rect.left + width / 2;
    let y = rect.top + height / 2;
    let radius = px(4, dpi).min((width - 1) / 2).min(height - 1);
    let rise = (radius / 2).max(1).min((height - 1) / 2);
    let direction = if up { -1 } else { 1 };
    Some([
        POINT {
            x: x - radius,
            y: y - direction * rise,
        },
        POINT {
            x,
            y: y + direction * rise,
        },
        POINT {
            x: x + radius,
            y: y - direction * rise,
        },
    ])
}

unsafe fn chevron(dc: HDC, rect: &RECT, color: u32, dpi: u32, up: bool) {
    // SAFETY: all coordinates are scalar. The nested DC guard deselects the pen
    // before Pen::drop, including early returns, without deleting stock objects.
    unsafe {
        let Some(points) = chevron_points(*rect, dpi, up) else {
            return;
        };
        let raw = CreatePen(PS_SOLID, px(1, dpi), color);
        if raw.is_null() {
            return;
        }
        let pen = Pen(raw);
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        SelectObject(dc, pen.0 as HGDIOBJ);
        IntersectClipRect(dc, rect.left, rect.top, rect.right, rect.bottom);
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

fn indicator_ink(p: Palette, enabled: bool) -> u32 {
    if enabled {
        blend(p.text, p.disabled, 35)
    } else {
        blend(p.subtle(), p.disabled, 65)
    }
}

fn combo_colors(p: Palette, enabled: bool, hover: bool, active: bool) -> (u32, u32, u32) {
    if !enabled {
        return (
            p.subtle(),
            blend(p.subtle(), p.border, 60),
            indicator_ink(p, false),
        );
    }
    let surface = if hover || active {
        blend(p.surface, p.hover(), 60)
    } else {
        p.surface
    };
    let border = if active {
        p.accent
    } else if hover {
        blend(p.border, p.disabled, 20)
    } else {
        blend(p.surface, p.border, 85)
    };
    (surface, border, indicator_ink(p, true))
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
        let opened = enabled && SendMessageW(hwnd, CB_GETDROPPEDSTATE, 0, 0) != 0;
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
                    SetBkColor(dc, surface);
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
    let width = px(38, dpi).min((client.right - client.left).max(0));
    let height = px(20, dpi).min((client.bottom - client.top).max(0));
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
    let inset = px(3, dpi).min(height / 3).min(width / 3);
    let diameter = (height - 2 * inset).min(width - 2 * inset).max(0);
    let knob_top = top + (height - diameter) / 2;
    let knob_left = if checked {
        track.right - inset - diameter
    } else {
        left + inset
    };
    let knob = RECT {
        left: knob_left,
        top: knob_top,
        right: knob_left + diameter,
        bottom: knob_top + diameter,
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
    if !enabled {
        // Keep the stored check direction, but do not suggest an enabled accent action.
        let track = blend(
            blend(p.subtle(), p.background, 40),
            p.disabled,
            if checked { 12 } else { 0 },
        );
        return (
            track,
            blend(track, p.border, 60),
            blend(track, p.disabled, 65),
        );
    }
    if checked {
        let track = blend(
            p.accent,
            p.text,
            if pressed {
                12
            } else if hover {
                6
            } else {
                0
            },
        );
        (track, track, p.on_accent())
    } else {
        let track = blend(
            p.subtle(),
            p.disabled,
            if pressed {
                10
            } else if hover {
                5
            } else {
                0
            },
        );
        (track, blend(p.border, p.disabled, 70), p.disabled)
    }
}

unsafe fn switch_shapes(
    dc: HDC,
    track: RECT,
    knob: RECT,
    dpi: u32,
    background: u32,
    colors: (u32, u32, u32),
) {
    let (track_color, border, knob_color) = colors;
    let width = track.right.saturating_sub(track.left);
    let height = track.bottom.saturating_sub(track.top);
    if let Some(mut raster) = super::raster::Raster::new(width, height, background) {
        let radius = height as f32 * 0.5;
        let stroke = (dpi.clamp(96, 768) as f32 / 96.0).min(radius);
        raster.round_rect([0.0, 0.0, width as f32, height as f32], radius, border);
        raster.round_rect(
            [
                stroke,
                stroke,
                width as f32 - stroke,
                height as f32 - stroke,
            ],
            (radius - stroke).max(0.0),
            track_color,
        );
        raster.circle(
            (knob.left - track.left) as f32 + (knob.right - knob.left) as f32 * 0.5,
            (knob.top - track.top) as f32 + (knob.bottom - knob.top) as f32 * 0.5,
            (knob.right - knob.left) as f32 * 0.5,
            knob_color,
        );
        // SAFETY: Same live saved paint/print DC as the GDI fallback below.
        if unsafe { raster.paint(dc, track.left, track.top) } {
            return;
        }
    }
    // SAFETY: Caller saves the borrowed DC; only stock objects are selected.
    // Allocation/budget/driver failures retain the original readable GDI shapes.
    unsafe {
        SelectObject(dc, GetStockObject(DC_BRUSH));
        SelectObject(dc, GetStockObject(DC_PEN));
        SetDCPenColor(dc, border);
        SetDCBrushColor(dc, track_color);
        RoundRect(
            dc,
            track.left,
            track.top,
            track.right,
            track.bottom,
            height,
            height,
        );
        SetDCPenColor(dc, knob_color);
        SetDCBrushColor(dc, knob_color);
        Ellipse(dc, knob.left, knob.top, knob.right, knob.bottom);
    }
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
        switch_shapes(
            dc,
            track,
            knob,
            value.dpi,
            p.background,
            switch_colors(p, checked, enabled, value.hover, pressed),
        );
        let ink = if enabled { p.text } else { p.disabled };
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
            if focus.right > focus.left && focus.bottom > focus.top {
                SetBkColor(dc, p.background);
                DrawFocusRect(dc, &focus);
            }
        }
    }
}

unsafe fn header_enabled(hwnd: HWND) -> bool {
    // SAFETY: Owner-thread native Header. These queries inspect opaque live HWNDs
    // without callbacks. A disabled ListView does not set WS_DISABLED on its Header.
    unsafe {
        let parent = GetParent(hwnd);
        IsWindowEnabled(hwnd) != 0 && (parent.is_null() || IsWindowEnabled(parent) != 0)
    }
}

fn header_colors(p: Palette, enabled: bool, hot: bool, pressed: bool) -> (u32, u32) {
    let surface = if enabled && pressed {
        blend(p.subtle(), p.text, 8)
    } else if enabled && hot {
        blend(p.subtle(), p.text, 4)
    } else {
        p.subtle()
    };
    (
        surface,
        if enabled {
            blend(p.text, p.disabled, 35)
        } else {
            p.disabled
        },
    )
}

fn header_states(
    rect: RECT,
    dpi: u32,
    enabled: bool,
    pointer: Option<(i32, i32)>,
    press_origin: Option<(i32, i32)>,
) -> (bool, bool) {
    // Do not present a column-resize divider as a pressed sort button. This
    // affects artwork only; native Header hit testing still owns every action.
    let margin = px(3, dpi);
    let contains = |(x, y)| {
        x >= rect.left + margin && x < rect.right - margin && y >= rect.top && y < rect.bottom
    };
    let hot = enabled && pointer.is_some_and(contains);
    (hot, hot && press_origin.is_some_and(contains))
}

fn header_divider(rect: RECT, client: RECT, dpi: u32) -> Option<RECT> {
    let inset = px(8, dpi).min((rect.bottom - rect.top).max(0) / 3);
    (rect.right > client.left
        && rect.right < client.right
        && rect.bottom - inset > rect.top + inset)
        .then_some(RECT {
            left: rect.right - 1,
            right: rect.right,
            top: rect.top + inset,
            bottom: rect.bottom - inset,
        })
}

unsafe fn header_input(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) {
    // SAFETY: Owner-thread Header. TrackMouseEvent runs before the fresh subclass
    // lookup; no pointer into its Box crosses tracking, invalidation or forwarding.
    unsafe {
        if !matches!(
            message,
            WM_MOUSEMOVE
                | WM_MOUSELEAVE
                | WM_LBUTTONDOWN
                | WM_LBUTTONDBLCLK
                | WM_LBUTTONUP
                | WM_CAPTURECHANGED
                | WM_CANCELMODE
                | WM_ENABLE
                | WM_SHOWWINDOW
        ) {
            return;
        }
        let enabled = header_enabled(hwnd);
        if message == WM_MOUSEMOVE && enabled && appearance(hwnd).is_some_and(|v| !v.hover) {
            let mut event = MouseTracking {
                size: size_of::<MouseTracking>() as u32,
                flags: TRACK_MOUSE_LEAVE,
                hwnd,
                hover_time: 0,
            };
            if TrackMouseEvent(&mut event) == 0 {
                return;
            }
        }
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) == 0 || data == 0 {
            return;
        }
        let before = *(data as *const Appearance);
        if before.control != Control::Header || before.kind == Resolved::Contrast {
            return;
        }
        let mut value = before;
        let point = (
            l as u16 as i16 as i32,
            (l as u32 >> 16) as u16 as i16 as i32,
        );
        match message {
            WM_MOUSEMOVE if enabled => {
                value.pointer = Some(point);
                value.hover = true;
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK if enabled => {
                value.pointer = Some(point);
                value.press_origin = Some(point);
            }
            WM_LBUTTONUP => value.press_origin = None,
            WM_MOUSELEAVE => {
                value.pointer = None;
                value.hover = false;
            }
            WM_CANCELMODE | WM_CAPTURECHANGED => value.press_origin = None,
            WM_ENABLE | WM_SHOWWINDOW if w == 0 => {
                value.pointer = None;
                value.press_origin = None;
                value.hover = false;
            }
            _ => {}
        }
        if (value.hover, value.pointer, value.press_origin)
            != (before.hover, before.pointer, before.press_origin)
        {
            *(data as *mut Appearance) = value;
            InvalidateRect(hwnd, null(), 0);
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
        let enabled = header_enabled(hwnd);
        let buttons = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & HDS_BUTTONS != 0;
        let show_focus = enabled
            && GetFocus() == hwnd
            && SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) as u32 & UISF_HIDEFOCUS == 0;
        let focused = if show_focus {
            SendMessageW(hwnd, HDM_GETFOCUSEDITEM, 0, 0)
        } else {
            -1
        };
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
            let (hot, pressed) = header_states(
                rect,
                value.dpi,
                enabled && buttons,
                value.pointer,
                value.press_origin,
            );
            let (surface, ink) = header_colors(p, enabled, hot, pressed);
            if hot || pressed {
                fill(dc, &rect, surface);
            }
            SetTextColor(dc, ink);
            let mut text = rect;
            text.left += px(10, value.dpi);
            text.right = (text.right - px(10, value.dpi)).max(text.left);
            if item.fmt & (HDF_SORTUP | HDF_SORTDOWN) != 0 {
                let arrow = RECT {
                    left: (text.right - px(16, value.dpi)).max(text.left),
                    ..text
                };
                chevron(
                    dc,
                    &arrow,
                    indicator_ink(p, enabled),
                    value.dpi,
                    item.fmt & HDF_SORTUP != 0,
                );
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
            if let Some(divider) = header_divider(rect, client, value.dpi) {
                fill(dc, &divider, blend(p.subtle(), p.border, 65));
            }
            if focused == index {
                let mut focus = rect;
                let inset = px(3, value.dpi);
                InflateRect(&mut focus, -inset, -inset);
                if focus.right > focus.left && focus.bottom > focus.top {
                    SetBkColor(dc, surface);
                    DrawFocusRect(dc, &focus);
                }
            }
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
        if message == WM_PRINT {
            // WM_PRINT's caller lends a live DC until dispatch returns. Preserve
            // the whole native pass, including synchronous child prints; the
            // guard retains no subclass pointer across detach or destruction.
            let Some(_saved) = SavedDc::new(w as HDC) else {
                return 0;
            };
            return DefSubclassProc(hwnd, message, w, l);
        }
        if value.control == Control::Header {
            header_input(hwnd, message, w, l);
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
                let _print_dc = if message == WM_PRINTCLIENT {
                    let Some(saved) = SavedDc::new(w as HDC) else {
                        return 0;
                    };
                    Some(saved)
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
        if value.control == Control::List && message == WM_ENABLE {
            let header = SendMessageW(hwnd, LVM_GETHEADER, 0, 0) as HWND;
            if !header.is_null() {
                header_input(header, WM_ENABLE, w, l);
                InvalidateRect(header, null(), 0);
            }
        }
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
            background: 0x202020,
            surface: 0x2b2b2b,
            text: 0xf5f5f5,
            disabled: 0xc5c5c5,
            accent: 0xffcd60,
            border: 0x454545,
            accent_surface: 0x473d2d,
        }
    }

    fn light_palette() -> Palette {
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
            assert_eq!(
                combo_colors(p, false, false, false).2,
                indicator_ink(p, false)
            );
            assert_ne!(normal.2, combo_colors(p, false, false, false).2);
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
            let stripe = blend(p.surface, p.background, 40);
            assert_eq!(row_background(p, 1, false, false, true), stripe);
            assert_eq!(row_background(p, 3, false, false, true), stripe);
            assert_ne!(stripe, p.surface);
            // Stripes recede into the page instead of looking like hovered rows.
            assert!(luminance(stripe) < luminance(p.surface));
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

    fn contrast(a: u32, b: u32) -> f64 {
        let a = luminance(a);
        let b = luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn assert_control_contrast(p: Palette) {
        for enabled in [false, true] {
            let text = if enabled { p.text } else { p.disabled };
            for hot in [false, true] {
                for active in [false, true] {
                    let combo = combo_colors(p, enabled, hot, active);
                    assert!(
                        contrast(text, combo.0) >= 4.5,
                        "combo label enabled={enabled}, hot={hot}, active={active}"
                    );
                    if enabled {
                        assert!(contrast(combo.2, combo.0) >= 4.5, "enabled combo arrow");
                    }
                    let (surface, ink) = header_colors(p, enabled, hot, active);
                    assert!(
                        contrast(ink, surface) >= 4.5,
                        "header label enabled={enabled}, hot={hot}, pressed={active}"
                    );
                    if enabled {
                        assert!(
                            contrast(indicator_ink(p, true), surface) >= 4.5,
                            "enabled header sort arrow"
                        );
                    }
                    for row in [0, 1] {
                        assert!(
                            contrast(text, row_background(p, row, active, hot, enabled)) >= 4.5,
                            "list row {row}, enabled={enabled}, selected={active}, hot={hot}"
                        );
                    }
                }
            }
            assert!(
                contrast(text, p.background) >= 4.5,
                "switch label enabled={enabled}"
            );
        }
        for checked in [false, true] {
            for hot in [false, true] {
                for pressed in [false, true] {
                    let (track, border, knob) = switch_colors(p, checked, true, hot, pressed);
                    assert!(
                        contrast(knob, track) >= 3.0,
                        "switch knob checked={checked}, hot={hot}, pressed={pressed}"
                    );
                    assert!(
                        contrast(border, p.background) >= 3.0,
                        "switch boundary checked={checked}, hot={hot}, pressed={pressed}"
                    );
                }
            }
        }
    }

    #[test]
    fn text_and_enabled_indicators_remain_readable_in_fixtures_and_real_palette() {
        for p in [light_palette(), palette()] {
            assert_control_contrast(p);
        }
        for mode in [
            crate::config::ThemeMode::Light,
            crate::config::ThemeMode::Dark,
        ] {
            let theme = crate::ui::theme::Theme::new(mode).unwrap();
            if theme.kind != Resolved::Contrast {
                assert_control_contrast(theme.palette);
            }
        }
    }

    #[test]
    fn combo_header_interactions_are_neutral_and_disabled_cannot_look_hot() {
        for p in [light_palette(), palette()] {
            for hot in [false, true] {
                let combo = combo_colors(p, true, hot, false);
                for color in [combo.0, combo.1, combo.2] {
                    assert_eq!(color & 255, color >> 8 & 255);
                    assert_eq!(color & 255, color >> 16 & 255);
                }
            }
            assert_ne!(indicator_ink(p, false), indicator_ink(p, true));
            for hot in [false, true] {
                for pressed in [false, true] {
                    assert_eq!(
                        header_colors(p, false, hot, pressed),
                        header_colors(p, false, false, false)
                    );
                    let (surface, ink) = header_colors(p, true, hot, pressed);
                    for color in [surface, ink] {
                        assert_eq!(color & 255, color >> 8 & 255);
                        assert_eq!(color & 255, color >> 16 & 255);
                    }
                }
            }
        }
    }

    #[test]
    fn chevrons_and_header_dividers_fit_fixed_control_geometry() {
        for dpi in [96, 144, 192] {
            for height in [28, 30, 36] {
                let client = RECT {
                    left: 5,
                    top: 3,
                    right: 5 + px(240, dpi),
                    bottom: 3 + px(height, dpi),
                };
                let (_, arrow) = combo_rects(client, dpi);
                for up in [false, true] {
                    let points = chevron_points(arrow, dpi, up).unwrap();
                    assert!(points.iter().all(|p| p.x >= arrow.left
                        && p.x < arrow.right
                        && p.y >= arrow.top
                        && p.y < arrow.bottom));
                    assert_eq!(points[0].y, points[2].y);
                    assert_eq!(points[0].x + points[2].x, 2 * points[1].x);
                    assert_eq!(points[1].y < points[0].y, up);
                }
                let column = RECT {
                    right: client.left + px(100, dpi),
                    ..client
                };
                let divider = header_divider(column, client, dpi).unwrap();
                assert_eq!(divider.right - divider.left, 1);
                assert!(divider.top > column.top && divider.bottom < column.bottom);
                assert!(header_divider(client, client, dpi).is_none());
                let center = (
                    (column.left + column.right) / 2,
                    (column.top + column.bottom) / 2,
                );
                assert_eq!(
                    header_states(column, dpi, true, Some(center), None),
                    (true, false)
                );
                assert_eq!(
                    header_states(column, dpi, true, Some(center), Some(center)),
                    (true, true)
                );
                assert_eq!(
                    header_states(column, dpi, false, Some(center), Some(center)),
                    (false, false)
                );
                assert_eq!(
                    header_states(
                        column,
                        dpi,
                        true,
                        Some((column.right - 1, center.1)),
                        Some(center)
                    ),
                    (false, false)
                );
                assert_eq!(
                    header_states(
                        column,
                        dpi,
                        true,
                        Some(center),
                        Some((column.right - 1, center.1))
                    ),
                    (true, false)
                );
            }
        }
        for width in 0..12 {
            for height in 0..8 {
                let rect = RECT {
                    right: width,
                    bottom: height,
                    ..RECT::default()
                };
                if let Some(points) = chevron_points(rect, 192, false) {
                    assert!(
                        points
                            .iter()
                            .all(|p| p.x >= 0 && p.x < width && p.y >= 0 && p.y < height)
                    );
                }
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
                assert_eq!(track.right - track.left, px(38, dpi));
                assert_eq!(track.bottom - track.top, px(20, dpi));
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
        for dpi in [96, 144, 192] {
            for width in [0, 1, 8, 34, 38, 240] {
                for height in [0, 1, 5, 28, 30, 36] {
                    let client = RECT {
                        left: 5,
                        top: 7,
                        right: 5 + px(width, dpi),
                        bottom: 7 + px(height, dpi),
                    };
                    for checked in [false, true] {
                        for right in [false, true] {
                            let (track, knob, label) = switch_rects(client, dpi, checked, right);
                            assert!(track.left >= client.left && track.right <= client.right);
                            assert!(track.top >= client.top && track.bottom <= client.bottom);
                            assert!(
                                knob.left >= track.left
                                    && knob.right <= track.right
                                    && knob.left <= knob.right
                            );
                            assert!(
                                knob.top >= track.top
                                    && knob.bottom <= track.bottom
                                    && knob.top <= knob.bottom
                            );
                            assert_eq!(knob.right - knob.left, knob.bottom - knob.top);
                            assert!(
                                label.left >= client.left
                                    && label.right <= client.right
                                    && label.left <= label.right
                            );
                        }
                    }
                }
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct DcState {
        colors: [u32; 4],
        objects: [HGDIOBJ; 4],
        modes: [i32; 5],
        viewport: (i32, i32),
        window: (i32, i32),
        position: (i32, i32),
        clip: (i32, i32, i32, i32, i32),
    }
    impl DcState {
        unsafe fn capture(dc: HDC) -> Self {
            // SAFETY: Borrowed test-owned memory DC; GDI writes only to locals.
            unsafe {
                let mut viewport = POINT::default();
                let mut window = POINT::default();
                let mut position = POINT::default();
                let mut clip = RECT::default();
                GetViewportOrgEx(dc, &mut viewport);
                GetWindowOrgEx(dc, &mut window);
                GetCurrentPositionEx(dc, &mut position);
                let clip_kind = GetClipBox(dc, &mut clip);
                Self {
                    colors: [
                        GetTextColor(dc),
                        GetBkColor(dc),
                        GetDCBrushColor(dc),
                        GetDCPenColor(dc),
                    ],
                    objects: [
                        GetCurrentObject(dc, OBJ_FONT as u32),
                        GetCurrentObject(dc, OBJ_PEN as u32),
                        GetCurrentObject(dc, OBJ_BRUSH as u32),
                        GetCurrentObject(dc, OBJ_BITMAP as u32),
                    ],
                    modes: [
                        GetBkMode(dc),
                        GetMapMode(dc),
                        GetROP2(dc),
                        GetPolyFillMode(dc),
                        GetStretchBltMode(dc),
                    ],
                    viewport: (viewport.x, viewport.y),
                    window: (window.x, window.y),
                    position: (position.x, position.y),
                    clip: (clip_kind, clip.left, clip.top, clip.right, clip.bottom),
                }
            }
        }
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

    struct SwitchTarget {
        dc: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
        parent: HWND,
    }
    impl SwitchTarget {
        unsafe fn new() -> Self {
            // SAFETY: All handles are privately owned by this test on one thread.
            // Initialize the partial owner before assertions for reliable cleanup.
            unsafe {
                let mut target = Self {
                    dc: CreateCompatibleDC(std::ptr::null_mut()),
                    bitmap: std::ptr::null_mut(),
                    previous: std::ptr::null_mut(),
                    parent: window("STATIC", WS_POPUP, std::ptr::null_mut()),
                };
                assert!(!target.dc.is_null());
                target.bitmap = CreateBitmap(720, 96, 1, 32, null());
                assert!(!target.bitmap.is_null());
                target.previous = SelectObject(target.dc, target.bitmap);
                assert!(!target.previous.is_null() && target.previous as isize != -1);
                target
            }
        }
    }
    impl Drop for SwitchTarget {
        fn drop(&mut self) {
            // SAFETY: Hidden child windows/subclasses die before their test-owned
            // DC/bitmap; no saved-DC guard can outlive this enclosing owner.
            unsafe {
                DestroyWindow(self.parent);
                if !self.previous.is_null() && self.previous as isize != -1 {
                    SelectObject(self.dc, self.previous);
                }
                if !self.bitmap.is_null() {
                    DeleteObject(self.bitmap);
                }
                DeleteDC(self.dc);
            }
        }
    }

    fn coverage(pixel: u32, background: u32, foreground: u32) -> bool {
        pixel != background
            && pixel != foreground
            && (1..16u32).any(|coverage| {
                pixel
                    == [0, 8, 16].into_iter().fold(0, |color, shift| {
                        let a = (background >> shift) & 255;
                        let b = (foreground >> shift) & 255;
                        color | (((a * (16 - coverage) + b * coverage + 8) / 16) << shift)
                    })
            })
    }

    #[test]
    fn antialiased_switch_print_keeps_native_state_caption_clip_and_gdi_stable() {
        use windows_sys::Win32::System::Threading::{
            GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
        };
        // SAFETY: Hidden native BUTTON, private memory DC and synthetic caption.
        // Never shows a window, sets focus, pumps input, or touches user data.
        unsafe {
            let target = SwitchTarget::new();
            let sentinel = 0x765432;
            let clear_bitmap = || {
                // Clear the whole bitmap, including clip holes, without changing
                // the borrowed DC state used by the next print message.
                let _saved = SavedDc::new(target.dc).unwrap();
                assert_ne!(SelectClipRgn(target.dc, std::ptr::null_mut()), ERROR);
                SetDCBrushColor(target.dc, sentinel);
                assert_ne!(
                    FillRect(
                        target.dc,
                        &RECT {
                            left: 0,
                            top: 0,
                            right: 720,
                            bottom: 96,
                        },
                        GetStockObject(DC_BRUSH),
                    ),
                    0
                );
            };
            let hwnd = window(
                "BUTTON",
                WS_CHILD | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                target.parent,
            );
            let native_style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let caption: Vec<_> = "启用代理 &switch".encode_utf16().chain(Some(0)).collect();
            SendMessageW(hwnd, WM_SETTEXT, 0, caption.as_ptr() as LPARAM);
            let focus = GetFocus();
            for dpi in [96, 144, 192] {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    px(240, dpi),
                    px(32, dpi),
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOREDRAW,
                );
                let mut client = RECT::default();
                GetClientRect(hwnd, &mut client);
                for (kind, p) in [
                    (Resolved::Light, light_palette()),
                    (Resolved::Dark, palette()),
                ] {
                    for right in [false, true] {
                        let style = native_style | if right { BS_LEFTTEXT as isize } else { 0 };
                        SetWindowLongPtrW(hwnd, GWL_STYLE, style);
                        apply(hwnd, kind, p, dpi);
                        for checked in [false, true] {
                            for (enabled, hover, pressed) in [
                                (true, false, false),
                                (true, true, false),
                                (true, false, true),
                                (false, true, true),
                            ] {
                                EnableWindow(hwnd, i32::from(enabled));
                                SendMessageW(hwnd, BM_SETCHECK, WPARAM::from(checked), 0);
                                SendMessageW(hwnd, BM_SETSTATE, WPARAM::from(pressed), 0);
                                set_hover(hwnd, hover);
                                let state = SendMessageW(hwnd, BM_GETSTATE, 0, 0);
                                SetTextColor(target.dc, 0x123456);
                                SetBkColor(target.dc, 0x234567);
                                SetDCPenColor(target.dc, 0x345678);
                                SetDCBrushColor(target.dc, 0x456789);
                                SetStretchBltMode(target.dc, HALFTONE);
                                let before = DcState::capture(target.dc);
                                for message in [WM_PRINT, WM_PRINTCLIENT] {
                                    clear_bitmap();
                                    assert_eq!(DcState::capture(target.dc), before);
                                    SendMessageW(
                                        hwnd,
                                        message,
                                        target.dc as WPARAM,
                                        PRF_CLIENT as LPARAM,
                                    );
                                    assert_eq!(DcState::capture(target.dc), before);
                                    assert_eq!(
                                        SendMessageW(hwnd, BM_GETCHECK, 0, 0),
                                        LRESULT::from(checked)
                                    );
                                    assert_eq!(SendMessageW(hwnd, BM_GETSTATE, 0, 0), state);
                                    assert_eq!(GetFocus(), focus);
                                    assert_eq!(
                                        GetWindowLongPtrW(hwnd, GWL_STYLE),
                                        style | if enabled { 0 } else { WS_DISABLED as isize }
                                    );
                                    let mut text = [0u16; 64];
                                    let count = SendMessageW(
                                        hwnd,
                                        WM_GETTEXT,
                                        text.len(),
                                        text.as_mut_ptr() as LPARAM,
                                    ) as usize;
                                    assert_eq!(&text[..count], &caption[..caption.len() - 1]);
                                    let (track, knob, _) =
                                        switch_rects(client, dpi, checked, right);
                                    let actual_pressed = state as u32 & BST_PUSHED != 0;
                                    let (track_color, border, knob_color) =
                                        switch_colors(p, checked, enabled, hover, actual_pressed);
                                    assert_eq!(
                                        GetPixel(
                                            target.dc,
                                            (track.left + track.right) / 2,
                                            (track.top + track.bottom) / 2
                                        ),
                                        track_color
                                    );
                                    assert_eq!(
                                        GetPixel(
                                            target.dc,
                                            (knob.left + knob.right) / 2,
                                            (knob.top + knob.bottom) / 2
                                        ),
                                        knob_color
                                    );
                                    let mut outer_aa = 0;
                                    let mut knob_aa = 0;
                                    for y in track.top..track.bottom {
                                        for x in track.left..track.right {
                                            let pixel = GetPixel(target.dc, x, y);
                                            outer_aa +=
                                                usize::from(coverage(pixel, p.background, border));
                                            if x >= knob.left
                                                && x < knob.right
                                                && y >= knob.top
                                                && y < knob.bottom
                                            {
                                                knob_aa += usize::from(coverage(
                                                    pixel,
                                                    track_color,
                                                    knob_color,
                                                ));
                                            }
                                        }
                                    }
                                    assert!(
                                        outer_aa >= 4,
                                        "outer coverage: message={message:#x}, dpi={dpi}, checked={checked}, enabled={enabled}"
                                    );
                                    assert!(
                                        knob_aa >= 4,
                                        "knob coverage: message={message:#x}, dpi={dpi}, checked={checked}, enabled={enabled}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
            EnableWindow(hwnd, 1);
            SendMessageW(hwnd, BM_SETSTATE, 0, 0);
            SetWindowLongPtrW(hwnd, GWL_STYLE, native_style);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                360,
                40,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOREDRAW,
            );
            apply(hwnd, Resolved::Dark, palette(), 96);
            SendMessageW(hwnd, BM_SETCHECK, BST_CHECKED as WPARAM, 0);
            set_hover(hwnd, false);
            let mut client = RECT::default();
            GetClientRect(hwnd, &mut client);
            let (track, knob, _) = switch_rects(client, 96, true, false);
            let p = palette();
            let (track_color, border, knob_color) = switch_colors(p, true, true, false, false);
            // Complex borrowed-print clip: verify the region and pixels in its
            // hole, not just a bounding box that could miss lost subregions.
            let dc = target.dc;
            let _saved = SavedDc::new(dc).unwrap();
            let clip = CreateRectRgn(0, 0, 360, 40);
            let hole = CreateRectRgn(5, 14, 11, 23);
            let after = CreateRectRgn(0, 0, 0, 0);
            assert!(!clip.is_null() && !hole.is_null() && !after.is_null());
            assert_eq!(CombineRgn(clip, clip, hole, RGN_DIFF), COMPLEXREGION);
            assert_ne!(SelectClipRgn(dc, clip), ERROR);
            let before = DcState::capture(dc);
            for message in [WM_PRINT, WM_PRINTCLIENT] {
                clear_bitmap();
                assert_eq!(DcState::capture(dc), before);
                assert_eq!(GetClipRgn(dc, after), 1);
                assert_ne!(EqualRgn(clip, after), 0);
                SendMessageW(hwnd, message, dc as WPARAM, PRF_CLIENT as LPARAM);
                assert_eq!(DcState::capture(dc), before);
                assert_eq!(GetClipRgn(dc, after), 1);
                assert_ne!(EqualRgn(clip, after), 0);
                let _inspect = SavedDc::new(dc).unwrap();
                assert_ne!(SelectClipRgn(dc, std::ptr::null_mut()), ERROR);
                assert_eq!(GetPixel(dc, 8, 18), sentinel);
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
                assert!((track.top..track.bottom).any(|y| {
                    (track.left..track.right)
                        .any(|x| coverage(GetPixel(dc, x, y), p.background, border))
                }));
                assert!((knob.top..knob.bottom).any(|y| {
                    (knob.left..knob.right)
                        .any(|x| coverage(GetPixel(dc, x, y), track_color, knob_color))
                }));
            }
            let process = GetCurrentProcess();
            let count = GetGuiResources(process, GR_GDIOBJECTS);
            for _ in 0..200 {
                SendMessageW(hwnd, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
            }
            assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), count);
            assert_eq!(DcState::capture(dc), before);
            DeleteObject(clip);
            DeleteObject(hole);
            DeleteObject(after);
        }
    }

    #[test]
    fn oversized_switch_shape_budget_keeps_readable_gdi_fallback() {
        // SAFETY: Private memory target only. Oversized track rejects CPU raster
        // allocation, while the original GDI geometry is still visible in it.
        unsafe {
            let target = SwitchTarget::new();
            let _saved = SavedDc::new(target.dc).unwrap();
            let track = RECT {
                left: 0,
                top: 0,
                right: 4097,
                bottom: 40,
            };
            let knob = RECT {
                left: 5,
                top: 5,
                right: 35,
                bottom: 35,
            };
            fill(
                target.dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: 720,
                    bottom: 96,
                },
                0x202020,
            );
            switch_shapes(
                target.dc,
                track,
                knob,
                96,
                0x202020,
                (0xb85f00, 0xb85f00, 0xffffff),
            );
            assert_eq!(GetPixel(target.dc, 200, 20), 0xb85f00);
            assert_eq!(GetPixel(target.dc, 20, 20), 0xffffff);
        }
    }

    #[test]
    fn native_header_feedback_keeps_sort_width_order_hit_testing_and_dc() {
        // SAFETY: Private never-shown native Header/parent and an owned bitmap;
        // all messages and presentation updates stay on their creating thread.
        unsafe {
            let init = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            assert_ne!(InitCommonControlsEx(&init), 0);
            let parent = window("STATIC", WS_POPUP, std::ptr::null_mut());
            let header = window("SysHeader32", WS_CHILD | HDS_BUTTONS | HDS_HOTTRACK, parent);
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            assert!(!dc.is_null());
            let bitmap = CreateBitmap(800, 120, 1, 32, null());
            assert!(!bitmap.is_null());
            let old = SelectObject(dc, bitmap);
            SelectObject(dc, GetStockObject(SYSTEM_FIXED_FONT));
            SelectObject(dc, GetStockObject(GRAY_BRUSH));
            SelectObject(dc, GetStockObject(BLACK_PEN));
            SetTextColor(dc, 0x123456);
            SetBkColor(dc, 0x234567);
            SetDCBrushColor(dc, 0x345678);
            SetDCPenColor(dc, 0x456789);
            SetBkMode(dc, OPAQUE as i32);
            SetViewportOrgEx(dc, 7, 9, std::ptr::null_mut());
            IntersectClipRect(dc, 0, 0, 700, 100);
            let font = GetStockObject(SYSTEM_FIXED_FONT);
            SendMessageW(header, WM_SETFONT, font as WPARAM, 0);
            let mut caption = crate::windows::wide("排序列");
            for index in 0..2 {
                let mut item = HDITEMW {
                    mask: HDI_TEXT | HDI_WIDTH | HDI_FORMAT,
                    cxy: 100,
                    pszText: caption.as_mut_ptr(),
                    fmt: HDF_STRING | if index == 0 { HDF_SORTUP } else { 0 },
                    ..Default::default()
                };
                assert_eq!(
                    SendMessageW(
                        header,
                        HDM_INSERTITEMW,
                        index,
                        (&mut item as *mut HDITEMW) as LPARAM
                    ),
                    index as LRESULT
                );
            }
            let style = GetWindowLongPtrW(header, GWL_STYLE);
            for dpi in [96, 144, 192] {
                MoveWindow(header, 0, 0, px(240, dpi), px(30, dpi), 0);
                for index in 0..2 {
                    let mut item = HDITEMW {
                        mask: HDI_WIDTH,
                        cxy: px(100, dpi),
                        ..Default::default()
                    };
                    assert_ne!(
                        SendMessageW(
                            header,
                            HDM_SETITEMW,
                            index,
                            (&mut item as *mut HDITEMW) as LPARAM
                        ),
                        0
                    );
                }
                let mut client = RECT::default();
                GetClientRect(header, &mut client);
                let mut first = RECT::default();
                assert_ne!(
                    SendMessageW(
                        header,
                        HDM_GETITEMRECT,
                        0,
                        (&mut first as *mut RECT) as LPARAM
                    ),
                    0
                );
                let center = POINT {
                    x: (first.left + first.right) / 2,
                    y: (first.top + first.bottom) / 2,
                };
                let point = (center.x as u16 as u32 | ((center.y as u16 as u32) << 16)) as LPARAM;
                for (kind, p) in [
                    (Resolved::Light, light_palette()),
                    (Resolved::Dark, palette()),
                ] {
                    apply(header, kind, p, dpi);
                    let sample_x = first.left + px(12, dpi);
                    let sample_y = first.top + px(3, dpi);
                    let before = DcState::capture(dc);
                    SendMessageW(header, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                    assert_eq!(
                        GetPixel(dc, sample_x, sample_y),
                        header_colors(p, true, false, false).0
                    );
                    SendMessageW(header, WM_MOUSEMOVE, 0, point);
                    assert_eq!(
                        appearance(header).unwrap().pointer,
                        Some((center.x, center.y))
                    );
                    SendMessageW(header, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                    assert_eq!(
                        GetPixel(dc, sample_x, sample_y),
                        header_colors(p, true, true, false).0
                    );
                    // Exercise pressed artwork without native Header's modal drag
                    // loop or desktop input; actual actions continue to be native.
                    header_input(header, WM_LBUTTONDOWN, 1, point);
                    SendMessageW(header, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                    assert_eq!(
                        GetPixel(dc, sample_x, sample_y),
                        header_colors(p, true, true, true).0
                    );
                    SendMessageW(header, WM_CANCELMODE, 0, 0);
                    assert!(appearance(header).unwrap().press_origin.is_none());
                    header_input(header, WM_LBUTTONDOWN, 1, point);
                    SendMessageW(header, WM_CAPTURECHANGED, 0, 0);
                    assert!(appearance(header).unwrap().press_origin.is_none());
                    SendMessageW(header, WM_MOUSELEAVE, 0, 0);
                    assert!(appearance(header).unwrap().pointer.is_none());
                    let text_right = first.right - px(10, dpi);
                    let arrow = RECT {
                        left: text_right - px(16, dpi),
                        right: text_right,
                        ..first
                    };
                    for enabled in [false, true] {
                        EnableWindow(header, i32::from(enabled));
                        if !enabled {
                            SendMessageW(header, WM_MOUSEMOVE, 0, point);
                            assert!(appearance(header).unwrap().pointer.is_none());
                        }
                        SendMessageW(header, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                        assert_eq!(
                            GetPixel(dc, sample_x, sample_y),
                            header_colors(p, enabled, false, false).0
                        );
                        let ink = indicator_ink(p, enabled);
                        let mut pixels = 0;
                        for y in arrow.top..arrow.bottom {
                            for x in arrow.left..arrow.right {
                                pixels += usize::from(GetPixel(dc, x, y) == ink);
                            }
                        }
                        assert!(pixels > 0, "header sort arrow missing, enabled={enabled}");
                        assert!(
                            pixels
                                < ((arrow.right - arrow.left) * (arrow.bottom - arrow.top))
                                    as usize
                        );
                        let divider = header_divider(first, client, dpi).unwrap();
                        assert_eq!(
                            GetPixel(dc, divider.left, (divider.top + divider.bottom) / 2),
                            blend(p.subtle(), p.border, 65)
                        );
                        assert_eq!(GetPixel(dc, divider.left, first.top), p.subtle());
                    }
                    EnableWindow(parent, 0);
                    assert!(!header_enabled(header));
                    SendMessageW(header, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                    assert_eq!(
                        GetPixel(dc, sample_x, sample_y),
                        header_colors(p, false, false, false).0
                    );
                    EnableWindow(parent, 1);
                    assert!(header_enabled(header));
                    assert_eq!(DcState::capture(dc), before);
                    let mut item = HDITEMW {
                        mask: HDI_WIDTH | HDI_FORMAT | HDI_ORDER,
                        ..Default::default()
                    };
                    assert_ne!(
                        SendMessageW(
                            header,
                            HDM_GETITEMW,
                            0,
                            (&mut item as *mut HDITEMW) as LPARAM
                        ),
                        0
                    );
                    assert_eq!(item.cxy, px(100, dpi));
                    assert_eq!(item.fmt & (HDF_SORTUP | HDF_SORTDOWN), HDF_SORTUP);
                    assert_eq!(item.iOrder, 0);
                    assert_eq!(GetWindowLongPtrW(header, GWL_STYLE), style);
                    assert_eq!(SendMessageW(header, WM_GETFONT, 0, 0), font as LRESULT);
                    let mut hit = HDHITTESTINFO {
                        pt: center,
                        ..Default::default()
                    };
                    assert_eq!(
                        SendMessageW(
                            header,
                            HDM_HITTEST,
                            0,
                            (&mut hit as *mut HDHITTESTINFO) as LPARAM
                        ),
                        0
                    );
                    let mut after = RECT::default();
                    SendMessageW(
                        header,
                        HDM_GETITEMRECT,
                        0,
                        (&mut after as *mut RECT) as LPARAM,
                    );
                    assert_eq!(
                        (first.left, first.top, first.right, first.bottom),
                        (after.left, after.top, after.right, after.bottom)
                    );
                    apply(header, Resolved::Contrast, p, dpi);
                    assert!(appearance(header).is_none());
                    assert_eq!(SendMessageW(header, HDM_GETITEMCOUNT, 0, 0), 2);
                }
            }
            SelectObject(dc, old);
            DeleteObject(bitmap);
            DeleteDC(dc);
            DestroyWindow(parent);
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
                assert_eq!(draw.clrTextBk, blend(colors.surface, colors.background, 40));
                draw.nmcd.uItemState = CDIS_HOT;
                custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM);
                assert_eq!(draw.clrTextBk, colors.hover());
                assert_eq!(draw.nmcd.uItemState & CDIS_HOT, 0);
                EnableWindow(list, 0);
                assert!(!header_enabled(header));
                assert!(appearance(header).unwrap().pointer.is_none());
                draw.nmcd.dwItemSpec = 0;
                custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM);
                assert_eq!(draw.clrText, colors.disabled);
                assert_eq!(
                    draw.clrTextBk,
                    blend(colors.surface, colors.accent_surface, 50)
                );
                EnableWindow(list, 1);
                assert!(header_enabled(header));
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
                        SetBkColor(dc, 0x234567);
                        SetDCBrushColor(dc, 0x345678);
                        SetDCPenColor(dc, 0x456789);
                        let before = DcState::capture(dc);
                        for message in [WM_PRINT, WM_PRINTCLIENT] {
                            SendMessageW(hwnd, message, dc as WPARAM, PRF_CLIENT as LPARAM);
                            assert_eq!(DcState::capture(dc), before, "print changed caller DC");
                        }
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
                        } else if hwnd == combo {
                            let mut client = RECT::default();
                            GetClientRect(combo, &mut client);
                            let (_, arrow) = combo_rects(client, 96);
                            let arrow_ink = indicator_ink(colors, enabled);
                            let mut pixels = 0;
                            for y in arrow.top..arrow.bottom {
                                for x in arrow.left..arrow.right {
                                    pixels += usize::from(GetPixel(dc, x, y) == arrow_ink);
                                }
                            }
                            assert!(pixels > 0, "combo arrow missing, enabled={enabled}");
                            assert!(
                                pixels
                                    < ((arrow.right - arrow.left) * (arrow.bottom - arrow.top))
                                        as usize
                            );
                            assert_eq!(SendMessageW(combo, CB_GETCURSEL, 0, 0), 0);
                        } else if hwnd == header {
                            let mut client = RECT::default();
                            GetClientRect(header, &mut client);
                            // Dividers are inset: the top stays clean while the
                            // resize boundary and shared bottom line remain legible.
                            assert_eq!(GetPixel(dc, column.cx - 1, client.top), colors.subtle());
                            assert_eq!(
                                GetPixel(dc, column.cx - 1, (client.top + client.bottom) / 2),
                                blend(colors.subtle(), colors.border, 65)
                            );
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
