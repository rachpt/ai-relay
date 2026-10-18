//! Native caption-icon menu. The host routes WM_SYSCOMMAND through `action`,
//! updates availability on WM_INITMENUPOPUP, and calls `show` for F10/Alt+M.
//! No custom non-client frame, controller access, file I/O, or implicit actions.
//!
//! The caption icon retains Windows' system-menu accessibility, Alt+Space and
//! double-click-to-close behavior. The host may intercept WM_NCLBUTTONDBLCLK /
//! HTSYSMENU if it deliberately wants to suppress the latter. Native popup menus
//! retain system styling; application dark mode does not force dark popups.
use super::theme::{Palette, Resolved};
use crate::windows::{Menu, last_error, scale, wide};
use std::{mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, COLOR_CAPTIONTEXT, ClientToScreen, CreateBitmap,
        CreateDIBSection, DIB_RGB_COLORS, DeleteObject, GetSysColor, HBITMAP,
    },
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

pub(super) const OPEN_DIR: u32 = 0x3000;
pub(super) const EXPORT_CONFIG: u32 = 0x3010;
pub(super) const IMPORT_CONFIG: u32 = 0x3020;
pub(super) const EXIT: u32 = 0x3030;
pub(super) const START: u32 = 0x3040;
pub(super) const STOP: u32 = 0x3050;
pub(super) const RESTART: u32 = 0x3060;
pub(super) const VIEW_OVERVIEW: u32 = 0x3070;
pub(super) const VIEW_HISTORY: u32 = 0x3080;
pub(super) const VIEW_LOGS: u32 = 0x3090;
pub(super) const VIEW_SETTINGS: u32 = 0x30a0;
pub(super) const ABOUT: u32 = 0x30b0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    OpenDir,
    ExportConfig,
    ImportConfig,
    Exit,
    Start,
    Stop,
    Restart,
    ViewOverview,
    ViewHistory,
    ViewLogs,
    ViewSettings,
    About,
}

/// Accept the low word of WM_SYSCOMMAND's wParam, including its reserved bits.
/// Unknown IDs (especially native SC_* commands) belong to DefWindowProcW.
pub(super) fn action(command: u32) -> Option<Action> {
    // Do not truncate an arbitrary larger command into our application range.
    match command & !0xf {
        OPEN_DIR => Some(Action::OpenDir),
        EXPORT_CONFIG => Some(Action::ExportConfig),
        IMPORT_CONFIG => Some(Action::ImportConfig),
        EXIT => Some(Action::Exit),
        START => Some(Action::Start),
        STOP => Some(Action::Stop),
        RESTART => Some(Action::Restart),
        VIEW_OVERVIEW => Some(Action::ViewOverview),
        VIEW_HISTORY => Some(Action::ViewHistory),
        VIEW_LOGS => Some(Action::ViewLogs),
        VIEW_SETTINGS => Some(Action::ViewSettings),
        ABOUT => Some(Action::About),
        _ => None,
    }
}

/// The host supplies actual controller availability, not inferred menu state.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Availability {
    pub start: bool,
    pub stop: bool,
    pub restart: bool,
    pub export: bool,
    pub import: bool,
    /// Matches the existing four native tabs: overview, history, logs, settings.
    pub active_page: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct IconKey {
    dpi: u32,
    kind: Resolved,
    // High-contrast scheme colors can change without changing Resolved::Contrast.
    color: u32,
}

pub(super) struct TitleMenu {
    hwnd: HWND,
    icon: Option<CaptionIcon>,
    original_small: HICON,
    key: Option<IconKey>,
    install_caption_icon: bool,
}
impl Default for TitleMenu {
    fn default() -> Self {
        Self {
            hwnd: std::ptr::null_mut(),
            icon: None,
            original_small: std::ptr::null_mut(),
            key: None,
            // Unit tests exercise the caption-icon path; the app disables it and
            // uses a client-side menu button so taskbar keeps the product icon.
            install_caption_icon: true,
        }
    }
}

