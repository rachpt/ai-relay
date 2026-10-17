use super::{BoxError, access::AccessRecording};
use crate::logging::ErrorCode;
use crate::usage::{UsageObserver, UsageStore};
use bytes::Bytes;
use hyper::body::{Body, Frame, Incoming, SizeHint};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    sync::OwnedSemaphorePermit,
    time::{Instant, Sleep},
};

/// Moves from the request future to its response body without recording twice.
/// Parsing is bounded CPU work outside the store's short in-memory record lock;
/// neither observation nor recording performs I/O or waits for the GUI.
pub(super) struct UsageRecording {
    store: Arc<UsageStore>,
    observer: Option<UsageObserver>,
}

impl UsageRecording {
    pub(super) fn new(store: Arc<UsageStore>) -> Self {
        Self {
            store,
            observer: Some(UsageObserver::new("", None)),
        }
    }

    pub(super) fn response_headers(&mut self, headers: &hyper::HeaderMap) {
        // Multiple/invalid encoding values are unsupported, never interpreted as
        // identity. Inspect the original headers before hop-by-hop filtering.
        let mut encodings = headers.get_all(hyper::header::CONTENT_ENCODING).iter();
        let encoding = encodings.next().map(|value| value.to_str().unwrap_or(""));
        let encoding = if encodings.next().is_some() {
            Some("")
        } else {
            encoding
        };
        let mut types = headers.get_all(hyper::header::CONTENT_TYPE).iter();
        let content_type = types
            .next()
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let content_type = if types.next().is_some() {
            ""
        } else {
            content_type
        };
        self.observer = Some(UsageObserver::new(content_type, encoding));
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) {
        if let Some(observer) = self.observer.as_mut() {
            observer.feed(bytes);
        }
    }

    pub(super) fn finish(&mut self, complete: bool) -> Option<crate::usage::UsageReport> {
        let observer = self.observer.take()?;
        let report = observer.finish(complete);
        self.store.record(&report);
        Some(report)
    }
}

impl Drop for UsageRecording {
    fn drop(&mut self) {
        self.finish(false);
    }
}

/// No pump or queue: polling by the downstream drives the upstream body.
/// Dropping this body drops Incoming and cancels its upstream HTTP/1 exchange.
pub(super) struct ResponseBody {
    inner: Option<Incoming>,
    permit: Option<OwnedSemaphorePermit>,
    idle: Option<Pin<Box<Sleep>>>,
    duration: Duration,
    access: AccessRecording,
}
impl ResponseBody {
    pub(super) fn new(
        inner: Incoming,
        permit: OwnedSemaphorePermit,
        idle_secs: u32,
        mut access: AccessRecording,
    ) -> Self {
        let duration = Duration::from_secs(u64::from(idle_secs));
        let ended = inner.is_end_stream();
        // Hyper may never poll an already-ended body (e.g. HEAD or 204).
        if ended {
            access.finish(None);
        }
        Self {
            inner: if ended { None } else { Some(inner) },
            permit: if ended { None } else { Some(permit) },
            idle: if idle_secs == 0 || ended {
                None
            } else {
                Some(Box::pin(tokio::time::sleep(duration)))
            },
            duration,
            access,
        }
    }
    fn finish(&mut self, error: Option<ErrorCode>) {
        self.access.finish(error);
        self.inner = None;
        self.permit = None;
        self.idle = None;
    }
}
impl Body for ResponseBody {
    type Data = Bytes;
    type Error = BoxError;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
        let this = self.get_mut();
        let Some(inner) = this.inner.as_mut() else {
            return Poll::Ready(None);
        };
        match Pin::new(inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                // Trailers are hop-by-hop and were not advertised downstream.
                let is_data = frame.is_data();
                if let Some(data) = frame.data_ref() {
                    this.access.feed(data);
                }
                if let Some(timer) = this.idle.as_mut() {
                    timer.as_mut().reset(Instant::now() + this.duration);
                }
                if this.inner.as_ref().is_some_and(Body::is_end_stream) {
                    this.finish(None);
                }
                if is_data {
                    Poll::Ready(Some(Ok(frame)))
                } else {
                    this.finish(None);
                    Poll::Ready(None)
                }
            }
            Poll::Ready(Some(Err(_))) => {
                this.finish(Some(ErrorCode::Protocol));
                Poll::Ready(Some(Err("upstream body failed".into())))
            }
            Poll::Ready(None) => {
                this.finish(None);
                Poll::Ready(None)
            }
            Poll::Pending => {
                if this
                    .idle
                    .as_mut()
                    .is_some_and(|timer| std::future::Future::poll(timer.as_mut(), cx).is_ready())
                {
                    this.finish(Some(ErrorCode::Timeout));
                    Poll::Ready(Some(Err("upstream body idle timeout".into())))
                } else {
                    Poll::Pending
                }
            }
        }
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_none()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner
            .as_ref()
            .map_or_else(|| SizeHint::with_exact(0), Body::size_hint)
    }
}
