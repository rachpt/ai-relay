//! 原生 Common Controls Tab；页面控件只隐藏，不销毁或重新填充。
use super::*;
use windows_sys::Win32::UI::Controls::*;

pub(super) const ID: u32 = 2100;
pub(super) const START: u32 = 2102;
pub(super) const STOP: u32 = 2103;

#[derive(Default)]
pub(super) struct Tabs {
    pub handle: HWND,
    pub active: usize,
    pub start: HWND,
    pub stop: HWND,
}
impl Tabs {
    pub fn create(&mut self, parent: HWND) -> Result<(), String> {
        let init = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TAB_CLASSES,
        };
        // SAFETY: Sized initialization structure and live parent; strings are copied synchronously.
        unsafe {
            if InitCommonControlsEx(&init) == 0 {
                return Err(last_error("初始化原生分页控件"));
            }
            self.handle = CreateWindowExW(
                0,
                wide("SysTabControl32").as_ptr(),
                null(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | TCS_FIXEDWIDTH,
                0,
                0,
                0,
                0,
                parent,
                ID as usize as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            if self.handle.is_null() {
                return Err(last_error("创建原生分页控件"));
            }
            for (index, title) in ["概览", "统计分析", "实时日志", "设置"].iter().enumerate()
            {
                let mut title = wide(title);
                let item = TCITEMW {
                    mask: TCIF_TEXT,
                    pszText: title.as_mut_ptr(),
                    ..Default::default()
                };
                if SendMessageW(
                    self.handle,
                    TCM_INSERTITEMW,
                    index,
                    (&item as *const TCITEMW) as isize,
                ) == -1
                {
                    return Err(last_error("添加分页"));
                }
            }
            for (title, id, target) in [
                ("启动已保存配置", START, &mut self.start),
                ("停止代理", STOP, &mut self.stop),
            ] {
                *target = CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    wide(title).as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    0,
                    0,
                    0,
                    0,
                    parent,
                    id as usize as HMENU,
                    GetModuleHandleW(null()),
                    null(),
                );
                if target.is_null() {
                    return Err(last_error("创建概览操作"));
                }
            }
        }
        Ok(())
    }
    pub fn selection(&self) -> usize {
        // SAFETY: Live native Tab, no pointer arguments.
        unsafe { SendMessageW(self.handle, TCM_GETCURSEL, 0, 0).clamp(0, 3) as usize }
    }
    pub fn select(&mut self, index: usize) {
        self.active = index.min(3);
        // SAFETY: Live native Tab; setting selection does not emit TCN_SELCHANGE.
        unsafe {
            SendMessageW(self.handle, TCM_SETCURSEL, self.active, 0);
        }
    }
    pub fn set_font(&self, font: windows_sys::Win32::Graphics::Gdi::HFONT) {
        for control in [self.handle, self.start, self.stop] {
            // SAFETY: Live controls; State retains the font.
            unsafe {
                SendMessageW(control, WM_SETFONT, font as usize, 1);
            }
        }
    }
    pub fn layout(&self, dpi: u32, width: i32) {
        // SAFETY: Live sibling controls; Tab occupies only the header, so it cannot cover page controls.
        unsafe {
            SendMessageW(
                self.handle,
                TCM_SETITEMSIZE,
                0,
                (scale(108, dpi) | (scale(34, dpi) << 16)) as isize,
            );
            MoveWindow(
                self.handle,
                scale(24, dpi),
                scale(12, dpi),
                width - scale(48, dpi),
                scale(36, dpi),
                1,
            );
            MoveWindow(
                self.start,
                scale(24, dpi),
                scale(312, dpi),
                scale(340, dpi),
                scale(36, dpi),
                1,
            );
            MoveWindow(
                self.stop,
                scale(380, dpi),
                scale(312, dpi),
                scale(140, dpi),
                scale(36, dpi),
                1,
            );
        }
    }
}
