//! 有界快照在后台格式化；界面只消费文本，不获取 store 锁。
use std::fmt::Write;

use crate::usage::{MAX_GROUPS, Provider, UsageAggregate, UsageSnapshot};

pub(crate) fn render(snapshot: &UsageSnapshot) -> String {
    let mut text = String::from(
        "当前程序会话用量观察（非账单 / 未持久化；跨代理启停累计）\r\n仅累加已报告字段，未知不等于 0；有值也不代表覆盖全部请求。缓存/推理子项不重复加总。\r\n",
    );
    aggregate(&mut text, "全部", &snapshot.aggregate);
    let _ = writeln!(text, "provider / model 详情（最多 {MAX_GROUPS} 组）：\r");
    for group in snapshot.groups.iter().take(MAX_GROUPS) {
        let provider = match group.provider {
            Provider::Unknown => "未知 provider",
            Provider::OpenAiChat => "OpenAI Chat",
            Provider::OpenAiResponses => "OpenAI Responses",
            Provider::Anthropic => "Anthropic",
        };
        let label = format!(
            "{provider} / {}",
            group.model.as_deref().unwrap_or("未知 model")
        );
        aggregate(&mut text, &label, &group.aggregate);
    }
    if snapshot.overflow.counters.reports != 0 {
        aggregate(
            &mut text,
            "分组上限溢出桶（已包含在全部中）",
            &snapshot.overflow,
        );
    }
    text
}

fn number(value: Option<u64>) -> String {
    value.map_or_else(|| "未知".into(), |n| n.to_string())
}

fn aggregate(text: &mut String, label: &str, value: &UsageAggregate) {
    let t = &value.observed_tokens;
    let c = &value.counters;
    let _ = write!(
        text,
        "[{label}] 报告 {}；完整 {} / 部分 {} / 未知 {}；截断 {} / 超限 {}\r\n  输入 {} / 输出 {} / 总 {}；缓存命中 {} / 缓存写入 {} / 缓存读取 {}；推理 {}\r\n  解析错误 {} / 上游错误 {} / 不支持 {} / 计算溢出 {} / 累加溢出 {}\r\n",
        c.reports,
        c.complete,
        c.partial,
        c.unknown,
        c.truncated,
        c.parse_limits,
        number(t.input_tokens),
        number(t.output_tokens),
        number(t.total_tokens),
        number(t.cached_input_tokens),
        number(t.cache_creation_input_tokens),
        number(t.cache_read_input_tokens),
        number(t.reasoning_tokens),
        c.parse_errors,
        c.provider_errors,
        c.unsupported,
        c.arithmetic_overflows,
        if value.sum_overflow {
            "是（饱和值）"
        } else {
            "否"
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{TokenUsage, UsageReport, UsageStore};

    #[test]
    fn unknown_is_not_zero_and_details_include_all_fields() {
        let store = UsageStore::new();
        store.record(&UsageReport {
            provider: Provider::Anthropic,
            model: Some("local-model".into()),
            usage: TokenUsage {
                input_tokens: Some(0),
                cache_creation_input_tokens: Some(5),
                cache_read_input_tokens: Some(7),
                ..Default::default()
            },
            truncated: true,
            limit_exceeded: true,
            ..Default::default()
        });
        let text = render(&store.snapshot());
        assert!(text.contains("输入 0 / 输出 未知 / 总 未知"));
        assert!(text.contains("缓存命中 未知 / 缓存写入 5 / 缓存读取 7；推理 未知"));
        assert!(text.contains("完整 0 / 部分 1 / 未知 0；截断 1 / 超限 1"));
        assert!(text.contains("Anthropic / local-model"));
        assert!(text.contains("非账单 / 未持久化"));
    }

    #[test]
    fn overflow_details_remain_visible_and_bounded() {
        let store = UsageStore::new();
        for index in 0..MAX_GROUPS + 3 {
            store.record(&UsageReport {
                model: Some(format!("model-{index}")),
                ..Default::default()
            });
        }
        let text = render(&store.snapshot());
        assert!(text.contains("分组上限溢出桶（已包含在全部中）] 报告 3"));
        assert_eq!(text.matches("未知 provider /").count(), MAX_GROUPS);
        assert!(text.len() < 128 * 1024);
    }
}
