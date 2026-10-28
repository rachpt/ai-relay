//! Single-cell interaction for an existing, non-owner-data report ListView.
//! Only a column is retained: the native control remains the source of row
//! selection. All access belongs to the HWND's thread; no application State is used.
use crate::windows::{Menu, last_error};
use std::{
    ptr::{null, null_mut},
    sync::atomic::{AtomicUsize, Ordering},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::Threading::GetCurrentThreadId,
    UI::{Controls::*, HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

const SUBCLASS_ID: usize = 0x4345_4c4c;
const MAX_COLUMNS: usize = 4096;
// UTF-16 code units, excluding NUL. Reject oversize/malformed text, never copy a
// silently truncated prefix (including a split surrogate pair).
const MAX_TEXT_UNITS: usize = 1024 * 1024;
const COPY_CELL: usize = 1;
const VK_LEFT: usize = 0x25;
const VK_RIGHT: usize = 0x27;
const VK_CONTROL: i32 = 0x11;
const VK_SHIFT: i32 = 0x10;
const VK_MENU: i32 = 0x12;
const VK_APPS: usize = 0x5d;
const VK_F10: usize = 0x79;
static NEXT_INSTANCE: AtomicUsize = AtomicUsize::new(1);

type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn GetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: *mut usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}
// Public APIs behind otherwise unused windows-sys input features.
#[link(name = "user32")]
unsafe extern "system" {
    fn GetKeyState(key: i32) -> i16;
    fn SetFocus(hwnd: HWND) -> HWND;
    fn IsWindowEnabled(hwnd: HWND) -> i32;
    fn MessageBeep(kind: u32) -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell {
    row: i32,
    column: i32,
}
#[derive(Clone, Copy)]
struct Interaction {
    instance: usize,
    revision: u64,
    column: Option<i32>,
    changing: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    instance: usize,
    revision: u64,
}
impl Interaction {
    fn stamp(self) -> Stamp {
        Stamp {
            instance: self.instance,
            revision: self.revision,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    stamp: Stamp,
    cell: Cell,
}

/// Install once (repeated calls are harmless) on the calling thread's existing
/// SysListView32, LVS_REPORT, without LVS_OWNERDATA. No columns/styles are changed.
/// Integration also routes custom draw through control_theme, as before.
pub(super) fn install(hwnd: HWND) -> Result<(), String> {
    // SAFETY: Validate owner-thread identity before class, messages or subclass
    // data access. A caller from another thread never dereferences our Box.
    unsafe {
        if hwnd.is_null() || GetWindowThreadProcessId(hwnd, null_mut()) != GetCurrentThreadId() {
            return Err("单元格表格必须属于当前界面线程".into());
        }
        let mut class = [0u16; 64];
        let length = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        if String::from_utf16_lossy(&class[..length.max(0) as usize]) != "SysListView32"
            || !report(hwnd)
        {
            return Err("单元格复制仅支持非虚拟报表 ListView".into());
        }
        if interaction(hwnd).is_some() {
            return Ok(());
        }
        let data = Box::into_raw(Box::new(Interaction {
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            revision: 0,
            column: None,
            changing: 0,
        }));
        if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, data as usize) == 0 {
            drop(Box::from_raw(data));
            return Err(last_error("安装单元格表格"));
        }
        Ok(())
    }
}

unsafe fn report(hwnd: HWND) -> bool {
    // SAFETY: same-thread HWND; invalid handles fail closed.
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    style & LVS_TYPEMASK == LVS_REPORT && style & LVS_OWNERDATA == 0
}
unsafe fn interaction(hwnd: HWND) -> Option<Interaction> {
    // SAFETY: owner thread only. Copy data before any potentially reentrant API;
    // this proc/id exclusively owns the Box until WM_NCDESTROY.
    unsafe {
        if hwnd.is_null() || GetWindowThreadProcessId(hwnd, null_mut()) != GetCurrentThreadId() {
            return None;
        }
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) == 0 || data == 0 {
            None
        } else {
            Some(*(data as *const Interaction))
        }
    }
}
unsafe fn store(hwnd: HWND, value: Interaction) {
    // SAFETY: same-thread lookup, no callback between lookup and assignment. A
    // distinct instance must not receive a stale write after HWND reuse.
    unsafe {
        if hwnd.is_null() || GetWindowThreadProcessId(hwnd, null_mut()) != GetCurrentThreadId() {
            return;
        }
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, &mut data) != 0
            && data != 0
            && (*(data as *const Interaction)).instance == value.instance
        {
            *(data as *mut Interaction) = value;
        }
    }
}
unsafe fn unchanged(hwnd: HWND, stamp: Stamp) -> bool {
    // SAFETY: caller is on owner thread, including after a nested message loop.
    unsafe { interaction(hwnd).is_some_and(|v| v.stamp() == stamp && v.changing == 0) }
}
unsafe fn activate(hwnd: HWND, stamp: Stamp, column: Option<i32>) {
    // SAFETY: only copy/assign our data, with no borrow across InvalidateRect.
    unsafe {
        if let Some(mut value) = interaction(hwnd).filter(|v| v.stamp() == stamp && v.changing == 0)
        {
            if value.column != column {
                value.column = column;
                value.revision = value.revision.wrapping_add(1);
                store(hwnd, value);
            }
            InvalidateRect(hwnd, null(), 0);
        }
    }
}
unsafe fn usable(hwnd: HWND) -> bool {
    // SAFETY: owner-thread HWND. IsWindowVisible includes hidden ancestors;
    // IsWindowEnabled does not, so explicitly check disabled child ancestors.
    unsafe {
        if IsWindowVisible(hwnd) == 0 || !report(hwnd) {
            return false;
        }
        let mut ancestor = hwnd;
        loop {
            if IsWindowEnabled(ancestor) == 0 {
                return false;
            }
            if GetWindowLongPtrW(ancestor, GWL_STYLE) as u32 & WS_CHILD == 0 {
                return true;
            }
            ancestor = GetParent(ancestor);
            if ancestor.is_null() {
                return false;
            }
        }
    }
}
unsafe fn column_count(hwnd: HWND) -> Option<usize> {
    // SAFETY: synchronous scalar messages on a native report control.
    unsafe {
        let header = SendMessageW(hwnd, LVM_GETHEADER, 0, 0) as HWND;
        if header.is_null() {
            return None;
        }
        let count = SendMessageW(header, HDM_GETITEMCOUNT, 0, 0);
        (count > 0 && count as usize <= MAX_COLUMNS).then_some(count as usize)
    }
}
unsafe fn selected_row(hwnd: HWND) -> Option<i32> {
    // SAFETY: native selection remains authoritative, including multi-selection.
    unsafe {
        let focused = SendMessageW(
            hwnd,
            LVM_GETNEXTITEM,
            usize::MAX,
            (LVNI_SELECTED | LVNI_FOCUSED) as LPARAM,
        );
        let row = if focused >= 0 {
            focused
        } else {
            SendMessageW(hwnd, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as LPARAM)
        };
        (row >= 0 && row <= i32::MAX as isize).then_some(row as i32)
    }
}
unsafe fn current(hwnd: HWND) -> Option<Target> {
    // SAFETY: all native calls use scalars, with a revision check AFTER callbacks.
    unsafe {
        let value = interaction(hwnd)?;
        if value.changing != 0 || !report(hwnd) {
            return None;
        }
        let column = value.column?;
        let count = column_count(hwnd)?;
        if column < 0
            || column as usize >= count
            || SendMessageW(hwnd, LVM_GETCOLUMNWIDTH, column as usize, 0) <= 0
        {
            return None;
        }
        let row = selected_row(hwnd)?;
        unchanged(hwnd, value.stamp()).then_some(Target {
            stamp: value.stamp(),
            cell: Cell { row, column },
        })
    }
}

unsafe fn cell_rect(hwnd: HWND, cell: Cell) -> Option<RECT> {
    // SAFETY: stack buffers live through each synchronous native message. Using
    // the header's horizontal bounds handles reordered columns AND column zero
    // (GETSUBITEMRECT/BOUNDS returns the whole row for that special column).
    unsafe {
        let mut rect = RECT {
            left: LVIR_BOUNDS as i32,
            top: cell.column,
            ..Default::default()
        };
        if SendMessageW(
            hwnd,
            LVM_GETSUBITEMRECT,
            cell.row as usize,
            (&mut rect as *mut RECT) as LPARAM,
        ) == 0
        {
            return None;
        }
        let header = SendMessageW(hwnd, LVM_GETHEADER, 0, 0) as HWND;
        let mut column = RECT::default();
        if header.is_null()
            || SendMessageW(
                header,
                HDM_GETITEMRECT,
                cell.column as usize,
                (&mut column as *mut RECT) as LPARAM,
            ) == 0
        {
            return None;
        }
        MapWindowPoints(header, hwnd, (&mut column as *mut RECT).cast::<POINT>(), 2);
        rect.left = column.left;
        rect.right = column.right;
        (rect.left < rect.right && rect.top < rect.bottom).then_some(rect)
    }
}
fn contains(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}
fn message_point(l: LPARAM) -> POINT {
    POINT {
        x: l as u16 as i16 as i32,
        y: (l as usize >> 16) as u16 as i16 as i32,
    }
}
unsafe fn hit_cell(hwnd: HWND, point: POINT) -> Option<Cell> {
    // SAFETY: full LVHITTESTINFO supports native grouped non-owner-data lists.
    // Group headers/footers/empty row tails cannot pass both hit and cell bounds.
    unsafe {
        let mut hit = LVHITTESTINFO {
            pt: point,
            ..Default::default()
        };
        let row = SendMessageW(
            hwnd,
            LVM_SUBITEMHITTEST,
            usize::MAX,
            (&mut hit as *mut LVHITTESTINFO) as LPARAM,
        );
        if row < 0
            || hit.iItem < 0
            || hit.iSubItem < 0
            || hit.flags & (LVHT_EX_GROUP | LVHT_EX_FOOTER) != 0
            || hit.flags & (LVHT_ONITEMICON | LVHT_ONITEMLABEL | LVHT_ONITEMSTATEICON) == 0
        {
            return None;
        }
        if hit.iSubItem as usize >= column_count(hwnd)? {
            return None;
        }
        let cell = Cell {
            row: hit.iItem,
            column: hit.iSubItem,
        };
        contains(cell_rect(hwnd, cell)?, point).then_some(cell)
    }
}
unsafe fn select_cell(hwnd: HWND, stamp: Stamp, cell: Cell) {
    // SAFETY: stack LVITEMW is used synchronously. Selection notifications can
    // rebuild/destroy the control, so revalidate before every subsequent write.
    unsafe {
        if !unchanged(hwnd, stamp) {
            return;
        }
        SetFocus(hwnd);
        if !unchanged(hwnd, stamp) {
            return;
        }
        let mut item = LVITEMW {
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        SendMessageW(
            hwnd,
            LVM_SETITEMSTATE,
            usize::MAX,
            (&mut item as *mut LVITEMW) as LPARAM,
        );
        if !unchanged(hwnd, stamp) {
            return;
        }
        item.state = LVIS_SELECTED | LVIS_FOCUSED;
        SendMessageW(
            hwnd,
            LVM_SETITEMSTATE,
            cell.row as usize,
            (&mut item as *mut LVITEMW) as LPARAM,
        );
        if unchanged(hwnd, stamp) && selected_row(hwnd) == Some(cell.row) {
            activate(hwnd, stamp, Some(cell.column));
        }
    }
}
unsafe fn reveal(hwnd: HWND, target: Target, right: bool) {
    // SAFETY: no retained references across ENSUREVISIBLE/SCROLL callbacks.
    unsafe {
        SendMessageW(hwnd, LVM_ENSUREVISIBLE, target.cell.row as usize, 0);
        if current(hwnd) != Some(target) {
            return;
        }
        let Some(rect) = cell_rect(hwnd, target.cell) else {
            return;
        };
        let mut client = RECT::default();
        if GetClientRect(hwnd, &mut client) == 0 || current(hwnd) != Some(target) {
            return;
        }
        let width = client.right - client.left;
        let dx = if rect.right - rect.left > width {
            if right {
                rect.left - client.left
            } else {
                rect.right - client.right
            }
        } else if rect.left < client.left {
            rect.left - client.left
        } else if rect.right > client.right {
            rect.right - client.right
        } else {
            0
        };
        if dx != 0 {
            SendMessageW(hwnd, LVM_SCROLL, dx as isize as usize, 0);
        }
    }
}
unsafe fn move_column(hwnd: HWND, right: bool) {
    // SAFETY: order/width buffers belong to this stack; revision guards native
    // callbacks. Native up/down/page/home/end never pass through this function.
    unsafe {
        let Some(value) = interaction(hwnd).filter(|v| v.changing == 0) else {
            return;
        };
        if !usable(hwnd) || selected_row(hwnd).is_none() {
            return;
        }
        let Some(count) = column_count(hwnd) else {
            return;
        };
        let mut order = vec![0i32; count];
        if SendMessageW(
            hwnd,
            LVM_GETCOLUMNORDERARRAY,
            count,
            order.as_mut_ptr() as LPARAM,
        ) == 0
        {
            return;
        }
        order.retain(|&column| {
            column >= 0
                && (column as usize) < count
                && SendMessageW(hwnd, LVM_GETCOLUMNWIDTH, column as usize, 0) > 0
        });
        let index = value
            .column
            .and_then(|column| order.iter().position(|&v| v == column));
        let next = match index {
            Some(index) if right => (index + 1).min(order.len().saturating_sub(1)),
            Some(index) => index.saturating_sub(1),
            None => 0,
        };
        let Some(&column) = order.get(next) else {
            return;
        };
        activate(hwnd, value.stamp(), Some(column));
        if let Some(target) = current(hwnd) {
            reveal(hwnd, target, right);
        }
    }
}

unsafe fn read_text(hwnd: HWND, target: Target) -> Result<Option<String>, String> {
    // SAFETY: each GETITEMTEXTW owns its bounded writable UTF-16 buffer for the
    // entire synchronous call. Callback text can reenter; never hold Box borrows.
    unsafe {
        let mut capacity = 256usize;
        loop {
            if current(hwnd) != Some(target) {
                return Ok(None);
            }
            let mut buffer = Vec::new();
            buffer
                .try_reserve_exact(capacity)
                .map_err(|_| "单元格文本内存不足".to_string())?;
            buffer.resize(capacity, 0u16);
            let mut item = LVITEMW {
                iSubItem: target.cell.column,
                pszText: buffer.as_mut_ptr(),
                cchTextMax: capacity as i32,
                ..Default::default()
            };
            let length = SendMessageW(
                hwnd,
                LVM_GETITEMTEXTW,
                target.cell.row as usize,
                (&mut item as *mut LVITEMW) as LPARAM,
            );
            if current(hwnd) != Some(target) {
                return Ok(None);
            }
            if length < 0 || length as usize >= capacity || buffer[length as usize] != 0 {
                return Err("读取单元格文本失败".into());
            }
            let length = length as usize;
            if length > MAX_TEXT_UNITS {
                return Err("单元格文本超过复制上限（1,048,576 个 UTF-16 单元）".into());
            }
            if length < capacity - 1 {
                return String::from_utf16(&buffer[..length])
                    .map(Some)
                    .map_err(|_| "单元格文本包含无效 UTF-16".into());
            }
            if capacity == MAX_TEXT_UNITS + 2 {
                return Err("单元格文本超过复制上限".into());
            }
            capacity = (capacity * 2).min(MAX_TEXT_UNITS + 2);
        }
    }
}
unsafe fn copy_target(
    hwnd: HWND,
    target: Target,
    write: impl FnOnce(HWND, &str) -> Result<(), String>,
) -> Result<bool, String> {
    // SAFETY: revalidate after extraction and visibility queries; writer only
    // receives an owned, complete string and is never called for absent selection.
    unsafe {
        if !usable(hwnd) || current(hwnd) != Some(target) {
            return Ok(false);
        }
        let Some(text) = read_text(hwnd, target)? else {
            return Ok(false);
        };
        if !usable(hwnd) || current(hwnd) != Some(target) {
            return Ok(false);
        }
        write(hwnd, &text)?;
        Ok(true)
    }
}
unsafe fn copy_current(
    hwnd: HWND,
    write: impl FnOnce(HWND, &str) -> Result<(), String>,
) -> Result<bool, String> {
    // SAFETY: caller is on owner thread, and copy_target revalidates the snapshot.
    unsafe {
        match current(hwnd) {
            Some(target) => copy_target(hwnd, target, write),
            None => Ok(false),
        }
    }
}
fn feedback(result: Result<bool, String>) {
    if result.is_err() {
        // SAFETY: MessageBeep has no window/pointer preconditions. No success UI.
        unsafe { MessageBeep(0x30) };
    }
}

struct Context {
    point: POINT,
    target: Option<Target>,
}
unsafe fn context(hwnd: HWND, l: LPARAM) -> Option<Context> {
    // SAFETY: synchronous owner-thread geometry/selection calls; keyboard menu
    // coordinates are derived from the active cell, not the last cursor position.
    unsafe {
        if !usable(hwnd) {
            return None;
        }
        let mut point;
        // Both sign-extended -1 and MAKELPARAM(-1, -1) occur on Win64.
        if l as u32 == u32::MAX {
            if let Some(target) = current(hwnd) {
                reveal(hwnd, target, true);
            }
            let target = current(hwnd);
            let mut client = RECT::default();
            GetClientRect(hwnd, &mut client);
            point =
                target
                    .and_then(|t| cell_rect(hwnd, t.cell))
                    .map_or(POINT { x: 4, y: 4 }, |r| POINT {
                        x: r.left.max(client.left).min((client.right - 1).max(0)),
                        y: r.bottom.min(client.bottom).max(client.top),
                    });
        } else {
            point = message_point(l);
            ScreenToClient(hwnd, &mut point);
            let value = interaction(hwnd)?;
            match hit_cell(hwnd, point) {
                Some(cell) => select_cell(hwnd, value.stamp(), cell),
                None => activate(hwnd, value.stamp(), None),
            }
        }
        ClientToScreen(hwnd, &mut point);
        Some(Context {
            point,
            target: current(hwnd),
        })
    }
}
unsafe fn popup_menu(enabled: bool) -> Result<Menu, String> {
    // SAFETY: Menu owns the native popup, AppendMenuW copies the static caption.
    unsafe {
        let menu = Menu::new()?;
        let flags = MF_STRING | if enabled { MF_ENABLED } else { MF_GRAYED };
        if AppendMenuW(
            menu.0,
            flags,
            COPY_CELL,
            windows_sys::w!("复制单元格\tCtrl+C"),
        ) == 0
        {
            return Err(last_error("添加单元格复制菜单"));
        }
        Ok(menu)
    }
}
unsafe fn show_context(hwnd: HWND, l: LPARAM) -> Result<bool, String> {
    // SAFETY: TrackPopupMenu runs a nested message loop. Context/Target contain
    // values only, no Box pointer/reference. A changed/destroyed/reused HWND or
    // refreshed table cannot use the old menu target after tracking returns.
    unsafe {
        let Some(context) = context(hwnd, l) else {
            return Ok(false);
        };
        let menu = popup_menu(context.target.is_some())?;
        let command = TrackPopupMenu(
            menu.0,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            context.point.x,
            context.point.y,
            0,
            hwnd,
            null(),
        );
        if command as usize == COPY_CELL
            && let Some(target) = context.target
        {
            return copy_target(hwnd, target, super::overview::copy);
        }
        Ok(false)
    }
}

/// Extra custom-draw flags and a postpaint-only two-tone active-cell frame.
/// `colors` supplies dark text/selection background; None uses system colors,
/// including high-contrast preferences. Never changes native text/fill/state.
///
/// # Safety
/// `draw` is a copied NMLVCUSTOMDRAW from the sender's owner thread; its HDC is
/// borrowed for this notification only. control_theme is the sole dispatcher.
pub(super) unsafe fn custom_draw(draw: NMLVCUSTOMDRAW, colors: Option<(u32, u32)>) -> u32 {
    // SAFETY: notification provenance is checked by control_theme. All state is
    // copied before reentrant messages, and DC state is restored before returning.
    unsafe {
        let hwnd = draw.nmcd.hdr.hwndFrom;
        if interaction(hwnd).is_none() {
            return CDRF_DODEFAULT;
        }
        match draw.nmcd.dwDrawStage {
            CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW,
            CDDS_ITEMPREPAINT => CDRF_NOTIFYSUBITEMDRAW,
            stage if stage == CDDS_ITEMPREPAINT | CDDS_SUBITEM => CDRF_NOTIFYPOSTPAINT,
            stage if stage == CDDS_ITEMPOSTPAINT | CDDS_SUBITEM => {
                let Some(target) = current(hwnd) else {
                    return CDRF_DODEFAULT;
                };
                if draw.dwItemType != LVCDI_ITEM
                    || draw.nmcd.dwItemSpec != target.cell.row as usize
                    || draw.iSubItem != target.cell.column
                    || draw.nmcd.hdc.is_null()
                    || !usable(hwnd)
                {
                    return CDRF_DODEFAULT;
                }
                let Some(mut rect) = cell_rect(hwnd, target.cell) else {
                    return CDRF_DODEFAULT;
                };
                let mut client = RECT::default();
                if GetClientRect(hwnd, &mut client) == 0 || current(hwnd) != Some(target) {
                    return CDRF_DODEFAULT;
                }
                let dc = draw.nmcd.hdc;
                let saved = SaveDC(dc);
                if saved == 0 {
                    return CDRF_DODEFAULT;
                }
                IntersectClipRect(dc, client.left, client.top, client.right, client.bottom);
                let (outer, inner) = colors
                    .unwrap_or_else(|| (GetSysColor(COLOR_WINDOWTEXT), GetSysColor(COLOR_WINDOW)));
                let brush = GetStockObject(DC_BRUSH) as HBRUSH;
                let width = GetDpiForWindow(hwnd).clamp(96, 768).div_ceil(96) as i32;
                for color in [outer, inner] {
                    SetDCBrushColor(dc, color);
                    for _ in 0..width {
                        if rect.right <= rect.left || rect.bottom <= rect.top {
                            break;
                        }
                        FrameRect(dc, &rect, brush);
                        rect.left += 1;
                        rect.top += 1;
                        rect.right -= 1;
                        rect.bottom -= 1;
                    }
                }
                RestoreDC(dc, saved);
                CDRF_DODEFAULT
            }
            _ => CDRF_DODEFAULT,
        }
    }
}

fn resets_cells(message: u32) -> bool {
    matches!(
        message,
        WM_STYLECHANGED
            | LVM_DELETEALLITEMS
            | LVM_DELETEITEM
            | LVM_DELETECOLUMN
            | LVM_INSERTITEMA
            | LVM_INSERTITEMW
            | LVM_INSERTCOLUMNA
            | LVM_INSERTCOLUMNW
            | LVM_SORTITEMS
            | LVM_SORTITEMSEX
            | LVM_SETITEMCOUNT
            | LVM_SETVIEW
            | LVM_SETITEMA
            | LVM_SETITEMW
            | LVM_SETITEMTEXTA
            | LVM_SETITEMTEXTW
            | LVM_ENABLEGROUPVIEW
            | LVM_INSERTGROUP
            | LVM_INSERTGROUPSORTED
            | LVM_REMOVEGROUP
            | LVM_REMOVEALLGROUPS
            | LVM_SETGROUPINFO
            | LVM_MOVEGROUP
            | LVM_MOVEITEMTOGROUP
            | LVM_SORTGROUPS
    )
}
unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: installed only on a validated owner-thread ListView. Never keep a
    // Box borrow across DefSubclassProc/SendMessage/TrackPopupMenu. Reacquire by
    // proc/id after callbacks; NCDESTROY may have freed `data` in a nested call.
    unsafe {
        if message == WM_NCDESTROY {
            // Free only after removal so an unexpected helper failure cannot
            // leave a callback pointing at freed memory. Normally removal here
            // succeeds; free before forwarding to potentially reentrant code.
            if RemoveWindowSubclass(hwnd, Some(procedure), id) != 0 {
                drop(Box::from_raw(data as *mut Interaction));
            }
            return DefSubclassProc(hwnd, message, w, l);
        }
        let compositing_only =
            message == WM_STYLECHANGED && w == GWL_EXSTYLE as WPARAM && l != 0 && {
                let style = &*(l as *const STYLESTRUCT);
                (style.styleOld ^ style.styleNew) & !WS_EX_COMPOSITED == 0
            };
        if resets_cells(message) && !compositing_only {
            let Some(mut value) = interaction(hwnd) else {
                return DefSubclassProc(hwnd, message, w, l);
            };
            value.column = None;
            value.revision = value.revision.wrapping_add(1);
            value.changing = value.changing.saturating_add(1);
            let instance = value.instance;
            store(hwnd, value);
            let result = DefSubclassProc(hwnd, message, w, l);
            if let Some(mut value) = interaction(hwnd).filter(|v| v.instance == instance) {
                value.changing = value.changing.saturating_sub(1);
                store(hwnd, value);
                InvalidateRect(hwnd, null(), 0);
            }
            return result;
        }
        match message {
            WM_COPY => {
                feedback(copy_current(hwnd, super::overview::copy));
                return 0;
            }
            WM_CONTEXTMENU => {
                feedback(show_context(hwnd, l));
                return 0;
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let ctrl = GetKeyState(VK_CONTROL) < 0;
                let shift = GetKeyState(VK_SHIFT) < 0;
                let alt = GetKeyState(VK_MENU) < 0;
                if w == b'C' as usize && ctrl && !alt {
                    feedback(copy_current(hwnd, super::overview::copy));
                    return 0;
                }
                if w == VK_APPS || (w == VK_F10 && shift && !ctrl && !alt) {
                    feedback(show_context(hwnd, -1));
                    return 0;
                }
                if (w == VK_LEFT || w == VK_RIGHT) && report(hwnd) {
                    move_column(hwnd, w == VK_RIGHT);
                    return 0;
                }
            }
            WM_KEYUP | WM_SYSKEYUP
                if w == VK_APPS || (w == VK_F10 && GetKeyState(VK_SHIFT) < 0) =>
            {
                return 0;
            }
            WM_CHAR if w == 3 => return 0,
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN => {
                let value = interaction(hwnd);
                let hit = hit_cell(hwnd, message_point(l));
                let result = DefSubclassProc(hwnd, message, w, l);
                if let Some(value) = value {
                    activate(hwnd, value.stamp(), hit.map(|cell| cell.column));
                }
                return result;
            }
            _ => {}
        }
        let result = DefSubclassProc(hwnd, message, w, l);
        if matches!(
            message,
            WM_KEYDOWN
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_ENABLE
                | WM_SHOWWINDOW
                | LVM_SETITEMSTATE
                | LVM_SETSELECTIONMARK
                | LVM_SETCOLUMNORDERARRAY
                | LVM_SETCOLUMNWIDTH
                | LVM_SETCOLUMNA
                | LVM_SETCOLUMNW
        ) {
            InvalidateRect(hwnd, null(), 0);
        }
        result
    }
}

#[cfg(test)]
mod tests;
