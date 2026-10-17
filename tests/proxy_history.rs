use ai_relay::{
    config::{Config, InjectionMode, SecretString},
    history::{self, HistoryService, Interface, Model, Outcome, Protocol, ProviderOrigin, Query},
    proxy,
    usage::UsageStore,
};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "relay-proxy-history-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn service(path: PathBuf) -> HistoryService {
    tokio::task::spawn_blocking(move || HistoryService::start(path).unwrap())
        .await
        .unwrap()
}
async fn query(service: HistoryService, q: Query) -> (HistoryService, history::QueryReport) {
    tokio::task::spawn_blocking(move || {
        service.barrier().unwrap();
        let report = service.query(q).unwrap();
        (service, report)
    })
    .await
    .unwrap()
}
async fn close(mut service: HistoryService) {
    tokio::task::spawn_blocking(move || service.shutdown().unwrap())
        .await
        .unwrap();
}
async fn listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}
async fn config(upstream: SocketAddr) -> Config {
    let bound = listener().await;
    let mut cfg = Config::default();
    cfg.listen.address = bound.local_addr().unwrap().ip();
    cfg.listen.port = bound.local_addr().unwrap().port();
    cfg.upstream.url = format!("http://{upstream}/prefix");
    cfg.network.connect_timeout_secs = 1;
    cfg.network.response_header_timeout_secs = 1;
    cfg.network.response_body_idle_timeout_secs = 1;
    cfg.network.shutdown_grace_secs = 1;
    cfg
}
async fn headers(socket: &mut TcpStream) -> String {
    timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(socket.read_u8().await.unwrap());
        }
        String::from_utf8(bytes).unwrap()
    })
    .await
    .unwrap()
}
async fn send(addr: SocketAddr, request: &str) -> TcpStream {
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket.write_all(request.as_bytes()).await.unwrap();
    socket
}
async fn request(addr: SocketAddr, request: &str) -> String {
    let mut socket = send(addr, request).await;
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(5), socket.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    String::from_utf8(bytes).unwrap()
}
fn identity(
    h: &history::HistoryHandle,
    source: &str,
    scheme: &str,
    secret: &str,
) -> history::CredentialId {
    h.credential(&format!("proxy-identity/v1\0{source}\0{scheme}\0{secret}"))
}
async fn admitted(h: &history::HistoryHandle, count: u64) {
    timeout(Duration::from_secs(5), async {
        while h.counters().queued < count {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn persisted_actual_response_and_injected_identity_combined_filters() {
    let temp = Temp::new();
    let service = service(temp.0.clone()).await;
    let handle = service.handle();
    let upstream = listener().await;
    let origin = format!("http://{}", upstream.local_addr().unwrap());
    let mut cfg = config(upstream.local_addr().unwrap()).await;
    cfg.keys.injection_mode = InjectionMode::Always;
    cfg.keys.upstream_api_key = SecretString::new("synthetic-upstream");
    let usage = Arc::new(UsageStore::new());
    let start = history::now_ms();
    let proxy = proxy::start_with_telemetry(cfg, usage.clone(), None, Some(handle.clone()))
        .await
        .unwrap();
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        let head = headers(&mut socket).await.to_ascii_lowercase();
        assert!(head.contains("/prefix/v1/chat/completions?private=query-marker"));
        assert!(head.contains("authorization: bearer synthetic-upstream"));
        assert!(!head.contains("proxy-authorization"));
        let body = r#"{"model":"actual-model","usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    });
    let response = request(proxy.local_addr(), "POST /v1/chat/completions?private=query-marker HTTP/1.1\r\nHost: test\r\nAuthorization: Bearer synthetic-client\r\nx-api-key: ignored-client\r\nProxy-Authorization: Bearer ignored-proxy\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
    assert!(response.contains("actual-model"));
    mock.await.unwrap();
    proxy.shutdown().await.unwrap();
    let (service, all) = query(service, Query::default()).await;
    assert_eq!(all.summary.requests, 1);
    assert_eq!(all.summary.tokens.total_tokens, Some(5));
    let r = &all.details[0];
    assert_eq!(r.requested_model, None);
    assert_eq!(r.report.model.as_deref(), Some("actual-model"));
    assert_eq!(r.outcome, Outcome::Success);
    assert!(r.latency_ms <= r.total_ms);
    assert_eq!(usage.snapshot().aggregate.counters.reports, 1);
    close(service).await;
    let service = self::service(temp.0.clone()).await;
    let q = Query {
        time_from: Some(start),
        time_to: Some(history::now_ms() + 1),
        ip: Some("127.0.0.1".parse().unwrap()),
        model: Some(Model::new("actual-model").unwrap()),
        client: Some(identity(
            &service.handle(),
            "authorization",
            "bearer",
            "synthetic-client",
        )),
        upstream: Some(identity(
            &service.handle(),
            "authorization",
            "bearer",
            "synthetic-upstream",
        )),
        provider: Some(ProviderOrigin::new(&origin).unwrap()),
        protocol: Some(Protocol::OpenAi),
        interface: Some(Interface::ChatCompletions),
        ..Query::default()
    };
    let (service, result) = query(service, q).await;
    assert_eq!(result.summary.requests, 1);
    close(service).await;
    tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(temp.0.join("history.sqlite3")).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for raw in [
            "synthetic-client",
            "synthetic-upstream",
            "ignored-client",
            "ignored-proxy",
            "query-marker",
            "/v1/chat/completions",
        ] {
            assert!(!text.contains(raw), "{raw}");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn local_rejections_health_and_identity_sources_remain_unknown() {
    let temp = Temp::new();
    let service = service(temp.0.clone()).await;
    let h = service.handle();
    let upstream = listener().await;
    let mut cfg = config(upstream.local_addr().unwrap()).await;
    cfg.access.client_token_enabled = true;
    cfg.access.client_token = SecretString::new("required-token");
    let usage = Arc::new(UsageStore::new());
    let proxy = proxy::start_with_telemetry(cfg, usage.clone(), None, Some(h.clone()))
        .await
        .unwrap();
    assert!(
        request(
            proxy.local_addr(),
            "GET /__proxy/health HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n"
        )
        .await
        .starts_with("HTTP/1.1 200")
    );
    for auth in [
        "Authorization: Bearer same",
        "Authorization: Basic same",
        "x-api-key: same",
        "Proxy-Authorization: Bearer same",
    ] {
        assert!(request(proxy.local_addr(), &format!("GET /arbitrary-secret?secret=query HTTP/1.1\r\nHost: test\r\n{auth}\r\nConnection: close\r\n\r\n")).await.starts_with("HTTP/1.1 407"));
    }
    // CONNECT authority must classify as CONNECT even when locally denied.
    assert!(
        request(
            proxy.local_addr(),
            "CONNECT secret.invalid:443 HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n"
        )
        .await
        .starts_with("HTTP/1.1 501")
    );
    proxy.shutdown().await.unwrap();
    let (service, result) = query(service, Query::default()).await;
    assert_eq!(result.summary.requests, 5);
    assert_eq!(result.summary.unknown, 5);
    assert_eq!(usage.snapshot().aggregate.counters.reports, 0);
    let ids: std::collections::HashSet<_> = result
        .details
        .iter()
        .filter_map(|r| r.client.as_ref().map(|c| c.as_str()))
        .collect();
    assert_eq!(ids.len(), 4);
    for r in &result.details {
        assert_eq!(r.upstream, None);
        assert_eq!(r.provider, None);
        assert_eq!(r.requested_model, None);
        assert_eq!(r.outcome, Outcome::Rejected);
    }
    assert!(
        result
            .details
            .iter()
            .any(|r| r.interface == Interface::Connect && r.protocol == Protocol::Http)
    );
    close(service).await;
}

#[tokio::test]
async fn forwarded_target_and_final_headers_not_configured_key() {
    for mode in [
        InjectionMode::Off,
        InjectionMode::Missing,
        InjectionMode::Always,
    ] {
        let temp = Temp::new();
        let service = service(temp.0.clone()).await;
        let h = service.handle();
        let configured = listener().await;
        let target = listener().await;
        let origin = format!("http://{}", target.local_addr().unwrap());
        let mut cfg = config(configured.local_addr().unwrap()).await;
        cfg.proxy.forward_enabled = true;
        cfg.proxy.rewrite_forward_to_upstream = false;
        cfg.keys.injection_mode = mode;
        cfg.keys.upstream_api_key = SecretString::new("never-injected");
        let proxy =
            proxy::start_with_telemetry(cfg, Arc::new(UsageStore::new()), None, Some(h.clone()))
                .await
                .unwrap();
        let mock = tokio::spawn(async move {
            let (mut socket, _) = target.accept().await.unwrap();
            let head = headers(&mut socket).await.to_ascii_lowercase();
            assert!(!head.contains("never-injected"));
            assert!(!head.contains("authorization:"));
            assert!(head.contains("x-api-key: actual-forward-key"));
            socket
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        request(proxy.local_addr(), &format!("GET {origin}/opaque?private=query HTTP/1.1\r\nHost: test\r\nAuthorization: Bearer stripped-client\r\nx-api-key: actual-forward-key\r\nConnection: close, authorization\r\n\r\n")).await;
        mock.await.unwrap();
        proxy.shutdown().await.unwrap();
        let (service, result) = query(
            service,
            Query {
                provider: Some(ProviderOrigin::new(&origin).unwrap()),
                client: Some(identity(&h, "authorization", "bearer", "stripped-client")),
                upstream: Some(identity(&h, "x-api-key", "key", "actual-forward-key")),
                protocol: Some(Protocol::Http),
                interface: Some(Interface::Forward),
                ..Query::default()
            },
        )
        .await;
        assert_eq!(result.summary.requests, 1);
        assert_eq!(result.details[0].status, Some(204));
        close(service).await;
    }
}

#[tokio::test]
async fn connect_records_once_without_fake_usage_or_origin() {
    let temp = Temp::new();
    let service = service(temp.0.clone()).await;
    let h = service.handle();
    let upstream = listener().await;
    let addr = upstream.local_addr().unwrap();
    let mut cfg = config(addr).await;
    cfg.proxy.connect_enabled = true;
    cfg.proxy.connect_allowed_ports = vec![addr.port()];
    let usage = Arc::new(UsageStore::new());
    let proxy = proxy::start_with_telemetry(cfg, usage.clone(), None, Some(h.clone()))
        .await
        .unwrap();
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        let mut bytes = [0; 4];
        socket.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    let mut client = send(
        proxy.local_addr(),
        &format!("CONNECT {addr} HTTP/1.1\r\nHost: test\r\n\r\n"),
    )
    .await;
    assert!(headers(&mut client).await.starts_with("HTTP/1.1 200"));
    client.write_all(b"ping").await.unwrap();
    let mut bytes = [0; 4];
    client.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"pong");
    drop(client);
    mock.await.unwrap();
    admitted(&h, 1).await;
    proxy.shutdown().await.unwrap();
    let (service, result) = query(service, Query::default()).await;
    assert_eq!(result.summary.requests, 1);
    assert_eq!(result.summary.unknown, 1);
    assert_eq!(result.details[0].interface, Interface::Connect);
    assert_eq!(result.details[0].provider, None); // Opaque TCP has no known HTTP(S) origin.
    assert_eq!(result.details[0].upstream, None);
    assert_eq!(usage.snapshot().aggregate.counters.reports, 0);
    close(service).await;
}

#[tokio::test]
async fn full_history_queue_drops_with_counters_without_blocking_proxy() {
    let temp = Temp::new();
    let path = temp.0.clone();
    let service = tokio::task::spawn_blocking(move || {
        HistoryService::start_with_options(
            path,
            history::Options {
                queue_capacity: 1,
                ..history::Options::default()
            },
        )
        .unwrap()
    })
    .await
    .unwrap();
    let h = service.handle();
    let path = temp.0.join("history.sqlite3");
    let lock = tokio::task::spawn_blocking(move || {
        let c = rusqlite::Connection::open(path).unwrap();
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        c
    })
    .await
    .unwrap();
    let upstream = listener().await;
    let cfg = config(upstream.local_addr().unwrap()).await;
    let proxy =
        proxy::start_with_telemetry(cfg, Arc::new(UsageStore::new()), None, Some(h.clone()))
            .await
            .unwrap();
    for _ in 0..16 {
        assert!(
            request(
                proxy.local_addr(),
                "CONNECT denied.invalid:443 HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n"
            )
            .await
            .starts_with("HTTP/1.1 501")
        );
    }
    assert!(h.counters().queue_full > 0);
    assert_eq!(h.counters().queued + h.counters().queue_full, 16);
    tokio::task::spawn_blocking(move || lock.execute_batch("ROLLBACK").unwrap())
        .await
        .unwrap();
    proxy.shutdown().await.unwrap();
    let (service, result) = query(service, Query::default()).await;
    assert_eq!(result.summary.requests, h.counters().queued);
    assert_eq!(h.counters().db_failures, 0);
    close(service).await;
}

#[tokio::test]
async fn before_and_after_headers_failure_timeout_cancel_finish_once() {
    for phase in ["before", "after"] {
        for mode in ["error", "timeout", "cancel"] {
            let temp = Temp::new();
            let service = service(temp.0.clone()).await;
            let h = service.handle();
            let upstream = listener().await;
            let cfg = config(upstream.local_addr().unwrap()).await;
            let usage = Arc::new(UsageStore::new());
            let proxy = proxy::start_with_telemetry(cfg, usage.clone(), None, Some(h.clone()))
                .await
                .unwrap();
            let (ready, wait) = tokio::sync::oneshot::channel();
            let (release, released) = tokio::sync::oneshot::channel();
            let mock = tokio::spawn(async move {
                let (mut socket, _) = upstream.accept().await.unwrap();
                headers(&mut socket).await;
                if phase == "after" {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 999999\r\n\r\ndata: {\"model\":\"prefix-model\",\"usage\":{\"prompt_tokens\":3}}\n\n").await.unwrap();
                }
                ready.send(()).unwrap();
                released.await.unwrap();
                if mode != "error" {
                    let mut bytes = Vec::new();
                    let _ = timeout(Duration::from_secs(5), socket.read_to_end(&mut bytes))
                        .await
                        .unwrap();
                }
            });
            let mut client = send(
                proxy.local_addr(),
                "GET /v1/responses HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n",
            )
            .await;
            wait.await.unwrap();
            if phase == "after" {
                headers(&mut client).await;
                let mut prefix = vec![
                        0;
                        b"data: {\"model\":\"prefix-model\",\"usage\":{\"prompt_tokens\":3}}\n\n"
                            .len()
                    ];
                client.read_exact(&mut prefix).await.unwrap();
            }
            release.send(()).unwrap();
            if mode == "cancel" {
                drop(client);
            } else {
                let mut bytes = Vec::new();
                let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut bytes))
                    .await
                    .unwrap();
            }
            admitted(&h, 1).await;
            mock.await.unwrap();
            proxy.shutdown().await.unwrap();
            let (service, result) = query(service, Query::default()).await;
            assert_eq!(result.summary.requests, 1, "{phase}/{mode}");
            assert_eq!(h.counters().queued, 1);
            assert_eq!(h.counters().db_failures, 0);
            assert_eq!(usage.snapshot().aggregate.counters.reports, 1);
            let r = &result.details[0];
            assert_eq!(
                r.outcome,
                match mode {
                    "cancel" => Outcome::Cancelled,
                    "timeout" => Outcome::Timeout,
                    _ => Outcome::TransportError,
                }
            );
            assert!(r.report.truncated);
            if phase == "after" {
                assert_eq!(r.report.usage.input_tokens, Some(3));
            } else {
                assert_eq!(r.report.model, None);
            }
            close(service).await;
        }
    }
}
