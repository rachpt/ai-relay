//! Public Win32 theme APIs only. Registry reads query a small Windows preference;
//! application configuration I/O remains on the controller worker.
use super::*;
use crate::config::ThemeMode;
use windows_sys::Win32::{
    Graphics::{Dwm::*, Gdi::*},
    System::Registry::*,
    UI::Controls::*,
};

// Public commctrl.h helpers (windows-sys places these behind an unused feature).
#[link(name = "user32")]
unsafe extern "system" {
    fn GetFocus() -> HWND;
}

type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

pub(super) const REFRESH: u32 = WM_APP + 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Resolved {
    Light,
    Dark,
    Contrast,
}
pub(super) fn resolve(mode: ThemeMode, light: bool, contrast: bool) -> Resolved {
    if contrast {
        Resolved::Contrast
    } else {
        match mode {
            ThemeMode::Light => Resolved::Light,
            ThemeMode::Dark => Resolved::Dark,
            ThemeMode::System => {
                if light {
                    Resolved::Light
                } else {
                    Resolved::Dark
                }
            }
        }
    }
}
#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub background: u32,
    pub surface: u32,
    pub text: u32,
    pub disabled: u32,
    pub accent: u32,
    pub border: u32,
    pub accent_surface: u32,
}
impl Palette {
    fn new(kind: Resolved) -> Self {
        // SAFETY: GetSysColor takes predefined indices, with no handle preconditions.
        unsafe {
            match kind {
                Resolved::Contrast => Self {
                    background: GetSysColor(COLOR_WINDOW),
                    surface: GetSysColor(COLOR_WINDOW),
                    text: GetSysColor(COLOR_WINDOWTEXT),
                    disabled: GetSysColor(COLOR_GRAYTEXT),
                    accent: GetSysColor(COLOR_HIGHLIGHT),
                    border: GetSysColor(COLOR_WINDOWTEXT),
                    accent_surface: GetSysColor(COLOR_HIGHLIGHT),
                },
                Resolved::Light => Self {
                    background: 0x00ffffff,
                    surface: 0x00ffffff,
                    text: 0x00262220,
                    disabled: 0x00756d66,
                    accent: 0x00d77800,
                    border: 0x00e3ded9,
                    accent_surface: 0x00fcf0e5,
                },
                Resolved::Dark => Self {
                    background: 0x0024201e,
                    surface: 0x00322c29,
                    text: 0x00f5f2ee,
                    disabled: 0x00b2aaa2,
                    accent: 0x00ffc078,
                    border: 0x00574e47,
                    accent_surface: 0x00543e29,
                },
            }
        }
    }
}
struct Brush(HBRUSH);
impl Brush {
    fn new(color: u32) -> Result<Self, String> {
        // SAFETY: CreateSolidBrush returns an owned GDI object.
        let handle = unsafe { CreateSolidBrush(color) };
        if handle.is_null() {
            Err(last_error("创建主题画刷"))
        } else {
            Ok(Self(handle))
        }
    }
}
impl Drop for Brush {
    fn drop(&mut self) {
        // SAFETY: Brushes are never selected into a DC; callers finish painting before replacement.
        unsafe {
            DeleteObject(self.0);
        }
    }
}
pub(super) struct Theme {
    pub mode: ThemeMode,
    pub kind: Resolved,
    pub palette: Palette,
    background: Brush,
    surface: Brush,
}
impl Theme {
    pub fn new(mode: ThemeMode) -> Result<Self, String> {
        let (light, contrast) = system();
        let kind = resolve(mode, light, contrast);
        let palette = Palette::new(kind);
        Ok(Self {
            mode,
            kind,
            palette,
            background: Brush::new(palette.background)?,
            surface: Brush::new(palette.surface)?,
        })
    }
}
// winuser.h ABI; Accessibility feature is not enabled, so keep the public SDK
// structure local rather than adding a Cargo feature/dependency.
#[repr(C)]
#[derive(Default)]
struct HighContrast {
    size: u32,
    flags: u32,
    scheme: *mut u16,
}

