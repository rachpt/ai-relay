//! Read-only owner-drawn history panel, intended for a 696 × 200 DIP (or wider) control.
//! The caller owns query state, fonts, WM_DRAWITEM dispatch and accessible static/table
//! equivalents. This module does not create controls, perform I/O or retain the report.
//! Compact numbers use decimal K/M with up to two decimals; `≈` marks truncation,
//! never missing data. Exact values remain available in the caller's text/table.
use crate::history::{QueryReport, Summary};
use crate::windows::scale;
use windows_sys::Win32::{
    Foundation::{RECT, SIZE},
    Graphics::Gdi::{
        DC_BRUSH, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DrawTextW,
        FillRect, FrameRect, GetStockObject, GetTextExtentPoint32W, HDC, HFONT, IntersectClipRect,
        RestoreDC, SaveDC, SelectObject, SetBkMode, SetDCBrushColor, SetTextColor, TRANSPARENT,
    },
    UI::Controls::DRAWITEMSTRUCT,
};

fn compact(value: Option<u64>) -> String {
    let Some(value) = value else {
        return "未知".into();
    };
    let (unit, suffix) = match value {
        0..1_000 => return value.to_string(),
        1_000..1_000_000 => (1_000, "K"),
        _ => (1_000_000, "M"),
    };
    // Integer truncation cannot round a value across the next unit boundary or
    // lose precision in large u64 totals; widen before scaling to avoid overflow.
    let hundredths = u128::from(value) * 100 / unit;
    let fraction = hundredths % 100;
    let decimal = if fraction == 0 {
        String::new()
    } else if fraction.is_multiple_of(10) {
        format!(".{}", fraction / 10)
    } else {
        format!(".{fraction:02}")
    };
    let approximate = if (u128::from(value) * 100).is_multiple_of(unit) {
        ""
    } else {
        "≈"
    };
    format!("{approximate}{}{decimal}{suffix}", hundredths / 100)
}

// None denotes an undefined proportion, not a zero numerator. Multiplication is
// widened before scaling, so even u64::MAX and inconsistent counts cannot overflow.
fn proportion(value: u64, total: u64, extent: i32) -> Option<i32> {
    if total == 0 {
        return None;
    }
    Some((u128::from(value.min(total)) * extent.max(0) as u128 / u128::from(total)) as i32)
}

fn percentage(value: u64, total: u64) -> String {
    match proportion(value, total, 1000) {
        None => "—".into(),
        Some(0) if value != 0 => "<0.1%".into(),
        Some(n) if n % 10 == 0 => format!("{}%", n / 10),
        Some(n) => format!("{}.{:01}%", n / 10, n % 10),
    }
}

fn visible_buckets(width: i32, cell: i32, available: usize) -> usize {
    ((width.max(0) / cell.max(1)).max(1) as usize)
        .min(12)
        .min(available)
}

fn bucket_lines(label: &str) -> (&str, &str) {
    // Hour labels retain both the calendar date and the UTC offset, including
    // repeated DST hours. Do not reinterpret legacy utc_day as an hour/month.
    label.split_once(' ').unwrap_or((label, ""))
}

fn bucket_value(summary: &Summary) -> String {
    let mut text = compact(summary.tokens.total_tokens);
    if summary.tokens.total_tokens.is_some() && summary.coverage[2] < summary.requests {
        text.push('*');
    }
    text
}

struct SavedDc {
    dc: HDC,
    level: i32,
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        // SAFETY: draw constructs this only after successful SaveDC, and its
        // borrowed WM_DRAWITEM DC remains live until after this guard is dropped.
        unsafe { RestoreDC(self.dc, self.level) };
    }
}

// Constructed only inside draw, with a live borrowed DC/fonts and a SavedDc guard.
struct Canvas {
    dc: HDC,
    font: HFONT,
    heading: HFONT,
    palette: super::theme::Palette,
    bounds: RECT,
    dpi: u32,
}
impl Canvas {
    fn px(&self, value: i32) -> i32 {
        scale(value, self.dpi)
    }

    fn rect(&self, x: i32, y: i32, width: i32, height: i32) -> RECT {
        RECT {
            left: self.bounds.left + x,
            top: self.bounds.top + y,
            right: self.bounds.left + x + width,
            bottom: self.bounds.top + y + height,
        }
    }

