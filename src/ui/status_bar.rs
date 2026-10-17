//! A docked status surface over the existing native STATIC. Text updates and
//! accessibility keep their original HWND/WM_SETTEXT contract with the controller.
use super::*;
use windows_sys::Win32::{Graphics::Gdi::*, UI::Controls::*};

const HEIGHT: i32 = 56;
// Public winuser.h STATIC style, not exposed by the enabled windows-sys features.
const SS_NOTIFY: isize = 0x0100;
const SUBCLASS_ID: usize = 0x5354;
type SubclassProc =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: HWND, proc: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

struct Surface {
    state: *mut State,
    tooltip: HWND,
    tip: Vec<u16>,
}

fn lines(text: &str) -> (&str, String) {
    let (status, message) = text.split_once('\n').unwrap_or((text, ""));
    (
        status.trim_end_matches('\r'),
        message.split_whitespace().collect::<Vec<_>>().join(" "),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusKind {
    Error,
    Busy,
    Running,
    Stopped,
}

fn status_kind(text: &str) -> StatusKind {
    let status = text.split_once('\n').map_or(text, |(status, _)| status);
    if status.contains("错误")
        || status.contains("失败")
        || status.to_ascii_lowercase().contains("error")
    {
        StatusKind::Error
    } else if [
        "启动中",
        "停止中",
        "重启中",
        "退出",
        "保存并应用中",
        "重新加载配置中",
    ]
    .iter()
    .any(|label| status.contains(label))
    {
        StatusKind::Busy
    } else if ["未运行", "未启动", "未配置", "已停止"]
        .iter()
        .any(|label| status.contains(label))
    {
        StatusKind::Stopped
    } else if status.contains("运行中") {
        StatusKind::Running
    } else {
        StatusKind::Stopped
    }
}

fn bounds(width: i32, height: i32, dpi: u32) -> RECT {
    RECT {
        left: 0,
        top: (height - scale(HEIGHT, dpi)).max(0),
        right: width.max(0),
        bottom: height.max(0),
    }
}

/// Attach once, on the UI thread, after State and its status STATIC are created.
/// State must outlive the child and the child must be destroyed on this thread.
pub(super) unsafe fn attach(state: *mut State) -> Result<(), String> {
    // SAFETY: run owns a stable boxed State through destruction of all children.
    // All strings are copied by window creation; subclass storage is reclaimed at NCDESTROY.
    unsafe {
        let hwnd = (*state).status_label;
        // A plain STATIC returns HTTRANSPARENT and will not receive the mouse
        // messages TTF_SUBCLASS needs. SS_NOTIFY gives this read-only surface its
        // own hit target without introducing a tab stop or changing its text API.
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style | SS_NOTIFY);
        let tooltip = CreateWindowExW(
            WS_EX_TOPMOST,
            wide("tooltips_class32").as_ptr(),
            null(),
            WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            hwnd,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if tooltip.is_null() {
            return Err(last_error("创建状态栏完整提示"));
        }
        let surface = Box::new(Surface {
            state,
            tooltip,
            tip: wide(&settings::text(hwnd)),
        });
        let mut tool = TTTOOLINFOW {
            cbSize: size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_IDISHWND | TTF_SUBCLASS,
            hwnd,
            uId: hwnd as usize,
            lpszText: surface.tip.as_ptr() as *mut u16,
            ..Default::default()
        };
        // TTM_ADDTOOL retains the text pointer; Surface keeps the allocation live.
        if SendMessageW(
            tooltip,
            TTM_ADDTOOLW,
            0,
            (&mut tool as *mut TTTOOLINFOW) as isize,
        ) == 0
        {
            DestroyWindow(tooltip);
            return Err(last_error("关联状态栏提示"));
        }
        SendMessageW(
            tooltip,
            TTM_SETMAXTIPWIDTH,
            0,
            scale(640, (*state).dpi) as isize,
        );
        let data = Box::into_raw(surface);
        if SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, data as usize) == 0 {
            DestroyWindow(tooltip);
            drop(Box::from_raw(data));
            return Err(last_error("安装状态栏绘制"));
        }
        Ok(())
    }
}

pub(super) fn layout(hwnd: HWND, dpi: u32, width: i32, height: i32) {
    let rect = bounds(width, height, dpi);
    // SAFETY: layout receives the live child on its owner thread. Positioning only
    // the child cannot recursively resize the main window; no State reference is retained.
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOP,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE,
        );
        InvalidateRect(hwnd, null(), 1);
    }
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: subclass is installed only on the owner-thread status STATIC. Its
    // Surface allocation and State are live until this child's WM_NCDESTROY.
    unsafe {
        let surface = data as *mut Surface;
        match message {
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(procedure), id);
                // Destroy the tooltip before freeing its retained text buffer.
                DestroyWindow((*surface).tooltip);
                let result = DefSubclassProc(hwnd, message, w, l);
                drop(Box::from_raw(surface));
                result
            }
            WM_SETTEXT => {
                // Native STATIC remains the source of truth for WM_GETTEXT/MSAA.
                let result = DefSubclassProc(hwnd, message, w, l);
                let tip = wide(&settings::text(hwnd));
                let mut tool = TTTOOLINFOW {
                    cbSize: size_of::<TTTOOLINFOW>() as u32,
                    hwnd,
                    uId: hwnd as usize,
                    lpszText: tip.as_ptr() as *mut u16,
                    ..Default::default()
                };
                SendMessageW((*surface).tooltip, TTM_POP, 0, 0);
                SendMessageW(
                    (*surface).tooltip,
                    TTM_UPDATETIPTEXTW,
                    0,
                    (&mut tool as *mut TTTOOLINFOW) as isize,
                );
                (*surface).tip = tip;
                InvalidateRect(hwnd, null(), 1);
                result
            }
            WM_ERASEBKGND => 1,
            WM_PAINT | WM_PRINTCLIENT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = if message == WM_PAINT {
                    BeginPaint(hwnd, &mut paint)
                } else {
                    w as HDC
                };
                if !dc.is_null() {
                    draw(hwnd, dc, (*surface).state);
                }
                if message == WM_PAINT {
                    EndPaint(hwnd, &paint);
                }
                0
            }
            _ => DefSubclassProc(hwnd, message, w, l),
        }
    }
}

