use super::*;
use ai_relay_usage::{ObservationFormat, Provider, UsageReport};
use rusqlite::{Connection, OptionalExtension, params};
fn check_files(dir: &Path) -> Result<()> {
    identity::check_path(dir)?;
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
    Ok(())
}
pub(super) fn open(dir: &std::path::Path, options: Options) -> Result<Connection> {
    check_files(dir)?;
    let mut c = Connection::open(dir.join("history.sqlite3"))?;
    c.busy_timeout(std::time::Duration::from_secs(2))?;
    let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if !(0..=2).contains(&version) {
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
        tx.execute_batch("CREATE TABLE records(request_id TEXT PRIMARY KEY,started_ms INTEGER NOT NULL,ip TEXT NOT NULL,client TEXT,upstream TEXT,origin TEXT,requested_model TEXT,response_model TEXT,protocol TEXT NOT NULL,interface TEXT NOT NULL,status INTEGER,outcome TEXT NOT NULL,latency_ms INTEGER NOT NULL,total_ms INTEGER NOT NULL,report TEXT NOT NULL); CREATE INDEX history_time ON records(started_ms,request_id);")?;
        create_alias_table(&tx)?;
        tx.pragma_update(None, "user_version", 2)?;
        tx.commit()?;
    }
    validate_columns(
        &c,
        "records",
        &[
            "request_id",
            "started_ms",
            "ip",
            "client",
            "upstream",
            "origin",
            "requested_model",
            "response_model",
            "protocol",
            "interface",
            "status",
            "outcome",
            "latency_ms",
            "total_ms",
            "report",
        ],
    )?;
    if version == 1 {
        // DDL and version advance commit together; never recreate records or the
        // separately protected identity. Unexpected/conflicting schemas fail.
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        create_alias_table(&tx)?;
        tx.pragma_update(None, "user_version", 2)?;
        tx.commit()?;
    }
    validate_columns(&c, "credential_aliases", &["credential", "alias"])?;
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA temp_store=FILE; PRAGMA cache_size=-2048; PRAGMA trusted_schema=OFF;")?;
    retain(&mut c, options)?;
    Ok(c)
}
/// Called only after the writer has committed schema setup/migration. The main
/// database is OS/SQLite read-only; TEMP remains writable for bounded sorting.
/// query_only is deliberately not enabled because it also forbids TEMP writes.
pub(super) fn open_reader(dir: &Path) -> Result<Connection> {
    check_files(dir)?;
    let c = Connection::open_with_flags(
        dir.join("history.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_PRIVATE_CACHE,
    )?;
    c.busy_timeout(Duration::from_millis(100))?;
    c.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA temp_store=FILE; PRAGMA cache_size=-2048; PRAGMA temp.cache_size=-2048;")?;
    let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let mode: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    if version != 2 || mode != "wal" || !c.is_readonly("main")? {
        return Err(Error::Invalid("reader schema or journal mode"));
    }
    validate_columns(
        &c,
        "records",
        &[
            "request_id",
            "started_ms",
            "ip",
            "client",
            "upstream",
            "origin",
            "requested_model",
            "response_model",
            "protocol",
            "interface",
            "status",
            "outcome",
            "latency_ms",
            "total_ms",
            "report",
        ],
    )?;
    validate_columns(&c, "credential_aliases", &["credential", "alias"])?;
    Ok(c)
}
fn create_alias_table(c: &Connection) -> Result<()> {
    c.execute_batch("CREATE TABLE credential_aliases(credential TEXT PRIMARY KEY NOT NULL CHECK(length(credential)=64),alias TEXT NOT NULL CHECK(length(alias) BETWEEN 1 AND 64));")?;
    Ok(())
}
fn validate_columns(c: &Connection, table: &str, expected: &[&str]) -> Result<()> {
    let stmt = c.prepare(&format!("SELECT * FROM {table} LIMIT 0"))?;
    if stmt.column_names() != expected {
        return Err(Error::Invalid("unexpected database schema"));
    }
    // Preparing alone must also reject views that could execute arbitrary SQL.
    let kind: String = c.query_row(
        "SELECT type FROM sqlite_master WHERE name=?",
        [table],
        |r| r.get(0),
    )?;
    if kind != "table" {
        return Err(Error::Invalid("unexpected database schema"));
    }
    Ok(())
}
pub(super) fn checkpoint(c: &Connection) -> Result<()> {
    let (busy, log, done): (i64, i64, i64) =
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    if busy != 0 || (log >= 0 && done < log) {
        return Err(Error::Busy);
    }
    Ok(())
}
/// Online maintenance must not wait for a report's WAL read snapshot.
pub(super) fn checkpoint_now(c: &Connection) -> Result<()> {
    c.busy_timeout(Duration::ZERO)?;
    let result = checkpoint(c);
    let restored = c.busy_timeout(Duration::from_secs(2)).map_err(Error::from);
    restored.and(result)
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
#[derive(Default)]
pub(super) struct RecordCount {
    cached: Option<(i64, i64)>,
}
impl RecordCount {
    fn current(&mut self, c: &Connection) -> Result<(i64, i64)> {
        let version: i64 = c.query_row("PRAGMA data_version", [], |r| r.get(0))?;
        if let Some((old, count)) = self.cached
            && old == version
        {
            return Ok((version, count));
        }
        Ok((
            version,
            c.query_row("SELECT count(*) FROM records", [], |r| r.get(0))?,
        ))
    }
    pub(super) fn invalidate(&mut self) {
        self.cached = None;
    }
}
pub(super) fn clear(c: &mut Connection, options: ClearOptions) -> Result<()> {
    c.execute_batch("PRAGMA secure_delete=ON;")?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM records", [])?;
    if options.remove_aliases {
        tx.execute("DELETE FROM credential_aliases", [])?;
    }
    tx.commit()?;
    c.execute_batch("VACUUM;")?;
    checkpoint_now(c)
}
pub(super) fn retain(c: &mut Connection, o: Options) -> Result<u64> {
    let cutoff = now_ms().saturating_sub(i64::from(o.retention_days) * 86_400_000);
    let tx = c.transaction()?;
    let removed = tx.execute("DELETE FROM records WHERE started_ms < ?", [cutoff])?;
    let count: i64 = tx.query_row("SELECT count(*) FROM records", [], |r| r.get(0))?;
    let overflow = trim_capacity(&tx, count.saturating_sub(i64::from(o.max_records)))?;
    tx.commit()?;
    Ok((removed + overflow) as u64)
}
fn trim_capacity(c: &Connection, excess: i64) -> Result<usize> {
    if excess <= 0 {
        return Ok(0);
    }
    Ok(c.execute("DELETE FROM records WHERE request_id IN (SELECT request_id FROM records ORDER BY started_ms,request_id LIMIT ?)", [excess])?)
}
#[cfg(test)]
pub(super) fn insert(c: &mut Connection, r: &RequestRecord, o: Options) -> Result<()> {
    insert_counted(c, r, o, &mut RecordCount::default())
}
pub(super) fn insert_counted(
    c: &mut Connection,
    r: &RequestRecord,
    o: Options,
    count: &mut RecordCount,
) -> Result<()> {
    let report=serde_json::json!({"tokens":usage_values(&r.report.usage),"provider":provider_text(r.report.provider),"format":format_text(r.report.format),"truncated":r.report.truncated,"parse_error":r.report.parse_error,"limit_exceeded":r.report.limit_exceeded,"provider_error":r.report.provider_error,"overflow":r.report.arithmetic_overflow}).to_string();
    // One writer owns this cache. BEGIN IMMEDIATE makes the data_version/count
    // check atomic with the insert, including externally committed test/tool rows.
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let (version, before) = count.current(&tx)?;
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
    let expired = tx.execute(
        "DELETE FROM records WHERE started_ms < ?",
        [now_ms().saturating_sub(i64::from(o.retention_days) * 86_400_000)],
    )?;
    let after = before + 1 - expired as i64;
    let removed = trim_capacity(&tx, after.saturating_sub(i64::from(o.max_records)))?;
    tx.commit()?;
    count.cached = Some((version, after - removed as i64));
    Ok(())
}
pub(super) fn decode(row: &rusqlite::Row<'_>) -> Result<RequestRecord> {
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
pub(super) fn add(s: &mut Summary, r: &RequestRecord) {
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
pub(super) fn alias(c: &Connection, credential: &CredentialId) -> Result<Option<CredentialAlias>> {
    let value: Option<String> = c
        .query_row(
            "SELECT alias FROM credential_aliases WHERE credential=?",
            [credential.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    value.map(|v| CredentialAlias::new(&v)).transpose()
}
pub(super) fn aliases(
    c: &Connection,
    budget: &reader::Budget,
) -> Result<Vec<CredentialAliasEntry>> {
    let mut stmt = c.prepare(
        "SELECT credential,alias FROM credential_aliases ORDER BY credential LIMIT 100001",
    )?;
    let mut rows = stmt.query([])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        budget.check()?;
        if result.len() == 100_000 {
            return Err(Error::Invalid("alias capacity"));
        }
        result.push(CredentialAliasEntry {
            credential: CredentialId::parse(&row.get::<_, String>(0)?)?,
            alias: CredentialAlias::new(&row.get::<_, String>(1)?)?,
        });
    }
    Ok(result)
}
pub(super) fn set_alias(
    c: &mut Connection,
    credential: &CredentialId,
    alias: &CredentialAlias,
) -> Result<()> {
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let count: i64 = tx.query_row("SELECT count(*) FROM credential_aliases", [], |r| r.get(0))?;
    if count >= 100_000 && self::alias(&tx, credential)?.is_none() {
        return Err(Error::Invalid("alias capacity"));
    }
    tx.execute("INSERT INTO credential_aliases VALUES (?,?) ON CONFLICT(credential) DO UPDATE SET alias=excluded.alias", params![credential.as_str(), alias.as_str()])?;
    tx.commit()?;
    Ok(())
}
pub(super) fn remove_alias(c: &Connection, credential: &CredentialId) -> Result<bool> {
    Ok(c.execute(
        "DELETE FROM credential_aliases WHERE credential=?",
        [credential.as_str()],
    )? != 0)
}
pub(super) fn stats(c: &Connection, dir: &Path, options: Options) -> Result<StorageStats> {
    let size = |name: &str| -> Result<u64> {
        let path = dir.join(name);
        identity::check_path(&path)?;
        match std::fs::metadata(path) {
            Ok(m) if m.is_file() => Ok(m.len()),
            Ok(_) => Err(Error::Invalid("database file type")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(e.into()),
        }
    };
    let read_count = |sql: &str| -> Result<u64> {
        let value: i64 = c.query_row(sql, [], |r| r.get(0))?;
        value
            .try_into()
            .map_err(|_| Error::Invalid("database size/count"))
    };
    let pages = read_count("PRAGMA freelist_count")?;
    let page_size = read_count("PRAGMA page_size")?;
    Ok(StorageStats {
        options,
        records: read_count("SELECT count(*) FROM records")?,
        aliases: read_count("SELECT count(*) FROM credential_aliases")?,
        database_bytes: size("history.sqlite3")?,
        wal_bytes: size("history.sqlite3-wal")?,
        shared_memory_bytes: size("history.sqlite3-shm")?,
        free_database_bytes: pages.saturating_mul(page_size),
    })
}
