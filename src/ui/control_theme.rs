//! Presentation-only subclasses for native controls. No controller/State pointer
//! is retained; native controls still own values, input, notifications and MSAA.
//! Call apply for each child after SetWindowTheme; route WM_NOTIFY to custom_draw
//! before the generic button custom-draw handler (control style bits overlap).
use super::theme::{Palette, Resolved};
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
// These documented user32 APIs are behind an otherwise unused input feature.
#[link(name = "user32")]
unsafe extern "system" {
    fn GetFocus() -> HWND;
    fn IsWindowEnabled(hwnd: HWND) -> i32;
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
    palette: Palette,
    dpi: u32,
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
            Some(*(data as *const Appearance))
        }
    }
}

unsafe fn attach(hwnd: HWND, value: Appearance) {
    // SAFETY: UI thread, live native control; each subclass owns exactly one Box.
    // Updating copies only colors/DPI, and no borrow spans a Win32 callback.
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) != 0 {
            if data != 0 {
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
    // SAFETY: same-thread lookup/removal; free only after successful removal, so
    // a failed removal cannot leave the installed callback with dangling data.
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) != 0
            && RemoveWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID) != 0
            && data != 0
        {
            drop(Box::from_raw(data as *mut Appearance));
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
            return;
        };
        let custom =
            kind == Resolved::Dark || (control == Control::Switch && kind == Resolved::Light);
        if custom {
            attach(
                hwnd,
                Appearance {
                    control,
                    palette,
                    dpi,
                },
            );
        } else {
            detach(hwnd);
        }
        if control == Control::List {
            let (background, text) = if kind == Resolved::Dark {
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
                let empty = [0u16];
                let theme = if kind == Resolved::Dark {
                    empty.as_ptr()
                } else {
                    null()
                };
                SetWindowTheme(header, theme, theme);
                apply(header, kind, palette, dpi);
            }
        }
        InvalidateRect(hwnd, null(), 1);
    }
}

