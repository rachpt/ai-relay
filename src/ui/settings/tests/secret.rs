use super::*;
use windows_sys::Win32::{
    Graphics::Gdi::*,
    UI::Controls::{
        DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_NOACCEL, ODS_NOFOCUSRECT, ODS_SELECTED,
        ODT_BUTTON,
    },
};

struct PaintTarget {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
}
impl PaintTarget {
    fn new() -> Self {
        // SAFETY: Private memory DC/bitmap, never shown or shared with the desktop.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            assert!(!dc.is_null());
            let bitmap = CreateBitmap(320, 80, 1, 32, null());
            assert!(!bitmap.is_null());
            let old = SelectObject(dc, bitmap);
            Self { dc, bitmap, old }
        }
    }
    fn mixed_pixels(&self, left: i32, top: i32, side: i32, ink: u32, fill: u32) -> usize {
        let mut mixed = 0;
        // SAFETY: All coordinates are inside this private 320×80 bitmap.
        unsafe {
            for y in top..top + side {
                for x in left..left + side {
                    let pixel = GetPixel(self.dc, x, y);
                    if pixel != ink && pixel != fill && pixel != u32::MAX {
                        mixed += 1;
                    }
                }
            }
        }
        mixed
    }
}
impl Drop for PaintTarget {
    fn drop(&mut self) {
        // SAFETY: Restore the borrowed bitmap, then release the private objects.
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

#[test]
fn native_key_visibility_preserves_utf16_selection_undo_and_unsaved_draft() {
    let mut native = NativeSettings::new();
    let settings = &mut native.settings;
    let saved = settings.config.clone().unwrap();
    let key = "synthetic-未保存-🔑-key";
    set_text(settings.key, key);
    set_text(settings.url, "https://unsaved.invalid/v1");
    let draft = settings.draft().unwrap();
    // SAFETY: Test-owned hidden controls and valid selection pointer buffers.
    unsafe {
        SendMessageW(settings.key, EM_SETSEL, 4, 19);
        let mut start = 0u32;
        let mut end = 0u32;
        SendMessageW(
            settings.key,
            EM_GETSEL,
            (&mut start as *mut u32) as WPARAM,
            (&mut end as *mut u32) as LPARAM,
        );
        let undo = SendMessageW(settings.key, 0x00c6, 0, 0);
        for _ in 0..3 {
            assert_ne!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);
            settings.toggle_key_visibility();
            assert_eq!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);
            assert_eq!(text(settings.key_visibility), "隐藏上游 Key(&K)");
            assert!(!text(settings.key_visibility).contains(key));
            assert_eq!(text(settings.key), key);
            settings.toggle_key_visibility();
            assert_ne!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);
            assert_eq!(text(settings.key_visibility), "显示上游 Key(&K)");
            let mut actual_start = 0u32;
            let mut actual_end = 0u32;
            SendMessageW(
                settings.key,
                EM_GETSEL,
                (&mut actual_start as *mut u32) as WPARAM,
                (&mut actual_end as *mut u32) as LPARAM,
            );
            assert_eq!((actual_start, actual_end), (start, end));
            assert_eq!(SendMessageW(settings.key, 0x00c6, 0, 0), undo);
            assert_eq!(settings.config.as_ref(), Some(&saved));
            assert_eq!(settings.draft().unwrap(), draft);
        }
        for category in [1, 2, 3, 4, 5] {
            settings.toggle_key_visibility();
            assert_eq!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);
            select(settings.category, category);
            settings.show_page(3);
            assert_ne!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);
            select(settings.category, 0);
            settings.show_page(3);
            assert_eq!(text(settings.key), key);
            assert_eq!(settings.draft().unwrap(), draft);
        }
        for page in [0, 1, 2] {
            settings.toggle_key_visibility();
            settings.show_page(page);
            assert_ne!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);
            settings.show_page(3);
            assert_ne!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);
            assert_eq!(settings.draft().unwrap(), draft);
        }
        settings.toggle_key_visibility();
        settings.hide_secrets();
        assert_ne!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);
        assert_eq!(settings.draft().unwrap(), draft);
        settings.toggle_key_visibility();
        settings.populate(saved.clone());
        assert_ne!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);
        assert_eq!(settings.draft().unwrap(), saved);
    }
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetKeyboardState(state: *mut u8) -> i32;
    fn SetKeyboardState(state: *const u8) -> i32;
}

