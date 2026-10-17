//! Bounded native historical analysis. One in-flight operation; scope and echoed query
//! are checked before accepting a result. Session usage is a separate subview.
use super::*;
use crate::{
    app::controller::{Command, HistoryResponse},
    history::{
        self as db, Counters, CredentialId, GroupBy, Interface, Model, Outcome, Protocol,
        ProviderOrigin, Query, QueryReport, Summary,
    },
};
use std::{fmt::Write, path::PathBuf};
use windows_sys::Win32::{Graphics::Gdi::*, UI::Controls::DRAWITEMSTRUCT};
const SS_OWNERDRAW: u32 = 0x000d;

pub(super) const VIEW: u32 = 2300;
const QUERY: u32 = 2320;
const PREV: u32 = 2321;
const NEXT: u32 = 2322;
const EXPORT: u32 = 2323;
const COUNTERS: u32 = 2324;
const CHART: u32 = 2332;
const RESET: u32 = 2317;

struct Pending {
    scope: Option<PathBuf>,
    generation: u64,
    key: String,
}
impl Pending {
    fn matches_scope(
        &self,
        scope: &Option<PathBuf>,
        generation: u64,
        directory: Option<&PathBuf>,
    ) -> bool {
        self.generation == generation
            && &self.scope == scope
            && directory.is_none_or(|directory| scope.as_ref() == Some(directory))
    }
}
#[derive(Default)]
pub(super) struct History {
    controls: Vec<(HWND, u32, i32, i32, i32, i32)>,
    pub session: bool,
    visible: bool,
    scope: Option<PathBuf>,
    generation: u64,
    pending: Option<Pending>,
    applied: Option<Query>,
    report: Option<Box<QueryReport>>,
    credentials: Vec<CredentialId>,
    file: Option<Receiver<files::Result>>,
    export_query: Option<(PathBuf, u64, Query)>,
    modal: bool,
}
fn number(v: Option<u64>) -> String {
    v.map_or_else(|| "未知".into(), |n| n.to_string())
}
fn summary(s: &Summary) -> String {
    format!(
        "请求 {}；完整 {} / 部分 {} / 未知 {}；截断 {} / 解析错误 {} / 超限 {} / 不支持 {} / 上游错误 {} / 溢出 {}\r\n输入 {} / 输出 {} / 总 {}；缓存命中 {} / 创建 {} / 读取 {}；推理 {}\r\n字段覆盖 [输入/输出/总/缓存命中/推理/创建/读取] {:?}；仅已报告值，非账单，未知不等于 0",
        s.requests,
        s.complete,
        s.partial,
        s.unknown,
        s.truncated,
        s.parse_errors,
        s.parse_limits,
        s.unsupported,
        s.provider_errors,
        s.arithmetic_overflows,
        number(s.tokens.input_tokens),
        number(s.tokens.output_tokens),
        number(s.tokens.total_tokens),
        number(s.tokens.cached_input_tokens),
        number(s.tokens.cache_creation_input_tokens),
        number(s.tokens.cache_read_input_tokens),
        number(s.tokens.reasoning_tokens),
        s.coverage
    )
}
// Gregorian UTC dates without locale or timezone-dependent parsing.
fn date_ms(text: &str) -> Result<Option<i64>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let parts: Vec<_> = text.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return Err("日期格式必须为 YYYY-MM-DD（UTC）。".into());
    }
    let y: i64 = parts[0].parse().map_err(|_| "年份无效")?;
    let m: i64 = parts[1].parse().map_err(|_| "月份无效")?;
    let d: i64 = parts[2].parse().map_err(|_| "日期无效")?;
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1970..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 || d > days[(m - 1) as usize]
    {
        return Err("UTC 日期无效（支持 1970–9999）。".into());
    }
    let y = y - i64::from(m <= 2);
    let era = y / 400;
    let yo = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    Ok(Some(
        (era * 146097 + yo * 365 + yo / 4 - yo / 100 + (153 * mp + 2) / 5 + d - 1 - 719468)
            * 86400000,
    ))
}
fn day_label(day: i64) -> String {
    let z = day + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yo = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yo + era * 400;
    let doy = doe - (365 * yo + yo / 4 - yo / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    format!("{:04}-{:02}-{:02}", y + i64::from(m <= 2), m, d)
}
impl History {
    fn control(&self, id: u32) -> HWND {
        self.controls
            .iter()
            .find(|c| c.1 == id)
            .map_or(null_mut(), |c| c.0)
    }
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        parent: HWND,
        id: u32,
        class: &str,
        title: &str,
        style: u32,
        row: (i32, i32, i32, i32),
    ) -> Result<HWND, String> {
        // SAFETY: UI-owned parent/module and copied strings; ID is not a pointer.
        let h = unsafe {
            CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(title).as_ptr(),
                WS_CHILD | style,
                0,
                0,
                0,
                0,
                parent,
                id as usize as HMENU,
                GetModuleHandleW(null()),
                null(),
            )
        };
        if h.is_null() {
            return Err(last_error("创建历史控件"));
        }
        self.controls.push((h, id, row.0, row.1, row.2, row.3));
        if class == "EDIT" {
            // SAFETY: Live edit, bounded user text.
            unsafe {
                SendMessageW(h, 0xc5, if id >= 2330 { 256 * 1024 } else { 512 }, 0);
            }
        }
        Ok(h)
    }
    fn combo(
        &mut self,
        parent: HWND,
        id: u32,
        items: &[&str],
        row: (i32, i32, i32, i32),
    ) -> Result<(), String> {
        let h = self.add(
            parent,
            id,
            "COMBOBOX",
            "",
            WS_TABSTOP | CBS_DROPDOWNLIST as u32 | WS_VSCROLL,
            row,
        )?;
        // SAFETY: Combo copies strings synchronously; first item exists.
        unsafe {
            for item in items {
                SendMessageW(h, CB_ADDSTRING, 0, wide(item).as_ptr() as isize);
            }
            SendMessageW(h, CB_SETCURSEL, 0, 0);
        }
        Ok(())
    }
    pub fn create(&mut self, parent: HWND) -> Result<(), String> {
        self.combo(
            parent,
            VIEW,
            &["历史持久化分析", "当前会话实时 usage（未持久化）"],
            (24, 60, 360, 180),
        )?;
        self.add(
            parent,
            RESET,
            "BUTTON",
            "重置筛选",
            WS_TABSTOP,
            (488, 60, 232, 30),
        )?;
        self.combo(
            parent,
            2301,
            &[
                "全部时间 UTC",
                "今日 UTC",
                "最近 7 个 UTC 日",
                "最近 30 个 UTC 日",
                "自定义 UTC 日期",
            ],
            (24, 102, 220, 200),
        )?;
        for (id, label, x, y, w) in [
            (2302, "起日 YYYY-MM-DD（含）", 256, 102, 220),
            (2303, "止日 YYYY-MM-DD（不含）", 488, 102, 232),
            (2304, "IP（空=全部）", 24, 164, 220),
            (2305, "响应模型（精确）", 256, 164, 220),
            (2306, "请求模型（精确）", 488, 164, 232),
            (2307, "客户端 credential ID", 24, 226, 220),
            (2308, "实际上游 credential ID", 256, 226, 220),
            (2309, "Provider origin（URL）", 488, 226, 232),
        ] {
            self.add(parent, 0, "STATIC", label, 0, (x, y, w, 22))?;
            self.add(
                parent,
                id,
                "EDIT",
                "",
                WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
                (x, y + 24, w, 28),
            )?;
        }
        self.combo(
            parent,
            2310,
            &["全部协议", "未知", "OpenAI 格式", "Anthropic 格式", "HTTP"],
            (24, 292, 168, 180),
        )?;
        self.combo(
            parent,
            2311,
            &[
                "全部接口",
                "未知",
                "ChatCompletions",
                "Responses",
                "Messages",
                "Forward",
                "CONNECT",
            ],
            (204, 292, 200, 220),
        )?;
        self.combo(
            parent,
            2312,
            &[
                "全部结果",
                "Success",
                "HttpError",
                "Cancelled",
                "Timeout",
                "TransportError",
                "Rejected",
            ],
            (416, 292, 168, 220),
        )?;
        self.add(
            parent,
            2313,
            "EDIT",
            "",
            WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            (596, 292, 124, 28),
        )?;
        self.add(
            parent,
            0,
            "STATIC",
            "三级分组（UTC 时间）；右上输入 HTTP 状态码；凭据请输入本地完整 HMAC ID，不是 API Key",
            0,
            (24, 330, 696, 24),
        )?;
        for (id, x) in [(2314, 24), (2315, 256), (2316, 488)] {
            self.combo(
                parent,
                id,
                &[
                    "不分组",
                    "UTC 日",
                    "UTC 小时",
                    "响应模型",
                    "IP",
                    "客户端凭据",
                    "上游凭据",
                    "接口",
                    "Provider",
                ],
                (x, 356, 220, 240),
            )?;
        }
        for (id, title, x, w) in [
            (QUERY, "查询／应用筛选", 24, 168),
            (PREV, "上一页", 204, 100),
            (NEXT, "下一页", 316, 100),
            (EXPORT, "导出当前明细页…", 428, 168),
            (COUNTERS, "记录状态", 608, 112),
        ] {
            self.add(parent, id, "BUTTON", title, WS_TABSTOP, (x, 398, w, 30))?;
        }
        self.add(
            parent,
            2325,
            "STATIC",
            "历史尚未查询；不可用时将显示后台真实错误。",
            0,
            (24, 436, 696, 44),
        )?;
        self.add(
            parent,
            CHART,
            "STATIC",
            "UTC 每日已报告总 Token 趋势",
            SS_OWNERDRAW,
            (24, 484, 696, 110),
        )?;
        let style = WS_TABSTOP
            | WS_BORDER
            | WS_VSCROLL
            | WS_HSCROLL
            | (ES_MULTILINE | ES_READONLY | ES_AUTOHSCROLL | ES_AUTOVSCROLL) as u32;
        self.add(
            parent,
            2330,
            "EDIT",
            "汇总／分组／趋势：等待真实历史查询",
            style,
            (24, 604, 696, 100),
        )?;
        self.add(
            parent,
            2331,
            "EDIT",
            "分页请求明细：等待真实历史查询",
            style,
            (24, 714, 696, 100),
        )?;
        Ok(())
    }
    pub fn show(&mut self, visible: bool) {
        self.visible = visible;
        for &(h, id, ..) in &self.controls {
            // SAFETY: Live UI-owned children.
            unsafe {
                ShowWindow(
                    h,
                    if visible && (id == VIEW || !self.session) {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
            }
        }
        self.enable();
    }
    fn enable(&self) {
        for &(h, id, ..) in &self.controls {
            // SAFETY: Live child controls; no modal dispatch.
            unsafe {
                settings::EnableWindow(
                    h,
                    i32::from(
                        id == VIEW
                            || ((self.scope.is_some()
                                && self.pending.is_none()
                                && self.file.is_none())
                                && match id {
                                    PREV => self.applied.as_ref().is_some_and(|q| q.offset > 0),
                                    NEXT => self.report.as_ref().is_some_and(|r| {
                                        self.applied.as_ref().is_some_and(|q| {
                                            q.offset.saturating_add(q.limit) <= 100_000
                                                && (u64::from(q.offset) + (r.details.len() as u64)
                                                    < r.summary.requests
                                                    || r.groups.len() == q.limit as usize)
                                        })
                                    }),
                                    EXPORT => self.report.is_some(),
                                    _ => true,
                                }),
                    ),
                );
            }
        }
    }
    pub fn set_font(&self, font: HFONT) {
        for &(h, ..) in &self.controls {
            // SAFETY: State retains font through child lifetime.
            unsafe {
                SendMessageW(h, WM_SETFONT, font as usize, 1);
            }
        }
    }
    pub fn layout(&self, dpi: u32, width: i32, height: i32) {
        // Reserve the parent's persistent status area, including at minimum window size.
        let chart_top = scale(484, dpi);
        let bottom = height - scale(100, dpi);
        let chart_height = ((bottom - chart_top) / 3).clamp(scale(64, dpi), scale(110, dpi));
        let gap = scale(8, dpi);
        let summary_top = chart_top + chart_height + gap;
        let editor_height = ((bottom - summary_top - gap) / 2).max(scale(36, dpi));
        for &(h, id, x, y, w, ht) in &self.controls {
            let (top, height) = match id {
                CHART => (chart_top, chart_height),
                2330 => (summary_top, editor_height),
                2331 => (summary_top + editor_height + gap, editor_height),
                _ => (scale(y, dpi), scale(ht, dpi)),
            };
            // SAFETY: Owned live children only.
            unsafe {
                MoveWindow(
                    h,
                    scale(x, dpi),
                    top,
                    if w == 696 {
                        (width - scale(48, dpi)).max(1)
                    } else {
                        scale(w, dpi)
                    },
                    height,
                    1,
                );
            }
        }
    }
    pub fn scope(&mut self, directory: PathBuf) {
        if self.scope.as_ref() != Some(&directory) {
            self.scope = Some(directory);
            // Keep the old operation in flight until it drains, but never accept it
            // after a directory transition, even if the path later changes back.
            self.generation = self.generation.wrapping_add(1);
            self.applied = None;
            self.report = None;
            self.credentials.clear();
            for id in [2307, 2308] {
                settings::set_text(self.control(id), "");
            }
            settings::set_text(
                self.control(2325),
                "数据目录已变化；旧查询／导出结果不会用于当前目录。",
            );
            settings::set_text(self.control(2330), "数据目录已变化；请重新查询历史。");
            settings::set_text(self.control(2331), "");
            self.invalidate();
            self.enable();
        }
    }
    fn invalidate(&self) {
        // SAFETY: Invalidate only queues painting for a live child.
        unsafe {
            InvalidateRect(self.control(CHART), null(), 1);
        }
    }
    fn draft(&self) -> Result<Query, String> {
        let get = |id| settings::text(self.control(id));
        let mut q = Query {
            limit: 50,
            ..Default::default()
        };
        let day = db::now_ms() / 86400000;
        match settings::selected(self.control(2301)) {
            1 => {
                q.time_from = Some(day * 86400000);
                q.time_to = Some((day + 1) * 86400000);
            }
            2 | 3 => {
                let days = if settings::selected(self.control(2301)) == 2 {
                    7
                } else {
                    30
                };
                q.time_from = Some((day + 1 - days) * 86400000);
                q.time_to = Some((day + 1) * 86400000);
            }
            4 => {
                q.time_from = date_ms(&get(2302))?;
                q.time_to = date_ms(&get(2303))?;
            }
            _ => {}
        }
        if matches!((q.time_from,q.time_to),(Some(a),Some(b)) if a>=b) {
            return Err("UTC 起日必须早于止日（止日不含）。".into());
        }
        if !get(2304).trim().is_empty() {
            q.ip = Some(get(2304).trim().parse().map_err(|_| "IP 无效")?);
        }
        for (id, target) in [(2305, &mut q.model), (2306, &mut q.requested_model)] {
            let s = get(id);
            if !s.trim().is_empty() {
                *target = Some(Model::new(s.trim()).map_err(|_| "模型格式无效")?);
            }
        }
        for (id, target) in [(2307, &mut q.client), (2308, &mut q.upstream)] {
            let s = get(id);
            if !s.trim().is_empty() {
                *target = Some(CredentialId::parse(s.trim()).map_err(
                    |_| "凭据必须是64位十六进制本地 HMAC ID，不能填写 API Key 或前后缀。",
                )?);
            }
        }
        let p = get(2309);
        if !p.trim().is_empty() {
            q.provider =
                Some(ProviderOrigin::new(p.trim()).map_err(|_| "Provider 必须是 HTTP(S) origin")?);
        }
        q.protocol = match settings::selected(self.control(2310)) {
            1 => Some(Protocol::Unknown),
            2 => Some(Protocol::OpenAi),
            3 => Some(Protocol::Anthropic),
            4 => Some(Protocol::Http),
            _ => None,
        };
        q.interface = match settings::selected(self.control(2311)) {
            1 => Some(Interface::Unknown),
            2 => Some(Interface::ChatCompletions),
            3 => Some(Interface::Responses),
            4 => Some(Interface::Messages),
            5 => Some(Interface::Forward),
            6 => Some(Interface::Connect),
            _ => None,
        };
        q.outcome = match settings::selected(self.control(2312)) {
            1 => Some(Outcome::Success),
            2 => Some(Outcome::HttpError),
            3 => Some(Outcome::Cancelled),
            4 => Some(Outcome::Timeout),
            5 => Some(Outcome::TransportError),
            6 => Some(Outcome::Rejected),
            _ => None,
        };
        let status = get(2313);
        if !status.trim().is_empty() {
            q.status = Some(
                status
                    .trim()
                    .parse::<u16>()
                    .ok()
                    .filter(|v| (100..=599).contains(v))
                    .ok_or("状态码应为 100–599")?,
            );
        }
        for id in 2314..=2316 {
            if let Some(group) = match settings::selected(self.control(id)) {
                1 => Some(GroupBy::Day),
                2 => Some(GroupBy::Hour),
                3 => Some(GroupBy::Model),
                4 => Some(GroupBy::Ip),
                5 => Some(GroupBy::Credential),
                6 => Some(GroupBy::UpstreamCredential),
                7 => Some(GroupBy::Interface),
                8 => Some(GroupBy::Provider),
                _ => None,
            } {
                if q.group_by.iter().any(|existing| {
                    std::mem::discriminant(existing) == std::mem::discriminant(&group)
                }) {
                    return Err("分组维度不能重复；请修改第二／三级分组。".into());
                }
                q.group_by.push(group);
            }
        }
        Ok(q)
    }
    fn send(&mut self, controller: &Controller, command: Command, key: String) {
        match controller.send(command) {
            Ok(()) => {
                self.pending = Some(Pending {
                    scope: self.scope.clone(),
                    generation: self.generation,
                    key,
                });
                settings::set_text(
                    self.control(2325),
                    "历史后台操作中；筛选已冻结，单请求在途……",
                );
            }
            Err(e) => settings::set_text(self.control(2325), &e),
        }
        self.enable();
    }
    pub fn handle(&mut self, controller: &Controller, id: u32) {
        if id == VIEW {
            self.session = settings::selected(self.control(VIEW)) == 1;
            self.show(self.visible);
            return;
        }
        if self.pending.is_some() || self.file.is_some() || self.modal {
            return;
        }
        match id {
            RESET => {
                // Reset only draft controls. The applied report/page remains labelled
                // until an explicit query; export always uses the applied query.
                for id in [2301, 2310, 2311, 2312, 2314, 2315, 2316] {
                    // SAFETY: Live native combo, no pointer argument or notification.
                    unsafe {
                        SendMessageW(self.control(id), CB_SETCURSEL, 0, 0);
                    }
                }
                for id in 2302..=2309 {
                    settings::set_text(self.control(id), "");
                }
                settings::set_text(self.control(2313), "");
                settings::set_text(
                    self.control(2325),
                    "筛选草稿已重置；点击查询应用。图表／分页／导出仍使用上次已应用查询。",
                );
            }
            QUERY | PREV | NEXT => {
                let q = if id == QUERY {
                    self.draft()
                } else {
                    self.applied
                        .clone()
                        .ok_or("请先查询历史".into())
                        .map(|mut q| {
                            q.offset = if id == PREV {
                                q.offset.saturating_sub(q.limit)
                            } else {
                                q.offset.saturating_add(q.limit)
                            };
                            q
                        })
                };
                match q {
                    Ok(q) => {
                        let key = format!("{q:?}");
                        self.send(controller, Command::HistoryQuery(q), key);
                    }
                    Err(e) => settings::set_text(self.control(2325), &e),
                }
            }
            COUNTERS => self.send(controller, Command::HistoryCounters, "counters".into()),
            _ => {}
        }
    }
    fn render(&mut self, query: Query, report: Box<QueryReport>, counters: Counters) {
        let mut text = summary(&report.summary);
        for g in &report.groups {
            let _ = write!(
                text,
                "\r\n分组 {}\r\n{}",
                g.keys
                    .iter()
                    .map(|v| v.as_deref().unwrap_or("未知"))
                    .collect::<Vec<_>>()
                    .join(" / "),
                summary(&g.summary)
            );
        }
        let _ = write!(text, "\r\nUTC 日趋势（截断 {}）：", report.trend_truncated);
        for p in &report.trend {
            let _ = write!(
                text,
                "\r\n{} 请求 {} 总Token {}（覆盖 {}）",
                day_label(p.utc_day),
                p.summary.requests,
                number(p.summary.tokens.total_tokens),
                p.summary.coverage[2]
            );
        }
        settings::set_text(self.control(2330), &text);
        let mut detail = String::from(
            "开始 UTC | TCP IP | 请求模型 → 响应模型 | provider | protocol / interface | 状态/结果 | 输入/输出/总 | 首字节/总耗时 | 完整度与异常 | 客户端ID | 上游ID\r\n",
        );
        for r in &report.details {
            for id in [&r.client, &r.upstream].into_iter().flatten() {
                if !self.credentials.contains(id) {
                    self.credentials.push(id.clone());
                }
            }
            let _ = write!(
                detail,
                "{} +{}ms | {} | {} → {} | {} | {:?}/{:?} | {:?}/{:?} | {}/{}/{} | {}/{}ms | {:?} 截断{} 解析错误{} 超限{} 格式{:?} 上游错误{} | {} | {}\r\n",
                day_label(r.started_ms / 86400000),
                r.started_ms % 86400000,
                r.peer_ip,
                r.requested_model.as_ref().map_or("未知", Model::as_str),
                r.report.model.as_deref().unwrap_or("未知"),
                r.provider.as_ref().map_or("未知", ProviderOrigin::as_str),
                r.protocol,
                r.interface,
                r.status,
                r.outcome,
                number(r.report.usage.input_tokens),
                number(r.report.usage.output_tokens),
                number(r.report.usage.total_tokens),
                r.latency_ms,
                r.total_ms,
                r.report.completeness(),
                r.report.truncated,
                r.report.parse_error,
                r.report.limit_exceeded,
                r.report.format,
                r.report.provider_error,
                r.client.as_ref().map_or("未知", CredentialId::as_str),
                r.upstream.as_ref().map_or("未知", CredentialId::as_str)
            );
        }
        self.credentials.truncate(4096);
        settings::set_text(self.control(2331), &detail);
        settings::set_text(
            self.control(2325),
            &format!(
                "历史已查询：匹配 {} 请求；明细 offset {} / {} 行（每页 {}）；记录计数 {:?}；导出仅本页",
                report.summary.requests,
                query.offset,
                report.details.len(),
                query.limit,
                counters
            ),
        );
        self.applied = Some(query);
        self.report = Some(report);
        self.invalidate();
    }
    pub fn poll(&mut self, controller: &Controller) {
        if self.modal {
            return;
        }
        if let Some(result) = self.file.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(files::Result::Error("导出文件选择线程已结束。".into()))
            }
            Err(_) => None,
        }) {
            self.file = None;
            match result {
                files::Result::HistoryPath(path) => {
                    if let Some((scope, generation, query)) = self.export_query.take() {
                        if self.scope.as_ref() != Some(&scope) || self.generation != generation {
                            settings::set_text(
                                self.control(2325),
                                "数据目录已变化；已取消旧目录导出，未写文件。",
                            );
                            self.enable();
                            return;
                        }
                        self.send(
                            controller,
                            Command::HistoryExport {
                                query,
                                path,
                                confirmed: true,
                            },
                            "export".into(),
                        );
                    }
                }
                files::Result::Error(e) => {
                    self.export_query = None;
                    settings::set_text(self.control(2325), &e);
                }
                _ => {
                    self.export_query = None;
                    settings::set_text(self.control(2325), "已取消历史导出，未写文件。");
                }
            }
        }
        while let Ok(event) = controller.history_events.try_recv() {
            let Some(pending) = self.pending.take() else {
                continue;
            };
            let directory = match &event {
                HistoryResponse::QueryResult { directory, .. }
                | HistoryResponse::Exported { directory, .. }
                | HistoryResponse::Counters { directory, .. } => Some(directory),
                HistoryResponse::Error { directory, .. } => directory.as_ref(),
            };
            if !pending.matches_scope(&self.scope, self.generation, directory) {
                settings::set_text(self.control(2325), "已拒绝旧目录历史结果；请重新查询。");
                continue;
            }
            let key = pending.key;
            match event {
                HistoryResponse::QueryResult {
                    query,
                    report,
                    counters,
                    ..
                } if format!("{query:?}") == key => self.render(query, report, counters),
                HistoryResponse::Exported {
                    path,
                    rows,
                    page_only,
                    counters,
                    ..
                } if key == "export" => settings::set_text(
                    self.control(2325),
                    &format!(
                        "CSV 已导出：{}；{} 行；仅当前页 {}；{:?}",
                        path.display(),
                        rows,
                        page_only,
                        counters
                    ),
                ),
                HistoryResponse::Counters { counters, .. } if key == "counters" => {
                    settings::set_text(self.control(2325), &format!("历史记录状态：{counters:?}"))
                }
                HistoryResponse::Error { message, .. } => {
                    settings::set_text(
                        self.control(2325),
                        &format!("历史不可用／操作失败：{message}"),
                    );
                }
                _ => settings::set_text(
                    self.control(2325),
                    "已拒绝不匹配的过期历史结果；请重新查询。",
                ),
            }
        }
        self.enable();
    }
}
pub(super) unsafe fn command(hwnd: HWND, state: *mut State, id: u32) {
    // SAFETY: Stable UI State; no references cross the modal confirmation.
    unsafe {
        if id == EXPORT {
            if (*state).history.pending.is_some()
                || (*state).history.file.is_some()
                || (*state).history.modal
            {
                return;
            }
            let Some(query) = (*state).history.applied.clone() else {
                return;
            };
            let Some(scope) = (*state).history.scope.clone() else {
                return;
            };
            let generation = (*state).history.generation;
            (*state).history.modal = true;
            let accepted = settings::confirm(
                hwnd,
                &format!(
                    "导出已应用查询的当前明细页：offset {}，最多 {} 行；不是全库导出。CSV 排除 IP、客户端和上游凭据 ID，不含密钥或请求正文。\n后台会按该查询重新读取此页；新记录可能影响页边界。选择一个新文件并确认？",
                    query.offset, query.limit
                ),
            );
            (*state).history.modal = false;
            if !accepted
                || (*state).exiting.is_some()
                || (*state).history.scope.as_ref() != Some(&scope)
                || (*state).history.generation != generation
            {
                return;
            }
            match files::spawn(files::Task::HistoryPath) {
                Ok(rx) => {
                    (*state).history.file = Some(rx);
                    (*state).history.export_query = Some((scope, generation, query));
                }
                Err(e) => settings::set_text((*state).history.control(2325), &e),
            }
            (*state).history.enable();
        } else {
            (*state).history.handle(&(*state).controller, id);
            let visible = (*state).tabs.active == 1 && (*state).history.session;
            (*state).settings.show_session(visible);
        }
    }
}
pub(super) unsafe fn draw(state: *mut State, l: LPARAM) -> bool {
    // SAFETY: WM_DRAWITEM supplies a live DRAWITEMSTRUCT/DC; all GDI objects restored/freed.
    unsafe {
        let item = &*(l as *const DRAWITEMSTRUCT);
        if item.CtlID != CHART {
            return false;
        }
        let palette = (*state).theme.palette;
        let dc = item.hDC;
        let bg = CreateSolidBrush(palette.surface);
        FillRect(dc, &item.rcItem, bg);
        DeleteObject(bg);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, palette.text);
        let old = SelectObject(dc, (*state).font.0);
        let mut title = item.rcItem;
        let dpi = (*state).dpi;
        title.bottom = title.top + scale(24, dpi);
        DrawTextW(
            dc,
            wide("UTC 每日已报告总 Token（最多显示最近 12 个有记录日；缺失不画 0）").as_ptr(),
            -1,
            &mut title,
            DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        if let Some(report) = &(*state).history.report {
            let points: Vec<_> = report.trend.iter().rev().take(12).rev().collect();
            let max = points
                .iter()
                .filter_map(|p| p.summary.tokens.total_tokens)
                .max()
                .unwrap_or(1)
                .max(1);
            let count = points.len().max(1) as i32;
            let width = (item.rcItem.right - item.rcItem.left) / count;
            let brush = CreateSolidBrush(palette.accent);
            for (i, p) in points.iter().enumerate() {
                let x = item.rcItem.left + i as i32 * width;
                let base = item.rcItem.bottom - scale(22, dpi);
                let mut label = RECT {
                    left: x,
                    top: base,
                    right: x + width,
                    bottom: item.rcItem.bottom,
                };
                DrawTextW(
                    dc,
                    wide(&day_label(p.utc_day)[5..]).as_ptr(),
                    -1,
                    &mut label,
                    DT_CENTER | DT_SINGLELINE,
                );
                let value = p.summary.tokens.total_tokens;
                let available = (base - title.bottom - scale(22, dpi)).max(0);
                let height = value.map(|v| ((v as u128 * available as u128) / max as u128) as i32);
                if let Some(height) = height {
                    let bar = RECT {
                        left: x + width / 4,
                        top: base - height,
                        right: x + width * 3 / 4,
                        bottom: base,
                    };
                    FillRect(dc, &bar, brush);
                }
                let mut value_rect = RECT {
                    left: x,
                    top: base - height.unwrap_or(0) - scale(22, dpi),
                    right: x + width,
                    bottom: base - height.unwrap_or(0),
                };
                DrawTextW(
                    dc,
                    wide(&number(value)).as_ptr(),
                    -1,
                    &mut value_rect,
                    DT_CENTER | DT_SINGLELINE,
                );
            }
            DeleteObject(brush);
            if points.is_empty() {
                let mut r = item.rcItem;
                r.top += scale(30, dpi);
                DrawTextW(
                    dc,
                    wide("无匹配记录（不是 0 Token）").as_ptr(),
                    -1,
                    &mut r,
                    DT_SINGLELINE,
                );
            }
        }
        SelectObject(dc, old);
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utc_dates() {
        assert_eq!(date_ms("1970-01-01").unwrap(), Some(0));
        assert!(date_ms("2025-02-29").is_err());
        for s in ["2024-02-29", "2026-10-17", "2000-01-01"] {
            assert_eq!(day_label(date_ms(s).unwrap().unwrap() / 86400000), s);
        }
    }
    #[test]
    fn rejects_results_after_scope_changes_including_return_to_same_directory() {
        let a = PathBuf::from("scope-a");
        let b = PathBuf::from("scope-b");
        let pending = Pending {
            scope: Some(a.clone()),
            generation: 1,
            key: String::new(),
        };
        assert!(pending.matches_scope(&Some(a.clone()), 1, Some(&a)));
        assert!(!pending.matches_scope(&Some(b.clone()), 2, Some(&a)));
        assert!(!pending.matches_scope(&Some(a.clone()), 3, Some(&a)));
        assert!(!pending.matches_scope(&Some(a.clone()), 1, Some(&b)));
        assert!(pending.matches_scope(&Some(a.clone()), 1, None));
        assert!(!pending.matches_scope(&Some(a), 3, None));
    }
    #[test]
    fn unknown_not_zero() {
        assert!(summary(&Summary::default()).contains("总 未知"));
        assert_eq!(number(Some(0)), "0");
    }
}