fn system() -> (bool, bool) {
    let mut value = 1u32;
    let mut bytes = size_of::<u32>() as u32;
    let mut contrast = HighContrast {
        size: size_of::<HighContrast>() as u32,
        ..Default::default()
    };
    // SAFETY: Predefined HKCU and terminated names; output is a sized DWORD. No registry writes.
    unsafe {
        let result = RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("AppsUseLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut bytes,
        );
        if result != 0 {
            value = 1;
        }
        let ok = SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.size,
            (&mut contrast as *mut HighContrast).cast(),
            0,
        );
        (value != 0, ok != 0 && contrast.flags & 1 != 0)
    }
}

/// No State reference crosses APIs which can synchronously reenter the parent.
pub(super) unsafe fn refresh(hwnd: HWND, state: *mut State) {
    // SAFETY: Owner thread and stable State pointer; replacement occurs outside paint callbacks.
    unsafe {
        let mode = (*state).theme.mode;
        let Ok(theme) = Theme::new(mode) else { return };
        let kind = theme.kind;
        (*state).theme = theme;
        let dpi = (*state).dpi;
        let palette = (*state).theme.palette;
        // Optional icon refresh failure leaves the prior valid caption icon.
        let _ = (*state).title_menu.refresh(hwnd, dpi, kind, palette);
        let tab = (*state).tabs.handle;
        SetWindowSubclass(tab, Some(tab_proc), 1, state as usize);
        // Native Tab still owns hit testing, selection, keyboard and accessibility.
        let style = GetWindowLongPtrW(tab, GWL_STYLE) as u32;
        SetWindowLongPtrW(tab, GWL_STYLE, (style & !TCS_OWNERDRAWFIXED) as isize);
        let dark = i32::from(kind == Resolved::Dark);
        // Unsupported attributes return HRESULT errors; retain native non-client rendering.
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&dark as *const i32).cast(),
            size_of::<i32>() as u32,
        );
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&corner as *const i32).cast(),
            size_of::<i32>() as u32,
        );
        let mut pending = Vec::new();
        let mut first = GetWindow(hwnd, GW_CHILD);
        while !first.is_null() {
            pending.push(first);
            first = GetWindow(first, GW_HWNDNEXT);
        }
        while let Some(child) = pending.pop() {
            // Visit page-host children too; native subcontrols are themed by their
            // owning control to avoid interpreting unrelated child styles.
            if child == (*state).viewport.handle {
                let mut nested = GetWindow(child, GW_CHILD);
                while !nested.is_null() {
                    pending.push(nested);
                    nested = GetWindow(nested, GW_HWNDNEXT);
                }
            }
            // Keep Common Controls v6 in light/high-contrast mode. Disabling it
            // globally made native buttons, combo arrows and tabs look classic.
            // Dark mode uses only documented color/owner-draw fallbacks.
            let empty = wide("");
            let _ = SetWindowTheme(
                child,
                if kind == Resolved::Dark {
                    empty.as_ptr()
                } else {
                    null()
                },
                if kind == Resolved::Dark {
                    empty.as_ptr()
                } else {
                    null()
                },
            );
            let mut class = [0u16; 32];
            let count = GetClassNameW(child, class.as_mut_ptr(), class.len() as i32);
            let style = GetWindowLongPtrW(child, GWL_STYLE) as u32;
            let class = String::from_utf16_lossy(&class[..count.max(0) as usize]);
            if class == "Edit" {
                let margin = scale(6, (*state).dpi);
                SendMessageW(child, 0x00d3, 3, (margin | (margin << 16)) as isize);
            }
            let button_kind = style & BS_TYPEMASK as u32;
            if class == "Button"
                && [
                    BS_PUSHBUTTON as u32,
                    BS_DEFPUSHBUTTON as u32,
                    BS_OWNERDRAW as u32,
                ]
                .contains(&button_kind)
            {
                SetWindowLongPtrW(
                    child,
                    GWL_STYLE,
                    ((style & !(BS_TYPEMASK as u32))
                        | if kind == Resolved::Contrast {
                            BS_PUSHBUTTON as u32
                        } else {
                            BS_OWNERDRAW as u32
                        }) as isize,
                );
            }
            control_theme::apply(child, kind, (*state).theme.palette, (*state).dpi);
            choices::apply(child, kind, (*state).theme.palette, (*state).dpi);
            date_picker::apply(child, kind, (*state).theme.palette, (*state).dpi);
            rounded::apply(child, kind, (*state).theme.palette, (*state).dpi);
            scroll_theme::apply(child, kind, (*state).theme.palette, (*state).dpi);
            InvalidateRect(child, null(), 1);
        }
        RedrawWindow(
            hwnd,
            null(),
            null_mut(),
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_FRAME,
        );
    }
}

