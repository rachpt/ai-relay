//! Terminal-only access observations. No URI, headers, body or raw errors retained.
use std::{
    net::SocketAddr,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::logging::{AccessMetadata, Endpoint, ErrorCode, LogEvent, LogHandle, LogLevel, Method};
use hyper::StatusCode;
use tokio::time::Instant;

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

/// Lives in the pending request future, then in the upstream response body.
/// Drop is cancellation only; explicit terminal paths consume the handle first.
/// Exactly one try_log attempt, not guaranteed delivery when the backend is full,
/// closed, or configured to filter access events. Never performs I/O or waits.
pub(super) struct AccessRecording {
    log: Option<LogHandle>,
    started: Instant,
    metadata: AccessMetadata,
    history: Option<super::history::HistoryRecording>,
    usage: Option<super::body::UsageRecording>,
    local_reply: bool,
}
impl AccessRecording {
    pub(super) fn new(
        log: Option<LogHandle>,
        peer: SocketAddr,
        method: &hyper::Method,
        path: &str,
    ) -> Self {
        let method = match *method {
            hyper::Method::GET => Method::Get,
            hyper::Method::POST => Method::Post,
            hyper::Method::PUT => Method::Put,
            hyper::Method::PATCH => Method::Patch,
            hyper::Method::DELETE => Method::Delete,
            hyper::Method::HEAD => Method::Head,
            hyper::Method::OPTIONS => Method::Options,
            hyper::Method::CONNECT => Method::Connect,
            _ => Method::Other,
        };
        Self {
            log,
            started: Instant::now(),
            history: None,
            usage: None,
            local_reply: false,
            metadata: AccessMetadata {
                request_id: Some(NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)),
                ip: Some(peer.ip()),
                method: Some(method),
                // CONNECT is authority-form; never classify its target as a path.
                endpoint: Some(if method == Method::Connect {
                    Endpoint::Redacted
                } else {
                    Endpoint::from_path(path)
                }),
                ..Default::default()
            },
        }
    }

    pub(super) fn set_history(&mut self, history: super::history::HistoryRecording) {
        self.history = Some(history);
    }

    pub(super) fn upstream(
        &mut self,
        uri: &hyper::Uri,
        headers: &hyper::HeaderMap,
        store: std::sync::Arc<crate::usage::UsageStore>,
    ) {
        if let Some(history) = &mut self.history {
            history.target(uri, headers);
        }
        self.usage = Some(super::body::UsageRecording::new(store));
    }

    pub(super) fn usage_headers(&mut self, headers: &hyper::HeaderMap) {
        if let Some(usage) = &mut self.usage {
            usage.response_headers(headers);
        }
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) {
        if let Some(usage) = &mut self.usage {
            usage.feed(bytes);
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }

    /// First byte means upstream response headers available, not first body data.
    /// Local replies and cancellation/failure before headers leave this absent.
    pub(super) fn response_headers(&mut self, status: StatusCode) {
        self.metadata.status = Some(status.as_u16());
        self.metadata.first_byte_ms = Some(self.elapsed_ms());
        let elapsed = self.elapsed_ms();
        if let Some(history) = &mut self.history {
            history.response_headers(status.as_u16(), elapsed);
        }
    }

    /// Local refusal/failure terminates at the decision, not delivery of its small
    /// static reply. HTTP error responses are identified by status, independently
    /// of transport errors; a completely read upstream 4xx/5xx has error=None.
    pub(super) fn reply(
        mut self,
        status: StatusCode,
        error: Option<ErrorCode>,
        text: &'static str,
    ) -> hyper::Response<super::Body> {
        self.metadata.status = Some(status.as_u16());
        self.local_reply = true;
        self.finish(error);
        super::reply(status, text)
    }

    pub(super) fn finish(&mut self, error: Option<ErrorCode>) {
        let report = self
            .usage
            .take()
            .and_then(|mut usage| usage.finish(error.is_none()))
            .unwrap_or_default();
        if let Some(history) = self.history.take() {
            use crate::history::Outcome;
            let outcome = match error {
                Some(ErrorCode::Cancelled) => Outcome::Cancelled,
                Some(ErrorCode::Timeout) => Outcome::Timeout,
                Some(ErrorCode::ResourceLimit) => Outcome::Rejected,
                Some(_) => Outcome::TransportError,
                None if self.local_reply => Outcome::Rejected,
                None if self.metadata.status.is_some_and(|s| s >= 400) => Outcome::HttpError,
                None => Outcome::Success,
            };
            history.finish(self.metadata.status, self.elapsed_ms(), outcome, report);
        }
        if let Some(log) = self.log.take() {
            self.metadata.duration_ms = Some(self.elapsed_ms());
            self.metadata.error = error;
            let level = if error.is_some() || self.metadata.status.is_some_and(|s| s >= 400) {
                LogLevel::Warn
            } else {
                LogLevel::Info
            };
            let _ = log.try_log(LogEvent::Access {
                level,
                metadata: self.metadata,
            });
        }
    }
}
impl Drop for AccessRecording {
    fn drop(&mut self) {
        self.finish(Some(ErrorCode::Cancelled));
    }
}
