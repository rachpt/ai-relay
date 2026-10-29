use super::*;
use std::rc::Weak;

std::thread_local! {
    pub(super) static FAIL_SAVE: Cell<bool> = const { Cell::new(false) };
}

#[link(name = "user32")]
unsafe extern "system" {
    fn EnableWindow(hwnd: HWND, enabled: i32) -> i32;
}

fn palette(kind: Resolved) -> Palette {
    match kind {
        Resolved::Dark => Palette {
            background: 0x202020,
            surface: 0x2b2b2b,
            text: 0xf5f5f5,
            disabled: 0xc5c5c5,
            accent: 0xffcd60,
            border: 0x454545,
            accent_surface: 0x473d2d,
        },
        _ => Palette {
            background: 0xf3f3f3,
            surface: 0xffffff,
            text: 0x1a1a1a,
            disabled: 0x616161,
            accent: 0xb85f00,
            border: 0xe0e0e0,
            accent_surface: 0xf9f2ea,
        },
    }
}

struct Host {
    hwnd: HWND,
    tabs: Tabs,
    notifications: Box<Cell<usize>>,
}
impl Host {
    fn new() -> Self {
        // SAFETY: Hidden owner-thread parent and native children. The boxed
        // notification counter stays stable until the entire HWND tree is gone.
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                null(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                1200,
                160,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!hwnd.is_null());
            let notifications = Box::new(Cell::new(0));
            assert_ne!(
                SetWindowSubclass(
                    hwnd,
                    Some(host_proc),
                    1,
                    (&*notifications as *const Cell<usize>) as usize,
                ),
                0
            );
            let mut tabs = Tabs::default();
            tabs.create(hwnd).unwrap();
            Self {
                hwnd,
                tabs,
                notifications,
            }
        }
    }

    fn layout(&self, dpi: u32) {
        self.tabs.layout(dpi, scale(600, dpi), 0);
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        // SAFETY: Destroy native children before releasing the counter and tabs.
        unsafe { DestroyWindow(self.hwnd) };
    }
}

unsafe extern "system" fn host_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: Only test-owned synchronous notifications with a live boxed Cell.
    // Never assert/panic inside native callbacks.
    unsafe {
        if message == WM_NOTIFY && l != 0 {
            let header = &*(l as *const NMHDR);
            if header.idFrom == ID as usize && header.code == TCN_SELCHANGE {
                let count = &*(data as *const Cell<usize>);
                count.set(count.get() + 1);
            }
        }
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(host_proc), id);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl Canvas {
    fn new() -> Self {
        // SAFETY: Owned memory DC/bitmap; no screen capture or global GUI state.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            assert!(!dc.is_null());
            let bitmap = CreateBitmap(1400, 160, 1, 32, null());
            assert!(!bitmap.is_null());
            let previous = SelectObject(dc, bitmap);
            assert!(!previous.is_null() && previous as isize != -1);
            Self {
                dc,
                bitmap,
                previous,
            }
        }
    }

    fn print(&self, hwnd: HWND) {
        // SAFETY: The test owns the native Tab and borrowed target memory DC.
        unsafe {
            SendMessageW(
                hwnd,
                WM_PRINTCLIENT,
                self.dc as WPARAM,
                PRF_CLIENT as LPARAM,
            );
        }
    }
}
impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: Deselect the owned bitmap before deleting either GDI object.
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

unsafe fn item_rect(hwnd: HWND, index: usize) -> RECT {
    // SAFETY: Live test Tab and sized stack output for the synchronous query.
    unsafe {
        let mut rect = RECT::default();
        assert_ne!(
            SendMessageW(
                hwnd,
                TCM_GETITEMRECT,
                index,
                (&mut rect as *mut RECT) as LPARAM
            ),
            0
        );
        rect
    }
}

fn tuple(rect: RECT) -> (i32, i32, i32, i32) {
    (rect.left, rect.top, rect.right, rect.bottom)
}

fn point(rect: RECT) -> LPARAM {
    let x = (rect.left + rect.right) / 2;
    let y = (rect.top + rect.bottom) / 2;
    (u32::from(x as u16) | (u32::from(y as u16) << 16)) as LPARAM
}

