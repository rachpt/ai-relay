//! Presentation-only push-button states; native Button still owns input and commands.
use super::{
    rounded, settings, tabs,
    theme::{Palette, Resolved, blend},
};
use crate::windows::scale;
use std::{mem::size_of, ptr::null};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    UI::{Controls::*, WindowsAndMessaging::*},
};

const SUBCLASS_ID: usize = 0x4254_4e53;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
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
#[link(name = "user32")]
unsafe extern "system" {
    fn TrackMouseEvent(event: *mut MouseTracking) -> i32;
}

#[derive(Clone, Copy, Default)]
struct State {
    hover: bool,
    enabled: bool,
}

unsafe fn state(hwnd: HWND) -> Option<State> {
    // SAFETY: Only this owner-thread subclass owns this proc/id pair and Box.
    unsafe {
        let mut data = 0;
        (GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) != 0 && data != 0)
            .then(|| *(data as *const State))
    }
}

pub(super) unsafe fn apply(hwnd: HWND, kind: Resolved) {
    // SAFETY: Live owner-thread child, copied presentation data only. No reference
    // into the Box spans any native call that can synchronously destroy the HWND.
    unsafe {
        let mut data = 0;
        let existing = GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) != 0;
        let mut class = [0u16; 16];
        let count = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        let custom = kind != Resolved::Contrast
            && String::from_utf16_lossy(&class[..count.max(0) as usize])
                .eq_ignore_ascii_case("Button")
            && GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & BS_TYPEMASK as u32
                == BS_OWNERDRAW as u32;
        if !custom {
            if existing && data != 0 {
                *(data as *mut State) = State::default();
                if RemoveWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID) != 0 {
                    drop(Box::from_raw(data as *mut State));
                }
            }
        } else if existing && data != 0 {
            (*(data as *mut State)).enabled = true;
        } else {
            let data = Box::into_raw(Box::new(State {
                hover: false,
                enabled: true,
            }));
            if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, data as usize) == 0 {
                drop(Box::from_raw(data));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Colors {
    fill: u32,
    border: u32,
    ink: u32,
}
fn colors(id: u32, flags: u32, hot: bool, p: Palette, kind: Resolved) -> Colors {
    if flags & ODS_DISABLED != 0 {
        return Colors {
            fill: blend(p.surface, p.background, 60),
            border: blend(p.border, p.background, 55),
            ink: p.disabled,
        };
    }
    let pressed = flags & ODS_SELECTED != 0;
    if [2001, tabs::START, 2320, 2219].contains(&id) {
        let toward = if kind == Resolved::Dark { 0xffffff } else { 0 };
        let fill = if pressed {
            blend(p.accent, toward, 12)
        } else if hot {
            blend(p.accent, toward, 6)
        } else {
            p.accent
        };
        Colors {
            fill,
            border: fill,
            ink: p.on_accent(),
        }
    } else if [2210, 2211, 2350, 2352].contains(&id) {
        let danger = if kind == Resolved::Dark {
            0x00abb4ff
        } else {
            0x001e0eb1
        };
        let fill = blend(
            p.surface,
            danger,
            if pressed {
                12
            } else if hot {
                8
            } else {
                4
            },
        );
        Colors {
            fill,
            border: blend(fill, p.border, 85),
            ink: danger,
        }
    } else {
        Colors {
            fill: if pressed {
                blend(p.surface, p.text, 10)
            } else if hot {
                p.hover()
            } else {
                p.surface
            },
            border: if pressed {
                blend(p.border, p.disabled, 25)
            } else if hot {
                blend(p.border, p.disabled, 15)
            } else {
                p.border
            },
            ink: p.text,
        }
    }
}

fn text_rect(bounds: RECT, dpi: u32) -> RECT {
    let padding = scale(8, dpi).min((bounds.right - bounds.left).max(0) / 2);
    RECT {
        left: bounds.left + padding,
        right: bounds.right - padding,
        ..bounds
    }
}

struct SavedDc(HDC, i32);
impl SavedDc {
    unsafe fn new(dc: HDC) -> Option<Self> {
        // SAFETY: Borrowed live DC; no drawing is permitted if the save fails.
        let saved = unsafe { SaveDC(dc) };
        (saved != 0).then_some(Self(dc, saved))
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: Owns one successful SaveDC; the borrowed DC outlives this guard.
        unsafe { RestoreDC(self.0, self.1) };
    }
}

pub(super) unsafe fn draw(item: &DRAWITEMSTRUCT, dpi: u32, p: Palette, kind: Resolved) {
    // SAFETY: WM_DRAWITEM supplies a live borrowed DC/control. SaveDC restores
    // font, colors and clipping; no owned GDI object or application state is retained.
    unsafe {
        let Some(_saved) = SavedDc::new(item.hDC) else {
            return;
        };
        let hot = state(item.hwndItem).is_some_and(|s| s.enabled && s.hover);
        let c = colors(item.CtlID, item.itemState, hot, p, kind);
        rounded::surface(item.hDC, &item.rcItem, dpi, p.background, c.fill, c.border);
        let font = SendMessageW(item.hwndItem, WM_GETFONT, 0, 0) as HGDIOBJ;
        if !font.is_null() {
            SelectObject(item.hDC, font);
        }
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, c.ink);
        let title = crate::windows::wide(&settings::text(item.hwndItem));
        // Press feedback is a surface change, not a moving text baseline.
        let mut rect = text_rect(item.rcItem, dpi);
        if rect.right > rect.left {
            DrawTextW(
                item.hDC,
                title.as_ptr(),
                -1,
                &mut rect,
                DT_CENTER
                    | DT_VCENTER
                    | DT_SINGLELINE
                    | DT_END_ELLIPSIS
                    | if item.itemState & ODS_NOACCEL != 0 {
                        DT_HIDEPREFIX
                    } else {
                        0
                    },
            );
        }
        if item.itemState & (ODS_DISABLED | ODS_FOCUS | ODS_NOFOCUSRECT) == ODS_FOCUS {
            let mut focus = item.rcItem;
            InflateRect(&mut focus, -scale(4, dpi), -scale(4, dpi));
            if focus.right > focus.left && focus.bottom > focus.top {
                SetBkColor(item.hDC, c.fill);
                DrawFocusRect(item.hDC, &focus);
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
    // SAFETY: This owner-thread subclass owns one Box until NCDESTROY/removal.
    // Copy/read before forwarding; never touch the pointer after native dispatch.
    unsafe {
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(procedure), id);
            drop(Box::from_raw(data as *mut State));
            return DefSubclassProc(hwnd, message, w, l);
        }
        if matches!(message, WM_PRINT | WM_PRINTCLIENT) {
            // Native Button printing also changes font, colors and clipping
            // outside WM_DRAWITEM. Preserve the caller's DC around the complete
            // native pass, without altering its drawing/mapping. The guard owns
            // no subclass data and survives synchronous detach or destruction.
            let Some(_saved) = SavedDc::new(w as HDC) else {
                return 0;
            };
            return DefSubclassProc(hwnd, message, w, l);
        }
        if (*(data as *const State)).enabled {
            if message == WM_MOUSEMOVE
                && !(*(data as *const State)).hover
                && GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_DISABLED == 0
            {
                let mut tracking = MouseTracking {
                    size: size_of::<MouseTracking>() as u32,
                    flags: 2,
                    hwnd,
                    hover_time: 0,
                };
                if TrackMouseEvent(&mut tracking) != 0 {
                    (*(data as *mut State)).hover = true;
                    InvalidateRect(hwnd, null(), 0);
                }
            } else if matches!(
                message,
                WM_MOUSELEAVE | WM_ENABLE | WM_SHOWWINDOW | WM_CANCELMODE
            ) {
                (*(data as *mut State)).hover = false;
                InvalidateRect(hwnd, null(), 0);
            }
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, ptr::null_mut, sync::Once};
    use windows_sys::Win32::{
        Foundation::POINT,
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources},
        },
    };

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetFocus() -> HWND;
        fn SetFocus(hwnd: HWND) -> HWND;
        fn EnableWindow(hwnd: HWND, enabled: i32) -> i32;
    }

    fn palette(kind: Resolved) -> Palette {
        if kind == Resolved::Dark {
            return Palette {
                background: 0x202020,
                surface: 0x2b2b2b,
                text: 0xf5f5f5,
                disabled: 0xc5c5c5,
                accent: 0xffcd60,
                border: 0x454545,
                accent_surface: 0x473d2d,
            };
        }
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
    fn primary_neutral_danger_hover_and_disabled_have_distinct_roles() {
        let p = palette(Resolved::Light);
        let primary = colors(2320, 0, false, p, Resolved::Light);
        assert_eq!(primary.fill, p.accent);
        assert_eq!(primary.ink, p.on_accent());
        assert_ne!(primary.fill, colors(2320, 0, true, p, Resolved::Light).fill);
        assert_ne!(
            primary.fill,
            colors(2320, ODS_SELECTED, false, p, Resolved::Light).fill
        );
        let neutral = colors(2317, 0, false, p, Resolved::Light);
        assert_eq!(neutral.fill, p.surface);
        // Keyboard focus has its own native focus rectangle, not a second accent stroke.
        assert_eq!(colors(2317, ODS_FOCUS, false, p, Resolved::Light), neutral);
        assert_ne!(colors(2350, 0, false, p, Resolved::Light).ink, p.text);
        for id in [2320, 2317, 2350] {
            let disabled = colors(id, ODS_DISABLED | ODS_SELECTED, true, p, Resolved::Light);
            assert_eq!(
                disabled.fill,
                colors(2317, ODS_DISABLED, false, p, Resolved::Light).fill
            );
            assert_ne!(disabled.ink, p.on_accent());
        }
    }

    fn luminance(color: u32) -> f64 {
        [0.2126, 0.7152, 0.0722]
            .into_iter()
            .enumerate()
            .map(|(index, weight)| {
                let channel = f64::from((color >> (index * 8)) & 255) / 255.0;
                weight
                    * if channel <= 0.04045 {
                        channel / 12.92
                    } else {
                        ((channel + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum()
    }

    fn assert_primary_contrast(p: Palette, kind: Resolved) {
        for id in [2001, tabs::START, 2320, 2219] {
            for (flags, hot, amount) in [
                (0, false, 0),
                (0, true, 6),
                (ODS_SELECTED, false, 12),
                (ODS_SELECTED | ODS_FOCUS, true, 12),
            ] {
                let c = colors(id, flags, hot, p, kind);
                let toward = if kind == Resolved::Dark { 0xffffff } else { 0 };
                assert_eq!(c.fill, blend(p.accent, toward, amount));
                assert_eq!(c.border, c.fill);
                assert_eq!(c.ink, p.on_accent());
                let ink = luminance(c.ink);
                let fill = luminance(c.fill);
                let ratio = (ink.max(fill) + 0.05) / (ink.min(fill) + 0.05);
                assert!(
                    ratio >= 4.5,
                    "primary {kind:?}, flags={flags}, hot={hot}: {ratio}"
                );
            }
        }
    }

    #[test]
    fn primary_interaction_colors_preserve_wcag_contrast_in_both_app_themes() {
        for kind in [Resolved::Light, Resolved::Dark] {
            assert_primary_contrast(palette(kind), kind);
        }
        // Also exercise the actual application palette, not just fixed fixtures.
        // Real system high contrast renders native buttons, never these fills.
        for mode in [
            crate::config::ThemeMode::Light,
            crate::config::ThemeMode::Dark,
        ] {
            let theme = crate::ui::theme::Theme::new(mode).unwrap();
            if theme.kind == Resolved::Contrast {
                eprintln!("system high contrast: app primary fills are inactive; fixtures checked");
            } else {
                assert_primary_contrast(theme.palette, theme.kind);
            }
        }
    }

    fn assert_action_contrast(p: Palette, kind: Resolved) {
        for id in [2320, 2317, 2350] {
            for flags in [
                0,
                ODS_SELECTED,
                ODS_FOCUS,
                ODS_SELECTED | ODS_FOCUS,
                ODS_DISABLED,
            ] {
                for hot in [false, true] {
                    let c = colors(id, flags, hot, p, kind);
                    let ink = luminance(c.ink);
                    let fill = luminance(c.fill);
                    let ratio = (ink.max(fill) + 0.05) / (ink.min(fill) + 0.05);
                    assert!(
                        ratio >= 4.5,
                        "{kind:?} action {id}, flags={flags}, hot={hot}: {ratio}"
                    );
                    if flags & ODS_DISABLED != 0 {
                        assert_eq!(
                            c,
                            colors(id, flags | ODS_SELECTED | ODS_FOCUS, true, p, kind)
                        );
                        assert_ne!(c.ink, p.text);
                    }
                }
            }
        }
    }

    #[test]
    fn all_action_text_states_preserve_contrast_in_fixtures_and_real_palette() {
        for kind in [Resolved::Light, Resolved::Dark] {
            assert_action_contrast(palette(kind), kind);
        }
        for mode in [
            crate::config::ThemeMode::Light,
            crate::config::ThemeMode::Dark,
        ] {
            let theme = crate::ui::theme::Theme::new(mode).unwrap();
            if theme.kind != Resolved::Contrast {
                assert_action_contrast(theme.palette, theme.kind);
            }
        }
    }

    #[test]
    fn neutral_actions_stay_achromatic_and_geometry_keeps_text_centered() {
        for kind in [Resolved::Light, Resolved::Dark] {
            let p = palette(kind);
            for (flags, hot) in [
                (0, false),
                (0, true),
                (ODS_SELECTED, true),
                (ODS_FOCUS, true),
                (ODS_DISABLED, true),
            ] {
                let c = colors(2317, flags, hot, p, kind);
                for color in [c.fill, c.border, c.ink] {
                    assert_eq!(color & 255, color >> 8 & 255);
                    assert_eq!(color & 255, color >> 16 & 255);
                }
            }
        }
        for dpi in [96, 144, 192] {
            for height in [28, 30, 36] {
                for width in [0, 1, 8, 180] {
                    let bounds = RECT {
                        left: 5,
                        top: 7,
                        right: 5 + scale(width, dpi),
                        bottom: 7 + scale(height, dpi),
                    };
                    let text = text_rect(bounds, dpi);
                    assert_eq!((text.top, text.bottom), (bounds.top, bounds.bottom));
                    assert!(
                        text.left >= bounds.left
                            && text.right <= bounds.right
                            && text.left <= text.right
                    );
                    assert_eq!(text.left - bounds.left, bounds.right - text.right);
                }
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct DcState {
        ink: u32,
        background: u32,
        brush_color: u32,
        pen_color: u32,
        background_mode: i32,
        font: HGDIOBJ,
        brush: HGDIOBJ,
        pen: HGDIOBJ,
        map_mode: i32,
        viewport: (i32, i32),
        window: (i32, i32),
        clip: (i32, i32, i32, i32),
        clip_kind: i32,
    }
    impl DcState {
        unsafe fn capture(dc: HDC) -> Self {
            // SAFETY: Test-owned memory DC or the live WM_DRAWITEM borrowed DC.
            unsafe {
                let mut viewport = POINT::default();
                let mut window = POINT::default();
                let mut clip = RECT::default();
                GetViewportOrgEx(dc, &mut viewport);
                GetWindowOrgEx(dc, &mut window);
                let clip_kind = GetClipBox(dc, &mut clip);
                Self {
                    ink: GetTextColor(dc),
                    background: GetBkColor(dc),
                    brush_color: GetDCBrushColor(dc),
                    pen_color: GetDCPenColor(dc),
                    background_mode: GetBkMode(dc),
                    font: GetCurrentObject(dc, OBJ_FONT as u32),
                    brush: GetCurrentObject(dc, OBJ_BRUSH as u32),
                    pen: GetCurrentObject(dc, OBJ_PEN as u32),
                    map_mode: GetMapMode(dc),
                    viewport: (viewport.x, viewport.y),
                    window: (window.x, window.y),
                    clip: (clip.left, clip.top, clip.right, clip.bottom),
                    clip_kind,
                }
            }
        }
    }

    #[derive(Clone, Copy)]
    struct Probe {
        kind: Resolved,
        palette: Palette,
        dpi: u32,
        draws: usize,
        flags: u32,
        dc_failures: usize,
        clicks: [usize; 2],
        click_action: u32,
    }

    unsafe extern "system" fn host_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        // SAFETY: Each hidden host retains its boxed Cell until DestroyWindow
        // returns. Copy before draw/native dispatch; never borrow it across FFI.
        // Assertions stay outside this extern callback so failures can unwind.
        unsafe {
            if message == WM_NCCREATE {
                let create = *(l as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            let probe = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Cell<Probe>;
            if !probe.is_null() {
                if message == WM_DRAWITEM && l != 0 {
                    let item = *(l as *const DRAWITEMSTRUCT);
                    if item.CtlType == ODT_BUTTON {
                        let value = (*probe).get();
                        let before = DcState::capture(item.hDC);
                        if value.kind != Resolved::Contrast {
                            draw(&item, value.dpi, value.palette, value.kind);
                        }
                        let changed = before != DcState::capture(item.hDC);
                        let mut value = (*probe).get();
                        value.draws += 1;
                        value.flags = item.itemState;
                        value.dc_failures += usize::from(changed);
                        (*probe).set(value);
                        return 1;
                    }
                }
                if message == WM_COMMAND && l != 0 && w >> 16 == BN_CLICKED as usize {
                    let index = match (w & 0xffff) as u32 {
                        2320 => Some(0),
                        2317 => Some(1),
                        _ => None,
                    };
                    if let Some(index) = index {
                        let mut value = (*probe).get();
                        value.clicks[index] += 1;
                        let action = if index == 0 { value.click_action } else { 0 };
                        value.click_action = 0;
                        (*probe).set(value);
                        match action {
                            1 => apply(l as HWND, Resolved::Contrast),
                            2 => {
                                DestroyWindow(l as HWND);
                            }
                            _ => {}
                        }
                        return 0;
                    }
                }
            }
            if message == WM_NCDESTROY {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            DefWindowProcW(hwnd, message, w, l)
        }
    }

    struct Host {
        hwnd: HWND,
        button: HWND,
        native: HWND,
        probe: Box<Cell<Probe>>,
    }
    impl Host {
        fn new() -> Self {
            static REGISTER: Once = Once::new();
            // SAFETY: Process-lifetime test class; all HWNDs stay hidden on this
            // test thread. No app State, controller, files, timers or message pump.
            unsafe {
                let instance = GetModuleHandleW(null());
                let class = windows_sys::w!("AIRelay.Buttons.Tests");
                REGISTER.call_once(|| {
                    let class = WNDCLASSW {
                        lpfnWndProc: Some(host_proc),
                        hInstance: instance,
                        lpszClassName: class,
                        ..Default::default()
                    };
                    assert_ne!(RegisterClassW(&class), 0);
                });
                let mut host = Self {
                    hwnd: null_mut(),
                    button: null_mut(),
                    native: null_mut(),
                    probe: Box::new(Cell::new(Probe {
                        kind: Resolved::Light,
                        palette: palette(Resolved::Light),
                        dpi: 96,
                        draws: 0,
                        flags: 0,
                        dc_failures: 0,
                        clicks: [0; 2],
                        click_action: 0,
                    })),
                };
                host.hwnd = CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    class,
                    null(),
                    WS_POPUP,
                    -32000,
                    -32000,
                    640,
                    160,
                    null_mut(),
                    null_mut(),
                    instance,
                    (&*host.probe as *const Cell<Probe>).cast(),
                );
                assert!(!host.hwnd.is_null());
                for (id, target) in [(2320, &mut host.button), (2317, &mut host.native)] {
                    *target = CreateWindowExW(
                        0,
                        windows_sys::w!("BUTTON"),
                        windows_sys::w!("执行 &操作 😀"),
                        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
                        0,
                        0,
                        180,
                        36,
                        host.hwnd,
                        id as HMENU,
                        instance,
                        null(),
                    );
                    assert!(!(*target).is_null());
                    SendMessageW(
                        *target,
                        WM_SETFONT,
                        GetStockObject(DEFAULT_GUI_FONT) as WPARAM,
                        0,
                    );
                }
                host.theme(Resolved::Light, 96);
                host
            }
        }

        fn theme(&self, kind: Resolved, dpi: u32) {
            // SAFETY: Live same-thread test Button. Match theme::refresh's native
            // theme/style selection; buttons::apply must NOT own style changes.
            unsafe {
                let mut value = self.probe.get();
                value.kind = kind;
                value.palette = palette(kind);
                value.dpi = dpi;
                self.probe.set(value);
                let empty = [0u16];
                let theme = if kind == Resolved::Dark {
                    empty.as_ptr()
                } else {
                    null()
                };
                SetWindowTheme(self.button, theme, theme);
                let style = GetWindowLongPtrW(self.button, GWL_STYLE) as u32;
                let style = (style & !(BS_TYPEMASK as u32))
                    | if kind == Resolved::Contrast {
                        BS_PUSHBUTTON as u32
                    } else {
                        BS_OWNERDRAW as u32
                    };
                SetWindowLongPtrW(self.button, GWL_STYLE, style as isize);
                apply(self.button, kind);
                assert_eq!(GetWindowLongPtrW(self.button, GWL_STYLE) as u32, style);
            }
        }

        fn print(&self, dc: HDC, message: u32) {
            // SAFETY: Live test Button and memory DC, no CHECKVISIBLE flag. The
            // custom subclass must preserve the entire native pass on its own.
            // Detached native fallback has no such contract: its caller protects
            // the DC explicitly. The independent draw callback audit stays strict.
            unsafe {
                let value = self.probe.get();
                let draws = value.draws;
                let before = DcState::capture(dc);
                let fallback_dc = if value.kind == Resolved::Contrast {
                    Some(SavedDc::new(dc).unwrap())
                } else {
                    None
                };
                SendMessageW(self.button, message, dc as WPARAM, PRF_CLIENT as LPARAM);
                drop(fallback_dc);
                assert_eq!(
                    DcState::capture(dc),
                    before,
                    "button printing did not restore caller DC"
                );
                let value = self.probe.get();
                assert_eq!(
                    value.dc_failures, 0,
                    "buttons::draw changed its borrowed DC"
                );
                if value.kind == Resolved::Contrast {
                    assert_eq!(
                        value.draws, draws,
                        "native fallback still requested owner drawing"
                    );
                } else {
                    assert!(
                        value.draws > draws,
                        "native print never reached parent WM_DRAWITEM"
                    );
                }
            }
        }
    }
    impl Drop for Host {
        fn drop(&mut self) {
            // SAFETY: Destroy children before the boxed callback data is dropped.
            unsafe { DestroyWindow(self.hwnd) };
        }
    }

    struct BitmapDc {
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
    }
    impl BitmapDc {
        fn new() -> Self {
            // SAFETY: Private memory DC/bitmap, no screen capture or owned font.
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                assert!(!dc.is_null());
                let bitmap = CreateBitmap(768, 192, 1, 32, null());
                assert!(!bitmap.is_null());
                let old = SelectObject(dc, bitmap);
                SelectObject(dc, GetStockObject(SYSTEM_FIXED_FONT));
                SelectObject(dc, GetStockObject(GRAY_BRUSH));
                SelectObject(dc, GetStockObject(BLACK_PEN));
                SetTextColor(dc, 0x123456);
                SetBkColor(dc, 0x234567);
                SetBkMode(dc, OPAQUE as i32);
                SetDCBrushColor(dc, 0x345678);
                SetDCPenColor(dc, 0x456789);
                SetViewportOrgEx(dc, 7, 9, null_mut());
                IntersectClipRect(dc, 0, 0, 640, 160);
                Self { dc, bitmap, old }
            }
        }

        fn assert_fill_pixel(&self, expected: u32) {
            // SAFETY: The sample avoids rounded edges, text and focus frames.
            // GetPixel accepts logical coordinates: (90, 2) maps to bitmap pixel
            // (97, 11) with this fixture's viewport, NOT device pixel (90, 2).
            unsafe {
                let mut pixel = POINT { x: 90, y: 2 };
                assert_ne!(LPtoDP(self.dc, &mut pixel, 1), 0);
                assert_eq!((pixel.x, pixel.y), (97, 11));
                assert_eq!(GetPixel(self.dc, 90, 2), expected);
                let _saved = SavedDc::new(self.dc).unwrap();
                SetViewportOrgEx(self.dc, 0, 0, null_mut());
                SetWindowOrgEx(self.dc, 0, 0, null_mut());
                assert_eq!(GetPixel(self.dc, pixel.x, pixel.y), expected);
            }
        }
    }
    impl Drop for BitmapDc {
        fn drop(&mut self) {
            // SAFETY: Deselect the owned bitmap before deleting it and its DC.
            unsafe {
                SelectObject(self.dc, self.old);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }

    unsafe fn assert_native_values(hwnd: HWND, caption: &[u16], font: HGDIOBJ) {
        // SAFETY: Hidden test Button; full WM_GETTEXT proves ellipsis did not
        // truncate its native caption (including UTF-16 surrogate pairs).
        unsafe {
            assert_eq!(SendMessageW(hwnd, WM_GETFONT, 0, 0), font as LRESULT);
            assert_eq!(
                SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0) as usize,
                caption.len() - 1
            );
            let mut actual = vec![0u16; caption.len()];
            assert_eq!(
                SendMessageW(
                    hwnd,
                    WM_GETTEXT,
                    actual.len(),
                    actual.as_mut_ptr() as LPARAM
                ) as usize,
                caption.len() - 1
            );
            assert_eq!(actual, caption);
        }
    }

    #[test]
    fn native_owner_draw_keeps_click_space_focus_text_font_and_disabled_semantics() {
        let host = Host::new();
        let bitmap = BitmapDc::new();
        let caption = crate::windows::wide(&format!("执行 &操作 {} 完整结尾", "中文😀".repeat(80)));
        // SAFETY: Only private, never-shown same-thread HWNDs and memory DCs. Space
        // is sent as native messages, not claimed as physical keyboard acceptance.
        unsafe {
            SetWindowTextW(host.button, caption.as_ptr());
            for kind in [Resolved::Light, Resolved::Dark] {
                let font = GetStockObject(if kind == Resolved::Light {
                    DEFAULT_GUI_FONT
                } else {
                    SYSTEM_FIXED_FONT
                });
                SendMessageW(host.button, WM_SETFONT, font as WPARAM, 0);
                host.theme(kind, 96);
                assert_native_values(host.button, &caption, font);
                let clicks = host.probe.get().clicks;
                SendMessageW(host.native, BM_CLICK, 0, 0);
                SendMessageW(host.button, BM_CLICK, 0, 0);
                assert_eq!(host.probe.get().clicks, [clicks[0] + 1, clicks[1] + 1]);
                for (index, hwnd) in [(1, host.native), (0, host.button)] {
                    SetFocus(hwnd);
                    assert_eq!(GetFocus(), hwnd);
                    let clicks = host.probe.get().clicks[index];
                    SendMessageW(hwnd, WM_KEYDOWN, 0x20, 1);
                    assert_ne!(SendMessageW(hwnd, BM_GETSTATE, 0, 0) as u32 & BST_PUSHED, 0);
                    SendMessageW(hwnd, WM_KEYUP, 0x20, 0xc000_0001u32 as LPARAM);
                    assert_eq!(host.probe.get().clicks[index], clicks + 1);
                }
                host.print(bitmap.dc, WM_PRINT);
                assert_ne!(host.probe.get().flags & ODS_FOCUS, 0);
                assert_eq!(GetFocus(), host.button);
                SetFocus(host.native);
                host.print(bitmap.dc, WM_PRINTCLIENT);
                assert_eq!(host.probe.get().flags & ODS_FOCUS, 0);
                let p = palette(kind);
                bitmap.assert_fill_pixel(p.accent);
                SendMessageW(host.button, BM_SETSTATE, 1, 0);
                host.print(bitmap.dc, WM_PRINT);
                assert_ne!(host.probe.get().flags & ODS_SELECTED, 0);
                bitmap.assert_fill_pixel(colors(2320, ODS_SELECTED, false, p, kind).fill);
                SendMessageW(host.button, BM_SETSTATE, 0, 0);
                EnableWindow(host.button, 0);
                EnableWindow(host.native, 0);
                let clicks = host.probe.get().clicks;
                SendMessageW(host.native, BM_CLICK, 0, 0);
                SendMessageW(host.button, BM_CLICK, 0, 0);
                let after = host.probe.get().clicks;
                assert_eq!(after[0] - clicks[0], after[1] - clicks[1]);
                host.print(bitmap.dc, WM_PRINT);
                assert_ne!(host.probe.get().flags & ODS_DISABLED, 0);
                bitmap.assert_fill_pixel(colors(2320, ODS_DISABLED, false, p, kind).fill);
                assert_native_values(host.button, &caption, font);
                EnableWindow(host.native, 1);
                EnableWindow(host.button, 1);
            }
            let font = GetStockObject(SYSTEM_FIXED_FONT);
            SetFocus(host.button);
            assert_eq!(GetFocus(), host.button);
            SendMessageW(
                host.button,
                WM_UPDATEUISTATE,
                UIS_CLEAR as WPARAM | ((UISF_HIDEFOCUS | UISF_HIDEACCEL) as WPARAM) << 16,
                0,
            );
            for dpi in [96, 144, 192] {
                for kind in [Resolved::Light, Resolved::Dark] {
                    host.theme(kind, dpi);
                    for (width, height) in [(1, 1), (8, 5), (16, 12), (180, 36)] {
                        MoveWindow(host.button, 0, 0, width, height, 0);
                        host.print(bitmap.dc, WM_PRINTCLIENT);
                        assert_ne!(host.probe.get().flags & ODS_FOCUS, 0);
                        assert_native_values(host.button, &caption, font);
                    }
                }
            }
        }
    }

    #[test]
    fn native_pressed_caption_keeps_the_same_ink_pixels_at_fixed_heights() {
        let host = Host::new();
        let bitmap = BitmapDc::new();
        let caption = crate::windows::wide("Stable &action");
        // SAFETY: Hidden test-owned native Button and memory bitmap. Exact ink
        // pixels use a stock raster font; no desktop input or screenshot is used.
        unsafe {
            SetWindowTextW(host.button, caption.as_ptr());
            let font = GetStockObject(SYSTEM_FIXED_FONT);
            SendMessageW(host.button, WM_SETFONT, font as WPARAM, 0);
            SetFocus(host.native);
            for dpi in [96, 144, 192] {
                for kind in [Resolved::Light, Resolved::Dark] {
                    host.theme(kind, dpi);
                    for height in [28, 30, 36] {
                        let bounds = RECT {
                            right: scale(180, dpi),
                            bottom: scale(height, dpi),
                            ..RECT::default()
                        };
                        MoveWindow(host.button, 0, 0, bounds.right, bounds.bottom, 0);
                        let rect = RECT {
                            top: bounds.top + scale(4, dpi),
                            bottom: bounds.bottom - scale(4, dpi),
                            ..text_rect(bounds, dpi)
                        };
                        let ink = palette(kind).on_accent();
                        let capture_ink = || {
                            let mut mask = Vec::new();
                            for y in rect.top..rect.bottom {
                                for x in rect.left..rect.right {
                                    mask.push(GetPixel(bitmap.dc, x, y) == ink);
                                }
                            }
                            mask
                        };
                        SendMessageW(host.button, BM_SETSTATE, 0, 0);
                        host.print(bitmap.dc, WM_PRINTCLIENT);
                        let normal = capture_ink();
                        assert!(normal.iter().any(|pixel| *pixel), "no caption ink");
                        assert!(
                            normal.iter().any(|pixel| !*pixel),
                            "solid/invalid caption image"
                        );
                        SendMessageW(host.button, BM_SETSTATE, 1, 0);
                        host.print(bitmap.dc, WM_PRINTCLIENT);
                        assert_ne!(host.probe.get().flags & ODS_SELECTED, 0);
                        assert_eq!(
                            normal,
                            capture_ink(),
                            "caption moved in {kind:?}, dpi={dpi}, height={height}"
                        );
                        assert_native_values(host.button, &caption, font);
                        SendMessageW(host.button, BM_SETSTATE, 0, 0);
                    }
                }
            }
        }
    }

    #[test]
    fn native_hover_contrast_detach_and_reentrant_command_cleanup() {
        let host = Host::new();
        let bitmap = BitmapDc::new();
        // SAFETY: Only local hidden HWND messages. The installed hover tracker is
        // exercised directly; no mouse input is injected into the user's desktop.
        unsafe {
            SendMessageW(host.button, WM_MOUSEMOVE, 0, 10 | (10 << 16));
            assert!(state(host.button).unwrap().hover);
            host.print(bitmap.dc, WM_PRINT);
            bitmap.assert_fill_pixel(
                colors(2320, 0, true, palette(Resolved::Light), Resolved::Light).fill,
            );
            host.theme(Resolved::Dark, 96);
            assert!(state(host.button).unwrap().hover);
            SendMessageW(host.button, WM_MOUSELEAVE, 0, 0);
            assert!(!state(host.button).unwrap().hover);
            SendMessageW(host.button, WM_MOUSEMOVE, 0, 10 | (10 << 16));
            EnableWindow(host.button, 0);
            assert!(!state(host.button).unwrap().hover);
            SendMessageW(host.button, WM_MOUSEMOVE, 0, 10 | (10 << 16));
            assert!(!state(host.button).unwrap().hover);
            EnableWindow(host.button, 1);
            SendMessageW(host.button, WM_MOUSEMOVE, 0, 10 | (10 << 16));
            assert!(state(host.button).unwrap().hover);
            SendMessageW(host.button, WM_CANCELMODE, 0, 0);
            assert!(!state(host.button).unwrap().hover);
            host.theme(Resolved::Contrast, 96);
            let mut data = 0;
            assert_eq!(
                GetWindowSubclass(host.button, Some(procedure), SUBCLASS_ID, &mut data),
                0
            );
            host.print(bitmap.dc, WM_PRINT);
            SendMessageW(host.button, WM_MOUSEMOVE, 0, 10 | (10 << 16));
            assert!(state(host.button).is_none());
            host.theme(Resolved::Light, 96);
            assert!(!state(host.button).unwrap().hover);
            for action in [1, 2] {
                let mut value = host.probe.get();
                value.click_action = action;
                host.probe.set(value);
                let clicks = value.clicks[0];
                SendMessageW(host.button, BM_CLICK, 0, 0);
                assert_eq!(host.probe.get().clicks[0], clicks + 1);
                assert!(state(host.button).is_none());
                if action == 1 {
                    assert_ne!(IsWindow(host.button), 0);
                    host.theme(Resolved::Light, 96);
                } else {
                    assert_eq!(IsWindow(host.button), 0);
                }
            }
        }
    }

    #[test]
    fn warmed_native_theme_and_print_cycles_keep_gdi_count_stable() {
        let host = Host::new();
        let bitmap = BitmapDc::new();
        let cycle = |index: usize| {
            let kind = [Resolved::Light, Resolved::Dark, Resolved::Contrast][index % 3];
            host.theme(kind, [96, 144, 192][index % 3]);
            host.print(bitmap.dc, WM_PRINT);
            host.print(bitmap.dc, WM_PRINTCLIENT);
        };
        // SAFETY: Resource counts query this process. Warm every tested native
        // theme/font path before measuring; intended for the serial native suite.
        unsafe {
            for index in 0..18 {
                cycle(index);
            }
            let process = GetCurrentProcess();
            let before = GetGuiResources(process, GR_GDIOBJECTS);
            assert_ne!(before, 0);
            for index in 0..180 {
                cycle(index);
            }
            assert_eq!(
                GetGuiResources(process, GR_GDIOBJECTS),
                before,
                "native button theme/print cycles leaked GDI objects"
            );
        }
    }
}
