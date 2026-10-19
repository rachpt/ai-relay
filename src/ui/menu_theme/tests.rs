use super::*;
use crate::{config::ThemeMode, ui::title_menu};

struct Window(HWND);
impl Window {
    fn new() -> Self {
        // SAFETY: Test-owned hidden built-in window on the current test thread.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!("Menu theme tests"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                800,
                600,
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
        // SAFETY: Destroy this test's own hidden HWND, on its creating thread.
        unsafe { DestroyWindow(self.0) };
    }
}
fn palette() -> Palette {
    super::super::theme::Theme::new(ThemeMode::Dark)
        .unwrap()
        .palette
}
unsafe fn background(menu: HMENU) -> HBRUSH {
    let mut info = MENUINFO {
        cbSize: size_of::<MENUINFO>() as u32,
        fMask: MIM_BACKGROUND,
        ..Default::default()
    };
    // SAFETY: Test-owned live menu and sized writable structure.
    unsafe { assert_ne!(GetMenuInfo(menu, &mut info), 0) };
    info.hbrBack
}
unsafe fn text(menu: HMENU, position: u32) -> String {
    let mut buffer = [0u16; 512];
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STRING,
        dwTypeData: buffer.as_mut_ptr(),
        cch: buffer.len() as u32,
        ..Default::default()
    };
    // SAFETY: Test-owned live menu and writable UTF-16 buffer.
    unsafe { assert_ne!(GetMenuItemInfoW(menu, position, 1, &mut info), 0) };
    String::from_utf16(&buffer[..info.cch as usize]).unwrap()
}

#[test]
fn native_hierarchy_strings_bitmaps_data_and_live_flags_survive_dark_light_contrast() {
    let window = Window::new();
    let mut title = title_menu::TitleMenu::default();
    title.set_install_caption_icon(false);
    // SAFETY: The hidden HWND and its borrowed system menu stay live on this
    // thread. No nested dispatch or user data/configuration is involved.
    unsafe {
        let root = GetSystemMenu(window.0, 0);
        title
            .attach(window.0, 96, Resolved::Light, palette())
            .unwrap();
        let menus: Vec<_> = std::iter::once(root)
            .chain((0..4).map(|position| GetSubMenu(root, position)))
            .collect();
        let original_backgrounds: Vec<_> = menus.iter().map(|menu| background(*menu)).collect();
        let snapshots: Vec<_> = menus
            .iter()
            .flat_map(|&menu| {
                (0..GetMenuItemCount(menu) as u32).map(move |position| {
                    let info = item_info(
                        menu,
                        position,
                        MIIM_ID | MIIM_FTYPE | MIIM_DATA | MIIM_BITMAP | MIIM_SUBMENU,
                    )
                    .unwrap();
                    (menu, position, info, text(menu, position))
                })
            })
            .collect();
        for (dpi, fallback) in [
            (96, Resolved::Light),
            (144, Resolved::Contrast),
            (192, Resolved::Light),
        ] {
            title
                .refresh(window.0, dpi, Resolved::Dark, palette())
                .unwrap();
            for &(menu, position, ref original, ref caption) in &snapshots {
                let info = item_info(
                    menu,
                    position,
                    MIIM_ID | MIIM_FTYPE | MIIM_DATA | MIIM_BITMAP | MIIM_SUBMENU,
                )
                .unwrap();
                assert_ne!(info.fType & MFT_OWNERDRAW, 0);
                assert!(info.dwItemData >= TAG);
                assert!(info.hbmpItem.is_null());
                assert_eq!(info.wID, original.wID);
                assert_eq!(info.hSubMenu, original.hSubMenu);
                assert_eq!(
                    text(menu, position),
                    *caption,
                    "native accessible text was lost"
                );
            }
            title_menu::update(
                window.0,
                title_menu::Availability {
                    start: true,
                    stop: false,
                    restart: true,
                    export: false,
                    import: true,
                    active_page: 2,
                },
            )
            .unwrap();
            let proxy = GetSubMenu(root, 1);
            let view = GetSubMenu(root, 2);
            title.refresh(window.0, dpi, fallback, palette()).unwrap();
            for &(menu, position, ref original, ref caption) in &snapshots {
                let info = item_info(
                    menu,
                    position,
                    MIIM_ID | MIIM_FTYPE | MIIM_DATA | MIIM_BITMAP | MIIM_SUBMENU,
                )
                .unwrap();
                assert_eq!(info.fType & MFT_OWNERDRAW, 0);
                assert_eq!(info.wID, original.wID);
                assert_eq!(info.hSubMenu, original.hSubMenu);
                assert_eq!(info.dwItemData, original.dwItemData);
                assert_eq!(info.hbmpItem, original.hbmpItem);
                assert_eq!(text(menu, position), *caption);
            }
            assert_eq!(
                GetMenuState(proxy, title_menu::START, MF_BYCOMMAND) & MF_GRAYED,
                0
            );
            assert_ne!(
                GetMenuState(proxy, title_menu::STOP, MF_BYCOMMAND) & MF_GRAYED,
                0
            );
            assert_ne!(
                GetMenuState(view, title_menu::VIEW_LOGS, MF_BYCOMMAND) & MF_CHECKED,
                0
            );
            for (&menu, &original) in menus.iter().zip(&original_backgrounds) {
                assert_eq!(background(menu), original);
            }
        }
        title
            .refresh(window.0, 96, Resolved::Dark, palette())
            .unwrap();
        drop(title);
        assert_eq!(
            item_info(root, 0, MIIM_FTYPE).unwrap().fType & MFT_OWNERDRAW,
            0
        );
        assert_eq!(background(root), original_backgrounds[0]);
    }
}