#[test]
fn header_artwork_geometry_stays_inside_native_items_at_common_dpi() {
    for dpi in [96, 120, 144, 168, 192, 288, 768] {
        let rect = RECT {
            left: scale(2, dpi),
            top: scale(2, dpi),
            right: scale(114, dpi),
            bottom: scale(38, dpi),
        };
        let surface = inset(rect, scale(3, dpi)).unwrap();
        let indicator = indicator_rect(surface, dpi).unwrap();
        let focus = inset(rect, scale(1, dpi)).unwrap();
        assert!(surface.left > rect.left && surface.top > rect.top);
        assert!(surface.right < rect.right && surface.bottom < rect.bottom);
        assert!(indicator.left > surface.left && indicator.right < surface.right);
        assert!(indicator.top > surface.top && indicator.bottom < surface.bottom);
        assert_eq!(indicator.right - indicator.left, scale(24, dpi));
        assert_eq!(indicator.bottom - indicator.top, scale(3, dpi));
        assert_eq!(
            indicator.left - surface.left,
            surface.right - indicator.right
        );
        assert!(focus.left < surface.left && focus.bottom > surface.bottom);
        assert!(inset(RECT::default(), scale(3, dpi)).is_none());
        assert!(indicator_rect(RECT::default(), dpi).is_none());
    }
}

#[test]
fn native_navigation_keeps_messages_text_geometry_focus_and_notifications() {
    let mut host = Host::new();
    let hwnd = host.tabs.handle;
    // SAFETY: Hidden native HWNDs and synchronous real mouse/key/TCM messages;
    // this does not inject hardware input or touch the user's foreground window.
    unsafe {
        let font = GetStockObject(DEFAULT_GUI_FONT);
        host.tabs.set_font(font);
        for dpi in [96, 144, 192] {
            host.layout(dpi);
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let dialog = SendMessageW(hwnd, WM_GETDLGCODE, 0, 0);
            for kind in [
                Resolved::Light,
                Resolved::Dark,
                Resolved::Contrast,
                Resolved::Light,
            ] {
                let original: Vec<_> = (0..LABELS.len())
                    .map(|i| tuple(item_rect(hwnd, i)))
                    .collect();
                apply(hwnd, kind, palette(kind), dpi);
                assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), style);
                assert_eq!(SendMessageW(hwnd, WM_GETDLGCODE, 0, 0), dialog);
                assert_eq!(SendMessageW(hwnd, WM_GETFONT, 0, 0), font as LRESULT);
                assert_eq!(
                    SendMessageW(hwnd, TCM_GETITEMCOUNT, 0, 0),
                    LABELS.len() as isize
                );
                for (index, label) in LABELS.iter().enumerate() {
                    let rect = item_rect(hwnd, index);
                    assert_eq!(tuple(rect), original[index]);
                    let mut hit = TCHITTESTINFO {
                        pt: POINT {
                            x: (rect.left + rect.right) / 2,
                            y: (rect.top + rect.bottom) / 2,
                        },
                        flags: 0,
                    };
                    assert_eq!(
                        SendMessageW(
                            hwnd,
                            TCM_HITTEST,
                            0,
                            (&mut hit as *mut TCHITTESTINFO) as LPARAM
                        ),
                        index as isize
                    );
                    let mut text = [0u16; 32];
                    let mut item = TCITEMW {
                        mask: TCIF_TEXT,
                        pszText: text.as_mut_ptr(),
                        cchTextMax: text.len() as i32,
                        ..Default::default()
                    };
                    assert_ne!(
                        SendMessageW(
                            hwnd,
                            TCM_GETITEMW,
                            index,
                            (&mut item as *mut TCITEMW) as LPARAM
                        ),
                        0
                    );
                    assert_eq!(
                        String::from_utf16_lossy(
                            &text[..text.iter().position(|c| *c == 0).unwrap()]
                        ),
                        *label
                    );
                }
                let before = host.notifications.get();
                host.tabs.select(0);
                assert_eq!(host.notifications.get(), before);
                assert_eq!(host.tabs.selection(), 0);
                let second = point(item_rect(hwnd, 1));
                SendMessageW(hwnd, WM_LBUTTONDOWN, 1, second);
                SendMessageW(hwnd, WM_LBUTTONUP, 0, second);
                assert_eq!(host.tabs.selection(), 1);
                assert_eq!(host.notifications.get(), before + 1);
                SendMessageW(hwnd, WM_KEYDOWN, 0x27, 0); // Native VK_RIGHT.
                assert_eq!(host.tabs.selection(), 2);
                SendMessageW(hwnd, WM_KEYDOWN, 0x25, 0); // Native VK_LEFT.
                assert_eq!(host.tabs.selection(), 1);
                assert_eq!(host.notifications.get(), before + 3);
                SendMessageW(hwnd, TCM_SETCURFOCUS, 3, 0);
                assert_eq!(SendMessageW(hwnd, TCM_GETCURFOCUS, 0, 0), 3);
                let selected = host.tabs.selection();
                let focus = SendMessageW(hwnd, TCM_GETCURFOCUS, 0, 0);
                let count = host.notifications.get();
                apply(hwnd, kind, palette(kind), dpi);
                assert_eq!(host.tabs.selection(), selected);
                assert_eq!(SendMessageW(hwnd, TCM_GETCURFOCUS, 0, 0), focus);
                assert_eq!(host.notifications.get(), count);
            }
        }
    }
}

