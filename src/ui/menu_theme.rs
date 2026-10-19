//! Documented owner-draw fallback for the title/system menu only. HMENU still
//! owns navigation, availability, radio state and command dispatch. Light and
//! high-contrast modes restore native items, strings, bitmaps and backgrounds.
use super::theme::{Palette, Resolved};
use crate::windows::{Font, last_error, scale};
use std::{mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Globalization::{CSTR_EQUAL, CompareStringOrdinal},
    Graphics::Gdi::*,
    UI::{Controls::*, HiDpi::*, WindowsAndMessaging::*},
};

const TAG: usize = 0x4152_0000;
const MAX_ITEMS: usize = 256;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct MenuTheme(Option<Style>);

impl MenuTheme {
    /// Owner thread only; no Rust borrow may cross the menu tracking loop.
    pub(super) unsafe fn refresh(
        &mut self,
        hwnd: HWND,
        dpi: u32,
        kind: Resolved,
        palette: Palette,
    ) -> Result<(), String> {
        // SAFETY: The host supplies its live owner-thread HWND. Menu mutation
        // does not dispatch application messages; EndMenu only requests exit.
        unsafe {
            if kind != Resolved::Dark {
                if self.0.is_some() {
                    EndMenu();
                    self.0 = None;
                }
                return Ok(());
            }
            let key = Key::new(dpi, palette);
            if self.0.as_ref().is_some_and(|style| style.key == key) {
                return Ok(());
            }
            if self.0.is_some() {
                EndMenu();
                self.0 = None;
            }
            let root = GetSystemMenu(hwnd, 0);
            if root.is_null() {
                return Err(last_error("获取深色标题菜单"));
            }
            let mut style = Style::new(hwnd, root, key, palette)?;
            style.install()?;
            self.0 = Some(style);
            Ok(())
        }
    }