#[test]
fn native_unicode_mnemonics_ignore_case_and_keep_disabled_state() {
    let window = Window::new();
    let mut title = title_menu::TitleMenu::default();
    title.set_install_caption_icon(false);
    // SAFETY: Hidden test-owned HWND and copied menu strings. No system locale
    // or keyboard setting is changed to test Unicode WM_MENUCHAR matching.
    unsafe {
        let root = GetSystemMenu(window.0, 0);
        assert_ne!(
            AppendMenuW(root, MF_STRING, 0x4000, windows_sys::w!("&Закрыть test")),
            0
        );
        assert_ne!(
            AppendMenuW(
                root,
                MF_STRING | MF_GRAYED,
                0x4010,
                windows_sys::w!("&Éditer test")
            ),
            0
        );
        title
            .attach(window.0, 96, Resolved::Dark, palette())
            .unwrap();
        let position = GetMenuItemCount(root) as u32 - 2;
        assert_eq!(
            title.message(WM_MENUCHAR, 'з' as usize, root as isize),
            Some((MNC_EXECUTE << 16 | position) as isize)
        );
        assert_eq!(
            title.message(WM_MENUCHAR, 'З' as usize, root as isize),
            Some((MNC_EXECUTE << 16 | position) as isize)
        );
        assert_eq!(
            title.message(WM_MENUCHAR, 'é' as usize, root as isize),
            None
        );
    }
    for (label, key) in [("&Ωμέγα", 'ω'), ("&Éditer", 'é'), ("&Закрыть", 'з')] {
        assert!(has_mnemonic(&crate::windows::wide(label), key as u16));
    }
    assert!(!has_mnemonic(
        &crate::windows::wide("&&Закрыть"),
        'з' as u16
    ));
    assert!(!has_mnemonic(
        &crate::windows::wide("Other\t&Закрыть"),
        'з' as u16
    ));
}

#[test]
fn native_mnemonics_respect_disabled_state_and_duplicate_cycle_and_do_not_claim_other_menus() {
    let window = Window::new();
    let mut title = title_menu::TitleMenu::default();
    title.set_install_caption_icon(false);
    // SAFETY: Only test-owned live native menus; actual current flags are read.
    unsafe {
        title
            .attach(window.0, 96, Resolved::Dark, palette())
            .unwrap();
        let root = GetSystemMenu(window.0, 0);
        assert_eq!(
            title.message(WM_MENUCHAR, b'v' as usize, root as isize),
            Some((MNC_EXECUTE << 16 | 2) as isize)
        );
        let proxy = GetSubMenu(root, 1);
        title_menu::update(
            window.0,
            title_menu::Availability {
                start: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            title.message(WM_MENUCHAR, b's' as usize, proxy as isize),
            Some((MNC_EXECUTE << 16) as isize)
        );
        assert_eq!(
            title.message(WM_MENUCHAR, b't' as usize, proxy as isize),
            None
        );
        let menu = crate::windows::Menu::new().unwrap();
        assert_eq!(
            title.message(WM_MENUCHAR, b'v' as usize, menu.0 as isize),
            None
        );
        assert_ne!(
            AppendMenuW(root, MF_STRING, 0x4000, windows_sys::w!("Duplicate (&V)")),
            0
        );
        title
            .refresh(window.0, 96, Resolved::Light, palette())
            .unwrap();
        title
            .refresh(window.0, 96, Resolved::Dark, palette())
            .unwrap();
        let last = GetMenuItemCount(root) as u32 - 1;
        assert_eq!(
            title.message(WM_MENUCHAR, b'v' as usize, root as isize),
            Some((MNC_SELECT << 16 | 2) as isize)
        );
        HiliteMenuItem(window.0, root, 2, MF_BYPOSITION | MF_HILITE);
        assert_eq!(
            title.message(WM_MENUCHAR, b'v' as usize, root as isize),
            Some((MNC_SELECT << 16 | last) as isize)
        );
    }
    assert!(has_mnemonic(
        &crate::windows::wide("测试(&F)\tAlt+F4"),
        b'f' as u16
    ));
    assert!(!has_mnemonic(
        &crate::windows::wide("Save && Exit"),
        b'&' as u16
    ));
    assert!(!has_mnemonic(
        &crate::windows::wide("测试\tAlt+&F"),
        b'f' as u16
    ));
}

struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    width: i32,
    height: i32,
}
impl Canvas {
    fn new(width: i32, height: i32) -> Self {
        // SAFETY: Test-owned memory DC and top-down 32-bit bitmap, selected until
        // Drop. No desktop pixels or user windows are read.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
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
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            assert!(!dc.is_null() && !bitmap.is_null());
            let old = SelectObject(dc, bitmap);
            Self {
                dc,
                bitmap,
                old,
                width,
                height,
            }
        }
    }
}
impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: This fixture owns its DC/bitmap and restores selection first.
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