#[test]
fn native_print_draws_neutral_selection_short_accent_and_passive_hover() {
    let host = Host::new();
    let canvas = Canvas::new();
    let hwnd = host.tabs.handle;
    // SAFETY: Native queries/drawing use test-owned HWNDs and private GDI objects.
    unsafe {
        for dpi in [96, 144, 192] {
            let font = Font::new(dpi).unwrap();
            host.tabs.set_font(font.0);
            host.layout(dpi);
            for kind in [Resolved::Light, Resolved::Dark] {
                let p = palette(kind);
                apply(hwnd, kind, p, dpi);
                SendMessageW(hwnd, TCM_SETCURSEL, 0, 0);
                let selected = item_rect(hwnd, 0);
                let second = item_rect(hwnd, 1);
                let surface = inset(selected, scale(3, dpi)).unwrap();
                let indicator = indicator_rect(surface, dpi).unwrap();
                let x = (surface.left + surface.right) / 2;
                let inactive = inset(second, scale(3, dpi)).unwrap();
                let hover_x = inactive.left + scale(2, dpi);
                let hover_y = (inactive.top + inactive.bottom) / 2;
                canvas.print(hwnd);
                assert_eq!(GetPixel(canvas.dc, 0, 0), p.background);
                assert_eq!(
                    GetPixel(canvas.dc, x, surface.top + scale(2, dpi)),
                    p.surface
                );
                assert_eq!(
                    GetPixel(canvas.dc, x, (indicator.top + indicator.bottom) / 2),
                    p.accent
                );
                assert_eq!(
                    GetPixel(
                        canvas.dc,
                        indicator.left - scale(2, dpi),
                        (indicator.top + indicator.bottom) / 2
                    ),
                    p.surface
                );
                assert_eq!(GetPixel(canvas.dc, hover_x, hover_y), p.background);
                let data = presentation(hwnd).unwrap();
                let focus = SendMessageW(hwnd, TCM_GETCURFOCUS, 0, 0);
                let count = host.notifications.get();
                SendMessageW(hwnd, WM_MOUSEMOVE, 0, point(second));
                assert_eq!(data.hovered.get(), Some(1));
                assert!(data.tracking_leave.get());
                canvas.print(hwnd);
                assert_eq!(GetPixel(canvas.dc, hover_x, hover_y), p.hover());
                assert_eq!(SendMessageW(hwnd, TCM_GETCURSEL, 0, 0), 0);
                assert_eq!(SendMessageW(hwnd, TCM_GETCURFOCUS, 0, 0), focus);
                assert_eq!(host.notifications.get(), count);
                assert_eq!(tuple(item_rect(hwnd, 0)), tuple(selected));
                assert_eq!(tuple(item_rect(hwnd, 1)), tuple(second));
                SendMessageW(hwnd, WM_MOUSEMOVE, 0, 0xffff_ffffu32 as LPARAM);
                assert_eq!(data.hovered.get(), None);
                SendMessageW(hwnd, WM_MOUSEMOVE, 0, point(second));
                SendMessageW(hwnd, WM_MOUSELEAVE, 0, 0);
                assert_eq!(data.hovered.get(), None);
                assert!(!data.tracking_leave.get());
                canvas.print(hwnd);
                assert_eq!(GetPixel(canvas.dc, hover_x, hover_y), p.background);
                SendMessageW(hwnd, WM_MOUSEMOVE, 0, point(second));
                EnableWindow(hwnd, 0);
                assert_eq!(data.hovered.get(), None);
                assert!(!data.tracking_leave.get());
                SendMessageW(hwnd, WM_MOUSEMOVE, 0, point(second));
                assert_eq!(data.hovered.get(), None);
                canvas.print(hwnd);
                assert_eq!(
                    GetPixel(canvas.dc, x, (indicator.top + indicator.bottom) / 2),
                    p.disabled
                );
                EnableWindow(hwnd, 1);
            }
            // The native Tab must not retain a borrowed font after its owner drops.
            host.tabs.set_font(GetStockObject(DEFAULT_GUI_FONT));
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct DcState {
    text: u32,
    background: u32,
    mode: i32,
    brush_color: u32,
    pen_color: u32,
    font: HGDIOBJ,
    brush: HGDIOBJ,
    pen: HGDIOBJ,
    map: i32,
    clip: (i32, i32, i32, i32),
    origin: (i32, i32),
}
unsafe fn dc_state(dc: HDC) -> DcState {
    // SAFETY: Private live memory DC and sized stack query outputs.
    unsafe {
        let mut clip = RECT::default();
        let mut origin = POINT::default();
        GetClipBox(dc, &mut clip);
        GetViewportOrgEx(dc, &mut origin);
        DcState {
            text: GetTextColor(dc),
            background: GetBkColor(dc),
            mode: GetBkMode(dc),
            brush_color: GetDCBrushColor(dc),
            pen_color: GetDCPenColor(dc),
            font: GetCurrentObject(dc, OBJ_FONT as u32),
            brush: GetCurrentObject(dc, OBJ_BRUSH as u32),
            pen: GetCurrentObject(dc, OBJ_PEN as u32),
            map: GetMapMode(dc),
            clip: tuple(clip),
            origin: (origin.x, origin.y),
        }
    }
}

struct FailSave;
impl FailSave {
    fn new() -> Self {
        FAIL_SAVE.with(|flag| flag.set(true));
        Self
    }
}
impl Drop for FailSave {
    fn drop(&mut self) {
        FAIL_SAVE.with(|flag| flag.set(false));
    }
}

#[test]
fn print_and_erase_restore_caller_dc_and_skip_drawing_if_save_fails() {
    let host = Host::new();
    let canvas = Canvas::new();
    let hwnd = host.tabs.handle;
    host.layout(96);
    // SAFETY: Nondefault state and clip belong solely to this test's memory DC.
    unsafe {
        apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
        SetTextColor(canvas.dc, 0x123456);
        SetBkColor(canvas.dc, 0x654321);
        SetBkMode(canvas.dc, OPAQUE as i32);
        SetDCBrushColor(canvas.dc, 0x234567);
        SetDCPenColor(canvas.dc, 0x345678);
        SelectObject(canvas.dc, GetStockObject(BLACK_BRUSH));
        SelectObject(canvas.dc, GetStockObject(WHITE_PEN));
        SetViewportOrgEx(canvas.dc, 3, 5, null_mut());
        IntersectClipRect(canvas.dc, 0, 0, 550, 40);
        let before = dc_state(canvas.dc);
        canvas.print(hwnd);
        assert_eq!(dc_state(canvas.dc), before);
        for flags in [
            PRF_CLIENT,
            PRF_CLIENT | PRF_ERASEBKGND,
            PRF_CLIENT | PRF_CHILDREN,
        ] {
            SendMessageW(hwnd, WM_PRINT, canvas.dc as WPARAM, flags as LPARAM);
            assert_eq!(dc_state(canvas.dc), before);
        }
        SendMessageW(hwnd, WM_ERASEBKGND, canvas.dc as WPARAM, 0);
        assert_eq!(dc_state(canvas.dc), before);
        let sentinel = 0x765432;
        SetPixel(canvas.dc, 1, 1, sentinel);
        for flags in [
            0,
            PRF_ERASEBKGND as LPARAM,
            (PRF_CLIENT | PRF_CHECKVISIBLE) as LPARAM,
        ] {
            SendMessageW(hwnd, WM_PRINTCLIENT, canvas.dc as WPARAM, flags);
            assert_eq!(dc_state(canvas.dc), before);
            assert_eq!(GetPixel(canvas.dc, 1, 1), sentinel);
        }
        for flags in [0, (PRF_CLIENT | PRF_CHECKVISIBLE) as LPARAM] {
            SendMessageW(hwnd, WM_PRINT, canvas.dc as WPARAM, flags);
            assert_eq!(GetPixel(canvas.dc, 1, 1), sentinel);
            assert_eq!(dc_state(canvas.dc), before);
        }
        let _failure = FailSave::new();
        SendMessageW(hwnd, WM_PRINT, canvas.dc as WPARAM, PRF_CLIENT as LPARAM);
        assert_eq!(GetPixel(canvas.dc, 1, 1), sentinel);
        assert_eq!(dc_state(canvas.dc), before);
        canvas.print(hwnd);
        assert_eq!(GetPixel(canvas.dc, 1, 1), sentinel);
        assert_eq!(dc_state(canvas.dc), before);
        assert_eq!(SendMessageW(hwnd, WM_ERASEBKGND, canvas.dc as WPARAM, 0), 0);
        assert_eq!(GetPixel(canvas.dc, 1, 1), sentinel);
        assert_eq!(dc_state(canvas.dc), before);
    }
}

#[test]
fn high_contrast_removes_presentation_and_theme_switch_releases_cache() {
    let host = Host::new();
    let canvas = Canvas::new();
    let hwnd = host.tabs.handle;
    host.layout(96);
    // SAFETY: Only this test's hidden Tab, memory DC and local Rc/Weak owners.
    unsafe {
        for kind in [Resolved::Light, Resolved::Dark, Resolved::Light] {
            apply(hwnd, kind, palette(kind), 96);
            let data = presentation(hwnd).unwrap();
            let second = point(item_rect(hwnd, 1));
            SendMessageW(hwnd, WM_MOUSEMOVE, 0, second);
            let before = SendMessageW(hwnd, TCM_GETCURSEL, 0, 0);
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            canvas.print(hwnd);
            let weak = Rc::downgrade(&data);
            apply(hwnd, Resolved::Contrast, palette(kind), 96);
            assert!(presentation(hwnd).is_none());
            assert!(!data.attached.get());
            assert!(data.appearance.get().is_none());
            assert_eq!(data.hovered.get(), None);
            assert!(!data.tracking_leave.get());
            assert_eq!(SendMessageW(hwnd, TCM_GETCURSEL, 0, 0), before);
            assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), style);
            drop(data);
            assert_eq!(weak.strong_count(), 0);
        }
        apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
        let data = presentation(hwnd).unwrap();
        let weak = Rc::downgrade(&data);
        drop(data);
        assert_ne!(DestroyWindow(hwnd), 0);
        assert_eq!(weak.strong_count(), 0);
    }
}

struct ReentryProbe {
    message: u32,
    armed: Cell<bool>,
    destroyed: Cell<bool>,
    removed: Cell<bool>,
    alive_during_destroy: Cell<bool>,
    original: Weak<Presentation>,
}

unsafe extern "system" fn destroy_during_query(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    pointer: usize,
) -> LRESULT {
    // SAFETY: Test-owned outer subclass and live boxed probe. Disarm/remove
    // before synchronous destruction; no assertions unwind through native code.
    unsafe {
        let probe = &*(pointer as *const ReentryProbe);
        if message == probe.message && probe.armed.replace(false) {
            probe
                .removed
                .set(RemoveWindowSubclass(hwnd, Some(destroy_during_query), id) != 0);
            probe.destroyed.set(DestroyWindow(hwnd) != 0);
            probe
                .alive_during_destroy
                .set(probe.original.strong_count() > 0);
            return 0;
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

#[test]
fn synchronous_native_query_destruction_keeps_callback_data_alive_then_releases_it() {
    for message in [
        TCM_HITTEST,
        TCM_GETCURSEL,
        TCM_GETCURFOCUS,
        WM_GETFONT,
        WM_QUERYUISTATE,
        TCM_GETITEMRECT,
    ] {
        let host = Host::new();
        let canvas = Canvas::new();
        host.layout(96);
        let hwnd = host.tabs.handle;
        // SAFETY: All queries/destruction target private same-thread windows.
        unsafe {
            apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
            let point = point(item_rect(hwnd, 1));
            let data = presentation(hwnd).unwrap();
            let original = Rc::downgrade(&data);
            drop(data);
            let probe = Box::new(ReentryProbe {
                message,
                armed: Cell::new(true),
                destroyed: Cell::new(false),
                removed: Cell::new(false),
                alive_during_destroy: Cell::new(false),
                original: original.clone(),
            });
            assert_ne!(
                SetWindowSubclass(
                    hwnd,
                    Some(destroy_during_query),
                    2,
                    (&*probe as *const ReentryProbe) as usize
                ),
                0
            );
            if message == TCM_HITTEST {
                SendMessageW(hwnd, WM_MOUSEMOVE, 0, point);
            } else {
                canvas.print(hwnd);
            }
            assert!(!probe.armed.get());
            assert!(probe.removed.get());
            assert!(probe.destroyed.get());
            assert!(probe.alive_during_destroy.get());
            assert_eq!(original.strong_count(), 0);
            assert!(presentation(hwnd).is_none());
        }
    }
}

struct ThemeProbe {
    armed: Cell<bool>,
    original: Weak<Presentation>,
    detached_during_query: Cell<bool>,
}

unsafe extern "system" fn contrast_during_item_query(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    pointer: usize,
) -> LRESULT {
    // SAFETY: Stable test probe and same-thread HWND. Remove this outer subclass
    // before invoking the real high-contrast presentation-removal path.
    unsafe {
        let probe = &*(pointer as *const ThemeProbe);
        if message == TCM_GETITEMRECT && probe.armed.replace(false) {
            RemoveWindowSubclass(hwnd, Some(contrast_during_item_query), id);
            apply(hwnd, Resolved::Contrast, palette(Resolved::Light), 96);
            if let Some(data) = probe.original.upgrade() {
                probe.detached_during_query.set(!data.attached.get());
            }
            return SendMessageW(hwnd, message, w, l);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

#[test]
fn synchronous_high_contrast_switch_rejects_stale_paint_snapshot() {
    let host = Host::new();
    let canvas = Canvas::new();
    let hwnd = host.tabs.handle;
    host.layout(96);
    // SAFETY: Reentrant theme removal is limited to a hidden test Tab and its
    // private target; sentinel pixels prove no stale app-theme drawing follows.
    unsafe {
        apply(hwnd, Resolved::Dark, palette(Resolved::Dark), 96);
        let data = presentation(hwnd).unwrap();
        let original = Rc::downgrade(&data);
        drop(data);
        let probe = Box::new(ThemeProbe {
            armed: Cell::new(true),
            original: original.clone(),
            detached_during_query: Cell::new(false),
        });
        assert_ne!(
            SetWindowSubclass(
                hwnd,
                Some(contrast_during_item_query),
                3,
                (&*probe as *const ThemeProbe) as usize,
            ),
            0
        );
        SetPixel(canvas.dc, 1, 1, 0x123456);
        let selection = SendMessageW(hwnd, TCM_GETCURSEL, 0, 0);
        canvas.print(hwnd);
        assert!(!probe.armed.get());
        assert!(probe.detached_during_query.get());
        assert!(presentation(hwnd).is_none());
        assert_eq!(GetPixel(canvas.dc, 1, 1), 0x123456);
        assert_eq!(SendMessageW(hwnd, TCM_GETCURSEL, 0, 0), selection);
        assert_eq!(original.strong_count(), 0);
    }
}

#[test]
fn repeated_theme_hover_print_and_destroy_do_not_leak_gdi_objects() {
    const CHILD: &str = "AI_RELAY_TAB_GDI_AUDIT_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // Isolate exact process GDI counts from unrelated parallel native tests.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ui::tabs::tests::repeated_theme_hover_print_and_destroy_do_not_leak_gdi_objects",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"),
            "isolated Tab GDI audit failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    use windows_sys::Win32::System::Threading::{
        GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
    };
    fn cycle() {
        let host = Host::new();
        let canvas = Canvas::new();
        // SAFETY: Private hidden native windows and target DC; no physical input.
        unsafe {
            for dpi in [96, 144, 192] {
                let font = Font::new(dpi).unwrap();
                host.layout(dpi);
                host.tabs.set_font(font.0);
                for kind in [Resolved::Light, Resolved::Dark] {
                    apply(host.tabs.handle, kind, palette(kind), dpi);
                    canvas.print(host.tabs.handle);
                    SendMessageW(
                        host.tabs.handle,
                        WM_MOUSEMOVE,
                        0,
                        point(item_rect(host.tabs.handle, 1)),
                    );
                    canvas.print(host.tabs.handle);
                    apply(host.tabs.handle, Resolved::Contrast, palette(kind), dpi);
                }
                host.tabs.set_font(GetStockObject(DEFAULT_GUI_FONT));
            }
        }
    }
    cycle(); // Warm up Common Controls/fonts before the exact-count baseline.
    // SAFETY: Queries inspect only this isolated process's GDI handle count.
    unsafe {
        GdiFlush();
        let process = GetCurrentProcess();
        let baseline = GetGuiResources(process, GR_GDIOBJECTS);
        for _ in 0..30 {
            cycle();
            GdiFlush();
            assert_eq!(GetGuiResources(process, GR_GDIOBJECTS), baseline);
        }
    }
}