struct ThreadKeyboardState([u8; 256]);
impl ThreadKeyboardState {
    fn shift_only() -> Self {
        let mut previous = [0u8; 256];
        // SAFETY: Writable complete keyboard-state buffer for this test thread.
        assert_ne!(unsafe { GetKeyboardState(previous.as_mut_ptr()) }, 0);
        let guard = Self(previous);
        let mut shifted = previous;
        for modifier in [0x10, 0x11, 0x12, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5] {
            shifted[modifier] = 0;
        }
        shifted[0x10] = 0x80;
        shifted[0xa0] = 0x80;
        // SAFETY: Changes only this thread's queue state; no AttachThreadInput,
        // SendInput, physical input, focus change or desktop message pump.
        // The guard is already live if this assertion unwinds.
        assert_ne!(unsafe { SetKeyboardState(shifted.as_ptr()) }, 0);
        guard
    }
}
impl Drop for ThreadKeyboardState {
    fn drop(&mut self) {
        // SAFETY: Restore the entire saved thread-local state, including toggle
        // bits and left/right modifiers, also when a test assertion unwinds.
        unsafe { SetKeyboardState(self.0.as_ptr()) };
    }
}

const REVERSE_ANCHOR: u32 = 14;
const REVERSE_ACTIVE: u32 = 4;

fn key_selection(key: HWND) -> (u32, u32) {
    let mut start = 0u32;
    let mut end = 0u32;
    // SAFETY: Live owner-thread EDIT and writable full-width selection outputs.
    unsafe {
        SendMessageW(
            key,
            EM_GETSEL,
            (&mut start as *mut u32) as WPARAM,
            (&mut end as *mut u32) as LPARAM,
        );
    }
    (start, end)
}

const SELECTION_PROBE_TEXT: &str = "synthetic-reverse-selection-key";

struct SelectionProbeEdit {
    parent: HWND,
    edit: HWND,
}
impl SelectionProbeEdit {
    fn new() -> Self {
        // SAFETY: Private hidden native STATIC/EDIT tree, with no Settings,
        // subclasses, custom parent procedure, timers or message pump. The parent
        // owns the child even if creation assertions unwind.
        unsafe {
            let instance = GetModuleHandleW(null());
            let parent = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                null(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                700,
                100,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            assert!(!parent.is_null());
            let mut probe = Self {
                parent,
                edit: null_mut(),
            };
            probe.edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                windows_sys::w!("EDIT"),
                wide(SELECTION_PROBE_TEXT).as_ptr(),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | WS_BORDER
                    | (ES_AUTOHSCROLL | ES_PASSWORD) as u32,
                0,
                0,
                640,
                40,
                parent,
                null_mut(),
                instance,
                null(),
            );
            assert!(!probe.edit.is_null());
            probe
        }
    }
}
impl Drop for SelectionProbeEdit {
    fn drop(&mut self) {
        // SAFETY: Owned hidden parent/child, destroyed on their creating thread.
        unsafe { DestroyWindow(self.parent) };
    }
}

fn keyboard_reverse_after_step(key: HWND, step: impl FnOnce()) -> (u32, u32) {
    let _keyboard = ThreadKeyboardState::shift_only();
    // SAFETY: Start with a collapsed caret inside the ASCII fixture, then let
    // native Shift+Left build the reverse selection. EM_SETSEL(14,4) alone did
    // not establish a reverse active end in the earlier native baseline.
    unsafe {
        SendMessageW(
            key,
            EM_SETSEL,
            REVERSE_ANCHOR as WPARAM,
            REVERSE_ANCHOR as LPARAM,
        );
        for _ in REVERSE_ACTIVE..REVERSE_ANCHOR {
            SendMessageW(key, WM_KEYDOWN, 0x25, 1);
        }
    }
    step();
    // SAFETY: Same hidden owner-thread EDIT and saved thread-local Shift state.
    // No range/text/mask/focus query intervenes before this continuation unless
    // it is part of the target step itself. Query only after Shift+Right.
    unsafe { SendMessageW(key, WM_KEYDOWN, 0x27, 1) };
    key_selection(key)
}