pub(super) unsafe fn paint(
    state: *mut State,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> Option<LRESULT> {
    // SAFETY: Called only for this window's messages, before any mutable State borrow.
    unsafe {
        let palette = (*state).theme.palette;
        match message {
            WM_ERASEBKGND => {
                let mut rect = RECT::default();
                GetClientRect(GetParent((*state).tabs.handle), &mut rect);
                FillRect(w as HDC, &rect, (*state).theme.background.0);
                Some(1)
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
                let dc = w as HDC;
                let edit = message == WM_CTLCOLOREDIT
                    || message == WM_CTLCOLORLISTBOX
                    || GetWindowLongPtrW(l as HWND, GWL_STYLE) as u32 & ES_READONLY as u32 != 0;
                SetTextColor(
                    dc,
                    if GetWindowLongPtrW(l as HWND, GWL_STYLE) as u32 & WS_DISABLED == 0 {
                        palette.text
                    } else {
                        palette.disabled
                    },
                );
                SetBkColor(
                    dc,
                    if edit {
                        palette.surface
                    } else {
                        palette.background
                    },
                );
                Some(if edit {
                    (*state).theme.surface.0
                } else {
                    (*state).theme.background.0
                } as isize)
            }
            WM_DRAWITEM => {
                let item = &*(l as *const DRAWITEMSTRUCT);
                if (*state).theme.kind == Resolved::Contrast {
                    return None;
                }
                if item.CtlType == ODT_BUTTON {
                    let active = item.itemState & (ODS_FOCUS | ODS_SELECTED) != 0;
                    let primary = [2001, tabs::START, 2320, 2219].contains(&item.CtlID)
                        && item.itemState & ODS_DISABLED == 0;
                    let danger = [2210, 2211, 2350, 2352].contains(&item.CtlID)
                        && item.itemState & ODS_DISABLED == 0;
                    let mut palette = palette;
                    if danger {
                        palette.accent = if (*state).theme.kind == Resolved::Dark {
                            0x009b9bff
                        } else {
                            0x003c36b4
                        };
                        palette.accent_surface = if (*state).theme.kind == Resolved::Dark {
                            0x00332a40
                        } else {
                            0x00f5f3ff
                        };
                    }
                    rounded::surface(
                        item.hDC,
                        &item.rcItem,
                        (*state).dpi,
                        palette.background,
                        if primary || danger || active {
                            palette.accent_surface
                        } else {
                            palette.surface
                        },
                        if active || primary || danger {
                            palette.accent
                        } else {
                            palette.border
                        },
                    );
                    SetBkMode(item.hDC, TRANSPARENT as i32);
                    SetTextColor(
                        item.hDC,
                        if item.itemState & ODS_DISABLED != 0 {
                            palette.disabled
                        } else if primary || danger {
                            palette.accent
                        } else {
                            palette.text
                        },
                    );
                    let title = wide(&settings::text(item.hwndItem));
                    let mut rect = item.rcItem;
                    if item.itemState & ODS_SELECTED != 0 {
                        OffsetRect(&mut rect, 1, 1);
                    }
                    let flags = DT_CENTER
                        | DT_VCENTER
                        | DT_SINGLELINE
                        | if item.itemState & ODS_NOACCEL != 0 {
                            DT_HIDEPREFIX
                        } else {
                            0
                        };
                    DrawTextW(item.hDC, title.as_ptr(), -1, &mut rect, flags);
                    if item.itemState & ODS_FOCUS != 0 && item.itemState & ODS_NOFOCUSRECT == 0 {
                        InflateRect(&mut rect, -4, -4);
                        DrawFocusRect(item.hDC, &rect);
                    }
                    return Some(1);
                }
                if item.CtlType != ODT_TAB || item.itemID > 3 {
                    return None;
                }
                let selected = item.itemState & ODS_SELECTED != 0;
                FillRect(
                    item.hDC,
                    &item.rcItem,
                    if selected {
                        (*state).theme.surface.0
                    } else {
                        (*state).theme.background.0
                    },
                );
                SetBkMode(item.hDC, TRANSPARENT as i32);
                SetTextColor(
                    item.hDC,
                    if selected {
                        palette.accent
                    } else {
                        palette.text
                    },
                );
                let title = wide(["概览", "统计分析", "实时日志", "设置"][item.itemID as usize]);
                let mut rect = item.rcItem;
                DrawTextW(
                    item.hDC,
                    title.as_ptr(),
                    -1,
                    &mut rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
                if item.itemState & ODS_FOCUS != 0 {
                    DrawFocusRect(item.hDC, &rect);
                }
                Some(1)
            }
            WM_NOTIFY if l != 0 => {
                if let Some(result) = control_theme::custom_draw(l) {
                    return Some(result);
                }
                let header = &*(l as *const NMHDR);
                if header.code != NM_CUSTOMDRAW || (*state).theme.kind != Resolved::Dark {
                    return None;
                }
                let draw = &*(l as *const NMCUSTOMDRAW);
                // Only push buttons; checkboxes retain native checked/keyboard semantics and glyphs.
                let style =
                    GetWindowLongPtrW(header.hwndFrom, GWL_STYLE) as u32 & BS_TYPEMASK as u32;
                if style != BS_PUSHBUTTON as u32 && style != BS_DEFPUSHBUTTON as u32 {
                    return None;
                }
                if draw.dwDrawStage != CDDS_PREPAINT {
                    return Some(CDRF_DODEFAULT as isize);
                }
                FillRect(draw.hdc, &draw.rc, (*state).theme.surface.0);
                let border = Brush::new(
                    if draw.uItemState & (CDIS_FOCUS | CDIS_HOT | CDIS_SELECTED) != 0 {
                        palette.accent
                    } else {
                        palette.disabled
                    },
                )
                .ok()?;
                FrameRect(draw.hdc, &draw.rc, border.0);
                SetBkMode(draw.hdc, TRANSPARENT as i32);
                SetTextColor(
                    draw.hdc,
                    if draw.uItemState & CDIS_DISABLED != 0 {
                        palette.disabled
                    } else {
                        palette.text
                    },
                );
                let title = wide(&settings::text(header.hwndFrom));
                let mut rect = draw.rc;
                DrawTextW(
                    draw.hdc,
                    title.as_ptr(),
                    -1,
                    &mut rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
                if draw.uItemState & CDIS_FOCUS != 0 {
                    InflateRect(&mut rect, -4, -4);
                    DrawFocusRect(draw.hdc, &rect);
                }
                Some(CDRF_SKIPDEFAULT as isize)
            }
            _ => None,
        }
    }
}

// Native Tab handles input, selection and accessibility; only its header is
// painted here, so no classic page border or raised button edges are exposed.
unsafe extern "system" fn tab_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: Subclass is attached only to our UI-thread Tab; State outlives its children.
    unsafe {
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(tab_proc), id);
        }
        let state = data as *mut State;
        if (message == WM_PAINT || message == WM_PRINTCLIENT)
            && (*state).theme.kind != Resolved::Contrast
        {
            let mut paint = PAINTSTRUCT::default();
            let dc = if message == WM_PAINT {
                BeginPaint(hwnd, &mut paint)
            } else {
                w as HDC
            };
            if !dc.is_null() {
                let saved = SaveDC(dc);
                let mut bounds = RECT::default();
                GetClientRect(hwnd, &mut bounds);
                FillRect(dc, &bounds, (*state).theme.background.0);
                let palette = (*state).theme.palette;
                let font = SelectObject(dc, (*state).font.0);
                SetBkMode(dc, TRANSPARENT as i32);
                let selected = SendMessageW(hwnd, TCM_GETCURSEL, 0, 0);
                for (index, title) in ["概览", "统计分析", "实时日志", "设置"].iter().enumerate()
                {
                    let mut rect = RECT::default();
                    if SendMessageW(
                        hwnd,
                        TCM_GETITEMRECT,
                        index,
                        (&mut rect as *mut RECT) as isize,
                    ) == 0
                    {
                        continue;
                    }
                    let active = selected == index as isize;
                    if active && let Ok(brush) = Brush::new(palette.accent_surface) {
                        FillRect(dc, &rect, brush.0);
                    }
                    SetTextColor(dc, if active { palette.accent } else { palette.text });
                    DrawTextW(
                        dc,
                        wide(title).as_ptr(),
                        -1,
                        &mut rect,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                    );
                    if active {
                        let mut line = rect;
                        line.top = line.bottom - scale(2, (*state).dpi);
                        if let Ok(brush) = Brush::new(palette.accent) {
                            FillRect(dc, &line, brush.0);
                        }
                    }
                    if active
                        && GetFocus() == hwnd
                        && SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) & UISF_HIDEFOCUS as isize == 0
                    {
                        InflateRect(&mut rect, -scale(4, (*state).dpi), -scale(4, (*state).dpi));
                        DrawFocusRect(dc, &rect);
                    }
                }
                SelectObject(dc, font);
                if saved != 0 {
                    RestoreDC(dc, saved);
                }
            }
            if message == WM_PAINT {
                EndPaint(hwnd, &paint);
            }
            return 0;
        }
        if message == WM_ERASEBKGND {
            let state = data as *mut State;
            if (*state).theme.kind != Resolved::Contrast {
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect);
                FillRect(w as HDC, &rect, (*state).theme.background.0);
                return 1;
            }
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_theme_rows_fit_minimum_client_at_common_dpi() {
        for dpi in [96, 144, 192] {
            // Minimum outer height 880; reserve a conservative 64 DIP non-client
            // area and the existing 84 DIP status region. Theme help ends at 590.
            assert!(scale(590, dpi) < scale(880 - 64 - 84, dpi));
            assert!(scale(486 - 48 + 32, dpi) < scale(546 - 48, dpi));
            assert_eq!(scale(32, dpi), [32, 48, 64][(dpi / 48 - 2) as usize]);
        }
    }

    #[test]
    fn resolves_all_modes_and_contrast_always_wins() {
        for light in [true, false] {
            for mode in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
                assert_eq!(resolve(mode, light, true), Resolved::Contrast);
            }
            assert_eq!(resolve(ThemeMode::Light, light, false), Resolved::Light);
            assert_eq!(resolve(ThemeMode::Dark, light, false), Resolved::Dark);
            assert_eq!(
                resolve(ThemeMode::System, light, false),
                if light {
                    Resolved::Light
                } else {
                    Resolved::Dark
                }
            );
        }
    }
}
