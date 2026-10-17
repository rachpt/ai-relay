//! One read actor per service; at most four admitted requests including active
//! work and writer-barrier waiters. Never runs on, or awaits, the record owner.
use super::*;
use std::sync::{Mutex, atomic::AtomicUsize};

const CAPACITY: usize = 4;
const READ_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) struct Budget {
    id: u64,
    deadline: Instant,
    cancelled: AtomicBool,
    stopping: Arc<AtomicBool>,
}
impl Budget {
    pub(super) fn check(&self) -> Result<()> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        if self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline {
            return Err(read_timeout());
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn for_test(timeout: Duration) -> Self {
        Self {
            id: 0,
            deadline: Instant::now() + timeout,
            cancelled: AtomicBool::new(false),
            stopping: Arc::new(AtomicBool::new(false)),
        }
    }
}
fn read_timeout() -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "history read deadline exceeded",
    ))
}
struct Shared {
    stopping: Arc<AtomicBool>,
    outstanding: AtomicUsize,
    next: AtomicU64,
    active: Mutex<Option<u64>>,
    interrupt: rusqlite::InterruptHandle,
}
impl Shared {
    fn cancel(&self, budget: &Budget) {
        budget.cancelled.store(true, Ordering::Release);
        // Serialize with task transitions so a timed-out queued request cannot
        // interrupt an unrelated request that starts at the same instant.
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if *active == Some(budget.id) {
            self.interrupt.interrupt();
        }
    }
    fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        // With no active task, the actor may already be inside close(). Avoid
        // contending with rusqlite's close mutex; the stop flag covers idle work.
        if active.is_some() {
            self.interrupt.interrupt();
        }
    }
}
struct Permit(Arc<Shared>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.outstanding.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Request {
    budget: Arc<Budget>,
    _permit: Permit,
}
enum Read {
    Query(Box<Query>, Request, mpsc::Sender<Result<QueryReport>>),
    Export(
        Box<Query>,
        PathBuf,
        Request,
        mpsc::Sender<ExportResult<ExportSummary>>,
    ),
    Aliases(Request, mpsc::Sender<Result<Vec<CredentialAliasEntry>>>),
    #[cfg(test)]
    Sql(
        &'static str,
        Request,
        mpsc::Sender<()>,
        mpsc::Sender<Result<()>>,
    ),
    #[cfg(test)]
    HoldSnapshot(
        Request,
        mpsc::Sender<()>,
        mpsc::Receiver<()>,
        mpsc::Sender<Result<()>>,
    ),
}
impl Read {
    fn budget(&self) -> &Budget {
        match self {
            Self::Query(_, r, _) | Self::Export(_, _, r, _) | Self::Aliases(r, _) => &r.budget,
            #[cfg(test)]
            Self::Sql(_, r, _, _) | Self::HoldSnapshot(r, _, _, _) => &r.budget,
        }
    }
}
#[derive(Clone)]
pub(super) struct Client {
    sender: SyncSender<Read>,
    shared: Arc<Shared>,
}
pub(super) struct Reader {
    pub(super) client: Client,
    worker: Option<JoinHandle<()>>,
    completion: mpsc::Receiver<Result<()>>,
    closed_ok: bool,
    failed: bool,
}
impl Reader {
    pub(super) fn start(
        dir: PathBuf,
        closed: Arc<AtomicBool>,
        metrics: Arc<Metrics>,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Read>(CAPACITY);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("history-reader".into())
            .spawn(move || {
                struct Closed(Arc<AtomicBool>);
                impl Drop for Closed {
                    fn drop(&mut self) {
                        self.0.store(true, Ordering::Release);
                    }
                }
                let _closed = Closed(closed);
                let mut conn = match db::open_reader(&dir) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let shared = Arc::new(Shared {
                    stopping: Arc::new(AtomicBool::new(false)),
                    outstanding: AtomicUsize::new(0),
                    next: AtomicU64::new(0),
                    active: Mutex::new(None),
                    interrupt: conn.get_interrupt_handle(),
                });
                if ready_tx.send(Ok(shared.clone())).is_err() {
                    return;
                }
                loop {
                    let command = match receiver.recv_timeout(Duration::from_millis(10)) {
                        Ok(c) => c,
                        Err(mpsc::RecvTimeoutError::Timeout)
                            if !shared.stopping.load(Ordering::Acquire) =>
                        {
                            continue;
                        }
                        Err(_) => break,
                    };
                    let id = command.budget().id;
                    *shared.active.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
                    match command {
                        Read::Query(q, request, reply) => {
                            let result = request
                                .budget
                                .check()
                                .and_then(|_| query::run(&mut conn, *q, &request.budget));
                            let result = request.budget.check().and(result);
                            if matches!(result, Err(Error::Database(_)))
                                && !shared.stopping.load(Ordering::Acquire)
                            {
                                metrics.db_failures.fetch_add(1, Ordering::Relaxed);
                            }
                            let _ = reply.send(result);
                        }
                        Read::Export(q, path, request, reply) => {
                            let result = match request.budget.check() {
                                Ok(()) => export::run(&mut conn, *q, path, &request.budget),
                                Err(cause) => Err(ExportError {
                                    cause,
                                    path,
                                    rows_written: 0,
                                    file_state: ExportFileState::NotCreated,
                                }),
                            };
                            if matches!(
                                result,
                                Err(ExportError {
                                    cause: Error::Database(_),
                                    ..
                                })
                            ) && request.budget.check().is_ok()
                            {
                                metrics.db_failures.fetch_add(1, Ordering::Relaxed);
                            }
                            let _ = reply.send(result);
                        }
                        Read::Aliases(request, reply) => {
                            let result = request
                                .budget
                                .check()
                                .and_then(|_| db::aliases(&conn, &request.budget));
                            let _ = reply.send(request.budget.check().and(result));
                        }
                        #[cfg(test)]
                        Read::Sql(sql, request, started, reply) => {
                            let result = request.budget.check().and_then(|_| {
                                let _ = started.send(());
                                conn.execute_batch(sql).map_err(Error::from)
                            });
                            let _ = reply.send(request.budget.check().and(result));
                        }
                        #[cfg(test)]
                        Read::HoldSnapshot(request, started, release, reply) => {
                            let result = (|| -> Result<()> {
                                let tx = conn.transaction()?;
                                tx.query_row("SELECT count(*) FROM records", [], |r| {
                                    r.get::<_, i64>(0)
                                })?;
                                let _ = started.send(());
                                // Models an OS file operation that sqlite_interrupt
                                // cannot cancel; tests must release it explicitly.
                                release
                                    .recv_timeout(Duration::from_secs(5))
                                    .map_err(|_| read_timeout())?;
                                request.budget.check()?;
                                tx.commit()?;
                                Ok(())
                            })();
                            let _ = reply.send(result);
                        }
                    }
                    *shared.active.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
                // Close the read connection (including TEMP/snapshot state) before
                // unblocking the writer's final checkpoint. No read-side checkpoint.
                let result = conn.close().map_err(|(conn, e)| {
                    drop(conn);
                    Error::Database(e)
                });
                drop(_closed);
                let _ = done_tx.send(result);
            })?;
        let shared = ready_rx.recv().map_err(|_| Error::Worker)??;
        Ok(Self {
            client: Client { sender, shared },
            worker: Some(worker),
            completion: done_rx,
            closed_ok: false,
            failed: false,
        })
    }
    pub(super) fn stop(&self) {
        self.client.shared.stop();
    }
    pub(super) fn wait_closed(&mut self, deadline: Instant) -> Result<()> {
        if self.closed_ok {
            return Ok(());
        }
        if self.failed {
            return Err(Error::Worker);
        }
        let result = self
            .completion
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|e| match e {
                mpsc::RecvTimeoutError::Timeout => Error::timeout(),
                mpsc::RecvTimeoutError::Disconnected => Error::Worker,
            })?;
        self.closed_ok = result.is_ok();
        self.failed = result.is_err();
        result
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.stop();
        // File I/O can outlive the caller's deadline. Never join an active actor.
        let _ = self.worker.take();
    }
}
impl Client {
    fn admit(&self, timeout: Duration) -> Result<Request> {
        if self.shared.stopping.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(Error::Invalid("read timeout"))?;
        self.shared
            .outstanding
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < CAPACITY).then_some(n + 1)
            })
            .map_err(|_| Error::Busy)?;
        Ok(Request {
            budget: Arc::new(Budget {
                id: self.shared.next.fetch_add(1, Ordering::Relaxed),
                deadline,
                cancelled: AtomicBool::new(false),
                stopping: self.shared.stopping.clone(),
            }),
            _permit: Permit(self.shared.clone()),
        })
    }
    fn send(&self, command: Read) -> Result<()> {
        command.budget().check()?;
        self.sender.try_send(command).map_err(|e| match e {
            TrySendError::Full(_) => Error::Busy,
            TrySendError::Disconnected(_) => Error::Closed,
        })
    }
    fn wait<T>(&self, rx: mpsc::Receiver<T>, budget: &Budget) -> Result<T> {
        match rx.recv_timeout(budget.deadline.saturating_duration_since(Instant::now())) {
            Ok(value) => Ok(value),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.shared.cancel(budget);
                Err(read_timeout())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::Worker),
        }
    }
    pub(super) fn query(&self, writer: &HistoryHandle, q: Query) -> Result<QueryReport> {
        query::validate(&q, true)?;
        let request = self.admit(READ_TIMEOUT)?;
        writer_barrier(writer, request.budget.deadline)?;
        let budget = request.budget.clone();
        let (tx, rx) = mpsc::channel();
        self.send(Read::Query(Box::new(q), request, tx))?;
        self.wait(rx, &budget)?
    }
    pub(super) fn export(
        &self,
        writer: &HistoryHandle,
        q: Query,
        path: PathBuf,
    ) -> ExportResult<ExportSummary> {
        let result = (|| -> Result<_> {
            query::validate(&q, false)?;
            identity::check_path(&path)?;
            let request = self.admit(READ_TIMEOUT)?;
            writer_barrier(writer, request.budget.deadline)?;
            let budget = request.budget.clone();
            let (tx, rx) = mpsc::channel();
            self.send(Read::Export(Box::new(q), path.clone(), request, tx))?;
            Ok((rx, budget))
        })();
        let (rx, budget) = result.map_err(|cause| ExportError {
            cause,
            path: path.clone(),
            rows_written: 0,
            file_state: ExportFileState::NotCreated,
        })?;
        self.wait(rx, &budget).unwrap_or_else(|cause| {
            Err(ExportError {
                cause,
                path,
                rows_written: 0,
                file_state: ExportFileState::Unknown,
            })
        })
    }
    pub(super) fn aliases(&self, writer: &HistoryHandle) -> Result<Vec<CredentialAliasEntry>> {
        let request = self.admit(READ_TIMEOUT)?;
        writer_barrier(writer, request.budget.deadline)?;
        let budget = request.budget.clone();
        let (tx, rx) = mpsc::channel();
        self.send(Read::Aliases(request, tx))?;
        self.wait(rx, &budget)?
    }
}
pub(super) fn writer_barrier(writer: &HistoryHandle, deadline: Instant) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut command = Command::Barrier(tx);
    loop {
        if !writer.accepting.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        match writer.sender.try_send(command) {
            Ok(()) => break,
            Err(TrySendError::Full(c)) => command = c,
            Err(TrySendError::Disconnected(_)) => return Err(Error::Closed),
        }
        if Instant::now() >= deadline {
            return Err(read_timeout());
        }
        thread::sleep(Duration::from_millis(1));
    }
    rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => read_timeout(),
            mpsc::RecvTimeoutError::Disconnected => Error::Worker,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "relay-history-reader-unit-{}",
                identity::hex(&identity::random::<16>().unwrap())
            ));
            std::fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn record(h: &HistoryHandle) -> RequestRecord {
        RequestRecord {
            request_id: h.request_id(),
            started_ms: now_ms(),
            peer_ip: "127.0.0.1".parse().unwrap(),
            client: None,
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
        }
    }
    fn hold(client: &Client) -> (mpsc::Sender<()>, mpsc::Receiver<Result<()>>) {
        let request = client.admit(Duration::from_secs(10)).unwrap();
        let (started, start_rx) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let (reply, rx) = mpsc::channel();
        client
            .send(Read::HoldSnapshot(request, started, wait, reply))
            .unwrap();
        start_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        (release, rx)
    }
    const LONG_SQL: &str = "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n;";
    #[test]
    fn reader_main_is_readonly_temp_writes_work_and_open_never_migrates_or_retains() {
        let temp = Temp::new();
        let writer = db::open(&temp.0, Options::default()).unwrap();
        writer
            .execute_batch("CREATE TABLE sentinel(value INTEGER); INSERT INTO sentinel VALUES (7);")
            .unwrap();
        let old = now_ms() - 100 * 86_400_000;
        writer.execute("INSERT INTO records VALUES('old',?,'127.0.0.1',NULL,NULL,NULL,NULL,NULL,'unknown','unknown',NULL,'cancelled',0,0,'{}')",[old]).unwrap();
        let read = db::open_reader(&temp.0).unwrap();
        assert!(read.is_readonly("main").unwrap());
        assert!(
            read.execute("UPDATE main.sentinel SET value=8", [])
                .is_err()
        );
        assert!(
            read.execute_batch("CREATE TABLE main.forbidden(value INTEGER)")
                .is_err()
        );
        read.execute_batch(
            "CREATE TEMP TABLE scratch(value INTEGER); INSERT INTO temp.scratch VALUES(1);",
        )
        .unwrap();
        assert_eq!(
            read.query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        read.close().unwrap();
        writer.pragma_update(None, "user_version", 1).unwrap();
        assert!(db::open_reader(&temp.0).is_err());
        assert_eq!(
            writer
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            writer
                .query_row("SELECT value FROM sentinel", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            7
        );
        writer.pragma_update(None, "user_version", 2).unwrap();
        db::close(writer).unwrap();
        assert!(db::open_reader(&temp.0.join("missing")).is_err());
        assert!(!temp.0.join("missing").exists());
    }
    #[test]
    fn reader_admission_is_bounded_and_never_blocks_writer_controls() {
        let temp = Temp::new();
        let mut s = HistoryService::start(&temp.0).unwrap();
        let h = s.handle();
        let client = s.reader.as_ref().unwrap().client.clone();
        let (release, held) = hold(&client);
        let mut queued = Vec::new();
        for _ in 0..CAPACITY - 1 {
            let request = client.admit(Duration::from_secs(10)).unwrap();
            let (tx, rx) = mpsc::channel();
            client
                .send(Read::Query(Box::default(), request, tx))
                .unwrap();
            queued.push(rx);
        }
        assert_eq!(client.shared.outstanding.load(Ordering::Acquire), CAPACITY);
        let before = Instant::now();
        assert!(matches!(s.query(Query::default()), Err(Error::Busy)));
        assert!(before.elapsed() < Duration::from_secs(1));
        let error = s
            .export_all(Query::default(), temp.0.join("busy.csv"))
            .unwrap_err();
        assert!(matches!(error.cause, Error::Busy));
        assert_eq!(error.file_state, ExportFileState::NotCreated);
        assert!(!temp.0.join("busy.csv").exists());
        assert_eq!(h.try_record(record(&h)), TryRecord::Queued);
        s.barrier().unwrap();
        assert_eq!(h.counters().written, 1);
        s.set_alias(
            h.credential("synthetic-reader-control"),
            CredentialAlias::new("still writable").unwrap(),
        )
        .unwrap();
        let policy = s.configure(Options::default()).unwrap();
        assert_eq!(policy.checkpoint, CheckpointStatus::Busy);
        assert!(before.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
        held.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        for rx in queued {
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(2))
                    .unwrap()
                    .unwrap()
                    .summary
                    .requests,
                1
            );
        }
        s.shutdown().unwrap();
    }
    #[test]
    fn read_deadline_interrupts_sql_and_the_same_actor_remains_usable() {
        let temp = Temp::new();
        let mut s = HistoryService::start(&temp.0).unwrap();
        let client = s.reader.as_ref().unwrap().client.clone();
        let request = client.admit(Duration::from_millis(100)).unwrap();
        let budget = request.budget.clone();
        let (started, ready) = mpsc::channel();
        let (reply, rx) = mpsc::channel();
        client
            .send(Read::Sql(LONG_SQL, request, started, reply))
            .unwrap();
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        let before = Instant::now();
        assert!(client.wait(rx, &budget).unwrap_err().is_timeout());
        assert!(before.elapsed() < Duration::from_secs(1));
        assert_eq!(s.query(Query::default()).unwrap().summary.requests, 0);
        assert_eq!(client.shared.outstanding.load(Ordering::Acquire), 0);
        s.shutdown().unwrap();
    }
    #[test]
    fn flush_close_cancels_active_sql_and_queued_export_before_checkpoint_proof() {
        let temp = Temp::new();
        let mut s = HistoryService::start(&temp.0).unwrap();
        let h = s.handle();
        let client = s.reader.as_ref().unwrap().client.clone();
        let request = client.admit(Duration::from_secs(10)).unwrap();
        let (started, ready) = mpsc::channel();
        let (reply, rx) = mpsc::channel();
        client
            .send(Read::Sql(LONG_SQL, request, started, reply))
            .unwrap();
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        let request = client.admit(Duration::from_secs(10)).unwrap();
        let (export_tx, export_rx) = mpsc::channel();
        let path = temp.0.join("cancelled.csv");
        client
            .send(Read::Export(
                Box::default(),
                path.clone(),
                request,
                export_tx,
            ))
            .unwrap();
        for _ in 0..20 {
            assert_eq!(h.try_record(record(&h)), TryRecord::Queued);
        }
        let before = Instant::now();
        s.flush_close(Duration::from_secs(2)).unwrap();
        assert!(before.elapsed() < Duration::from_secs(2));
        assert!(matches!(rx.recv().unwrap(), Err(Error::Closed)));
        let export = export_rx.recv().unwrap().unwrap_err();
        assert!(matches!(export.cause, Error::Closed));
        assert_eq!(export.file_state, ExportFileState::NotCreated);
        assert!(!path.exists());
        assert_eq!(h.counters().written, 20);
        assert!(s.reads_closed.load(Ordering::Acquire));
        assert_eq!(h.try_record(record(&h)), TryRecord::Closed);
        s.flush_close(Duration::ZERO).unwrap();
        assert_eq!(
            std::fs::metadata(temp.0.join("history.sqlite3-wal"))
                .map(|m| m.len())
                .unwrap_or(0),
            0
        );
        let mut reopened = HistoryService::start(&temp.0).unwrap();
        assert_eq!(
            reopened.query(Query::default()).unwrap().summary.requests,
            20
        );
        reopened.shutdown().unwrap();
    }
    #[test]
    fn closure_timeout_is_not_proof_and_retry_waits_for_uninterruptible_read() {
        let temp = Temp::new();
        let mut s = HistoryService::start(&temp.0).unwrap();
        let client = s.reader.as_ref().unwrap().client.clone();
        let (release, held) = hold(&client);
        let queued = client.admit(Duration::from_secs(10)).unwrap();
        let (reply, queued_rx) = mpsc::channel();
        client
            .send(Read::Query(Box::default(), queued, reply))
            .unwrap();
        let before = Instant::now();
        assert!(
            s.flush_close(Duration::from_millis(20))
                .unwrap_err()
                .is_timeout()
        );
        assert!(before.elapsed() < Duration::from_secs(1));
        assert!(!s.reads_closed.load(Ordering::Acquire));
        release.send(()).unwrap();
        assert!(matches!(
            held.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err(Error::Closed)
        ));
        s.flush_close(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            queued_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err(Error::Closed)
        ));
        assert!(s.reads_closed.load(Ordering::Acquire));
    }
    #[test]
    fn drop_never_joins_a_stalled_read_actor() {
        let temp = Temp::new();
        let s = HistoryService::start(&temp.0).unwrap();
        let client = s.reader.as_ref().unwrap().client.clone();
        let closed = s.reads_closed.clone();
        let (release, held) = hold(&client);
        let before = Instant::now();
        drop(s);
        assert!(before.elapsed() < Duration::from_secs(1));
        assert!(!closed.load(Ordering::Acquire));
        release.send(()).unwrap();
        assert!(held.recv_timeout(Duration::from_secs(1)).unwrap().is_err());
        let deadline = Instant::now() + Duration::from_secs(2);
        while !closed.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(closed.load(Ordering::Acquire));
    }
}
