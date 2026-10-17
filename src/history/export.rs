use super::*;
use rusqlite::params_from_iter;
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
};

pub(super) fn run(
    c: &mut rusqlite::Connection,
    q: Query,
    path: PathBuf,
    budget: &reader::Budget,
) -> ExportResult<ExportSummary> {
    run_with_snapshot_hook(c, q, path, budget, || {})
}
fn run_with_snapshot_hook(
    c: &mut rusqlite::Connection,
    q: Query,
    path: PathBuf,
    budget: &reader::Budget,
    after_snapshot: impl FnOnce(),
) -> ExportResult<ExportSummary> {
    let mut rows_written = 0;
    let mut file_state = ExportFileState::NotCreated;
    let result = (|| -> Result<ExportSummary> {
        budget.check()?;
        identity::check_path(&path)?;
        query::validate(&q, false)?;
        let (filter, values) = query::filter(&q);
        let tx = c.transaction()?;
        // Establish one main-database read snapshot before opening the output.
        // Aliases, ordering and every streamed row use this same transaction.
        tx.query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))?;
        after_snapshot();
        query::prepare_detail_order(&tx, &q, &filter, &values, budget)?;
        budget.check()?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        file_state = ExportFileState::Incomplete;
        let mut writer = BufWriter::with_capacity(64 * 1024, file);
        let output = (|| -> Result<u64> {
            csv::header(&mut writer)?;
            let mut stmt = tx.prepare(&query::ordered_sql(&q, &filter, false))?;
            let mut rows = stmt.query(params_from_iter(&values))?;
            while let Some(row) = rows.next()? {
                budget.check()?;
                let record = db::decode(row)?;
                let client = record
                    .client
                    .as_ref()
                    .map(|id| db::alias(&tx, id))
                    .transpose()?
                    .flatten();
                let upstream = record
                    .upstream
                    .as_ref()
                    .map(|id| db::alias(&tx, id))
                    .transpose()?
                    .flatten();
                csv::record(
                    &mut writer,
                    &record,
                    q.time_zone,
                    client.as_ref(),
                    upstream.as_ref(),
                )?;
                rows_written += 1;
            }
            budget.check()?;
            writer.flush()?;
            budget.check()?;
            writer.get_ref().sync_all()?;
            budget.check()?;
            Ok(writer.get_ref().metadata()?.len())
        })();
        // Do not let BufWriter::drop silently retry a failed flush. An incomplete
        // file is deliberately retained and explicitly reported to the caller.
        let (file, _) = writer.into_parts();
        drop(file);
        let bytes = output?;
        budget.check()?;
        tx.commit()?;
        Ok(ExportSummary {
            path: path.clone(),
            rows: rows_written,
            bytes,
        })
    })();
    result.map_err(|cause| ExportError {
        cause: budget.check().err().unwrap_or(cause),
        path,
        rows_written,
        file_state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_commit_cannot_change_export_records_or_aliases() {
        struct Temp(PathBuf);
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let temp = Temp(std::env::temp_dir().join(format!(
            "relay-history-export-snapshot-{}",
            identity::hex(&identity::random::<16>().unwrap())
        )));
        std::fs::create_dir(&temp.0).unwrap();
        let mut c = db::open(&temp.0, Options::default()).unwrap();
        let credential = CredentialId::parse(&"ab".repeat(32)).unwrap();
        let record = RequestRecord {
            request_id: RequestId("synthetic-request".into()),
            started_ms: now_ms(),
            peer_ip: "127.0.0.1".parse().unwrap(),
            client: Some(credential.clone()),
            upstream: None,
            provider: None,
            requested_model: None,
            protocol: Protocol::Unknown,
            interface: Interface::Unknown,
            status: None,
            outcome: Outcome::Cancelled,
            latency_ms: 0,
            total_ms: 0,
            report: ai_relay_usage::UsageReport::default(),
        };
        db::insert(&mut c, &record, Options::default()).unwrap();
        db::set_alias(
            &mut c,
            &credential,
            &CredentialAlias::new("before-snapshot").unwrap(),
        )
        .unwrap();
        let external = rusqlite::Connection::open(temp.0.join("history.sqlite3")).unwrap();
        let path = temp.0.join("snapshot.csv");
        let mut read = db::open_reader(&temp.0).unwrap();
        let budget = reader::Budget::for_test(Duration::from_secs(60));
        let result = run_with_snapshot_hook(&mut read, Query { detail_sort: DetailSort::TokensDesc(TokenField::Total), ..Query::default() }, path.clone(), &budget, || {
            external.execute_batch("BEGIN IMMEDIATE; UPDATE credential_aliases SET alias='after-snapshot'; INSERT INTO records SELECT request_id||'-later',started_ms,ip,client,upstream,origin,requested_model,response_model,protocol,interface,status,outcome,latency_ms,total_ms,report FROM records; COMMIT;").unwrap();
        }).unwrap();
        assert_eq!(result.rows, 1);
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("before-snapshot"));
        assert!(!text.contains("after-snapshot"));
        assert!(!text.contains("synthetic-request-later"));
        assert_eq!(
            db::stats(&c, &temp.0, Options::default()).unwrap().records,
            2
        );
        read.close().unwrap();
        drop(external);
        db::close(c).unwrap();
    }
}
