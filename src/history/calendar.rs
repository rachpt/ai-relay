//! Calendar conversion uses the zone's rules at the recorded instant. Never
//! substitute Local::now().offset(): that would misbucket historical DST data.
use super::{Error, Result, TimeZone, TrendGranularity};
use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDateTime, Offset,
    TimeZone as ChronoTimeZone, Timelike, Utc,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarBucket {
    pub bucket_start_ms: i64,
    pub label: String,
    /// Calendar-day label retained for consumers of TrendPoint::utc_day.
    pub utc_day: i64,
}

pub(super) fn bucket(
    ms: i64,
    granularity: TrendGranularity,
    zone: TimeZone,
) -> Result<CalendarBucket> {
    match zone {
        TimeZone::Utc => calendar_bucket_with_zone(ms, granularity, &Utc),
        TimeZone::Local => calendar_bucket_with_zone(ms, granularity, &Local),
    }
}

/// Pure, injectable equivalent of the Local/UTC calendar bucketing used by
/// reports. It does not change the process timezone or the system clock.
/// Repeated hours have distinct UTC starts and offset-bearing labels. A missing
/// midnight/hour starts at the first representable instant after the gap.
pub fn calendar_bucket_with_zone<T: ChronoTimeZone>(
    ms: i64,
    granularity: TrendGranularity,
    zone: &T,
) -> Result<CalendarBucket> {
    let instant =
        DateTime::from_timestamp_millis(ms).ok_or(Error::Invalid("calendar timestamp"))?;
    let local = instant.with_timezone(zone);
    let wall_time = instant
        .naive_utc()
        .checked_add_offset(local.offset().fix())
        .ok_or(Error::Invalid("calendar local timestamp"))?;
    let date = match granularity {
        TrendGranularity::Month => wall_time.date().with_day(1),
        _ => Some(wall_time.date()),
    }
    .ok_or(Error::Invalid("calendar date"))?;
    let boundary = date
        .and_hms_opt(
            if granularity == TrendGranularity::Hour {
                wall_time.hour()
            } else {
                0
            },
            0,
            0,
        )
        .ok_or(Error::Invalid("calendar boundary"))?;
    let choose = |value: NaiveDateTime| {
        let result = zone.from_local_datetime(&value);
        if granularity == TrendGranularity::Hour {
            match result {
                LocalResult::Single(value) => Some(value).filter(|v| {
                    v.timestamp_millis() <= ms && v.offset().fix() == local.offset().fix()
                }),
                LocalResult::Ambiguous(first, second) => [first, second]
                    .into_iter()
                    .filter(|v| {
                        v.timestamp_millis() <= ms && v.offset().fix() == local.offset().fix()
                    })
                    .max_by_key(|v| v.timestamp_millis()),
                LocalResult::None => None,
            }
        } else {
            result.earliest()
        }
    };
    let start = if let Some(start) = choose(boundary) {
        start
    } else {
        // Rare midnight/non-hour DST gaps. Minute probes bound the work; the
        // final second scan preserves historical second-resolution transitions.
        let mut found = None;
        for minute in 1..=2880 {
            let mut probe = boundary
                .checked_add_signed(Duration::minutes(minute))
                .ok_or(Error::Invalid("calendar gap"))?;
            if granularity == TrendGranularity::Hour {
                probe = probe.min(wall_time);
            }
            if choose(probe).is_some() {
                for second in (minute - 1) * 60 + 1..=minute * 60 {
                    let probe = boundary
                        .checked_add_signed(Duration::seconds(second))
                        .ok_or(Error::Invalid("calendar gap"))?;
                    if let Some(start) = choose(probe) {
                        found = Some(start);
                        break;
                    }
                }
                break;
            }
        }
        found.ok_or(Error::Invalid("unresolvable calendar boundary"))?
    };
    if start.timestamp_millis() > ms {
        return Err(Error::Invalid("calendar boundary after request"));
    }
    let label = match granularity {
        TrendGranularity::Hour => start
            .fixed_offset()
            .format("%Y-%m-%d %H:%M %:z")
            .to_string(),
        TrendGranularity::Day => date.format("%Y-%m-%d").to_string(),
        TrendGranularity::Month => date.format("%Y-%m").to_string(),
    };
    let utc_day = date
        .and_hms_opt(0, 0, 0)
        .ok_or(Error::Invalid("calendar day"))?
        .and_utc()
        .timestamp_millis()
        .div_euclid(86_400_000);
    Ok(CalendarBucket {
        bucket_start_ms: start.timestamp_millis(),
        label,
        utc_day,
    })
}
