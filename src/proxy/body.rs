use super::{
    BoxError, ProxyMetrics,
    access::AccessRecording,
    request_model::{ModelObserver, RequestMetadata},
};
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

/// Zero preread, pump or queue: only upstream demand polls the inbound body.
/// Every successful frame (including trailers) is returned unchanged. A model
/// is published only at real EOS, never merely because a token/JSON was found.
pub(super) struct RequestBody<B> {
    inner: B,
    observer: Option<ModelObserver>,
    metadata: Arc<RequestMetadata>,
    metrics: Arc<ProxyMetrics>,
}
impl<B: Body<Data = Bytes> + Unpin> RequestBody<B> {
    pub(super) fn new(
        inner: B,
        observer: ModelObserver,
        metadata: Arc<RequestMetadata>,
        metrics: Arc<ProxyMetrics>,
    ) -> Self {
        let mut this = Self {
            inner,
            observer: Some(observer),
            metadata,
            metrics,
        };
        // Hyper need not poll an initially empty body.
        if this.inner.is_end_stream() {
            this.finish(true);
        }
        this
    }
    fn finish(&mut self, complete: bool) {
        if let Some(observer) = self.observer.take() {
            self.metadata.publish(observer.finish(complete));
        }
    }
}
impl<B: Body<Data = Bytes> + Unpin> Body for RequestBody<B> {
    type Data = Bytes;
    type Error = BoxError;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    this.metrics.upload(data.len());
                    if let Some(observer) = &mut this.observer {
                        observer.feed(data);
                    }
                }
                if this.inner.is_end_stream() {
                    this.finish(true);
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(None) => {
                this.finish(true);
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(_))) => {
                this.finish(false);
                Poll::Ready(Some(Err("request body failed".into())))
            }
            Poll::Pending => Poll::Pending,
        }
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
impl<B> Drop for RequestBody<B> {
    fn drop(&mut self) {
        // Drop is not upload completion, even if a full model token was seen.
        if self.observer.take().is_some() {
            self.metadata.publish(None);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use std::{
        collections::VecDeque,
        sync::atomic::{AtomicUsize, Ordering},
    };

    enum Step {
        Frame(Frame<Bytes>),
        Error,
        End,
        Pending,
    }
    struct Script {
        steps: VecDeque<Step>,
        polls: Arc<AtomicUsize>,
        ended: bool,
    }
    impl Body for Script {
        type Data = Bytes;
        type Error = BoxError;
        fn poll_frame(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
            let this = self.get_mut();
            this.polls.fetch_add(1, Ordering::Relaxed);
            match this.steps.pop_front().unwrap_or(Step::Pending) {
                Step::Frame(frame) => Poll::Ready(Some(Ok(frame))),
                Step::Error => {
                    this.ended = true;
                    Poll::Ready(Some(Err("test body failure".into())))
                }
                Step::End => {
                    this.ended = true;
                    Poll::Ready(None)
                }
                Step::Pending => Poll::Pending,
            }
        }
        fn is_end_stream(&self) -> bool {
            self.ended
        }
        fn size_hint(&self) -> SizeHint {
            SizeHint::with_exact(57)
        }
    }
    fn wrapped(
        steps: Vec<Step>,
    ) -> (
        RequestBody<Script>,
        Arc<AtomicUsize>,
        Arc<RequestMetadata>,
        Arc<ProxyMetrics>,
    ) {
        let polls = Arc::new(AtomicUsize::new(0));
        let metadata = Arc::new(RequestMetadata::default());
        let metrics = Arc::new(ProxyMetrics::new());
        let mut headers = hyper::HeaderMap::new();
        headers.insert(
            hyper::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        let body = RequestBody::new(
            Script {
                steps: steps.into(),
                polls: polls.clone(),
                ended: false,
            },
            ModelObserver::new(&headers),
            metadata.clone(),
            metrics.clone(),
        );
        (body, polls, metadata, metrics)
    }

    #[tokio::test]
    async fn demand_only_passes_original_data_and_trailer_frames_and_counts_only_data() {
        let data = Bytes::from_static(br#"{"model":"raw-model"}"#);
        let mut trailers = hyper::HeaderMap::new();
        trailers.insert("x-test", "original".parse().unwrap());
        let (mut body, polls, metadata, metrics) = wrapped(vec![
            Step::Frame(Frame::data(data.clone())),
            Step::Frame(Frame::trailers(trailers.clone())),
            Step::End,
        ]);
        assert_eq!(polls.load(Ordering::Relaxed), 0);
        assert_eq!(body.size_hint().exact(), Some(57));
        let received = body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert_eq!(received, data);
        assert_eq!(received.as_ptr(), data.as_ptr());
        assert_eq!(polls.load(Ordering::Relaxed), 1);
        let received = body
            .frame()
            .await
            .unwrap()
            .unwrap()
            .into_trailers()
            .unwrap();
        assert_eq!(received, trailers);
        assert!(body.frame().await.is_none());
        assert_eq!(metadata.finish().unwrap().as_str(), "raw-model");
        assert_eq!(metrics.snapshot().upload_body_bytes, data.len() as u64);
    }

    #[tokio::test]
    async fn drop_or_body_error_after_complete_json_is_not_upload_completion() {
        for error in [true, false] {
            let data = Bytes::from_static(br#"{"model":"not-complete-upload"}"#);
            let (mut body, _, metadata, _) = wrapped(vec![
                Step::Frame(Frame::data(data)),
                if error { Step::Error } else { Step::Pending },
            ]);
            body.frame().await.unwrap().unwrap();
            if error {
                assert!(body.frame().await.unwrap().is_err());
            }
            drop(body);
            assert_eq!(metadata.finish(), None);
        }
    }
}
