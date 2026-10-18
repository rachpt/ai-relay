//! A clipped, native scrollable page. Controls retain their HWND, values and input;
//! only their parent and vertical position change. The main Tab/footer stay fixed.
use super::*;
use windows_sys::Win32::{Graphics::Gdi::*, UI::Controls::SetScrollInfo};

const TOP: i32 = 48;
const FOOTER: i32 = 56;
const ID: usize = 2400;
const REVEAL: u32 = WM_APP + 80;
type Proc = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: Proc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: Proc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn SetParent(child: HWND, parent: HWND) -> HWND;
    fn GetFocus() -> HWND;
}

#[derive(Default)]
pub(super) struct Viewport {
    pub handle: HWND,
}
struct Page {
    offsets: [i32; 4],
    active: usize,
    height: i32,
    extent: i32,
    dpi: u32,
}
fn limit(extent: i32, height: i32) -> i32 {
    (extent - height).max(0)
}
impl Viewport {
    pub fn create(&mut self, parent: HWND) -> Result<(), String> {
        // SAFETY: UI-owned parent; native STATIC copies the terminated strings.
        unsafe {
            self.handle = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                wide("STATIC").as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | 0x0100,
                0,
                0,
                0,
                0,
                parent,
                ID as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            if self.handle.is_null() {
                return Err(last_error("创建页面滚动区"));
            }
            let data = Box::into_raw(Box::new(Page {
                offsets: [0; 4],
                active: 0,
                height: 0,
                extent: 0,
                dpi: 96,
            }));
            if SetWindowSubclass(self.handle, Some(procedure), ID, data as usize) == 0 {
                drop(Box::from_raw(data));
                DestroyWindow(self.handle);
                return Err(last_error("安装页面滚动"));
            }
        }
        Ok(())
    }
    pub fn adopt(&self, parent: HWND, keep: &[HWND]) -> Result<(), String> {
        // SAFETY: All children belong to the same owner thread; handles/IDs and
        // values survive reparenting. The host forwards control messages unchanged.
        unsafe {
            let mut child = GetWindow(parent, GW_CHILD);
            while !child.is_null() {
                let next = GetWindow(child, GW_HWNDNEXT);
                if child != self.handle && !keep.contains(&child) {
                    if SetParent(child, self.handle).is_null() {
                        return Err(last_error("关联滚动页面"));
                    }
                    // SetParent inserts at the front. Append in original order
                    // so native dialog navigation does not reverse the form.
                    SetWindowPos(
                        child,
                        HWND_BOTTOM,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                    SetWindowSubclass(child, Some(focus_proc), ID, 0);
                }
                child = next;
            }
        }
        Ok(())
    }
    pub fn layout(&self, dpi: u32, width: i32, height: i32, active: usize) -> (i32, i32) {
        // SAFETY: owner-thread live host; subclass retrieves dimensions on SIZE.
        unsafe {
            MoveWindow(
                self.handle,
                0,
                scale(TOP, dpi),
                width.max(1),
                (height - scale(TOP + FOOTER, dpi)).max(1),
                1,
            );
        }
        let mut rect = RECT::default();
        // SAFETY: writable local RECT, no application borrow enters this host.
        unsafe {
            GetClientRect(self.handle, &mut rect);
        }
        let minimum = if active == 2 { 848 } else { 820 };
        (rect.right.max(1), (height).max(scale(minimum, dpi)))
    }
    pub fn arrange(&self, dpi: u32, virtual_height: i32, active: usize) {
        // SAFETY: This private message passes scalars only. Every caller first
        // resets controls to their unscrolled page coordinates via existing layouts.
        unsafe {
            SendMessageW(self.handle, WM_APP + 81, active, dpi as isize);
            SendMessageW(self.handle, WM_APP + 82, virtual_height as usize, 0);
        }
    }
}
unsafe fn scroll(hwnd: HWND, page: *mut Page, target: i32) {
    // SAFETY: host callback owns Page, only copied scalar state crosses child moves.
    unsafe {
        let active = (*page).active;
        let old = (*page).offsets[active];
        let next = target.clamp(0, limit((*page).extent, (*page).height));
        (*page).offsets[active] = next;
        let mut child = GetWindow(hwnd, GW_CHILD);
        while !child.is_null() {
            let after = GetWindow(child, GW_HWNDNEXT);
            let mut r = RECT::default();
            GetWindowRect(child, &mut r);
            let mut p = POINT {
                x: r.left,
                y: r.top,
            };
            ScreenToClient(hwnd, &mut p);
            SetWindowPos(
                child,
                null_mut(),
                p.x,
                p.y + old - next,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            child = after;
        }
        let info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_POS,
            nPos: next,
            ..Default::default()
        };
        SetScrollInfo(hwnd, SB_VERT, &info, 1);
        InvalidateRect(hwnd, null(), 1);
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
    // SAFETY: host/subclass data are owned by this UI thread through NCDESTROY.
    unsafe {
        let page = data as *mut Page;
        match message {
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(procedure), id);
                let r = DefSubclassProc(hwnd, message, w, l);
                drop(Box::from_raw(page));
                r
            }
            WM_SIZE => {
                let mut r = RECT::default();
                GetClientRect(hwnd, &mut r);
                (*page).height = r.bottom;
                DefSubclassProc(hwnd, message, w, l)
            }
            x if x == WM_APP + 81 => {
                (*page).active = w.min(3);
                let next_dpi = (l as u32).max(96);
                if next_dpi != (*page).dpi {
                    for offset in &mut (*page).offsets {
                        *offset = (i64::from(*offset) * i64::from(next_dpi)
                            / i64::from((*page).dpi.max(96)))
                        .min(i64::from(i32::MAX)) as i32;
                    }
                }
                (*page).dpi = next_dpi;
                0
            }
            x if x == WM_APP + 82 => {
                (*page).extent = (w as i32 - scale(TOP + 84, (*page).dpi)).max(0);
                let active = (*page).active;
                (*page).offsets[active] =
                    (*page).offsets[active].clamp(0, limit((*page).extent, (*page).height));
                let offset = (*page).offsets[active] + scale(TOP, (*page).dpi);
                let mut child = GetWindow(hwnd, GW_CHILD);
                while !child.is_null() {
                    let next = GetWindow(child, GW_HWNDNEXT);
                    let mut r = RECT::default();
                    GetWindowRect(child, &mut r);
                    let mut p = POINT {
                        x: r.left,
                        y: r.top,
                    };
                    ScreenToClient(hwnd, &mut p);
                    SetWindowPos(
                        child,
                        null_mut(),
                        p.x,
                        p.y - offset,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    child = next;
                }
                let info = SCROLLINFO {
                    cbSize: size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                    nMin: 0,
                    nMax: (*page).extent.saturating_sub(1),
                    nPage: (*page).height.max(1) as u32,
                    nPos: (*page).offsets[active],
                    ..Default::default()
                };
                SetScrollInfo(hwnd, SB_VERT, &info, 1);
                0
            }
            WM_VSCROLL => {
                let mut info = SCROLLINFO {
                    cbSize: size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_ALL,
                    ..Default::default()
                };
                GetScrollInfo(hwnd, SB_VERT, &mut info);
                let old = (*page).offsets[(*page).active];
                let step = scale(40, (*page).dpi);
                let next = match (w & 0xffff) as i32 {
                    SB_LINEUP => old - step,
                    SB_LINEDOWN => old + step,
                    SB_PAGEUP => old - (*page).height * 3 / 4,
                    SB_PAGEDOWN => old + (*page).height * 3 / 4,
                    SB_TOP => 0,
                    SB_BOTTOM => (*page).extent,
                    SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos,
                    _ => old,
                };
                scroll(hwnd, page, next);
                0
            }
            WM_MOUSEWHEEL => {
                let delta = ((w >> 16) as u16 as i16) as i32;
                let old = (*page).offsets[(*page).active];
                scroll(hwnd, page, old - delta * scale(48, (*page).dpi) / 120);
                0
            }
            REVEAL => {
                let child = w as HWND;
                if GetParent(child) != hwnd || GetFocus() != child || IsWindowVisible(child) == 0 {
                    return 0;
                }
                let mut r = RECT::default();
                GetWindowRect(child, &mut r);
                let mut p = POINT {
                    x: r.left,
                    y: r.top,
                };
                ScreenToClient(hwnd, &mut p);
                let h = (r.bottom - r.top).min(scale(40, (*page).dpi));
                let old = (*page).offsets[(*page).active];
                if p.y < 0 {
                    scroll(hwnd, page, old + p.y - scale(8, (*page).dpi));
                } else if p.y + h > (*page).height {
                    scroll(
                        hwnd,
                        page,
                        old + p.y + h - (*page).height + scale(8, (*page).dpi),
                    );
                }
                0
            }
            WM_ERASEBKGND => {
                let mut r = RECT::default();
                GetClientRect(hwnd, &mut r);
                let parent = GetParent(hwnd);
                let brush = SendMessageW(parent, WM_CTLCOLORSTATIC, w, hwnd as isize) as HBRUSH;
                FillRect(w as HDC, &r, brush);
                1
            }
            WM_COMMAND | WM_NOTIFY | WM_DRAWITEM | WM_MEASUREITEM | WM_CTLCOLORSTATIC
            | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
                SendMessageW(GetParent(hwnd), message, w, l)
            }
            theme::REFRESH => {
                PostMessageW(GetParent(hwnd), message, w, l);
                0
            }
            _ => DefSubclassProc(hwnd, message, w, l),
        }
    }
}
unsafe extern "system" fn focus_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    // SAFETY: installed on a live native child; notification is asynchronous so
    // dialog navigation never holds a State borrow while scrolling child windows.
    unsafe {
        if message == WM_SETFOCUS {
            PostMessageW(GetParent(hwnd), REVEAL, hwnd as usize, 0);
        }
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(focus_proc), id);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const ROUTED: LRESULT = 0x2468;
    const FORWARDED: [u32; 8] = [
        WM_COMMAND,
        WM_NOTIFY,
        WM_DRAWITEM,
        WM_MEASUREITEM,
        WM_CTLCOLORSTATIC,
        WM_CTLCOLOREDIT,
        WM_CTLCOLORLISTBOX,
        WM_CTLCOLORBTN,
    ];

    struct Parent(HWND);
    impl Drop for Parent {
        fn drop(&mut self) {
            // SAFETY: This thread owns the parent and all adopted children.
            // DestroyWindow also releases the viewport's subclass-owned Page.
            unsafe { DestroyWindow(self.0) };
        }
    }

    struct NativeViewport {
        // Drop the HWND tree before the subclass's boxed observation data.
        parent: Parent,
        viewport: Viewport,
        main_tab: HWND,
        status: HWND,
        controls: [HWND; 3],
        routed: Box<Cell<(u32, WPARAM, LPARAM)>>,
    }

    unsafe extern "system" fn parent_proc(
        hwnd: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: The boxed Cell remains at a stable address until after parent
        // destruction. Calls are synchronous on its owner thread, with no State.
        unsafe {
            if FORWARDED.contains(&message) {
                (*(data as *const Cell<(u32, WPARAM, LPARAM)>)).set((message, w, l));
                return ROUTED;
            }
            if message == WM_NCDESTROY {
                RemoveWindowSubclass(hwnd, Some(parent_proc), id);
            }
            DefSubclassProc(hwnd, message, w, l)
        }
    }

    fn child(parent: HWND, id: usize, y: i32) -> HWND {
        // SAFETY: The test owns this live parent on the current thread; STATIC
        // copies the terminated class name and has no application callbacks.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                null(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                12,
                y,
                120,
                24,
                parent,
                id as HMENU,
                GetModuleHandleW(null()),
                null(),
            )
        };
        assert!(!hwnd.is_null(), "create native child {id}");
        hwnd
    }

    fn children(parent: HWND) -> Vec<HWND> {
        let mut result = Vec::new();
        // SAFETY: Enumerates only this thread's live test-owned HWND tree.
        unsafe {
            let mut hwnd = GetWindow(parent, GW_CHILD);
            while !hwnd.is_null() {
                result.push(hwnd);
                hwnd = GetWindow(hwnd, GW_HWNDNEXT);
            }
        }
        result
    }

    fn rect_in(hwnd: HWND, parent: HWND) -> RECT {
        let mut rect = RECT::default();
        // SAFETY: Both HWNDs are live on this thread; RECT/POINT are writable
        // locals, and screen coordinates work even with a hidden top-level HWND.
        unsafe {
            assert_ne!(GetWindowRect(hwnd, &mut rect), 0);
            let mut origin = POINT {
                x: rect.left,
                y: rect.top,
            };
            assert_ne!(ScreenToClient(parent, &mut origin), 0);
            RECT {
                left: origin.x,
                top: origin.y,
                right: origin.x + rect.right - rect.left,
                bottom: origin.y + rect.bottom - rect.top,
            }
        }
    }

    fn scroll_info(hwnd: HWND) -> SCROLLINFO {
        let mut info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_ALL,
            ..Default::default()
        };
        // SAFETY: Live test-owned viewport and correctly sized writable info.
        assert_ne!(unsafe { GetScrollInfo(hwnd, SB_VERT, &mut info) }, 0);
        info
    }

    fn scroll_command(hwnd: HWND, command: i32) {
        // SAFETY: Synchronous scalar-only message to the test-owned viewport.
        unsafe { SendMessageW(hwnd, WM_VSCROLL, command as WPARAM, 0) };
    }

    impl NativeViewport {
        fn new() -> Self {
            // SAFETY: Hidden native parent on this thread; no app/controller,
            // activation, system DPI changes, or external resources are involved.
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    null(),
                    WS_POPUP,
                    0,
                    0,
                    1600,
                    1000,
                    null_mut(),
                    null_mut(),
                    GetModuleHandleW(null()),
                    null(),
                )
            };
            assert!(!hwnd.is_null(), "create native parent");
            let parent = Parent(hwnd);
            let main_tab = child(hwnd, 1, 0);
            let status = child(hwnd, 2, 320);
            let controls = [child(hwnd, 3, 64), child(hwnd, 4, 320), child(hwnd, 5, 700)];
            let mut viewport = Viewport::default();
            viewport.create(hwnd).unwrap();
            let original = children(hwnd);
            let expected: Vec<_> = original
                .iter()
                .copied()
                .filter(|child| ![main_tab, status, viewport.handle].contains(child))
                .collect();
            assert_eq!(expected.len(), controls.len());
            viewport.adopt(hwnd, &[main_tab, status]).unwrap();
            assert_eq!(
                children(viewport.handle),
                expected,
                "adopt preserves native order"
            );
            assert_eq!(
                children(hwnd),
                original
                    .into_iter()
                    .filter(|child| [main_tab, status, viewport.handle].contains(child))
                    .collect::<Vec<_>>()
            );
            // SAFETY: All handles are test-owned, live and on this thread.
            unsafe {
                assert_eq!(GetParent(main_tab), hwnd);
                assert_eq!(GetParent(status), hwnd);
                for control in controls {
                    assert_eq!(GetParent(control), viewport.handle);
                }
            }
            let fixture = Self {
                parent,
                viewport,
                main_tab,
                status,
                controls,
                routed: Box::new(Cell::new((0, 0, 0))),
            };
            // SAFETY: The Cell is heap-stable across returning this fixture;
            // field drop order keeps it live through parent WM_NCDESTROY.
            assert_ne!(
                unsafe {
                    SetWindowSubclass(
                        hwnd,
                        Some(parent_proc),
                        1,
                        fixture.routed.as_ref() as *const _ as usize,
                    )
                },
                0
            );
            fixture
        }

        fn layout_reset_arrange(&self, dpi: u32, active: usize) -> i32 {
            let (width, virtual_height) =
                self.viewport
                    .layout(dpi, scale(640, dpi), scale(360, dpi), active);
            assert!(width > 0);
            let tops = [
                scale(64, dpi),
                scale(320, dpi),
                virtual_height - scale(84 + 24, dpi),
            ];
            for (control, y) in self.controls.into_iter().zip(tops) {
                // SAFETY: Reset live children to original page coordinates before
                // arrange, exactly as the production per-page layouts must do.
                assert_ne!(
                    unsafe {
                        MoveWindow(
                            control,
                            scale(12, dpi),
                            y,
                            scale(120, dpi),
                            scale(24, dpi),
                            0,
                        )
                    },
                    0
                );
            }
            self.viewport.arrange(dpi, virtual_height, active);
            virtual_height
        }
    }

    #[test]
    fn scrolling_bounds() {
        assert_eq!(limit(800, 500), 300);
        assert_eq!(limit(300, 500), 0);
        for dpi in [96, 144, 192] {
            assert!(scale(560 - TOP - FOOTER, dpi) > 0);
            assert_eq!(
                limit(scale(848 - TOP - 84, dpi), scale(560 - TOP - FOOTER, dpi)),
                scale(260, dpi)
            );
        }
    }

    #[test]
    fn native_adopt_preserves_order_keeps_fixed_children_and_routes_messages() {
        let native = NativeViewport::new();
        native.layout_reset_arrange(96, 0);
        assert_eq!(rect_in(native.main_tab, native.parent.0).top, 0);
        assert_eq!(rect_in(native.status, native.parent.0).top, 320);
        for message in FORWARDED {
            // SAFETY: The test parent subclass consumes these probe scalars
            // without dereferencing lParam, before STATIC's default procedure.
            let result = unsafe { SendMessageW(native.viewport.handle, message, 0x1234, 0x5678) };
            assert_eq!(result, ROUTED, "forward message {message:#x}");
            assert_eq!(native.routed.get(), (message, 0x1234, 0x5678));
        }
    }

    #[test]
    fn native_layout_maps_top_and_bottom_scroll_reveals_entire_last_control() {
        for dpi in [96, 144, 192] {
            let native = NativeViewport::new();
            let virtual_height = native.layout_reset_arrange(dpi, 0);
            let host = rect_in(native.viewport.handle, native.parent.0);
            assert_eq!(host.top, scale(TOP, dpi));
            assert_eq!(host.bottom, scale(360 - FOOTER, dpi));
            assert_eq!(
                rect_in(native.controls[0], native.viewport.handle).top,
                scale(64 - TOP, dpi)
            );
            assert_eq!(
                rect_in(native.controls[0], native.parent.0).top,
                scale(64, dpi)
            );
            let before = scroll_info(native.viewport.handle);
            assert_eq!(before.nPos, 0);
            assert_eq!(before.nMax + 1, virtual_height - scale(TOP + 84, dpi));
            assert_eq!(before.nPage as i32, scale(360 - TOP - FOOTER, dpi));
            assert!(rect_in(native.controls[2], native.viewport.handle).top >= before.nPage as i32);
            scroll_command(native.viewport.handle, SB_BOTTOM);
            let bottom = scroll_info(native.viewport.handle);
            assert_eq!(bottom.nPos, bottom.nMax - bottom.nPage as i32 + 1);
            assert!(bottom.nPos > 0);
            let last = rect_in(native.controls[2], native.viewport.handle);
            assert!(last.top >= 0);
            assert_eq!(last.bottom, bottom.nPage as i32);
            assert_eq!(rect_in(native.main_tab, native.parent.0).top, 0);
            assert_eq!(rect_in(native.status, native.parent.0).top, 320);
            scroll_command(native.viewport.handle, SB_TOP);
            assert_eq!(scroll_info(native.viewport.handle).nPos, 0);
            assert_eq!(
                rect_in(native.controls[0], native.viewport.handle).top,
                scale(64 - TOP, dpi)
            );
        }
    }

    #[test]
    fn native_switch_active_retains_each_offset_without_coordinate_drift() {
        let native = NativeViewport::new();
        let offsets = [40, 80, 120, 160];
        for (active, expected) in offsets.into_iter().enumerate() {
            native.layout_reset_arrange(96, active);
            for _ in 0..=active {
                scroll_command(native.viewport.handle, SB_LINEDOWN);
            }
            assert_eq!(scroll_info(native.viewport.handle).nPos, expected);
        }
        for active in [3, 1, 0, 2, 3, 0] {
            for _ in 0..2 {
                native.layout_reset_arrange(96, active);
                assert_eq!(scroll_info(native.viewport.handle).nPos, offsets[active]);
                assert_eq!(
                    rect_in(native.controls[0], native.viewport.handle).top,
                    64 - TOP - offsets[active]
                );
            }
        }
    }

    #[test]
    fn native_private_dpi_message_scales_all_four_saved_offsets() {
        let native = NativeViewport::new();
        let offsets = [40, 80, 120, 160];
        for (active, expected) in offsets.into_iter().enumerate() {
            native.layout_reset_arrange(96, active);
            for _ in 0..=active {
                scroll_command(native.viewport.handle, SB_LINEDOWN);
            }
            assert_eq!(scroll_info(native.viewport.handle).nPos, expected);
        }
        for dpi in [144, 192, 96] {
            // SAFETY: Exercise the scalar-only private DPI transition explicitly;
            // no system/thread DPI setting or physical monitor change is needed.
            unsafe { SendMessageW(native.viewport.handle, WM_APP + 81, 0, dpi as LPARAM) };
            for (active, offset) in offsets.into_iter().enumerate() {
                native.layout_reset_arrange(dpi, active);
                let expected = scale(offset, dpi);
                assert_eq!(scroll_info(native.viewport.handle).nPos, expected);
                assert_eq!(
                    rect_in(native.controls[0], native.viewport.handle).top,
                    scale(64 - TOP, dpi) - expected
                );
            }
        }
    }

    #[test]
    fn native_stale_reveal_for_hidden_unfocused_child_keeps_scroll_position() {
        let native = NativeViewport::new();
        native.layout_reset_arrange(96, 0);
        scroll_command(native.viewport.handle, SB_LINEDOWN);
        let before = scroll_info(native.viewport.handle).nPos;
        assert!(before > 0);
        let last = native.controls[2];
        let rect = rect_in(last, native.viewport.handle);
        assert!(rect.top > scroll_info(native.viewport.handle).nPage as i32);
        // SAFETY: Live test-owned child; the top-level parent remains hidden, so
        // this test deliberately makes no claim about real keyboard focus/reveal.
        unsafe {
            ShowWindow(last, SW_HIDE);
            assert_eq!(IsWindowVisible(last), 0);
            assert_ne!(GetFocus(), last);
            SendMessageW(native.viewport.handle, REVEAL, last as WPARAM, 0);
        }
        assert_eq!(scroll_info(native.viewport.handle).nPos, before);
        assert_eq!(rect_in(last, native.viewport.handle).top, rect.top);
    }
}
