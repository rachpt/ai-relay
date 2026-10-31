use super::*;
use crate::ui::tests::NativeState;
use windows_sys::Win32::UI::Controls::{NMHDR, TCM_SETCURSEL, TCN_SELCHANGE};

fn assert_key_state(native: &NativeState, masked: bool, selection: (u32, u32)) {
    let settings = &native.state.settings;
    let mut start = 0u32;
    let mut end = 0u32;
    // SAFETY: Live same-thread native EDIT, sized writable selection outputs.
    unsafe {
        assert_eq!(SendMessageW(settings.key, 0x00d2, 0, 0) != 0, masked);
        SendMessageW(
            settings.key,
            EM_GETSEL,
            (&mut start as *mut u32) as WPARAM,
            (&mut end as *mut u32) as LPARAM,
        );
    }
    assert_eq!((start, end), selection);
    assert_eq!(
        text(settings.key_visibility),
        if masked {
            "显示上游 Key(&K)"
        } else {
            "隐藏上游 Key(&K)"
        }
    );
}

fn assert_preserved(native: &NativeState, draft: &Config, saved: &Config) {
    assert_eq!(native.state.settings.draft().unwrap(), *draft);
    assert_eq!(native.state.settings.config.as_ref(), Some(saved));
    assert!(native.state.controller.is_finished());
}

const REVEALED_SELECTION: (u32, u32) = (3, 16);

fn revealed_key_fixture() -> (NativeState, Config, Config) {
    let mut native = NativeState::new();
    let mut saved = Config::default();
    saved.proxy.auto_start = false;
    saved.upstream.url = "https://saved.invalid/v1".into();
    saved.keys.upstream_api_key = crate::config::SecretString::new("synthetic-saved-key");
    native.state.settings.populate(saved.clone());
    native.state.settings.busy = false;
    native.state.settings.enable();
    select_main_page(&mut native, 3);
    native.command(CATEGORY, Some(0));
    let key = native.state.settings.key;
    let reveal = native.control(KEY_VISIBILITY);
    set_text(key, "synthetic-未保存-🔑-upstream-key");
    set_text(native.state.settings.url, "https://draft.invalid/v2");
    set_text(native.control(2050), "4294967295");
    let draft = native.state.settings.draft().unwrap();
    assert_ne!(draft, saved);
    // SAFETY: Private hidden controls with the production command/viewport
    // routes, no timer or message pump. BM_CLICK starts from a masked, enabled
    // page; the later gate must hide this already-revealed draft, not just deny
    // a new reveal. No file worker, real clipboard or physical input is used.
    unsafe {
        assert_ne!(is_enabled(key), 0);
        assert_ne!(is_enabled(reveal), 0);
        SendMessageW(
            key,
            EM_SETSEL,
            REVEALED_SELECTION.0 as WPARAM,
            REVEALED_SELECTION.1 as LPARAM,
        );
        SendMessageW(reveal, BM_CLICK, 0, 0);
    }
    assert_key_state(&native, false, REVEALED_SELECTION);
    assert_preserved(&native, &draft, &saved);
    (native, saved, draft)
}

#[derive(Clone, Copy, Debug)]
enum DisableGate {
    Busy,
    ActionBusy,
    FileTask,
    MissingConfig,
}

fn assert_enable_remasks_revealed_key(gate: DisableGate) {
    let (mut native, saved, draft) = revealed_key_fixture();
    let key = native.state.settings.key;
    let reveal = native.control(KEY_VISIBILITY);
    // Keep a sender alive for the synthetic pending task; never spawn a file
    // dialog or poll the stopped controller to create these availability states.
    let (_sender, receiver) = std::sync::mpsc::channel();
    match gate {
        DisableGate::Busy => native.state.settings.busy = true,
        DisableGate::ActionBusy => native.state.settings.action_busy = true,
        DisableGate::FileTask => native.state.settings.file_task = Some(receiver),
        DisableGate::MissingConfig => native.state.settings.config = None,
    }
    native.state.settings.enable();
    // SAFETY: Same-thread scalar queries of the private native controls.
    unsafe {
        assert_eq!(is_enabled(key), 0, "{gate:?} must disable the Key Edit");
        assert_eq!(
            is_enabled(reveal),
            0,
            "{gate:?} must disable the reveal button"
        );
    }
    assert_eq!(text(key), draft.keys.upstream_api_key.expose_secret());
    assert_key_state(&native, true, REVEALED_SELECTION);
    // A stale command must not undo enable()'s remask while the gate is active.
    native.command(KEY_VISIBILITY, None);
    assert_key_state(&native, true, REVEALED_SELECTION);
    if matches!(gate, DisableGate::MissingConfig) {
        assert!(native.state.settings.config.is_none());
        assert!(native.state.settings.draft().is_err());
        // draft() requires backing configuration. Restore only that snapshot,
        // never populate(): the exact native draft must still be present.
        native.state.settings.config = Some(saved.clone());
    }
    assert_preserved(&native, &draft, &saved);
    native.state.settings.busy = false;
    native.state.settings.action_busy = false;
    native.state.settings.file_task = None;
    native.state.settings.enable();
    // SAFETY: Removing a gate should re-enable both private controls, not reveal.
    unsafe {
        assert_ne!(is_enabled(key), 0);
        assert_ne!(is_enabled(reveal), 0);
    }
    assert_key_state(&native, true, REVEALED_SELECTION);
    assert_preserved(&native, &draft, &saved);
    native.command(KEY_VISIBILITY, None);
    assert_key_state(&native, false, REVEALED_SELECTION);
    assert_preserved(&native, &draft, &saved);
}