fn assert_keyboard_reverse_survives_step(key: HWND, label: &str, step: impl FnOnce()) {
    let expected = (REVERSE_ACTIVE + 1, REVERSE_ANCHOR);
    assert_eq!(
        keyboard_reverse_after_step(key, || {}),
        expected,
        "{label}: direct native Shift+Left reverse precondition failed; target step has not run"
    );
    assert_eq!(
        keyboard_reverse_after_step(key, step),
        expected,
        "{label}: changed the keyboard-built reverse selection's active end"
    );
}

#[test]
fn native_password_char_preserves_reverse_selection_active_end() {
    for masked in [false, true] {
        let probe = SelectionProbeEdit::new();
        // SAFETY: Establish the opposite mask state before constructing either
        // selection, so the target always changes the native password character.
        unsafe {
            SendMessageW(probe.edit, 0x00cc, if masked { 0 } else { 0x25cf }, 0);
        }
        assert_keyboard_reverse_survives_step(
            probe.edit,
            &format!("EM_SETPASSWORDCHAR masked={masked}"),
            || {
                // SAFETY: Independent native EDIT; isolate just the mask change,
                // without Settings, text queries or an EM_SETSEL restoration.
                unsafe {
                    SendMessageW(probe.edit, 0x00cc, if masked { 0x25cf } else { 0 }, 0);
                }
            },
        );
        assert_eq!(text(probe.edit), SELECTION_PROBE_TEXT);
    }
}

#[derive(Clone, Copy, Debug)]
enum KeyVisibilityChange {
    Reveal,
    ToggleMask,
    Hide,
}

fn assert_reverse_selection_survives_visibility_change(change: KeyVisibilityChange) {
    let native = NativeSettings::new();
    let settings = &native.settings;
    settings.layout(96, 696, 820);
    let saved = settings.config.clone().unwrap();
    let key = "synthetic-reverse-selection-key";
    set_text(settings.key, key);
    set_text(settings.url, "https://reverse-draft.invalid/v1");
    let draft = settings.draft().unwrap();
    assert_ne!(draft, saved);
    if !matches!(change, KeyVisibilityChange::Reveal) {
        settings.toggle_key_visibility();
        // SAFETY: Scalar native EDIT mask query, no text pointer is retained.
        assert_eq!(unsafe { SendMessageW(settings.key, 0x00d2, 0, 0) }, 0);
    }
    // Prove the keyboard-built direct baseline before applying the target step.
    // No range/mask/text/draft query follows the step until after Shift+Right.
    assert_keyboard_reverse_survives_step(settings.key, &format!("Settings::{change:?}"), || {
        match change {
            KeyVisibilityChange::Reveal | KeyVisibilityChange::ToggleMask => {
                settings.toggle_key_visibility();
            }
            KeyVisibilityChange::Hide => settings.hide_secrets(),
        }
    });
    // SAFETY: Queries run only after the continuation has already been observed.
    assert_eq!(
        unsafe { SendMessageW(settings.key, 0x00d2, 0, 0) } != 0,
        !matches!(change, KeyVisibilityChange::Reveal)
    );
    assert_eq!(text(settings.key), key);
    assert_eq!(settings.config.as_ref(), Some(&saved));
    assert_eq!(settings.draft().unwrap(), draft);
}

#[test]
fn native_key_reveal_preserves_reverse_selection_active_end() {
    assert_reverse_selection_survives_visibility_change(KeyVisibilityChange::Reveal);
}

#[test]
fn native_key_toggle_mask_preserves_reverse_selection_active_end() {
    assert_reverse_selection_survives_visibility_change(KeyVisibilityChange::ToggleMask);
}

#[test]
fn native_key_hide_preserves_reverse_selection_active_end() {
    assert_reverse_selection_survives_visibility_change(KeyVisibilityChange::Hide);
}

