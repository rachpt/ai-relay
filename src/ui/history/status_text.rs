//! 历史状态的纯文本格式；不读取文件或解析导出路径。
use crate::history::{Counters, Query, QueryReport, TimeZone, TrendGranularity};
use std::path::Path;

pub(super) fn query(query: &Query, report: &QueryReport, counters: Counters) -> String {
    query_page(
        query,
        report.summary.requests,
        report.details.len(),
        report.groups.len(),
        counters,
    )
}

pub(super) fn counters(counters: Counters) -> String {
    format!("历史记录状态：\r\n{}", recording(counters))
}

pub(super) fn exported(path: &Path, rows: usize, page_only: bool, counters: Counters) -> String {
    let scope = if page_only {
        "当前明细页"
    } else {
        "筛选后全部明细"
    };
    format!(
        "CSV 已导出：{rows} 行 · 范围：{scope}\r\n文件：{}\r\n{}",
        path.display(),
        recording(counters),
    )
}

fn query_page(
    query: &Query,
    requests: u64,
    detail_count: usize,
    group_count: usize,
    counters: Counters,
) -> String {
    let zone = match query.time_zone {
        TimeZone::Utc => "协调世界时",
        TimeZone::Local => "本地",
    };
    let granularity = match query.trend_granularity {
        TrendGranularity::Hour => "小时",
        TrendGranularity::Day => "天",
        TrendGranularity::Month => "月",
    };
    format!(
        "历史已查询：匹配 {requests} 请求 · {} · 本页汇总 {group_count} 组\r\n{zone}/{granularity} · 每页 {} 条 · {}",
        details(query.offset, detail_count),
        query.limit,
        recording(counters),
    )
}

fn details(offset: u32, count: usize) -> String {
    if count == 0 {
        return "明细 0 条".into();
    }
    let offset = u64::from(offset);
    let count = u64::try_from(count).unwrap_or(u64::MAX);
    let first = offset.saturating_add(1);
    let last = offset.saturating_add(count);
    format!("明细第 {first}–{last} 条")
}

