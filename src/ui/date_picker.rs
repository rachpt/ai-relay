//! Nullable native date boundaries. Integration (on the owner UI thread):
//! create(parent, id, "开始（含）") / create(parent, id, "结束（不含）");
//! call apply after the parent's theme selection and notify(l) from WM_NOTIFY.
//! WM_SETTEXT accepts only ISO YYYY-MM-DD or empty; WM_GETTEXT always returns
//! that value, never the display prefix. No history query or app State is used.
//!
//! Dark presentation tints the themed picker and uses native classic calendar
//! colors, including direct navigation animation paths. Native font/minimum and
//! popup-host sizing preserve the full grid/footer. Local popup chrome adapts
//! only native outer borders/gutters; navigation and styles remain untouched.
#![cfg(windows)]

use super::theme::{Palette, Resolved};
use chrono::{Datelike, NaiveDate};
use std::{mem::size_of, ptr::null};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SYSTEMTIME, WPARAM},
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{Controls::*, WindowsAndMessaging::*},
};

mod popup;
mod tint;

const PROPERTY: &[u16] = &[
    65, 73, 82, 101, 108, 97, 121, 46, 68, 97, 116, 101, 80, 105, 99, 107, 101, 114, 46, 118, 49, 0,
]; // AIRelay.DatePicker.v1
const SUBCLASS_ID: usize = 0x4454_5031;
const REFRESH_POPUP: u32 = WM_APP + 60;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}
// Documented user32 APIs whose windows-sys bindings require an unused feature.
#[link(name = "user32")]
unsafe extern "system" {
    fn GetFocus() -> HWND;
    fn IsWindowEnabled(hwnd: HWND) -> i32;
}

#[derive(Clone, Copy)]
struct Appearance {
    kind: Resolved,
    palette: Palette,
    dpi: u32,
}
struct Settings {
    label: String,
    appearance: Option<Appearance>,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn parse_date(text: &str) -> Result<Option<NaiveDate>, &'static str> {
    if text.is_empty() {
        return Ok(None);
    }
    let bytes = text.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return Err("日期必须为 YYYY-MM-DD 或空（1970–9999）");
    }
    let date = NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| "日期无效")?;
    if !(1970..=9999).contains(&date.year()) {
        return Err("日期必须在 1970-01-01 至 9999-12-31 之间");
    }
    Ok(Some(date))
}
fn system_time(date: NaiveDate) -> SYSTEMTIME {
    SYSTEMTIME {
        wYear: date.year() as u16,
        wMonth: date.month() as u16,
        wDay: date.day() as u16,
        wDayOfWeek: date.weekday().num_days_from_sunday() as u16,
        ..Default::default()
    }
}
fn display_format(label: &str) -> String {
    if label.is_empty() {
        "yyyy-MM-dd".into()
    } else {
        // A doubled apostrophe is a literal apostrophe inside a native literal.
        format!("'{} 'yyyy-MM-dd", label.replace('\'', "''"))
    }
}

/// Create an initially empty DTS_SHOWNONE date picker, with a display-only label.
/// The parent must be live and owned by the calling thread; layout/font are the
/// caller's responsibility. Native checkbox, dropdown, keyboard and MSAA remain.
/// The inclusive native range is 1970-01-01..=9999-12-31; boundary semantics and
/// time-zone conversion belong to history, not this control.
pub(super) fn create(parent: HWND, id: u32, label: &str) -> Result<HWND, String> {
    if label.contains('\0') || label.encode_utf16().count() > 128 {
        return Err("日期标签不能含 NUL，且不能超过 128 个 UTF-16 单元".into());
    }
    // SAFETY: validate the supplied parent/owner thread before creating children.
    // All messages below use scalars or live buffers and execute synchronously.
    unsafe {
        if parent.is_null()
            || GetWindowThreadProcessId(parent, std::ptr::null_mut()) != GetCurrentThreadId()
        {
            return Err("日期控件必须在父窗口所属的 UI 线程创建".into());
        }
        let init = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_DATE_CLASSES,
        };
        if InitCommonControlsEx(&init) == 0 {
            return Err(format!(
                "无法初始化日期控件：{}",
                std::io::Error::last_os_error()
            ));
        }
        let hwnd = CreateWindowExW(
            0,
            wide("SysDateTimePick32").as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | DTS_SHOWNONE,
            0,
            0,
            240,
            30,
            parent,
            id as usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        if hwnd.is_null() {
            return Err(format!(
                "无法创建日期控件：{}",
                std::io::Error::last_os_error()
            ));
        }
        let range = [
            SYSTEMTIME {
                wYear: 1970,
                wMonth: 1,
                wDay: 1,
                ..Default::default()
            },
            SYSTEMTIME {
                wYear: 9999,
                wMonth: 12,
                wDay: 31,
                ..Default::default()
            },
        ];
        let format = wide(&display_format(label));
        if SendMessageW(
            hwnd,
            DTM_SETRANGE,
            (GDTR_MIN | GDTR_MAX) as WPARAM,
            range.as_ptr() as LPARAM,
        ) == 0
            || SendMessageW(hwnd, DTM_SETFORMATW, 0, format.as_ptr() as LPARAM) == 0
            || SendMessageW(hwnd, DTM_SETSYSTEMTIME, GDT_NONE as WPARAM, 0) == 0
        {
            DestroyWindow(hwnd);
            return Err("无法设置日期格式、范围或初始空值".into());
        }
        let settings = Box::into_raw(Box::new(Settings {
            label: label.into(),
            appearance: None,
        }));
        if SetPropW(hwnd, PROPERTY.as_ptr(), settings.cast()) == 0 {
            let error = std::io::Error::last_os_error();
            drop(Box::from_raw(settings));
            DestroyWindow(hwnd);
            return Err(format!("无法标记日期控件：{error}"));
        }
        if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, settings as usize) == 0 {
            RemovePropW(hwnd, PROPERTY.as_ptr());
            drop(Box::from_raw(settings));
            DestroyWindow(hwnd);
            return Err("无法安装日期控件文本桥接".into());
        }
        Ok(hwnd)
    }
}

