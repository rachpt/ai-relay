//! Key 显隐只改变原生 Edit 的掩码，不复制、替换或提交秘密文本。
use super::*;
use windows_sys::Win32::{
    Graphics::Gdi::*,
    UI::Controls::{
        DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_HOTLIGHT, ODS_NOACCEL, ODS_NOFOCUSRECT,
        ODS_SELECTED, ODT_BUTTON,
    },
};

const EM_SETPASSWORDCHAR: u32 = 0x00cc;
const EM_GETPASSWORDCHAR: u32 = 0x00d2;
const MASK: WPARAM = 0x25cf;

pub(super) fn eye(
    size: i32,
    ink: u32,
    background: u32,
    crossed: bool,
) -> Option<super::super::raster::Raster> {
    let mut raster = super::super::raster::Raster::new(size, size, background)?;
    let side = size as f32;
    let lens = |inset: f32| {
        std::array::from_fn::<_, 32, _>(|index| {
            let t = if index < 16 {
                index as f32 / 15.0
            } else {
                (31 - index) as f32 / 15.0
            };
            let arc = (t * std::f32::consts::PI).sin();
            let sign = if index < 16 { -1.0 } else { 1.0 };
            (
                side * (0.10 + inset + t * (0.80 - inset * 2.0)),
                side * (0.5 + sign * arc * (0.23 - inset)),
            )
        })
    };
    raster.polygon(&lens(0.0), ink);
    raster.polygon(&lens(0.065), background);
    raster.circle(side * 0.5, side * 0.5, side * 0.11, ink);
    if crossed {
        raster.line(
            (side * 0.18, side * 0.16),
            (side * 0.82, side * 0.84),
            side * 0.16,
            background,
        );
        raster.line(
            (side * 0.18, side * 0.16),
            (side * 0.82, side * 0.84),
            side * 0.065,
            ink,
        );
    }
    Some(raster)
}

/// Route before the generic themed Button painter; contrast keeps a native
/// Chinese text Button. The native HWND/caption still owns input/accessibility.
pub(in crate::ui) unsafe fn draw(
    lparam: LPARAM,
    dpi: u32,
    palette: theme::Palette,
    kind: theme::Resolved,
) -> bool {
    // SAFETY: Caller supplies the live WM_DRAWITEM payload. The DC is borrowed
    // for this callback only and its complete state is restored before return.
    unsafe {
        if lparam == 0 || kind == theme::Resolved::Contrast {
            return false;
        }
        let item = &*(lparam as *const DRAWITEMSTRUCT);
        if item.CtlType != ODT_BUTTON || item.CtlID != KEY_VISIBILITY {
            return false;
        }
        let saved = SaveDC(item.hDC);
        if saved == 0 {
            // Let the existing generic Button text renderer be the fallback.
            return false;
        }
        struct Saved(HDC, i32);
        impl Drop for Saved {
            fn drop(&mut self) {
                // SAFETY: Owns this successful SaveDC; WM_DRAWITEM DC outlives it.
                unsafe { RestoreDC(self.0, self.1) };
            }
        }
        let _saved = Saved(item.hDC, saved);
        let disabled = item.itemState & ODS_DISABLED != 0;
        let fill = if disabled {
            theme::blend(palette.surface, palette.background, 60)
        } else if item.itemState & ODS_SELECTED != 0 {
            theme::blend(palette.surface, palette.text, 10)
        } else if item.itemState & ODS_HOTLIGHT != 0 {
            palette.hover()
        } else {
            palette.surface
        };
        let ink = if disabled {
            palette.disabled
        } else {
            palette.text
        };
        rounded::surface(
            item.hDC,
            &item.rcItem,
            dpi,
            palette.background,
            fill,
            palette.border,
        );
        let font = SendMessageW(item.hwndItem, WM_GETFONT, 0, 0) as HGDIOBJ;
        if !font.is_null() {
            SelectObject(item.hDC, font);
        }
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, ink);
        let size = scale(22, dpi).min(item.rcItem.bottom - item.rcItem.top - scale(4, dpi));
        let mut caption_rect = RECT {
            left: item.rcItem.left + scale(8, dpi),
            right: item.rcItem.right - scale(6, dpi),
            ..item.rcItem
        };
        let crossed = text(item.hwndItem).starts_with("显示");
        if let Some(mut raster) = eye(size, ink, fill, crossed)
            && raster.paint(
                item.hDC,
                caption_rect.left,
                item.rcItem.top + (item.rcItem.bottom - item.rcItem.top - size) / 2,
            )
        {
            caption_rect.left += size + scale(6, dpi);
        }
        let caption = wide(&text(item.hwndItem));
        DrawTextW(
            item.hDC,
            caption.as_ptr(),
            -1,
            &mut caption_rect,
            DT_SINGLELINE
                | DT_VCENTER
                | DT_CENTER
                | DT_END_ELLIPSIS
                | if item.itemState & ODS_NOACCEL != 0 {
                    DT_HIDEPREFIX
                } else {
                    0
                },
        );
        if item.itemState & (ODS_DISABLED | ODS_FOCUS | ODS_NOFOCUSRECT) == ODS_FOCUS {
            let mut focus = item.rcItem;
            InflateRect(&mut focus, -scale(4, dpi), -scale(4, dpi));
            SetBkColor(item.hDC, fill);
            DrawFocusRect(item.hDC, &focus);
        }
        true
    }
}

impl Settings {
    pub(super) fn toggle_key_visibility(&self) {
        // Disabled/native programmatic clicks must not reveal a hidden/busy page.
        if self.page != 3 || selected(self.category) != 0
            || self.key_visibility.is_null()
            // SAFETY: Same-thread live Button; scalar enabled-state query.
            || unsafe { is_enabled(self.key_visibility) == 0 }
        {
            self.hide_secrets();
            return;
        }
        // SAFETY: Owner-thread live native EDIT; scalar password-character query.
        let masked = unsafe { SendMessageW(self.key, EM_GETPASSWORDCHAR, 0, 0) != 0 };
        self.set_key_masked(!masked);
    }

    /// Call before hiding the main window (including tray/background paths).
    /// Category/main-tab switches and draft population also call this locally.
    pub fn hide_secrets(&self) {
        self.set_key_masked(true);
    }

    fn set_key_masked(&self, masked: bool) {
        if self.key.is_null() || self.key_visibility.is_null() {
            return;
        }
        // SAFETY: Native EDIT owns the text, undo stack and UTF-16 selection.
        // EM_SETPASSWORDCHAR does not replace text or notify EN_CHANGE. Restore
        // selection defensively without taking focus from the native Button.
        unsafe {
            let current = SendMessageW(self.key, EM_GETPASSWORDCHAR, 0, 0);
            if (current != 0) != masked || (masked && current != MASK as isize) {
                let mut start = 0u32;
                let mut end = 0u32;
                SendMessageW(
                    self.key,
                    EM_GETSEL,
                    (&mut start as *mut u32) as WPARAM,
                    (&mut end as *mut u32) as LPARAM,
                );
                SendMessageW(
                    self.key,
                    EM_SETPASSWORDCHAR,
                    if masked { MASK } else { 0 },
                    0,
                );
                SendMessageW(self.key, EM_SETSEL, start as WPARAM, end as LPARAM);
                InvalidateRect(self.key, null(), 0);
            }
        }
        let caption = if masked {
            "显示上游 Key(&K)"
        } else {
            "隐藏上游 Key(&K)"
        };
        if text(self.key_visibility) != caption {
            set_text(self.key_visibility, caption);
        }
    }
}