unsafe fn draw(hwnd: HWND, dc: HDC, state: *mut State) {
    // SAFETY: live paint DC/native control and UI-owned fonts/State. SaveDC must
    // succeed before changing DC state; stock brush is borrowed, never deleted.
    unsafe {
        let saved = SaveDC(dc);
        if saved == 0 {
            return;
        }
        let dpi = (*state).dpi;
        let palette = (*state).theme.palette;
        let contrast = (*state).theme.kind == theme::Resolved::Contrast;
        let background = if (*state).theme.kind == theme::Resolved::Light {
            0x00f7f7f7
        } else {
            palette.surface
        };
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect);
        let brush = GetStockObject(DC_BRUSH);
        if !brush.is_null() {
            SetDCBrushColor(dc, background);
            FillRect(dc, &rect, brush);
            let line = RECT {
                bottom: scale(1, dpi),
                ..rect
            };
            SetDCBrushColor(dc, palette.border);
            FillRect(dc, &line, brush);
        }
        SelectObject(dc, (*state).font.0);
        SetBkMode(dc, TRANSPARENT as i32);
        let text = settings::text(hwnd);
        let (status, message) = lines(&text);
        let padding = scale(12, dpi);
        // COLORREF is BGR; text shades also remain legible on each theme's surface.
        let ink = if contrast {
            palette.text
        } else {
            match (status_kind(status), (*state).theme.kind) {
                (StatusKind::Error, theme::Resolved::Light) => 0x001823b4,
                (StatusKind::Error, _) => 0x00959bff,
                (StatusKind::Busy, theme::Resolved::Light) => 0x0000588a,
                (StatusKind::Busy, _) => 0x0078d0ff,
                (StatusKind::Running, theme::Resolved::Light) => 0x003c7316,
                (StatusKind::Running, _) => 0x009bd672,
                (StatusKind::Stopped, _) => palette.disabled,
            }
        };
        if !brush.is_null() {
            SelectObject(dc, brush);
            SelectObject(dc, GetStockObject(NULL_PEN));
            SetDCBrushColor(dc, ink);
            Ellipse(
                dc,
                padding,
                scale(13, dpi),
                padding + scale(8, dpi),
                scale(21, dpi),
            );
        }
        let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;
        let mut top = RECT {
            left: padding + scale(16, dpi),
            top: scale(6, dpi),
            right: (rect.right - padding).max(padding),
            bottom: scale(28, dpi),
        };
        SetTextColor(dc, ink);
        DrawTextW(dc, wide(status).as_ptr(), -1, &mut top, flags);
        let mut bottom = RECT {
            left: padding,
            top: scale(28, dpi),
            bottom: rect.bottom - scale(5, dpi),
            ..top
        };
        SetTextColor(
            dc,
            if contrast {
                palette.text
            } else {
                palette.disabled
            },
        );
        DrawTextW(dc, wide(&message).as_ptr(), -1, &mut bottom, flags);
        RestoreDC(dc, saved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_preserves_status_and_compacts_only_the_visible_detail() {
        assert_eq!(
            lines("代理状态：运行中\r\n已启动\r\n监听正常"),
            ("代理状态：运行中", "已启动 监听正常".into())
        );
        assert_eq!(lines("操作失败"), ("操作失败", String::new()));
        assert_eq!(lines(""), ("", String::new()));
        assert_eq!(
            lines("状态\n路径 C:\\data\\a&b"),
            ("状态", "路径 C:\\data\\a&b".into())
        );
    }

    #[test]
    fn status_classification_prioritizes_errors_then_busy_over_running() {
        for status in [
            "代理错误：运行中",
            "代理启动中：失败",
            "停止失败，正在退出",
            "ERROR: 运行中",
            "代理 error：已停止",
        ] {
            assert_eq!(status_kind(status), StatusKind::Error, "{status}");
        }
        for status in [
            "代理启动中：后台初始化",
            "代理停止中：运行中请求尚未结束",
            "代理重启中",
            "正在停止代理并退出",
            "后台保存并应用中（运行中将受控重启）",
            "后台重新加载配置中……",
        ] {
            assert_eq!(status_kind(status), StatusKind::Busy, "{status}");
        }
        assert_eq!(status_kind("代理状态：运行中"), StatusKind::Running);
    }

    #[test]
    fn status_classification_requires_explicit_running_in_the_first_line() {
        for status in [
            "",
            "代理状态：未运行",
            "代理状态：未运行中",
            "代理状态：已停止",
            "代理状态：未启动",
            "代理状态：未配置",
            "配置已保存",
            "状态未知\r\n运行中",
            "代理状态：已停止\r\n启动失败",
        ] {
            assert_eq!(status_kind(status), StatusKind::Stopped, "{status}");
        }
        assert_eq!(
            status_kind("代理状态：运行中\r\n上次启动失败，正在退出是历史说明"),
            StatusKind::Running
        );
        assert_eq!(status_kind("代理启动中\r\n失败"), StatusKind::Busy);
    }

    #[test]
    fn docked_bounds_fit_client_at_common_dpi() {
        for dpi in [96, 144, 192] {
            let rect = bounds(scale(800, dpi), scale(820, dpi), dpi);
            assert_eq!(rect.left, 0);
            assert_eq!(rect.right, scale(800, dpi));
            assert_eq!(rect.bottom, scale(820, dpi));
            assert_eq!(rect.bottom - rect.top, scale(HEIGHT, dpi));
            // Existing pages reserve at least 84 DIP for their footer.
            assert!(rect.top > scale(820 - 84, dpi));
            assert_eq!(bounds(0, 0, dpi).top, 0);
            assert_eq!(bounds(20, 10, dpi).top, 0);
        }
    }
}