unsafe fn appearance(hwnd: HWND) -> Option<Appearance> {
    // SAFETY: only this module installs this property on its owner UI thread.
    // Copy before any reentrant native call; no reference to Settings escapes.
    unsafe {
        let data = GetPropW(hwnd, PROPERTY.as_ptr()) as *const Settings;
        if data.is_null() {
            None
        } else {
            (*data).appearance
        }
    }
}
unsafe fn current(hwnd: HWND) -> Option<NaiveDate> {
    // SAFETY: actual SDK SYSTEMTIME, writable until synchronous message returns.
    unsafe {
        let mut value = SYSTEMTIME::default();
        if SendMessageW(
            hwnd,
            DTM_GETSYSTEMTIME,
            0,
            (&mut value as *mut SYSTEMTIME) as LPARAM,
        ) != GDT_VALID as LRESULT
            || !(1970..=9999).contains(&value.wYear)
        {
            return None;
        }
        NaiveDate::from_ymd_opt(value.wYear.into(), value.wMonth.into(), value.wDay.into())
    }
}
unsafe fn iso(hwnd: HWND) -> String {
    // SAFETY: caller supplies this live native picker on its owner thread.
    unsafe { current(hwnd) }
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}
unsafe fn set_text(hwnd: HWND, text: *const u16) -> LRESULT {
    // SAFETY: WM_SETTEXT's non-null lParam is a readable NUL-terminated UTF-16
    // string. Stop at NUL; inspect at most eleven units, never allocate by input.
    unsafe {
        let mut buffer = [0u8; 10];
        let mut length = 0;
        if !text.is_null() {
            loop {
                let unit = *text.add(length);
                if unit == 0 {
                    break;
                }
                if length == buffer.len() || unit > 0x7f {
                    return 0;
                }
                buffer[length] = unit as u8;
                length += 1;
            }
        }
        let Ok(text) = std::str::from_utf8(&buffer[..length]) else {
            return 0;
        };
        let Ok(date) = parse_date(text) else {
            return 0;
        };
        let result = if let Some(date) = date {
            let value = system_time(date);
            SendMessageW(
                hwnd,
                DTM_SETSYSTEMTIME,
                GDT_VALID as WPARAM,
                (&value as *const SYSTEMTIME) as LPARAM,
            )
        } else {
            SendMessageW(hwnd, DTM_SETSYSTEMTIME, GDT_NONE as WPARAM, 0)
        };
        isize::from(result != 0)
    }
}

#[repr(C)]
struct HighContrast {
    size: u32,
    flags: u32,
    scheme: *mut u16,
}
fn high_contrast() -> bool {
    let mut value = HighContrast {
        size: size_of::<HighContrast>() as u32,
        flags: 0,
        scheme: std::ptr::null_mut(),
    };
    // SAFETY: layout is public HIGHCONTRASTW; query only, no system changes.
    // A failed preference query conservatively retains native drawing.
    unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            value.size,
            (&mut value as *mut HighContrast).cast(),
            0,
        ) == 0
            || value.flags & 1 != 0
    }
}

