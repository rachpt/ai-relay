//! Opaque owner-drawn STATIC: the buffered dashboard paints every client pixel.
use crate::windows::last_error;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT},
    UI::{
        Controls::{DRAWITEMSTRUCT, ODA_DRAWENTIRE, ODT_STATIC},
        WindowsAndMessaging::*,
    },
};

const ID: usize = 0x4f56_5346;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

pub(in crate::ui) fn attach(hwnd: HWND) -> Result<(), String> {
    // SAFETY: The caller owns this live STATIC on the UI thread. No app State or
    // heap pointer is retained; destruction removes only this presentation hook.
    if unsafe { SetWindowSubclass(hwnd, Some(procedure), ID, 0) } == 0 {
        Err(last_error("安装概览缓冲绘制"))
    } else {
        Ok(())
    }
}
unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    // SAFETY: Native subclass dispatch on the owner thread. All input, native
    // text/accessibility and owner drawing remain with the original STATIC.
    unsafe {
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(procedure), ID);
        }
        if message == WM_ERASEBKGND {
            return 1;
        }
        if message == WM_SETTEXT {
            // STATIC's own SETTEXT synchronously clears and redraws the client.
            // Base window text storage preserves WM_GETTEXT/MSAA without that
            // intermediate erase; queue the buffered owner draw on the UI turn.
            let result = DefWindowProcW(hwnd, message, w, l);
            if result != 0 {
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            return result;
        }
        if message == WM_PRINTCLIENT
            && (l & PRF_CLIENT as LPARAM == 0
                || (l & PRF_CHECKVISIBLE as LPARAM != 0 && IsWindowVisible(hwnd) == 0))
        {
            return 0;
        }
        if message == WM_PAINT || message == WM_PRINTCLIENT {
            let mut paint = PAINTSTRUCT::default();
            let dc = if message == WM_PAINT {
                BeginPaint(hwnd, &mut paint)
            } else {
                w as _
            };
            if !dc.is_null() {
                let mut item = DRAWITEMSTRUCT {
                    CtlType: ODT_STATIC,
                    CtlID: GetDlgCtrlID(hwnd) as u32,
                    itemAction: ODA_DRAWENTIRE,
                    hwndItem: hwnd,
                    hDC: dc,
                    ..Default::default()
                };
                if GetClientRect(hwnd, &mut item.rcItem) != 0 {
                    SendMessageW(
                        GetParent(hwnd),
                        WM_DRAWITEM,
                        item.CtlID as WPARAM,
                        (&item as *const DRAWITEMSTRUCT) as LPARAM,
                    );
                }
            }
            if message == WM_PAINT {
                EndPaint(hwnd, &paint);
            }
            return 0;
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ui::settings, windows::wide};
    use std::{
        cell::Cell,
        ptr::{null, null_mut},
    };
    use windows_sys::Win32::{
        Foundation::RECT, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
        UI::Controls::DRAWITEMSTRUCT,
    };

    #[derive(Default)]
    struct Counts {
        draws: Cell<usize>,
        backgrounds: Cell<usize>,
    }
    struct Host {
        hwnd: HWND,
        child: HWND,
        counts: Box<Counts>,
    }
    impl Host {
        fn new() -> Self {
            static REGISTER: std::sync::Once = std::sync::Once::new();
            // SAFETY: Private same-thread test windows; boxed counters outlive
            // both HWNDs and all synchronous callbacks, including destruction.
            unsafe {
                let instance = GetModuleHandleW(null());
                let class = windows_sys::w!("AIRelay.Overview.Refresh.Tests");
                REGISTER.call_once(|| {
                    assert_ne!(
                        RegisterClassW(&WNDCLASSW {
                            lpfnWndProc: Some(host_proc),
                            hInstance: instance,
                            lpszClassName: class,
                            ..Default::default()
                        }),
                        0
                    );
                });
                let counts = Box::<Counts>::default();
                let hwnd = CreateWindowExW(
                    0,
                    class,
                    null(),
                    WS_POPUP | WS_CLIPCHILDREN,
                    0,
                    0,
                    420,
                    240,
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&*counts as *const Counts) as isize);
                let child = CreateWindowExW(
                    0,
                    windows_sys::w!("STATIC"),
                    wide("initial").as_ptr(),
                    WS_CHILD | WS_VISIBLE | 0x000d,
                    10,
                    10,
                    400,
                    220,
                    hwnd,
                    super::super::CONTROL_ID as usize as HMENU,
                    instance,
                    null(),
                );
                assert!(!child.is_null());
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                RedrawWindow(
                    hwnd,
                    null(),
                    null_mut(),
                    RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW,
                );
                Self {
                    hwnd,
                    child,
                    counts,
                }
            }
        }
        fn reset(&self) {
            self.counts.draws.set(0);
            self.counts.backgrounds.set(0);
        }
    }
    impl Drop for Host {
        fn drop(&mut self) {
            // SAFETY: Test owns the parent and child; counters remain live.
            unsafe {
                DestroyWindow(self.hwnd);
            }
        }
    }
    unsafe extern "system" fn host_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        // SAFETY: Private test-only GWLP_USERDATA is set after creation and points
        // into Host's stable Box until DestroyWindow has completed.
        unsafe {
            let counts = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Counts;
            if !counts.is_null() {
                if message == WM_CTLCOLORSTATIC {
                    (*counts).backgrounds.set((*counts).backgrounds.get() + 1);
                    return GetStockObject(BLACK_BRUSH) as isize;
                }
                if message == WM_DRAWITEM {
                    (*counts).draws.set((*counts).draws.get() + 1);
                    let item = &*(l as *const DRAWITEMSTRUCT);
                    FillRect(item.hDC, &item.rcItem, GetStockObject(BLACK_BRUSH));
                    return 1;
                }
            }
            DefWindowProcW(hwnd, message, w, l)
        }
    }

    #[test]
    fn native_print_flags_and_hidden_refresh_preserve_the_borrowed_dc() {
        let host = Host::new();
        attach(host.child).unwrap();
        // SAFETY: Test-only bitmap/DC and same-thread HWNDs. Selections are
        // restored before deleting private objects; no desktop pixels are read.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let bitmap = CreateBitmap(480, 280, 1, 32, null());
            assert!(!dc.is_null() && !bitmap.is_null());
            let old = SelectObject(dc, bitmap);
            let pen = GetCurrentObject(dc, OBJ_PEN as u32);
            let font = GetCurrentObject(dc, OBJ_FONT as u32);
            SetTextColor(dc, 0x123456);
            SetViewportOrgEx(dc, 7, 9, null_mut());
            IntersectClipRect(dc, 4, 5, 390, 210);
            for visible in [false, true] {
                ShowWindow(host.hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
                RedrawWindow(
                    host.hwnd,
                    null(),
                    null_mut(),
                    RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW,
                );
                for flags in [
                    0,
                    PRF_NONCLIENT,
                    PRF_CHECKVISIBLE,
                    PRF_CLIENT,
                    PRF_CLIENT | PRF_CHECKVISIBLE,
                ] {
                    SetDCBrushColor(dc, 0x475869);
                    FillRect(
                        dc,
                        &RECT {
                            left: 0,
                            top: 0,
                            right: 480,
                            bottom: 280,
                        },
                        GetStockObject(DC_BRUSH),
                    );
                    host.reset();
                    let before = GetPixel(dc, 20, 20);
                    assert_eq!(
                        SendMessageW(host.child, WM_PRINTCLIENT, dc as WPARAM, flags as LPARAM),
                        0
                    );
                    let draws =
                        flags & PRF_CLIENT != 0 && (visible || flags & PRF_CHECKVISIBLE == 0);
                    assert_eq!(host.counts.draws.get(), usize::from(draws));
                    assert_eq!(GetPixel(dc, 20, 20), if draws { 0 } else { before });
                    assert_eq!(GetCurrentObject(dc, OBJ_PEN as u32), pen);
                    assert_eq!(GetCurrentObject(dc, OBJ_FONT as u32), font);
                    assert_eq!(GetTextColor(dc), 0x123456);
                }
            }
            host.reset();
            ShowWindow(host.hwnd, SW_HIDE);
            settings::set_text(host.child, "hidden precise metadata");
            assert_eq!(settings::text(host.child), "hidden precise metadata");
            assert_eq!(host.counts.draws.get(), 0);
            assert_eq!(host.counts.backgrounds.get(), 0);
            SelectObject(dc, old);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }

    #[test]
    fn native_text_updates_and_opaque_refresh_keep_text_and_geometry() {
        let host = Host::new();
        host.reset();
        settings::set_text(host.child, "native baseline");
        eprintln!(
            "native STATIC WM_SETTEXT: draws={}, background requests={}",
            host.counts.draws.get(),
            host.counts.backgrounds.get()
        );
        attach(host.child).unwrap();
        host.reset();
        settings::set_text(host.child, "精确指标 18446744073709551615 B / 未知");
        eprintln!(
            "opaque STATIC WM_SETTEXT: draws={}, background requests={}",
            host.counts.draws.get(),
            host.counts.backgrounds.get()
        );
        assert_eq!(
            host.counts.draws.get(),
            0,
            "metadata must not synchronously repaint"
        );
        assert_eq!(
            host.counts.backgrounds.get(),
            0,
            "metadata must not clear the client"
        );
        assert_eq!(
            settings::text(host.child),
            "精确指标 18446744073709551615 B / 未知"
        );
        // SAFETY: All scalar queries and writable native structs belong to this fixture.
        unsafe {
            let mut rect = RECT::default();
            assert_ne!(GetWindowRect(host.child, &mut rect), 0);
            let expected = (rect.left, rect.top, rect.right, rect.bottom);
            assert_eq!(SendMessageW(host.child, WM_ERASEBKGND, 0, 0), 1);
            RedrawWindow(
                host.child,
                null(),
                null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
            );
            assert_eq!(host.counts.draws.get(), 1);
            assert_eq!(
                host.counts.backgrounds.get(),
                0,
                "paint must not erase before the buffered draw"
            );
            GetWindowRect(host.child, &mut rect);
            assert_eq!((rect.left, rect.top, rect.right, rect.bottom), expected);
            assert_eq!(
                GetWindowLongPtrW(host.child, GWL_STYLE) as u32 & 0x1f,
                0x000d
            );
        }
    }
}
