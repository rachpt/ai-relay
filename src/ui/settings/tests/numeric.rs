use super::*;
use std::cell::Cell;

type ReentryProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: ReentryProc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: ReentryProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}
const REENTRY_ID: usize = 0x4e55_4d54;

struct QueryDestroy {
    edit: HWND,
    hint: HWND,
    queried: HWND,
    message: u32,
    armed: Cell<bool>,
    fired: Cell<u32>,
    destroyed: Cell<bool>,
    hint_writes_after_destroy: Cell<u32>,
    hint_queries_after_destroy: Cell<u32>,
}
struct QueryDestroyGuard(Box<QueryDestroy>);
impl QueryDestroyGuard {
    fn new(edit: HWND, hint: HWND, queried: HWND, message: u32) -> Self {
        let guard = Self(Box::new(QueryDestroy {
            edit,
            hint,
            queried,
            message,
            armed: Cell::new(true),
            fired: Cell::new(0),
            destroyed: Cell::new(false),
            hint_writes_after_destroy: Cell::new(0),
            hint_queries_after_destroy: Cell::new(0),
        }));
        // SAFETY: The stable Box outlives both subclasses; Drop removes them
        // before freeing it, including when an assertion unwinds this test.
        unsafe {
            for hwnd in [edit, hint] {
                assert_ne!(
                    SetWindowSubclass(
                        hwnd,
                        Some(destroy_during_numeric_query),
                        REENTRY_ID,
                        (&*guard.0 as *const QueryDestroy) as usize
                    ),
                    0
                );
            }
        }
        guard
    }
}
impl Drop for QueryDestroyGuard {
    fn drop(&mut self) {
        // SAFETY: Only removes this proc/id; a destroyed HWND is tolerated.
        unsafe {
            for hwnd in [self.0.edit, self.0.hint] {
                RemoveWindowSubclass(hwnd, Some(destroy_during_numeric_query), REENTRY_ID);
            }
        }
    }
}
unsafe extern "system" fn destroy_during_numeric_query(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: The test guard owns the stable probe through all native callbacks.
    // Destruction is recorded, not asserted, so no Rust panic crosses Win32 FFI.
    unsafe {
        let probe = &*(data as *const QueryDestroy);
        if hwnd == probe.queried && message == probe.message && probe.armed.replace(false) {
            probe.fired.set(probe.fired.get() + 1);
            probe.destroyed.set(DestroyWindow(probe.edit) != 0);
            if message == WM_GETTEXT && w != 0 && l != 0 {
                *(l as *mut u16) = 0;
            }
            return 0;
        }
        if hwnd == probe.hint && probe.destroyed.get() {
            if message == WM_SETTEXT {
                probe
                    .hint_writes_after_destroy
                    .set(probe.hint_writes_after_destroy.get() + 1);
            } else if matches!(message, WM_GETTEXTLENGTH | WM_GETTEXT) {
                probe
                    .hint_queries_after_destroy
                    .set(probe.hint_queries_after_destroy.get() + 1);
            }
        }
        if message == WM_NCDESTROY {
            if hwnd == probe.edit {
                probe.destroyed.set(true);
            }
            RemoveWindowSubclass(hwnd, Some(destroy_during_numeric_query), id);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

#[test]
fn native_numeric_query_destruction_never_writes_a_surviving_hint() {
    // Keep the first case identical to the regression observed before the fix:
    // Edit WM_GETTEXTLENGTH destroys Edit, then the old refresh writes the hint.
    for (query_hint, query, message, input) in [
        (false, WM_GETTEXTLENGTH, WM_SETTEXT, "42"),
        (false, WM_GETTEXT, WM_SETTEXT, "42"),
        (true, WM_GETTEXTLENGTH, WM_SETTEXT, "0"),
        (true, WM_GETTEXT, WM_SETTEXT, "0"),
        (true, WM_SETTEXT, WM_SETTEXT, "0"),
        (false, EM_GETSEL, 0x00c2, "2"),
        (false, WM_GETTEXTLENGTH, 0x00c2, "2"),
        (false, WM_GETTEXT, 0x00c2, "2"),
        (false, EM_GETSEL, WM_CHAR, "2"),
        (false, WM_GETTEXTLENGTH, WM_CHAR, "2"),
        (false, WM_GETTEXT, WM_CHAR, "2"),
        (true, WM_GETTEXTLENGTH, WM_CHAR, "x"),
        (true, WM_GETTEXT, WM_CHAR, "x"),
        (true, WM_SETTEXT, 0x00c2, "x"),
        (true, WM_SETTEXT, WM_SETTEXT, "x"),
    ] {
        let native = NativeSettings::new();
        let field = native
            .settings
            .numbers
            .iter()
            .find(|number| number.control == native.settings.port)
            .copied()
            .unwrap();
        let previous_hint = text(field.hint);
        let queried = if query_hint {
            field.hint
        } else {
            field.control
        };
        let probe = QueryDestroyGuard::new(field.control, field.hint, queried, query);
        // SAFETY: Hidden owner-thread fixture. Query/text callbacks destroy only
        // Edit; the surviving sibling records any stale query/write continuation.
        unsafe {
            let buffer = wide(input);
            SendMessageW(
                field.control,
                message,
                if message == WM_CHAR {
                    WPARAM::from(buffer[0])
                } else {
                    0
                },
                if message == WM_CHAR {
                    0
                } else {
                    buffer.as_ptr() as LPARAM
                },
            );
            assert_eq!(
                probe.0.fired.get(),
                1,
                "query {query:#x}, hint {query_hint}, input {message:#x}"
            );
            assert!(probe.0.destroyed.get());
            assert_eq!(IsWindow(field.control), 0);
            assert_ne!(IsWindow(field.hint), 0);
        }
        assert_eq!(
            probe.0.hint_writes_after_destroy.get(),
            0,
            "stale hint write after query {query:#x}, hint {query_hint}"
        );
        assert_eq!(
            probe.0.hint_queries_after_destroy.get(),
            0,
            "stale hint query after query {query:#x}, hint {query_hint}"
        );
        assert_eq!(text(field.hint), previous_hint);
    }
    fn delayed_provider_destroys_edit(hwnd: HWND) {
        // SAFETY: One-shot callback owns the test EDIT on this thread. Simulates
        // reentry before clipboard returns; never opens/changes the OS clipboard.
        unsafe { DestroyWindow(hwnd) };
    }
    for input in [Ok(wide("443")), Err("未粘贴：测试提供方失败。")] {
        let native = NativeSettings::new();
        let field = native
            .settings
            .numbers
            .iter()
            .find(|number| number.control == native.settings.port)
            .copied()
            .unwrap();
        let previous_hint = text(field.hint);
        let probe = QueryDestroyGuard::new(field.control, field.hint, field.hint, WM_NULL);
        // Disarm query destruction; NCDESTROY below must still be recorded.
        probe.0.armed.set(false);
        numeric::test_clipboard(input);
        numeric::test_clipboard_reentry(delayed_provider_destroys_edit);
        // SAFETY: Synthetic clipboard source enters the same WM_PASTE path; no
        // external clipboard data, input focus, timer or network is involved.
        unsafe {
            SendMessageW(field.control, WM_PASTE, 0, 0);
            assert_eq!(IsWindow(field.control), 0);
            assert_ne!(IsWindow(field.hint), 0);
        }
        assert_eq!(probe.0.hint_writes_after_destroy.get(), 0);
        assert_eq!(probe.0.hint_queries_after_destroy.get(), 0);
        assert_eq!(text(field.hint), previous_hint);
    }
}

#[test]
fn numeric_metadata_matches_all_config_boundaries_without_narrowing_u32() {
    for spec in std::iter::once(numeric::PORT).chain(numeric::NETWORK) {
        let minimum = spec.min.to_string();
        let maximum = spec.max.to_string();
        assert_eq!(spec.parse(&minimum), Ok(spec.min));
        assert_eq!(spec.parse(&maximum), Ok(spec.max));
        assert_eq!(spec.digits(), maximum.len());
        let mut config = toml::Value::try_from(Config::default()).unwrap();
        for value in [spec.min, spec.max] {
            config[spec.section][spec.field] = toml::Value::Integer(i64::from(value));
            let actual: Config = config.clone().try_into().unwrap();
            actual.validate_for_save().unwrap();
            let original = toml::Value::Integer(1);
            assert_eq!(
                field_value(
                    &original,
                    spec.section,
                    spec.field,
                    &value.to_string(),
                    false
                )
                .unwrap(),
                toml::Value::Integer(i64::from(value))
            );
        }
        if spec.min > 0 {
            assert!(spec.parse("0").is_err());
            config[spec.section][spec.field] = toml::Value::Integer(0);
            let actual: Config = config.clone().try_into().unwrap();
            assert!(actual.validate_for_save().is_err());
        }
        assert!(spec.parse(&(u64::from(spec.max) + 1).to_string()).is_err());
        for invalid in [
            "",
            "+1",
            "-1",
            "1.0",
            "1e2",
            "0xff",
            " 1",
            "1 ",
            "1\n",
            "１",
            "١",
            "1\0",
            "00000000001",
        ] {
            assert!(
                spec.parse(invalid).is_err(),
                "{} accepted non-ASCII integer",
                spec.field
            );
        }
    }
}

#[test]
fn numeric_replacements_preserve_selection_and_reject_entire_invalid_pastes() {
    let port = numeric::PORT;
    assert_eq!(
        numeric::replacement(port, "8080", (1, 3), "44").unwrap(),
        "8440"
    );
    assert_eq!(
        numeric::replacement(port, "65535", (0, 5), "443").unwrap(),
        "443"
    );
    assert_eq!(numeric::replacement(port, "8080", (4, 0), "").unwrap(), "");
    assert_eq!(
        numeric::replacement(port, "8080", (99, 99), "1").unwrap(),
        "80801"
    );
    // Out-of-range/empty intermediate drafts remain editable and are not clamped.
    for draft in ["", "0", "99999"] {
        assert_eq!(
            numeric::replacement(port, "8080", (0, 4), draft).unwrap(),
            draft
        );
        assert!(port.parse(draft).is_err());
    }
    for invalid in [
        "12x",
        " 443",
        "443\r\n",
        "１２３",
        "-1",
        "1.5",
        "123456",
        "\0",
    ] {
        assert!(numeric::replacement(port, "8080", (0, 4), invalid).is_err());
    }
    assert!(numeric::replacement(port, "65535", (5, 5), "1").is_err());
    for valid in ["", "0", "443", "65535", "99999"] {
        assert_eq!(numeric::clipboard_text(port, &wide(valid)).unwrap(), valid);
    }
    for invalid in ["12x", " 443", "443\r\n", "１２３", "-1", "1.5", "123456"] {
        assert!(numeric::clipboard_text(port, &wide(invalid)).is_err());
    }
    assert!(numeric::clipboard_text(port, &[b'1' as u16; 12]).is_err());
    assert!(numeric::clipboard_text(port, &[0xd800, 0]).is_err());
    assert_eq!(
        numeric::replacement(numeric::NETWORK[0], "1", (0, 1), "4294967295").unwrap(),
        "4294967295"
    );
    assert!(numeric::replacement(numeric::NETWORK[0], "1", (0, 1), "42949672950").is_err());
    assert_eq!(layout::network_columns(679), 1);
    assert_eq!(layout::network_columns(680), 2);
    assert!(layout::network_actions_y(680) < layout::network_actions_y(679));
    assert!(numeric::shift_insert(0x2d, true, false));
    assert!(!numeric::shift_insert(0x2d, false, false));
    assert!(!numeric::shift_insert(0x2d, true, true));
    assert!(!numeric::shift_insert(0x2e, true, false));
}

#[test]
fn connect_port_array_keeps_unsigned_u16_boundaries_and_config_zero_rule() {
    let original = toml::Value::Array(vec![]);
    for input in ["[1]", "[65535]", "[1, 443, 65535]"] {
        let ports = field_value(&original, "proxy", "connect_allowed_ports", input, false).unwrap();
        let mut config = toml::Value::try_from(Config::default()).unwrap();
        config["proxy"]["connect_allowed_ports"] = ports;
        config["proxy"]["connect_enabled"] = toml::Value::Boolean(true);
        let config: Config = config.try_into().unwrap();
        config.validate_for_save().unwrap();
    }
    for input in ["[-1]", "[65536]", "[4294967295]", "[1.0]", "[\"443\"]"] {
        assert!(field_value(&original, "proxy", "connect_allowed_ports", input, false).is_err());
    }
    // Zero fits u16 but is forbidden by Config, not a TOML parsing error.
    let zero = field_value(&original, "proxy", "connect_allowed_ports", "[0]", false).unwrap();
    let mut config = toml::Value::try_from(Config::default()).unwrap();
    config["proxy"]["connect_allowed_ports"] = zero;
    let config: Config = config.try_into().unwrap();
    assert_eq!(
        config.validate_for_save().unwrap_err().field,
        "proxy.connect_allowed_ports"
    );
}

#[test]
fn native_numeric_edits_validate_replace_character_and_empty_drafts_without_clamping() {
    let mut native = NativeSettings::new();
    let settings = &mut native.settings;
    let saved = settings.config.clone().unwrap();
    const REPLACE: u32 = 0x00c2;
    // SAFETY: Hidden test-owned EDITs; UTF-16 buffers and selection pointers remain
    // live through synchronous SendMessage. No system clipboard is read or changed.
    unsafe {
        for number in &settings.numbers {
            let control = number.control;
            assert_ne!(
                GetWindowLongPtrW(control, GWL_STYLE) as u32 & ES_NUMBER as u32,
                0
            );
            assert_eq!(
                SendMessageW(control, 0x00d5, 0, 0),
                number.spec.digits() as isize
            );
            set_text(control, "8080");
            SendMessageW(control, EM_SETSEL, 1, 3);
            SendMessageW(control, REPLACE, 1, wide("44").as_ptr() as LPARAM);
            assert_eq!(text(control), "8440");
            SendMessageW(control, EM_SETSEL, 0, -1);
            numeric::test_clipboard(Ok(wide("443")));
            SendMessageW(control, WM_PASTE, 0, 0);
            assert_eq!(text(control), "443");
            SendMessageW(control, EM_SETSEL, 0, -1);
            numeric::test_clipboard(Ok(wide("8080")));
            SendMessageW(control, WM_CHAR, 0x16, 0);
            assert_eq!(text(control), "8080");
            SendMessageW(control, EM_SETSEL, 1, 3);
            numeric::test_clipboard(Ok(wide("44")));
            SendMessageW(control, WM_PASTE, 0, 0);
            assert_eq!(text(control), "8440");
            set_text(control, "8080");
            SendMessageW(control, EM_SETSEL, 1, 3);
            let before = SendMessageW(control, EM_GETSEL, 0, 0);
            for invalid in ["+1", "12x", " 1", "1\n", "１", "42949672950"] {
                assert_eq!(
                    SendMessageW(control, REPLACE, 1, wide(invalid).as_ptr() as LPARAM),
                    0
                );
                assert_eq!(text(control), "8080");
                assert_eq!(SendMessageW(control, EM_GETSEL, 0, 0), before);
                numeric::test_clipboard(Ok(wide(invalid)));
                SendMessageW(control, WM_PASTE, 0, 0);
                assert_eq!(text(control), "8080");
                assert_eq!(SendMessageW(control, EM_GETSEL, 0, 0), before);
                assert!(text(number.hint).starts_with("未"));
                set_text(control, invalid);
                assert_eq!(text(control), "8080");
            }
            set_text(control, &"9".repeat(number.spec.digits()));
            assert_eq!(text(control), "9".repeat(number.spec.digits()));
            assert_ne!(text(number.hint), number.spec.hint);
            assert!(settings.draft().is_err());
            set_text(control, "8080");
            numeric::test_clipboard(Err("未粘贴：剪贴板正忙，请重试。"));
            SendMessageW(control, WM_PASTE, 0, 0);
            assert_eq!(text(control), "8080");
            assert!(text(number.hint).contains("剪贴板正忙"));
            SendMessageW(control, EM_SETSEL, 0, -1);
            SendMessageW(control, REPLACE, 1, wide("").as_ptr() as LPARAM);
            assert!(text(control).is_empty());
            assert!(settings.draft().is_err());
            assert!(text(number.hint).contains("必填"));
            set_text(control, &number.spec.max.to_string());
            assert_eq!(text(control), number.spec.max.to_string());
            assert_eq!(text(number.hint), number.spec.hint);
            let at_end = text(control).len();
            SendMessageW(control, EM_SETSEL, at_end, at_end as LPARAM);
            SendMessageW(control, WM_CHAR, b'1' as WPARAM, 0);
            assert_eq!(text(control), number.spec.max.to_string());
            SendMessageW(control, EM_SETSEL, 0, -1);
            SendMessageW(control, WM_CHAR, '２' as WPARAM, 0);
            assert_eq!(text(control), number.spec.max.to_string());
            set_text(control, "0");
            assert_eq!(text(control), "0");
            if number.spec.min > 0 {
                assert!(settings.draft().is_err());
                assert_ne!(text(number.hint), number.spec.hint);
            }
            let initial = toml::Value::try_from(&saved).unwrap();
            set_text(
                control,
                &initial[number.spec.section][number.spec.field].to_string(),
            );
        }
        assert_eq!(settings.config.as_ref(), Some(&saved));
        assert_eq!(settings.draft().unwrap(), saved);
    }
}

#[test]
fn native_numeric_invalid_range_survives_category_switch_and_is_rejected_on_save() {
    let mut native = NativeSettings::new();
    let settings = &mut native.settings;
    let saved = settings.config.clone().unwrap();
    set_text(settings.port, "99999");
    for category in [0, 3, 4, 2, 0] {
        select(settings.category, category);
        settings.show_page(3);
        assert_eq!(text(settings.port), "99999");
        let error = settings.draft().unwrap_err();
        assert!(error.contains("1–65535"));
        assert!(!error.contains("99999"));
        assert_eq!(settings.config.as_ref(), Some(&saved));
    }
    settings.populate(saved.clone());
    assert_eq!(settings.draft().unwrap(), saved);
}