#[test]
fn native_measure_draw_at_three_dpi_keeps_text_shortcuts_states_clipping_and_dc() {
    let window = Window::new();
    let mut title = title_menu::TitleMenu::default();
    title.set_install_caption_icon(false);
    // SAFETY: Hidden test HWND/menu and synchronous borrowed DRAW/MEASURE payloads.
    unsafe {
        title
            .attach(window.0, 96, Resolved::Light, palette())
            .unwrap();
        let root = GetSystemMenu(window.0, 0);
        let mut drawing = MenuTheme::default();
        let mut previous = (0, 0);
        for dpi in [96, 144, 192] {
            drawing
                .refresh(window.0, dpi, Resolved::Dark, palette())
                .unwrap();
            let style = drawing.0.as_ref().unwrap();
            for (index, entry) in style.entries.iter().enumerate() {
                let mut measure = MEASUREITEMSTRUCT {
                    CtlType: ODT_MENU,
                    itemID: entry.id,
                    itemData: TAG + index,
                    ..Default::default()
                };
                assert_eq!(
                    drawing.message(
                        WM_MEASUREITEM,
                        0,
                        (&mut measure as *mut MEASUREITEMSTRUCT) as isize
                    ),
                    Some(1)
                );
                assert!(measure.itemWidth >= scale(176, dpi) as u32);
                assert!(measure.itemHeight > 0);
                let canvas = Canvas::new(
                    measure.itemWidth as i32 + 20,
                    measure.itemHeight as i32 + 20,
                );
                let rect = RECT {
                    left: 10,
                    top: 10,
                    right: canvas.width - 10,
                    bottom: canvas.height - 10,
                };
                for state in [
                    0,
                    ODS_SELECTED,
                    ODS_DISABLED,
                    ODS_SELECTED | ODS_GRAYED,
                    ODS_CHECKED,
                    ODS_DEFAULT,
                    ODS_NOACCEL,
                ] {
                    fill(
                        canvas.dc,
                        RECT {
                            left: 0,
                            top: 0,
                            right: canvas.width,
                            bottom: canvas.height,
                        },
                        0x00ff00ff,
                    );
                    SetTextColor(canvas.dc, 0x123456);
                    SetBkColor(canvas.dc, 0x654321);
                    SetBkMode(canvas.dc, OPAQUE as i32);
                    SetDCBrushColor(canvas.dc, 0x010203);
                    let old_font = GetCurrentObject(canvas.dc, OBJ_FONT as u32);
                    let item = DRAWITEMSTRUCT {
                        CtlType: ODT_MENU,
                        itemID: entry.id,
                        itemAction: ODA_DRAWENTIRE,
                        itemState: state,
                        hwndItem: entry.menu,
                        hDC: canvas.dc,
                        rcItem: rect,
                        itemData: TAG + index,
                        ..Default::default()
                    };
                    assert_eq!(
                        drawing.message(WM_DRAWITEM, 0, (&item as *const DRAWITEMSTRUCT) as isize),
                        Some(1)
                    );
                    assert_eq!(
                        GetPixel(canvas.dc, 0, 0),
                        0xff00ff,
                        "draw escaped its item rect"
                    );
                    assert_eq!(GetPixel(canvas.dc, 10, 10), palette().surface);
                    assert_eq!(
                        GetPixel(canvas.dc, rect.left + scale(4, dpi), rect.top + 2),
                        if state & ODS_SELECTED != 0 && !entry.separator {
                            palette().accent_surface
                        } else {
                            palette().surface
                        }
                    );
                    assert_eq!(GetTextColor(canvas.dc), 0x123456);
                    assert_eq!(GetBkColor(canvas.dc), 0x654321);
                    assert_eq!(GetBkMode(canvas.dc), OPAQUE as i32);
                    assert_eq!(GetDCBrushColor(canvas.dc), 0x010203);
                    assert_eq!(GetCurrentObject(canvas.dc, OBJ_FONT as u32), old_font);
                }
                if entry.menu == root && entry.position == 0 {
                    assert!(measure.itemWidth > previous.0 && measure.itemHeight > previous.1);
                    previous = (measure.itemWidth, measure.itemHeight);
                }
            }
            let mut unrelated = MEASUREITEMSTRUCT {
                CtlType: ODT_BUTTON,
                itemData: TAG,
                ..Default::default()
            };
            assert_eq!(
                drawing.message(
                    WM_MEASUREITEM,
                    0,
                    (&mut unrelated as *mut MEASUREITEMSTRUCT) as isize
                ),
                None
            );
        }
    }
}

