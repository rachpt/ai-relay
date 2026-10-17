//! Win32 宿主、目录发现、受限单实例 IPC 和当前用户登录自启。

pub mod autostart;
pub mod instance;
pub mod paths;
pub mod preferences;

use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::GetLastError,
    Graphics::Gdi::{CreateFontW, DeleteObject, HFONT},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
        WindowsAndMessaging::{
            CreatePopupMenu, DestroyIcon, DestroyMenu, HICON, HMENU, IDI_APPLICATION, IMAGE_ICON,
            LR_DEFAULTSIZE, LR_SHARED, LoadImageW,
        },
    },
};

pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub(crate) fn last_error(operation: &str) -> String {
    // SAFETY: GetLastError has no pointer or handle preconditions.
    let code = unsafe { GetLastError() };
    format!("{operation}失败（Windows 错误 {code}）")
}

/// Call before creating any HWND. An existing manifest/host DPI context is kept.
pub(crate) fn enable_dpi_awareness() -> Result<(), String> {
    // SAFETY: This is a predefined DPI context, called before this host creates windows.
    if unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } == 0 {
        // SAFETY: No preconditions; read immediately after the failed call.
        let code = unsafe { GetLastError() };
        if code != windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED {
            return Err(format!("设置 DPI 感知失败（Windows 错误 {code}）"));
        }
    }
    Ok(())
}

pub(crate) struct Icon {
    pub(crate) handle: HICON,
    owned: bool,
}

impl Icon {
    pub(crate) fn load() -> Result<Self, String> {
        // SAFETY: A null module name requests this executable's loaded module.
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        if instance.is_null() {
            return Err(last_error("获取应用模块"));
        }
        // SAFETY: Integer resource 1 is passed using MAKEINTRESOURCE semantics.
        // Without LR_SHARED, a successful icon load is owned by this wrapper.
        let resource = unsafe {
            LoadImageW(
                instance,
                std::ptr::without_provenance::<u16>(1),
                IMAGE_ICON,
                0,
                0,
                LR_DEFAULTSIZE,
            )
        };
        if !resource.is_null() {
            return Ok(Self {
                handle: resource,
                owned: true,
            });
        }
        // SAFETY: IDI_APPLICATION is a system resource and LR_SHARED retains OS ownership.
        let fallback = unsafe {
            LoadImageW(
                null_mut(),
                IDI_APPLICATION,
                IMAGE_ICON,
                0,
                0,
                LR_DEFAULTSIZE | LR_SHARED,
            )
        };
        if fallback.is_null() {
            return Err(last_error("加载应用图标"));
        }
        Ok(Self {
            handle: fallback,
            owned: false,
        })
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: Only the non-shared icon loaded by this wrapper is destroyed.
            unsafe { DestroyIcon(self.handle) };
        }
    }
}

pub(crate) struct Menu(pub(crate) HMENU);

impl Menu {
    pub(crate) fn new() -> Result<Self, String> {
        // SAFETY: CreatePopupMenu has no preconditions; this wrapper owns the result.
        let handle = unsafe { CreatePopupMenu() };
        if handle.is_null() {
            Err(last_error("创建托盘菜单"))
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for Menu {
    fn drop(&mut self) {
        // SAFETY: The menu is owned, not attached to a window, and no longer tracking.
        unsafe { DestroyMenu(self.0) };
    }
}

pub(crate) struct Font(pub(crate) HFONT);

impl Font {
    pub(crate) fn new(dpi: u32) -> Result<Self, String> {
        Self::sized(dpi, 14, 400)
    }

    pub(crate) fn heading(dpi: u32) -> Result<Self, String> {
        Self::sized(dpi, 20, 600)
    }

    fn sized(dpi: u32, pixels: i32, weight: i32) -> Result<Self, String> {
        let face = wide("Microsoft YaHei UI");
        // SAFETY: face is NUL-terminated and live for the call. The returned GDI font
        // is owned here and is only deleted after controls stop referring to it.
        let handle = unsafe {
            CreateFontW(
                -scale(pixels, dpi),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0,
                face.as_ptr(),
            )
        };
        if handle.is_null() {
            Err(last_error("创建界面字体"))
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        // SAFETY: The font is owned and has been detached from all live controls.
        unsafe { DeleteObject(self.0) };
    }
}

pub(crate) fn scale(value: i32, dpi: u32) -> i32 {
    ((i64::from(value) * i64::from(dpi) + 48) / 96).clamp(i64::from(i32::MIN), i64::from(i32::MAX))
        as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_includes_terminator_and_chinese() {
        let text = wide("AI Relay 开发版");
        assert_eq!(text.last(), Some(&0));
        assert_eq!(
            String::from_utf16(&text[..text.len() - 1]).unwrap(),
            "AI Relay 开发版"
        );
    }

    #[test]
    fn scales_common_dpi_values() {
        assert_eq!(scale(24, 96), 24);
        assert_eq!(scale(24, 144), 36);
        assert_eq!(scale(24, 192), 48);
        assert_eq!(scale(1, 144), 2);
        assert_eq!(scale(i32::MAX, u32::MAX), i32::MAX);
    }
}