    /// Only call for actual owner-window messages, before generic control drawing.
    pub(super) unsafe fn message(&self, message: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
        let style = self.0.as_ref()?;
        // SAFETY: Win32 supplies valid borrowed message structures. itemData is
        // a checked scalar index, never a pointer into relocatable Rust storage.
        unsafe {
            match message {
                WM_MEASUREITEM if l != 0 => {
                    let item = &mut *(l as *mut MEASUREITEMSTRUCT);
                    if item.CtlType != ODT_MENU {
                        return None;
                    }
                    let entry = style.entry(item.itemData)?;
                    let dc = GetDC(style.hwnd);
                    let (width, height) = style.measure(dc, entry);
                    if !dc.is_null() {
                        ReleaseDC(style.hwnd, dc);
                    }
                    item.itemWidth = width as u32;
                    item.itemHeight = height as u32;
                    Some(1)
                }
                WM_DRAWITEM if l != 0 => {
                    let item = &*(l as *const DRAWITEMSTRUCT);
                    if item.CtlType != ODT_MENU {
                        return None;
                    }
                    let entry = style.entry(item.itemData)?;
                    if !std::ptr::eq(item.hwndItem, entry.menu) {
                        return None;
                    }
                    style.draw(item, entry);
                    Some(1)
                }
                WM_MENUCHAR => style.menu_char(l as HMENU, w as u16),
                _ => None,
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    dpi: u32,
    colors: [u32; 5],
}
impl Key {
    fn new(dpi: u32, p: Palette) -> Self {
        Self {
            dpi: dpi.max(96),
            colors: [p.surface, p.text, p.disabled, p.border, p.accent_surface],
        }
    }
}

struct Entry {
    menu: HMENU,
    position: u32,
    id: u32,
    child: HMENU,
    original_data: usize,
    original_bitmap: HBITMAP,
    text: Vec<u16>,
    separator: bool,
    installed: bool,
}
struct Background {
    menu: HMENU,
    original: HBRUSH,
    installed: bool,
}
struct Style {
    hwnd: HWND,
    key: Key,
    palette: Palette,
    font: Font,
    bold: Font,
    brush: HBRUSH,
    entries: Vec<Entry>,
    backgrounds: Vec<Background>,
}

impl Style {
    unsafe fn new(hwnd: HWND, root: HMENU, key: Key, palette: Palette) -> Result<Self, String> {
        // SAFETY: Public NONCLIENTMETRICS ABI and owned fonts/brush. All
        // allocations finish before changing the borrowed native menu hierarchy.
        unsafe {
            let mut metrics = NONCLIENTMETRICSW {
                cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
                ..Default::default()
            };
            if SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS,
                metrics.cbSize,
                (&mut metrics as *mut NONCLIENTMETRICSW).cast(),
                0,
                key.dpi,
            ) == 0
            {
                return Err(last_error("读取菜单字体"));
            }
            let normal = CreateFontIndirectW(&metrics.lfMenuFont);
            if normal.is_null() {
                return Err(last_error("创建菜单字体"));
            }
            let font = Font(normal);
            metrics.lfMenuFont.lfWeight = FW_BOLD as i32;
            let bold = CreateFontIndirectW(&metrics.lfMenuFont);
            if bold.is_null() {
                return Err(last_error("创建默认菜单项字体"));
            }
            let bold = Font(bold);
            let brush = CreateSolidBrush(palette.surface);
            if brush.is_null() {
                return Err(last_error("创建菜单背景"));
            }
            let mut style = Self {
                hwnd,
                key,
                palette,
                font,
                bold,
                brush,
                entries: Vec::new(),
                backgrounds: Vec::new(),
            };
            style.collect(root, 0)?;
            Ok(style)
        }
    }

    unsafe fn collect(&mut self, menu: HMENU, depth: usize) -> Result<(), String> {
        // SAFETY: Synchronous bounded traversal of live menus on their owner
        // thread. MIIM_STRING copies text; no application-defined data is read.
        unsafe {
            if depth > 8 || self.backgrounds.iter().any(|b| b.menu == menu) {
                return Err("标题菜单层级无效。".into());
            }
            let mut background = MENUINFO {
                cbSize: size_of::<MENUINFO>() as u32,
                fMask: MIM_BACKGROUND,
                ..Default::default()
            };
            if GetMenuInfo(menu, &mut background) == 0 {
                return Err(last_error("读取菜单背景"));
            }
            self.backgrounds.push(Background {
                menu,
                original: background.hbrBack,
                installed: false,
            });
            let count = GetMenuItemCount(menu);
            if count < 0 || self.entries.len() + count as usize > MAX_ITEMS {
                return Err("标题菜单项目数量无效。".into());
            }
            for position in 0..count as u32 {
                let original = item_info(
                    menu,
                    position,
                    MIIM_FTYPE | MIIM_ID | MIIM_SUBMENU | MIIM_DATA | MIIM_BITMAP,
                )?;
                // Do not take over a foreign owner-draw item's data or callbacks.
                if original.fType & MFT_OWNERDRAW != 0 {
                    return Err("标题菜单已包含其他自绘项目。".into());
                }
                // GetMenuItemInfo may clear fields outside fMask. Keep the
                // identity/ownership snapshot separate from the text queries.
                let mut info = MENUITEMINFOW {
                    cbSize: size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_STRING,
                    ..Default::default()
                };
                if GetMenuItemInfoW(menu, position, 1, &mut info) == 0 || info.cch > 4096 {
                    return Err(last_error("读取菜单文字长度"));
                }
                let mut text = vec![0; info.cch as usize + 1];
                info.dwTypeData = text.as_mut_ptr();
                info.cch = text.len() as u32;
                if GetMenuItemInfoW(menu, position, 1, &mut info) == 0 {
                    return Err(last_error("读取菜单文字"));
                }
                text.truncate(info.cch as usize);
                self.entries.push(Entry {
                    menu,
                    position,
                    id: original.wID,
                    child: original.hSubMenu,
                    original_data: original.dwItemData,
                    original_bitmap: original.hbmpItem,
                    text,
                    separator: original.fType & MFT_SEPARATOR != 0,
                    installed: false,
                });
                if !original.hSubMenu.is_null() {
                    self.collect(original.hSubMenu, depth + 1)?;
                }
            }
            Ok(())
        }
    }

    unsafe fn install(&mut self) -> Result<(), String> {
        // SAFETY: All saved menus/items are still live. A partial failure is
        // rolled back by Drop, without modifying IDs, strings, states or children.
        unsafe {
            for (index, entry) in self.entries.iter_mut().enumerate() {
                let mut info = item_info(entry.menu, entry.position, MIIM_FTYPE)?;
                info.fMask = MIIM_FTYPE | MIIM_DATA | MIIM_BITMAP;
                info.fType |= MFT_OWNERDRAW;
                info.dwItemData = TAG + index;
                info.hbmpItem = null_mut();
                if SetMenuItemInfoW(entry.menu, entry.position, 1, &info) == 0 {
                    return Err(last_error("设置深色菜单项目"));
                }
                entry.installed = true;
            }
            for background in &mut self.backgrounds {
                let info = MENUINFO {
                    cbSize: size_of::<MENUINFO>() as u32,
                    fMask: MIM_BACKGROUND,
                    hbrBack: self.brush,
                    ..Default::default()
                };
                if SetMenuInfo(background.menu, &info) == 0 {
                    return Err(last_error("设置深色菜单背景"));
                }
                background.installed = true;
            }
            Ok(())
        }
    }

    fn entry(&self, data: usize) -> Option<&Entry> {
        self.entries.get(data.checked_sub(TAG)?)
    }

    unsafe fn dimensions(&self, dc: HDC, entry: &Entry) -> (i32, i32, i32) {
        // SAFETY: Caller saved the borrowed DC and selected a live owned font.
        unsafe {
            let mut label_width = 0;
            let mut shortcut_width = 0;
            let mut height = 0;
            for row in self
                .entries
                .iter()
                .filter(|row| row.menu == entry.menu && !row.separator)
            {
                let (label, shortcut) = split_text(&row.text);
                let label = text_size(dc, label, false);
                let shortcut = text_size(dc, shortcut, true);
                label_width = label_width.max(label.right);
                shortcut_width = shortcut_width.max(shortcut.right);
                height = height.max(label.bottom).max(shortcut.bottom);
            }
            (label_width, shortcut_width, height)
        }
    }

    unsafe fn measure(&self, dc: HDC, entry: &Entry) -> (i32, i32) {
        let dpi = self.key.dpi;
        if entry.separator {
            return (scale(176, dpi), scale(9, dpi));
        }
        // SAFETY: A nonnull borrowed DC is saved/restored before its release.
        unsafe {
            let Some(_saved) = SavedDc::new(dc) else {
                return (scale(320, dpi), scale(32, dpi));
            };
            SelectObject(dc, self.bold.0);
            let (label, shortcut, height) = self.dimensions(dc, entry);
            (
                scale(176, dpi)
                    .max(label + shortcut + scale(if shortcut > 0 { 80 } else { 60 }, dpi)),
                GetSystemMetricsForDpi(SM_CYMENU, dpi).max(height + scale(10, dpi)),
            )
        }
    }

    unsafe fn draw(&self, item: &DRAWITEMSTRUCT, entry: &Entry) {
        // SAFETY: WM_DRAWITEM's live borrowed DC and rect. Explicit clipping and
        // SaveDC restore font, colors, stock objects and clip on every exit.
        unsafe {
            let dc = item.hDC;
            let Some(_saved) = SavedDc::new(dc) else {
                return;
            };
            let rect = item.rcItem;
            IntersectClipRect(dc, rect.left, rect.top, rect.right, rect.bottom);
            fill(dc, rect, self.palette.surface);
            let dpi = self.key.dpi;
            if entry.separator {
                let y = (rect.top + rect.bottom) / 2;
                fill(
                    dc,
                    RECT {
                        left: rect.left + scale(10, dpi),
                        top: y,
                        right: rect.right - scale(10, dpi),
                        bottom: y + scale(1, dpi),
                    },
                    self.palette.border,
                );
                return;
            }
            let disabled = item.itemState & (ODS_DISABLED | ODS_GRAYED) != 0;
            let selected = item.itemState & ODS_SELECTED != 0;
            if selected {
                fill(
                    dc,
                    RECT {
                        left: rect.left + scale(3, dpi),
                        top: rect.top + 1,
                        right: rect.right - scale(3, dpi),
                        bottom: rect.bottom - 1,
                    },
                    self.palette.accent_surface,
                );
            }
            let color = if disabled {
                self.palette.disabled
            } else {
                self.palette.text
            };
            SetTextColor(dc, color);
            SetBkMode(dc, TRANSPARENT as i32);
            SelectObject(
                dc,
                if item.itemState & ODS_DEFAULT != 0 {
                    self.bold.0
                } else {
                    self.font.0
                },
            );
            let (label, shortcut) = split_text(&entry.text);
            let (_, shortcut_width, _) = self.dimensions(dc, entry);
            let mut label_rect = RECT {
                left: rect.left + scale(32, dpi),
                right: rect.right - scale(24, dpi),
                ..rect
            };
            if shortcut_width > 0 {
                label_rect.right -= shortcut_width + scale(20, dpi);
            }
            let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;
            DrawTextW(
                dc,
                label.as_ptr(),
                label.len() as i32,
                &mut label_rect,
                flags
                    | if item.itemState & ODS_NOACCEL != 0 {
                        DT_HIDEPREFIX
                    } else {
                        0
                    },
            );
            if !shortcut.is_empty() {
                let mut shortcut_rect = RECT {
                    left: label_rect.right + scale(20, dpi),
                    right: rect.right - scale(24, dpi),
                    ..rect
                };
                DrawTextW(
                    dc,
                    shortcut.as_ptr(),
                    shortcut.len() as i32,
                    &mut shortcut_rect,
                    flags | DT_RIGHT | DT_NOPREFIX,
                );
            }
            SelectObject(dc, GetStockObject(DC_PEN));
            SetDCPenColor(dc, color);
            SelectObject(dc, GetStockObject(NULL_BRUSH));
            let x = rect.left + scale(16, dpi);
            let y = (rect.top + rect.bottom) / 2;
            if item.itemState & ODS_CHECKED != 0 {
                let radio = item_info(entry.menu, entry.position, MIIM_FTYPE)
                    .is_ok_and(|info| info.fType & MFT_RADIOCHECK != 0);
                if radio {
                    SelectObject(dc, GetStockObject(DC_BRUSH));
                    SetDCBrushColor(dc, color);
                    let r = scale(3, dpi);
                    Ellipse(dc, x - r, y - r, x + r + 1, y + r + 1);
                } else {
                    stroke(
                        dc,
                        &[
                            (x - scale(4, dpi), y),
                            (x - scale(1, dpi), y + scale(3, dpi)),
                            (x + scale(5, dpi), y - scale(4, dpi)),
                        ],
                    );
                }
            } else {
                system_glyph(dc, entry.id, x, y, dpi);
            }
            if !entry.child.is_null() {
                let x = rect.right - scale(12, dpi);
                stroke(
                    dc,
                    &[
                        (x - scale(2, dpi), y - scale(4, dpi)),
                        (x + scale(2, dpi), y),
                        (x - scale(2, dpi), y + scale(4, dpi)),
                    ],
                );
            }
        }
    }

    unsafe fn menu_char(&self, menu: HMENU, key: u16) -> Option<LRESULT> {
        if !self
            .backgrounds
            .iter()
            .any(|background| background.menu == menu)
        {
            return None;
        }
        // SAFETY: This menu is part of the saved native hierarchy. Read current
        // state, not a stale availability snapshot; disabled mnemonics do nothing.
        unsafe {
            let mut matches = Vec::new();
            let mut highlighted = None;
            for entry in self
                .entries
                .iter()
                .filter(|entry| entry.menu == menu && !entry.separator)
            {
                let info = item_info(menu, entry.position, MIIM_STATE).ok()?;
                if info.fState & MFS_HILITE != 0 {
                    highlighted = Some(entry.position);
                }
                if info.fState & MFS_DISABLED == 0 && has_mnemonic(&entry.text, key) {
                    matches.push(entry.position);
                }
            }
            let first = *matches.first()?;
            let (position, action) = if matches.len() == 1 {
                (first, MNC_EXECUTE)
            } else {
                (
                    matches
                        .iter()
                        .copied()
                        .find(|position| highlighted.is_none_or(|current| *position > current))
                        .unwrap_or(first),
                    MNC_SELECT,
                )
            };
            Some(((action << 16) | position) as LRESULT)
        }
    }
}

impl Drop for Style {
    fn drop(&mut self) {
        // SAFETY: UI-thread teardown/rollback. Restore only our tagged entries;
        // preserve live enabled/check/radio/default state and all native strings.
        // Never delete a brush that a surviving menu could still reference.
        unsafe {
            for (index, entry) in self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.installed)
            {
                if let Ok(mut info) = item_info(
                    entry.menu,
                    entry.position,
                    MIIM_FTYPE | MIIM_DATA | MIIM_ID | MIIM_SUBMENU,
                ) && info.dwItemData == TAG + index
                    && info.wID == entry.id
                    && info.hSubMenu == entry.child
                {
                    info.fMask = MIIM_FTYPE | MIIM_DATA | MIIM_BITMAP;
                    info.fType &= !MFT_OWNERDRAW;
                    info.dwItemData = entry.original_data;
                    info.hbmpItem = entry.original_bitmap;
                    SetMenuItemInfoW(entry.menu, entry.position, 1, &info);
                }
            }
            let mut can_delete = true;
            for background in self
                .backgrounds
                .iter()
                .filter(|background| background.installed)
            {
                if IsMenu(background.menu) == 0 {
                    continue;
                }
                let mut info = MENUINFO {
                    cbSize: size_of::<MENUINFO>() as u32,
                    fMask: MIM_BACKGROUND,
                    ..Default::default()
                };
                if GetMenuInfo(background.menu, &mut info) == 0 {
                    can_delete = false;
                } else if info.hbrBack == self.brush {
                    info.hbrBack = background.original;
                    if SetMenuInfo(background.menu, &info) == 0 {
                        can_delete = false;
                    }
                }
            }
            if can_delete {
                DeleteObject(self.brush);
            }
        }
    }
}

unsafe fn item_info(menu: HMENU, position: u32, mask: u32) -> Result<MENUITEMINFOW, String> {
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: mask,
        ..Default::default()
    };
    // SAFETY: The caller supplies a live HMENU; failure is an ordinary error.
    if unsafe { GetMenuItemInfoW(menu, position, 1, &mut info) } == 0 {
        Err(last_error("读取菜单项目"))
    } else {
        Ok(info)
    }
}