fn recording(counters: Counters) -> String {
    // 数据库失败也含读取、维护及关闭失败，不能当作丢失记录条数。
    format!(
        "记录累计：已入队 {} 条 · 已写入 {} 条 · 队满丢弃 {} 条 · 数据库失败 {} 次 · 关闭丢弃 {} 条",
        counters.queued,
        counters.written,
        counters.queue_full,
        counters.db_failures,
        counters.closed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Group, Model, Summary};

    fn report(requests: u64) -> QueryReport {
        QueryReport {
            summary: Summary {
                requests,
                ..Summary::default()
            },
            groups: Vec::new(),
            details: Vec::new(),
            trend: Vec::new(),
            trend_truncated: false,
            aliases: Vec::new(),
        }
    }

    fn nonzero_counters() -> Counters {
        Counters {
            queued: 11,
            written: 7,
            queue_full: 3,
            db_failures: 5,
            closed: 2,
        }
    }

    fn maximum_counters() -> Counters {
        Counters {
            queued: u64::MAX,
            written: u64::MAX,
            queue_full: u64::MAX,
            db_failures: u64::MAX,
            closed: u64::MAX,
        }
    }

    fn assert_no_debug_fields(text: &str) {
        for forbidden in [
            "Counters {",
            "queued",
            "written",
            "queue_full",
            "db_failures",
            "closed",
            "offset",
            "page_only",
            "Local",
            "Utc",
            "Hour",
            "Day",
            "Month",
            "true",
            "false",
        ] {
            assert!(!text.contains(forbidden), "{forbidden}: {text}");
        }
    }

    #[test]
    fn query_page_has_two_compact_chinese_sections() {
        let request = Query {
            limit: 50,
            ..Query::default()
        };
        let text = query_page(
            &request,
            3058,
            50,
            0,
            Counters {
                queued: 6,
                written: 6,
                ..Counters::default()
            },
        );
        assert_eq!(
            text,
            "历史已查询：匹配 3058 请求 · 明细第 1–50 条 · 本页汇总 0 组\r\n本地/天 · 每页 50 条 · 记录累计：已入队 6 条 · 已写入 6 条 · 队满丢弃 0 条 · 数据库失败 0 次 · 关闭丢弃 0 条"
        );
        assert_eq!(text.split("\r\n").count(), 2);
        assert_no_debug_fields(&text);
    }

    #[test]
    fn query_uses_report_counts_without_echoing_filter_or_group_values() {
        let request = Query {
            requested_model: Some(Model::new("synthetic-status-model").unwrap()),
            limit: 50,
            ..Query::default()
        };
        let mut report = report(3058);
        report.groups.push(Group {
            keys: vec![Some("仅用于纯格式测试的分组值".into())],
            summary: Summary::default(),
        });
        let text = query(&request, &report, nonzero_counters());
        assert!(text.starts_with(
            "历史已查询：匹配 3058 请求 · 明细 0 条 · 本页汇总 1 组\r\n本地/天 · 每页 50 条 · "
        ));
        assert!(text.ends_with(&recording(nonzero_counters())));
        assert!(!text.contains("synthetic-status-model"));
        assert!(!text.contains("仅用于纯格式测试的分组值"));
        assert_no_debug_fields(&text);
    }

    #[test]
    fn empty_detail_pages_never_show_a_reversed_range() {
        for offset in [0, 50, u32::MAX] {
            for requests in [0, 3058, u64::MAX] {
                let request = Query {
                    offset,
                    limit: 50,
                    ..Query::default()
                };
                let text = query(&request, &report(requests), Counters::default());
                assert!(text.contains(&format!(
                    "匹配 {requests} 请求 · 明细 0 条 · 本页汇总 0 组\r\n"
                )));
                assert!(!text.contains('–'));
                assert_no_debug_fields(&text);
            }
        }
    }

    #[test]
    fn detail_ranges_are_one_based_and_do_not_add_in_u32() {
        for (offset, count, expected) in [
            (0, 50, "明细第 1–50 条"),
            (50, 3, "明细第 51–53 条"),
            (u32::MAX, 1, "明细第 4294967296–4294967296 条"),
        ] {
            assert_eq!(details(offset, count), expected);
        }
    }

    #[test]
    fn maximum_page_counts_saturate_without_wrapping() {
        let request = Query {
            offset: u32::MAX,
            limit: u32::MAX,
            ..Query::default()
        };
        let text = query_page(
            &request,
            u64::MAX,
            usize::MAX,
            usize::MAX,
            maximum_counters(),
        );
        let last = (u128::from(u32::MAX) + usize::MAX as u128).min(u128::from(u64::MAX));
        assert!(text.contains(&format!("匹配 {} 请求", u64::MAX)));
        assert!(text.contains(&format!("明细第 4294967296–{last} 条")));
        assert!(text.contains(&format!("本页汇总 {} 组", usize::MAX)));
        assert!(text.contains(&format!("每页 {} 条", u32::MAX)));
        assert!(text.ends_with(&recording(maximum_counters())));
        assert_no_debug_fields(&text);
    }

    #[test]
    fn both_time_zones_and_all_three_granularities_are_localized() {
        for (time_zone, zone) in [(TimeZone::Local, "本地"), (TimeZone::Utc, "协调世界时")] {
            for (trend_granularity, granularity) in [
                (TrendGranularity::Hour, "小时"),
                (TrendGranularity::Day, "天"),
                (TrendGranularity::Month, "月"),
            ] {
                let request = Query {
                    time_zone,
                    trend_granularity,
                    limit: 50,
                    ..Query::default()
                };
                let text = query(&request, &report(0), Counters::default());
                let metadata = text.split("\r\n").nth(1).unwrap();
                assert!(
                    metadata
                        .starts_with(&format!("{zone}/{granularity} · 每页 50 条 · 记录累计："))
                );
                assert_no_debug_fields(&text);
            }
        }
    }

    #[test]
    fn counters_keep_all_nonzero_metrics_and_distinguish_drops_from_failures() {
        let text = counters(nonzero_counters());
        assert_eq!(
            text,
            "历史记录状态：\r\n记录累计：已入队 11 条 · 已写入 7 条 · 队满丢弃 3 条 · 数据库失败 5 次 · 关闭丢弃 2 条"
        );
        assert!(!text.contains("积压"));
        assert!(!text.contains("待写"));
        assert_no_debug_fields(&text);
    }

    #[test]
    fn all_five_counters_preserve_u64_maximum_without_arithmetic() {
        let text = counters(maximum_counters());
        let maximum = u64::MAX.to_string();
        assert_eq!(text.matches(&maximum).count(), 5);
        for (label, unit) in [
            ("已入队", "条"),
            ("已写入", "条"),
            ("队满丢弃", "条"),
            ("数据库失败", "次"),
            ("关闭丢弃", "条"),
        ] {
            assert!(text.contains(&format!("{label} {maximum} {unit}")));
        }
        assert_no_debug_fields(&text);
    }

    #[test]
    fn exported_names_the_scope_and_formats_only_the_supplied_path() {
        let path = Path::new("纯格式导出.csv");
        for (page_only, rows, scope) in [
            (true, 0, "当前明细页"),
            (false, usize::MAX, "筛选后全部明细"),
        ] {
            let text = exported(path, rows, page_only, nonzero_counters());
            let lines: Vec<_> = text.split("\r\n").collect();
            assert_eq!(lines.len(), 3);
            assert_eq!(lines[0], format!("CSV 已导出：{rows} 行 · 范围：{scope}"));
            assert_eq!(lines[1], "文件：纯格式导出.csv");
            assert_eq!(lines[2], recording(nonzero_counters()));
            assert_no_debug_fields(&text);
        }
    }
}
