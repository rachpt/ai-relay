//! Bounded native historical analysis. One in-flight operation; scope and echoed query
//! are checked before accepting a result. Session usage is a separate subview.
use super::*;
use crate::{
    app::controller::{Command, HistoryResponse},
    history::{
        self as db, Counters, CredentialAlias, CredentialId, DetailSort, GroupBy, GroupSort,
        Interface, Model, Outcome, Protocol, ProviderOrigin, Query, QueryReport, StorageStats,
        Summary, TimeZone, TokenField, TrendGranularity,
    },
};
use chrono::{Datelike, Local, NaiveDate, TimeZone as ChronoTimeZone, Utc};
use std::{fmt::Write, path::PathBuf};
use windows_sys::Win32::{Graphics::Gdi::*, UI::Controls::*};
const SS_OWNERDRAW: u32 = 0x000d;

pub(super) const VIEW: u32 = 2300;
const QUERY: u32 = 2320;
const PREV: u32 = 2321;
const NEXT: u32 = 2322;
const EXPORT: u32 = 2323;
const COUNTERS: u32 = 2324;
const CHART: u32 = 2332;
const RESET: u32 = 2317;
const ADVANCED: u32 = 2318;
const ZONE: u32 = 2333;
const TREND: u32 = 2334;
const DETAIL_SORT: u32 = 2335;
const GROUP_SORT: u32 = 2336;
const TABLE: u32 = 2337;
const TABLE_VIEW: u32 = 2338;
const MANAGE: u32 = 2339;
const ALIAS_PICK: u32 = 2340;
const ALIAS_ID: u32 = 2341;
const ALIAS_LABEL: u32 = 2342;
const ALIAS_SET: u32 = 2343;
const ALIAS_REMOVE: u32 = 2344;
const STORAGE: u32 = 2345;
const RETENTION: u32 = 2346;
const MAX_RECORDS: u32 = 2347;
const CONFIGURE: u32 = 2348;
const REMOVE_ALIASES: u32 = 2349;
const CLEAR: u32 = 2350;
const STORAGE_TEXT: u32 = 2351;
const CLEAR_CONFIRM: u32 = 2352;
const EXPORT_SCOPE: u32 = 2353;
const MANAGEMENT_LABEL: u32 = 2398;
const ADVANCED_LABEL: u32 = 2399;
// Read-only automation state and an owner-thread queue fence. Neither performs I/O.
const OPERATION_STATE: u32 = 2354;
const QUEUE_FENCE: u32 = 2397;

struct ClearPreview {
    scope: PathBuf,
    generation: u64,
    records: u64,
    remove_aliases: bool,
}

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
    export_query: Option<(PathBuf, u64, Query, bool)>,
    modal: bool,
    advanced: bool,
    management: bool,
    preview: Option<ClearPreview>,
    applied_policy: Option<db::Options>,
    operation_sequence: u64,
    queue_fence: u64,
    rejected_commands: u64,
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