#[test]
fn native_enable_busy_remasks_revealed_key_without_losing_draft() {
    assert_enable_remasks_revealed_key(DisableGate::Busy);
}

#[test]
fn native_enable_action_busy_remasks_revealed_key_without_losing_draft() {
    assert_enable_remasks_revealed_key(DisableGate::ActionBusy);
}

#[test]
fn native_enable_file_task_remasks_revealed_key_without_losing_draft() {
    assert_enable_remasks_revealed_key(DisableGate::FileTask);
}

#[test]
fn native_enable_missing_config_remasks_revealed_key_without_losing_draft() {
    assert_enable_remasks_revealed_key(DisableGate::MissingConfig);
}

#[test]
fn native_modal_guard_keeps_revealed_key_enabled_without_toggling() {
    let (mut native, saved, draft) = revealed_key_fixture();
    let key = native.state.settings.key;
    let reveal = native.control(KEY_VISIBILITY);
    native.state.settings.modal = true;
    native.state.settings.enable();
    // SAFETY: modal is a command guard, not a native disable condition. Preserve
    // that existing distinction rather than imposing a new remask policy on it.
    unsafe {
        assert_ne!(is_enabled(key), 0);
        assert_ne!(is_enabled(reveal), 0);
    }
    assert_key_state(&native, false, REVEALED_SELECTION);
    native.command(KEY_VISIBILITY, None);
    assert_key_state(&native, false, REVEALED_SELECTION);
    assert_preserved(&native, &draft, &saved);
    native.state.settings.modal = false;
    native.command(KEY_VISIBILITY, None);
    assert_key_state(&native, true, REVEALED_SELECTION);
    assert_preserved(&native, &draft, &saved);
}

fn select_main_page(native: &mut NativeState, index: usize) {
    let hwnd = native.hwnd;
    let tab = native.state.tabs.handle;
    // SAFETY: Actual native Tab selection and synchronous production WM_NOTIFY
    // route. No borrow of State crosses SetFocus/layout/reentrant callbacks.
    unsafe {
        SendMessageW(tab, TCM_SETCURSEL, index, 0);
        let mut notification = NMHDR {
            hwndFrom: tab,
            idFrom: GetDlgCtrlID(tab) as usize,
            code: TCN_SELCHANGE,
        };
        SendMessageW(
            hwnd,
            WM_NOTIFY,
            notification.idFrom,
            (&mut notification as *mut NMHDR) as LPARAM,
        );
    }
    assert_eq!(native.state.tabs.active, index);
}

