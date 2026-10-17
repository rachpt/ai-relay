//! Streaming HTTP/1.1 reverse/forward proxy and opaque CONNECT. No retries or redirects.
//! Requires a Tokio runtime; dropping the handle requests bounded shutdown.
mod access;
mod body;
mod cors;
mod headers;
mod history;
mod metrics;
mod request_model;
mod tunnel;

pub use metrics::{ProxyMetrics, ProxyMetricsSnapshot};

use std::{convert::Infallible, fmt, net::SocketAddr, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::UnsyncBoxBody};
use hyper::{
    Method, Request, Response, StatusCode, Uri, body::Incoming, server::conn::http1,
    service::service_fn,
};
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::{
    client::legacy::{
        Client,
        connect::{HttpConnector, capture_connection},
    },
    rt::{TokioExecutor, TokioIo, TokioTimer},
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc, watch},
    task::{JoinHandle, JoinSet},
    time::{Instant, timeout},
};

use crate::{
    config::{Config, ValidationError},
    history::HistoryHandle,
    logging::{ErrorCode, LogHandle},
    usage::UsageStore,
};
use access::AccessRecording;
use body::{RequestBody, ResponseBody};

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type Body = UnsyncBoxBody<Bytes, BoxError>;
type UpstreamClient = Client<hyper_rustls::HttpsConnector<HttpConnector>, Body>;

/// Deliberately contains no underlying error chain, URL or credentials.
#[derive(Debug)]
pub enum ProxyError {
    InvalidConfig(ValidationError),
    UnsupportedFeature,
    InvalidUpstream,
    InvalidCredential,
    TlsRoots,
    Bind,
    Listener,
    Task,
}
impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidConfig(_) => "invalid proxy configuration",
            Self::UnsupportedFeature => "unsupported proxy feature",
            Self::InvalidUpstream => "invalid upstream target",
            Self::InvalidCredential => "credential cannot be represented as an HTTP header",
            Self::TlsRoots => "cannot load system TLS trust roots",
            Self::Bind => "cannot bind proxy listener",
            Self::Listener => "proxy listener failed",
            Self::Task => "proxy task failed",
        })
    }
}
impl std::error::Error for ProxyError {}