#[test]
fn native_settings_tab_order_has_category_then_fields_reveal_and_actions() {
    let mut native = NativeSettings::new();
    let settings = &mut native.settings;
    select(settings.category, 0);
    settings.show_page(3);
    settings.layout(96, 696, 820);
    // SAFETY: Hidden same-thread dialog tree; native Tab traversal reads styles
    // without moving desktop focus. BM_CLICK is a native message, not a claim
    // that physical Space/accelerator/IME/reader acceptance has been performed.
    unsafe {
        let order = [
            settings.category,
            settings.url,
            settings.ip,
            settings.port,
            settings.key,
            settings.key_visibility,
            settings.injection,
        ];
        for pair in order.windows(2) {
            assert_eq!(GetNextDlgTabItem(native.parent, pair[0], 0), pair[1]);
            assert_eq!(GetNextDlgTabItem(native.parent, pair[1], 1), pair[0]);
        }
        assert_eq!(
            GetNextDlgTabItem(native.parent, settings.injection, 0),
            GetDlgItem(native.parent, SAVE as i32)
        );
        assert_eq!(
            GetNextDlgTabItem(native.parent, GetDlgItem(native.parent, LOAD as i32), 0),
            settings.category
        );
        assert_eq!(text(settings.key_visibility), "显示上游 Key(&K)");
        assert_eq!(GetDlgCtrlID(settings.key_visibility) as u32, KEY_VISIBILITY);
        // Publish only a shared Settings pointer: the callback calls &self, and
        // no mutable Settings reference is used across synchronous BM_CLICK.
        SetWindowLongPtrW(
            native.parent,
            GWLP_USERDATA,
            (std::ptr::from_ref(&*settings)) as isize,
        );
        SendMessageW(settings.key_visibility, BM_CLICK, 0, 0);
        assert_eq!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);
        SendMessageW(settings.key_visibility, BM_CLICK, 0, 0);
        assert_ne!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);

        let mut clicks = 0;
        let mut message = MSG::default();
        while PeekMessageW(
            &mut message,
            native.parent,
            WM_APP + 120,
            WM_APP + 120,
            PM_REMOVE,
        ) != 0
        {
            assert_eq!(message.wParam as u32, KEY_VISIBILITY);
            assert_eq!(message.lParam, settings.key_visibility as LPARAM);
            clicks += 1;
        }
        assert_eq!(clicks, 2, "native BM_CLICK must route the reveal command");
        EnableWindow(settings.key_visibility, 0);
        SendMessageW(settings.key_visibility, BM_CLICK, 0, 0);
        assert_ne!(SendMessageW(settings.key, 0x00d2, 0, 0), 0);
        while PeekMessageW(
            &mut message,
            native.parent,
            WM_APP + 120,
            WM_APP + 120,
            PM_REMOVE,
        ) != 0
        {}
        EnableWindow(settings.key_visibility, 1);
        SetWindowLongPtrW(native.parent, GWLP_USERDATA, 0);
        for category in 1..6 {
            select(settings.category, category);
            settings.show_page(3);
            let next = GetNextDlgTabItem(native.parent, settings.category, 0);
            assert_ne!(next, settings.key_visibility);
            assert!(visible(next));
        }
        for field in [
            "client_token_enabled",
            "forward_enabled",
            "rewrite_forward_to_upstream",
            "connect_enabled",
            "enabled",
            "allow_credentials",
        ] {
            let control = settings
                .fields
                .iter()
                .find(|(_, _, name)| *name == field)
                .unwrap()
                .0;
            assert!(
                !text(control).is_empty(),
                "native checkbox has no accessible caption"
            );
        }
    }
}

