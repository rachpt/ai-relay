use ai_relay::{
    config::{Config, SecretString},
    proxy::{self, ProxyMetrics, ProxyMetricsSnapshot},
    usage::UsageStore,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

async fn listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}
async fn config(addr: SocketAddr) -> Config {
    let reserved = listener().await;
    let mut cfg = Config::default();
    cfg.listen.address = reserved.local_addr().unwrap().ip();
    cfg.listen.port = reserved.local_addr().unwrap().port();
    cfg.upstream.url = format!("http://{addr}/");
    cfg.network.connect_timeout_secs = 1;
    cfg.network.response_header_timeout_secs = 10;
    cfg.network.response_body_idle_timeout_secs = 0;
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
async fn drain(socket: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(5), socket.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    bytes
}
async fn send(addr: SocketAddr, raw: &str) -> TcpStream {
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket.write_all(raw.as_bytes()).await.unwrap();
    socket
}
async fn wait(metrics: &ProxyMetrics, predicate: impl Fn(ProxyMetricsSnapshot) -> bool) {
    timeout(Duration::from_secs(5), async {
        while !predicate(metrics.snapshot()) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn metrics_survive_restart_and_exclude_health_but_count_local_refusals() {
    let upstream = listener().await;
    let mut cfg = config(upstream.local_addr().unwrap()).await;
    cfg.access.client_token_enabled = true;
    cfg.access.client_token = SecretString::new("synthetic-required-token");
    let metrics = Arc::new(ProxyMetrics::new());
    let usage = Arc::new(UsageStore::new());
    let proxy = proxy::start_with_metrics(cfg.clone(), usage.clone(), None, None, metrics.clone())
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&metrics, &proxy.metrics()));
    let idle = TcpStream::connect(proxy.local_addr()).await.unwrap();
    wait(&metrics, |s| s.inbound_connections == 1).await;
    assert_eq!(metrics.snapshot().total_requests, 0);
    assert_eq!(metrics.snapshot().active_requests, 0);
    drop(idle);
    for path in ["/__proxy", "/__proxy/health"] {
        let mut socket = send(proxy.local_addr(), &format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: 3\r\n\r\nabc")).await;
        assert!(drain(&mut socket).await.starts_with(b"HTTP/1.1 200"));
    }
    assert_eq!(metrics.snapshot().total_requests, 0);
    assert_eq!(metrics.snapshot().errors, 0);
    assert_eq!(metrics.snapshot().upload_body_bytes, 0);
    assert_eq!(metrics.snapshot().download_body_bytes, 0);
    for raw in [
        "POST /local HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: 3\r\n\r\nabc",
        "CONNECT denied.invalid:443 HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n",
    ] {
        let mut socket = send(proxy.local_addr(), raw).await;
        let reply = drain(&mut socket).await;
        assert!(reply.starts_with(b"HTTP/1.1 407") || reply.starts_with(b"HTTP/1.1 501"));
    }
    proxy.shutdown().await.unwrap();
    let first = metrics.snapshot();
    assert_eq!(first.total_requests, 2);
    assert_eq!(first.errors, 2);
    assert_eq!(first.active_requests, 0);
    assert_eq!(first.inbound_connections, 0);
    assert_eq!(first.upload_body_bytes, 0);
    assert_eq!(first.download_body_bytes, 0);
    assert_eq!(usage.snapshot().aggregate.counters.reports, 0);
    assert!(!first.saturated);
    assert!(first.uptime > Duration::ZERO);

    cfg.access.client_token_enabled = false;
    let proxy = proxy::start_with_metrics(cfg.clone(), usage.clone(), None, None, metrics.clone())
        .await
        .unwrap();
    let mock = tokio::spawn(async move {
        for status in ["200 OK", "503 Unavailable"] {
            let (mut socket, _) = upstream.accept().await.unwrap();
            headers(&mut socket).await;
            let mut upload = [0; 3];
            socket.read_exact(&mut upload).await.unwrap();
            assert_eq!(&upload, b"abc");
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Length: 5\r\nConnection: close\r\n\r\nreply"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    for _ in 0..2 {
        let mut socket = send(
            proxy.local_addr(),
            "POST / HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: 3\r\n\r\nabc",
        )
        .await;
        assert!(drain(&mut socket).await.ends_with(b"reply"));
    }
    mock.await.unwrap();
    proxy.shutdown().await.unwrap();
    let second = metrics.snapshot();
    assert_eq!(second.total_requests, 4);
    assert_eq!(second.errors, 3); // Each HTTP error counted once, independent of Drop.
    assert_eq!(second.upload_body_bytes, 6);
    assert_eq!(second.download_body_bytes, 10);
    assert_eq!(second.active_requests, 0);
    assert_eq!(second.active_tunnels, 0);
    assert_eq!(second.inbound_connections, 0);
    assert!(second.uptime >= first.uptime);

    // A legacy start has a fresh store and does not reset the retained Arc.
    let proxy = proxy::start_with_telemetry(cfg, usage, None, None)
        .await
        .unwrap();
    assert_eq!(proxy.metrics().snapshot().total_requests, 0);
    proxy.shutdown().await.unwrap();
    assert_eq!(metrics.snapshot().total_requests, 4);
}

#[tokio::test]
async fn head_204_and_content_length_zero_finish_before_keepalive_socket_closes() {
    for (method, response) in [
        ("HEAD", "HTTP/1.1 200 OK\r\nContent-Length: 999\r\n\r\n"),
        ("GET", "HTTP/1.1 204 No Content\r\n\r\n"),
        ("GET", "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"),
    ] {
        let upstream = listener().await;
        let proxy = proxy::start_with_telemetry(
            config(upstream.local_addr().unwrap()).await,
            Arc::new(UsageStore::new()),
            None,
            None,
        )
        .await
        .unwrap();
        let metrics = proxy.metrics();
        let (release, wait) = tokio::sync::oneshot::channel();
        let mock = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            headers(&mut socket).await;
            socket.write_all(response.as_bytes()).await.unwrap();
            wait.await.unwrap();
        });
        let mut socket = send(
            proxy.local_addr(),
            &format!("{method} / HTTP/1.1\r\nHost: test\r\n\r\n"),
        )
        .await;
        assert!(headers(&mut socket).await.starts_with("HTTP/1.1 2"));
        wait_for_empty(&metrics).await;
        assert_eq!(metrics.snapshot().total_requests, 1);
        assert_eq!(metrics.snapshot().errors, 0);
        assert_eq!(metrics.snapshot().inbound_connections, 1);
        assert_eq!(metrics.snapshot().upload_body_bytes, 0);
        assert_eq!(metrics.snapshot().download_body_bytes, 0);
        release.send(()).unwrap();
        mock.await.unwrap();
        drop(socket);
        proxy.shutdown().await.unwrap();
        assert_eq!(metrics.snapshot().errors, 0);
        assert_eq!(metrics.snapshot().inbound_connections, 0);
    }
}
async fn wait_for_empty(metrics: &ProxyMetrics) {
    wait(metrics, |s| s.active_requests == 0).await;
}

#[tokio::test]
async fn shutdown_cancels_pending_headers_and_streaming_body_once_and_releases_all_gauges() {
    for phase in ["headers", "body"] {
        let upstream = listener().await;
        let proxy = proxy::start_with_telemetry(
            config(upstream.local_addr().unwrap()).await,
            Arc::new(UsageStore::new()),
            None,
            None,
        )
        .await
        .unwrap();
        let metrics = proxy.metrics();
        let (ready, seen) = tokio::sync::oneshot::channel();
        let mock = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            headers(&mut socket).await;
            if phase == "body" {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 999999\r\n\r\nfirst")
                    .await
                    .unwrap();
            }
            ready.send(()).unwrap();
            drain(&mut socket).await;
        });
        let mut client = send(proxy.local_addr(), "GET / HTTP/1.1\r\nHost: test\r\n\r\n").await;
        seen.await.unwrap();
        if phase == "body" {
            headers(&mut client).await;
            let mut first = [0; 5];
            client.read_exact(&mut first).await.unwrap();
            assert_eq!(&first, b"first");
        }
        assert_eq!(metrics.snapshot().active_requests, 1);
        timeout(Duration::from_secs(4), proxy.shutdown())
            .await
            .unwrap()
            .unwrap();
        mock.await.unwrap();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_requests, 1);
        assert_eq!(snapshot.errors, 1);
        assert_eq!(snapshot.active_requests, 0);
        assert_eq!(snapshot.active_tunnels, 0);
        assert_eq!(snapshot.inbound_connections, 0);
        assert_eq!(
            snapshot.download_body_bytes,
            if phase == "body" { 5 } else { 0 }
        );
        drop(client);
    }
}
