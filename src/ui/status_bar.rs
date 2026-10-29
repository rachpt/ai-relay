//! A docked status surface over the existing native STATIC. Text updates and
//! accessibility keep their original HWND/WM_SETTEXT contract with the controller.
use super::*;
use windows_sys::Win32::{Graphics::Gdi::*, UI::Controls::*};

pub(super) const HEIGHT: i32 = 40;
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

#[derive(Clone, Copy)]
struct StatusColors {
    icon: u32,
    badge: u32,
    text: u32,
    detail: u32,
}

fn colors(status: StatusKind, kind: theme::Resolved, palette: theme::Palette) -> StatusColors {
    if kind == theme::Resolved::Contrast {
        return StatusColors {
            icon: palette.text,
            badge: palette.background,
            text: palette.text,
            detail: palette.text,
        };
    }
    let icon = match (status, kind) {
        (StatusKind::Error, theme::Resolved::Light) => 0x001c2bc4,
        (StatusKind::Error, _) => 0x00a499ff,
        (StatusKind::Busy, theme::Resolved::Light) => 0x0000528a,
        (StatusKind::Busy, _) => 0x0000e1fc,
        (StatusKind::Running, theme::Resolved::Light) => 0x000f6b0f,
        (StatusKind::Running, _) => 0x005fcb6c,
        (StatusKind::Stopped, _) => palette.disabled,
    };
    StatusColors {
        icon,
        badge: theme::blend(palette.background, icon, 8),
        text: palette.text,
        detail: palette.disabled,
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

unsafe fn status_icon(
    dc: HDC,
    left: i32,
    top: i32,
    size: i32,
    kind: StatusKind,
    ink: u32,
    surface: u32,
) {
    // SAFETY: The caller's saved DC owns all state. Only stock GDI objects are
    // selected; they are restored before returning and never deleted.
    unsafe {
        let brush = GetStockObject(DC_BRUSH);
        let pen = GetStockObject(DC_PEN);
        let no_pen = GetStockObject(NULL_PEN);
        if brush.is_null() || pen.is_null() || no_pen.is_null() {
            return;
        }
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);
        SetDCBrushColor(dc, surface);
        SetDCPenColor(dc, ink);
        Ellipse(dc, left, top, left + size, top + size);
        SetDCBrushColor(dc, ink);
        SetDCPenColor(dc, ink);
        SelectObject(dc, no_pen);
        match kind {
            StatusKind::Running => {
                let points = [
                    POINT {
                        x: left + size * 36 / 100,
                        y: top + size * 28 / 100,
                    },
                    POINT {
                        x: left + size * 36 / 100,
                        y: top + size * 72 / 100,
                    },
                    POINT {
                        x: left + size * 72 / 100,
                        y: top + size * 50 / 100,
                    },
                ];
                Polygon(dc, points.as_ptr(), points.len() as i32);
            }
            StatusKind::Stopped => {
                let inset = size / 4;
                Rectangle(
                    dc,
                    left + inset,
                    top + inset,
                    left + size - inset,
                    top + size - inset,
                );
            }
            StatusKind::Busy => {
                let radius = (size / 8).max(1);
                let cy = top + size / 2;
                for offset in [-size / 4, 0, size / 4] {
                    let cx = left + size / 2 + offset;
                    Ellipse(dc, cx - radius, cy - radius, cx + radius, cy + radius);
                }
            }
            StatusKind::Error => {
                let bar = (size / 8).max(1);
                let cx = left + size / 2;
                Rectangle(
                    dc,
                    cx - bar / 2,
                    top + size / 4,
                    cx + bar - bar / 2,
                    top + size * 62 / 100,
                );
                Ellipse(
                    dc,
                    cx - bar / 2,
                    top + size * 72 / 100,
                    cx + bar - bar / 2,
                    top + size * 72 / 100 + bar,
                );
            }
        }
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
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
        let background = palette.background;
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
        let padding = scale(24, dpi);
        let icon_size = scale(16, dpi);
        let icon_top = rect.top + ((rect.bottom - rect.top - icon_size) / 2).max(0);
        let colors = colors(status_kind(status), (*state).theme.kind, palette);
        status_icon(
            dc,
            padding,
            icon_top,
            icon_size,
            status_kind(status),
            colors.icon,
            colors.badge,
        );
        let text_left = padding + icon_size + scale(8, dpi);
        let text_right = (rect.right - padding).max(text_left);
        if text_right <= text_left {
            RestoreDC(dc, saved);
            return;
        }
        let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;
        let mut measured = RECT {
            left: 0,
            top: 0,
            right: text_right - text_left,
            bottom: rect.bottom - rect.top,
        };
        DrawTextW(
            dc,
            wide(status).as_ptr(),
            -1,
            &mut measured,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        );
        let status_limit = if message.is_empty() {
            text_right - text_left
        } else {
            ((text_right - text_left) * 3 / 5).max(1)
        };
        let status_width = (measured.right - measured.left).clamp(0, status_limit);
        let mut top = RECT {
            left: text_left,
            right: text_left + status_width,
            top: rect.top,
            bottom: rect.bottom,
        };
        SetTextColor(dc, colors.text);
        DrawTextW(dc, wide(status).as_ptr(), -1, &mut top, flags);
        if !message.is_empty() {
            let separator = text_left + status_width + scale(8, dpi);
            let message_left = separator + scale(9, dpi);
            if message_left < text_right {
                if !brush.is_null() {
                    let line = RECT {
                        left: separator,
                        right: separator + scale(1, dpi),
                        top: rect.top + scale(12, dpi),
                        bottom: rect.bottom - scale(12, dpi),
                    };
                    SetDCBrushColor(dc, palette.border);
                    FillRect(dc, &line, brush);
                }
                let mut detail = RECT {
                    left: message_left,
                    right: text_right,
                    top: rect.top,
                    bottom: rect.bottom,
                };
                SetTextColor(dc, colors.detail);
                DrawTextW(dc, wide(&message).as_ptr(), -1, &mut detail, flags);
            }
        }
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

    fn contrast(a: u32, b: u32) -> f64 {
        let luminance = |color: u32| {
            [0.2126, 0.7152, 0.0722]
                .into_iter()
                .enumerate()
                .map(|(index, weight)| {
                    let channel = f64::from((color >> (index * 8)) & 255) / 255.0;
                    weight
                        * if channel <= 0.04045 {
                            channel / 12.92
                        } else {
                            ((channel + 0.055) / 1.055).powf(2.4)
                        }
                })
                .sum::<f64>()
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn status_colors_keep_semantics_in_icons_without_tinting_status_text() {
        for mode in [
            crate::config::ThemeMode::Light,
            crate::config::ThemeMode::Dark,
        ] {
            let theme = theme::Theme::new(mode).unwrap();
            for status in [
                StatusKind::Error,
                StatusKind::Busy,
                StatusKind::Running,
                StatusKind::Stopped,
            ] {
                let colors = colors(status, theme.kind, theme.palette);
                assert_eq!(colors.text, theme.palette.text);
                assert!(contrast(colors.text, theme.palette.background) >= 4.5);
                assert!(contrast(colors.detail, theme.palette.background) >= 4.5);
                assert!(contrast(colors.icon, colors.badge) >= 3.0);
                if theme.kind != theme::Resolved::Contrast {
                    assert_ne!(colors.icon, colors.text);
                    assert_ne!(colors.badge, theme.palette.background);
                }
            }
            let colors = colors(StatusKind::Error, theme::Resolved::Contrast, theme.palette);
            assert_eq!(colors.icon, theme.palette.text);
            assert_eq!(colors.badge, theme.palette.background);
            assert_eq!(colors.detail, theme.palette.text);
        }
    }

    #[test]
    fn docked_bounds_fit_client_at_common_dpi() {
        for dpi in [96, 144, 192] {
            let rect = bounds(scale(800, dpi), scale(820, dpi), dpi);
            assert_eq!(rect.left, 0);
            assert_eq!(rect.right, scale(800, dpi));
            assert_eq!(rect.bottom, scale(820, dpi));
            assert_eq!(rect.bottom - rect.top, scale(HEIGHT, dpi));
            assert_eq!(rect.top, scale(820 - HEIGHT, dpi));
            // Page bodies reserve at least this one-line footer plus their own padding.
            assert!(rect.top > scale(820 - 92, dpi));
            assert_eq!(bounds(0, 0, dpi).top, 0);
            assert_eq!(bounds(20, 10, dpi).top, 0);
        }
    }
}

#[cfg(test)]
mod paint_tests;