#[test]
fn native_eye_draw_scales_restores_dc_and_keeps_contrast_native() {
    let light = theme::Palette {
        background: 0x00f3f3f3,
        surface: 0x00ffffff,
        text: 0x001a1a1a,
        disabled: 0x00616161,
        accent: 0x00b85f00,
        border: 0x00e0e0e0,
        accent_surface: 0x00f9f2ea,
    };
    let dark = theme::Palette {
        background: 0x00202020,
        surface: 0x002b2b2b,
        text: 0x00f5f5f5,
        disabled: 0x00c5c5c5,
        accent: 0x00ffcd60,
        border: 0x00454545,
        accent_surface: 0x00473d2d,
    };
    // Explicit palettes keep this drawing-contract test independent of the OS
    // high-contrast state; Contrast is exercised separately as a native fallback.
    // Font handles outlive every child that borrows them, also on test failure.
    let fonts = [96, 120, 144, 168, 192].map(|dpi| (dpi, Font::new(dpi).unwrap()));
    let native = NativeSettings::new();
    let target = PaintTarget::new();
    // SAFETY: Private compatible DC/bitmap, borrowed only during synchronous draw;
    // original bitmap is restored before deletion. No screen capture/focus.
    unsafe {
        let dc = target.dc;
        for (dpi, font) in &fonts {
            let dpi = *dpi;
            native.settings.set_font(font.0);
            SelectObject(dc, font.0);
            for (kind, palette) in [
                (theme::Resolved::Light, light),
                (theme::Resolved::Dark, dark),
            ] {
                for revealed in [false, true] {
                    if revealed {
                        native.settings.toggle_key_visibility();
                    } else {
                        native.settings.hide_secrets();
                    }
                    let mut item = DRAWITEMSTRUCT {
                        CtlType: ODT_BUTTON,
                        CtlID: KEY_VISIBILITY,
                        itemState: ODS_FOCUS,
                        hwndItem: native.settings.key_visibility,
                        hDC: dc,
                        rcItem: RECT {
                            left: 0,
                            top: 0,
                            right: scale(148, dpi),
                            bottom: scale(30, dpi),
                        },
                        ..Default::default()
                    };
                    let fill_brush = CreateSolidBrush(palette.surface);
                    assert!(!fill_brush.is_null());
                    let bounds = RECT {
                        left: 0,
                        top: 0,
                        right: 320,
                        bottom: 80,
                    };
                    FillRect(dc, &bounds, fill_brush);
                    DeleteObject(fill_brush);
                    let side = scale(22, dpi);
                    let mut eye =
                        super::super::secret::eye(side, palette.text, palette.surface, !revealed)
                            .unwrap();
                    assert!(eye.paint(dc, 0, 0));
                    assert!(
                        target.mixed_pixels(0, 0, side, palette.text, palette.surface) > 0,
                        "eye has no antialias coverage"
                    );
                    let title = wide(&text(native.settings.key_visibility).replace('&', ""));
                    let mut extent = windows_sys::Win32::Foundation::SIZE::default();
                    assert_ne!(
                        GetTextExtentPoint32W(
                            dc,
                            title.as_ptr(),
                            title.len() as i32 - 1,
                            &mut extent
                        ),
                        0
                    );
                    assert!(
                        extent.cx <= scale(148 - 8 - 22 - 6 - 6, dpi),
                        "reveal caption clipped at DPI {dpi}"
                    );
                    assert!(extent.cy <= scale(30 - 4, dpi));
                    SetTextColor(dc, 0x00010203);
                    SetBkColor(dc, 0x00040506);
                    SetBkMode(dc, OPAQUE as i32);
                    assert!(super::super::secret::draw(
                        (&mut item as *mut DRAWITEMSTRUCT) as LPARAM,
                        dpi,
                        palette,
                        kind
                    ));

                    for flags in [
                        ODS_DISABLED,
                        ODS_SELECTED,
                        ODS_FOCUS | ODS_NOACCEL,
                        ODS_FOCUS | ODS_NOFOCUSRECT,
                    ] {
                        item.itemState = flags;
                        assert!(super::super::secret::draw(
                            (&mut item as *mut DRAWITEMSTRUCT) as LPARAM,
                            dpi,
                            palette,
                            kind
                        ));
                    }
                    assert_eq!(GetTextColor(dc), 0x00010203);
                    assert_eq!(GetBkColor(dc), 0x00040506);
                    assert_eq!(GetBkMode(dc), OPAQUE as i32);
                    assert!(!super::super::secret::draw(
                        (&mut item as *mut DRAWITEMSTRUCT) as LPARAM,
                        dpi,
                        palette,
                        theme::Resolved::Contrast
                    ));
                }
            }
        }
        let mut unrelated = DRAWITEMSTRUCT {
            CtlType: ODT_BUTTON,
            CtlID: SAVE,
            hDC: dc,
            ..Default::default()
        };
        assert!(!super::super::secret::draw(
            (&mut unrelated as *mut DRAWITEMSTRUCT) as LPARAM,
            96,
            light,
            theme::Resolved::Light
        ));
        // The private DC is dropped before the fonts it has borrowed.
    }
}