fn split_text(text: &[u16]) -> (&[u16], &[u16]) {
    text.iter()
        .position(|ch| *ch == b'\t' as u16)
        .map_or((text, &[]), |index| (&text[..index], &text[index + 1..]))
}
fn has_mnemonic(text: &[u16], key: u16) -> bool {
    let mut chars = split_text(text).0.iter().copied();
    while let Some(ch) = chars.next() {
        if ch == b'&' as u16
            && let Some(next) = chars.next()
            && next != b'&' as u16
            // SAFETY: Compare exactly one valid UTF-16 unit from each stack
            // value. Windows' Unicode case mapping matches native menu semantics.
            && (next == key || unsafe { CompareStringOrdinal(&next, 1, &key, 1, 1) } == CSTR_EQUAL)
        {
            return true;
        }
    }
    false
}

struct SavedDc(HDC, i32);
impl SavedDc {
    unsafe fn new(dc: HDC) -> Option<Self> {
        if dc.is_null() {
            return None;
        }
        // SAFETY: The caller supplies a live borrowed DC.
        let level = unsafe { SaveDC(dc) };
        (level != 0).then_some(Self(dc, level))
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: The borrowed DC outlives this guard and its successful SaveDC.
        unsafe { RestoreDC(self.0, self.1) };
    }
}
unsafe fn text_size(dc: HDC, text: &[u16], literal: bool) -> RECT {
    let mut rect = RECT::default();
    if text.is_empty() {
        return rect;
    }
    // SAFETY: Sized UTF-16 slice and a writable rect, valid borrowed DC/font.
    unsafe {
        DrawTextW(
            dc,
            text.as_ptr(),
            text.len() as i32,
            &mut rect,
            DT_CALCRECT | DT_SINGLELINE | if literal { DT_NOPREFIX } else { 0 },
        )
    };
    rect
}
unsafe fn fill(dc: HDC, rect: RECT, color: u32) {
    // SAFETY: Borrowed saved DC; stock objects are never deleted.
    unsafe {
        SetDCBrushColor(dc, color);
        FillRect(dc, &rect, GetStockObject(DC_BRUSH));
    }
}
unsafe fn stroke(dc: HDC, points: &[(i32, i32)]) {
    let points: Vec<POINT> = points.iter().map(|&(x, y)| POINT { x, y }).collect();
    // SAFETY: Sized point slice, borrowed saved DC and selected stock pen.
    unsafe { Polyline(dc, points.as_ptr(), points.len() as i32) };
}
unsafe fn system_glyph(dc: HDC, command: u32, x: i32, y: i32, dpi: u32) {
    let r = scale(4, dpi);
    // SAFETY: Caller clips/saves the DC and selects a stock pen/null brush.
    unsafe {
        match command & 0xfff0 {
            SC_CLOSE => {
                stroke(dc, &[(x - r, y - r), (x + r, y + r)]);
                stroke(dc, &[(x - r, y + r), (x + r, y - r)]);
            }
            SC_MINIMIZE => stroke(
                dc,
                &[(x - r, y + scale(2, dpi)), (x + r, y + scale(2, dpi))],
            ),
            SC_MAXIMIZE => {
                Rectangle(dc, x - r, y - r, x + r + 1, y + r + 1);
            }
            SC_RESTORE => {
                Rectangle(
                    dc,
                    x - r + scale(2, dpi),
                    y - r,
                    x + r + 1,
                    y + r - scale(1, dpi),
                );
                Rectangle(
                    dc,
                    x - r,
                    y - r + scale(2, dpi),
                    x + r - scale(1, dpi),
                    y + r + 1,
                );
            }
            SC_MOVE => {
                stroke(dc, &[(x - r, y), (x + r, y)]);
                stroke(dc, &[(x, y - r), (x, y + r)]);
            }
            SC_SIZE => stroke(dc, &[(x - r, y + r), (x + r, y - r)]),
            _ => {}
        }
    }
}
