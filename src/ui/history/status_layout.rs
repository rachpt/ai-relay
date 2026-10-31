//! 用原生状态字体及最终宽度计量换行，不以固定两行高度裁剪查询／错误信息。
use super::*;

pub(super) const MIN_HEIGHT: i32 = 34;

pub(super) fn height(control: HWND, dpi: u32, width: i32) -> i32 {
    let minimum = scale(MIN_HEIGHT, dpi);
    if control.is_null() || width <= 0 {
        return minimum;
    }
    let text = wide(&settings::text(control));
    // SAFETY: Owner-thread native STATIC and its borrowed live font. The saved
    // window DC is always restored/released; DrawText borrows bounded UTF-16.
    unsafe {
        let dc = GetDC(control);
        if dc.is_null() {
            return minimum;
        }
        let saved = SaveDC(dc);
        if saved == 0 {
            ReleaseDC(control, dc);
            return minimum;
        }
        let font = SendMessageW(control, WM_GETFONT, 0, 0) as HFONT;
        if !font.is_null() {
            SelectObject(dc, font);
        }
        let mut rect = RECT {
            right: width,
            ..Default::default()
        };
        let measured = DrawTextW(
            dc,
            text.as_ptr(),
            -1,
            &mut rect,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        RestoreDC(dc, saved);
        ReleaseDC(control, dc);
        if measured > 0 {
            rect.bottom.saturating_add(scale(2, dpi)).max(minimum)
        } else {
            minimum
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Geometry {
    pub status_top: i32,
    pub status_height: i32,
    pub chart_top: i32,
    pub sort_top: i32,
    pub table_top: i32,
}

impl Geometry {
    pub fn new(dpi: u32, advanced: bool, status_height: i32) -> Self {
        let status_top = scale(if advanced { 424 } else { 270 }, dpi);
        let status_height = status_height.max(scale(MIN_HEIGHT, dpi));
        let chart_top = status_top + status_height + scale(8, dpi);
        let sort_top = if advanced {
            chart_top
        } else {
            chart_top + scale(200 + 8, dpi)
        };
        Self {
            status_top,
            status_height,
            chart_top,
            sort_top,
            table_top: sort_top + scale(72, dpi),
        }
    }
}
