//! 独立概览仪表盘：只保存有界计数采样，只用 GDI 绘制，不查询数据库或执行 I/O。
//! 接入方负责 2109/SS_OWNERDRAW STATIC、WM_DRAWITEM、显隐及文字可访问性；
//! 隐藏时可继续 record，但不要调用 draw。建议至少 696 × 350 DIP，增高可扩大趋势图。
//! 字体借用调用方提供的正文/标题字体，不创建字体。累计精确值见 description。
use crate::proxy::ProxyMetricsSnapshot;
use std::{collections::VecDeque, time::Duration};

pub(super) const CONTROL_ID: u32 = 2109;
pub(super) const MIN_WIDTH_DIP: i32 = 696;
pub(super) const MIN_HEIGHT_DIP: i32 = 350;
const CAPACITY: usize = 120;
const WINDOW: Duration = Duration::from_secs(120);
const INTERVAL: Duration = Duration::from_secs(1);
const MAX_GAP: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rates {
    requests: f64,
    errors: f64,
    upload: f64,
    download: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unknown {
    First,
    Reset,
    Gap,
    Saturated,
}
impl Unknown {
    fn text(self) -> &'static str {
        match self {
            Self::First => "首个样本：等待约 1 秒建立速率",
            Self::Reset => "计数／时长回退：已重建基线",
            Self::Gap => "采样间隔超过 5 秒：此区间未知",
            Self::Saturated => "计数饱和：此区间速率未知",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    at: Duration,
    rates: Option<Rates>,
    unknown: Option<Unknown>,
}

/// UI 线程持有即可；不含正文、地址、凭据或数据库状态。
/// 每次收到真实快照均可调用；小于 1 秒的更新合并，最多保留 120 个点。
#[derive(Default)]
pub(super) struct Timeline {
    baseline: Option<ProxyMetricsSnapshot>,
    latest: Option<ProxyMetricsSnapshot>,
    samples: VecDeque<Sample>,
}
impl Timeline {
    pub fn record(&mut self, m: &ProxyMetricsSnapshot) {
        let reset = self
            .latest
            .is_some_and(|last| m.uptime < last.uptime || counters_decreased(m, &last));
        self.latest = Some(*m);
        if reset || self.baseline.is_none() {
            self.samples.clear();
            self.baseline = Some(*m);
            self.push(Sample {
                at: m.uptime,
                rates: None,
                unknown: Some(if reset {
                    Unknown::Reset
                } else {
                    Unknown::First
                }),
            });
            return;
        }
        let Some(base) = self.baseline else {
            return;
        };
        let Some(elapsed) = m.uptime.checked_sub(base.uptime) else {
            return;
        };
        if elapsed < INTERVAL {
            // Saturation seen in a coalesced update still invalidates this interval.
            if let Some(base) = &mut self.baseline {
                base.saturated |= m.saturated;
            }
            self.prune(m.uptime);
            return;
        }
        let unknown = if base.saturated || m.saturated {
            Some(Unknown::Saturated)
        } else if elapsed > MAX_GAP {
            Some(Unknown::Gap)
        } else {
            None
        };
        let rates = if unknown.is_none() {
            rates_between(&base, m, elapsed)
        } else {
            None
        };
        self.push(Sample {
            at: m.uptime,
            rates,
            unknown: unknown.or_else(|| rates.is_none().then_some(Unknown::Reset)),
        });
        self.baseline = Some(*m);
    }

    fn push(&mut self, sample: Sample) {
        // Evict before insertion; allocation and every drawing pass stay bounded.
        if self.samples.len() == CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
        self.prune(sample.at);
    }

    fn prune(&mut self, now: Duration) {
        while self
            .samples
            .front()
            .is_some_and(|s| now.checked_sub(s.at).is_some_and(|age| age > WINDOW))
        {
            self.samples.pop_front();
        }
    }

    fn current_sample(&self) -> Option<Sample> {
        if self.baseline.is_some_and(|base| base.saturated) {
            return Some(Sample {
                at: self.latest?.uptime,
                rates: None,
                unknown: Some(Unknown::Saturated),
            });
        }
        self.samples.back().copied()
    }

    /// 最新完成区间的平均速率，不把首样本、缺口或未知解释为零。
    /// 接入方可将它与精确累计值设置为原生 STATIC 的 WM_GETTEXT 元数据。
    pub fn rates_text(&self) -> String {
        match self.current_sample() {
            Some(Sample { rates: Some(r), .. }) => format!(
                "最新采样区间平均：请求 {}，错误 {}，上传 {}，下载 {}",
                rate_text(r.requests, false),
                rate_text(r.errors, false),
                rate_text(r.upload, true),
                rate_text(r.download, true),
            ),
            Some(s) => s.unknown.unwrap_or(Unknown::First).text().into(),
            None => "尚无快照：速率未知".into(),
        }
    }

    /// 只含实际指标的精确数值及单位；不读取 OverviewSnapshot 的上游等字段。
    pub fn description(&self) -> String {
        let Some(m) = self.latest else {
            return "概览：等待实际指标快照；无数据不代表零。".into();
        };
        format!(
            "本次程序会话：累计请求 {} 次（不含健康检查）；错误 {} 次（非成功率）；活动请求 {} 个（不含已建立隧道）；活动隧道 {} 个；入站连接 {} 个（含空闲及健康连接）；下载 {} B；上传 {} B。流量为 HTTP 正文／隧道字节，不含 HTTP 头／帧，不保证 TCP 交付，非账单。会话时长 {:.3} 秒（含停止时间）。{}{}。趋势为最近 120 秒内实际采样，超过 5 秒的缺口不连线，未知不补零。",
            m.total_requests,
            m.errors,
            m.active_requests,
            m.active_tunnels,
            m.inbound_connections,
            m.download_body_bytes,
            m.upload_body_bytes,
            m.uptime.as_secs_f64(),
            if m.saturated {
                "至少一个计数已饱和，相关累计仅为下界；"
            } else {
                ""
            },
            self.rates_text(),
        )
    }
}

fn counters_decreased(now: &ProxyMetricsSnapshot, old: &ProxyMetricsSnapshot) -> bool {
    now.total_requests < old.total_requests
        || now.errors < old.errors
        || now.upload_body_bytes < old.upload_body_bytes
        || now.download_body_bytes < old.download_body_bytes
    // Active gauges legitimately decrease and must never reset the timeline.
}

fn rates_between(
    old: &ProxyMetricsSnapshot,
    now: &ProxyMetricsSnapshot,
    elapsed: Duration,
) -> Option<Rates> {
    if elapsed.is_zero() || old.saturated || now.saturated {
        return None;
    }
    let seconds = elapsed.as_secs_f64();
    // Subtract integers before conversion: adjacent near-MAX counters retain
    // their real delta, with neither unsigned overflow nor f64 cancellation.
    Some(Rates {
        requests: now.total_requests.checked_sub(old.total_requests)? as f64 / seconds,
        errors: now.errors.checked_sub(old.errors)? as f64 / seconds,
        upload: now.upload_body_bytes.checked_sub(old.upload_body_bytes)? as f64 / seconds,
        download: now
            .download_body_bytes
            .checked_sub(old.download_body_bytes)? as f64
            / seconds,
    })
}

fn rate_number(value: f64) -> String {
    if value == 0.0 {
        "0".into()
    } else if value < 0.01 {
        "<0.01".into()
    } else if value >= 1_000_000.0 {
        format!("≈{value:.2e}")
    } else {
        format!("{value:.2}")
    }
}
fn rate_text(value: f64, bytes: bool) -> String {
    format!(
        "{} {}",
        rate_number(value),
        if bytes { "B/s" } else { "次/s" }
    )
}

// Integer-only compact totals; approximation is explicit, exact u64s are in
// description(). Binary bytes and decimal counts never silently mix units.
fn total_text(value: u64, bytes: bool) -> String {
    let radix = if bytes { 1024_u128 } else { 1000_u128 };
    let units = if bytes {
        ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"]
    } else {
        ["", "K", "M", "G", "T", "P", "E"]
    };
    let mut divisor = 1_u128;
    let mut index = 0;
    while index < units.len() - 1 && u128::from(value) >= divisor * radix {
        divisor *= radix;
        index += 1;
    }
    let scaled = u128::from(value) * 100 / divisor;
    let approximate = if (u128::from(value) * 100).is_multiple_of(divisor) {
        ""
    } else {
        "≈"
    };
    let number = if index == 0 {
        value.to_string()
    } else {
        format!("{}.{:02}", scaled / 100, scaled % 100)
    };
    format!(
        "{approximate}{number}{}{}",
        if bytes { " " } else { "" },
        units[index]
    )
}

// Work with relative durations, not absolute f64 uptimes or sample indices.
fn window_fraction(now: Duration, at: Duration) -> Option<f64> {
    let age = now.checked_sub(at)?;
    (age <= WINDOW).then(|| 1.0 - age.as_secs_f64() / WINDOW.as_secs_f64())
}

#[cfg(windows)]
pub(super) use painting::draw;

#[cfg(windows)]
mod painting {
    use super::super::theme::{Palette, Resolved};
    use super::*;
    use crate::windows::scale;
    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{
            DC_BRUSH, DC_PEN, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
            DT_VCENTER, DrawTextW, Ellipse, FillRect, GetStockObject, HDC, HFONT,
            IntersectClipRect, LineTo, MoveToEx, Rectangle, RestoreDC, RoundRect, SaveDC,
            SelectObject, SetBkMode, SetDCBrushColor, SetDCPenColor, SetTextColor, TRANSPARENT,
        },
        UI::Controls::DRAWITEMSTRUCT,
    };

    struct SavedDc {
        dc: HDC,
        level: i32,
    }
    impl Drop for SavedDc {
        fn drop(&mut self) {
            // SAFETY: Constructed after successful SaveDC; WM_DRAWITEM's borrowed
            // DC remains live, including on all early returns from draw.
            unsafe {
                RestoreDC(self.dc, self.level);
            }
        }
    }

    fn blend(a: u32, b: u32, percent: u32) -> u32 {
        let mut color = 0;
        for shift in [0, 8, 16] {
            let channel =
                (((a >> shift) & 255) * (100 - percent) + ((b >> shift) & 255) * percent) / 100;
            color |= channel << shift;
        }
        color
    }

    struct Canvas {
        dc: HDC,
        font: HFONT,
        heading: HFONT,
        dpi: u32,
        palette: Palette,
        kind: Resolved,
        colors: [u32; 4],
    }
    impl Canvas {
        fn px(&self, n: i32) -> i32 {
            scale(n, self.dpi)
        }
        fn rect(&self, parent: RECT, x: i32, y: i32, w: i32, h: i32) -> RECT {
            RECT {
                left: parent.left + x,
                top: parent.top + y,
                right: parent.left + x + w,
                bottom: parent.top + y + h,
            }
        }
        fn fill(&self, rect: RECT, color: u32) {
            if rect.right <= rect.left || rect.bottom <= rect.top {
                return;
            }
            // SAFETY: DC is borrowed under SavedDc; stock objects are never
            // deleted. FillRect borrows the stack RECT only for this call.
            unsafe {
                let brush = GetStockObject(DC_BRUSH);
                if !brush.is_null() {
                    SetDCBrushColor(self.dc, color);
                    FillRect(self.dc, &rect, brush);
                }
            }
        }
        fn panel(&self, rect: RECT, fill: u32, border: u32, radius: i32) {
            // SAFETY: Live guarded DC; draw selected valid stock pen/brush.
            // Their selections and per-DC colors are restored, never deleted.
            unsafe {
                SetDCBrushColor(self.dc, fill);
                SetDCPenColor(self.dc, border);
                RoundRect(
                    self.dc,
                    rect.left,
                    rect.top,
                    rect.right,
                    rect.bottom,
                    self.px(radius),
                    self.px(radius),
                );
            }
        }
        fn text(&self, text: &str, mut rect: RECT, color: u32, heading: bool, align: u32) {
            if rect.right <= rect.left || rect.bottom <= rect.top {
                return;
            }
            let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            // SAFETY: Fonts/DC are borrowed for this paint; terminated UTF-16
            // and writable RECT live through DrawTextW; no DT_MODIFYSTRING.
            unsafe {
                SelectObject(self.dc, if heading { self.heading } else { self.font });
                SetTextColor(self.dc, color);
                DrawTextW(
                    self.dc,
                    text.as_ptr(),
                    -1,
                    &mut rect,
                    DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX | align,
                );
            }
        }
        fn line(&self, a: (i32, i32), b: (i32, i32), color: u32) {
            // SAFETY: Coordinates are bounded by the clipped panel. MoveToEx
            // accepts null for the unused old point; DC/stock pen are guarded.
            unsafe {
                SetDCPenColor(self.dc, color);
                MoveToEx(self.dc, a.0, a.1, std::ptr::null_mut());
                LineTo(self.dc, b.0, b.1);
            }
        }
        fn series_line(&self, a: (i32, i32), b: (i32, i32), color: u32, secondary: bool) {
            if self.kind != Resolved::Contrast || !secondary {
                self.line(a, b, color);
                return;
            }
            // Fixed eight subdivisions avoid per-pixel loops and allocated pens.
            // Dashes and square markers distinguish the second system-color line.
            for part in [0, 2, 4, 6] {
                let point = |step| (a.0 + (b.0 - a.0) * step / 8, a.1 + (b.1 - a.1) * step / 8);
                self.line(point(part), point(part + 1), color);
            }
        }
        fn dot(&self, x: i32, y: i32, color: u32, square: bool) {
            let r = self.px(2).max(2);
            // SAFETY: Valid stock objects are selected in the live guarded DC;
            // dots are bounded and clipping remains within the STATIC.
            unsafe {
                SetDCPenColor(self.dc, color);
                SetDCBrushColor(self.dc, if square { self.palette.surface } else { color });
                if square {
                    Rectangle(self.dc, x - r, y - r, x + r + 1, y + r + 1);
                } else {
                    Ellipse(self.dc, x - r, y - r, x + r + 1, y + r + 1);
                }
            }
        }
        fn icon(&self, rect: RECT, index: usize, color: u32) {
            let x = rect.left + self.px(10);
            let y = rect.top + self.px(10);
            let d = self.px(5);
            match index {
                0 => {
                    for row in [-1, 0, 1] {
                        self.line((x - d, y + row * d), (x + d, y + row * d), color);
                    }
                }
                1 => {
                    self.line((x, y - d), (x, y + self.px(1)), color);
                    self.dot(x, y + d, color, false);
                }
                2 => {
                    for (a, b) in [
                        ((-7, 0), (-3, 0)),
                        ((-3, 0), (0, -5)),
                        ((0, -5), (3, 5)),
                        ((3, 5), (5, 0)),
                        ((5, 0), (8, 0)),
                    ] {
                        self.line(
                            (x + self.px(a.0), y + self.px(a.1)),
                            (x + self.px(b.0), y + self.px(b.1)),
                            color,
                        );
                    }
                }
                _ => {
                    self.line((x, y - d), (x, y + d), color);
                    self.line((x - d, y), (x, y + d), color);
                    self.line((x + d, y), (x, y + d), color);
                }
            }
        }
        fn cards(&self, bounds: RECT, m: Option<&ProxyMetricsSnapshot>) {
            let gap = self.px(12);
            let width = bounds.right - bounds.left;
            let cell = (width - gap * 3) / 4;
            let values = [
                m.map(|m| m.total_requests),
                m.map(|m| m.errors),
                m.map(|m| m.active_requests),
                m.map(|m| m.download_body_bytes),
            ];
            let details = [
                "次 · 不含健康检查".into(),
                "次数 · 非成功率".into(),
                m.map_or_else(
                    || "隧道 — · 入站 —".into(),
                    |m| format!("隧道 {} · 入站 {}", m.active_tunnels, m.inbound_connections),
                ),
                m.map_or_else(
                    || "上传 —".into(),
                    |m| format!("上传 {}", total_text(m.upload_body_bytes, true)),
                ),
            ];
            for i in 0..4 {
                let x = i as i32 * (cell + gap);
                let w = if i == 3 { width - x } else { cell };
                let card = self.rect(bounds, x, 0, w, self.px(112));
                let color = self.colors[i];
                self.panel(card, self.palette.surface, self.palette.border, 12);
                let badge = self.rect(card, self.px(12), self.px(12), self.px(22), self.px(22));
                let tint = if self.kind == Resolved::Contrast {
                    self.palette.surface
                } else {
                    blend(self.palette.surface, color, 12)
                };
                self.panel(badge, tint, tint, 6);
                self.icon(badge, i, color);
                self.text(
                    ["累计请求", "错误次数", "活动请求", "下载累计"][i],
                    self.rect(card, self.px(42), self.px(12), w - self.px(52), self.px(22)),
                    self.palette.text,
                    false,
                    0,
                );
                let value = values[i].map_or_else(|| "—".into(), |v| total_text(v, i == 3));
                self.text(
                    &value,
                    self.rect(card, self.px(12), self.px(40), w - self.px(24), self.px(36)),
                    color,
                    true,
                    0,
                );
                self.text(
                    &details[i],
                    self.rect(card, self.px(12), self.px(82), w - self.px(24), self.px(20)),
                    self.palette.text,
                    false,
                    0,
                );
            }
        }
        fn chart(&self, panel: RECT, timeline: &Timeline, bytes: bool, available: bool) {
            self.panel(panel, self.palette.surface, self.palette.border, 12);
            let w = panel.right - panel.left;
            let h = panel.bottom - panel.top;
            let color = if bytes {
                [self.colors[2], self.colors[0]]
            } else {
                [self.colors[0], self.colors[1]]
            };
            let names = if bytes {
                ["上传", "下载"]
            } else {
                ["请求", "错误"]
            };
            self.text(
                if bytes {
                    "上传 / 下载速率"
                } else {
                    "请求 / 错误速率"
                },
                self.rect(panel, self.px(14), self.px(8), w - self.px(28), self.px(24)),
                self.palette.text,
                false,
                0,
            );
            for i in 0..2 {
                let x = self.px(16) + i as i32 * (w / 2);
                self.series_line(
                    (panel.left + x, panel.top + self.px(43)),
                    (panel.left + x + self.px(18), panel.top + self.px(43)),
                    color[i],
                    i == 1,
                );
                self.dot(
                    panel.left + x + self.px(9),
                    panel.top + self.px(43),
                    color[i],
                    i == 1,
                );
                self.text(
                    names[i],
                    self.rect(
                        panel,
                        x + self.px(24),
                        self.px(32),
                        w / 2 - self.px(38),
                        self.px(22),
                    ),
                    self.palette.text,
                    false,
                    0,
                );
            }
            let current = available.then(|| timeline.current_sample()).flatten();
            let latest = current.and_then(|s| s.rates).map(|r| {
                if bytes {
                    [r.upload, r.download]
                } else {
                    [r.requests, r.errors]
                }
            });
            let label = latest.map_or_else(
                || "最新：未知".into(),
                |r| {
                    format!(
                        "区间均值 {} / {} {}",
                        rate_number(r[0]),
                        rate_number(r[1]),
                        if bytes { "B/s" } else { "次/s" }
                    )
                },
            );
            self.text(
                &label,
                self.rect(
                    panel,
                    self.px(14),
                    self.px(56),
                    w - self.px(28),
                    self.px(20),
                ),
                self.palette.text,
                false,
                0,
            );
            let plot = self.rect(
                panel,
                self.px(62),
                self.px(108),
                w - self.px(80),
                h - self.px(148),
            );
            let now = timeline.latest.map(|m| m.uptime).unwrap_or_default();
            let mut maximum = 1.0_f64;
            let mut known = false;
            if available {
                for sample in &timeline.samples {
                    if window_fraction(now, sample.at).is_some()
                        && let Some(r) = sample.rates
                    {
                        let values = if bytes {
                            [r.upload, r.download]
                        } else {
                            [r.requests, r.errors]
                        };
                        maximum = maximum.max(values[0]).max(values[1]);
                        known = true;
                    }
                }
            }
            // Y labels always include their units; a unit-sized axis for idle
            // data does not imply a nonzero rate. No per-pixel or unbounded loops.
            for i in 0..3 {
                let y = plot.top + (plot.bottom - plot.top) * i / 2;
                self.line((plot.left, y), (plot.right, y), self.palette.border);
                self.text(
                    &rate_number(maximum * (2 - i) as f64 / 2.0),
                    RECT {
                        left: panel.left + self.px(4),
                        top: y - self.px(10),
                        right: plot.left - self.px(6),
                        bottom: y + self.px(10),
                    },
                    self.palette.text,
                    false,
                    DT_RIGHT,
                );
            }
            self.text(
                if bytes { "B/s" } else { "次/s" },
                self.rect(panel, self.px(6), self.px(76), self.px(52), self.px(18)),
                self.palette.text,
                false,
                DT_RIGHT,
            );
            self.line(
                (plot.left, plot.top),
                (plot.left, plot.bottom),
                self.palette.border,
            );
            for (i, label) in ["-120s", "-60s", "现在"].iter().enumerate() {
                let x = plot.left + (plot.right - plot.left) * i as i32 / 2;
                self.text(
                    label,
                    RECT {
                        left: x - self.px(26),
                        top: plot.bottom + self.px(6),
                        right: x + self.px(26),
                        bottom: plot.bottom + self.px(28),
                    },
                    self.palette.text,
                    false,
                    DT_CENTER,
                );
            }
            if !available || !known {
                let message = if !available {
                    "等待实际快照 · 无历史"
                } else {
                    current
                        .and_then(|s| s.unknown)
                        .unwrap_or(Unknown::First)
                        .text()
                };
                self.text(message, plot, self.palette.text, false, DT_CENTER);
                return;
            }
            for series in 0..2 {
                let mut previous: Option<((i32, i32), Duration)> = None;
                for sample in &timeline.samples {
                    let Some(fraction) = window_fraction(now, sample.at) else {
                        previous = None;
                        continue;
                    };
                    let Some(r) = sample.rates else {
                        previous = None;
                        continue;
                    };
                    let values = if bytes {
                        [r.upload, r.download]
                    } else {
                        [r.requests, r.errors]
                    };
                    let x = plot.left + ((plot.right - plot.left) as f64 * fraction).round() as i32;
                    let y = plot.bottom
                        - ((plot.bottom - plot.top) as f64
                            * (values[series] / maximum).clamp(0.0, 1.0))
                        .round() as i32;
                    if let Some((point, at)) = previous
                        && sample.at.checked_sub(at).is_some_and(|gap| gap <= MAX_GAP)
                    {
                        self.series_line(point, (x, y), color[series], series == 1);
                    }
                    self.dot(x, y, color[series], series == 1);
                    previous = Some(((x, y), sample.at));
                }
            }
            if let Some(reason) = current.and_then(|s| s.unknown) {
                // Keep historical valid segments visible, but do not imply that
                // they extend to an unknown current interval.
                self.text(
                    reason.text(),
                    self.rect(
                        panel,
                        self.px(64),
                        self.px(77),
                        w - self.px(78),
                        self.px(18),
                    ),
                    self.palette.text,
                    false,
                    0,
                );
            }
        }
    }

    /// Paint only this owner-drawn STATIC, ideally at least 696 × 350 DIP.
    /// `snapshot=None` suppresses stale timeline curves. No collection occurs here.
    ///
    /// # Safety
    /// Call on the owning UI thread with the live WM_DRAWITEM DC/rcItem and
    /// borrowed live fonts. Do not delete fonts concurrently. The caller must
    /// skip this function while the overview/control is hidden.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn draw(
        item: &DRAWITEMSTRUCT,
        dpi: u32,
        font: HFONT,
        heading: HFONT,
        palette: super::super::theme::Palette,
        kind: Resolved,
        snapshot: Option<&crate::app::controller::OverviewSnapshot>,
        timeline: &Timeline,
    ) {
        if item.hDC.is_null()
            || font.is_null()
            || heading.is_null()
            || item.rcItem.right <= item.rcItem.left
            || item.rcItem.bottom <= item.rcItem.top
        {
            return;
        }
        // SAFETY: Caller supplies the live paint DC. SaveDC is guarded before
        // mutation; clip, fonts, stock pen/brush selections/colors, background
        // and text modes are restored on every exit. No GDI object is allocated.
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
            let pen = GetStockObject(DC_PEN);
            let brush = GetStockObject(DC_BRUSH);
            if pen.is_null() || brush.is_null() {
                return;
            }
            SelectObject(item.hDC, pen);
            SelectObject(item.hDC, brush);
            SetBkMode(item.hDC, TRANSPARENT as i32);
            let colors = match kind {
                Resolved::Contrast => [palette.text, palette.text, palette.text, palette.text],
                Resolved::Dark => [palette.accent, 0x008bb2ff, 0x00c4d25e, 0x00efca71],
                Resolved::Light => [palette.accent, 0x00333dc2, 0x007d7711, 0x00a06c12],
            };
            let canvas = Canvas {
                dc: item.hDC,
                font,
                heading,
                dpi: if dpi == 0 { 96 } else { dpi },
                palette,
                kind,
                colors,
            };
            canvas.fill(bounds, palette.background);
            if bounds.right - bounds.left < canvas.px(MIN_WIDTH_DIP)
                || bounds.bottom - bounds.top < canvas.px(MIN_HEIGHT_DIP)
            {
                let mut top = bounds.top;
                let rows = if let Some(s) = snapshot {
                    vec![
                        format!(
                            "累计请求 {} 次    错误 {} 次",
                            s.metrics.total_requests, s.metrics.errors
                        ),
                        format!(
                            "活动请求 {}    隧道 {}    入站连接 {}",
                            s.metrics.active_requests,
                            s.metrics.active_tunnels,
                            s.metrics.inbound_connections
                        ),
                        format!(
                            "上传 {} B    下载 {} B",
                            s.metrics.upload_body_bytes, s.metrics.download_body_bytes
                        ),
                        timeline.rates_text(),
                        "窗口较矮：已保留精确指标，增大窗口可查看趋势图".into(),
                    ]
                } else {
                    vec!["等待实际指标快照；无数据不代表零".into()]
                };
                for row in rows {
                    canvas.text(
                        &row,
                        RECT {
                            top,
                            bottom: (top + canvas.px(24)).min(bounds.bottom),
                            ..bounds
                        },
                        palette.text,
                        false,
                        0,
                    );
                    top += canvas.px(28);
                }
                return;
            }
            canvas.cards(bounds, snapshot.map(|s| &s.metrics));
            let gap = canvas.px(12);
            let width = (bounds.right - bounds.left - gap) / 2;
            let y = canvas.px(124);
            let height = bounds.bottom - bounds.top - y - canvas.px(24);
            // Avoid drawing history from a different/unrecorded snapshot as now.
            let available = snapshot.is_some_and(|s| timeline.latest == Some(s.metrics));
            for i in 0..2 {
                let panel = canvas.rect(
                    bounds,
                    i * (width + gap),
                    y,
                    if i == 1 {
                        bounds.right - bounds.left - width - gap
                    } else {
                        width
                    },
                    height,
                );
                canvas.chart(panel, timeline, i == 1, available);
            }
            canvas.text(
                if snapshot.is_some_and(|s| s.metrics.saturated) {
                    "计数已饱和：相关累计仅为下界 · 速率未知 · 缺口不补零"
                } else {
                    "本次程序会话 · 正文／隧道字节，非线速 · 缺口／未知不补零"
                },
                RECT {
                    left: bounds.left,
                    top: bounds.bottom - canvas.px(22),
                    right: bounds.right,
                    bottom: bounds.bottom,
                },
                palette.text,
                false,
                0,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(ms: u64, requests: u64) -> ProxyMetricsSnapshot {
        ProxyMetricsSnapshot {
            uptime: Duration::from_millis(ms),
            total_requests: requests,
            ..Default::default()
        }
    }
    fn last(t: &Timeline) -> Option<Rates> {
        t.samples.back().and_then(|s| s.rates)
    }

    #[test]
    fn first_sample_is_unknown_not_zero_and_second_uses_interval_delta() {
        let mut t = Timeline::default();
        t.record(&metrics(8_000, 50));
        assert_eq!(last(&t), None);
        assert!(t.rates_text().contains("首个样本"));
        t.record(&metrics(10_000, 56));
        assert_eq!(last(&t).unwrap().requests, 3.0);
        assert_eq!(last(&t).unwrap().errors, 0.0);
    }
    #[test]
    fn duplicate_and_subsecond_updates_coalesce_without_losing_delta() {
        let mut t = Timeline::default();
        t.record(&metrics(0, 5));
        t.record(&metrics(0, 5));
        t.record(&metrics(400, 7));
        t.record(&metrics(999, 9));
        assert_eq!(t.samples.len(), 1);
        t.record(&metrics(1_250, 10));
        assert_eq!(last(&t).unwrap().requests, 4.0);
        assert_eq!(t.samples.len(), 2);
        assert!(t.description().contains("累计请求 10 次"));
    }
    #[test]
    fn gap_is_unknown_then_resumes_without_inventing_zero_or_history() {
        let mut t = Timeline::default();
        t.record(&metrics(0, 0));
        t.record(&metrics(1_000, 2));
        t.record(&metrics(6_001, 20));
        assert_eq!(last(&t), None);
        assert_eq!(t.samples.back().unwrap().unknown, Some(Unknown::Gap));
        t.record(&metrics(7_001, 24));
        assert_eq!(last(&t).unwrap().requests, 4.0);
        assert!(t.samples[2].rates.is_none());
        assert_eq!(t.samples.len(), 4);
    }
    #[test]
    fn five_second_interval_is_valid_but_longer_is_not() {
        let mut t = Timeline::default();
        t.record(&metrics(0, 0));
        t.record(&metrics(5_000, 10));
        assert_eq!(last(&t).unwrap().requests, 2.0);
        t.record(&metrics(10_001, 20));
        assert!(last(&t).is_none());
    }
    #[test]
    fn uptime_decrease_discards_previous_epoch() {
        let mut t = Timeline::default();
        t.record(&metrics(100_000, 20));
        t.record(&metrics(101_000, 21));
        t.record(&metrics(2_000, 30));
        assert_eq!(t.samples.len(), 1);
        assert_eq!(t.samples[0].unknown, Some(Unknown::Reset));
        t.record(&metrics(3_000, 32));
        assert_eq!(last(&t).unwrap().requests, 2.0);
    }
    #[test]
    fn every_cumulative_counter_decrease_resets_even_within_coalescing_window() {
        for index in 0..4 {
            let mut t = Timeline::default();
            let mut m = metrics(0, 20);
            m.errors = 10;
            m.upload_body_bytes = 100;
            m.download_body_bytes = 200;
            t.record(&m);
            m.uptime = Duration::from_millis(250);
            match index {
                0 => m.total_requests -= 1,
                1 => m.errors -= 1,
                2 => m.upload_body_bytes -= 1,
                _ => m.download_body_bytes -= 1,
            }
            t.record(&m);
            assert_eq!(t.samples.len(), 1);
            assert_eq!(t.samples[0].unknown, Some(Unknown::Reset));
            m.uptime += INTERVAL;
            t.record(&m);
            assert_eq!(last(&t).unwrap().requests, 0.0);
        }
    }
    #[test]
    fn gauges_may_decrease_without_resetting_history() {
        let mut t = Timeline::default();
        let mut m = metrics(0, 5);
        m.active_requests = 10;
        m.active_tunnels = 4;
        m.inbound_connections = 15;
        t.record(&m);
        m.uptime = INTERVAL;
        m.active_requests = 0;
        m.active_tunnels = 0;
        m.inbound_connections = 0;
        t.record(&m);
        assert_eq!(t.samples.len(), 2);
        assert_eq!(last(&t).unwrap().requests, 0.0);
        assert!(t.description().contains("活动请求 0 个"));
    }
    #[test]
    fn saturated_intervals_and_coalesced_saturation_are_unknown() {
        let mut t = Timeline::default();
        let mut m = metrics(0, 0);
        t.record(&m);
        m.uptime = Duration::from_millis(300);
        m.saturated = true;
        t.record(&m);
        assert!(t.rates_text().contains("饱和"));
        m.uptime = INTERVAL;
        m.saturated = false;
        t.record(&m);
        assert_eq!(t.samples.back().unwrap().unknown, Some(Unknown::Saturated));
        m.uptime += INTERVAL;
        m.saturated = true;
        t.record(&m);
        assert!(last(&t).is_none());
        assert!(t.description().contains("下界"));
        m.uptime += INTERVAL;
        t.record(&m);
        assert!(last(&t).is_none());
    }
    #[test]
    fn bounded_to_120_samples_and_real_time_window() {
        let mut t = Timeline::default();
        for second in 0..10_000 {
            t.record(&metrics(second * 1_000, second));
            assert!(t.samples.len() <= 120);
        }
        assert_eq!(t.samples.len(), 120);
        assert_eq!(t.samples.front().unwrap().at, Duration::from_secs(9_880));
        t.record(&metrics(10_200_000, 10_000));
        assert_eq!(t.samples.len(), 1);
        assert!(last(&t).is_none());
    }
    #[test]
    fn near_max_counters_preserve_small_deltas_and_large_rates_stay_finite() {
        let mut t = Timeline::default();
        let mut m = metrics(0, u64::MAX - 2);
        m.errors = u64::MAX - 1;
        m.upload_body_bytes = u64::MAX - 100;
        m.download_body_bytes = u64::MAX - 5;
        t.record(&m);
        m.uptime = INTERVAL;
        m.total_requests += 1;
        m.errors += 1;
        m.upload_body_bytes += 50;
        m.download_body_bytes += 5;
        t.record(&m);
        assert_eq!(
            last(&t).unwrap(),
            Rates {
                requests: 1.0,
                errors: 1.0,
                upload: 50.0,
                download: 5.0
            }
        );
        let r = rates_between(&metrics(0, 0), &metrics(1_000, u64::MAX), INTERVAL).unwrap();
        assert!(r.requests.is_finite());
        assert!(t.description().contains(&u64::MAX.to_string()));
    }
    #[test]
    fn x_mapping_grows_from_right_and_subtracts_duration_before_float_conversion() {
        assert_eq!(
            window_fraction(Duration::from_secs(10), Duration::from_secs(10)),
            Some(1.0)
        );
        assert_eq!(
            window_fraction(Duration::from_secs(10), Duration::from_secs(0)),
            Some(11.0 / 12.0)
        );
        assert_eq!(
            window_fraction(Duration::from_secs(180), Duration::from_secs(60)),
            Some(0.0)
        );
        assert_eq!(
            window_fraction(Duration::from_secs(181), Duration::from_secs(60)),
            None
        );
        assert_eq!(
            window_fraction(Duration::from_secs(0), Duration::from_secs(1)),
            None
        );
        let now = Duration::from_secs(u64::MAX);
        assert_eq!(
            window_fraction(now, now - Duration::from_secs(60)),
            Some(0.5)
        );
    }
    #[test]
    fn formatting_distinguishes_unknown_zero_counts_and_byte_units() {
        assert!(Timeline::default().rates_text().contains("未知"));
        assert_eq!(rate_text(0.0, false), "0 次/s");
        assert_eq!(rate_text(0.0, true), "0 B/s");
        assert_eq!(total_text(1_024, true), "1.00 KiB");
        assert_eq!(total_text(1_001, false), "≈1.00K");
        assert_eq!(total_text(1_025, true), "≈1.00 KiB");
        assert!(total_text(u64::MAX, false).starts_with('≈'));
    }
}
