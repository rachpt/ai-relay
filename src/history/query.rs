use super::*;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter, types::Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate(q: &Query, paged: bool) -> Result<()> {
    if (paged && (q.limit == 0 || q.limit > 100 || q.offset > 100_000))
        || q.group_by.len() > 3
        || q.time_from.is_some_and(|v| v < 0)
        || q.time_to.is_some_and(|v| v < 0)
        || q.status.is_some_and(|v| !(100..=599).contains(&v))
        || matches!((q.time_from, q.time_to), (Some(a), Some(b)) if a >= b)
    {
        return Err(Error::Invalid("query bounds"));
    }
    Ok(())
}
pub(super) fn filter(q: &Query) -> (String, Vec<Value>) {
    let mut clauses = vec!["1=1".to_string()];
    let mut values = Vec::<Value>::new();
    macro_rules! filter {
        ($value:expr,$sql:literal) => {
            if let Some(v) = $value {
                clauses.push($sql.into());
                values.push(v.into());
            }
        };
    }
    filter!(q.time_from, "started_ms >= ?");
    filter!(q.time_to, "started_ms < ?");
    filter!(q.ip.map(|v| v.to_string()), "ip = ?");
    filter!(
        q.model.as_ref().map(|v| v.as_str().to_owned()),
        "response_model = ?"
    );
    filter!(
        q.requested_model.as_ref().map(|v| v.as_str().to_owned()),
        "requested_model = ?"
    );
    filter!(
        q.client.as_ref().map(|v| v.as_str().to_owned()),
        "client = ?"
    );
    filter!(
        q.upstream.as_ref().map(|v| v.as_str().to_owned()),
        "upstream = ?"
    );
    filter!(
        q.provider.as_ref().map(|v| v.as_str().to_owned()),
        "origin = ?"
    );
    filter!(q.protocol.map(|v| v.text().to_owned()), "protocol = ?");
    filter!(q.interface.map(|v| v.text().to_owned()), "interface = ?");
    filter!(q.outcome.map(|v| v.text().to_owned()), "outcome = ?");
    filter!(q.status.map(i64::from), "status = ?");
    (clauses.join(" AND "), values)
}
fn token_key(value: Option<u64>) -> Option<String> {
    // SQLite INTEGER cannot represent all u64; fixed-width decimal TEXT can.
    value.map(|v| format!("{v:020}"))
}
fn detail_token(sort: DetailSort) -> Option<TokenField> {
    match sort {
        DetailSort::TokensAsc(field) | DetailSort::TokensDesc(field) => Some(field),
        _ => None,
    }
}
pub(super) fn reset_detail_order(c: &Connection, sort: DetailSort) -> Result<()> {
    if detail_token(sort).is_some() {
        c.execute_batch("CREATE TEMP TABLE IF NOT EXISTS history_detail_order(request_id TEXT PRIMARY KEY,token TEXT); DELETE FROM temp.history_detail_order;")?;
    }
    Ok(())
}
fn add_detail_order(c: &Connection, sort: DetailSort, record: &RequestRecord) -> Result<()> {
    if let Some(field) = detail_token(sort) {
        c.execute(
            "INSERT INTO temp.history_detail_order VALUES (?,?)",
            params![
                record.request_id.as_str(),
                token_key(usage_values(&record.report.usage)[field.index()])
            ],
        )?;
    }
    Ok(())
}
pub(super) fn prepare_detail_order(
    c: &Connection,
    q: &Query,
    filter: &str,
    values: &[Value],
    budget: &reader::Budget,
) -> Result<()> {
    budget.check()?;
    reset_detail_order(c, q.detail_sort)?;
    if detail_token(q.detail_sort).is_some() {
        let mut stmt = c.prepare(&format!("SELECT * FROM records WHERE {filter}"))?;
        let mut rows = stmt.query(params_from_iter(values))?;
        while let Some(row) = rows.next()? {
            budget.check()?;
            add_detail_order(c, q.detail_sort, &db::decode(row)?)?;
        }
    }
    Ok(())
}
pub(super) fn ordered_sql(q: &Query, filter: &str, paged: bool) -> String {
    let (source, order) = match q.detail_sort {
        DetailSort::TimeAsc => ("records", "started_ms ASC,request_id ASC"),
        DetailSort::TimeDesc => ("records", "started_ms DESC,request_id DESC"),
        DetailSort::TokensAsc(_) => (
            "records JOIN temp.history_detail_order USING(request_id)",
            "token IS NULL ASC,token ASC,started_ms DESC,request_id DESC",
        ),
        DetailSort::TokensDesc(_) => (
            "records JOIN temp.history_detail_order USING(request_id)",
            "token IS NULL ASC,token DESC,started_ms DESC,request_id DESC",
        ),
    };
    format!(
        "SELECT records.* FROM {source} WHERE {filter} ORDER BY {order}{}",
        if paged { " LIMIT ? OFFSET ?" } else { "" }
    )
}
fn group_keys(q: &Query, r: &RequestRecord) -> Result<Vec<Option<String>>> {
    q.group_by
        .iter()
        .map(|group| {
            Ok(match group {
                // Preserve the original UTC numeric day/hour group keys. Local and new
                // month groups carry calendar labels (repeated hours include offsets).
                GroupBy::Day if q.time_zone == TimeZone::Utc => {
                    Some((r.started_ms / 86_400_000).to_string())
                }
                GroupBy::Hour if q.time_zone == TimeZone::Utc => {
                    Some((r.started_ms / 3_600_000).to_string())
                }
                GroupBy::Day => {
                    Some(calendar::bucket(r.started_ms, TrendGranularity::Day, q.time_zone)?.label)
                }
                GroupBy::Hour => {
                    Some(calendar::bucket(r.started_ms, TrendGranularity::Hour, q.time_zone)?.label)
                }
                GroupBy::Month => Some(
                    calendar::bucket(r.started_ms, TrendGranularity::Month, q.time_zone)?.label,
                ),
                GroupBy::Model => r.report.model.clone(),
                GroupBy::RequestedModel => {
                    r.requested_model.as_ref().map(|v| v.as_str().to_owned())
                }
                GroupBy::Protocol => Some(r.protocol.text().into()),
                GroupBy::Ip => Some(r.peer_ip.to_string()),
                GroupBy::Credential => r.client.as_ref().map(|v| v.as_str().to_owned()),
                GroupBy::UpstreamCredential => r.upstream.as_ref().map(|v| v.as_str().to_owned()),
                GroupBy::Interface => Some(r.interface.text().into()),
                GroupBy::Provider => r.provider.as_ref().map(|v| v.as_str().to_owned()),
            })
        })
        .collect()
}
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredSummary {
    counts: [u64; 10],
    tokens: [Option<u64>; 7],
    coverage: [u64; 7],
}
fn encode_summary(s: &Summary) -> Result<String> {
    serde_json::to_string(&StoredSummary {
        counts: [
            s.requests,
            s.complete,
            s.partial,
            s.unknown,
            s.truncated,
            s.parse_errors,
            s.parse_limits,
            s.provider_errors,
            s.arithmetic_overflows,
            s.unsupported,
        ],
        tokens: usage_values(&s.tokens),
        coverage: s.coverage,
    })
    .map_err(|_| Error::Invalid("temporary group summary"))
}
fn decode_summary(text: &str) -> Result<Summary> {
    let s: StoredSummary =
        serde_json::from_str(text).map_err(|_| Error::Invalid("temporary group summary"))?;
    Ok(Summary {
        requests: s.counts[0],
        complete: s.counts[1],
        partial: s.counts[2],
        unknown: s.counts[3],
        truncated: s.counts[4],
        parse_errors: s.counts[5],
        parse_limits: s.counts[6],
        provider_errors: s.counts[7],
        arithmetic_overflows: s.counts[8],
        unsupported: s.counts[9],
        tokens: usage_from(s.tokens),
        coverage: s.coverage,
    })
}
fn add_group(c: &Connection, q: &Query, record: &RequestRecord) -> Result<()> {
    let keys =
        serde_json::to_string(&group_keys(q, record)?).map_err(|_| Error::Invalid("group keys"))?;
    let previous: Option<(String, i64, i64)> = c
        .query_row(
            "SELECT summary,first_ms,last_ms FROM temp.history_query_groups WHERE keys=?",
            [&keys],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (mut summary, first, last) = match previous {
        Some((summary, first, last)) => (
            decode_summary(&summary)?,
            first.min(record.started_ms),
            last.max(record.started_ms),
        ),
        None => (Summary::default(), record.started_ms, record.started_ms),
    };
    db::add(&mut summary, record);
    let token = match q.group_sort {
        GroupSort::TokensAsc(field) | GroupSort::TokensDesc(field) => {
            token_key(usage_values(&summary.tokens)[field.index()])
        }
        _ => None,
    };
    c.execute("INSERT INTO temp.history_query_groups VALUES (?,?,?,?,?,?) ON CONFLICT(keys) DO UPDATE SET summary=excluded.summary,first_ms=excluded.first_ms,last_ms=excluded.last_ms,requests=excluded.requests,token=excluded.token",
        params![keys, encode_summary(&summary)?, first, last, i64::try_from(summary.requests).map_err(|_| Error::Invalid("group request count"))?, token])?;
    Ok(())
}
fn groups(c: &Connection, q: &Query) -> Result<Vec<Group>> {
    if q.group_by.is_empty() {
        return Ok(Vec::new());
    }
    let metric = match q.group_sort {
        GroupSort::KeysAsc => "",
        GroupSort::KeysDesc => "",
        GroupSort::CountAsc => "requests ASC,",
        GroupSort::CountDesc => "requests DESC,",
        GroupSort::TimeAsc => "first_ms ASC,",
        GroupSort::TimeDesc => "last_ms DESC,",
        GroupSort::TokensAsc(_) => "token IS NULL ASC,token ASC,",
        GroupSort::TokensDesc(_) => "token IS NULL ASC,token DESC,",
    };
    // Sort each dimension (not serialized JSON) so NULL ordering and multi-key
    // ties have the same semantics as the original SQL GROUP BY report.
    let direction = if q.group_sort == GroupSort::KeysDesc {
        "DESC"
    } else {
        "ASC"
    };
    let keys = (0..q.group_by.len())
        .map(|i| {
            let key = format!("json_extract(keys,'$[{i}]')");
            if q.time_zone == TimeZone::Utc && matches!(q.group_by[i], GroupBy::Day | GroupBy::Hour)
            {
                format!("CAST({key} AS INTEGER) {direction}")
            } else {
                format!("{key} {direction}")
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let mut stmt = c.prepare(&format!("SELECT keys,summary FROM temp.history_query_groups ORDER BY {metric}{keys} LIMIT ? OFFSET ?"))?;
    let mut rows = stmt.query(params![q.limit, q.offset])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        let keys: String = row.get(0)?;
        result.push(Group {
            keys: serde_json::from_str(&keys)
                .map_err(|_| Error::Invalid("temporary group keys"))?,
            summary: decode_summary(&row.get::<_, String>(1)?)?,
        });
    }
    Ok(result)
}
fn aliases(
    c: &Connection,
    q: &Query,
    details: &[RequestRecord],
    groups: &[Group],
) -> Result<Vec<CredentialAliasEntry>> {
    let mut ids = BTreeSet::new();
    for record in details {
        ids.extend(record.client.iter().cloned());
        ids.extend(record.upstream.iter().cloned());
    }
    for group in groups {
        for (dimension, key) in q.group_by.iter().zip(&group.keys) {
            if matches!(dimension, GroupBy::Credential | GroupBy::UpstreamCredential)
                && let Some(key) = key
            {
                ids.insert(CredentialId::parse(key)?);
            }
        }
    }
    let mut result = Vec::new();
    for credential in ids {
        if let Some(alias) = db::alias(c, &credential)? {
            result.push(CredentialAliasEntry { credential, alias });
        }
    }
    Ok(result)
}
pub(super) fn run(c: &mut Connection, q: Query, budget: &reader::Budget) -> Result<QueryReport> {
    validate(&q, true)?;
    let (filter, values) = filter(&q);
    budget.check()?;
    let tx = c.transaction()?;
    // Establish the main WAL snapshot before writing connection-local TEMP.
    tx.query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))?;
    reset_detail_order(&tx, q.detail_sort)?;
    if !q.group_by.is_empty() {
        tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS history_query_groups(keys TEXT PRIMARY KEY,summary TEXT NOT NULL,first_ms INTEGER NOT NULL,last_ms INTEGER NOT NULL,requests INTEGER NOT NULL,token TEXT); DELETE FROM temp.history_query_groups;")?;
    }
    let mut total = Summary::default();
    let mut trend = BTreeMap::<i64, TrendPoint>::new();
    let mut trend_truncated = false;
    {
        let mut stmt = tx.prepare(&format!(
            "SELECT * FROM records WHERE {filter} ORDER BY started_ms,request_id"
        ))?;
        let mut rows = stmt.query(params_from_iter(&values))?;
        while let Some(row) = rows.next()? {
            budget.check()?;
            let record = db::decode(row)?;
            db::add(&mut total, &record);
            add_detail_order(&tx, q.detail_sort, &record)?;
            if !q.group_by.is_empty() {
                add_group(&tx, &q, &record)?;
            }
            let bucket = calendar::bucket(record.started_ms, q.trend_granularity, q.time_zone)?;
            if !trend.contains_key(&bucket.bucket_start_ms) && trend.len() == 366 {
                trend_truncated = true;
                if trend
                    .last_key_value()
                    .is_some_and(|(key, _)| bucket.bucket_start_ms > *key)
                {
                    continue;
                }
                trend.pop_last();
            }
            let point = trend
                .entry(bucket.bucket_start_ms)
                .or_insert_with(|| TrendPoint {
                    utc_day: bucket.utc_day,
                    bucket_start_ms: bucket.bucket_start_ms,
                    label: bucket.label,
                    summary: Summary::default(),
                });
            db::add(&mut point.summary, &record);
        }
    }
    let mut details = Vec::new();
    {
        let mut args = values;
        args.push(i64::from(q.limit).into());
        args.push(i64::from(q.offset).into());
        let mut stmt = tx.prepare(&ordered_sql(&q, &filter, true))?;
        let mut rows = stmt.query(params_from_iter(&args))?;
        while let Some(row) = rows.next()? {
            budget.check()?;
            details.push(db::decode(row)?);
        }
    }
    let groups = groups(&tx, &q)?;
    let aliases = aliases(&tx, &q, &details, &groups)?;
    budget.check()?;
    tx.commit()?;
    Ok(QueryReport {
        summary: total,
        groups,
        details,
        trend: trend.into_values().collect(),
        trend_truncated,
        aliases,
    })
}
