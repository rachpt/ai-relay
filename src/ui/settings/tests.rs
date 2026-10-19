use super::draft::field_value;
use super::*;
use crate::config::Config;
use windows_sys::Win32::{
    Graphics::Gdi::ClientToScreen,
    System::LibraryLoader::GetModuleHandleW,
    UI::Controls::{TCM_GETITEMCOUNT, TCM_GETITEMRECT},
};

#[test]
fn directory_action_requires_initialized_scope_and_idle_controller() {
    let mut settings = Settings::default();
    assert!(!settings.data_directory_available());
    settings.scope = Some(std::env::temp_dir());
    assert!(!settings.data_directory_available());
    settings.locked = true;
    assert!(settings.data_directory_available());
    settings.busy = true;
    assert!(!settings.data_directory_available());
    settings.busy = false;
    settings.action_busy = true;
    assert!(!settings.data_directory_available());
    settings.action_busy = false;
    let (_sender, receiver) = std::sync::mpsc::channel();
    settings.file_task = Some(receiver);
    assert!(!settings.data_directory_available());
    settings.file_task = None;
    settings.scope = None;
    assert!(!settings.data_directory_available());
}

struct NativeSettings {
    parent: HWND,
    settings: Settings,
}
impl NativeSettings {
    fn new() -> Self {
        static REGISTER: std::sync::Once = std::sync::Once::new();
        // SAFETY: This test creates/destroys its own hidden windows on this thread.
        // The process-lifetime class has no state; its procedure only routes Choices.
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = windows_sys::w!("AIRelay.Settings.Tests");
            REGISTER.call_once(|| {
                let class = WNDCLASSW {
                    lpfnWndProc: Some(settings_test_proc),
                    hInstance: instance,
                    lpszClassName: class,
                    ..Default::default()
                };
                assert_ne!(RegisterClassW(&class), 0);
            });
            let parent = CreateWindowExW(
                0,
                class,
                null(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                1600,
                1800,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            assert!(!parent.is_null());
            let mut native = Self {
                parent,
                settings: Settings::default(),
            };
            native.settings.create(parent, instance).unwrap();
            native.settings.populate(Config::default());
            native.settings.busy = false;
            native.settings.enable();
            native.settings.show_page(3);
            native
        }
    }
}
impl Drop for NativeSettings {
    fn drop(&mut self) {
        // SAFETY: Parent and all children belong to this test's creating thread.
        unsafe { DestroyWindow(self.parent) };
    }
}
unsafe extern "system" fn settings_test_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> LRESULT {
    // SAFETY: The window manager supplies the live notification for this call.
    unsafe {
        if message == WM_NOTIFY && choices::notify(l) {
            return 0;
        }
        DefWindowProcW(hwnd, message, w, l)
    }
}
fn native_rect(control: HWND) -> RECT {
    let mut rect = RECT::default();
    // SAFETY: The test owns this live child and provides a writable rectangle.
    assert_ne!(unsafe { GetWindowRect(control, &mut rect) }, 0);
    rect
}
fn visible(control: HWND) -> bool {
    // Parent stays hidden to avoid affecting the desktop; inspect the child's own style.
    // SAFETY: The test owns this live child on the current thread.
    unsafe { GetWindowLongPtrW(control, GWL_STYLE) as u32 & WS_VISIBLE != 0 }
}

#[test]
fn native_auto_start_checkbox_keeps_saved_preference_and_login_separate() {
    let mut native = NativeSettings::new();
    let settings = &mut native.settings;
    assert!(settings.config.as_ref().unwrap().proxy.auto_start);
    // SAFETY: Test-owned hidden controls, scalar checkbox messages only; no system Startup access.
    unsafe {
        assert_eq!(GetDlgCtrlID(settings.auto_start) as u32, AUTO_START);
        assert_eq!(
            text(settings.auto_start),
            "打开软件后自动启动代理（使用已保存配置）"
        );
        assert_eq!(GetDlgCtrlID(settings.login) as u32, LOGIN);
        let login = SendMessageW(settings.login, BM_GETCHECK, 0, 0);
        for enabled in [true, false] {
            let mut saved = Config::default();
            saved.proxy.auto_start = enabled;
            settings.populate(saved.clone());
            assert_eq!(
                SendMessageW(settings.auto_start, BM_GETCHECK, 0, 0),
                isize::from(enabled)
            );
            select(settings.category, 4);
            settings.show_page(3);
            assert!(visible(settings.auto_start));
            assert_eq!(settings.draft().unwrap(), saved);
            // 隐藏父窗口不参与真实点击；直接设置草稿勾选状态，验证读取与切页保留。
            SendMessageW(settings.auto_start, BM_SETCHECK, usize::from(!enabled), 0);
            for category in [0, 5, 4] {
                select(settings.category, category);
                settings.show_page(3);
                assert_eq!(visible(settings.auto_start), category == 4);
                let draft = settings.draft().unwrap();
                assert_eq!(draft.proxy.auto_start, !enabled);
                assert_eq!(draft.system, saved.system);
                assert_eq!(SendMessageW(settings.login, BM_GETCHECK, 0, 0), login);
                assert_eq!(settings.config.as_ref(), Some(&saved));
            }
            settings.populate(saved.clone());
            assert_eq!(settings.draft().unwrap(), saved);
        }
    }
}

#[test]
fn login_startup_probe_cannot_replace_proxy_launch_result() {
    use super::events::proxy_status;
    for message in [
        "配置已加载；正在使用已保存配置自动启动代理……",
        "配置已加载；已关闭打开软件后自动启动代理，可手动启动。",
        "操作失败：无法监听指定 IP 和端口，请检查地址、端口占用及权限。",
        "配置已加载；非回环监听需要手动启动并确认风险，未自动启动。",
        "操作失败：保存应用不得改变登录自启；请单独操作。",
    ] {
        let status = proxy_status(false, message).unwrap();
        assert!(status.starts_with("代理状态：已停止\r\n"));
        assert!(status.ends_with(message));
    }
    let running = "代理运行中；实际监听回环地址，使用已保存配置。";
    assert_eq!(
        proxy_status(true, running).unwrap(),
        format!("代理状态：运行中\r\n{running}")
    );
    for message in [
        "登录自启操作完成；实际状态见独立自启事件。",
        "登录自启操作失败：系统操作失败，请重新查询。",
        "历史策略独立保存并应用；代理未停止。",
        "配置操作完成；详情见独立配置事件。",
    ] {
        assert!(proxy_status(false, message).is_none());
        assert!(proxy_status(true, message).is_none());
    }
}

#[test]
fn native_categories_fit_common_dpi_without_overlapping_or_losing_drafts() {
    let mut native = NativeSettings::new();
    let settings = &mut native.settings;
    set_text(settings.url, "https://draft.invalid/v1");
    let token = settings
        .fields
        .iter()
        .find(|(_, _, field)| *field == "client_token")
        .unwrap()
        .0;
    set_text(token, "synthetic-unsaved-client-token");
    select(settings.theme, 2);
    let appearance = theme::Theme::new(ThemeMode::Light).unwrap();
    // SAFETY: All controls/rectangles belong to this test thread; no State or modal calls.
    unsafe {
        for dpi in [96, 144, 192] {
            choices::apply(settings.category, appearance.kind, appearance.palette, dpi);
            choices::apply(settings.theme, appearance.kind, appearance.palette, dpi);
            for content_width in [696, 952] {
                for category in 0..6 {
                    select(settings.category, category);
                    settings.show_page(3);
                    settings.layout(dpi, scale(content_width, dpi), scale(820, dpi));
                    let actions_y = settings.actions_y();
                    assert_eq!(actions_y, [342, 432, 656, 480, 610, 552][category]);

                    let mut origin = windows_sys::Win32::Foundation::POINT::default();
                    assert_ne!(ClientToScreen(native.parent, &mut origin), 0);
                    let category_rect = native_rect(settings.category);
                    assert_eq!(category_rect.left - origin.x, scale(24, dpi));
                    assert_eq!(category_rect.top - origin.y, scale(60, dpi));
                    assert_eq!(
                        category_rect.right - category_rect.left,
                        scale(content_width, dpi)
                    );
                    assert_eq!(category_rect.bottom - category_rect.top, scale(34, dpi));
                    let controls: Vec<_> = settings
                        .controls
                        .iter()
                        .filter(|(control, ..)| visible(*control))
                        .map(|&(control, ..)| (control, native_rect(control)))
                        .collect();
                    for (index, &(control, rect)) in controls.iter().enumerate() {
                        let id = GetDlgCtrlID(control) as u32;
                        assert!(rect.right > rect.left && rect.bottom > rect.top);
                        assert!(rect.left >= origin.x + scale(24, dpi));
                        assert!(rect.right <= origin.x + scale(24 + content_width, dpi));
                        assert!(rect.bottom <= origin.y + scale(820 - 56, dpi));
                        if matches!(id, SAVE | START | STOP | LOAD) {
                            assert_eq!(rect.top - origin.y, scale(actions_y - 48, dpi));
                        }
                        for &(other, other_rect) in &controls[index + 1..] {
                            assert!(
                                rect.right <= other_rect.left
                                    || other_rect.right <= rect.left
                                    || rect.bottom <= other_rect.top
                                    || other_rect.bottom <= rect.top,
                                "DPI {dpi}, category {category}: {id} overlaps {}",
                                GetDlgCtrlID(other)
                            );
                        }
                    }
                    for &(control, group) in &settings.groups {
                        assert_eq!(visible(control), group == category);
                    }
                    assert_eq!(text(settings.url), "https://draft.invalid/v1");
                    assert_eq!(text(token), "synthetic-unsaved-client-token");
                    assert_eq!(selected(settings.theme), 2);
                }
            }
        }
        settings.show_page(0);
        assert!(
            settings
                .controls
                .iter()
                .all(|(control, ..)| !visible(*control))
        );
        settings.show_page(3);
        assert!(visible(settings.category));
        assert_eq!(selected(settings.category), 5);
    }
}

#[test]
fn native_category_selection_keeps_original_command_and_combo_compatibility() {
    let native = NativeSettings::new();
    let category = native.settings.category;
    let appearance = theme::Theme::new(ThemeMode::Light).unwrap();
    // SAFETY: Owner-thread native controls and writable message/item buffers.
    unsafe {
        assert_eq!(GetDlgCtrlID(category) as u32, CATEGORY);
        assert_eq!(SendMessageW(category, CB_GETCOUNT, 0, 0), 6);
        assert_eq!(SendMessageW(category, TCM_GETITEMCOUNT, 0, 0), 6);
        let mut class = [0u16; 32];
        let count = GetClassNameW(category, class.as_mut_ptr(), class.len() as i32);
        assert_eq!(
            String::from_utf16_lossy(&class[..count as usize]),
            "SysTabControl32"
        );
        for dpi in [96, 144, 192] {
            choices::apply(category, appearance.kind, appearance.palette, dpi);
            native
                .settings
                .layout(dpi, scale(696, dpi), scale(820, dpi));
            let mut previous_right = 0;
            let mut row_top = None;
            for index in 0..6 {
                let mut item = RECT::default();
                assert_ne!(
                    SendMessageW(
                        category,
                        TCM_GETITEMRECT,
                        index,
                        (&mut item as *mut RECT) as LPARAM
                    ),
                    0
                );
                assert!(item.left >= previous_right);
                assert!(item.right <= scale(696, dpi));
                assert!(item.top >= 0 && item.bottom <= scale(34, dpi));
                assert_eq!(*row_top.get_or_insert(item.top), item.top);
                previous_right = item.right;
                select(category, (index + 1) % 6);
                let mut message = MSG::default();
                assert_eq!(
                    PeekMessageW(
                        &mut message,
                        native.parent,
                        WM_COMMAND,
                        WM_COMMAND,
                        PM_REMOVE
                    ),
                    0
                );
                let point = (((item.left + item.right) / 2) as u32
                    | ((((item.top + item.bottom) / 2) as u32) << 16))
                    as LPARAM;
                SendMessageW(category, WM_LBUTTONDOWN, 1, point);
                SendMessageW(category, WM_LBUTTONUP, 0, point);
                assert_eq!(selected(category), index);
                assert_ne!(
                    PeekMessageW(
                        &mut message,
                        native.parent,
                        WM_COMMAND,
                        WM_COMMAND,
                        PM_REMOVE
                    ),
                    0
                );
                assert_eq!(message.wParam, (CATEGORY | (CBN_SELCHANGE << 16)) as WPARAM);
                assert_eq!(message.lParam, category as LPARAM);
            }
            select(category, 0);
            SendMessageW(category, WM_KEYDOWN, 0x27, 0);
            assert_eq!(selected(category), 1);
            let mut message = MSG::default();
            assert_ne!(
                PeekMessageW(
                    &mut message,
                    native.parent,
                    WM_COMMAND,
                    WM_COMMAND,
                    PM_REMOVE
                ),
                0
            );
            assert_eq!(message.wParam, (CATEGORY | (CBN_SELCHANGE << 16)) as WPARAM);
            assert_eq!(message.lParam, category as LPARAM);
        }
    }
}

#[test]
fn access_and_protocol_fields_round_trip_without_losing_secrets() {
    let config = Config::default();
    let mut value = toml::Value::try_from(&config).unwrap();
    for (section, field, input, checked) in [
        (
            "access",
            "whitelist",
            "[\"127.0.0.1\", \"::1\", \"192.0.2.0/24\"]",
            false,
        ),
        ("access", "client_token_enabled", "", true),
        (
            "access",
            "client_token",
            "synthetic-local-client-token",
            false,
        ),
        ("proxy", "forward_enabled", "", true),
        ("proxy", "rewrite_forward_to_upstream", "", false),
        ("proxy", "connect_enabled", "", true),
        ("proxy", "connect_allowed_ports", "[443, 8443]", false),
        ("cors", "enabled", "", true),
        (
            "cors",
            "allowed_origins",
            "[\"https://client.invalid\"]",
            false,
        ),
        ("cors", "allow_credentials", "", true),
        ("network", "max_active_requests", "17", false),
    ] {
        let target = value.get_mut(section).unwrap().get_mut(field).unwrap();
        *target = field_value(target, field, input, checked).unwrap();
    }
    let actual: Config = value.try_into().unwrap();
    actual.validate_for_save().unwrap();
    assert_eq!(actual.access.whitelist.len(), 3);
    assert!(actual.access.client_token_enabled);
    assert_eq!(
        actual.access.client_token.expose_secret(),
        "synthetic-local-client-token"
    );
    assert!(actual.proxy.forward_enabled && actual.proxy.connect_enabled);
    assert!(!actual.proxy.rewrite_forward_to_upstream);
    assert_eq!(actual.proxy.connect_allowed_ports, [443, 8443]);
    assert!(actual.cors.enabled && actual.cors.allow_credentials);
    assert_eq!(actual.network.max_active_requests, 17);
}

#[test]
fn window_checkboxes_round_trip_without_changing_proxy_preferences() {
    let original = Config::default();
    let mut value = toml::Value::try_from(&original).unwrap();
    for field in ["close_to_tray", "remember_window_geometry"] {
        let target = value.get_mut("ui").unwrap().get_mut(field).unwrap();
        *target = field_value(target, field, "", false).unwrap();
    }
    let actual: Config = value.try_into().unwrap();
    assert!(!actual.ui.close_to_tray);
    assert!(!actual.ui.remember_window_geometry);
    for section in ["proxy", "listen", "upstream", "network", "access", "cors"] {
        let before = toml::Value::try_from(&original).unwrap();
        let after = toml::Value::try_from(&actual).unwrap();
        assert_eq!(before.get(section), after.get(section));
    }
}

#[test]
fn invalid_field_input_is_rejected_without_echoing_the_input() {
    let array = toml::Value::Array(vec![]);
    for (field, input) in [
        ("whitelist", "[\"synthetic-private-value\"]"),
        ("connect_allowed_ports", "[65536]"),
        ("connect_allowed_ports", "[\"443\"]"),
        ("allowed_origins", "[1]"),
        ("allowed_origins", "[]\nextra = 'synthetic-private-value'"),
    ] {
        let error = field_value(&array, field, input, false).unwrap_err();
        assert!(!error.contains("synthetic-private-value"));
    }
    let integer = toml::Value::Integer(1);
    for input in ["-1", "4294967296", "1.5", ""] {
        assert!(field_value(&integer, "max_active_requests", input, false).is_err());
    }
}