impl TitleMenu {
    pub(super) fn set_install_caption_icon(&mut self, install: bool) {
        self.install_caption_icon = install;
    }
    /// Attach once per window. Only our own insertions are removed on failure;
    /// GetSystemMenu(hwnd, TRUE) is never used to reset somebody else's entries.
    ///
    /// # Safety
    /// Call on hwnd's UI thread, outside a menu tracking loop. Keep this object
    /// until window destruction (or drop it on that thread before destroying hwnd).
    /// The host must not reset/replace the system menu after attachment, or access
    /// this object recursively while handling WM_SETICON / WM_GETICON.
    pub(super) unsafe fn attach(
        &mut self,
        hwnd: HWND,
        dpi: u32,
        kind: Resolved,
        palette: Palette,
    ) -> Result<(), String> {
        if !self.hwnd.is_null() {
            return Err("标题菜单已经附加到窗口。".into());
        }
        // SAFETY: The caller provides the live owner-thread HWND. The root is
        // borrowed; Menu owns only newly allocated, unattached submenus.
        unsafe {
            if IsWindow(hwnd) == 0 {
                return Err("标题菜单需要有效的窗口。".into());
            }
            let root = GetSystemMenu(hwnd, 0);
            if root.is_null() {
                return Err(last_error("获取窗口系统菜单"));
            }
            if !command_menu(root, OPEN_DIR).is_null() {
                return Err("窗口已经包含标题菜单命令。".into());
            }
            // Allocate everything before changing the borrowed root.
            let caption = if self.install_caption_icon {
                let key = icon_key(dpi, kind, palette);
                Some((key, CaptionIcon::new(key)?))
            } else {
                None
            };
            let menus = [
                (
                    "文件(&F)",
                    submenu(&[
                        (OPEN_DIR, "打开数据目录(&O)"),
                        (EXPORT_CONFIG, "导出配置(&E)…"),
                        (IMPORT_CONFIG, "导入配置(&I)…"),
                        (EXIT, "退出(&X)"),
                    ])?,
                ),
                (
                    "代理(&P)",
                    submenu(&[
                        (START, "▶ 启动代理(&S)"),
                        (STOP, "■ 停止代理(&T)"),
                        (RESTART, "↻ 重启代理(&R)"),
                    ])?,
                ),
                (
                    "视图(&V)",
                    submenu(&[
                        (VIEW_OVERVIEW, "● 概览(&O)"),
                        (VIEW_HISTORY, "▥ 统计分析(&H)"),
                        (VIEW_LOGS, "≡ 实时日志(&L)"),
                        (VIEW_SETTINGS, "◆ 设置(&S)"),
                    ])?,
                ),
                ("帮助(&H)", submenu(&[(ABOUT, "关于 AI Relay(&A)…")])?),
            ];
            let mut insertion = Insertion {
                root,
                children: Vec::with_capacity(menus.len()),
                committed: false,
            };
            for (position, (label, menu)) in menus.into_iter().enumerate() {
                if InsertMenuW(
                    root,
                    position as u32,
                    MF_BYPOSITION | MF_POPUP,
                    menu.0 as usize,
                    wide(label).as_ptr(),
                ) == 0
                {
                    return Err(last_error("插入标题菜单"));
                }
                insertion.children.push(menu.0);
                // The root now owns this submenu and all its descendants.
                std::mem::forget(menu);
            }
            // Last fallible insertion: rollback never needs to identify/remove
            // an anonymous separator or touch any of the original system items.
            if InsertMenuW(root, 4, MF_BYPOSITION | MF_SEPARATOR, 0, std::ptr::null()) == 0 {
                return Err(last_error("插入标题菜单分隔线"));
            }
            insertion.committed = true;
            self.hwnd = hwnd;
            if let Some((key, icon)) = caption {
                // WM_SETICON has no failure return; its result is the previous icon,
                // which remains borrowed, not something we may DestroyIcon.
                self.original_small =
                    SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon.0 as isize) as HICON;
                self.icon = Some(icon);
                self.key = Some(key);
            }
            DrawMenuBar(hwnd);
            Ok(())
        }
    }

    /// Regenerate only for DPI/theme/color changes; a failed allocation preserves
    /// the installed icon. High contrast uses the native caption text color.
    ///
    /// # Safety
    /// Same UI-thread, HWND lifetime and non-reentrancy requirements as `attach`.
    pub(super) unsafe fn refresh(
        &mut self,
        hwnd: HWND,
        dpi: u32,
        kind: Resolved,
        palette: Palette,
    ) -> Result<(), String> {
        if self.hwnd.is_null() || self.hwnd != hwnd {
            return Err("标题菜单尚未附加到此窗口。".into());
        }
        if !self.install_caption_icon {
            return Ok(());
        }
        let key = icon_key(dpi, kind, palette);
        if self.key == Some(key) {
            return Ok(());
        }
        let next = CaptionIcon::new(key)?;
        // SAFETY: The caller keeps hwnd live on its UI thread. Both old and new
        // icons remain alive until synchronous WM_SETICON has completed.
        unsafe {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, next.0 as isize);
        }
        self.icon = Some(next);
        self.key = Some(key);
        Ok(())
    }
}