#[test]
fn owner_window_routes_menu_measure_before_control_paint_and_theme_refresh_restores_native() {
    let mut native = super::super::tests::NativeState::new();
    let hwnd = native.hwnd;
    // SAFETY: This isolated native State has a stopped controller, hidden HWND
    // and no timer/modal pump. The payload lives throughout synchronous dispatch.
    unsafe {
        native.state.title_menu.set_install_caption_icon(false);
        native
            .state
            .title_menu
            .attach(hwnd, 96, Resolved::Dark, palette())
            .unwrap();
        let root = GetSystemMenu(hwnd, 0);
        let info = item_info(root, 0, MIIM_DATA).unwrap();
        let mut measure = MEASUREITEMSTRUCT {
            CtlType: ODT_MENU,
            itemData: info.dwItemData,
            ..Default::default()
        };
        assert_eq!(
            SendMessageW(
                hwnd,
                WM_MEASUREITEM,
                0,
                (&mut measure as *mut MEASUREITEMSTRUCT) as isize
            ),
            1
        );
        assert!(measure.itemWidth >= 176 && measure.itemHeight >= 20);
        native.state.theme.mode = ThemeMode::Light;
        let state = &mut *native.state as *mut super::super::State;
        super::super::theme::refresh(hwnd, state);
        assert_eq!(
            item_info(root, 0, MIIM_FTYPE).unwrap().fType & MFT_OWNERDRAW,
            0
        );
        native.state.theme.mode = ThemeMode::Dark;
        super::super::theme::refresh(hwnd, state);
        assert_ne!(
            item_info(root, 0, MIIM_FTYPE).unwrap().fType & MFT_OWNERDRAW,
            0
        );
    }
}

#[test]
fn warmed_theme_refresh_and_destroyed_hwnd_teardown_keep_gdi_count_stable() {
    // SAFETY: Current-process pseudo-handle and documented GDI resource flag.
    unsafe fn resources() -> u32 {
        unsafe extern "system" {
            fn GetGuiResources(process: windows_sys::Win32::Foundation::HANDLE, flags: u32) -> u32;
        }
        unsafe {
            GetGuiResources(
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
                0,
            )
        }
    }
    let window = Window::new();
    let mut title = title_menu::TitleMenu::default();
    title.set_install_caption_icon(false);
    // SAFETY: Test owns all native resources and uses no modal dispatch.
    unsafe {
        title
            .attach(window.0, 96, Resolved::Dark, palette())
            .unwrap();
        title
            .refresh(window.0, 96, Resolved::Light, palette())
            .unwrap();
        let baseline = resources();
        for iteration in 0..30 {
            title
                .refresh(
                    window.0,
                    [96, 144, 192][iteration % 3],
                    Resolved::Dark,
                    palette(),
                )
                .unwrap();
            title
                .refresh(
                    window.0,
                    96,
                    [Resolved::Light, Resolved::Contrast][iteration % 2],
                    palette(),
                )
                .unwrap();
        }
        assert_eq!(resources(), baseline);
        title
            .refresh(window.0, 96, Resolved::Dark, palette())
            .unwrap();
        drop(window);
        drop(title);
        assert!(resources() <= baseline);
    }
}