pub struct ProxyHandle {
    addr: SocketAddr,
    usage: Arc<UsageStore>,
    metrics: Arc<ProxyMetrics>,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<(), ProxyError>>>,
}
impl ProxyHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Passive observations of upstream attempts only, not billing statistics.
    pub fn usage_store(&self) -> Arc<UsageStore> {
        self.usage.clone()
    }

    /// Nonblocking overview counters. Keep this Arc after shutdown, or pass it
    /// to `start_with_metrics` to retain counters across listener restarts.
    pub fn metrics(&self) -> Arc<ProxyMetrics> {
        self.metrics.clone()
    }

    /// Stops accepting immediately, drains connections for the configured grace
    /// period, then aborts and joins the remaining connection tasks.
    pub async fn shutdown(mut self) -> Result<(), ProxyError> {
        self.stop.send_replace(true);
        match self.task.take() {
            Some(task) => task.await.map_err(|_| ProxyError::Task)?,
            None => Ok(()),
        }
    }
}
impl Drop for ProxyHandle {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

pub async fn start(config: Config) -> Result<ProxyHandle, ProxyError> {
    start_with_usage(config, Arc::new(UsageStore::new())).await
}

/// Reuses a session store across proxy restarts; never resets existing reports.
pub async fn start_with_usage(
    config: Config,
    usage: Arc<UsageStore>,
) -> Result<ProxyHandle, ProxyError> {
    start_with_observers(config, usage, None).await
}

/// Reuses session usage and an optional nonblocking access-log backend.
/// Health requests (`/__proxy` and `/__proxy/health`) are not logged.
/// The controller owns backend replacement across configuration changes.
pub async fn start_with_observers(
    config: Config,
    usage: Arc<UsageStore>,
    log: Option<LogHandle>,
) -> Result<ProxyHandle, ProxyError> {
    start_with_telemetry(config, usage, log, None).await
}

/// History service lifecycle belongs to the controller; request paths only HMAC
/// bounded metadata and try-record. Health is excluded from request/log/history
/// observations; its admitted TCP socket still contributes to connection gauges.
pub async fn start_with_telemetry(
    config: Config,
    usage: Arc<UsageStore>,
    log: Option<LogHandle>,
    history: Option<HistoryHandle>,
) -> Result<ProxyHandle, ProxyError> {
    start_with_metrics(config, usage, log, history, Arc::new(ProxyMetrics::new())).await
}

/// Reuses overview metrics as well as usage/log/history. Existing start APIs
/// remain compatible and create a fresh metrics session. See the snapshot type
/// for health exclusions, byte boundaries, errors and session uptime semantics.
pub async fn start_with_metrics(
    config: Config,
    usage: Arc<UsageStore>,
    log: Option<LogHandle>,
    history: Option<HistoryHandle>,
    metrics: Arc<ProxyMetrics>,
) -> Result<ProxyHandle, ProxyError> {
    let state = State::new(config, usage, log, history, metrics).await?;
    let listener = TcpListener::bind((state.config.listen.address, state.config.listen.port))
        .await
        .map_err(|_| ProxyError::Bind)?;
    launch(state, listener)
}

/// Uses the supplied listener (including an OS-assigned port) instead of listen
/// address/port. Other configuration validation remains identical to `start`.
pub async fn start_on_listener(
    config: Config,
    listener: TcpListener,
) -> Result<ProxyHandle, ProxyError> {
    start_on_listener_with_usage(config, listener, Arc::new(UsageStore::new())).await
}

/// Listener-injected variant of `start_with_usage`.
pub async fn start_on_listener_with_usage(
    config: Config,
    listener: TcpListener,
    usage: Arc<UsageStore>,
) -> Result<ProxyHandle, ProxyError> {
    launch(
        State::new(config, usage, None, None, Arc::new(ProxyMetrics::new())).await?,
        listener,
    )
}

fn launch(state: State, listener: TcpListener) -> Result<ProxyHandle, ProxyError> {
    let addr = listener.local_addr().map_err(|_| ProxyError::Listener)?;
    let (stop, rx) = watch::channel(false);
    let usage = state.usage.clone();
    let metrics = state.metrics.clone();
    let task = tokio::spawn(serve(
        Arc::new(state),
        listener,
        rx,
        Duration::from_secs(10),
    ));
    Ok(ProxyHandle {
        addr,
        usage,
        metrics,
        stop,
        task: Some(task),
    })
}

struct State {
    config: Config,
    log: Option<LogHandle>,
    history: Option<HistoryHandle>,
    usage: Arc<UsageStore>,
    metrics: Arc<ProxyMetrics>,
    origin: String,
    prefix: String,
    host: hyper::header::HeaderValue,
    auth: hyper::header::HeaderValue,
    key: hyper::header::HeaderValue,
    client: UpstreamClient,
    slots: Arc<Semaphore>,
    started: Instant,
}
impl State {
    async fn new(
        config: Config,
        usage: Arc<UsageStore>,
        log: Option<LogHandle>,
        history: Option<HistoryHandle>,
        metrics: Arc<ProxyMetrics>,
    ) -> Result<Self, ProxyError> {
        config
            .validate_for_start()
            .map_err(ProxyError::InvalidConfig)?;

        let url = url::Url::parse(&config.upstream.url).map_err(|_| ProxyError::InvalidUpstream)?;
        let origin = url.origin().ascii_serialization();
        let origin_uri: Uri = origin.parse().map_err(|_| ProxyError::InvalidUpstream)?;
        let host = origin_uri
            .authority()
            .ok_or(ProxyError::InvalidUpstream)?
            .as_str()
            .parse()
            .map_err(|_| ProxyError::InvalidUpstream)?;
        // Parse the raw configured path as an HTTP URI, not Url::join: no dot
        // segment normalization or decoding of escaped delimiters is allowed.
        let raw_path = config
            .upstream
            .url
            .split_once("://")
            .and_then(|(_, rest)| rest.find('/').map(|i| &rest[i..]))
            .unwrap_or("");
        let prefix = raw_path.trim_end_matches('/').to_owned();
        let _: Uri = format!("{origin}{prefix}/")
            .parse()
            .map_err(|_| ProxyError::InvalidUpstream)?;
        let mut auth: hyper::header::HeaderValue =
            format!("Bearer {}", config.keys.upstream_api_key.expose_secret())
                .parse()
                .map_err(|_| ProxyError::InvalidCredential)?;
        let mut key: hyper::header::HeaderValue = config
            .keys
            .upstream_api_key
            .expose_secret()
            .parse()
            .map_err(|_| ProxyError::InvalidCredential)?;
        auth.set_sensitive(true);
        key.set_sensitive(true);
        // Native root loading can touch disk and must not block a runtime/UI thread.
        let tls = tokio::task::spawn_blocking(|| HttpsConnectorBuilder::new().with_native_roots())
            .await
            .map_err(|_| ProxyError::TlsRoots)?
            .map_err(|_| ProxyError::TlsRoots)?;
        let mut http = HttpConnector::new();
        http.enforce_http(false);
        http.set_connect_timeout(Some(secs(config.network.connect_timeout_secs)));
        let connector = tls.https_or_http().enable_http1().wrap_connector(http);
        let mut builder = Client::builder(TokioExecutor::new());
        builder
            .retry_canceled_requests(false)
            .pool_max_idle_per_host(config.network.max_active_requests as usize)
            .pool_idle_timeout(Duration::from_secs(30));
        Ok(Self {
            slots: Arc::new(Semaphore::new(config.network.max_active_requests as usize)),
            config,
            log,
            history,
            usage,
            metrics,
            origin,
            prefix,
            host,
            auth,
            key,
            client: builder.build(connector),
            started: Instant::now(),
        })
    }
}
fn secs(value: u32) -> Duration {
    Duration::from_secs(u64::from(value))
}

fn inbound_http1(header_read_timeout: Duration) -> http1::Builder {
    let mut builder = http1::Builder::new();
    // Only bounds idle/header reads, not uploads or long-lived SSE responses.
    builder
        .max_buf_size(32 * 1024)
        .timer(TokioTimer::new())
        .header_read_timeout(header_read_timeout);
    builder
}

async fn serve(
    state: Arc<State>,
    listener: TcpListener,
    mut stop: watch::Receiver<bool>,
    header_read_timeout: Duration,
) -> Result<(), ProxyError> {
    let mut tasks = JoinSet::new();
    // Bound idle/slow inbound connections as well as active proxy requests.
    let connections = Arc::new(Semaphore::new(
        state.config.network.max_active_requests as usize + 16,
    ));
    let mut failure = false;
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            accepted = listener.accept() => {
                let (stream, peer) = match accepted { Ok(pair) => pair, Err(_) => { failure = true; break; } };
                let Ok(permit) = connections.clone().try_acquire_owned() else { drop(stream); continue; };
                let state = state.clone();
                let connection_guard = state.metrics.connection();
                let mut stop = stop.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    let _connection_guard = connection_guard;
                    let (tunnel_tx, mut tunnel_rx) = mpsc::channel(1);
                    let service = service_fn(move |request| {
                        let state = state.clone();
                        let tunnel_tx = tunnel_tx.clone();
                        async move { Ok::<_, Infallible>(forward(state, request, peer, tunnel_tx).await) }
                    });
                    let connection = inbound_http1(header_read_timeout)
                        .serve_connection(TokioIo::new(stream), service).with_upgrades();
                    tokio::pin!(connection);
                    tokio::select! {
                        _ = &mut connection => {},
                        _ = stop.changed() => {
                            connection.as_mut().graceful_shutdown();
                            let _ = connection.await;
                        }
                    }
                    // A successful CONNECT is transferred through a bounded channel,
                    // never spawned. The outer JoinSet owns it until EOF or abort.
                    if let Ok(tunnel) = tunnel_rx.try_recv() {
                        tunnel.run().await;
                    }
                });
            }
        }
    }
    drop(listener);
    if timeout(secs(state.config.network.shutdown_grace_secs), async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
    if failure {
        Err(ProxyError::Listener)
    } else {
        Ok(())
    }
}