impl Drop for TitleMenu {
    fn drop(&mut self) {
        if let Some(icon) = &self.icon {
            // SAFETY: The owner-thread lifetime contract is established by attach.
            // Restore only if the live window still uses our icon. Never destroy
            // the borrowed previous icon, root, or OS-owned submenu hierarchy.
            unsafe {
                if IsWindow(self.hwnd) != 0
                    && SendMessageW(self.hwnd, WM_GETICON, ICON_SMALL as usize, 0) as HICON
                        == icon.0
                {
                    SendMessageW(
                        self.hwnd,
                        WM_SETICON,
                        ICON_SMALL as usize,
                        self.original_small as isize,
                    );
                }
            }
        }
    }
}

/// # Safety
/// Call on the live window's UI thread; do not hold a State/TitleMenu borrow
/// across the native modal loop. It dispatches messages and may destroy hwnd.
/// WM_INITMENUPOPUP handling may call `update` using a fresh state snapshot.
pub(super) unsafe fn show(hwnd: HWND) -> Result<(), String> {
    // SAFETY: Only local values/borrowed native handles cross the modal loop;
    // there are no Rust references into the host's window state here.
    unsafe {
        let root = GetSystemMenu(hwnd, 0);
        if root.is_null() {
            return Err(last_error("获取窗口系统菜单"));
        }
        let mut window = RECT::default();
        let mut client_origin = POINT::default();
        if GetWindowRect(hwnd, &mut window) == 0 || ClientToScreen(hwnd, &mut client_origin) == 0 {
            return Err(last_error("定位标题菜单"));
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        SetForegroundWindow(hwnd);
        let command = TrackPopupMenuEx(
            root,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON,
            window.left.saturating_add(scale(8, dpi)),
            client_origin.y,
            hwnd,
            std::ptr::null(),
        );
        // A zero selection is normal cancellation. Do not invent an action or
        // retry a popup; callback APIs may also have changed GetLastError.
        if IsWindow(hwnd) == 0 {
            return Ok(());
        }
        PostMessageW(hwnd, WM_NULL, 0, 0);
        if command != 0 && PostMessageW(hwnd, WM_SYSCOMMAND, command as usize, 0) == 0 {
            return Err(last_error("发送标题菜单命令"));
        }
        Ok(())
    }
}

/// # Safety
/// hwnd must be live on this UI thread with an attached title menu. This changes
/// native menu flags only; it never calls the controller or performs the action.
pub(super) unsafe fn update(hwnd: HWND, enabled: Availability) -> Result<(), String> {
    if enabled.active_page > 3 {
        return Err("标题菜单页面索引超出范围。".into());
    }
    // SAFETY: Each leaf's actual owning HMENU is located explicitly, rather than
    // relying on undocumented recursive behavior of EnableMenuItem.
    unsafe {
        let root = GetSystemMenu(hwnd, 0);
        if root.is_null() {
            return Err(last_error("获取窗口系统菜单"));
        }
        for (id, available) in [
            (START, enabled.start),
            (STOP, enabled.stop),
            (RESTART, enabled.restart),
            (EXPORT_CONFIG, enabled.export),
            (IMPORT_CONFIG, enabled.import),
        ] {
            let menu = command_menu(root, id);
            if menu.is_null() {
                return Err("标题菜单命令不存在。".into());
            }
            let flags = MF_BYCOMMAND | if available { MF_ENABLED } else { MF_GRAYED };
            if EnableMenuItem(menu, id, flags) == -1 {
                return Err(last_error("更新标题菜单可用状态"));
            }
        }
        let view = command_menu(root, VIEW_OVERVIEW);
        if view.is_null() {
            return Err("标题菜单视图子菜单不存在。".into());
        }
        if CheckMenuRadioItem(
            view,
            VIEW_OVERVIEW,
            VIEW_SETTINGS,
            VIEW_OVERVIEW + enabled.active_page as u32 * 0x10,
            MF_BYCOMMAND,
        ) == 0
        {
            return Err(last_error("更新标题菜单当前页面"));
        }
        Ok(())
    }
}

fn submenu(items: &[(u32, &str)]) -> Result<Menu, String> {
    let menu = Menu::new()?;
    for &(id, label) in items {
        // SAFETY: This unattached, owned menu copies each terminated label.
        if unsafe { AppendMenuW(menu.0, MF_STRING, id as usize, wide(label).as_ptr()) } == 0 {
            return Err(last_error("添加标题菜单命令"));
        }
    }
    Ok(menu)
}

unsafe fn command_menu(root: HMENU, command: u32) -> HMENU {
    // SAFETY: Traversal is synchronous on the UI thread, with no message dispatch
    // or menu mutation while each borrowed root/child handle is inspected.
    unsafe {
        for position in 0..GetMenuItemCount(root) {
            if GetMenuItemID(root, position) == command {
                return root;
            }
            let child = GetSubMenu(root, position);
            if !child.is_null() {
                let found = command_menu(child, command);
                if !found.is_null() {
                    return found;
                }
            }
        }
        null_mut()
    }
}

struct Insertion {
    root: HMENU,
    children: Vec<HMENU>,
    committed: bool,
}

impl Drop for Insertion {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // SAFETY: No modal loop/callback intervenes in the insertion transaction.
        // Remove by matching our own handles, never by a preexisting item's ID.
        // Destroy only after successful detachment; otherwise the OS still owns it.
        unsafe {
            for &child in self.children.iter().rev() {
                for position in 0..GetMenuItemCount(self.root) {
                    if GetSubMenu(self.root, position) == child {
                        if RemoveMenu(self.root, position as u32, MF_BYPOSITION) != 0 {
                            DestroyMenu(child);
                        }
                        break;
                    }
                }
            }
        }
    }
}