fn date_boundary<T: ChronoTimeZone>(date: NaiveDate, zone: &T) -> Result<i64, String> {
    // Find an instant on this civil date, then let the backend choose its day
    // boundary (earliest fold / first valid instant after a midnight gap).
    for hour in 0..24 {
        let wall = date.and_hms_opt(hour, 0, 0).ok_or("日期超出范围")?;
        if let Some(instant) = zone.from_local_datetime(&wall).earliest() {
            let bucket = db::calendar_bucket_with_zone(
                instant.timestamp_millis(),
                TrendGranularity::Day,
                zone,
            )
            .map_err(|_| "无法解析此日期的时区边界")?;
            return Ok(bucket.bucket_start_ms);
        }
    }
    Err("所选时区不存在此日历日期；请选相邻的有效日期。".into())
}
fn parse_date(text: &str, zone: TimeZone) -> Result<Option<i64>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let date =
        NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| "日期必须为有效 YYYY-MM-DD")?;
    if date.format("%Y-%m-%d").to_string() != text || !(1970..=9999).contains(&date.year()) {
        return Err("日期必须为 YYYY-MM-DD（1970–9999）".into());
    }
    boundary(date, zone).map(Some)
}
fn boundary(date: NaiveDate, zone: TimeZone) -> Result<i64, String> {
    match zone {
        TimeZone::Utc => date_boundary(date, &Utc),
        TimeZone::Local => date_boundary(date, &Local),
    }
}
fn date_range(
    now: i64,
    preset: usize,
    zone: TimeZone,
) -> Result<(Option<i64>, Option<i64>), String> {
    if preset == 0 {
        return Ok((None, None));
    }
    let instant = chrono::DateTime::from_timestamp_millis(now).ok_or("时钟超出范围")?;
    let today = match zone {
        TimeZone::Utc => instant.date_naive(),
        TimeZone::Local => instant.with_timezone(&Local).date_naive(),
    };
    let days = match preset {
        2 => 7,
        3 => 30,
        _ => 1,
    };
    let end = today.succ_opt().ok_or("日期超出范围")?;
    let start = end
        .checked_sub_signed(chrono::Duration::days(days))
        .ok_or("日期超出范围")?;
    Ok((Some(boundary(start, zone)?), Some(boundary(end, zone)?)))
}
fn timestamp(ms: i64, zone: TimeZone) -> String {
    let Some(value) = chrono::DateTime::from_timestamp_millis(ms) else {
        return "无效时间".into();
    };
    match zone {
        TimeZone::Utc => value.format("%Y-%m-%d %H:%M:%S%.3f +00:00").to_string(),
        TimeZone::Local => value
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S%.3f %:z")
            .to_string(),
    }
}
fn group_key(value: Option<&str>, dimension: &GroupBy, query: &Query) -> String {
    let Some(value) = value else {
        return "未知".into();
    };
    // Backend retains numeric UTC day/hour keys for compatibility. Format only
    // the view; never rewrite keys, aggregate values or the returned report.
    let unit = match dimension {
        GroupBy::Day if query.time_zone == TimeZone::Utc => Some(86_400_000),
        GroupBy::Hour if query.time_zone == TimeZone::Utc => Some(3_600_000),
        _ => None,
    };
    if let Some(unit) = unit
        && let Ok(index) = value.parse::<i64>()
        && let Some(ms) = index.checked_mul(unit)
        && let Ok(bucket) = db::calendar_bucket_with_zone(
            ms,
            if unit == 86_400_000 {
                TrendGranularity::Day
            } else {
                TrendGranularity::Hour
            },
            &Utc,
        )
    {
        return bucket.label;
    }
    value.to_string()
}
fn alias_label<'a>(report: &'a QueryReport, id: Option<&CredentialId>) -> &'a str {
    id.and_then(|id| report.aliases.iter().find(|entry| &entry.credential == id))
        .map_or("未设别名", |entry| entry.alias.as_str())
}
fn detail_cells(r: &db::RequestRecord, report: &QueryReport, zone: TimeZone) -> Vec<String> {
    vec![
        timestamp(r.started_ms, zone),
        r.peer_ip.to_string(),
        r.requested_model
            .as_ref()
            .map_or("未知", Model::as_str)
            .into(),
        r.report.model.as_deref().unwrap_or("未知").into(),
        r.provider
            .as_ref()
            .map_or("未知", ProviderOrigin::as_str)
            .into(),
        format!("{:?}/{:?}", r.protocol, r.interface),
        format!("{:?}/{:?}", r.status, r.outcome),
        format!(
            "{}/{}/{}",
            number(r.report.usage.input_tokens),
            number(r.report.usage.output_tokens),
            number(r.report.usage.total_tokens)
        ),
        format!("{}/{}", r.latency_ms, r.total_ms),
        format!(
            "{:?} 截断{} 解析错误{} 超限{} 格式{:?} 上游错误{}",
            r.report.completeness(),
            r.report.truncated,
            r.report.parse_error,
            r.report.limit_exceeded,
            r.report.format,
            r.report.provider_error
        ),
        alias_label(report, r.client.as_ref()).into(),
        r.client
            .as_ref()
            .map_or("未知", CredentialId::as_str)
            .into(),
        alias_label(report, r.upstream.as_ref()).into(),
        r.upstream
            .as_ref()
            .map_or("未知", CredentialId::as_str)
            .into(),
    ]
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
                let limit = match id {
                    2330 | 2331 | STORAGE_TEXT => 256 * 1024,
                    ALIAS_ID => 64,
                    ALIAS_LABEL => 128,
                    RETENTION | MAX_RECORDS => 10,
                    _ => 512,
                };
                SendMessageW(h, 0xc5, limit, 0);
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
        if matches!(id, VIEW | ZONE | TREND | TABLE_VIEW | EXPORT_SCOPE | 2301) {
            let h = choices::create(parent, id, items)?;
            self.controls.push((h, id, row.0, row.1, row.2, 32));
            return Ok(());
        }
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
        self.add(parent, OPERATION_STATE, "STATIC", "", 0, (0, 0, 1, 1))?;
        self.combo(
            parent,
            VIEW,
            &["▥ 历史统计", "◷ 当前会话"],
            (24, 60, 270, 180),
        )?;
        self.add(
            parent,
            RESET,
            "BUTTON",
            "重置筛选",
            WS_TABSTOP,
            (306, 60, 100, 30),
        )?;
        self.add(
            parent,
            ADVANCED,
            "BUTTON",
            "高级筛选 ▾",
            WS_TABSTOP,
            (418, 60, 140, 30),
        )?;
        self.combo(
            parent,
            2301,
            &["全部", "今日", "7 天", "30 天", "自定义"],
            (24, 102, 348, 32),
        )?;
        for (id, label, x, y, w) in [
            (2304, "来源 IP", 24, 164, 220),
            (2305, "响应模型（精确）", 256, 164, 220),
            (2306, "请求模型（精确）", 488, 164, 232),
            (2307, "客户端凭据 ID", 24, 226, 220),
            (2308, "实际上游凭据 ID", 256, 226, 220),
            (2309, "Provider origin", 488, 226, 232),
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
        for (id, label, x) in [(2302, "开始（含）", 24), (2303, "结束（不含）", 372)] {
            let picker = date_picker::create(parent, id, label)?;
            self.controls.push((picker, id, x, 136, 336, 28));
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
        self.add(parent, 0, "STATIC", "HTTP", 0, (596, 286, 36, 26))?;
        self.add(
            parent,
            2313,
            "EDIT",
            "",
            WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            (636, 292, 84, 28),
        )?;
        self.add(
            parent,
            ADVANCED_LABEL,
            "STATIC",
            "三级分组使用所选时区；凭据仅填本地完整 HMAC ID，切勿填写密钥",
            0,
            (24, 322, 696, 24),
        )?;
        for (id, x) in [(2314, 24), (2315, 256), (2316, 488)] {
            self.combo(
                parent,
                id,
                &[
                    "不分组",
                    "日（所选时区）",
                    "小时（含偏移）",
                    "响应模型",
                    "IP",
                    "客户端凭据",
                    "上游凭据",
                    "接口",
                    "Provider",
                    "月（所选时区）",
                    "请求模型",
                    "协议格式",
                ],
                (x, 348, 220, 240),
            )?;
        }
        for (id, title, x, w) in [
            (QUERY, "⌕ 查询 / 应用", 24, 168),
            (PREV, "‹ 上一页", 204, 100),
            (NEXT, "下一页 ›", 316, 100),
            (EXPORT, "导出当前明细页…", 428, 168),
            (COUNTERS, "ⓘ 记录状态", 608, 112),
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
            "所选时区小时／日／月已报告总 Token 趋势",
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
        self.add(
            parent,
            MANAGE,
            "BUTTON",
            "凭据／存储…",
            WS_TABSTOP,
            (570, 60, 150, 30),
        )?;
        self.combo(parent, ZONE, &["UTC", "本地"], (384, 102, 132, 32))?;
        self.combo(parent, TREND, &["日", "小时", "月"], (528, 102, 192, 32))?;
        self.combo(
            parent,
            DETAIL_SORT,
            &["明细：时间↓", "明细：时间↑", "明细：Token↓", "明细：Token↑"],
            (24, 548, 196, 180),
        )?;
        self.combo(
            parent,
            GROUP_SORT,
            &[
                "汇总：分组↑",
                "汇总：分组↓",
                "汇总：请求↓",
                "汇总：请求↑",
                "汇总：时间↓",
                "汇总：时间↑",
                "汇总：Token↓",
                "汇总：Token↑",
            ],
            (232, 548, 196, 240),
        )?;
        self.combo(
            parent,
            TABLE_VIEW,
            &["▤ 请求明细", "▥ 分组汇总", "▧ 汇总文本", "≡ 元数据"],
            (24, 548, 696, 32),
        )?;
        self.combo(
            parent,
            EXPORT_SCOPE,
            &["本页 CSV", "全量 CSV"],
            (440, 548, 280, 32),
        )?;
        // SAFETY: initialize the system-owned list-view class before creation.
        unsafe {
            let init = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            if InitCommonControlsEx(&init) == 0 {
                return Err(last_error("初始化历史表格"));
            }
        }
        let table = self.add(
            parent,
            TABLE,
            "SysListView32",
            "历史请求明细／分页汇总",
            WS_TABSTOP
                | WS_BORDER
                | LVS_REPORT
                | LVS_SHOWSELALWAYS
                | LVS_SINGLESEL
                | LVS_NOSORTHEADER,
            (24, 584, 696, 100),
        )?;
        // SAFETY: live native list view; scalar style flags and no borrowed pointers.
        unsafe {
            SendMessageW(
                table,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                0,
                (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP) as isize,
            );
        }
        for (title, y) in [
            (
                "凭据：从已查询元数据选择或粘贴完整 HMAC ID；别名与 ID 分开，勿输入密钥或片段。",
                110,
            ),
            ("本地 HMAC ID（不可逆标识，不是 API Key）", 180),
            ("用户显示别名（1–64 字；不是凭据筛选值，不得使用密钥）", 246),
            (
                "存储策略：左侧保留天数 1–90；中间记录上限 1–100000。缩小策略可能立即删除旧记录。",
                362,
            ),
        ] {
            self.add(
                parent,
                MANAGEMENT_LABEL,
                "STATIC",
                title,
                0,
                (24, y, 696, 26),
            )?;
        }
        self.combo(
            parent,
            ALIAS_PICK,
            &["选择已查询凭据 ID"],
            (24, 142, 696, 260),
        )?;
        for (id, y, w) in [
            (ALIAS_ID, 210, 696),
            (ALIAS_LABEL, 276, 696),
            (RETENTION, 394, 160),
            (MAX_RECORDS, 394, 180),
        ] {
            let x = if id == MAX_RECORDS { 196 } else { 24 };
            self.add(
                parent,
                id,
                "EDIT",
                "",
                WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
                (x, y, w, 28),
            )?;
        }
        for (id, title, x, y, w) in [
            (ALIAS_SET, "保存别名", 24, 316, 160),
            (ALIAS_REMOVE, "移除别名", 196, 316, 180),
            (STORAGE, "刷新占用／策略", 388, 316, 220),
            (CONFIGURE, "确认并应用策略…", 388, 394, 220),
            (CLEAR, "预览清空历史…", 24, 488, 220),
            (CLEAR_CONFIRM, "确认清空…", 256, 488, 220),
        ] {
            self.add(parent, id, "BUTTON", title, WS_TABSTOP, (x, y, w, 30))?;
        }
        self.add(
            parent,
            REMOVE_ALIASES,
            "BUTTON",
            "清空时同时删除全部别名（默认保留）",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            (24, 446, 696, 30),
        )?;
        self.add(
            parent,
            STORAGE_TEXT,
            "EDIT",
            "点击刷新占用／策略获取后台真实状态。保留 HMAC 身份文件，不删除配置、密钥或日志。",
            style,
            (24, 530, 696, 120),
        )?;
        settings::set_text(self.control(RETENTION), "");
        settings::set_text(self.control(MAX_RECORDS), "");
        for id in 2304..=2309 {
            let hint = "留空表示全部";
            // SAFETY: Live single-line EDIT copies the cue string synchronously.
            unsafe {
                SendMessageW(self.control(id), 0x1501, 0, wide(hint).as_ptr() as isize);
            }
        }
        // SAFETY: Live single-line EDIT; no credential values are used in hints.
        unsafe {
            SendMessageW(
                self.control(2313),
                0x1501,
                0,
                wide("HTTP 状态码").as_ptr() as isize,
            );
        }
        Ok(())
    }
    pub fn show(&mut self, visible: bool) {
        self.visible = visible;
        for &(h, id, _, y, ..) in &self.controls {
            let advanced = (2307..=2316).contains(&id)
                || id == ADVANCED_LABEL
                || (id == 0 && (226..=380).contains(&y));
            let management = (ALIAS_PICK..=STORAGE_TEXT).contains(&id)
                || id == CLEAR_CONFIRM
                || id == MANAGEMENT_LABEL;
            let view = settings::selected(self.control(TABLE_VIEW));
            let content = match id {
                OPERATION_STATE => false,
                2302 | 2303 => settings::selected(self.control(2301)) == 4,
                2330 => view == 2,
                2331 => view == 3,
                TABLE => view <= 1,
                CHART => !self.advanced,
                _ => true,
            };
            let date_label = id == 0 && y == 102;
            // SAFETY: Live UI-owned children.
            unsafe {
                ShowWindow(
                    h,
                    if visible
                        && (id == VIEW
                            || (!self.session
                                && !date_label
                                && content
                                && if management {
                                    self.management
                                } else if id == MANAGE || id == 2325 {
                                    true
                                } else {
                                    !self.management && (!advanced || self.advanced)
                                }))
                    {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
            }
        }
        self.enable();
    }
    fn busy(&self) -> bool {
        self.pending.is_some() || self.file.is_some() || self.modal
    }
    fn publish_operation_state(&self) {
        settings::set_text(
            self.control(OPERATION_STATE),
            &format!(
                "idle={};sequence={};fence={};rejected={};generation={}",
                u8::from(!self.busy() && self.scope.is_some()),
                self.operation_sequence,
                self.queue_fence,
                self.rejected_commands,
                self.generation,
            ),
        );
    }
    fn reject_busy_command(&mut self, id: u32) -> bool {
        if !self.busy() {
            return false;
        }
        self.rejected_commands = self.rejected_commands.wrapping_add(1);
        settings::set_text(
            self.control(2325),
            &format!("历史操作仍在进行；命令 {id} 未执行，请等待完成后重试（未排队或重复发送）。"),
        );
        self.publish_operation_state();
        true
    }
    fn enable(&self) {
        self.publish_operation_state();
        for &(h, id, ..) in &self.controls {
            // SAFETY: Live child controls; no modal dispatch.
            unsafe {
                settings::EnableWindow(
                    h,
                    i32::from(
                        id == VIEW
                            || ((self.scope.is_some()
                                && self.pending.is_none()
                                && self.file.is_none()
                                && !self.modal)
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
                                    CLEAR_CONFIRM => self.preview.as_ref().is_some_and(|p| {
                                        p.generation == self.generation
                                            && self.scope.as_ref() == Some(&p.scope)
                                    }),
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
        let chart_top = scale(312, dpi);
        let bottom = height - scale(92, dpi);
        let chart_height = scale(200, dpi);
        let gap = scale(8, dpi);
        // Expanding filters temporarily replaces the chart, not the usable table.
        let sort_top = if self.advanced {
            scale(466, dpi)
        } else {
            chart_top + chart_height + gap
        };
        let table_top = sort_top + scale(72, dpi);
        let editor_height = (bottom - table_top).max(scale(28, dpi));
        for &(h, id, x, y, w, ht) in &self.controls {
            let (top, height) = match id {
                CHART => (chart_top, chart_height),
                2302 | 2303 => (scale(136, dpi), scale(ht, dpi)),
                2330 | 2331 | TABLE => (table_top, editor_height),
                TABLE_VIEW => (sort_top + scale(36, dpi), scale(ht, dpi)),
                DETAIL_SORT | GROUP_SORT | EXPORT_SCOPE => (sort_top, scale(ht, dpi)),
                CLEAR_CONFIRM => (scale(y, dpi), scale(ht, dpi)),
                STORAGE_TEXT => (
                    scale(y, dpi),
                    (bottom - scale(y + 48, dpi)).max(scale(70, dpi)),
                ),
                _ if (ALIAS_PICK..=STORAGE_TEXT).contains(&id) || id == MANAGEMENT_LABEL => {
                    (scale(y, dpi), scale(ht, dpi))
                }
                QUERY | PREV | NEXT | EXPORT | COUNTERS => (
                    scale(if self.advanced { 386 } else { 232 }, dpi),
                    scale(ht, dpi),
                ),
                2325 => (
                    if self.management {
                        height - scale(132, dpi)
                    } else {
                        scale(if self.advanced { 424 } else { 270 }, dpi)
                    },
                    scale(34, dpi),
                ),
                _ => (scale(if y == 292 { 284 } else { y }, dpi), scale(ht, dpi)),
            };
            // SAFETY: Owned live children only.
            unsafe {
                MoveWindow(
                    h,
                    scale(24, dpi) + ((width - scale(48, dpi)) * (x - 24) / 696),
                    top,
                    ((width - scale(48, dpi)) * w / 696).max(1),
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
            self.preview = None;
            settings::set_text(
                self.control(STORAGE_TEXT),
                "数据目录已变化；请刷新占用和策略。旧预览不可确认。",
            );
            for id in [ALIAS_ID, ALIAS_LABEL, RETENTION, MAX_RECORDS] {
                settings::set_text(self.control(id), "");
            }
            // SAFETY: live list view/combo; messages own no borrowed data.
            unsafe {
                SendMessageW(self.control(TABLE), LVM_DELETEALLITEMS, 0, 0);
                SendMessageW(self.control(ALIAS_PICK), CB_RESETCONTENT, 0, 0);
                SendMessageW(
                    self.control(REMOVE_ALIASES),
                    BM_SETCHECK,
                    BST_UNCHECKED as usize,
                    0,
                );
            }
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
        q.time_zone = if settings::selected(self.control(ZONE)) == 1 {
            TimeZone::Local
        } else {
            TimeZone::Utc
        };
        q.trend_granularity = match settings::selected(self.control(TREND)) {
            1 => TrendGranularity::Hour,
            2 => TrendGranularity::Month,
            _ => TrendGranularity::Day,
        };
        q.detail_sort = match settings::selected(self.control(DETAIL_SORT)) {
            1 => DetailSort::TimeAsc,
            2 => DetailSort::TokensDesc(TokenField::Total),
            3 => DetailSort::TokensAsc(TokenField::Total),
            _ => DetailSort::TimeDesc,
        };
        q.group_sort = match settings::selected(self.control(GROUP_SORT)) {
            1 => GroupSort::KeysDesc,
            2 => GroupSort::CountDesc,
            3 => GroupSort::CountAsc,
            4 => GroupSort::TimeDesc,
            5 => GroupSort::TimeAsc,
            6 => GroupSort::TokensDesc(TokenField::Total),
            7 => GroupSort::TokensAsc(TokenField::Total),
            _ => GroupSort::KeysAsc,
        };
        let preset = settings::selected(self.control(2301));
        if preset == 4 {
            q.time_from = parse_date(&get(2302), q.time_zone)?;
            q.time_to = parse_date(&get(2303), q.time_zone)?;
        } else {
            (q.time_from, q.time_to) = date_range(db::now_ms(), preset, q.time_zone)?;
        }
        if matches!((q.time_from,q.time_to),(Some(a),Some(b)) if a>=b) {
            return Err("起日必须早于止日（使用所选时区，止日不含）。".into());
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
                9 => Some(GroupBy::Month),
                10 => Some(GroupBy::RequestedModel),
                11 => Some(GroupBy::Protocol),
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
        self.preview = None;
        match controller.send(command) {
            Ok(()) => {
                self.operation_sequence = self.operation_sequence.wrapping_add(1);
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
        self.invalidate();
        self.enable();
    }
    pub fn handle(&mut self, controller: &Controller, id: u32) {
        if id == VIEW {
            self.session = settings::selected(self.control(VIEW)) == 1;
            self.show(self.visible);
            return;
        }
        if self.reject_busy_command(id) {
            return;
        }
        match id {
            2301 => self.show(self.visible),
            ZONE | TREND if self.report.is_some() => {
                // Direct choices requery only after a first successful report;
                // no request is sent merely by opening the page.
                self.handle(controller, QUERY);
            }
            MANAGE => {
                self.management = !self.management;
                settings::set_text(
                    self.control(MANAGE),
                    if self.management {
                        "返回统计表格"
                    } else {
                        "凭据／存储…"
                    },
                );
                self.show(self.visible);
                if self.management {
                    self.send(controller, Command::HistoryStorage, "storage".into());
                }
            }
            TABLE_VIEW => {
                self.render_table();
                self.show(self.visible);
            }
            ALIAS_PICK => {
                let index = settings::selected(self.control(ALIAS_PICK));
                if let Some(id) = self.credentials.get(index) {
                    settings::set_text(self.control(ALIAS_ID), id.as_str());
                    settings::set_text(
                        self.control(ALIAS_LABEL),
                        self.report.as_ref().map_or("", |r| {
                            r.aliases
                                .iter()
                                .find(|a| &a.credential == id)
                                .map_or("", |a| a.alias.as_str())
                        }),
                    );
                }
            }
            ALIAS_SET | ALIAS_REMOVE => {
                let operation = self.alias_command(id == ALIAS_REMOVE);
                match operation {
                    Ok(command) => self.send(controller, command, "alias".into()),
                    Err(error) => settings::set_text(self.control(STORAGE_TEXT), &error),
                }
            }
            STORAGE => self.send(controller, Command::HistoryStorage, "storage".into()),
            REMOVE_ALIASES => {
                self.preview = None;
                self.enable();
            }
            CLEAR => {
                self.preview = None;
                // SAFETY: scalar query to a live native checkbox.
                let remove_aliases = unsafe {
                    SendMessageW(self.control(REMOVE_ALIASES), BM_GETCHECK, 0, 0)
                        == BST_CHECKED as isize
                };
                self.send(
                    controller,
                    Command::HistoryClear {
                        confirmed: false,
                        remove_aliases,
                    },
                    "clear-preview".into(),
                );
            }
            ADVANCED => {
                self.advanced = !self.advanced;
                settings::set_text(
                    self.control(ADVANCED),
                    if self.advanced {
                        "收起高级筛选 ▴"
                    } else {
                        "高级筛选 ▾（条件保留）"
                    },
                );
                self.show(self.visible);
            }
            RESET => {
                // Reset only draft controls. The applied report/page remains labelled
                // until an explicit query; export always uses the applied query.
                for id in [
                    2301,
                    2310,
                    2311,
                    2312,
                    2314,
                    2315,
                    2316,
                    ZONE,
                    TREND,
                    DETAIL_SORT,
                    GROUP_SORT,
                ] {
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
                self.show(self.visible);
            }
            QUERY | PREV | NEXT | DETAIL_SORT | GROUP_SORT => {
                let q = if matches!(id, QUERY | DETAIL_SORT | GROUP_SORT) {
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
    fn alias_command(&self, remove: bool) -> Result<Command, String> {
        let credential = CredentialId::parse(settings::text(self.control(ALIAS_ID)).trim())
            .map_err(|_| "必须选择或填写完整64位本地 HMAC ID，不能填写 API Key。")?;
        let alias = if remove {
            None
        } else {
            Some(
                CredentialAlias::new(settings::text(self.control(ALIAS_LABEL)).trim())
                    .map_err(|_| "别名需1–64字且不含控制字符或常见密钥格式；请勿粘贴密钥。")?,
            )
        };
        if !self.credentials.contains(&credential) {
            return Err("请先查询包含该凭据的历史，再从已查询的 HMAC ID 设置别名。".into());
        }
        Ok(Command::HistorySetAlias { credential, alias })
    }
    fn policy(&self) -> Result<(u32, u32), String> {
        let days = settings::text(self.control(RETENTION))
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|v| (1..=90).contains(v))
            .ok_or("保留天数应为1–90")?;
        let records = settings::text(self.control(MAX_RECORDS))
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|v| (1..=100_000).contains(v))
            .ok_or("记录上限应为1–100000")?;
        Ok((days, records))
    }
    fn storage(&self, stats: &StorageStats) {
        settings::set_text(
            self.control(RETENTION),
            &stats.options.retention_days.to_string(),
        );
        settings::set_text(
            self.control(MAX_RECORDS),
            &stats.options.max_records.to_string(),
        );
        settings::set_text(
            self.control(STORAGE_TEXT),
            &format!(
                "历史存储：{} 条记录；{} 个别名\r\n数据库 {} B；WAL {} B；共享内存 {} B；合计 {} B\r\n可复用数据库页 {} B（不代表已回收磁盘空间）。\r\n当前策略：{}天／{}条，先到先清理；不影响真实 usage，不删除配置、密钥或日志。",
                stats.records,
                stats.aliases,
                stats.database_bytes,
                stats.wal_bytes,
                stats.shared_memory_bytes,
                stats.total_bytes(),
                stats.free_database_bytes,
                stats.options.retention_days,
                stats.options.max_records,
            ),
        );
    }
    fn clear_report(&mut self, message: &str) {
        self.applied = None;
        self.report = None;
        settings::set_text(self.control(2330), message);
        settings::set_text(self.control(2331), "");
        // SAFETY: live UI-owned list view; scalar delete message.
        unsafe {
            SendMessageW(self.control(TABLE), LVM_DELETEALLITEMS, 0, 0);
        }
        self.invalidate();
    }
    fn render_table(&self) {
        let grouped = settings::selected(self.control(TABLE_VIEW)) == 1;
        let columns: &[(&str, i32)] = if grouped {
            &[
                ("分组键／HMAC ID", 280),
                ("凭据别名（仅显示）", 160),
                ("请求", 80),
                ("输入", 90),
                ("输出", 90),
                ("总 Token", 100),
                ("总量覆盖", 110),
                ("完整／部分／未知", 150),
                ("截断／解析错／超限", 160),
            ]
        } else {
            &[
                ("开始时间（含 UTC 偏移）", 280),
                ("TCP IP", 130),
                ("请求模型", 150),
                ("响应模型", 150),
                ("Provider", 220),
                ("协议／接口", 200),
                ("状态／结果", 160),
                ("输入／输出／总", 170),
                ("首字节／总耗时 ms", 160),
                ("完整度／异常", 320),
                ("客户端别名", 150),
                ("客户端 HMAC ID", 460),
                ("上游别名", 150),
                ("上游 HMAC ID", 460),
            ]
        };
        let table = self.control(TABLE);
        // SAFETY: live UI-owned list view. Insert/set messages copy UTF-16 text
        // synchronously; stack structs and buffers remain live through each call.
        unsafe {
            let was_visible = GetWindowLongPtrW(table, GWL_STYLE) as u32 & WS_VISIBLE != 0;
            SendMessageW(table, WM_SETREDRAW, 0, 0);
            SendMessageW(table, LVM_DELETEALLITEMS, 0, 0);
            while SendMessageW(table, LVM_DELETECOLUMN, 0, 0) != 0 {}
            let dpi = GetDpiForWindow(table);
            for (index, (title, width)) in columns.iter().enumerate() {
                let mut title = wide(title);
                let column = LVCOLUMNW {
                    mask: LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM,
                    pszText: title.as_mut_ptr(),
                    cx: scale(*width, dpi),
                    iSubItem: index as i32,
                    ..std::mem::zeroed()
                };
                SendMessageW(
                    table,
                    LVM_INSERTCOLUMNW,
                    index,
                    &column as *const _ as isize,
                );
            }
            if let (Some(query), Some(report)) = (&self.applied, &self.report) {
                let rows: Vec<Vec<String>> = if grouped {
                    report
                        .groups
                        .iter()
                        .map(|group| {
                            let labels = group
                                .keys
                                .iter()
                                .zip(&query.group_by)
                                .filter_map(|(key, dimension)| {
                                    if matches!(
                                        dimension,
                                        GroupBy::Credential | GroupBy::UpstreamCredential
                                    ) {
                                        key.as_deref()
                                            .and_then(|value| CredentialId::parse(value).ok())
                                            .map(|id| alias_label(report, Some(&id)).to_string())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join(" / ");
                            let s = &group.summary;
                            vec![
                                group
                                    .keys
                                    .iter()
                                    .zip(&query.group_by)
                                    .map(|(v, dimension)| group_key(v.as_deref(), dimension, query))
                                    .collect::<Vec<_>>()
                                    .join(" / "),
                                labels,
                                s.requests.to_string(),
                                number(s.tokens.input_tokens),
                                number(s.tokens.output_tokens),
                                number(s.tokens.total_tokens),
                                format!("{} / {}", s.coverage[2], s.requests),
                                format!("{} / {} / {}", s.complete, s.partial, s.unknown),
                                format!(
                                    "{} / {} / {}",
                                    s.truncated, s.parse_errors, s.parse_limits
                                ),
                            ]
                        })
                        .collect()
                } else {
                    report
                        .details
                        .iter()
                        .map(|r| detail_cells(r, report, query.time_zone))
                        .collect()
                };
                for (row, cells) in rows.iter().enumerate() {
                    for (col, text) in cells.iter().enumerate() {
                        let mut text = wide(text);
                        let item = LVITEMW {
                            mask: LVIF_TEXT,
                            iItem: row as i32,
                            iSubItem: col as i32,
                            pszText: text.as_mut_ptr(),
                            ..std::mem::zeroed()
                        };
                        SendMessageW(
                            table,
                            if col == 0 {
                                LVM_INSERTITEMW
                            } else {
                                LVM_SETITEMW
                            },
                            0,
                            &item as *const _ as isize,
                        );
                    }
                }
            }
            SendMessageW(table, WM_SETREDRAW, 1, 0);
            // DefWindowProc can add WS_VISIBLE when enabling redraw. A query
            // completing in management/text/session views must not reveal it.
            if !was_visible {
                ShowWindow(table, SW_HIDE);
            }
            InvalidateRect(table, null(), 1);
        }
    }
    fn render(&mut self, query: Query, report: Box<QueryReport>, counters: Counters) {
        self.credentials.clear();
        let mut text = format!(
            "已应用：时区 {:?}；趋势 {:?}；明细排序 {:?}；汇总排序 {:?}；offset {}\r\n{}",
            query.time_zone,
            query.trend_granularity,
            query.detail_sort,
            query.group_sort,
            query.offset,
            summary(&report.summary)
        );
        for g in &report.groups {
            let _ = write!(
                text,
                "\r\n分组 {}\r\n{}",
                g.keys
                    .iter()
                    .zip(&query.group_by)
                    .map(|(v, dimension)| group_key(v.as_deref(), dimension, &query))
                    .collect::<Vec<_>>()
                    .join(" / "),
                summary(&g.summary)
            );
        }
        let _ = write!(
            text,
            "\r\n{:?} {:?}趋势（截断 {}）：",
            query.time_zone, query.trend_granularity, report.trend_truncated
        );
        for p in &report.trend {
            let _ = write!(
                text,
                "\r\n{} 请求 {} 总Token {}（覆盖 {}）",
                p.label,
                p.summary.requests,
                number(p.summary.tokens.total_tokens),
                p.summary.coverage[2]
            );
        }
        settings::set_text(self.control(2330), &text);
        let mut detail = String::from(
            "开始时间（含偏移） | TCP IP | 请求模型 | 响应模型 | provider | protocol / interface | 状态/结果 | 输入/输出/总 | 首字节/总耗时 | 完整度与异常 | 客户端别名 | 上游别名 | 客户端ID | 上游ID\r\n",
        );
        for r in &report.details {
            for id in [&r.client, &r.upstream].into_iter().flatten() {
                if !self.credentials.contains(id) {
                    self.credentials.push(id.clone());
                }
            }
            let cells = detail_cells(r, &report, query.time_zone);
            // Compatibility text keeps the final two columns as full opaque IDs.
            let order = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 11, 13];
            let _ = write!(
                detail,
                "{}\r\n",
                order
                    .iter()
                    .map(|i| cells[*i].as_str())
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
        }
        for group in &report.groups {
            for (key, dimension) in group.keys.iter().zip(&query.group_by) {
                if matches!(dimension, GroupBy::Credential | GroupBy::UpstreamCredential)
                    && let Some(key) = key
                    && let Ok(id) = CredentialId::parse(key)
                    && !self.credentials.contains(&id)
                {
                    self.credentials.push(id);
                }
            }
        }
        for alias in &report.aliases {
            if !self.credentials.contains(&alias.credential) {
                self.credentials.push(alias.credential.clone());
            }
        }
        self.credentials.truncate(4096);
        // SAFETY: live combo copies the opaque identifier strings synchronously.
        unsafe {
            SendMessageW(self.control(ALIAS_PICK), CB_RESETCONTENT, 0, 0);
            for id in &self.credentials {
                SendMessageW(
                    self.control(ALIAS_PICK),
                    CB_ADDSTRING,
                    0,
                    wide(id.as_str()).as_ptr() as isize,
                );
            }
        }
        settings::set_text(self.control(2331), &detail);
        settings::set_text(
            self.control(2325),
            &format!(
                "历史已查询：匹配 {} 请求；明细 offset {} / {} 行（每页 {}）；{:?}/{:?}；汇总本页 {} 组；记录计数 {:?}",
                report.summary.requests,
                query.offset,
                report.details.len(),
                query.limit,
                query.time_zone,
                query.trend_granularity,
                report.groups.len(),
                counters
            ),
        );
        self.applied = Some(query);
        self.report = Some(report);
        self.render_table();
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
                    if let Some((scope, generation, query, all)) = self.export_query.take() {
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
                            if all {
                                Command::HistoryExportAll {
                                    query,
                                    path,
                                    confirmed: true,
                                }
                            } else {
                                Command::HistoryExport {
                                    query,
                                    path,
                                    confirmed: true,
                                }
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
            self.invalidate();
            let directory = match &event {
                HistoryResponse::QueryResult { directory, .. }
                | HistoryResponse::Exported { directory, .. }
                | HistoryResponse::Counters { directory, .. }
                | HistoryResponse::Storage { directory, .. }
                | HistoryResponse::AliasChanged { directory, .. }
                | HistoryResponse::PolicyChanged { directory, .. }
                | HistoryResponse::ClearPreview { directory, .. }
                | HistoryResponse::Cleared { directory, .. } => Some(directory),
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
                HistoryResponse::Storage { stats, .. } if key == "storage" => {
                    self.storage(&stats);
                    settings::set_text(self.control(2325), "历史存储状态已刷新（后台真实值）。");
                }
                HistoryResponse::AliasChanged { .. } if key == "alias" => {
                    settings::set_text(
                        self.control(STORAGE_TEXT),
                        "别名已更新；ID与真实聚合数据不变。重新查询以刷新表格标签。",
                    );
                    if let Some(query) = self.applied.clone() {
                        let key = format!("{query:?}");
                        self.clear_report("别名已更新；正在重新查询真实历史。");
                        self.send(controller, Command::HistoryQuery(query), key);
                    } else {
                        settings::set_text(self.control(2325), "别名已更新，请重新查询。");
                    }
                }
                HistoryResponse::PolicyChanged { report, .. } if key == "policy" => {
                    self.applied_policy = Some(report.options);
                    self.preview = None;
                    self.storage(&report.storage);
                    self.clear_report("策略已更新；旧报表已失效，请重新查询。");
                    settings::set_text(
                        self.control(2325),
                        &format!(
                            "历史策略已保存并应用；删除 {} 条记录；checkpoint {:?}。",
                            report.removed_records, report.checkpoint
                        ),
                    );
                }
                HistoryResponse::ClearPreview {
                    directory,
                    records,
                    remove_aliases,
                } if key == "clear-preview" => {
                    self.preview = Some(ClearPreview {
                        scope: directory,
                        generation: self.generation,
                        records,
                        remove_aliases,
                    });
                    settings::set_text(
                        self.control(STORAGE_TEXT),
                        &format!(
                            "清空预览：整个当前目录历史，约 {records} 条（不限当前筛选）；同时移除别名：{remove_aliases}。\r\n保留 HMAC 身份、配置、密钥、日志；不可撤销。后台写入可能增加实际删除数。\r\n请点击“确认清空…”进行最终确认；未确认不删除。"
                        ),
                    );
                    settings::set_text(self.control(2325), "清空预览已收到；尚未删除任何记录。");
                }
                HistoryResponse::Cleared { remove_aliases, .. } if key == "clear" => {
                    self.preview = None;
                    if remove_aliases {
                        self.credentials.clear();
                        settings::set_text(self.control(ALIAS_ID), "");
                        settings::set_text(self.control(ALIAS_LABEL), "");
                        // SAFETY: live combo, scalar reset message.
                        unsafe {
                            SendMessageW(self.control(ALIAS_PICK), CB_RESETCONTENT, 0, 0);
                        }
                    }
                    self.clear_report(
                        "历史已清空；请重新查询。后续请求仍会写入，不保证目录永远为空。",
                    );
                    settings::set_text(
                        self.control(STORAGE_TEXT),
                        "清空已完成；HMAC 身份／配置／密钥／日志未删除。请刷新存储状态。",
                    );
                    settings::set_text(
                        self.control(2325),
                        &format!("历史已清空；移除别名 {remove_aliases}；后续请求仍会记录。"),
                    );
                }
                HistoryResponse::Error { message, .. } => {
                    self.preview = None;
                    settings::set_text(
                        self.control(2325),
                        &format!("历史不可用／操作失败：{message}"),
                    );
                    if self.management {
                        settings::set_text(self.control(STORAGE_TEXT), &message);
                    }
                }
                _ => settings::set_text(
                    self.control(2325),
                    "已拒绝不匹配的过期历史结果；请重新查询。",
                ),
            }
        }
        self.enable();
    }
    pub(super) fn take_applied_policy(&mut self) -> Option<db::Options> {
        self.applied_policy.take()
    }
}
pub(super) unsafe fn command(hwnd: HWND, state: *mut State, id: u32) {
    // SAFETY: Stable UI State; no references cross the modal confirmation.
    unsafe {
        if id == QUEUE_FENCE {
            (*state).history.queue_fence = (*state).history.queue_fence.wrapping_add(1);
            (*state).history.publish_operation_state();
            return;
        }
        if matches!(id, CONFIGURE | CLEAR_CONFIRM) {
            if (*state).history.reject_busy_command(id) {
                return;
            }
            let Some(scope) = (*state).history.scope.clone() else {
                return;
            };
            let generation = (*state).history.generation;
            let (message, action, key) = if id == CONFIGURE {
                let (retention_days, max_records) = match (*state).history.policy() {
                    Ok(value) => value,
                    Err(error) => {
                        settings::set_text((*state).history.control(STORAGE_TEXT), &error);
                        return;
                    }
                };
                (
                    format!(
                        "将历史策略保存为 {retention_days} 天／{max_records} 条，先到先清理。后台可能立即删除超出范围的历史记录，不可撤销。保留别名、HMAC身份、配置密钥和日志。确认保存并应用？"
                    ),
                    Command::HistoryConfigure {
                        retention_days,
                        max_records,
                        confirmed: true,
                    },
                    "policy",
                )
            } else {
                let Some(preview) = (*state).history.preview.as_ref() else {
                    return;
                };
                if preview.generation != generation || preview.scope != scope {
                    return;
                }
                let remove_aliases =
                    SendMessageW((*state).history.control(REMOVE_ALIASES), BM_GETCHECK, 0, 0)
                        == BST_CHECKED as isize;
                if remove_aliases != preview.remove_aliases {
                    (*state).history.preview = None;
                    (*state).history.enable();
                    return;
                }
                (
                    format!(
                        "后台预览：当前目录全部历史约 {} 条（不受筛选限制）；同时删除全部别名：{}。新写入也可能被删除。保留HMAC身份、配置、密钥和日志。此操作不可撤销，确认清空？",
                        preview.records, remove_aliases
                    ),
                    Command::HistoryClear {
                        confirmed: true,
                        remove_aliases,
                    },
                    "clear",
                )
            };
            (*state).history.modal = true;
            (*state).history.enable();
            let accepted = settings::confirm(hwnd, &message);
            (*state).history.modal = false;
            if accepted
                && (*state).exiting.is_none()
                && (*state).history.scope.as_ref() == Some(&scope)
                && (*state).history.generation == generation
            {
                (*state).history.preview = None;
                (*state)
                    .history
                    .send(&(*state).controller, action, key.into());
            }
            (*state).history.enable();
        } else if id == EXPORT {
            if (*state).history.reject_busy_command(id) {
                return;
            }
            let Some(query) = (*state).history.applied.clone() else {
                return;
            };
            let Some(scope) = (*state).history.scope.clone() else {
                return;
            };
            let generation = (*state).history.generation;
            let all = settings::selected((*state).history.control(EXPORT_SCOPE)) == 1;
            let range = if all {
                "已应用筛选后的全部匹配请求（忽略分页，不是仅当前页）".to_string()
            } else {
                format!(
                    "已应用查询的当前明细页：offset {}，最多 {} 行；不是全量导出",
                    query.offset, query.limit
                )
            };
            (*state).history.modal = true;
            (*state).history.enable();
            let accepted = settings::confirm(
                hwnd,
                &format!(
                    "导出{range}。CSV排除来源IP及客户端／上游凭据HMAC ID，不含密钥或请求正文；全量CSV包含用户别名、请求ID、时间、模型、Provider和用量等可关联元数据。别名不能当作匿名数据，请仅存放到可信位置。\n后台重新读取当前已应用查询，新记录可能影响结果。只创建新文件，不覆盖已有文件。确认后选择路径？"
                ),
            );
            (*state).history.modal = false;
            if !accepted
                || (*state).exiting.is_some()
                || (*state).history.scope.as_ref() != Some(&scope)
                || (*state).history.generation != generation
            {
                (*state).history.enable();
                return;
            }
            match files::spawn(files::Task::HistoryPath) {
                Ok(rx) => {
                    (*state).history.file = Some(rx);
                    (*state).history.export_query = Some((scope, generation, query, all));
                }
                Err(e) => settings::set_text((*state).history.control(2325), &e),
            }
            (*state).history.enable();
        } else {
            (*state).history.handle(&(*state).controller, id);
            let visible = (*state).tabs.active == 1 && (*state).history.session;
            (*state).settings.show_session(visible);
            if matches!(id, ADVANCED | MANAGE | 2301) {
                (*state).layout(hwnd);
                InvalidateRect(hwnd, null(), 1);
            }
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
        super::history_chart::draw(
            item,
            (*state).dpi,
            (*state).font.0,
            (*state).heading_font.0,
            (*state).theme.palette,
            (*state).history.report.as_deref(),
            (*state).history.pending.is_some(),
        );
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_requires_no_query_file_chooser_or_confirmation() {
        let mut history = History::default();
        assert!(!history.busy());
        history.pending = Some(Pending {
            scope: None,
            generation: 0,
            key: "query".into(),
        });
        assert!(history.busy());
        history.pending = None;
        let (_sender, receiver) = std::sync::mpsc::channel();
        history.file = Some(receiver);
        assert!(history.busy());
        history.file = None;
        history.modal = true;
        assert!(history.busy());
        history.modal = false;
        assert!(!history.busy());
    }
    #[test]
    fn utc_dates() {
        assert_eq!(parse_date("1970-01-01", TimeZone::Utc).unwrap(), Some(0));
        assert!(parse_date("2025-02-29", TimeZone::Utc).is_err());
        for s in ["2024-02-29", "2026-10-17", "2000-01-01"] {
            assert!(
                timestamp(
                    parse_date(s, TimeZone::Utc).unwrap().unwrap(),
                    TimeZone::Utc
                )
                .starts_with(s)
            );
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
    fn chart_and_results_fit_minimum_client_at_common_dpi() {
        for dpi in [96, 144, 192] {
            let bottom = scale(880 - 64 - 92, dpi);
            for sort_top in [312 + 200 + 8, 450] {
                let table_top = scale(sort_top + 36, dpi);
                assert!(bottom - table_top >= scale(80, dpi));
            }
        }
    }
    #[derive(Clone)]
    struct Eastern2024;
    impl chrono::TimeZone for Eastern2024 {
        type Offset = chrono::FixedOffset;
        fn from_offset(_: &Self::Offset) -> Self {
            Self
        }
        fn offset_from_local_date(&self, value: &NaiveDate) -> chrono::LocalResult<Self::Offset> {
            self.offset_from_local_datetime(&value.and_hms_opt(0, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(
            &self,
            value: &chrono::NaiveDateTime,
        ) -> chrono::LocalResult<Self::Offset> {
            let standard = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
            let daylight = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
            let spring = NaiveDate::from_ymd_opt(2024, 3, 10)
                .unwrap()
                .and_hms_opt(2, 0, 0)
                .unwrap();
            let fall = NaiveDate::from_ymd_opt(2024, 11, 3)
                .unwrap()
                .and_hms_opt(1, 0, 0)
                .unwrap();
            if *value >= spring && *value < spring + chrono::Duration::hours(1) {
                chrono::LocalResult::None
            } else if *value >= fall && *value < fall + chrono::Duration::hours(1) {
                chrono::LocalResult::Ambiguous(daylight, standard)
            } else if *value >= spring + chrono::Duration::hours(1) && *value < fall {
                chrono::LocalResult::Single(daylight)
            } else {
                chrono::LocalResult::Single(standard)
            }
        }
        fn offset_from_utc_date(&self, value: &NaiveDate) -> Self::Offset {
            self.offset_from_utc_datetime(&value.and_hms_opt(0, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, value: &chrono::NaiveDateTime) -> Self::Offset {
            let ms = value.and_utc().timestamp_millis();
            let spring = chrono::DateTime::parse_from_rfc3339("2024-03-10T07:00:00Z")
                .unwrap()
                .timestamp_millis();
            let fall = chrono::DateTime::parse_from_rfc3339("2024-11-03T06:00:00Z")
                .unwrap()
                .timestamp_millis();
            chrono::FixedOffset::west_opt(if (spring..fall).contains(&ms) {
                4 * 3600
            } else {
                5 * 3600
            })
            .unwrap()
        }
    }
    #[test]
    fn calendar_inputs_use_each_dates_dst_rules_not_current_offset() {
        let date = |month, day| NaiveDate::from_ymd_opt(2024, month, day).unwrap();
        for (month, day, hours) in [(3, 10, 23), (11, 3, 25)] {
            let start = date_boundary(date(month, day), &Eastern2024).unwrap();
            let end = date_boundary(date(month, day + 1), &Eastern2024).unwrap();
            assert_eq!(end - start, hours * 3_600_000);
        }
        let east = chrono::FixedOffset::east_opt(14 * 3600).unwrap();
        assert_eq!(
            date_boundary(date(3, 1), &east).unwrap(),
            chrono::DateTime::parse_from_rfc3339("2024-02-29T10:00:00Z")
                .unwrap()
                .timestamp_millis()
        );
        assert_eq!(parse_date("1970-01-01", TimeZone::Utc).unwrap(), Some(0));
        assert!(parse_date("2024-2-01", TimeZone::Utc).is_err());
        assert!(parse_date("2025-02-29", TimeZone::Local).is_err());
        let now = db::now_ms();
        let (start, end) = date_range(now, 1, TimeZone::Local).unwrap();
        assert!(start.unwrap() <= now && now < end.unwrap());
        assert_eq!(
            start.unwrap(),
            db::calendar_bucket_with_zone(now, TrendGranularity::Day, &Local)
                .unwrap()
                .bucket_start_ms
        );
    }
    #[test]
    fn aliases_are_display_only_and_secret_like_labels_are_rejected() {
        let id = CredentialId::parse(&"ab".repeat(32)).unwrap();
        let report = QueryReport {
            aliases: vec![db::CredentialAliasEntry {
                credential: id.clone(),
                alias: CredentialAlias::new("研发客户端").unwrap(),
            }],
            summary: Summary::default(),
            groups: Vec::new(),
            details: Vec::new(),
            trend: Vec::new(),
            trend_truncated: false,
        };
        assert_eq!(alias_label(&report, Some(&id)), "研发客户端");
        let query = Query::default();
        assert_eq!(group_key(Some("0"), &GroupBy::Day, &query), "1970-01-01");
        assert_eq!(
            group_key(Some("0"), &GroupBy::Hour, &query),
            "1970-01-01 00:00 +00:00"
        );
        assert_eq!(
            group_key(Some(id.as_str()), &GroupBy::Credential, &query),
            id.as_str()
        );
        assert_eq!(id.as_str(), "ab".repeat(32));
        assert!(CredentialAlias::new("sk-not-a-real-key").is_err());
        assert!(CredentialId::parse("研发客户端").is_err());
    }
    #[test]
    fn unknown_not_zero() {
        assert!(summary(&Summary::default()).contains("总 未知"));
        assert_eq!(number(Some(0)), "0");
    }
}
