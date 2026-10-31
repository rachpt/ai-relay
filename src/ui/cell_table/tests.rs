//! Local HWND tests only. No test invokes the production clipboard writer or
//! tracks a real popup. Hosts start hidden; copy-gating cases show them offscreen
//! without activation. Each test owns/destroys its windows on its own thread.
use super::*;
use crate::windows::wide;
use std::mem::size_of;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

#[link(name = "user32")]
unsafe extern "system" {
    fn EnableWindow(hwnd: HWND, enabled: i32) -> i32;
}

struct Host {
    hwnd: HWND,
    list: HWND,
}
impl Host {
    fn new(columns: &[i32], rows: usize) -> Self {
        // SAFETY: private, hidden same-thread windows, using only local fixtures.
        unsafe {
            let init = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            assert_ne!(InitCommonControlsEx(&init), 0);
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                windows_sys::w!("STATIC"),
                null(),
                WS_POPUP,
                -32000,
                -32000,
                400,
                260,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!hwnd.is_null());
            let list = CreateWindowExW(
                0,
                windows_sys::w!("SysListView32"),
                null(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | LVS_REPORT | LVS_SINGLESEL,
                0,
                0,
                320,
                180,
                hwnd,
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            if list.is_null() {
                DestroyWindow(hwnd);
                panic!("create local ListView");
            }
            let host = Self { hwnd, list };
            SendMessageW(
                list,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                0,
                LVS_EX_FULLROWSELECT as LPARAM,
            );
            for (index, &width) in columns.iter().enumerate() {
                let mut text = wide(&format!("Header {index}"));
                let mut column = LVCOLUMNW {
                    mask: LVCF_TEXT | LVCF_WIDTH,
                    cx: width,
                    pszText: text.as_mut_ptr(),
                    ..Default::default()
                };
                assert_eq!(
                    SendMessageW(
                        list,
                        LVM_INSERTCOLUMNW,
                        index,
                        (&mut column as *mut LVCOLUMNW) as LPARAM
                    ),
                    index as isize
                );
            }
            for row in 0..rows {
                let mut text = wide(&format!("row {row}"));
                let mut item = LVITEMW {
                    mask: LVIF_TEXT,
                    iItem: row as i32,
                    pszText: text.as_mut_ptr(),
                    ..Default::default()
                };
                assert_eq!(
                    SendMessageW(
                        list,
                        LVM_INSERTITEMW,
                        0,
                        (&mut item as *mut LVITEMW) as LPARAM
                    ),
                    row as isize
                );
            }
            install(list).unwrap();
            host
        }
    }
    fn text(&self, row: i32, column: i32, text: &str) {
        // SAFETY: native ListView copies our synchronous NUL-terminated fixture.
        unsafe {
            let mut text = wide(text);
            let mut item = LVITEMW {
                iSubItem: column,
                pszText: text.as_mut_ptr(),
                ..Default::default()
            };
            assert_ne!(
                SendMessageW(
                    self.list,
                    LVM_SETITEMTEXTW,
                    row as usize,
                    (&mut item as *mut LVITEMW) as LPARAM
                ),
                0
            );
        }
    }
    fn select(&self, row: i32, column: i32) -> Target {
        // SAFETY: stack state buffer and component access on the owner thread.
        unsafe {
            let mut item = LVITEMW {
                stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                ..Default::default()
            };
            SendMessageW(
                self.list,
                LVM_SETITEMSTATE,
                usize::MAX,
                (&mut item as *mut LVITEMW) as LPARAM,
            );
            item.state = LVIS_SELECTED | LVIS_FOCUSED;
            SendMessageW(
                self.list,
                LVM_SETITEMSTATE,
                row as usize,
                (&mut item as *mut LVITEMW) as LPARAM,
            );
            let value = interaction(self.list).unwrap();
            activate(self.list, value.stamp(), Some(column));
            current(self.list).unwrap()
        }
    }
    fn show(&self) {
        // SAFETY: only this offscreen test host; never activates a desktop window.
        unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
    }
    fn click(&self, point: POINT) {
        // SAFETY: local owner-thread HWND only, with no global input injection.
        // ListView can enter a mouse-tracking loop inside LBUTTONDOWN. Queue UP
        // first so that loop can finish before the synchronous DOWN returns.
        unsafe {
            let position = packed(point);
            assert_ne!(PostMessageW(self.list, WM_LBUTTONUP, 0, position), 0);
            SendMessageW(self.list, WM_LBUTTONDOWN, 1, position);
            // If native handling did not consume UP, finish the sequence here.
            // Filter to this HWND/message; do not pump unrelated test messages.
            let mut message = MSG::default();
            if PeekMessageW(
                &mut message,
                self.list,
                WM_LBUTTONUP,
                WM_LBUTTONUP,
                PM_REMOVE,
            ) != 0
            {
                DispatchMessageW(&message);
            }
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        // SAFETY: owns parent/children and subclass Box on the creating thread.
        unsafe { DestroyWindow(self.hwnd) };
    }
}
fn packed(point: POINT) -> LPARAM {
    ((point.x as u16 as u32) | ((point.y as u16 as u32) << 16)) as LPARAM
}
fn center(rect: RECT) -> POINT {
    POINT {
        x: (rect.left + rect.right) / 2,
        y: (rect.top + rect.bottom) / 2,
    }
}

#[test]
fn install_validates_thread_class_view_and_is_idempotent() {
    let host = Host::new(&[120, 120], 2);
    assert!(install(null_mut()).is_err());
    assert!(install(host.hwnd).is_err());
    // SAFETY: live local control, no raw pointer is dereferenced on other thread.
    unsafe {
        let before = interaction(host.list).unwrap();
        install(host.list).unwrap();
        assert_eq!(interaction(host.list).unwrap().instance, before.instance);
        let handle = host.list as usize;
        assert!(
            std::thread::spawn(move || install(handle as HWND).is_err())
                .join()
                .unwrap()
        );
        let style = GetWindowLongPtrW(host.list, GWL_STYLE);
        let virtual_list = CreateWindowExW(
            0,
            windows_sys::w!("SysListView32"),
            null(),
            WS_CHILD | LVS_REPORT | LVS_OWNERDATA,
            0,
            0,
            100,
            100,
            host.hwnd,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        assert!(!virtual_list.is_null());
        assert!(install(virtual_list).is_err());
        DestroyWindow(virtual_list);
        SetWindowLongPtrW(
            host.list,
            GWL_STYLE,
            (style & !(LVS_TYPEMASK as isize)) | LVS_ICON as isize,
        );
        assert!(install(host.list).is_err());
        SetWindowLongPtrW(host.list, GWL_STYLE, style);
    }
}

#[test]
fn mouse_subitems_column_zero_tail_and_blank_clear_active_column() {
    let host = Host::new(&[90, 100], 3);
    // SAFETY: sends local synthetic mouse messages without moving real cursor.
    unsafe {
        for column in [1, 0] {
            let cell = Cell { row: 0, column };
            let rect = cell_rect(host.list, cell).unwrap();
            assert_eq!(rect.right - rect.left, if column == 0 { 90 } else { 100 });
            let point = center(rect);
            assert_eq!(hit_cell(host.list, point), Some(cell));
            host.click(point);
            assert_eq!(current(host.list).unwrap().cell, cell);
        }
        let row = cell_rect(host.list, Cell { row: 0, column: 1 }).unwrap();
        for point in [
            POINT {
                x: 250,
                y: center(row).y,
            },
            POINT { x: 25, y: 160 },
        ] {
            assert_eq!(hit_cell(host.list, point), None);
            host.click(point);
            assert!(interaction(host.list).unwrap().column.is_none());
        }
    }
}

#[test]
fn dynamic_unicode_empty_invalid_utf16_and_limit_never_copy_prefixes() {
    let host = Host::new(&[100, 100], 1);
    // Put a surrogate pair across the first 255-unit buffer boundary.
    let long = format!("{}😀中文\tline\r\n{}🦀", "a".repeat(254), "𐐷".repeat(5000));
    host.text(0, 0, "other column");
    host.text(0, 1, &long);
    // SAFETY: tests extraction only, never accesses the system clipboard.
    unsafe {
        let target = host.select(0, 1);
        assert_eq!(read_text(host.list, target).unwrap(), Some(long));
        assert_eq!(
            read_text(host.list, host.select(0, 0)).unwrap().as_deref(),
            Some("other column")
        );
        host.text(0, 1, &"x".repeat(255));
        assert_eq!(
            read_text(host.list, host.select(0, 1))
                .unwrap()
                .unwrap()
                .len(),
            255
        );
        host.text(0, 1, "");
        let target = host.select(0, 1);
        assert_eq!(read_text(host.list, target).unwrap(), Some(String::new()));
        let mut invalid = [0xd800u16, 0];
        let mut item = LVITEMW {
            iSubItem: 1,
            pszText: invalid.as_mut_ptr(),
            ..Default::default()
        };
        SendMessageW(
            host.list,
            LVM_SETITEMTEXTW,
            0,
            (&mut item as *mut LVITEMW) as LPARAM,
        );
        assert!(read_text(host.list, host.select(0, 1)).is_err());
        host.text(0, 1, &"x".repeat(MAX_TEXT_UNITS));
        assert_eq!(
            read_text(host.list, host.select(0, 1))
                .unwrap()
                .unwrap()
                .len(),
            MAX_TEXT_UNITS
        );
        host.text(0, 1, &"x".repeat(MAX_TEXT_UNITS + 1));
        assert!(read_text(host.list, host.select(0, 1)).is_err());
    }
}

#[test]
fn injected_copy_writes_only_one_cell_and_obeys_selection_and_visibility() {
    let host = Host::new(&[100, 100], 2);
    host.text(0, 0, "never include Header 0 or this column");
    host.text(0, 1, "唯一 😀");
    // SAFETY: the injected writer only touches a local Vec, not the clipboard.
    unsafe {
        host.select(0, 1);
        let mut writes = Vec::new();
        assert!(
            !copy_current(host.list, |_, text| {
                writes.push(text.to_owned());
                Ok(())
            })
            .unwrap()
        );
        host.show();
        assert!(
            copy_current(host.list, |_, text| {
                writes.push(text.to_owned());
                Ok(())
            })
            .unwrap()
        );
        assert_eq!(writes, ["唯一 😀"]);
        EnableWindow(host.list, 0);
        assert!(!copy_current(host.list, |_, _| panic!("disabled write")).unwrap());
        EnableWindow(host.list, 1);
        EnableWindow(host.hwnd, 0);
        assert!(!copy_current(host.list, |_, _| panic!("disabled ancestor write")).unwrap());
        EnableWindow(host.hwnd, 1);
        ShowWindow(host.list, SW_HIDE);
        assert!(!copy_current(host.list, |_, _| panic!("hidden write")).unwrap());
        ShowWindow(host.list, SW_SHOWNOACTIVATE);
        let mut item = LVITEMW {
            stateMask: LVIS_SELECTED,
            ..Default::default()
        };
        SendMessageW(
            host.list,
            LVM_SETITEMSTATE,
            usize::MAX,
            (&mut item as *mut LVITEMW) as LPARAM,
        );
        assert!(!copy_current(host.list, |_, _| panic!("no selection write")).unwrap());
        host.text(0, 1, "");
        host.select(0, 1);
        assert!(
            copy_current(host.list, |_, text| {
                assert!(text.is_empty());
                Ok(())
            })
            .unwrap()
        );
        assert!(copy_current(host.list, |_, _| Err("busy".into())).is_err());
    }
}

#[test]
fn scroll_theme_light_dark_light_preserves_active_cell_and_full_copy() {
    use super::super::theme::{Resolved, Theme};
    use crate::config::ThemeMode;

    let host = Host::new(&[100, 100], 2);
    let text = format!("{}😀中文\tline\r\n{}🦀", "a".repeat(254), "𐐷".repeat(500));
    host.text(0, 1, "never copy this row");
    host.text(1, 0, "never copy this column");
    host.text(1, 1, &text);
    host.show();
    // SAFETY: theme changes target the owned, same-thread offscreen HWND;
    // the injected writer only stores local strings, never clipboard data.
    unsafe {
        let target = host.select(1, 1);
        assert_eq!(target.cell, Cell { row: 1, column: 1 });
        let style = GetWindowLongPtrW(host.list, GWL_EXSTYLE);
        assert_eq!(style & WS_EX_COMPOSITED as isize, 0);
        let mut writes = Vec::new();
        for (kind, mode) in [
            (Resolved::Light, ThemeMode::Light),
            (Resolved::Dark, ThemeMode::Dark),
            (Resolved::Light, ThemeMode::Light),
            (Resolved::Contrast, ThemeMode::Light),
            (Resolved::Light, ThemeMode::Light),
        ] {
            let palette = Theme::new(mode).unwrap().palette;
            super::super::scroll_theme::apply(host.list, kind, palette, 96);

            assert_eq!(
                GetWindowLongPtrW(host.list, GWL_EXSTYLE),
                style
                    | if kind != Resolved::Contrast {
                        WS_EX_COMPOSITED as isize
                    } else {
                        0
                    }
            );
            assert_eq!(interaction(host.list).unwrap().column, Some(1));
            assert_eq!(current(host.list), Some(target));
            assert!(
                copy_current(host.list, |owner, text| {
                    assert_eq!(owner, host.list);
                    writes.push(text.to_owned());
                    Ok(())
                })
                .unwrap()
            );
            assert_eq!(current(host.list), Some(target));
        }
        assert_eq!(writes, vec![text; 5]);
    }
}

#[test]
fn other_extended_style_changes_invalidate_active_cell_and_copy() {
    let host = Host::new(&[100, 100], 2);
    host.text(1, 1, "must not reach the writer after invalidation");
    host.show();
    // SAFETY: mutate only the test-owned HWND on its owner thread; writers
    // deliberately panic if an invalid current or saved target reaches them.
    unsafe {
        let style = GetWindowLongPtrW(host.list, GWL_EXSTYLE);
        for toggled in [
            WS_EX_CONTROLPARENT,
            0,
            WS_EX_CONTROLPARENT | WS_EX_COMPOSITED,
            0,
        ] {
            let target = host.select(1, 1);
            let next = style ^ toggled as isize;
            assert_ne!(
                (GetWindowLongPtrW(host.list, GWL_EXSTYLE) ^ next) & !(WS_EX_COMPOSITED as isize),
                0
            );
            SetWindowLongPtrW(host.list, GWL_EXSTYLE, next);
            assert_eq!(GetWindowLongPtrW(host.list, GWL_EXSTYLE), next);
            let value = interaction(host.list).unwrap();
            assert!(value.column.is_none());
            assert_ne!(value.stamp(), target.stamp);
            assert_eq!(value.changing, 0);
            assert!(current(host.list).is_none());
            assert!(!copy_current(host.list, |_, _| panic!("invalid current write")).unwrap());
            assert!(!copy_target(host.list, target, |_, _| panic!("stale target write")).unwrap());
        }
    }
}

#[test]
fn horizontal_keys_follow_visual_order_skip_hidden_columns_and_reveal() {
    let host = Host::new(&[170, 170, 170, 0], 3);
    host.show();
    // SAFETY: all geometry/keyboard messages target the local native control.
    unsafe {
        host.select(0, 0);
        SendMessageW(host.list, WM_KEYDOWN, VK_RIGHT, 0);
        assert_eq!(current(host.list).unwrap().cell.column, 1);
        SendMessageW(host.list, WM_KEYDOWN, VK_RIGHT, 0);
        let target = current(host.list).unwrap();
        assert_eq!(target.cell.column, 2);
        let rect = cell_rect(host.list, target.cell).unwrap();
        let mut client = RECT::default();
        GetClientRect(host.list, &mut client);
        assert!(rect.left >= client.left && rect.right <= client.right);
        SendMessageW(host.list, WM_KEYDOWN, VK_RIGHT, 0);
        assert_eq!(current(host.list).unwrap().cell.column, 2);
        SendMessageW(host.list, WM_KEYDOWN, VK_LEFT, 0);
        SendMessageW(host.list, WM_KEYDOWN, VK_LEFT, 0);
        let target = current(host.list).unwrap();
        assert_eq!(target.cell.column, 0);
        assert!(cell_rect(host.list, target.cell).unwrap().left >= 0);
        let order = [2i32, 0, 3, 1];
        assert_ne!(
            SendMessageW(
                host.list,
                LVM_SETCOLUMNORDERARRAY,
                order.len(),
                order.as_ptr() as LPARAM
            ),
            0
        );
        host.select(0, 0);
        SendMessageW(host.list, WM_KEYDOWN, VK_LEFT, 0);
        assert_eq!(current(host.list).unwrap().cell.column, 2);
        SendMessageW(host.list, WM_KEYDOWN, VK_RIGHT, 0);
        SendMessageW(host.list, WM_KEYDOWN, VK_RIGHT, 0);
        assert_eq!(current(host.list).unwrap().cell.column, 1);
    }
}

#[test]
fn native_row_keys_preserve_active_column() {
    let host = Host::new(&[100, 100], 50);
    host.show();
    // SAFETY: native key navigation, no global keyboard-state changes.
    unsafe {
        host.select(0, 1);
        for (key, expected) in [
            (0x28, Some(1)),
            (0x22, None),
            (0x23, Some(49)),
            (0x21, None),
            (0x24, Some(0)),
        ] {
            let before = selected_row(host.list).unwrap();
            SendMessageW(host.list, WM_KEYDOWN, key, 0);
            let target = current(host.list).unwrap();
            assert_eq!(target.cell.column, 1);
            if let Some(row) = expected {
                assert_eq!(target.cell.row, row);
            } else {
                assert_ne!(target.cell.row, before);
            }
        }
        SendMessageW(host.list, WM_KEYDOWN, 0x28, 0);
        SendMessageW(host.list, WM_KEYDOWN, 0x26, 0);
        assert_eq!(current(host.list).unwrap().cell, Cell { row: 0, column: 1 });
    }
}

#[test]
fn context_mouse_keyboard_blank_and_menu_enablement_without_tracking() {
    let host = Host::new(&[100, 100], 3);
    host.show();
    // SAFETY: builds native menus but never enters a popup loop or copies data.
    unsafe {
        host.select(0, 0);
        let cell = Cell { row: 1, column: 1 };
        let mut point = center(cell_rect(host.list, cell).unwrap());
        ClientToScreen(host.list, &mut point);
        let mouse = context(host.list, packed(point)).unwrap();
        assert_eq!(mouse.target.unwrap().cell, cell);
        let keyboard = context(host.list, -1).unwrap();
        assert_eq!(keyboard.target, mouse.target);
        assert_eq!(
            context(host.list, u32::MAX as LPARAM).unwrap().target,
            keyboard.target
        );
        let mut local = keyboard.point;
        ScreenToClient(host.list, &mut local);
        let rect = cell_rect(host.list, cell).unwrap();
        assert!(local.x >= rect.left && local.x < rect.right);
        let menu = popup_menu(true).unwrap();
        assert_eq!(
            GetMenuState(menu.0, COPY_CELL as u32, MF_BYCOMMAND) & MF_GRAYED,
            0
        );
        let mut caption = [0u16; 64];
        let count = GetMenuStringW(
            menu.0,
            COPY_CELL as u32,
            caption.as_mut_ptr(),
            caption.len() as i32,
            MF_BYCOMMAND,
        );
        assert_eq!(
            String::from_utf16(&caption[..count as usize]).unwrap(),
            "复制单元格\tCtrl+C"
        );
        let mut blank = POINT { x: 270, y: 160 };
        ClientToScreen(host.list, &mut blank);
        assert!(context(host.list, packed(blank)).unwrap().target.is_none());
        assert!(context(host.list, -1).unwrap().target.is_none());
        let menu = popup_menu(false).unwrap();
        assert_ne!(
            GetMenuState(menu.0, COPY_CELL as u32, MF_BYCOMMAND) & MF_GRAYED,
            0
        );
    }
}

#[test]
fn rebuild_delete_edit_and_column_mutation_invalidate_old_targets() {
    let host = Host::new(&[100, 100], 3);
    host.show();
    // SAFETY: never applies a stale target to a real clipboard writer.
    unsafe {
        let old = host.select(0, 1);
        host.text(0, 1, "replacement");
        assert!(current(host.list).is_none());
        host.select(0, 1);
        assert!(!copy_target(host.list, old, |_, _| panic!("stale target")).unwrap());
        SendMessageW(host.list, LVM_DELETEITEM, 2, 0);
        assert!(interaction(host.list).unwrap().column.is_none());
        host.select(0, 1);
        SendMessageW(host.list, LVM_DELETECOLUMN, 1, 0);
        assert!(current(host.list).is_none());
        host.select(0, 0);
        SendMessageW(host.list, LVM_DELETEALLITEMS, 0, 0);
        assert!(interaction(host.list).unwrap().column.is_none());
        assert!(!copy_current(host.list, |_, _| panic!("deleted rows")).unwrap());
    }
}

struct Reentry {
    nested: bool,
    saw_inactive: bool,
    stayed_guarded: bool,
}
struct ReentryGuard(HWND);
impl Drop for ReentryGuard {
    fn drop(&mut self) {
        // SAFETY: declared after the stack state, removed before it can be freed
        // even during assertion unwinding. Parent outlives this guard too.
        unsafe { RemoveWindowSubclass(self.0, Some(rebuilding), 7) };
    }
}
unsafe extern "system" fn rebuilding(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: test owns this stack allocation until subclass removal. Only scalar
    // fields are accessed, never borrowed across the nested SendMessageW call.
    unsafe {
        let state = data as *mut Reentry;
        if message == LVM_DELETEALLITEMS && !(*state).nested {
            (*state).nested = true;
            (*state).saw_inactive = current(hwnd).is_none();
            let mut item = LVITEMW {
                iItem: 0,
                ..Default::default()
            };
            SendMessageW(
                hwnd,
                LVM_INSERTITEMW,
                0,
                (&mut item as *mut LVITEMW) as LPARAM,
            );
            (*state).stayed_guarded = interaction(hwnd).is_some_and(|v| v.changing > 0);
        }
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(rebuilding), id);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}
#[test]
fn reentrant_rebuild_blocks_targets_until_outer_native_mutation_finishes() {
    let host = Host::new(&[100], 2);
    // SAFETY: lower test subclass runs inside the cell component's mutation guard.
    unsafe {
        let mut state = Reentry {
            nested: false,
            saw_inactive: false,
            stayed_guarded: false,
        };
        let mut data = 0;
        assert_ne!(
            GetWindowSubclass(host.list, Some(procedure), SUBCLASS_ID, &mut data),
            0
        );
        assert_ne!(
            RemoveWindowSubclass(host.list, Some(procedure), SUBCLASS_ID),
            0
        );
        drop(Box::from_raw(data as *mut Interaction));
        assert_ne!(
            SetWindowSubclass(
                host.list,
                Some(rebuilding),
                7,
                (&mut state as *mut Reentry) as usize
            ),
            0
        );
        let _guard = ReentryGuard(host.list);
        install(host.list).unwrap();
        host.select(0, 0);
        SendMessageW(host.list, LVM_DELETEALLITEMS, 0, 0);
        assert!(state.nested && state.saw_inactive && state.stayed_guarded);
        assert_eq!(interaction(host.list).unwrap().changing, 0);
        assert!(current(host.list).is_none());
    }
}

struct RefreshOnRead {
    fired: bool,
    destroy: bool,
}
unsafe extern "system" fn refresh_on_read(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: test-owned stack data, scalar accesses only. No reference is held
    // when rebuilding/destroying reenters this subclass and the cell component.
    unsafe {
        let state = data as *mut RefreshOnRead;
        if message == LVM_GETITEMTEXTW && !(*state).fired {
            (*state).fired = true;
            if (*state).destroy {
                DestroyWindow(hwnd);
            } else {
                SendMessageW(hwnd, LVM_DELETEALLITEMS, 0, 0);
            }
            return 0;
        }
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(refresh_on_read), id);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}
struct ReadGuard(HWND);
impl Drop for ReadGuard {
    fn drop(&mut self) {
        // SAFETY: same-thread removal before the borrowed test state is dropped.
        unsafe { RemoveWindowSubclass(self.0, Some(refresh_on_read), 8) };
    }
}
#[test]
fn text_read_reentry_refresh_or_destroy_never_calls_writer() {
    for destroy in [false, true] {
        let host = Host::new(&[100], 1);
        host.show();
        // SAFETY: synthetic GETITEMTEXT interception, writer cannot touch system
        // clipboard. Destruction also exercises Box release inside a nested call.
        unsafe {
            host.select(0, 0);
            let mut state = RefreshOnRead {
                fired: false,
                destroy,
            };
            assert_ne!(
                SetWindowSubclass(
                    host.list,
                    Some(refresh_on_read),
                    8,
                    (&mut state as *mut RefreshOnRead) as usize
                ),
                0
            );
            let _guard = ReadGuard(host.list);
            assert!(!copy_current(host.list, |_, _| panic!("stale read write")).unwrap());
            assert!(state.fired);
            assert!(current(host.list).is_none());
        }
    }
}

#[test]
fn custom_draw_is_postpaint_only_preserves_dc_and_native_selection() {
    let host = Host::new(&[100, 100], 2);
    host.show();
    // SAFETY: private memory DC/bitmap, restored/deleted after all notifications.
    unsafe {
        let target = host.select(0, 1);
        let dc = CreateCompatibleDC(null_mut());
        assert!(!dc.is_null());
        let bitmap = CreateBitmap(320, 180, 1, 32, null());
        assert!(!bitmap.is_null());
        let old = SelectObject(dc, bitmap as HGDIOBJ);
        let mut draw = NMLVCUSTOMDRAW::default();
        draw.nmcd.hdr.hwndFrom = host.list;
        draw.nmcd.hdr.code = NM_CUSTOMDRAW;
        draw.nmcd.hdc = dc;
        draw.nmcd.dwItemSpec = target.cell.row as usize;
        draw.iSubItem = 1;
        draw.clrText = 0x123456;
        draw.clrTextBk = 0x654321;
        for (stage, flags) in [
            (CDDS_PREPAINT, CDRF_NOTIFYITEMDRAW),
            (CDDS_ITEMPREPAINT, CDRF_NOTIFYSUBITEMDRAW),
            (CDDS_ITEMPREPAINT | CDDS_SUBITEM, CDRF_NOTIFYPOSTPAINT),
        ] {
            draw.nmcd.dwDrawStage = stage;
            assert_eq!(custom_draw(draw, None), flags);
        }
        let original = SetDCBrushColor(dc, 0x112233);
        let rect = cell_rect(host.list, target.cell).unwrap();
        for colors in [None, Some((0xf5f2ee, 0x543e29))] {
            SetDCBrushColor(dc, 0x777777);
            let client = RECT {
                left: 0,
                top: 0,
                right: 320,
                bottom: 180,
            };
            FillRect(dc, &client, GetStockObject(DC_BRUSH) as HBRUSH);
            SetDCBrushColor(dc, 0x112233);
            draw.nmcd.dwDrawStage = CDDS_ITEMPOSTPAINT | CDDS_SUBITEM;
            assert_eq!(custom_draw(draw, colors), CDRF_DODEFAULT);
            assert_eq!(GetDCBrushColor(dc), 0x112233);
            assert_ne!(GetPixel(dc, rect.left, rect.top), 0x777777);
            assert_eq!(GetPixel(dc, center(rect).x, center(rect).y), 0x777777);
            assert_eq!(draw.clrText, 0x123456);
            assert_eq!(draw.clrTextBk, 0x654321);
            assert_eq!(current(host.list), Some(target));
        }
        SetDCBrushColor(dc, original);
        SelectObject(dc, old);
        DeleteObject(bitmap as HGDIOBJ);
        DeleteDC(dc);
    }
}

#[test]
fn grouped_report_cells_are_items_not_group_headers() {
    let host = Host::new(&[100, 100], 2);
    // SAFETY: group fixture remains local; v6 availability is checked explicitly.
    unsafe {
        let mut title = wide("Local group");
        let mut group = LVGROUP {
            cbSize: size_of::<LVGROUP>() as u32,
            mask: LVGF_HEADER | LVGF_GROUPID,
            pszHeader: title.as_mut_ptr(),
            iGroupId: 42,
            ..Default::default()
        };
        if SendMessageW(
            host.list,
            LVM_INSERTGROUP,
            usize::MAX,
            (&mut group as *mut LVGROUP) as LPARAM,
        ) == -1
        {
            eprintln!(
                "group-view case skipped: this test process has no Common Controls v6 activation"
            );
            return;
        }
        for row in 0..2 {
            let mut item = LVITEMW {
                mask: LVIF_GROUPID,
                iItem: row,
                iGroupId: 42,
                ..Default::default()
            };
            assert_ne!(
                SendMessageW(
                    host.list,
                    LVM_SETITEMW,
                    0,
                    (&mut item as *mut LVITEMW) as LPARAM
                ),
                0
            );
        }
        assert_ne!(SendMessageW(host.list, LVM_ENABLEGROUPVIEW, 1, 0), -1);
        host.show();
        let cell = Cell { row: 0, column: 1 };
        assert_eq!(
            hit_cell(host.list, center(cell_rect(host.list, cell).unwrap())),
            Some(cell)
        );
        let mut header = RECT {
            top: LVGGR_HEADER as i32,
            ..Default::default()
        };
        assert_ne!(
            SendMessageW(
                host.list,
                LVM_GETGROUPRECT,
                42,
                (&mut header as *mut RECT) as LPARAM
            ),
            0
        );
        assert_eq!(hit_cell(host.list, center(header)), None);
        host.select(0, 1);
        SendMessageW(host.list, LVM_REMOVEALLGROUPS, 0, 0);
        assert!(current(host.list).is_none());
    }
}