/// Apply after native theme selection, and repeat on theme/DPI changes.
/// # Safety
/// `hwnd` must be live on the calling UI thread. Foreign controls are ignored;
/// only create's property identifies a picker. No app State or GDI owner stored.
pub(super) unsafe fn apply(hwnd: HWND, kind: Resolved, palette: Palette, dpi: u32) {
    // SAFETY: owner-thread property access. No borrow survives the native calls.
    unsafe {
        let data = GetPropW(hwnd, PROPERTY.as_ptr()) as *mut Settings;
        if data.is_null() {
            return;
        }
        (*data).appearance = Some(Appearance {
            kind,
            palette,
            dpi: dpi.clamp(96, 768),
        });
        // Keep visual styles/geometry intact; tint changes the native raster,
        // not the control's theme metrics or native field-selection model.
        SetWindowTheme(hwnd, null(), null());
        tint::apply(hwnd, kind, palette);
        popup_colors(hwnd);
        InvalidateRect(hwnd, null(), 1);
    }
}

unsafe fn popup_colors(hwnd: HWND) {
    // SAFETY: caller supplies this live owner-thread picker. Scalars and borrowed
    // native font handles only; the control owns its ephemeral month calendar.
    unsafe {
        let owner = GetPropW(hwnd, PROPERTY.as_ptr()) as *const Settings;
        if owner.is_null() {
            return;
        }
        let value = appearance(hwnd);
        let dark = value.filter(|value| value.kind == Resolved::Dark && !high_contrast());

        let calendar = SendMessageW(hwnd, DTM_GETMONTHCAL, 0, 0) as HWND;
        if !calendar.is_null() {
            // Native month navigation uses a TimerProc that paints outside the
            // window procedure. Give that renderer dark source colors rather
            // than tinting only WM_PAINT/PRINT and leaving its final frame white.
            if let Some(value) = value {
                tint::apply(calendar, Resolved::Light, value.palette);
            }
            if dark.is_none()
                && let Some(value) = value
            {
                // Remove only local border presentation before restoring the
                // native light/contrast theme. No control styles were changed.
                popup::apply(calendar, Resolved::Light, value.palette);
                if GetPropW(hwnd, PROPERTY.as_ptr()) as *const Settings != owner
                    || SendMessageW(hwnd, DTM_GETMONTHCAL, 0, 0) as HWND != calendar
                {
                    return;
                }
            }
            let empty = [0u16];
            let theme = if dark.is_some() {
                empty.as_ptr()
            } else {
                null()
            };
            SetWindowTheme(calendar, theme, theme);
        }
        let colors = if let Some(value) = dark {
            let p = value.palette;
            // Classic monthcal reuses TITLEBK as weekday text ink. It must
            // contrast with MONTHBK as well as with the month title's text.
            [
                p.surface,
                p.text,
                p.accent,
                p.on_accent(),
                p.surface,
                p.disabled,
            ]
        } else {
            [
                GetSysColor(COLOR_WINDOW),
                GetSysColor(COLOR_WINDOWTEXT),
                GetSysColor(COLOR_ACTIVECAPTION),
                GetSysColor(COLOR_CAPTIONTEXT),
                GetSysColor(COLOR_WINDOW),
                GetSysColor(COLOR_GRAYTEXT),
            ]
        };
        for (part, color) in [
            MCSC_BACKGROUND,
            MCSC_TEXT,
            MCSC_TITLEBK,
            MCSC_TITLETEXT,
            MCSC_MONTHBK,
            MCSC_TRAILINGTEXT,
        ]
        .into_iter()
        .zip(colors)
        {
            SendMessageW(hwnd, DTM_SETMCCOLOR, part as WPARAM, color as LPARAM);
            if !calendar.is_null() {
                SendMessageW(calendar, MCM_SETCOLOR, part as WPARAM, color as LPARAM);
            }
        }
        let font = SendMessageW(hwnd, WM_GETFONT, 0, 0);
        let font = if font == 0 {
            GetStockObject(DEFAULT_GUI_FONT) as WPARAM
        } else {
            font as WPARAM
        };
        SendMessageW(hwnd, DTM_SETMCFONT, font, 1);
        if !calendar.is_null() {
            // Recompute after the DPI-scaled font is installed so native day
            // columns and the footer fit without inventing pixel dimensions.
            let mut minimum = RECT::default();
            let mut outer = RECT::default();
            let mut client = RECT::default();
            if SendMessageW(
                calendar,
                MCM_GETMINREQRECT,
                0,
                (&mut minimum as *mut RECT) as LPARAM,
            ) != 0
                && GetWindowRect(calendar, &mut outer) != 0
                && GetClientRect(calendar, &mut client) != 0
            {
                let today_width = SendMessageW(calendar, MCM_GETMAXTODAYWIDTH, 0, 0) as i32;
                let width = (minimum.right - minimum.left).max(today_width)
                    + (outer.right - outer.left - client.right).max(0);
                let height = minimum.bottom - minimum.top
                    + (outer.bottom - outer.top - client.bottom).max(0);
                SetWindowPos(
                    calendar,
                    std::ptr::null_mut(),
                    0,
                    0,
                    width,
                    height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                fit_popup_host(calendar);
            }
            if let Some(value) = dark {
                popup::apply(calendar, value.kind, value.palette);
                if GetPropW(hwnd, PROPERTY.as_ptr()) as *const Settings != owner
                    || SendMessageW(hwnd, DTM_GETMONTHCAL, 0, 0) as HWND != calendar
                {
                    return;
                }
            }
            RedrawWindow(
                calendar,
                null(),
                std::ptr::null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_FRAME,
            );
        }
    }
}

unsafe fn fit_popup_host(calendar: HWND) {
    // SAFETY: the native picker may embed the calendar in its own DropDown
    // popup. Resize only that same-thread native host, never the app/viewport.
    // Resizing just the child leaves its last columns/footer clipped by the
    // smaller themed host after switching the calendar renderer or DPI font.
    unsafe {
        let Some(host) = popup::native_host(calendar) else {
            return;
        };
        let mut child = RECT::default();
        let mut outer = RECT::default();
        let mut client = RECT::default();
        let mut origin = POINT::default();
        if GetWindowRect(calendar, &mut child) == 0
            || GetWindowRect(host, &mut outer) == 0
            || GetClientRect(host, &mut client) == 0
            || ClientToScreen(host, &mut origin) == 0
        {
            return;
        }
        let inset_x = (child.left - origin.x).max(0);
        let inset_y = (child.top - origin.y).max(0);
        let width =
            child.right - origin.x + inset_x + (outer.right - outer.left - client.right).max(0);
        let height =
            child.bottom - origin.y + inset_y + (outer.bottom - outer.top - client.bottom).max(0);
        let (mut x, mut y) = (outer.left, outer.top);
        if let Some(work) = super::overview::work_area(host) {
            x = x.clamp(work.left, (work.right - width).max(work.left));
            y = y.clamp(work.top, (work.bottom - height).max(work.top));
        }
        if width > 0
            && height > 0
            && (width != outer.right - outer.left
                || height != outer.bottom - outer.top
                || x != outer.left
                || y != outer.top)
        {
            SetWindowPos(
                host,
                std::ptr::null_mut(),
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// Route WM_NOTIFY here, including DTN_DROPDOWN, before application handling.
/// Returns true only for this component's dropdown; never submits a query or
/// suppresses DTN_DATETIMECHANGE. The parent may return zero when handled.
/// # Safety
/// `l` is zero or WM_NOTIFY's valid NMHDR pointer, delivered on the owner thread.
pub(super) unsafe fn notify(l: LPARAM) -> bool {
    // SAFETY: WM_NOTIFY guarantees a live NMHDR through this call. Copy fields
    // before sending native messages, which can reenter the parent's procedure.
    unsafe {
        if l == 0 {
            return false;
        }
        let header = &*(l as *const NMHDR);
        let (hwnd, code) = (header.hwndFrom, header.code);
        if code != DTN_DROPDOWN || GetPropW(hwnd, PROPERTY.as_ptr()).is_null() {
            return false;
        }
        popup_colors(hwnd);
        // The native picker can overwrite the popup size after the synchronous
        // dropdown notification returns. Finalize sizing on the next UI turn.
        PostMessageW(hwnd, REFRESH_POPUP, 0, 0);
        true
    }
}

fn px(value: i32, dpi: u32) -> i32 {
    ((i64::from(value) * i64::from(dpi.clamp(96, 768)) + 48) / 96) as i32
}
struct SavedDc {
    dc: HDC,
    index: i32,
}
impl SavedDc {
    unsafe fn new(dc: HDC) -> Option<Self> {
        // SAFETY: borrowed live paint/print DC; do not mutate if saving fails.
        let index = unsafe { SaveDC(dc) };
        (index != 0).then_some(Self { dc, index })
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: guard ends before its borrowed DC is released; stock objects
        // and the control's font are restored, never deleted by this component.
        unsafe { RestoreDC(self.dc, self.index) };
    }
}
unsafe fn fill(dc: HDC, rect: &RECT, color: u32, border: bool) {
    // SAFETY: saved live DC; stock DC_BRUSH is borrowed, not an owned resource.
    unsafe {
        SetDCBrushColor(dc, color);
        let brush = GetStockObject(DC_BRUSH) as HBRUSH;
        if border {
            FrameRect(dc, rect, brush);
        } else {
            FillRect(dc, rect, brush);
        }
    }
}
unsafe fn dark_snapshot(hwnd: HWND) -> Option<(Appearance, String)> {
    // SAFETY: property is module-owned. Copy before callbacks; never obscure
    // native focus/field highlights, an in-place child editor or an open popup.
    unsafe {
        let value = appearance(hwnd)?;
        if tint::active(hwnd)
            || value.kind != Resolved::Dark
            || high_contrast()
            || IsWindowEnabled(hwnd) == 0
        {
            return None;
        }
        let focus = GetFocus();
        if focus == hwnd
            || (!focus.is_null() && IsChild(hwnd, focus) != 0)
            || SendMessageW(hwnd, DTM_GETMONTHCAL, 0, 0) != 0
        {
            return None;
        }
        let data = GetPropW(hwnd, PROPERTY.as_ptr()) as *const Settings;
        if data.is_null() {
            None
        } else {
            Some((value, (*data).label.clone()))
        }
    }
}
unsafe fn draw_closed(hwnd: HWND, dc: HDC, value: Appearance, label: &str) {
    // SAFETY: live owner-thread control/DC; use documented picker rectangles so
    // the painted checkbox/button agree with native hit testing. Old comctl32
    // without picker-info support keeps its complete, readable native drawing.
    unsafe {
        let mut info = DATETIMEPICKERINFO {
            cbSize: size_of::<DATETIMEPICKERINFO>() as u32,
            ..Default::default()
        };
        SendMessageW(
            hwnd,
            DTM_GETDATETIMEPICKERINFO,
            0,
            (&mut info as *mut DATETIMEPICKERINFO) as LPARAM,
        );
        if info.rcCheck.right <= info.rcCheck.left
            || info.rcCheck.bottom <= info.rcCheck.top
            || info.rcButton.right <= info.rcButton.left
            || info.rcButton.bottom <= info.rcButton.top
        {
            return;
        }
        let Some(_saved) = SavedDc::new(dc) else {
            return;
        };
        let mut client = RECT::default();
        if GetClientRect(hwnd, &mut client) == 0 {
            return;
        }
        let p = value.palette;
        let date = current(hwnd);
        super::rounded::surface(dc, &client, value.dpi, p.background, p.surface, p.border);
        fill(dc, &info.rcCheck, p.disabled, true);
        SelectObject(dc, GetStockObject(DC_PEN));
        if date.is_some() {
            let r = info.rcCheck;
            let width = r.right - r.left;
            let height = r.bottom - r.top;
            SetDCPenColor(dc, p.accent);
            let points = [
                POINT {
                    x: r.left + width / 5,
                    y: r.top + height / 2,
                },
                POINT {
                    x: r.left + width * 2 / 5,
                    y: r.top + height * 3 / 4,
                },
                POINT {
                    x: r.right - width / 5,
                    y: r.top + height / 4,
                },
            ];
            Polyline(dc, points.as_ptr(), points.len() as i32);
        }
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
        SetTextColor(dc, if date.is_some() { p.text } else { p.disabled });
        let date = date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "未设置".into());
        let text = wide(&if label.is_empty() {
            date
        } else {
            format!("{label} {date}")
        });
        let mut text_rect = client;
        text_rect.left = info.rcCheck.right + px(5, value.dpi);
        text_rect.right = info.rcButton.left - px(3, value.dpi);
        if text_rect.right > text_rect.left {
            DrawTextW(
                dc,
                text.as_ptr(),
                (text.len() - 1) as i32,
                &mut text_rect,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
        }
        // Small calendar glyph inside the native dropdown's actual rectangle.
        let button = info.rcButton;
        let half = px(6, value.dpi)
            .min((button.right - button.left) / 3)
            .min((button.bottom - button.top) / 3)
            .max(1);
        let x = (button.left + button.right) / 2;
        let y = (button.top + button.bottom) / 2;
        let icon = RECT {
            left: x - half,
            top: y - half,
            right: x + half + 1,
            bottom: y + half + 1,
        };
        fill(dc, &icon, p.text, true);
        SetDCPenColor(dc, p.text);
        MoveToEx(dc, icon.left, y - half / 3, std::ptr::null_mut());
        LineTo(dc, icon.right, y - half / 3);
        for dx in [-half / 2, half / 2] {
            MoveToEx(
                dc,
                x + dx,
                icon.top - px(2, value.dpi),
                std::ptr::null_mut(),
            );
            LineTo(dc, x + dx, icon.top + px(3, value.dpi));
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
    // SAFETY: installed by create on this UI thread, with one owned Settings Box.
    // No reference to it survives native calls, which may reenter or destroy it.
    unsafe {
        match message {
            REFRESH_POPUP => {
                popup_colors(hwnd);
                return 0;
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(procedure), id);
                RemovePropW(hwnd, PROPERTY.as_ptr());
                let owned = Box::from_raw(data as *mut Settings);
                let result = DefSubclassProc(hwnd, message, w, l);
                drop(owned);
                return result;
            }
            WM_SETTEXT => return set_text(hwnd, l as *const u16),
            WM_GETTEXTLENGTH => return iso(hwnd).len() as LRESULT,
            WM_GETTEXT => {
                if w == 0 || l == 0 {
                    return 0;
                }
                let text = iso(hwnd);
                let count = text.len().min(w - 1);
                let output = l as *mut u16;
                for (i, byte) in text.bytes().take(count).enumerate() {
                    *output.add(i) = u16::from(byte);
                }
                *output.add(count) = 0;
                return count as LRESULT;
            }
            WM_PAINT | WM_PRINTCLIENT => {
                // Native paint first validates its update region. PRINTCLIENT's
                // caller-owned DC is saved around native painting as well.
                let _print_dc = if message == WM_PRINTCLIENT && w != 0 {
                    SavedDc::new(w as HDC)
                } else {
                    None
                };
                if message == WM_PRINTCLIENT && (w == 0 || _print_dc.is_none()) {
                    return 0;
                }
                let result = DefSubclassProc(hwnd, message, w, l);
                if let Some((value, label)) = dark_snapshot(hwnd) {
                    let dc = if message == WM_PAINT {
                        GetDC(hwnd)
                    } else {
                        w as HDC
                    };
                    if !dc.is_null() {
                        draw_closed(hwnd, dc, value, &label);
                        if message == WM_PAINT {
                            ReleaseDC(hwnd, dc);
                        }
                    }
                }
                return result;
            }
            _ => {}
        }
        let result = DefSubclassProc(hwnd, message, w, l);
        if (message == WM_SETFONT || message == WM_ENABLE)
            && !GetPropW(hwnd, PROPERTY.as_ptr()).is_null()
        {
            popup_colors(hwnd);
        }
        if matches!(
            message,
            DTM_SETSYSTEMTIME
                | WM_ENABLE
                | WM_SETFONT
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_THEMECHANGED
                | WM_SETTINGCHANGE
                | WM_UPDATEUISTATE
                | WM_KEYUP
                | WM_LBUTTONUP
                | DTM_CLOSEMONTHCAL
        ) {
            InvalidateRect(hwnd, null(), 0);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_iso_boundaries_and_empty() {
        assert_eq!(parse_date(""), Ok(None));
        for text in [
            "1970-01-01",
            "9999-12-31",
            "2000-02-29",
            "2024-02-29",
            "2100-02-28",
        ] {
            let date = parse_date(text).unwrap().unwrap();
            assert_eq!(date.format("%Y-%m-%d").to_string(), text);
            let native = system_time(date);
            assert_eq!(i32::from(native.wYear), date.year());
            assert_eq!(
                u32::from(native.wDayOfWeek),
                date.weekday().num_days_from_sunday()
            );
            assert_eq!(
                (
                    native.wHour,
                    native.wMinute,
                    native.wSecond,
                    native.wMilliseconds
                ),
                (0, 0, 0, 0)
            );
        }
    }
    #[test]
    fn rejects_invalid_dates_without_normalizing() {
        for text in [
            "1969-12-31",
            "10000-01-01",
            "0000-01-01",
            "2025-02-29",
            "1900-02-29",
            "2100-02-29",
            "9999-02-29",
            "2024-04-31",
            "2024-00-01",
            "2024-13-01",
            "2024-01-00",
            "2024-01-32",
            "2024-2-01",
            "2024-02-1",
            " 2024-02-29",
            "2024-02-29 ",
            " ",
            "2024/02/29",
            "２０２４-02-29",
            "2024-02-29T00:00:00",
            "2024-02-29\0",
        ] {
            assert!(parse_date(text).is_err(), "{text:?}");
        }
    }
    #[test]
    fn native_display_literal_does_not_become_value() {
        assert_eq!(display_format(""), "yyyy-MM-dd");
        assert_eq!(display_format("开始（含）"), "'开始（含） 'yyyy-MM-dd");
        assert_eq!(display_format("end's"), "'end''s 'yyyy-MM-dd");
    }

    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: test-owned hidden windows remain on the creating thread.
            // Destroying the parent also destroys children and their subclasses.
            unsafe { DestroyWindow(self.0) };
        }
    }
    fn host() -> Window {
        // SAFETY: hidden test window, no application State, disk or network use.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                null(),
                WS_POPUP,
                0,
                0,
                400,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        assert!(!hwnd.is_null());
        Window(hwnd)
    }
    unsafe fn text(hwnd: HWND) -> String {
        // SAFETY: test-owned live HWND; buffer capacity includes its terminator.
        unsafe {
            let length = GetWindowTextLengthW(hwnd);
            assert!(length == 0 || length == 10);
            let mut buffer = [0u16; 11];
            let count = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
            assert_eq!(count, length);
            String::from_utf16(&buffer[..count as usize]).unwrap()
        }
    }
    unsafe fn set(hwnd: HWND, text: &str) -> LRESULT {
        // SAFETY: local UTF-16 string lives until synchronous WM_SETTEXT returns.
        unsafe { SendMessageW(hwnd, WM_SETTEXT, 0, wide(text).as_ptr() as LPARAM) }
    }

    #[test]
    fn native_text_bridge_range_and_standard_systemtime_on_owner_thread() {
        let parent = host();
        let start = create(parent.0, 2302, "开始（含）").unwrap();
        let end = create(parent.0, 2303, "结束（不含）").unwrap();
        // SAFETY: all HWNDs/buffers are test-owned on this thread; no app runs.
        unsafe {
            assert_eq!(
                GetWindowThreadProcessId(start, std::ptr::null_mut()),
                GetCurrentThreadId()
            );
            assert_ne!(GetWindowLongPtrW(start, GWL_STYLE) as u32 & DTS_SHOWNONE, 0);
            assert_ne!(GetWindowLongPtrW(start, GWL_STYLE) as u32 & WS_TABSTOP, 0);
            let mut native = SYSTEMTIME::default();
            assert_eq!(
                SendMessageW(
                    start,
                    DTM_GETSYSTEMTIME,
                    0,
                    (&mut native as *mut SYSTEMTIME) as LPARAM
                ),
                GDT_NONE as LRESULT
            );
            assert_eq!(text(start), "");
            assert_eq!(set(start, "2025-02-29"), 0);
            assert_eq!(text(start), "");
            assert_eq!(text(end), "");
            let mut range = [SYSTEMTIME::default(); 2];
            assert_eq!(
                SendMessageW(start, DTM_GETRANGE, 0, range.as_mut_ptr() as LPARAM),
                (GDTR_MIN | GDTR_MAX) as LRESULT
            );
            assert_eq!(
                (range[0].wYear, range[0].wMonth, range[0].wDay),
                (1970, 1, 1)
            );
            assert_eq!(
                (range[1].wYear, range[1].wMonth, range[1].wDay),
                (9999, 12, 31)
            );
            for value in ["1970-01-01", "2024-02-29", "9999-12-31"] {
                assert_ne!(set(start, value), 0);
                assert_eq!(text(start), value);
                assert_eq!(
                    SendMessageW(
                        start,
                        DTM_GETSYSTEMTIME,
                        0,
                        (&mut native as *mut SYSTEMTIME) as LPARAM
                    ),
                    GDT_VALID as LRESULT
                );
                assert_eq!(native.wYear, value[..4].parse::<u16>().unwrap());
                assert_eq!(native.wMonth, value[5..7].parse::<u16>().unwrap());
                assert_eq!(native.wDay, value[8..].parse::<u16>().unwrap());
            }
            for invalid in [
                "2025-02-29",
                "1969-12-31",
                "10000-01-01",
                "2024-2-29",
                " ",
                "中文",
                "2024-02-29extra",
            ] {
                assert_eq!(set(start, invalid), 0);
                assert_eq!(text(start), "9999-12-31");
            }
            // Native keyboard/calendar changes are read live, not from a cache.
            native = SYSTEMTIME {
                wYear: 2000,
                wMonth: 2,
                wDay: 29,
                wHour: 12,
                ..Default::default()
            };
            assert_ne!(
                SendMessageW(
                    start,
                    DTM_SETSYSTEMTIME,
                    GDT_VALID as WPARAM,
                    (&native as *const SYSTEMTIME) as LPARAM
                ),
                0
            );
            assert_eq!(text(start), "2000-02-29");
            let mut buffer = [0xaaaau16; 12];
            assert_eq!(
                SendMessageW(start, WM_GETTEXT, 6, buffer.as_mut_ptr() as LPARAM),
                5
            );
            assert_eq!(&buffer[..7], &[50, 48, 48, 48, 45, 0, 0xaaaa]);
            assert_eq!(
                SendMessageW(start, WM_GETTEXT, 1, buffer.as_mut_ptr() as LPARAM),
                0
            );
            assert_eq!(buffer[0], 0);
            buffer[0] = 0xaaaa;
            assert_eq!(
                SendMessageW(start, WM_GETTEXT, 0, buffer.as_mut_ptr() as LPARAM),
                0
            );
            assert_eq!(buffer[0], 0xaaaa);
            assert_eq!(SendMessageW(start, WM_GETTEXT, 11, 0), 0);
            assert_ne!(set(start, ""), 0);
            assert_eq!(text(start), "");
            assert_eq!(
                SendMessageW(
                    start,
                    DTM_GETSYSTEMTIME,
                    0,
                    (&mut native as *mut SYSTEMTIME) as LPARAM
                ),
                GDT_NONE as LRESULT
            );
            assert_ne!(set(end, "2026-10-18"), 0);
            assert_eq!(text(start), ""); // independent nullable boundaries
            assert_eq!(text(end), "2026-10-18");
            assert_ne!(SendMessageW(end, WM_SETTEXT, 0, 0), 0);
            assert_eq!(text(end), "");
        }
    }

    #[test]
    fn theme_and_notifications_preserve_value_and_ignore_foreign_controls() {
        let parent = host();
        let picker = create(parent.0, 2302, "开始（含）").unwrap();
        let palette = Palette {
            background: 0x24201e,
            surface: 0x322c29,
            text: 0xf5f2ee,
            disabled: 0xb2aaa2,
            accent: 0xffc078,
            border: 0x574e47,
            accent_surface: 0x543e29,
        };
        // SAFETY: live owner-thread test controls and NMHDR buffers only.
        unsafe {
            assert_ne!(set(picker, "2024-02-29"), 0);
            let style = GetWindowLongPtrW(picker, GWL_STYLE);
            for kind in [
                Resolved::Dark,
                Resolved::Light,
                Resolved::Contrast,
                Resolved::Dark,
            ] {
                apply(picker, kind, palette, 192);
                assert_eq!(text(picker), "2024-02-29");
                assert_eq!(GetWindowLongPtrW(picker, GWL_STYLE), style);
                let dc = CreateCompatibleDC(std::ptr::null_mut());
                assert!(!dc.is_null());
                let bitmap = CreateCompatibleBitmap(dc, 240, 30);
                assert!(!bitmap.is_null());
                let old_bitmap = SelectObject(dc, bitmap as HGDIOBJ);
                let old_font = GetCurrentObject(dc, OBJ_FONT as u32);
                let old_pen = GetCurrentObject(dc, OBJ_PEN as u32);
                SetTextColor(dc, 0x123456);
                SetBkMode(dc, OPAQUE as i32);
                SendMessageW(picker, WM_PRINTCLIENT, dc as WPARAM, PRF_CLIENT as LPARAM);
                assert_eq!(GetTextColor(dc), 0x123456);
                assert_eq!(GetBkMode(dc), OPAQUE as i32);
                assert_eq!(GetCurrentObject(dc, OBJ_FONT as u32), old_font);
                assert_eq!(GetCurrentObject(dc, OBJ_PEN as u32), old_pen);
                SelectObject(dc, old_bitmap);
                assert_ne!(DeleteObject(bitmap as HGDIOBJ), 0);
                assert_ne!(DeleteDC(dc), 0);
                let header = NMHDR {
                    hwndFrom: picker,
                    idFrom: 2302,
                    code: DTN_DROPDOWN,
                };
                assert!(notify((&header as *const NMHDR) as LPARAM));
                let changed = NMHDR {
                    code: DTN_DATETIMECHANGE,
                    ..header
                };
                assert!(!notify((&changed as *const NMHDR) as LPARAM));
            }
            apply(parent.0, Resolved::Dark, palette, 96);
            assert!(GetPropW(parent.0, PROPERTY.as_ptr()).is_null());
            let foreign = NMHDR {
                hwndFrom: parent.0,
                idFrom: 0,
                code: DTN_DROPDOWN,
            };
            assert!(!notify((&foreign as *const NMHDR) as LPARAM));
            assert!(!notify(0));
        }
        assert!(create(parent.0, 1, "bad\0label").is_err());
        assert!(create(parent.0, 1, &"a".repeat(129)).is_err());
        assert!(create(std::ptr::null_mut(), 1, "date").is_err());
    }
}
