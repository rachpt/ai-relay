//! 历史筛选草稿解析：日期边界、分页/排序/分组和字段校验。
use super::*;
use crate::history::{
    CredentialId, DetailSort, GroupBy, GroupSort, Interface, Model, Outcome, Protocol,
    ProviderOrigin, Query, TimeZone, TokenField, TrendGranularity,
};
use chrono::{Datelike, Local, NaiveDate, TimeZone as ChronoTimeZone, Utc};

pub(super) fn date_boundary<T: ChronoTimeZone>(date: NaiveDate, zone: &T) -> Result<i64, String> {
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
pub(super) fn parse_date(text: &str, zone: TimeZone) -> Result<Option<i64>, String> {
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
pub(super) fn date_range(
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

impl History {
    pub(super) fn draft(&self) -> Result<Query, String> {
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
}
