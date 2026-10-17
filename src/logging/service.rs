use super::{
    files::{Files, error_kind},
    *,
};
use std::{
    collections::VecDeque,
    marker::PhantomData,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle, ThreadId},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_bounds_and_shutdown_timeout_do_not_wait_for_worker() {
        let (tx, _rx) = mpsc::sync_channel(1);
        let shared = Arc::new(Shared {
            memory: Mutex::new(Memory {
                entries: VecDeque::new(),
                sequence: 0,
                status: LogStatus::default(),
            }),
            dropped: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let handle = LogHandle { tx, shared };
        let event = LogEvent::Runtime {
            level: LogLevel::Info,
            event: RuntimeEvent::Started,
            error: None,
        };
        assert_eq!(handle.try_log(event), TryLogResult::Queued);
        for _ in 0..100 {
            assert_eq!(handle.try_log(event), TryLogResult::Full);
        }
        assert_eq!(handle.snapshot(None).status.dropped, 100);
        let mut service = LogService {
            handle,
            worker: None,
            owner: thread::current().id(),
            queue_capacity: 1,
            _controller_only: PhantomData,
        };
        assert!(matches!(
            service.shutdown(Duration::ZERO),
            Err(LogError::QueueFull)
        ));
        assert_eq!(service.handle().try_log(event), TryLogResult::Closed);
    }
    #[test]
    fn command_timeout_is_bounded() {
        let (tx, _rx) = mpsc::sync_channel(1);
        let shared = Arc::new(Shared {
            memory: Mutex::new(Memory {
                entries: VecDeque::new(),
                sequence: 0,
                status: LogStatus::default(),
            }),
            dropped: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let mut service = LogService {
            handle: LogHandle { tx, shared },
            worker: None,
            owner: thread::current().id(),
            queue_capacity: 1,
            _controller_only: PhantomData,
        };
        assert!(matches!(
            service.shutdown(Duration::from_millis(1)),
            Err(LogError::Timeout)
        ));
    }
}
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
struct Memory {
    entries: VecDeque<LogEntry>,
    sequence: u64,
    status: LogStatus,
}
struct Shared {
    memory: Mutex<Memory>,
    dropped: AtomicU64,
    stop: AtomicBool,
}
impl Shared {
    fn memory(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.memory.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn disk_error(&self, error: &LogError) {
        let mut memory = self.memory();
        memory.status.disk_errors = memory.status.disk_errors.saturating_add(1);
        memory.status.last_disk_error = Some(error_kind(error));
    }
}
#[derive(Clone)]
pub struct LogHandle {
    tx: SyncSender<Command>,
    shared: Arc<Shared>,
}
impl LogHandle {
    /// 无磁盘操作，不获取 snapshot 锁，不等待队列容量。
    pub fn try_log(&self, event: LogEvent) -> TryLogResult {
        if self.shared.stop.load(Ordering::Acquire) {
            return TryLogResult::Closed;
        }
        match self.tx.try_send(Command::Event(event)) {
            Ok(()) => TryLogResult::Queued,
            Err(TrySendError::Full(_)) => {
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                TryLogResult::Full
            }
            Err(TrySendError::Disconnected(_)) => TryLogResult::Closed,
        }
    }
    /// 只复制共享内存，不访问文件。after_sequence=None 返回全部有界缓存。
    pub fn snapshot(&self, after_sequence: Option<u64>) -> LogSnapshot {
        let memory = self.shared.memory();
        let mut status = memory.status.clone();
        status.dropped = self.shared.dropped.load(Ordering::Relaxed);
        let gap = after_sequence.is_some_and(|after| {
            memory
                .entries
                .front()
                .is_some_and(|first| after.saturating_add(1) < first.sequence)
        });
        LogSnapshot {
            entries: memory
                .entries
                .iter()
                .filter(|e| after_sequence.is_none_or(|after| e.sequence > after))
                .cloned()
                .collect(),
            cursor: memory.sequence,
            gap,
            status,
        }
    }
}
type Reply<T> = SyncSender<Result<T>>;
enum Command {
    Event(LogEvent),
    Configure(LogOptions, Reply<()>),
    List(Reply<Vec<LogFile>>),
    Tail(String, usize, usize, Reply<TailResult>),
    Page(String, PageCursor, ReadBounds, Reply<ReadResponse>),
    Follow(String, FileCursor, ReadBounds, Reply<ReadResponse>),
    Clear(bool, Reply<ClearReport>),
    Barrier(Reply<()>),
    Shutdown(Reply<()>),
}
/// 在后台 controller 线程创建并调用；!Send/!Sync，不能转移到 UI/网络线程。
/// 所有控制请求均有界排队，默认最多等 5 秒。超时不意味着已排队操作被撤回。
pub struct LogService {
    handle: LogHandle,
    worker: Option<JoinHandle<()>>,
    owner: ThreadId,
    queue_capacity: usize,
    _controller_only: PhantomData<Rc<()>>,
}
impl LogService {
    pub fn start(directory: impl Into<PathBuf>, options: LogOptions) -> Result<Self> {
        options.validate()?;
        let queue_capacity = options.queue_capacity;
        let (tx, rx) = mpsc::sync_channel(queue_capacity);
        let shared = Arc::new(Shared {
            memory: Mutex::new(Memory {
                entries: VecDeque::new(),
                sequence: 0,
                status: LogStatus::default(),
            }),
            dropped: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let worker_shared = shared.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let directory = directory.into();
        let worker = thread::Builder::new()
            .name("ai-relay-logging".into())
            .spawn(move || {
                match Files::start(directory) {
                    Ok(mut files) => {
                        // 持久化失败可见但不禁用实时日志。
                        if let Err(error) = files.configure(&options) {
                            worker_shared.disk_error(&error);
                        }
                        let _ = ready_tx.send(Ok(()));
                        run(rx, &worker_shared, files, options);
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
                worker_shared.stop.store(true, Ordering::Release);
                worker_shared.memory().status.worker_stopped = true;
            })?;
        match ready_rx.recv_timeout(COMMAND_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                handle: LogHandle { tx, shared },
                worker: Some(worker),
                owner: thread::current().id(),
                queue_capacity,
                _controller_only: PhantomData,
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                shared.stop.store(true, Ordering::Release);
                Err(LogError::Timeout)
            }
        }
    }
    pub fn handle(&self) -> LogHandle {
        self.handle.clone()
    }
    fn request<T>(&self, timeout: Duration, make: impl FnOnce(Reply<T>) -> Command) -> Result<T> {
        if thread::current().id() != self.owner {
            return Err(LogError::WrongThread);
        }
        if self.handle.shared.stop.load(Ordering::Acquire) {
            return Err(LogError::Closed);
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.handle
            .tx
            .try_send(make(tx))
            .map_err(|error| match error {
                TrySendError::Full(_) => LogError::QueueFull,
                TrySendError::Disconnected(_) => LogError::Closed,
            })?;
        rx.recv_timeout(timeout).map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => LogError::Timeout,
            mpsc::RecvTimeoutError::Disconnected => LogError::Closed,
        })?
    }
    /// 队列容量创建后固定；重配不得改变它。
    pub fn configure(&self, options: LogOptions) -> Result<()> {
        options.validate()?;
        if options.queue_capacity != self.queue_capacity {
            return Err(LogError::InvalidOptions);
        }
        self.request(COMMAND_TIMEOUT, |reply| Command::Configure(options, reply))
    }
    pub fn list_files(&self) -> Result<Vec<LogFile>> {
        self.request(COMMAND_TIMEOUT, Command::List)
    }
    pub fn read_tail(&self, name: &str, max_bytes: usize, max_lines: usize) -> Result<TailResult> {
        if super::files::file_index(name).is_none() {
            return Err(LogError::InvalidName);
        }
        if !(1..=MAX_TAIL_BYTES).contains(&max_bytes) || !(1..=MAX_TAIL_LINES).contains(&max_lines)
        {
            return Err(LogError::InvalidTailBounds);
        }
        self.request(COMMAND_TIMEOUT, |reply| {
            Command::Tail(name.into(), max_bytes, max_lines, reply)
        })
    }
    /// 阻塞控制 API，仅在创建本服务的后台 controller 调用；磁盘读取在日志 worker。
    pub fn read_page(
        &self,
        name: &str,
        cursor: PageCursor,
        bounds: ReadBounds,
    ) -> Result<ReadResponse> {
        Self::validate_read(name, bounds)?;
        self.request(COMMAND_TIMEOUT, |reply| {
            Command::Page(name.into(), cursor, bounds, reply)
        })
    }
    /// 从上次响应的 follow 位置增量读取；不等待未来追加，不自动重置变更的文件。
    pub fn read_follow(
        &self,
        name: &str,
        cursor: FileCursor,
        bounds: ReadBounds,
    ) -> Result<ReadResponse> {
        Self::validate_read(name, bounds)?;
        self.request(COMMAND_TIMEOUT, |reply| {
            Command::Follow(name.into(), cursor, bounds, reply)
        })
    }
    fn validate_read(name: &str, bounds: ReadBounds) -> Result<()> {
        if super::files::file_index(name).is_none() {
            return Err(LogError::InvalidName);
        }
        bounds.validate()
    }
    /// 调用方负责展示确认；本方法仅删除严格命名的已关闭历史日志。
    pub fn clear_history(&self) -> Result<ClearReport> {
        self.request(COMMAND_TIMEOUT, |r| Command::Clear(false, r))
    }
    /// 调用方负责二次确认；关闭当前句柄、删除、按开关重开。
    pub fn clear_all(&self) -> Result<ClearReport> {
        self.request(COMMAND_TIMEOUT, |r| Command::Clear(true, r))
    }
    /// 等待此前接受事件处理完成，不执行 fsync，不代表物理介质持久化。
    pub fn barrier(&self) -> Result<()> {
        self.request(COMMAND_TIMEOUT, Command::Barrier)
    }
    /// 成功时此前排队事件已处理；超时/满队列时不 join，不无限等待磁盘。
    /// 超时后置停止标记，未处理事件可能丢失。
    pub fn shutdown(&mut self, timeout: Duration) -> Result<()> {
        let result = self.request(timeout, Command::Shutdown);
        self.handle.shared.stop.store(true, Ordering::Release);
        // 不 join：即使完成应答后，线程退出也不应使 controller 无限等待。
        self.worker.take();
        result
    }
}
impl Drop for LogService {
    fn drop(&mut self) {
        self.handle.shared.stop.store(true, Ordering::Release);
    }
}
fn run(rx: Receiver<Command>, shared: &Shared, mut files: Files, mut options: LogOptions) {
    let mut last_prune = std::time::Instant::now();
    while !shared.stop.load(Ordering::Acquire) {
        let command = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_prune.elapsed() >= Duration::from_secs(60) {
                    if let Err(e) = files.prune(&options) {
                        shared.disk_error(&e);
                    }
                    last_prune = std::time::Instant::now();
                }
                continue;
            }
            Err(_) => break,
        };
        // Also prune under continuous traffic, not just during idle time.
        if last_prune.elapsed() >= Duration::from_secs(60) {
            if let Err(e) = files.prune(&options) {
                shared.disk_error(&e);
            }
            last_prune = std::time::Instant::now();
        }
        match command {
            Command::Event(event) => {
                if !enabled(event, &options) {
                    continue;
                }
                let entry = {
                    let mut memory = shared.memory();
                    memory.sequence = memory.sequence.saturating_add(1);
                    let entry = LogEntry {
                        sequence: memory.sequence,
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        event,
                    };
                    if memory.entries.len() >= options.recent_entries {
                        memory.entries.pop_front();
                    }
                    memory.entries.push_back(entry.clone());
                    entry
                };
                if options.file_enabled {
                    match files.write(&entry, &options) {
                        Ok(()) => {
                            shared.memory().status.last_persisted_sequence = Some(entry.sequence)
                        }
                        Err(error) => shared.disk_error(&error),
                    }
                }
            }
            Command::Configure(next, reply) => {
                options = next;
                {
                    let mut memory = shared.memory();
                    while memory.entries.len() > options.recent_entries {
                        memory.entries.pop_front();
                    }
                }
                let result = files.configure(&options);
                if let Err(e) = &result {
                    shared.disk_error(e);
                }
                let _ = reply.send(result);
            }
            Command::List(reply) => {
                let _ = reply.send(files.list());
            }
            Command::Tail(name, bytes, lines, reply) => {
                let _ = reply.send(files.tail(&name, bytes, lines));
            }
            Command::Page(name, cursor, bounds, reply) => {
                let _ = reply.send(files.read_page(&name, cursor, bounds));
            }
            Command::Follow(name, cursor, bounds, reply) => {
                let _ = reply.send(files.read_follow(&name, cursor, bounds));
            }
            Command::Clear(all, reply) => {
                let result = files.clear(all, &options);
                match &result {
                    Err(e) => shared.disk_error(e),
                    Ok(report) => {
                        for failure in &report.errors {
                            shared.disk_error(&LogError::Io(std::io::Error::from(failure.kind)));
                        }
                    }
                }
                let _ = reply.send(result);
            }
            Command::Barrier(reply) => {
                let _ = reply.send(Ok(()));
            }
            Command::Shutdown(reply) => {
                drop(files);
                let _ = reply.send(Ok(()));
                break;
            }
        }
    }
}