    fn fill(&self, rect: RECT, color: u32, frame: bool) {
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return;
        }
        // SAFETY: the DC is borrowed for this paint; rect is live. DC_BRUSH is a
        // system-owned stock object, never deleted or selected here. Its per-DC
        // color is restored by SavedDc, including all early-return paths.
        unsafe {
            let brush = GetStockObject(DC_BRUSH);
            if !brush.is_null() {
                SetDCBrushColor(self.dc, color);
                if frame {
                    FrameRect(self.dc, &rect, brush);
                } else {
                    FillRect(self.dc, &rect, brush);
                }
            }
        }
    }

    fn text(&self, text: &str, mut rect: RECT, heading: bool, centered: bool) {
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return;
        }
        let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        // SAFETY: caller's borrowed fonts/DC remain live; text is terminated and
        // rect is writable for the duration of DrawTextW. No DT_MODIFYSTRING is
        // used. Font selection and text/DC attributes are restored by SavedDc.
        unsafe {
            SelectObject(self.dc, if heading { self.heading } else { self.font });
            DrawTextW(
                self.dc,
                text.as_ptr(),
                -1,
                &mut rect,
                DT_SINGLELINE
                    | DT_VCENTER
                    | DT_END_ELLIPSIS
                    | DT_NOPREFIX
                    | if centered { DT_CENTER } else { 0 },
            );
        }
    }

    fn text_width(&self, text: &str) -> i32 {
        let text: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE { cx: 0, cy: 0 };
        // SAFETY: the borrowed font/DC are live, text has exactly the supplied
        // UTF-16 length, and size is writable. Report labels are bounded calendar
        // strings; the checked conversion also rejects an oversized input.
        unsafe {
            SelectObject(self.dc, self.font);
            if let Ok(length) = i32::try_from(text.len())
                && GetTextExtentPoint32W(self.dc, text.as_ptr(), length, &mut size) != 0
            {
                return size.cx;
            }
        }
        self.px(128)
    }

    fn summaries(&self, report: Option<&QueryReport>, pending: bool, width: i32) {
        let summary = report.filter(|_| !pending).map(|r| &r.summary);
        let titles = [
            "请求",
            "已报告输入 Token",
            "已报告输出 Token",
            "已报告总 Token",
        ];
        for (index, title) in titles.into_iter().enumerate() {
            let x = self.px(8) + width * index as i32 / 4;
            let right = self.px(8) + width * (index as i32 + 1) / 4;
            let cell = right - x - self.px(8);
            self.text(
                title,
                self.rect(x, self.px(4), cell, self.px(20)),
                false,
                false,
            );
            let value = match summary {
                None => "—".into(),
                Some(s) if index == 0 => format!("{} 次", compact(Some(s.requests))),
                Some(s) if s.requests == 0 => "无数据".into(),
                Some(s) => compact(match index {
                    1 => s.tokens.input_tokens,
                    2 => s.tokens.output_tokens,
                    _ => s.tokens.total_tokens,
                }),
            };
            self.text(
                &value,
                self.rect(x, self.px(24), cell, self.px(28)),
                true,
                false,
            );
            let note = match summary {
                _ if pending => "查询中…".into(),
                None => "尚未查询".into(),
                Some(s) if s.requests == 0 => "无匹配请求".into(),
                Some(s) if index == 0 && s.arithmetic_overflows != 0 => "累计曾溢出，见明细".into(),
                Some(_) if index == 0 => "K=千 · M=百万 · ≈近似".into(),
                Some(s) => format!(
                    "覆盖 {} 请求",
                    percentage(s.coverage[index - 1], s.requests)
                ),
            };
            self.text(
                &note,
                self.rect(x, self.px(52), cell, self.px(20)),
                false,
                false,
            );
        }
    }

    fn trend(&self, report: &QueryReport, x: i32, width: i32) {
        if report.trend.is_empty() {
            self.text(
                "总 Token · 无趋势桶（不是 0 Token）",
                self.rect(x, self.px(106), width, self.px(70)),
                false,
                false,
            );
            return;
        }
        // Query reports are chronological. Take only the returned tail, never
        // invent empty calendar buckets or claim truncated reports are up to date.
        let tail = &report.trend[report.trend.len().saturating_sub(12)..];
        let cell = tail.iter().fold(self.px(64), |cell, point| {
            let (date, hour) = bucket_lines(&point.label);
            cell.max(self.text_width(date) + self.px(12))
                .max(self.text_width(hour) + self.px(12))
                .max(self.text_width(&bucket_value(&point.summary)) + self.px(12))
        });
        let count = visible_buckets(width, cell, tail.len());
        let title = format!(
            "总 Token · 返回 {} 桶/显示 {count} · *部分覆盖{}",
            report.trend.len(),
            if report.trend_truncated {
                " · 已截断非全区间"
            } else {
                ""
            }
        );
        self.text(
            &title,
            self.rect(x, self.px(78), width, self.px(20)),
            false,
            false,
        );
        let points = &tail[tail.len() - count..];
        let max = points
            .iter()
            .filter_map(|p| p.summary.tokens.total_tokens)
            .max()
            .unwrap_or(0);
        let base = self.px(154);
        let available = self.px(36);
        for (index, point) in points.iter().enumerate() {
            let left = x + width * index as i32 / count as i32;
            let right = x + width * (index as i32 + 1) / count as i32;
            let cell = right - left;
            let value = point.summary.tokens.total_tokens;
            let height = value.map(|value| {
                if value == 0 {
                    0
                } else {
                    proportion(value, max, available).unwrap_or(0).max(1)
                }
            });
            if let Some(height) = height {
                // A reported zero has a baseline tick and the label 0; an
                // unknown has only the word 未知 and no bar/tick at all.
                self.fill(
                    self.rect(
                        left + cell / 4,
                        base - height.max(1),
                        cell / 2,
                        height.max(1),
                    ),
                    if height == 0 {
                        self.palette.text
                    } else {
                        self.palette.accent
                    },
                    false,
                );
            }
            self.text(
                &bucket_value(&point.summary),
                self.rect(
                    left,
                    base - height.unwrap_or(0) - self.px(20),
                    cell,
                    self.px(20),
                ),
                false,
                true,
            );
            let (date, hour) = bucket_lines(&point.label);
            self.text(date, self.rect(left, base, cell, self.px(20)), false, true);
            self.text(
                hour,
                self.rect(left, base + self.px(20), cell, self.px(20)),
                false,
                true,
            );
        }
    }

    fn completeness(&self, summary: &Summary, x: i32, width: i32) {
        self.text(
            "字段完整度 · 请求占比",
            self.rect(x, self.px(78), width, self.px(20)),
            false,
            false,
        );
        for (index, (name, count)) in [
            ("完整", summary.complete),
            ("部分", summary.partial),
            ("未知", summary.unknown),
        ]
        .into_iter()
        .enumerate()
        {
            let top = self.px(100 + index as i32 * 26);
            self.text(
                &format!(
                    "{name} {} 次 · {}",
                    compact(Some(count)),
                    percentage(count, summary.requests)
                ),
                self.rect(x, top, width, self.px(20)),
                false,
                false,
            );
            let track = self.rect(x, top + self.px(21), width, self.px(4));
            self.fill(track, self.palette.disabled, true);
            if let Some(length) = proportion(count, summary.requests, width)
                && count != 0
            {
                self.fill(
                    RECT {
                        right: track.left + length.max(1),
                        ..track
                    },
                    self.palette.accent,
                    false,
                );
            }
        }
        self.text(
            "字段完整 ≠ 账单完整；不补未知桶",
            self.rect(x, self.px(180), width, self.px(20)),
            false,
            false,
        );
    }
}