fn icon_key(dpi: u32, kind: Resolved, palette: Palette) -> IconKey {
    let color = if kind == Resolved::Contrast {
        // SAFETY: A predefined system color index needs no handle or pointer.
        unsafe { GetSysColor(COLOR_CAPTIONTEXT) }
    } else {
        palette.text
    };
    IconKey {
        dpi: if dpi == 0 { 96 } else { dpi },
        kind,
        color,
    }
}

struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: These owned bitmaps are never selected into any DC.
        unsafe { DeleteObject(self.0) };
    }
}

struct CaptionIcon(HICON);
impl CaptionIcon {
    fn new(key: IconKey) -> Result<Self, String> {
        // SAFETY: Public metric queries, bounded allocations, and a sized DIB
        // descriptor. Bitmap and icon resources are owned immediately on success.
        unsafe {
            let width = GetSystemMetricsForDpi(SM_CXSMICON, key.dpi).clamp(8, 64);
            let height = GetSystemMetricsForDpi(SM_CYSMICON, key.dpi).clamp(8, 64);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = null_mut();
            let color =
                CreateDIBSection(null_mut(), &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if color.is_null() {
                return Err(last_error("创建标题菜单图标位图"));
            }
            let color = Bitmap(color);
            if bits.is_null() {
                return Err("标题菜单图标位图没有可写像素。".into());
            }
            // 32-bit top-down DIB memory is BGRA. Opaque strokes (alpha 255)
            // and zero transparent pixels are already premultiplied.
            let pixels =
                std::slice::from_raw_parts_mut(bits.cast::<u32>(), (width * height) as usize);
            pixels.fill(0);
            let stride = (width as usize).div_ceil(16) * 2;
            let mut mask = vec![0xffu8; stride * height as usize];
            let rgb = key.color;
            let bgra = 0xff00_0000 | (rgb & 0xff) << 16 | (rgb & 0xff00) | (rgb >> 16 & 0xff);
            let inset = (width / 5).max(1);
            let thickness = (height / 10).max(1);
            for center in [height / 4, height / 2, height * 3 / 4] {
                for y in center - thickness / 2..center - thickness / 2 + thickness {
                    for x in inset..width - inset {
                        pixels[(y * width + x) as usize] = bgra;
                        // CreateBitmap uses bottom-up, WORD-aligned monochrome rows.
                        mask[(height - 1 - y) as usize * stride + x as usize / 8] &=
                            !(0x80 >> (x % 8));
                    }
                }
            }
            let mask = CreateBitmap(width, height, 1, 1, mask.as_ptr().cast());
            if mask.is_null() {
                return Err(last_error("创建标题菜单图标透明掩码"));
            }
            let mask = Bitmap(mask);
            let info = ICONINFO {
                fIcon: 1,
                hbmMask: mask.0,
                hbmColor: color.0,
                ..Default::default()
            };
            // CreateIconIndirect copies the two bitmaps; both wrappers can now drop.
            let icon = CreateIconIndirect(&info);
            if icon.is_null() {
                Err(last_error("创建标题菜单图标"))
            } else {
                Ok(Self(icon))
            }
        }
    }
}
impl Drop for CaptionIcon {
    fn drop(&mut self) {
        // SAFETY: Only icons created by CreateIconIndirect are owned here. The
        // window has stopped using this icon before replacement or owner drop.
        unsafe { DestroyIcon(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetGuiResources(process: windows_sys::Win32::Foundation::HANDLE, flags: u32) -> u32;
    }

    fn resources() -> (u32, u32) {
        // SAFETY: Current-process pseudo-handle and documented GDI / USER flags.
        unsafe {
            let process = windows_sys::Win32::System::Threading::GetCurrentProcess();
            (GetGuiResources(process, 0), GetGuiResources(process, 1))
        }
    }

    const COMMANDS: [u32; 12] = [
        OPEN_DIR,
        EXPORT_CONFIG,
        IMPORT_CONFIG,
        EXIT,
        START,
        STOP,
        RESTART,
        VIEW_OVERVIEW,
        VIEW_HISTORY,
        VIEW_LOGS,
        VIEW_SETTINGS,
        ABOUT,
    ];

    #[test]
    fn commands_are_unique_aligned_and_below_system_range() {
        for (index, command) in COMMANDS.into_iter().enumerate() {
            assert_eq!(command & 0xf, 0);
            assert!(command < 0xf000);
            assert!(!COMMANDS[..index].contains(&command));
            for reserved in 0..16 {
                assert_eq!(action(command | reserved), action(command));
                assert!(action(command | reserved).is_some());
            }
        }
    }

    #[test]
    fn native_and_unknown_commands_are_not_claimed() {
        for command in [0, 2030, 0x2ff0, 0x30c0, 0x1_3000, u32::MAX] {
            assert_eq!(action(command), None);
        }
        for command in [
            SC_RESTORE,
            SC_MOVE,
            SC_SIZE,
            SC_MINIMIZE,
            SC_MAXIMIZE,
            SC_CLOSE,
        ] {
            for reserved in 0..16 {
                assert_eq!(action(command | reserved), None);
            }
        }
    }

    fn palette(text: u32) -> Palette {
        Palette {
            background: 0,
            surface: 0,
            text,
            disabled: 0,
            accent: 0,
            border: 0,
            accent_surface: 0,
        }
    }

    struct Window(HWND);
    impl Window {
        fn new() -> Self {
            // SAFETY: A built-in class and unshown top-level window on this test
            // thread; no application State, I/O, hooks, or message loop is needed.
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide("Title menu test").as_ptr(),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    320,
                    240,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!hwnd.is_null());
            Self(hwnd)
        }
    }
    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: Only this test owns this unshown window on the current thread.
            unsafe { DestroyWindow(self.0) };
        }
    }

