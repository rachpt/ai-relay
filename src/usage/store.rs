use std::sync::Mutex;

use super::{Provider, TokenUsage, UsageCompleteness, UsageReport, filtered_model};

pub const MAX_GROUPS: usize = 128;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UsageCounters {
    pub reports: u64,
    pub complete: u64,
    pub partial: u64,
    pub unknown: u64,
    pub truncated: u64,
    pub parse_limits: u64,
    pub parse_errors: u64,
    pub provider_errors: u64,
    pub unsupported: u64,
    pub arithmetic_overflows: u64,
}

impl UsageCounters {
    fn record(&mut self, report: &UsageReport) {
        self.reports = self.reports.saturating_add(1);
        let count = match report.completeness() {
            UsageCompleteness::Complete => &mut self.complete,
            UsageCompleteness::Partial => &mut self.partial,
            UsageCompleteness::Unknown => &mut self.unknown,
        };
        *count = count.saturating_add(1);
        for (count, yes) in [
            (&mut self.truncated, report.truncated),
            (&mut self.parse_limits, report.limit_exceeded),
            (&mut self.parse_errors, report.parse_error),
            (&mut self.provider_errors, report.provider_error),
            (&mut self.arithmetic_overflows, report.arithmetic_overflow),
            (
                &mut self.unsupported,
                matches!(
                    report.format,
                    super::ObservationFormat::UnsupportedContentType
                        | super::ObservationFormat::UnsupportedContentEncoding
                ),
            ),
        ] {
            *count = count.saturating_add(u64::from(yes));
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UsageAggregate {
    pub counters: UsageCounters,
    /// 各字段仅累加有值的观察值；None 表示没有任何报告提供此字段。
    /// 有值也不代表覆盖全部请求；请同时查看 counters。
    pub observed_tokens: TokenUsage,
    pub sum_overflow: bool,
}

impl UsageAggregate {
    fn record(&mut self, report: &UsageReport) {
        self.counters.record(report);
        macro_rules! add {
            ($($field:ident),*) => { $(
                if let Some(value) = report.usage.$field {
                    let old = self.observed_tokens.$field.unwrap_or(0);
                    let sum = old.checked_add(value);
                    self.sum_overflow |= sum.is_none();
                    self.observed_tokens.$field = Some(sum.unwrap_or(u64::MAX));
                }
            )* };
        }
        add!(
            input_tokens,
            output_tokens,
            total_tokens,
            cached_input_tokens,
            reasoning_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens
        );
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageGroup {
    pub provider: Provider,
    pub model: Option<String>,
    pub aggregate: UsageAggregate,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UsageSnapshot {
    pub aggregate: UsageAggregate,
    pub groups: Vec<UsageGroup>,
    /// 新分组超出上限时进入此桶；全局统计仍包含这些报告。
    pub overflow: UsageAggregate,
}

impl UsageSnapshot {
    /// 不在 store 锁内格式化，也不声称这是账单或完整计费用量。
    pub fn summary(&self) -> String {
        let c = &self.aggregate.counters;
        let total = self
            .aggregate
            .observed_tokens
            .total_tokens
            .map_or_else(|| "未知".to_owned(), |n| n.to_string());
        format!(
            "用量观察：{} 次，完整 {} / 部分 {} / 未知 {}；截断 {}，超限 {}，解析错误 {}；已报告 total {}（非计费统计）",
            c.reports,
            c.complete,
            c.partial,
            c.unknown,
            c.truncated,
            c.parse_limits,
            c.parse_errors,
            total
        )
    }
}

/// 短临界区中仅更新有界内存；无网络、磁盘、GUI 或回调。
/// snapshot 最多复制 MAX_GROUPS 个短模型标识符，格式化由调用方锁外完成。
#[derive(Default)]
pub struct UsageStore {
    state: Mutex<UsageSnapshot>,
}

impl UsageStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 每次响应调用一次。跨请求相同的 report 仍是不同观察，store 不猜测请求身份。
    pub fn record(&self, report: &UsageReport) {
        let model = report.model.as_deref().and_then(filtered_model);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.aggregate.record(report);
        if let Some(group) = state
            .groups
            .iter_mut()
            .find(|g| g.provider == report.provider && g.model == model)
        {
            group.aggregate.record(report);
        } else if state.groups.len() < MAX_GROUPS {
            let mut aggregate = UsageAggregate::default();
            aggregate.record(report);
            state.groups.push(UsageGroup {
                provider: report.provider,
                model,
                aggregate,
            });
        } else {
            state.overflow.record(report);
        }
    }

    pub fn snapshot(&self) -> UsageSnapshot {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}
