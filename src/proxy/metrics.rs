//! Session counters: atomics only, no I/O, locks, body contents or credentials.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Reuse one `Arc` across starts to retain this program session's counters.
/// Uptime starts at construction and includes time stopped between listeners.
#[derive(Debug)]
pub struct ProxyMetrics {
    started: Instant,
    total_requests: AtomicU64,
    errors: AtomicU64,
    active_requests: AtomicU64,
    active_tunnels: AtomicU64,
    inbound_connections: AtomicU64,
    upload_body_bytes: AtomicU64,
    download_body_bytes: AtomicU64,
    saturated: AtomicBool,
}

/// Individually atomic, approximate live view (not a transactional snapshot).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProxyMetricsSnapshot {
    /// Parsed non-health HTTP requests, including CONNECT and local replies.
    pub total_requests: u64,
    /// One per terminal HTTP >=400, transport error, timeout or cancellation.
    /// Successful local CORS preflights are not errors; local refusals are.
    pub errors: u64,
    /// Pending HTTP exchanges, excluding CONNECT once its upstream is connected.
    pub active_requests: u64,
    /// Connected CONNECT tunnels, including their bounded upgrade wait.
    pub active_tunnels: u64,
    /// Live admitted inbound TCP connections, including idle and health sockets.
    pub inbound_connections: u64,
    /// HTTP data bytes handed to upstream, plus CONNECT bytes written upstream.
    /// Excludes HTTP framing, headers and unconsumed/rejected request bodies.
    pub upload_body_bytes: u64,
    /// HTTP data bytes handed downstream, plus CONNECT bytes written downstream.
    /// Excludes framing, headers and locally generated replies (including health).
    /// Handed-off HTTP data is not a TCP delivery/acknowledgement guarantee.
    pub download_body_bytes: u64,
    pub uptime: Duration,
    /// Sticky: at least one counter addition exceeded u64::MAX; never wraps.
    pub saturated: bool,
}

impl Default for ProxyMetrics {
    fn default() -> Self {
        Self::new()
    }
}
impl ProxyMetrics {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            total_requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            active_requests: AtomicU64::new(0),
            active_tunnels: AtomicU64::new(0),
            inbound_connections: AtomicU64::new(0),
            upload_body_bytes: AtomicU64::new(0),
            download_body_bytes: AtomicU64::new(0),
            saturated: AtomicBool::new(false),
        }
    }

    pub fn snapshot(&self) -> ProxyMetricsSnapshot {
        ProxyMetricsSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            active_requests: self.active_requests.load(Ordering::Relaxed),
            active_tunnels: self.active_tunnels.load(Ordering::Relaxed),
            inbound_connections: self.inbound_connections.load(Ordering::Relaxed),
            upload_body_bytes: self.upload_body_bytes.load(Ordering::Relaxed),
            download_body_bytes: self.download_body_bytes.load(Ordering::Relaxed),
            uptime: self.started.elapsed(),
            saturated: self.saturated.load(Ordering::Relaxed),
        }
    }

    fn add(&self, counter: &AtomicU64, amount: u64) {
        let mut value = counter.load(Ordering::Relaxed);
        loop {
            match counter.compare_exchange_weak(
                value,
                value.saturating_add(amount),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    if value.checked_add(amount).is_none() {
                        self.saturated.store(true, Ordering::Relaxed);
                    }
                    break;
                }
                Err(current) => value = current,
            }
        }
    }

    pub(super) fn upload(&self, bytes: usize) {
        self.add(&self.upload_body_bytes, bytes as u64);
    }
    pub(super) fn download(&self, bytes: usize) {
        self.add(&self.download_body_bytes, bytes as u64);
    }
    pub(super) fn connection(self: &Arc<Self>) -> ConnectionGuard {
        self.add(&self.inbound_connections, 1);
        ConnectionGuard(self.clone())
    }
    pub(super) fn request(self: &Arc<Self>) -> RequestGuard {
        self.add(&self.total_requests, 1);
        self.add(&self.active_requests, 1);
        RequestGuard {
            metrics: self.clone(),
            tunnel: false,
            finished: false,
        }
    }
}
fn decrement(counter: &AtomicU64) {
    let mut value = counter.load(Ordering::Relaxed);
    loop {
        match counter.compare_exchange_weak(
            value,
            value.saturating_sub(1),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(current) => value = current,
        }
    }
}

pub(super) struct ConnectionGuard(Arc<ProxyMetrics>);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        decrement(&self.0.inbound_connections);
    }
}

pub(super) struct RequestGuard {
    metrics: Arc<ProxyMetrics>,
    tunnel: bool,
    finished: bool,
}
impl RequestGuard {
    pub(super) fn tunnel(&mut self) {
        if !self.finished && !self.tunnel {
            self.tunnel = true;
            self.metrics.add(&self.metrics.active_tunnels, 1);
            decrement(&self.metrics.active_requests);
        }
    }
    pub(super) fn download(&self, bytes: usize) {
        self.metrics.download(bytes);
    }
    pub(super) fn finish(&mut self, error: bool) {
        if self.finished {
            return;
        }
        self.finished = true;
        if error {
            self.metrics.add(&self.metrics.errors, 1);
        }
        decrement(if self.tunnel {
            &self.metrics.active_tunnels
        } else {
            &self.metrics.active_requests
        });
    }
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.finish(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guards_have_exactly_one_terminal_state() {
        let metrics = Arc::new(ProxyMetrics::new());
        let connection = metrics.connection();
        let mut request = metrics.request();
        assert_eq!(metrics.snapshot().active_requests, 1);
        request.tunnel();
        request.tunnel();
        assert_eq!(metrics.snapshot().active_requests, 0);
        assert_eq!(metrics.snapshot().active_tunnels, 1);
        request.finish(false);
        request.finish(true);
        drop(request);
        drop(connection);
        drop(metrics.request());
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_requests, 2);
        assert_eq!(snapshot.errors, 1);
        assert_eq!(snapshot.active_requests, 0);
        assert_eq!(snapshot.active_tunnels, 0);
        assert_eq!(snapshot.inbound_connections, 0);
    }

    #[test]
    fn additions_saturate_and_report_overflow_without_wrapping() {
        let metrics = Arc::new(ProxyMetrics::new());
        metrics.total_requests.store(u64::MAX, Ordering::Relaxed);
        metrics
            .upload_body_bytes
            .store(u64::MAX - 1, Ordering::Relaxed);
        metrics
            .download_body_bytes
            .store(u64::MAX, Ordering::Relaxed);
        let mut guard = metrics.request();
        metrics.upload(3);
        metrics.download(1);
        guard.finish(false);
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_requests, u64::MAX);
        assert_eq!(snapshot.upload_body_bytes, u64::MAX);
        assert_eq!(snapshot.download_body_bytes, u64::MAX);
        assert_eq!(snapshot.active_requests, 0);
        assert!(snapshot.saturated);
    }
}