async fn forward(
    state: Arc<State>,
    request: Request<Incoming>,
    peer: SocketAddr,
    tunnel_tx: mpsc::Sender<tunnel::Tunnel>,
) -> Response<Body> {
    let cors = cors::Cors::new(&state.config.cors, request.headers());
    let mut response = dispatch(state, request, peer, tunnel_tx, &cors).await;
    cors.apply(&mut response);
    response
}

async fn dispatch(
    state: Arc<State>,
    mut request: Request<Incoming>,
    peer: SocketAddr,
    tunnel_tx: mpsc::Sender<tunnel::Tunnel>,
    cors: &cors::Cors,
) -> Response<Body> {
    // Classify the original path before target rewriting; never retain query or
    // arbitrary path/method strings. Health is excluded even when rejected.
    let health = request.method() != Method::CONNECT
        && request.uri().scheme().is_none()
        && request.uri().authority().is_none()
        && matches!(request.uri().path(), "/__proxy/health" | "/__proxy");
    let mut access = AccessRecording::new(
        if health { None } else { state.log.clone() },
        peer,
        request.method(),
        request.uri().path(),
    );
    if !health {
        access.set_metrics(state.metrics.request());
    }
    if !health && let Some(history) = &state.history {
        access.set_history(history::HistoryRecording::new(
            history.clone(),
            peer,
            &request,
        ));
    }
    if !state.config.access.allows_ip(peer.ip()) {
        return access.reply(StatusCode::FORBIDDEN, None, "source denied");
    }
    if (request.method() == Method::CONNECT && !state.config.proxy.connect_enabled)
        || (request.method() != Method::CONNECT
            && request.uri().scheme().is_some()
            && (!state.config.proxy.forward_enabled || request.uri().scheme_str() != Some("http")))
        || headers::is_upgrade(request.headers())
    {
        return access.reply(
            StatusCode::NOT_IMPLEMENTED,
            None,
            "unsupported proxy request",
        );
    }
    if health {
        if !is_loopback(peer.ip()) {
            return reply(StatusCode::FORBIDDEN, "health is loopback only");
        }
        let mut response = reply(
            StatusCode::OK,
            format!(
                "{{\"ok\":true,\"uptime_secs\":{}}}",
                state.started.elapsed().as_secs()
            ),
        );
        response.headers_mut().insert(
            hyper::header::CONTENT_TYPE,
            hyper::header::HeaderValue::from_static("application/json"),
        );
        return response;
    }
    if let Some(response) = cors.preflight(request.method(), request.headers()) {
        let _ = access.reply(response.status(), None, "");
        return response;
    }
    if state.config.access.client_token_enabled
        && !headers::authorized(
            request.headers(),
            state.config.access.client_token.expose_secret(),
        )
    {
        let mut response = access.reply(
            StatusCode::PROXY_AUTHENTICATION_REQUIRED,
            None,
            "proxy authentication required",
        );
        response.headers_mut().insert(
            hyper::header::PROXY_AUTHENTICATE,
            hyper::header::HeaderValue::from_static("Bearer realm=\"AI Relay\""),
        );
        return response;
    }
    let Ok(permit) = state.slots.clone().try_acquire_owned() else {
        return access.reply(
            StatusCode::SERVICE_UNAVAILABLE,
            Some(ErrorCode::ResourceLimit),
            "active request limit reached",
        );
    };
    if request.method() == Method::CONNECT {
        return match tunnel::prepare(&state, &mut request, permit, access).await {
            Ok(tunnel) => match tunnel_tx.try_send(tunnel) {
                Ok(()) => tunnel::established(),
                Err(_) => reply(StatusCode::SERVICE_UNAVAILABLE, "CONNECT unavailable"),
            },
            Err(response) => *response,
        };
    }
    let absolute = request.uri().scheme().is_some();
    if request.uri().authority().is_some() && !absolute {
        return access.reply(StatusCode::BAD_REQUEST, None, "invalid request target");
    }
    if absolute && !valid_absolute_target(request.uri()) {
        return access.reply(StatusCode::BAD_REQUEST, None, "invalid request target");
    }
    let path = request.uri().path_and_query().map_or("/", |pq| pq.as_str());
    let direct = absolute && !state.config.proxy.rewrite_forward_to_upstream;
    let (uri, host, trusted) = if direct {
        let uri = request.uri().clone();
        let Some(authority) = uri.authority() else {
            return access.reply(StatusCode::BAD_REQUEST, None, "invalid request target");
        };

        let Ok(host) = authority.as_str().parse() else {
            return access.reply(StatusCode::BAD_REQUEST, None, "invalid request target");
        };
        // Compare the authority actually consumed by Hyper, not URL's host
        // normalization (e.g. percent-encoded or alternative IPv4 spellings).
        let trusted_origin: Uri = match state.origin.parse() {
            Ok(origin) => origin,
            Err(_) => return access.reply(StatusCode::BAD_REQUEST, None, "invalid request target"),
        };
        let trusted = uri.scheme_str() == trusted_origin.scheme_str()
            && uri
                .host()
                .zip(trusted_origin.host())
                .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
            && uri.port_u16().unwrap_or(80) == trusted_origin.port_u16().unwrap_or(80);
        (uri, host, trusted)
    } else {
        let Ok(uri) = target_uri(&state.origin, &state.prefix, path) else {
            return access.reply(StatusCode::BAD_REQUEST, None, "invalid request target");
        };
        (uri, state.host.clone(), true)
    };
    *request.uri_mut() = uri;
    // Observe original content headers: Connection filtering must not turn an
    // unsupported/ambiguous encoding into apparently unencoded JSON.
    let model = request_model::ModelObserver::new(request.headers());
    let metadata = Arc::new(request_model::RequestMetadata::default());
    access.set_request_metadata(metadata.clone());
    headers::strip(request.headers_mut());
    request.headers_mut().insert(hyper::header::HOST, host);
    if trusted {
        headers::inject(
            request.headers_mut(),
            state.config.keys.injection_mode,
            &state.auth,
            &state.key,
        );
    }
    let mut request = request
        .map(|body| RequestBody::new(body, model, metadata, state.metrics.clone()).boxed_unsync());
    let mut captured = capture_connection(&mut request);
    // Scope: one observation per upstream attempt, including failures/cancellation
    // before headers. Local health/auth/validation/limit replies above do not count.
    access.upstream(request.uri(), request.headers(), state.usage.clone());
    let pending = state.client.request(request);
    tokio::pin!(pending);
    // Includes pool acquisition, DNS, TCP and TLS. Response-header timeout starts
    // only once the connection has been assigned, including on a pooled socket.
    let result = tokio::select! {
        biased;
        result = &mut pending => Some(result),
        _ = captured.wait_for_connection_metadata() => None,
        _ = tokio::time::sleep(secs(state.config.network.connect_timeout_secs)) => return access.reply(StatusCode::GATEWAY_TIMEOUT, Some(ErrorCode::Timeout), "upstream connection timeout"),
    };
    let result = match result {
        Some(result) => result,
        None => match timeout(
            secs(state.config.network.response_header_timeout_secs),
            pending,
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                return access.reply(
                    StatusCode::GATEWAY_TIMEOUT,
                    Some(ErrorCode::Timeout),
                    "upstream response header timeout",
                );
            }
        },
    };
    match result {
        Ok(response) if response.status() == StatusCode::SWITCHING_PROTOCOLS => {
            access.response_headers(response.status());
            access.reply(
                StatusCode::BAD_GATEWAY,
                Some(ErrorCode::Protocol),
                "unsupported upstream upgrade",
            )
        }
        Ok(response) => {
            access.response_headers(response.status());
            let (mut parts, body) = response.into_parts();
            access.usage_headers(&parts.headers);
            headers::strip(&mut parts.headers);
            if state.config.cors.enabled {
                for name in [
                    "access-control-allow-methods",
                    "access-control-allow-headers",
                    "access-control-max-age",
                    "access-control-allow-private-network",
                ] {
                    parts.headers.remove(name);
                }
            }
            Response::from_parts(
                parts,
                ResponseBody::new(
                    body,
                    permit,
                    state.config.network.response_body_idle_timeout_secs,
                    access,
                )
                .boxed_unsync(),
            )
        }
        Err(_) => access.reply(
            StatusCode::BAD_GATEWAY,
            Some(ErrorCode::Connection),
            "upstream request failed",
        ),
    }
}