    #[test]
    fn native_menu_preserves_system_commands_and_refreshes_owned_icons() {
        let window = Window::new();
        let mut title = TitleMenu::default();
        // SAFETY: The test owns this hidden window and all calls stay on its thread.
        unsafe {
            let original = LoadIconW(null_mut(), IDI_APPLICATION);
            assert!(!original.is_null());
            SendMessageW(window.0, WM_SETICON, ICON_BIG as usize, original as isize);
            SendMessageW(window.0, WM_SETICON, ICON_SMALL as usize, original as isize);
            let root = GetSystemMenu(window.0, 0);
            let count = GetMenuItemCount(root);
            title
                .attach(window.0, 96, Resolved::Light, palette(0x202020))
                .unwrap();
            assert_eq!(GetMenuItemCount(root), count + 5);
            assert!(!command_menu(root, SC_CLOSE).is_null());
            assert!(!command_menu(root, SC_RESTORE).is_null());
            assert!(
                title
                    .attach(window.0, 96, Resolved::Light, palette(0x202020))
                    .is_err()
            );
            let mut second = TitleMenu::default();
            assert!(
                second
                    .attach(window.0, 96, Resolved::Light, palette(0x202020))
                    .is_err()
            );
            assert_eq!(GetMenuItemCount(root), count + 5);
            let first = title.icon.as_ref().unwrap().0;
            title
                .refresh(window.0, 96, Resolved::Light, palette(0x202020))
                .unwrap();
            assert_eq!(title.icon.as_ref().unwrap().0, first);
            let baseline = resources();
            for index in 0..20 {
                let kind = if index % 2 == 0 {
                    Resolved::Dark
                } else {
                    Resolved::Light
                };
                let dpi = [96, 144, 192, 384][index % 4];
                title
                    .refresh(window.0, dpi, kind, palette(0xf0f0f0))
                    .unwrap();
                assert_eq!(
                    SendMessageW(window.0, WM_GETICON, ICON_BIG as usize, 0) as HICON,
                    original
                );
                let mut info = ICONINFO::default();
                assert_ne!(GetIconInfo(title.icon.as_ref().unwrap().0, &mut info), 0);
                let mask = Bitmap(info.hbmMask);
                let color = Bitmap(info.hbmColor);
                drop((mask, color));
            }
            let after = resources();
            assert_eq!(
                after, baseline,
                "GDI/USER resource growth after 20 refreshes"
            );
            title
                .refresh(window.0, 192, Resolved::Contrast, palette(0))
                .unwrap();
            assert_eq!(title.key.unwrap().color, GetSysColor(COLOR_CAPTIONTEXT));
            for page in 0..4 {
                update(
                    window.0,
                    Availability {
                        start: true,
                        stop: false,
                        restart: true,
                        export: true,
                        import: false,
                        active_page: page,
                    },
                )
                .unwrap();
                let proxy = command_menu(root, START);
                assert_eq!(GetMenuState(proxy, START, MF_BYCOMMAND) & MF_GRAYED, 0);
                assert_ne!(GetMenuState(proxy, STOP, MF_BYCOMMAND) & MF_GRAYED, 0);
                let view = command_menu(root, VIEW_OVERVIEW);
                for (index, id) in [VIEW_OVERVIEW, VIEW_HISTORY, VIEW_LOGS, VIEW_SETTINGS]
                    .into_iter()
                    .enumerate()
                {
                    assert_eq!(
                        GetMenuState(view, id, MF_BYCOMMAND) & MF_CHECKED != 0,
                        index == page
                    );
                }
            }
            drop(title);
            assert_eq!(
                SendMessageW(window.0, WM_GETICON, ICON_SMALL as usize, 0) as HICON,
                original
            );
            // Dropping the icon owner must not destroy OS-owned submenus.
            assert!(!command_menu(root, ABOUT).is_null());
        }
    }