/// Paint a non-interactive panel; recommended control height is 200 DIP.
/// `pending` takes precedence over `report`, so stale results are never presented
/// as the current query. None/false means not queried; zero requests means no data.
///
/// # Safety
/// `item.hDC` must be the live DC supplied for the current WM_DRAWITEM, with
/// `rcItem` in its normal pixel coordinate system. Both fonts must remain valid
/// and must not be deleted concurrently. Call only on the owning UI thread.
pub(super) unsafe fn draw(
    item: &DRAWITEMSTRUCT,
    dpi: u32,
    font: HFONT,
    heading: HFONT,
    palette: super::theme::Palette,
    report: Option<&crate::history::QueryReport>,
    pending: bool,
) {
    if item.hDC.is_null()
        || item.rcItem.right <= item.rcItem.left
        || item.rcItem.bottom <= item.rcItem.top
    {
        return;
    }
    // SAFETY: the caller supplies a live WM_DRAWITEM DC. A successful SaveDC is
    // guarded before any mutation; every exit restores clipping, selected fonts,
    // stock-brush color, background mode and text color. No owned GDI objects are
    // allocated, and the borrowed DC/fonts/stock brush are never deleted here.
    unsafe {
        let level = SaveDC(item.hDC);
        if level == 0 {
            return;
        }
        let _saved = SavedDc {
            dc: item.hDC,
            level,
        };
        let bounds = item.rcItem;
        IntersectClipRect(
            item.hDC,
            bounds.left,
            bounds.top,
            bounds.right,
            bounds.bottom,
        );
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, palette.text);
        let canvas = Canvas {
            dc: item.hDC,
            font,
            heading,
            palette,
            bounds,
            dpi: if dpi == 0 { 96 } else { dpi },
        };
        canvas.fill(bounds, palette.surface, false);
        let width = (bounds.right - bounds.left - canvas.px(16)).max(0);
        canvas.summaries(report, pending, width);
        canvas.fill(
            canvas.rect(canvas.px(8), canvas.px(74), width, canvas.px(1)),
            palette.disabled,
            false,
        );
        let message = if pending {
            Some("查询中… 等待当前筛选结果，不显示旧结果")
        } else {
            match report {
                None => Some("尚未查询 · 请先执行历史查询"),
                Some(r) if r.summary.requests == 0 => Some("无匹配记录 · 无数据不代表 0 Token"),
                Some(_) => None,
            }
        };
        if let Some(message) = message {
            canvas.text(
                message,
                canvas.rect(canvas.px(8), canvas.px(90), width, canvas.px(90)),
                false,
                true,
            );
        } else if let Some(report) = report {
            let right_width = canvas.px(224).min(width);
            let left_width = (width - right_width - canvas.px(16)).max(0);
            canvas.trend(report, canvas.px(8), left_width);
            canvas.completeness(
                &report.summary,
                canvas.px(8) + left_width + canvas.px(16),
                right_width,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_distinguishes_unknown_zero_and_large_values() {
        assert_eq!(compact(None), "未知");
        assert_eq!(compact(Some(0)), "0");
        assert_eq!(compact(Some(999)), "999");
        assert_eq!(compact(Some(1_000)), "1K");
        assert_eq!(compact(Some(1_001)), "≈1K");
        assert_eq!(compact(Some(1_250)), "1.25K");
        assert_eq!(compact(Some(1_500)), "1.5K");
        assert_eq!(compact(Some(9_999)), "≈9.99K");
        assert_eq!(compact(Some(10_000)), "10K");
        assert_eq!(compact(Some(999_999)), "≈999.99K");
        assert_eq!(compact(Some(1_000_000)), "1M");
        assert_eq!(compact(Some(1_234_567)), "≈1.23M");
        assert_eq!(compact(Some(2_500_000)), "2.5M");
        assert_eq!(compact(Some(100_000_000)), "100M");
        assert_eq!(compact(Some(u64::MAX)), "≈18446744073709.55M");
    }

    #[test]
    fn ratios_are_bounded_without_u64_overflow() {
        assert_eq!(proportion(0, 0, 100), None);
        assert_eq!(proportion(0, 1, 100), Some(0));
        assert_eq!(proportion(1, 4, 100), Some(25));
        assert_eq!(proportion(u64::MAX, u64::MAX, i32::MAX), Some(i32::MAX));
        assert_eq!(proportion(u64::MAX, 1, 100), Some(100));
        assert_eq!(proportion(1, u64::MAX, 100), Some(0));
        assert_eq!(proportion(1, 1, -1), Some(0));
        assert_eq!(percentage(0, 0), "—");
        assert_eq!(percentage(0, 10), "0%");
        assert_eq!(percentage(1, u64::MAX), "<0.1%");
        assert_eq!(percentage(1, 3), "33.3%");
        assert_eq!(percentage(u64::MAX, u64::MAX), "100%");
    }

    #[test]
    fn labels_keep_hour_offsets_and_missing_coverage_is_explicit() {
        assert_eq!(bucket_lines("2026-10"), ("2026-10", ""));
        assert_eq!(bucket_lines("2026-10-18"), ("2026-10-18", ""));
        assert_eq!(
            bucket_lines("2026-11-01 01:00 -04:00"),
            ("2026-11-01", "01:00 -04:00")
        );
        let mut summary = Summary {
            requests: 2,
            ..Summary::default()
        };
        assert_eq!(bucket_value(&summary), "未知");
        summary.tokens.total_tokens = Some(0);
        summary.coverage[2] = 1;
        assert_eq!(bucket_value(&summary), "0*");
        summary.coverage[2] = 2;
        assert_eq!(bucket_value(&summary), "0");
        summary.tokens.total_tokens = Some(1_234_567);
        summary.coverage[2] = 1;
        assert_eq!(bucket_value(&summary), "≈1.23M*");
    }

    #[test]
    fn narrow_panel_reduces_buckets_at_common_dpi() {
        for dpi in [96, 144, 192] {
            let trend_width = scale(696 - 16 - 224 - 16, dpi);
            assert_eq!(visible_buckets(trend_width, scale(96, dpi), 366), 4);
            assert_eq!(visible_buckets(trend_width, scale(128, dpi), 366), 3);
            assert_eq!(visible_buckets(scale(2000, dpi), scale(96, dpi), 366), 12);
            assert_eq!(visible_buckets(trend_width, scale(96, dpi), 0), 0);
            assert_eq!(visible_buckets(0, 0, 1), 1);
            assert!(scale(194, dpi) <= scale(200, dpi));
        }
    }
}