fn valid_absolute_target(uri: &Uri) -> bool {
    let Some(authority) = uri.authority() else {
        return false;
    };
    let Ok(url) = url::Url::parse(&uri.to_string()) else {
        return false;
    };
    if authority.as_str().contains('@')
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port() == Some(0)
    {
        return false;
    }
    let host = authority.host();
    if host.starts_with('[') {
        host.strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .is_some_and(|h| h.parse::<std::net::Ipv6Addr>().is_ok())
    } else {
        !host.is_empty()
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    }
}

fn is_loopback(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map_or(ip.is_loopback(), |ip| ip.is_loopback()),
        ip => ip.is_loopback(),
    }
}

/// Prefix is omitted when the raw path already equals it or begins with it at
/// a slash boundary. Otherwise prepend it. Never decode, normalize or Url::join.
fn target_uri(
    origin: &str,
    prefix: &str,
    path_and_query: &str,
) -> Result<Uri, hyper::http::uri::InvalidUri> {
    let path = path_and_query.split('?').next().unwrap_or("/");
    let already_prefixed = path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'));
    format!(
        "{origin}{}{path_and_query}",
        if already_prefixed { "" } else { prefix }
    )
    .parse()
}

fn reply(status: StatusCode, text: impl Into<Bytes>) -> Response<Body> {
    let mut response = Response::new(
        Full::new(text.into())
            .map_err(|never| -> BoxError { match never {} })
            .boxed_unsync(),
    );
    *response.status_mut() = status;
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    struct TestSseBody<F>(F);

    impl<F> hyper::body::Body for TestSseBody<F>
    where
        F: std::future::Future<Output = Option<Result<hyper::body::Frame<Bytes>, Infallible>>>
            + Unpin,
    {
        type Data = Bytes;
        type Error = Infallible;

        fn poll_frame(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Result<hyper::body::Frame<Bytes>, Infallible>>> {
            std::pin::Pin::new(&mut self.get_mut().0).poll(cx)
        }
    }

    #[tokio::test]
    async fn idle_and_partial_headers_expire_and_release_connection_slots() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut config = Config::default();
        config.upstream.url = format!("http://{addr}/");
        config.network.max_active_requests = 1;
        config.access.client_token_enabled = true;
        config.access.client_token = crate::config::SecretString::new("test-client-token");
        let state = State::new(
            config,
            Arc::new(UsageStore::new()),
            None,
            None,
            Arc::new(ProxyMetrics::new()),
        )
        .await
        .unwrap();
        let (stop, rx) = watch::channel(false);
        let task = tokio::spawn(serve(Arc::new(state), listener, rx, Duration::from_secs(1)));
        let mut clients = Vec::new();
        for i in 0..17 {
            let mut client = TcpStream::connect(addr).await.unwrap();
            if i % 2 == 0 {
                client
                    .write_all(b"GET / HTTP/1.1\r\nHost: test\r\nX-Slow: ")
                    .await
                    .unwrap();
            }
            clients.push(client);
        }
        // All 17 connections must be admitted and still open before the deadline.
        for client in &mut clients {
            let mut byte = [0];
            assert!(
                timeout(Duration::from_millis(10), client.read(&mut byte))
                    .await
                    .is_err()
            );
        }
        let mut excess = TcpStream::connect(addr).await.unwrap();
        let mut byte = [0];
        assert_eq!(
            timeout(Duration::from_millis(300), excess.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        for mut client in clients {
            let mut response = Vec::new();
            timeout(Duration::from_secs(2), client.read_to_end(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert!(response.is_empty() || response.starts_with(b"HTTP/1.1 408"));
        }
        let mut healthy = TcpStream::connect(addr).await.unwrap();
        healthy
            .write_all(b"GET /__proxy/health HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        timeout(
            Duration::from_secs(2),
            healthy.read_to_string(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        stop.send_replace(true);
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn header_deadline_does_not_limit_sse_response_lifetime() {
        let (client, server) = tokio::io::duplex(4096);
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let finish_rx = Arc::new(tokio::sync::Mutex::new(Some(finish_rx)));
        let service = service_fn(move |_| {
            let finish_rx = finish_rx.clone();
            async move {
                let mut first = true;
                let finish_rx = finish_rx.lock().await.take().unwrap();
                let mut finish_rx = Box::pin(finish_rx);
                let frames = std::future::poll_fn(move |cx| {
                    if first {
                        first = false;
                        return std::task::Poll::Ready(Some(Ok::<_, Infallible>(
                            hyper::body::Frame::data(Bytes::from_static(b"data: first\n\n")),
                        )));
                    }
                    match std::future::Future::poll(finish_rx.as_mut(), cx) {
                        std::task::Poll::Pending => std::task::Poll::Pending,
                        std::task::Poll::Ready(_) => std::task::Poll::Ready(None),
                    }
                });
                // A custom body below polls this future for each frame.
                Ok::<_, Infallible>(Response::new(TestSseBody(frames)))
            }
        });
        let task = tokio::spawn(async move {
            inbound_http1(Duration::from_secs(1))
                .serve_connection(TokioIo::new(server), service)
                .await
                .unwrap();
        });
        let mut client = client;
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut received = Vec::new();
        while !received.ends_with(b"data: first\n\n\r\n") {
            let mut byte = [0];
            timeout(Duration::from_secs(2), client.read_exact(&mut byte))
                .await
                .unwrap()
                .unwrap();
            received.push(byte[0]);
        }
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!task.is_finished(), "SSE must outlive the header deadline");
        finish_tx.send(()).unwrap();
        let mut rest = Vec::new();
        timeout(Duration::from_secs(2), client.read_to_end(&mut rest))
            .await
            .unwrap()
            .unwrap();
        assert!(rest.ends_with(b"0\r\n\r\n"));
        task.await.unwrap();
    }
}
