//! Native Tab-backed segmented choices. Input and MSAA remain native; only the
//! header is painted. The compatibility messages cover fixed, Unicode lists,
//! not the mutable ComboBox API. All entry points belong to the HWND owner thread.
use super::theme::{Palette, Resolved};
use crate::windows::{last_error, scale, wide};
use std::{
    mem::size_of,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
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
#[link(name = "user32")]
unsafe extern "system" {
    fn GetFocus() -> HWND;
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
    appearance: Option<Appearance>,
    dpi: u32,
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
        let data = Box::into_raw(Box::new(Choices {
            labels,
            appearance: None,
            dpi: GetDpiForWindow(parent).max(96),
        }));
        if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, data as usize) == 0 {
            let error = last_error("安装分段选择绘制");
            drop(Box::from_raw(data));
            DestroyWindow(hwnd);
            return Err(error);
        }
        // Subclass now owns data, including failures after this point.
        if SetPropW(hwnd, PROPERTY, data.cast()) == 0 {
            let error = last_error("标记分段选择");
            DestroyWindow(hwnd);
            return Err(error);
        }
        SendMessageW(hwnd, TCM_SETCURSEL, 0, 0);
        Ok(hwnd)
    }
}

/// Returns only this component's data, never a native main Tab's State pointer.
unsafe fn marked(hwnd: HWND) -> *mut Choices {
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
            null_mut()
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
    // The assignment ends before sending messages that could invoke the subclass.
    unsafe {
        let data = marked(hwnd);
        if data.is_null() {
            return;
        }
        (*data).dpi = dpi.max(96);
        (*data).appearance = Some(Appearance {
            kind,
            palette,
            dpi: dpi.max(96),
        });
        resize(hwnd, (*data).labels.len(), (*data).dpi);
        InvalidateRect(hwnd, null(), 1);
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

unsafe fn draw(hwnd: HWND, dc: HDC, data: *const Choices, appearance: Appearance) {
    // SAFETY: Paint/print supplies a live DC. Choices outlives this callback;
    // no mutable reference crosses native messages. Stock GDI objects are borrowed,
    // and SaveDC/RestoreDC restores selections/colors before this function returns.
    unsafe {
        let saved = SaveDC(dc);
        if saved == 0 {
            return;
        }
        let palette = appearance.palette;
        let mut bounds = RECT::default();
        GetClientRect(hwnd, &mut bounds);
        SelectObject(dc, GetStockObject(DC_BRUSH));
        SelectObject(dc, GetStockObject(DC_PEN));
        SetDCBrushColor(dc, palette.background);
        FillRect(dc, &bounds, GetStockObject(DC_BRUSH));
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
        let show_focus = GetFocus() == hwnd
            && SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) & UISF_HIDEFOCUS as isize == 0;
        let count = (*data).labels.len();
        for index in 0..count {
            let mut rect = RECT::default();
            if SendMessageW(
                hwnd,
                TCM_GETITEMRECT,
                index,
                (&mut rect as *mut RECT) as LPARAM,
            ) == 0
            {
                continue;
            }
            // Inset only the artwork; native rectangles still define hit testing.
            InflateRect(
                &mut rect,
                -scale(1, appearance.dpi),
                -scale(1, appearance.dpi),
            );
            if rect.right <= rect.left || rect.bottom <= rect.top {
                continue;
            }
            let active = selected == index as isize;
            SetDCBrushColor(
                dc,
                if active && enabled {
                    palette.accent_surface
                } else {
                    palette.surface
                },
            );
            SetDCPenColor(
                dc,
                if active && enabled {
                    palette.accent
                } else {
                    palette.border
                },
            );
            let radius = scale(8, appearance.dpi);
            RoundRect(
                dc,
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                radius,
                radius,
            );
            SetTextColor(
                dc,
                if !enabled {
                    palette.disabled
                } else if active {
                    palette.accent
                } else {
                    palette.disabled
                },
            );
            let mut text = rect;
            InflateRect(&mut text, -scale(7, appearance.dpi), 0);
            if text.right > text.left {
                let label = &(&(*data).labels)[index];
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
                    -scale(3, appearance.dpi),
                    -scale(3, appearance.dpi),
                );
                if rect.right > rect.left && rect.bottom > rect.top {
                    // Focus is independent of the selection/accent color.
                    SetTextColor(dc, palette.text);
                    SetBkColor(
                        dc,
                        if active {
                            palette.accent_surface
                        } else {
                            palette.surface
                        },
                    );
                    DrawFocusRect(dc, &rect);
                }
            }
        }
        RestoreDC(dc, saved);
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
    // SAFETY: Installed only on this thread's native Tab. Its Box remains live
    // until WM_NCDESTROY, with no retained mutable borrows across native calls.
    // Pointer-bearing messages retain their documented Win32 caller contracts.
    unsafe {
        let data = data as *mut Choices;
        match message {
            WM_NCDESTROY => {
                RemovePropW(hwnd, PROPERTY);
                RemoveWindowSubclass(hwnd, Some(procedure), id);
                let result = DefSubclassProc(hwnd, message, w, l);
                drop(Box::from_raw(data));
                return result;
            }
            CB_GETCURSEL => return SendMessageW(hwnd, TCM_GETCURSEL, 0, 0),
            CB_SETCURSEL => {
                // TCM_SETCURSEL returns the OLD selection; ComboBox returns the
                // NEW index on success. Invalid positive indices preserve selection.
                if w == usize::MAX || w < (*data).labels.len() {
                    SendMessageW(hwnd, TCM_SETCURSEL, w, 0);
                    InvalidateRect(hwnd, null(), 0);
                    return SendMessageW(hwnd, TCM_GETCURSEL, 0, 0);
                }
                return CB_ERR as LRESULT;
            }
            CB_GETCOUNT => return (*data).labels.len() as LRESULT,
            CB_GETLBTEXTLEN | CB_GETLBTEXT => {
                let labels = &(*data).labels;
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
                let labels = &(*data).labels;
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
                // Native stores the borrowed HFONT; caller keeps it alive, as for any Tab.
                let result = DefSubclassProc(hwnd, message, w, l);
                resize(hwnd, (*data).labels.len(), (*data).dpi);
                InvalidateRect(hwnd, null(), 0);
                return result;
            }
            _ => {}
        }
        let appearance = (*data).appearance;
        if let Some(appearance) = appearance.filter(|a| a.kind != Resolved::Contrast) {
            if message == WM_PAINT || message == WM_PRINTCLIENT {
                let mut paint = PAINTSTRUCT::default();
                let dc = if message == WM_PAINT {
                    BeginPaint(hwnd, &mut paint)
                } else {
                    w as HDC
                };
                if !dc.is_null() {
                    draw(hwnd, dc, data, appearance);
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
            background: 0xffffff,
            surface: 0xf5f5f5,
            text: 0x202020,
            disabled: 0x606060,
            accent: 0xa04000,
            border: 0xc0c0c0,
            accent_surface: 0xffeee0,
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
                apply(hwnd, kind, palette(), 96);
                assert_eq!(SendMessageW(hwnd, CB_GETCURSEL, 0, 0), 0);
                assert_eq!(SendMessageW(hwnd, WM_GETFONT, 0, 0), font as LRESULT);
            }
            // Other controls, including the native main Tab, are never interpreted as Choices.
            apply(host.0, Resolved::Dark, palette(), 192);
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
                    apply(hwnd, kind, palette(), dpi);
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
            apply(other, Resolved::Dark, palette(), 192);
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
