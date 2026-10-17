use super::*;
use ai_relay_usage::{ObservationFormat, Provider, UsageReport};
use rusqlite::{Connection, params, params_from_iter, types::Value};
pub(super) fn open(dir: &std::path::Path, options: Options) -> Result<Connection> {
    for name in [
        "history.sqlite3",
        "history.sqlite3-wal",
        "history.sqlite3-shm",
        "history.sqlite3-journal",
    ] {
        let path = dir.join(name);
        identity::check_path(&path)?;
        match std::fs::symlink_metadata(&path) {
            Ok(m) if !m.is_file() => return Err(Error::Invalid("database file type")),
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    let mut c = Connection::open(dir.join("history.sqlite3"))?;
    c.busy_timeout(std::time::Duration::from_secs(2))?;
    let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != 0 && version != 1 {
        return Err(Error::Invalid("unsupported schema version"));
    }
    if version == 0 {
        let tables: i64 = c.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        if tables != 0 {
            return Err(Error::Invalid("unversioned database"));
        }
        let tx = c.transaction()?;
        tx.execute_batch("CREATE TABLE records(request_id TEXT PRIMARY KEY,started_ms INTEGER NOT NULL,ip TEXT NOT NULL,client TEXT,upstream TEXT,origin TEXT,requested_model TEXT,response_model TEXT,protocol TEXT NOT NULL,interface TEXT NOT NULL,status INTEGER,outcome TEXT NOT NULL,latency_ms INTEGER NOT NULL,total_ms INTEGER NOT NULL,report TEXT NOT NULL); CREATE INDEX history_time ON records(started_ms,request_id); PRAGMA user_version=1;")?;
        tx.commit()?;
    }
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA temp_store=FILE; PRAGMA cache_size=-2048; PRAGMA trusted_schema=OFF;")?;
    retain(&mut c, options)?;
    Ok(c)
}
fn checkpoint(c: &Connection) -> Result<()> {
    let (busy, log, done): (i64, i64, i64) =
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    if busy != 0 || (log >= 0 && done < log) {
        return Err(Error::Busy);
    }
    Ok(())
}
pub(super) fn close(c: Connection) -> Result<()> {
    let checkpoint_result = checkpoint(&c);
    let close_result = c.close().map_err(|(_, e)| Error::Database(e));
    checkpoint_result.and(close_result)
}
pub(super) fn snapshot(c: &Connection, target: &std::path::Path, key: &[u8; 32]) -> Result<()> {
    identity::create_private_dir(target)?;
    // VACUUM INTO is a SQLite-managed coherent snapshot, never a live WAL copy.
    // Target is a fresh private directory; no existing database is overwritten.
    let path = target.join("history.sqlite3");
    let text = path
        .to_str()
        .ok_or(Error::Invalid("snapshot path encoding"))?;
    c.execute("VACUUM INTO ?1", [text])?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)?
        .sync_all()?;
    identity::save_key(target, key)?;
    Ok(())
}
pub(super) fn clear(c: &mut Connection) -> Result<()> {
    c.execute_batch("PRAGMA secure_delete=ON;")?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM records", [])?;
    tx.commit()?;
    c.execute_batch("VACUUM;")?;
    checkpoint(c)
}
pub(super) fn retain(c: &mut Connection, o: Options) -> Result<()> {
    let cutoff = now_ms().saturating_sub(i64::from(o.retention_days) * 86_400_000);
    let tx = c.transaction()?;
    tx.execute("DELETE FROM records WHERE started_ms < ?", [cutoff])?;
    tx.execute("DELETE FROM records WHERE request_id IN (SELECT request_id FROM records ORDER BY started_ms DESC,request_id DESC LIMIT -1 OFFSET ?)",[o.max_records])?;
    tx.commit()?;
    Ok(())
}
pub(super) fn insert(c: &mut Connection, r: &RequestRecord, o: Options) -> Result<()> {
    let report=serde_json::json!({"tokens":usage_values(&r.report.usage),"provider":provider_text(r.report.provider),"format":format_text(r.report.format),"truncated":r.report.truncated,"parse_error":r.report.parse_error,"limit_exceeded":r.report.limit_exceeded,"provider_error":r.report.provider_error,"overflow":r.report.arithmetic_overflow}).to_string();
    let tx = c.transaction()?;
    tx.execute(
        "INSERT INTO records VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            r.request_id.0,
            r.started_ms,
            r.peer_ip.to_string(),
            r.client.as_ref().map(|v| &v.0),
            r.upstream.as_ref().map(|v| &v.0),
            r.provider.as_ref().map(|v| &v.0),
            r.requested_model.as_ref().map(|v| &v.0),
            r.report.model,
            r.protocol.text(),
            r.interface.text(),
            r.status,
            r.outcome.text(),
            r.latency_ms as i64,
            r.total_ms as i64,
            report
        ],
    )?;
    tx.execute(
        "DELETE FROM records WHERE started_ms < ?",
        [now_ms().saturating_sub(i64::from(o.retention_days) * 86_400_000)],
    )?;
    tx.execute("DELETE FROM records WHERE request_id IN (SELECT request_id FROM records ORDER BY started_ms DESC,request_id DESC LIMIT -1 OFFSET ?)",[o.max_records])?;
    tx.commit()?;
    Ok(())
}
fn decode(row: &rusqlite::Row<'_>) -> Result<RequestRecord> {
    let json: String = row.get(14)?;
    if json.len() > 4096 {
        return Err(Error::Invalid("stored report size"));
    }
    let v: serde_json::Value =
        serde_json::from_str(&json).map_err(|_| Error::Invalid("stored report"))?;
    let tokens: [Option<u64>; 7] =
        serde_json::from_value(v["tokens"].clone()).map_err(|_| Error::Invalid("stored tokens"))?;
    let flag = |name: &str| v[name].as_bool().ok_or(Error::Invalid("stored flag"));
    let provider = match v["provider"].as_str() {
        Some("unknown") => Provider::Unknown,
        Some("chat") => Provider::OpenAiChat,
        Some("responses") => Provider::OpenAiResponses,
        Some("anthropic") => Provider::Anthropic,
        _ => return Err(Error::Invalid("stored provider")),
    };
    let format = match v["format"].as_str() {
        Some("type") => ObservationFormat::UnsupportedContentType,
        Some("encoding") => ObservationFormat::UnsupportedContentEncoding,
        Some("json") => ObservationFormat::Json,
        Some("sse") => ObservationFormat::Sse,
        _ => return Err(Error::Invalid("stored format")),
    };
    let r = RequestRecord {
        request_id: RequestId(row.get(0)?),
        started_ms: row.get(1)?,
        peer_ip: row
            .get::<_, String>(2)?
            .parse()
            .map_err(|_| Error::Invalid("stored IP"))?,
        client: row.get::<_, Option<String>>(3)?.map(CredentialId),
        upstream: row.get::<_, Option<String>>(4)?.map(CredentialId),
        provider: row.get::<_, Option<String>>(5)?.map(ProviderOrigin),
        requested_model: row.get::<_, Option<String>>(6)?.map(Model),
        protocol: Protocol::parse(&row.get::<_, String>(8)?)?,
        interface: Interface::parse(&row.get::<_, String>(9)?)?,
        status: row.get(10)?,
        outcome: Outcome::parse(&row.get::<_, String>(11)?)?,
        latency_ms: row
            .get::<_, i64>(12)?
            .try_into()
            .map_err(|_| Error::Invalid("stored latency"))?,
        total_ms: row
            .get::<_, i64>(13)?
            .try_into()
            .map_err(|_| Error::Invalid("stored duration"))?,
        report: UsageReport {
            model: row.get(7)?,
            provider,
            format,
            usage: usage_from(tokens),
            truncated: flag("truncated")?,
            parse_error: flag("parse_error")?,
            limit_exceeded: flag("limit_exceeded")?,
            provider_error: flag("provider_error")?,
            arithmetic_overflow: flag("overflow")?,
        },
    };
    r.validate()?;
    Ok(r)
}
fn add(s: &mut Summary, r: &RequestRecord) {
    s.requests += 1;
    match r.report.completeness() {
        ai_relay_usage::UsageCompleteness::Complete => s.complete += 1,
        ai_relay_usage::UsageCompleteness::Partial => s.partial += 1,
        ai_relay_usage::UsageCompleteness::Unknown => s.unknown += 1,
    };
    s.truncated += u64::from(r.report.truncated);
    s.parse_errors += u64::from(r.report.parse_error);
    s.parse_limits += u64::from(r.report.limit_exceeded);
    s.provider_errors += u64::from(r.report.provider_error);
    s.arithmetic_overflows += u64::from(r.report.arithmetic_overflow);
    s.unsupported += u64::from(matches!(
        r.report.format,
        ObservationFormat::UnsupportedContentType | ObservationFormat::UnsupportedContentEncoding
    ));
    let mut totals = usage_values(&s.tokens);
    for (i, v) in usage_values(&r.report.usage).into_iter().enumerate() {
        if let Some(v) = v {
            s.coverage[i] += 1;
            totals[i] = Some(match totals[i].unwrap_or(0).checked_add(v) {
                Some(n) => n,
                None => {
                    s.arithmetic_overflows += 1;
                    u64::MAX
                }
            });
        }
    }
    s.tokens = usage_from(totals);
}
fn summary(c: &Connection, filter: &str, values: &[Value]) -> Result<Summary> {
    let mut stmt = c.prepare(&format!("SELECT * FROM records WHERE {filter}"))?;
    let mut rows = stmt.query(params_from_iter(values))?;
    let mut s = Summary::default();
    while let Some(row) = rows.next()? {
        add(&mut s, &decode(row)?);
    }
    Ok(s)
}
pub(super) fn query(c: &mut Connection, q: Query) -> Result<QueryReport> {
    if q.limit == 0
        || q.limit > 100
        || q.offset > 100_000
        || q.group_by.len() > 3
        || q.time_from.is_some_and(|v| v < 0)
        || q.time_to.is_some_and(|v| v < 0)
        || matches!((q.time_from,q.time_to),(Some(a),Some(b)) if a>=b)
    {
        return Err(Error::Invalid("query bounds"));
    }
    let tx = c.transaction()?;
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
    filter!(q.model.map(|v| v.0), "response_model = ?");
    filter!(q.requested_model.map(|v| v.0), "requested_model = ?");
    filter!(q.client.map(|v| v.0), "client = ?");
    filter!(q.upstream.map(|v| v.0), "upstream = ?");
    filter!(q.provider.map(|v| v.0), "origin = ?");
    filter!(q.protocol.map(|v| v.text().to_owned()), "protocol = ?");
    filter!(q.interface.map(|v| v.text().to_owned()), "interface = ?");
    filter!(q.outcome.map(|v| v.text().to_owned()), "outcome = ?");
    filter!(q.status.map(i64::from), "status = ?");
    let filter = clauses.join(" AND ");
    let total = summary(&tx, &filter, &values)?;
    let mut details = Vec::new();
    {
        let mut args = values.clone();
        args.push(i64::from(q.limit).into());
        args.push(i64::from(q.offset).into());
        let mut stmt=tx.prepare(&format!("SELECT * FROM records WHERE {filter} ORDER BY started_ms DESC,request_id DESC LIMIT ? OFFSET ?"))?;
        let mut rows = stmt.query(params_from_iter(&args))?;
        while let Some(row) = rows.next()? {
            details.push(decode(row)?);
        }
    }
    let mut groups = Vec::new();
    if !q.group_by.is_empty() {
        let names = q.group_by.iter().map(|v| v.sql()).collect::<Vec<_>>();
        let keys = names
            .iter()
            .map(|v| format!("CAST({v} AS TEXT)"))
            .collect::<Vec<_>>()
            .join(",");
        let mut args = values.clone();
        args.push(i64::from(q.limit).into());
        args.push(i64::from(q.offset).into());
        let mut stmt = tx.prepare(&format!(
            "SELECT {keys} FROM records WHERE {filter} GROUP BY {} ORDER BY {} LIMIT ? OFFSET ?",
            names.join(","),
            names.join(",")
        ))?;
        let mut rows = stmt.query(params_from_iter(&args))?;
        while let Some(row) = rows.next()? {
            let mut group_keys = Vec::new();
            let mut sub = filter.clone();
            let mut args = values.clone();
            for (i, name) in names.iter().enumerate() {
                let key: Option<String> = row.get(i)?;
                sub.push_str(&format!(" AND CAST({name} AS TEXT) IS ?"));
                args.push(key.clone().map(Value::Text).unwrap_or(Value::Null));
                group_keys.push(key);
            }
            groups.push(Group {
                keys: group_keys,
                summary: summary(&tx, &sub, &args)?,
            });
        }
    }
    let mut trend = Vec::new();
    let mut trend_truncated = false;
    {
        let mut stmt=tx.prepare(&format!("SELECT started_ms / 86400000 FROM records WHERE {filter} GROUP BY started_ms / 86400000 ORDER BY 1 LIMIT 367"))?;
        let mut rows = stmt.query(params_from_iter(&values))?;
        while let Some(row) = rows.next()? {
            if trend.len() == 366 {
                trend_truncated = true;
                break;
            }
            let day: i64 = row.get(0)?;
            let mut args = values.clone();
            args.push(day.into());
            trend.push(TrendPoint {
                utc_day: day,
                summary: summary(
                    &tx,
                    &format!("{filter} AND started_ms / 86400000 = ?"),
                    &args,
                )?,
            });
        }
    }
    tx.commit()?;
    Ok(QueryReport {
        summary: total,
        groups,
        details,
        trend,
        trend_truncated,
    })
}
