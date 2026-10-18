//! 历史表格、分组/趋势文本和 CSV 无关的展示格式化。
use super::*;
use crate::history::{
    Counters, CredentialId, GroupBy, Model, ProviderOrigin, Query, QueryReport, Summary, TimeZone,
    TrendGranularity,
};
use chrono::{Local, Utc};
use std::fmt::Write;

pub(super) fn number(v: Option<u64>) -> String {
    v.map_or_else(|| "未知".into(), |n| n.to_string())
}
pub(super) fn summary(s: &Summary) -> String {
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

pub(super) fn timestamp(ms: i64, zone: TimeZone) -> String {
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
pub(super) fn group_key(value: Option<&str>, dimension: &GroupBy, query: &Query) -> String {
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
pub(super) fn alias_label<'a>(report: &'a QueryReport, id: Option<&CredentialId>) -> &'a str {
    id.and_then(|id| report.aliases.iter().find(|entry| &entry.credential == id))
        .map_or("未设别名", |entry| entry.alias.as_str())
}
pub(super) fn detail_cells(
    r: &db::RequestRecord,
    report: &QueryReport,
    zone: TimeZone,
) -> Vec<String> {
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
    pub(super) fn render_table(&self) {
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
            // DefWindowProc toggles WS_VISIBLE for WM_SETREDRAW. Hidden tables
            // already need no painting: do not reveal them, even transiently, when
            // a query finishes in a text/session/management or inactive page.
            if was_visible {
                SendMessageW(table, WM_SETREDRAW, 0, 0);
            }
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
            if was_visible {
                SendMessageW(table, WM_SETREDRAW, 1, 0);
            }
            // WM_SETREDRAW does not repair non-client borders or the nested
            // list header. Queue the complete final tree, never paint mid-render.
            viewport::redraw(table);
        }
    }
    pub(super) fn render(&mut self, query: Query, report: Box<QueryReport>, counters: Counters) {
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
}