    #[test]
    fn failed_insertion_transaction_preserves_preexisting_items() {
        let root = Menu::new().unwrap();
        let original = submenu(&[(SC_CLOSE, "Original")]).unwrap();
        // SAFETY: All menus are unattached and test-owned; insertion takes
        // ownership of child menus exactly as it does for a borrowed system root.
        unsafe {
            assert_ne!(
                AppendMenuW(
                    root.0,
                    MF_POPUP,
                    original.0 as usize,
                    wide("Original").as_ptr()
                ),
                0
            );
            let original_handle = original.0;
            std::mem::forget(original);
            let child = submenu(&[(OPEN_DIR, "Owned")]).unwrap();
            assert_ne!(
                InsertMenuW(
                    root.0,
                    0,
                    MF_BYPOSITION | MF_POPUP,
                    child.0 as usize,
                    wide("Owned").as_ptr()
                ),
                0
            );
            let insertion = Insertion {
                root: root.0,
                children: vec![child.0],
                committed: false,
            };
            std::mem::forget(child);
            drop(insertion);
            assert_eq!(GetMenuItemCount(root.0), 1);
            assert_eq!(GetSubMenu(root.0, 0), original_handle);
            assert!(!command_menu(root.0, SC_CLOSE).is_null());
            assert!(command_menu(root.0, OPEN_DIR).is_null());
        }
    }