/// Handle WM_NOTIFY before theme's generic button NM_CUSTOMDRAW dispatch.
/// Recognized native list/header notifications return DODEFAULT even in light
/// mode, preventing their style bits from being mistaken for button styles.
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
        let header = &*(l as *const NMHDR);
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
        // Copy the notification before any reentrant ListView messages. Cell
        // postpaint adds only a frame; existing/native text and row colors stay.
        let cell_flags = super::cell_table::custom_draw(
            *(l as *const NMLVCUSTOMDRAW),
            value.map(|v| (v.palette.text, v.palette.accent_surface)),
        );
        let Some(value) = value else {
            return Some(cell_flags as LRESULT);
        };
        let mut draw = *(l as *const NMLVCUSTOMDRAW);
        match draw.nmcd.dwDrawStage {
            CDDS_PREPAINT => Some(CDRF_NOTIFYITEMDRAW as LRESULT),
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
                let active = state & LVIS_SELECTED != 0
                    || draw.nmcd.uItemState & (CDIS_SELECTED | CDIS_HOT) != 0
                    || (hot >= 0 && hot as usize == draw.nmcd.dwItemSpec);
                draw.clrText = if IsWindowEnabled(hwnd) != 0 {
                    value.palette.text
                } else {
                    value.palette.disabled
                };
                draw.clrTextBk = if active {
                    value.palette.accent_surface
                } else {
                    value.palette.surface
                };
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
        let raw = CreatePen(PS_SOLID, px(2, dpi), color);
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
        let focused = GetFocus() == hwnd;
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        // Owner-drawn combos may display icons/arbitrary application content.
        // Leave that native contract intact and cover only their arrow button.
        if style & (CBS_OWNERDRAWFIXED | CBS_OWNERDRAWVARIABLE) as u32 == 0 {
            let mut client = RECT::default();
            GetClientRect(hwnd, &mut client);
            super::rounded::surface(
                dc,
                &client,
                value.dpi,
                p.background,
                p.surface,
                if focused { p.accent } else { p.border },
            );
            font(hwnd, dc);
            SetTextColor(dc, ink);
            let text = title(hwnd);
            let mut rect = info.rcItem;
            rect.left += px(5, value.dpi);
            rect.right = rect.right.min(info.rcButton.left - px(3, value.dpi));
            DrawTextW(
                dc,
                text.as_ptr(),
                text.len() as i32,
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            let ui = SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) as u32;
            if focused && ui & UISF_HIDEFOCUS == 0 {
                let mut focus = info.rcItem;
                InflateRect(&mut focus, -1, -1);
                DrawFocusRect(dc, &focus);
            }
        }
        fill(dc, &info.rcButton, p.surface);
        chevron(dc, &info.rcButton, ink, value.dpi, false);
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
        let border = if !enabled {
            p.disabled
        } else if checked || pressed {
            p.accent
        } else {
            p.border
        };
        SelectObject(dc, GetStockObject(DC_BRUSH));
        SelectObject(dc, GetStockObject(DC_PEN));
        SetDCPenColor(dc, border);
        SetDCBrushColor(
            dc,
            if checked && enabled {
                p.accent_surface
            } else {
                p.surface
            },
        );
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
        let knob_color = if checked && enabled { p.accent } else { ink };
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
        fill(dc, &client, p.surface);
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
            let separator = RECT {
                left: rect.right - 1,
                ..rect
            };
            fill(dc, &separator, p.border);
            let mut text = rect;
            text.left += px(8, value.dpi);
            text.right -= px(8, value.dpi);
            if item.fmt & (HDF_SORTUP | HDF_SORTDOWN) != 0 {
                let arrow = RECT {
                    left: (text.right - px(16, value.dpi)).max(text.left),
                    ..text
                };
                chevron(dc, &arrow, p.accent, value.dpi, item.fmt & HDF_SORTUP != 0);
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
        fill(dc, &line, p.border);
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
        if value.control == Control::List && message == LVM_INSERTCOLUMNW {
            // Some Common Controls versions create their header only on the
            // first column. Apply colors at that point, not just at theme refresh.
            apply(hwnd, Resolved::Dark, value.palette, value.dpi);
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
                | WM_LBUTTONUP
                | WM_KEYUP
                | WM_STYLECHANGED
                | WM_THEMECHANGED
                | CB_SETCURSEL
        ) {
            InvalidateRect(hwnd, null(), 0);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let combo = window("COMBOBOX", WS_CHILD | CBS_DROPDOWNLIST as u32, parent);
            let list = window("SysListView32", WS_CHILD | LVS_REPORT, parent);
            // Older common-controls versions create the header lazily.
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
            let mut row = LVITEMW {
                mask: LVIF_TEXT | LVIF_STATE,
                pszText: column_text.as_mut_ptr(),
                stateMask: LVIS_SELECTED,
                state: LVIS_SELECTED,
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
            assert_eq!(
                SendMessageW(combo, CB_ADDSTRING, 0, column_text.as_ptr() as LPARAM),
                0
            );
            SendMessageW(combo, CB_SETCURSEL, 0, 0);
            let p = palette();
            let original_style = GetWindowLongPtrW(switch, GWL_STYLE);
            SendMessageW(switch, BM_SETCHECK, BST_CHECKED as WPARAM, 0);
            for kind in [
                Resolved::Dark,
                Resolved::Light,
                Resolved::Dark,
                Resolved::Contrast,
            ] {
                for hwnd in [switch, combo, list] {
                    apply(hwnd, kind, p, 192);
                }
                assert_eq!(GetWindowLongPtrW(switch, GWL_STYLE), original_style);
                assert_eq!(
                    SendMessageW(switch, BM_GETCHECK, 0, 0),
                    BST_CHECKED as LRESULT
                );
                let expected = if kind == Resolved::Dark {
                    p.surface
                } else {
                    GetSysColor(COLOR_WINDOW)
                };
                assert_eq!(SendMessageW(list, LVM_GETBKCOLOR, 0, 0) as u32, expected);
                assert_eq!(
                    SendMessageW(list, LVM_GETTEXTBKCOLOR, 0, 0) as u32,
                    expected
                );
                assert_eq!(appearance(combo).is_some(), kind == Resolved::Dark);
                assert_eq!(appearance(switch).is_some(), kind != Resolved::Contrast);
                let header = SendMessageW(list, LVM_GETHEADER, 0, 0) as HWND;
                assert!(!header.is_null());
                assert_eq!(appearance(header).is_some(), kind == Resolved::Dark);
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
            draw.nmcd.dwDrawStage = CDDS_ITEMPREPAINT | CDDS_SUBITEM;
            draw.nmcd.uItemState = 0;
            custom_draw((&mut draw as *mut NMLVCUSTOMDRAW) as LPARAM);
            assert_eq!(draw.clrTextBk, p.accent_surface);
            assert_eq!(
                SendMessageW(list, LVM_GETITEMSTATE, 0, LVIS_SELECTED as LPARAM) as u32,
                LVIS_SELECTED
            );
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
            for hwnd in [switch, combo, header] {
                let color = SetTextColor(dc, 0x123456);
                SendMessageW(hwnd, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                assert_eq!(GetTextColor(dc), 0x123456);
                SetTextColor(dc, color);
            }
            // The native paint pass must validate its update region even though
            // Combo's dark overlay uses GetDC after DefSubclassProc returns.
            InvalidateRect(combo, null(), 1);
            SendMessageW(combo, WM_PAINT, 0, 0);
            assert_eq!(GetUpdateRect(combo, std::ptr::null_mut(), 0), 0);
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