#[test]
fn native_window_routes_mask_hide_minimize_close_and_reveal_without_saving_drafts() {
    let mut native = NativeState::new();
    let mut saved = Config::default();
    saved.ui.theme = ThemeMode::Dark;
    // Use the close-to-exit branch: it posts an Exit command, which is removed
    // below without dispatch. No fake tray, modal notice or real Startup access.
    saved.ui.close_to_tray = false;
    saved.proxy.auto_start = false;
    saved.upstream.url = "https://saved.invalid/v1".into();
    saved.keys.upstream_api_key = crate::config::SecretString::new("synthetic-saved-upstream-key");
    native.state.settings.populate(saved.clone());
    native.state.settings.busy = false;
    native.state.settings.enable();
    select_main_page(&mut native, 3);
    native.command(CATEGORY, Some(0));
    let key = native.state.settings.key;
    let reveal = native.control(KEY_VISIBILITY);
    let hwnd = native.hwnd;
    set_text(key, "synthetic-未保存-🔑-upstream-key");
    set_text(native.state.settings.url, "https://draft.invalid/v2");
    set_text(native.control(2050), "4294967295");
    let draft = native.state.settings.draft().unwrap();
    assert_ne!(draft, saved);
    assert_eq!(draft.network.connect_timeout_secs, u32::MAX);
    let selection = (3, 16);
    // SAFETY: Owned, off-screen fixture registered with production window_proc;
    // the controller is stopped and no message pump/timer is run. Scalar hide /
    // minimize messages test the actual handlers, not desktop window-manager or
    // physical-keyboard acceptance. Child BM_CLICK traverses viewport forwarding.
    unsafe {
        assert_eq!(GetParent(reveal), native.state.viewport.handle);
        assert_eq!(IsWindowVisible(hwnd), 0);
        assert_ne!(is_enabled(reveal), 0);
        SendMessageW(key, EM_SETSEL, selection.0 as WPARAM, selection.1 as LPARAM);
        assert_key_state(&native, true, selection);
        SendMessageW(reveal, BM_CLICK, 0, 0);
        assert_key_state(&native, false, selection);
        assert_preserved(&native, &draft, &saved);

        SendMessageW(hwnd, WM_SHOWWINDOW, 0, 0);
        assert_key_state(&native, true, selection);
        assert_preserved(&native, &draft, &saved);
        SendMessageW(hwnd, WM_SHOWWINDOW, 1, 0);
        assert_key_state(&native, true, selection);
        native.command(KEY_VISIBILITY, None);
        assert_key_state(&native, false, selection);
        SendMessageW(hwnd, WM_SIZE, SIZE_MINIMIZED as WPARAM, 0);
        assert_key_state(&native, true, selection);
        assert_preserved(&native, &draft, &saved);
        SendMessageW(hwnd, WM_SIZE, SIZE_RESTORED as WPARAM, 0);
        assert_key_state(&native, true, selection);

        native.command(KEY_VISIBILITY, None);
        assert_key_state(&native, false, selection);
        SendMessageW(hwnd, WM_CLOSE, 0, 0);
        assert_key_state(&native, true, selection);
        assert_preserved(&native, &draft, &saved);
        assert_ne!(IsWindow(hwnd), 0);
        assert!(native.state.exiting.is_none());
        let mut exit = MSG::default();
        assert_ne!(
            PeekMessageW(&mut exit, hwnd, WM_COMMAND, WM_COMMAND, PM_REMOVE),
            0
        );
        assert_eq!(exit.wParam as u32, crate::ui::EXIT_COMMAND);
        assert_eq!(exit.lParam, 0);
        assert_eq!(
            PeekMessageW(&mut exit, hwnd, WM_COMMAND, WM_COMMAND, PM_REMOVE),
            0
        );

        // Direct WM_COMMAND cannot bypass each real availability guard, even if
        // a stale click has already entered the main window's message queue.
        for gate in 0..5 {
            let (_sender, receiver) = std::sync::mpsc::channel();
            match gate {
                0 => native.state.settings.busy = true,
                1 => native.state.settings.action_busy = true,
                2 => native.state.settings.file_task = Some(receiver),
                3 => native.state.settings.modal = true,
                _ => native.state.settings.config = None,
            }
            native.state.settings.enable();
            if gate != 3 {
                assert_eq!(is_enabled(reveal), 0);
            }
            native.command(KEY_VISIBILITY, None);
            assert_key_state(&native, true, selection);
            native.state.settings.busy = false;
            native.state.settings.action_busy = false;
            native.state.settings.file_task = None;
            native.state.settings.modal = false;
            native.state.settings.config = Some(saved.clone());
            native.state.settings.enable();
            assert_ne!(is_enabled(reveal), 0);
            assert_preserved(&native, &draft, &saved);
            native.command(KEY_VISIBILITY, None);
            assert_key_state(&native, false, selection);
            native.command(KEY_VISIBILITY, None);
            assert_key_state(&native, true, selection);
        }
        EnableWindow(reveal, 0);
        native.command(KEY_VISIBILITY, None);
        assert_key_state(&native, true, selection);
        native.state.settings.enable();

        native.command(KEY_VISIBILITY, None);
        native.command(CATEGORY, Some(1));
        assert_key_state(&native, true, selection);
        native.command(KEY_VISIBILITY, None);
        assert_key_state(&native, true, selection);
        native.command(CATEGORY, Some(0));
        assert_preserved(&native, &draft, &saved);
        native.command(KEY_VISIBILITY, None);
        select_main_page(&mut native, 0);
        assert_key_state(&native, true, selection);
        native.command(KEY_VISIBILITY, None);
        assert_key_state(&native, true, selection);
        select_main_page(&mut native, 3);
        assert_key_state(&native, true, selection);
        assert_preserved(&native, &draft, &saved);

        native.command(KEY_VISIBILITY, None);
        native.state.exiting = Some(Instant::now());
        native.command(KEY_VISIBILITY, None);
        assert_key_state(&native, false, selection);
        SendMessageW(hwnd, WM_CLOSE, 0, 0);
        assert_key_state(&native, true, selection);
        native.command(KEY_VISIBILITY, None);
        assert_key_state(&native, true, selection);
        native.state.exiting = None;
        assert_preserved(&native, &draft, &saved);

        native.command(KEY_VISIBILITY, None);
        native.command(DISCARD, None);
        assert_ne!(SendMessageW(key, 0x00d2, 0, 0), 0);
        assert_preserved(&native, &saved, &saved);
        assert_eq!(text(key), saved.keys.upstream_api_key.expose_secret());
    }
}