    #[test]
    fn client_menu_mode_keeps_window_and_taskbar_icons() {
        let window = Window::new();
        let mut title = TitleMenu::default();
        title.set_install_caption_icon(false);
        // SAFETY: Test-owned hidden window; all calls stay on its thread.
        unsafe {
            let original = LoadIconW(null_mut(), IDI_APPLICATION);
            assert!(!original.is_null());
            SendMessageW(window.0, WM_SETICON, ICON_SMALL as usize, original as isize);
            SendMessageW(window.0, WM_SETICON, ICON_BIG as usize, original as isize);
            title
                .attach(window.0, 96, Resolved::Light, palette(0x202020))
                .unwrap();
            assert!(title.icon.is_none());
            assert_eq!(
                SendMessageW(window.0, WM_GETICON, ICON_SMALL as usize, 0) as HICON,
                original
            );
            assert_eq!(
                SendMessageW(window.0, WM_GETICON, ICON_BIG as usize, 0) as HICON,
                original
            );
            title
                .refresh(window.0, 192, Resolved::Dark, palette(0xf0f0f0))
                .unwrap();
            assert!(title.icon.is_none());
            drop(title);
            assert_eq!(
                SendMessageW(window.0, WM_GETICON, ICON_SMALL as usize, 0) as HICON,
                original
            );
            assert_eq!(
                SendMessageW(window.0, WM_GETICON, ICON_BIG as usize, 0) as HICON,
                original
            );
        }
    }
}
