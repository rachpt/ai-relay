//! Local metadata only. Start/query/shutdown belong on a controller worker, never
//! the GUI/network thread. Handles only hash, validate and try-send bounded data.
//! Credential identifiers are irreversible HMACs, not encrypted credentials.
//! Losing history.identity breaks identity continuity; it must not be regenerated
//! for an existing database. Windows protects it with current-user DPAPI; other
//! platforms use mode 0600. The data directory must be private to its owner.
//! A fixed read actor owns its READ_ONLY main connection and writable TEMP;
//! long reports/exports never execute on the record/control owner.
use crate::usage as ai_relay_usage;
mod calendar;
mod csv;
mod db;
mod export;
mod identity;
mod query;
mod reader;
mod types;
pub use calendar::{CalendarBucket, calendar_bucket_with_zone};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
pub use types::*;

#[derive(Default)]
struct Metrics {
    queued: AtomicU64,
    written: AtomicU64,
    queue_full: AtomicU64,
    db_failures: AtomicU64,
    closed: AtomicU64,
}
struct Identity {
    key: [u8; 32],
    session: [u8; 16],
    sequence: AtomicU64,
}
impl Drop for Identity {
    fn drop(&mut self) {
        for b in &mut self.key {
            // SAFETY: exclusive access to live owned key bytes during destruction.
            unsafe {
                std::ptr::write_volatile(b, 0);
            }
        }
    }
}
#[derive(Clone)]
pub struct HistoryHandle {
    sender: SyncSender<Command>,
    identity: Arc<Identity>,
    metrics: Arc<Metrics>,
    accepting: Arc<AtomicBool>,
    inflight: Arc<AtomicU64>,
}
enum Command {
    Record(Box<RequestRecord>),
    Snapshot(PathBuf, mpsc::Sender<Result<()>>),
    Clear(ClearOptions, mpsc::Sender<Result<()>>),
    SetAlias(CredentialId, CredentialAlias, mpsc::Sender<Result<()>>),
    RemoveAlias(CredentialId, mpsc::Sender<Result<bool>>),
    Configure(Options, mpsc::Sender<Result<PolicyReport>>),
    Stats(mpsc::Sender<Result<StorageStats>>),
    Barrier(mpsc::Sender<()>),
    Stop,
}
pub struct HistoryService {
    handle: HistoryHandle,
    worker: Option<JoinHandle<()>>,
    completion: mpsc::Receiver<Result<()>>,
    stop_sent: bool,
    closed_ok: bool,
    reader: Option<reader::Reader>,
    reads_closed: Arc<AtomicBool>,
}
impl HistoryHandle {
    pub fn credential_id(&self, secret: &str) -> String {
        // HMAC accepts any key length. Avoid expect even on this infallible path.
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&self.identity.key) else {
            unreachable!("HMAC accepts arbitrary key lengths")
        };
        mac.update(b"ai-relay/history/credential/v1\0");
        mac.update(secret.as_bytes());
        identity::hex(&mac.finalize().into_bytes())
    }
    pub fn credential(&self, secret: &str) -> CredentialId {
        CredentialId(self.credential_id(secret))
    }
    pub fn request_id(&self) -> RequestId {
        RequestId(format!(
            "{}-{:016x}",
            identity::hex(&self.identity.session),
            self.identity.sequence.fetch_add(1, Ordering::Relaxed)
        ))
    }
    pub fn try_record(&self, record: RequestRecord) -> TryRecord {
        if record.validate().is_err() {
            return TryRecord::Invalid;
        }
        self.inflight.fetch_add(1, Ordering::SeqCst);
        struct Admission<'a>(&'a AtomicU64);
        impl Drop for Admission<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _admission = Admission(&self.inflight);
        if !self.accepting.load(Ordering::SeqCst) {
            self.metrics.closed.fetch_add(1, Ordering::Relaxed);
            return TryRecord::Closed;
        }
        match self.sender.try_send(Command::Record(Box::new(record))) {
            Ok(()) => {
                self.metrics.queued.fetch_add(1, Ordering::Relaxed);
                TryRecord::Queued
            }
            Err(TrySendError::Full(_)) => {
                self.metrics.queue_full.fetch_add(1, Ordering::Relaxed);
                TryRecord::Full
            }
            Err(TrySendError::Disconnected(_)) => {
                self.metrics.closed.fetch_add(1, Ordering::Relaxed);
                TryRecord::Closed
            }
        }
    }
    pub fn counters(&self) -> Counters {
        let m = &self.metrics;
        Counters {
            queued: m.queued.load(Ordering::Relaxed),
            written: m.written.load(Ordering::Relaxed),
            queue_full: m.queue_full.load(Ordering::Relaxed),
            db_failures: m.db_failures.load(Ordering::Relaxed),
            closed: m.closed.load(Ordering::Relaxed),
        }
    }
}
impl HistoryService {
    pub fn start(data_dir: impl AsRef<Path>) -> Result<Self> {
        Self::start_with_options(data_dir, Options::default())
    }
    pub fn start_with_options(data_dir: impl AsRef<Path>, options: Options) -> Result<Self> {
        options.validate()?;
        let dir = data_dir.as_ref().to_path_buf();
        identity::check_path(&dir)?;
        std::fs::create_dir_all(&dir)?;
        identity::check_path(&dir)?;
        if dir.join("history.sqlite3").exists() && !dir.join("history.identity").exists() {
            return Err(Error::Invalid("missing identity for existing history"));
        }
        let key = identity::load_key(&dir)?;
        let session = identity::random()?;
        let (sender, receiver) = mpsc::sync_channel(options.queue_capacity);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker_identity = Arc::new(Identity {
            key,
            session,
            sequence: AtomicU64::new(0),
        });
        let handle_identity = worker_identity.clone();
        let metrics = Arc::new(Metrics::default());
        let worker_metrics = metrics.clone();
        let accepting = Arc::new(AtomicBool::new(true));
        let worker_accepting = accepting.clone();
        let inflight = Arc::new(AtomicU64::new(0));
        let worker_inflight = inflight.clone();
        let reads_closed = Arc::new(AtomicBool::new(true));
        let worker_reads_closed = reads_closed.clone();
        let reader_dir = dir.clone();
        let worker = thread::Builder::new()
            .name("history-sqlite".into())
            .spawn(move || {
                let mut conn = match db::open(&dir, options) {
                    Ok(c) => {
                        let _ = ready_tx.send(Ok(()));
                        c
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        worker_accepting.store(false, Ordering::Release);
                        return;
                    }
                };
                let mut failed_writes = false;
                let mut record_count = db::RecordCount::default();
                let mut options = options;
                let mut last_maintenance = Instant::now();
                loop {
                    // Retention must also advance while idle; timeout wakeups are
                    // already required for shutdown admission/drain coordination.
                    if last_maintenance.elapsed() >= Duration::from_secs(60) {
                        record_count.invalidate();
                        let maintenance =
                            db::retain(&mut conn, options).and_then(|_| db::checkpoint_now(&conn));
                        if !matches!(maintenance, Ok(()) | Err(Error::Busy)) {
                            worker_metrics.db_failures.fetch_add(1, Ordering::Relaxed);
                        }
                        last_maintenance = Instant::now();
                    }
                    let command = match receiver.recv_timeout(Duration::from_millis(10)) {
                        Ok(command) => command,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if !worker_accepting.load(Ordering::SeqCst)
                                && worker_inflight.load(Ordering::SeqCst) == 0
                            {
                                // Recheck after observing the final admission; a sender may
                                // have enqueued between recv_timeout and that observation.
                                match receiver.try_recv() {
                                    Ok(command) => command,
                                    Err(_) => break,
                                }
                            } else {
                                continue;
                            }
                        }
                    };
                    match command {
                        Command::Record(r) => {
                            if db::insert_counted(&mut conn, &r, options, &mut record_count)
                                .is_err()
                            {
                                record_count.invalidate();
                                failed_writes = true;
                                worker_metrics.db_failures.fetch_add(1, Ordering::Relaxed);
                            } else {
                                worker_metrics.written.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Command::Snapshot(target, reply) => {
                            let result = db::snapshot(&conn, &target, &worker_identity.key);
                            if result.is_err() {
                                worker_metrics.db_failures.fetch_add(1, Ordering::Relaxed);
                            }
                            let _ = reply.send(result);
                        }
                        Command::Clear(options, reply) => {
                            record_count.invalidate();
                            let result = db::clear(&mut conn, options);
                            if result.is_err() {
                                worker_metrics.db_failures.fetch_add(1, Ordering::Relaxed);
                            }
                            let _ = reply.send(result);
                        }
                        Command::SetAlias(id, alias, reply) => {
                            let _ = reply.send(db::set_alias(&mut conn, &id, &alias));
                        }
                        Command::RemoveAlias(id, reply) => {
                            let _ = reply.send(db::remove_alias(&conn, &id));
                        }
                        Command::Stats(reply) => {
                            let _ = reply.send(db::stats(&conn, &dir, options));
                        }
                        Command::Configure(next, reply) => {
                            let result = (|| -> Result<PolicyReport> {
                                next.validate()?;
                                // The bounded channel cannot be resized in place.
                                if next.queue_capacity != options.queue_capacity {
                                    return Err(Error::Invalid(
                                        "queue capacity change requires restart",
                                    ));
                                }
                                record_count.invalidate();
                                let removed_records = db::retain(&mut conn, next)?;
                                options = next;
                                let checkpoint = match db::checkpoint_now(&conn) {
                                    Ok(()) => CheckpointStatus::Complete,
                                    Err(Error::Busy) => CheckpointStatus::Busy,
                                    Err(e) => return Err(e),
                                };
                                Ok(PolicyReport {
                                    options,
                                    removed_records,
                                    checkpoint,
                                    storage: db::stats(&conn, &dir, options)?,
                                })
                            })();
                            let _ = reply.send(result);
                        }
                        Command::Barrier(reply) => {
                            let _ = reply.send(());
                        }
                        Command::Stop => break,
                    }
                }
                worker_accepting.store(false, Ordering::Release);
                // Admission is stopped and records drained. A successful final
                // checkpoint must not race our own reader's still-live snapshot.
                while !worker_reads_closed.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
                let result = db::close(conn);
                if result.is_err() {
                    worker_metrics.db_failures.fetch_add(1, Ordering::Relaxed);
                }
                let result = result.and_then(|()| {
                    if failed_writes {
                        Err(Error::Invalid(
                            "one or more queued writes failed; history incomplete",
                        ))
                    } else {
                        Ok(())
                    }
                });
                let _ = done_tx.send(result);
            })?;
        let mut service = match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                handle: HistoryHandle {
                    sender,
                    identity: handle_identity,
                    metrics,
                    accepting,
                    inflight,
                },
                worker: Some(worker),
                completion: done_rx,
                stop_sent: false,
                closed_ok: false,
                reader: None,
                reads_closed: reads_closed.clone(),
            }),
            other => {
                // The failing initialization thread owns its cleanup; do not
                // block indefinitely waiting for a JoinHandle, even on failure.
                drop(worker);
                match other {
                    Ok(Err(e)) => Err(e),
                    _ => Err(Error::Worker),
                }
            }
        }?;
        // Migrations are committed before a READ_ONLY connection is opened.
        reads_closed.store(false, Ordering::Release);
        match reader::Reader::start(
            reader_dir,
            reads_closed.clone(),
            service.handle.metrics.clone(),
        ) {
            Ok(reader) => {
                service.reader = Some(reader);
                Ok(service)
            }
            Err(e) => {
                reads_closed.store(true, Ordering::Release);
                Err(e)
            }
        }
    }
    pub fn handle(&self) -> HistoryHandle {
        self.handle.clone()
    }
    /// Blocking API for controller workers, never GUI/network threads. A short
    /// writer barrier includes prior admitted writes, then the independent reader
    /// takes one WAL snapshot. At most four reads are admitted; excess is Busy.
    /// Reads have a 60-second end-to-end deadline including barrier/queue wait.
    /// No retention writes run here; the writer owns policy and maintenance.
    pub fn query(&self, q: Query) -> Result<QueryReport> {
        self.reader
            .as_ref()
            .ok_or(Error::Closed)?
            .client
            .query(&self.handle, q)
    }
    pub fn barrier(&self) -> Result<()> {
        reader::writer_barrier(&self.handle, Instant::now() + Duration::from_secs(60))
    }
    /// Coherent metadata snapshot with the same local identity. Target must not
    /// exist. Only the current OS user can read the newly created directory.
    /// Failure may leave an incomplete target; never publish it as a snapshot.
    pub fn snapshot_to(&self, target: impl AsRef<Path>) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.control(Command::Snapshot(target.as_ref().to_path_buf(), tx))?;
        rx.recv().map_err(|_| Error::Worker)?
    }
    /// Caller must obtain explicit confirmation first. Deletes request metadata,
    /// preserves identity and aliases. Not forensic secure erasure; may partially succeed.
    pub fn clear(&self) -> Result<()> {
        self.clear_with_options(ClearOptions::default())
    }
    /// Removing aliases requires a separate, explicit caller-confirmed opt-in.
    /// HMAC identity is always retained, including when aliases are removed.
    pub fn clear_with_options(&self, options: ClearOptions) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.control(Command::Clear(options, tx))?;
        rx.recv().map_err(|_| Error::Worker)?
    }
    /// Exports every matching request, ignoring pagination and group selection.
    /// Runs on the bounded reader actor in one WAL read snapshot, with bounded row/buffer
    /// memory and file-backed SQLite sorting. Excludes IPs, credential IDs and
    /// raw keys; includes request IDs, display aliases and all usage/coverage flags.
    /// Path must be absolute, trusted and new. Never overwrites or removes it.
    /// On failure inspect ExportError::file_state/path; incomplete files remain.
    /// Uses the same four-read limit/60-second deadline as query(). On timeout a
    /// blocked OS file write may still finish later: Unknown is not closure proof.
    pub fn export_all(&self, query: Query, path: impl AsRef<Path>) -> ExportResult<ExportSummary> {
        let path = path.as_ref().to_path_buf();
        match &self.reader {
            Some(reader) => reader.client.export(&self.handle, query, path),
            None => Err(ExportError {
                cause: Error::Closed,
                path,
                rows_written: 0,
                file_state: ExportFileState::NotCreated,
            }),
        }
    }
    /// User-chosen labels only; never pass a raw key (even one unrecognized by
    /// the common-secret-pattern validator). List is capped at 100,000 aliases.
    pub fn aliases(&self) -> Result<Vec<CredentialAliasEntry>> {
        self.reader
            .as_ref()
            .ok_or(Error::Closed)?
            .client
            .aliases(&self.handle)
    }
    pub fn list_aliases(&self) -> Result<Vec<CredentialAliasEntry>> {
        self.aliases()
    }
    pub fn set_alias(&self, credential: CredentialId, alias: CredentialAlias) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.control(Command::SetAlias(credential, alias, tx))?;
        rx.recv().map_err(|_| Error::Worker)?
    }
    pub fn remove_alias(&self, credential: CredentialId) -> Result<bool> {
        let (tx, rx) = mpsc::channel();
        self.control(Command::RemoveAlias(credential, tx))?;
        rx.recv().map_err(|_| Error::Worker)?
    }
    /// Applies retention immediately and to subsequent queued writes/idle
    /// maintenance. Runtime-only: caller owns durable settings. Queue capacity
    /// changes require restart and are rejected without changing policy.
    /// Busy checkpoint is reported separately from already committed deletions.
    /// Other post-commit errors may also leave the policy applied; inspect stats().
    pub fn configure(&self, options: Options) -> Result<PolicyReport> {
        let (tx, rx) = mpsc::channel();
        self.control(Command::Configure(options, tx))?;
        rx.recv().map_err(|_| Error::Worker)?
    }
    /// Observed file sizes, not a byte quota or promise of reclaimed disk space.
    pub fn stats(&self) -> Result<StorageStats> {
        let (tx, rx) = mpsc::channel();
        self.control(Command::Stats(tx))?;
        rx.recv().map_err(|_| Error::Worker)?
    }
    fn control(&self, command: Command) -> Result<()> {
        if !self.handle.accepting.load(Ordering::SeqCst) {
            return Err(Error::Closed);
        }
        self.handle.sender.try_send(command).map_err(|e| match e {
            TrySendError::Full(_) => Error::Busy,
            TrySendError::Disconnected(_) => Error::Closed,
        })
    }
    /// Compatibility entry point; bounded to five seconds. Ok is closure proof.
    pub fn shutdown(&mut self) -> Result<()> {
        self.flush_close(Duration::from_secs(5))
    }
    /// Stops admission, cancels active/queued reads, drains pending records,
    /// closes the reader, then TRUNCATE-checkpoints and closes the writer.
    /// Only Ok proves both connections closed; on timeout do NOT copy source
    /// files. Retry this method to await completion. Never joins a running worker.
    pub fn flush_close(&mut self, timeout: Duration) -> Result<()> {
        if self.closed_ok {
            return Ok(());
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(Error::Invalid("timeout"))?;
        self.handle.accepting.store(false, Ordering::SeqCst);
        if let Some(reader) = &self.reader {
            reader.stop();
        }
        while self.handle.inflight.load(Ordering::SeqCst) != 0 {
            if Instant::now() >= deadline {
                return Err(Error::timeout());
            }
            thread::yield_now();
        }
        while !self.stop_sent {
            match self.handle.sender.try_send(Command::Stop) {
                Ok(()) => self.stop_sent = true,
                Err(TrySendError::Disconnected(_)) => self.stop_sent = true,
                Err(TrySendError::Full(_)) => {
                    if Instant::now() >= deadline {
                        return Err(Error::timeout());
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
        if let Some(reader) = &mut self.reader {
            reader.wait_closed(deadline)?;
        }
        if !self.reads_closed.load(Ordering::Acquire) {
            return Err(Error::timeout());
        }
        let result = self
            .completion
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|e| match e {
                mpsc::RecvTimeoutError::Timeout => Error::timeout(),
                mpsc::RecvTimeoutError::Disconnected => Error::Worker,
            })?;
        // Completion is sent only after checkpoint and explicit close have run.
        self.closed_ok = result.is_ok();
        result
    }
}
impl Drop for HistoryService {
    fn drop(&mut self) {
        let _ = self.flush_close(Duration::from_millis(100));
        // Dropping JoinHandle detaches; Drop must never join an active worker.
        let _ = self.worker.take();
    }
}
