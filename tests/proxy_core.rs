use std::{net::SocketAddr, time::Duration};

use ai_relay::{
    config::{Config, InjectionMode, SecretString},
    proxy::{self, ProxyHandle},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::{sleep, timeout},
};

async fn listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}
fn config(addr: SocketAddr) -> Config {
    let mut config = Config::default();
    config.upstream.url = format!("http://{addr}/gateway/");
    config.network.shutdown_grace_secs = 1;
    config.network.connect_timeout_secs = 1;
    config.network.response_header_timeout_secs = 1;
    config.network.response_body_idle_timeout_secs = 1;
    config
}
#[tokio::test]
async fn tls_handshake_stall_uses_connection_timeout() {
    let upstream = listener().await;
    let mut cfg = config(upstream.local_addr().unwrap());
    cfg.upstream.url = format!("https://{}/", upstream.local_addr().unwrap());
    cfg.network.response_header_timeout_secs = 30;
    let proxy = start(cfg).await;
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        // Accept TCP but never answer the TLS ClientHello.
        closed(&mut socket).await;
    });
    let response = request(proxy.local_addr(), "/", "").await;
    assert!(response.starts_with("HTTP/1.1 504"), "{response}");
    assert!(response.contains("connection timeout"));
    mock.await.unwrap();
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn sent_post_is_not_retried_when_upstream_disconnects() {
    let upstream = listener().await;
    let proxy = start(config(upstream.local_addr().unwrap())).await;
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        assert!(headers(&mut socket).await.starts_with("POST "));
        let mut body = [0; 4];
        socket.read_exact(&mut body).await.unwrap();
        drop(socket);
        assert!(
            timeout(Duration::from_millis(300), upstream.accept())
                .await
                .is_err()
        );
    });
    let response = all(connect(
        proxy.local_addr(),
        "POST / HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: 4\r\n\r\ntest",
    )
    .await)
    .await;
    assert!(response.starts_with("HTTP/1.1 502"));
    mock.await.unwrap();
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_allows_inflight_response_to_finish_within_grace() {
    let upstream = listener().await;
    let proxy = start(config(upstream.local_addr().unwrap())).await;
    let (finish_tx, finish_rx) = oneshot::channel();
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        headers(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n")
            .await
            .unwrap();
        finish_rx.await.unwrap();
        socket.write_all(b"done").await.unwrap();
    });
    let mut client = connect(proxy.local_addr(), "GET / HTTP/1.1\r\nHost: test\r\n\r\n").await;
    assert!(headers(&mut client).await.starts_with("HTTP/1.1 200"));
    let shutdown = tokio::spawn(proxy.shutdown());
    sleep(Duration::from_millis(50)).await;
    finish_tx.send(()).unwrap();
    assert_eq!(all(client).await, "done");
    shutdown.await.unwrap().unwrap();
    mock.await.unwrap();
}

async fn start(config: Config) -> ProxyHandle {
    proxy::start_on_listener(config, listener().await)
        .await
        .unwrap()
}
async fn connect(addr: SocketAddr, request: &str) -> TcpStream {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    stream
}
async fn headers(stream: &mut TcpStream) -> String {
    timeout(Duration::from_secs(4), async {
        let mut data = Vec::new();
        while !data.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            if stream.read(&mut byte).await.unwrap() == 0 {
                break;
            }
            data.push(byte[0]);
        }
        String::from_utf8(data).unwrap()
    })
    .await
    .unwrap()
}
async fn all(mut stream: TcpStream) -> String {
    timeout(Duration::from_secs(4), async {
        let mut data = Vec::new();
        stream.read_to_end(&mut data).await.unwrap();
        String::from_utf8(data).unwrap()
    })
    .await
    .unwrap()
}
async fn request(addr: SocketAddr, path: &str, extra: &str) -> String {
    all(connect(
        addr,
        &format!("GET {path} HTTP/1.1\r\nHost: client.invalid\r\nConnection: close\r\n{extra}\r\n"),
    )
    .await)
    .await
}
async fn closed(stream: &mut TcpStream) {
    timeout(Duration::from_secs(3), async {
        let mut bytes = [0; 1024];
        loop {
            match stream.read(&mut bytes).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await
    .expect("upstream was not cancelled");
}

#[tokio::test]
async fn path_headers_body_and_redirect_are_preserved_without_following() {
    let upstream = listener().await;
    let mut cfg = config(upstream.local_addr().unwrap());
    cfg.keys.injection_mode = InjectionMode::Always;
    cfg.keys.upstream_api_key = SecretString::new("test-upstream-key");
    cfg.access.client_token_enabled = true;
    cfg.access.client_token = SecretString::new("test-client-token");
    let proxy = start(cfg).await;
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        let received = headers(&mut socket).await.to_ascii_lowercase();
        assert!(
            received.starts_with(
                "post /gateway/v1/a%2fb?q=%2f%26 HTTP/1.1"
                    .to_ascii_lowercase()
                    .as_str()
            ),
            "{received}"
        );
        assert!(received.contains("authorization: bearer test-upstream-key\r\n"));
        assert!(received.contains("x-api-key: test-upstream-key\r\n"));
        assert!(!received.contains("proxy-authorization"));
        assert!(!received.contains("x-remove"));
        assert!(!received.contains("client.invalid"));
        let mut body = [0; 4];
        socket.read_exact(&mut body).await.unwrap();
        assert_eq!(&body, b"test");
        socket.write_all(b"HTTP/1.1 307 Temporary Redirect\r\nLocation: http://untrusted.invalid/next\r\nConnection: close, X-Secret\r\nX-Secret: remove\r\nContent-Length: 4\r\n\r\nbody").await.unwrap();
        drop(socket);
        assert!(
            timeout(Duration::from_millis(250), upstream.accept())
                .await
                .is_err(),
            "redirect followed or request retried"
        );
    });
    let response = all(connect(proxy.local_addr(), "POST /v1/a%2Fb?q=%2F%26 HTTP/1.1\r\nHost: client.invalid\r\nProxy-Authorization: Bearer test-client-token\r\nAuthorization: Bearer client\r\nx-api-key: client\r\nConnection: close, X-Remove\r\nX-Remove: value\r\nContent-Length: 4\r\n\r\ntest").await).await;
    assert!(response.starts_with("HTTP/1.1 307"));
    assert!(response.contains("http://untrusted.invalid/next"));
    assert!(!response.to_ascii_lowercase().contains("x-secret"));
    assert!(response.ends_with("body"));
    mock.await.unwrap();
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn prefix_rules_preserve_raw_encoding_and_queries() {
    let upstream = listener().await;
    let proxy = start(config(upstream.local_addr().unwrap())).await;
    let cases = [
        ("/v1/a%2Fb?x=%252f", "/gateway/v1/a%2Fb?x=%252f"),
        ("/gateway/v1", "/gateway/v1"),
        ("/gateway?q=1", "/gateway?q=1"),
        ("/gatewayish", "/gateway/gatewayish"),
        ("/a/../%2e%2e/x", "/gateway/a/../%2e%2e/x"),
        ("//other.invalid/x", "/gateway//other.invalid/x"),
        ("/?x=/gateway/", "/gateway/?x=/gateway/"),
    ];
    let mock = tokio::spawn(async move {
        for (_, expected) in cases {
            let (mut socket, _) = upstream.accept().await.unwrap();
            assert!(
                headers(&mut socket)
                    .await
                    .starts_with(&format!("GET {expected} HTTP/1.1\r\n"))
            );
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        }
    });
    for (path, _) in cases {
        assert!(
            request(proxy.local_addr(), path, "")
                .await
                .starts_with("HTTP/1.1 200")
        );
    }
    mock.await.unwrap();
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn key_modes_match_legacy_openai_and_anthropic_rules() {
    for (mode, extra, expected_auth, expected_key) in [
        (
            InjectionMode::Off,
            "Authorization: Bearer client\r\nx-api-key: client\r\n",
            Some("Bearer client"),
            Some("client"),
        ),
        (
            InjectionMode::Missing,
            "Authorization: Bearer client\r\n",
            Some("Bearer client"),
            None,
        ),
        (
            InjectionMode::Missing,
            "x-api-key: client\r\n",
            None,
            Some("client"),
        ),
        (
            InjectionMode::Missing,
            "anthropic-version: test\r\n",
            Some("Bearer test-key"),
            Some("test-key"),
        ),
        (InjectionMode::Always, "", Some("Bearer test-key"), None),
        (
            InjectionMode::Always,
            "x-api-key: client\r\n",
            Some("Bearer test-key"),
            Some("test-key"),
        ),
    ] {
        let upstream = listener().await;
        let mut cfg = config(upstream.local_addr().unwrap());
        cfg.keys.injection_mode = mode;
        cfg.keys.upstream_api_key = SecretString::new("test-key");
        let proxy = start(cfg).await;
        let mock = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let received = headers(&mut socket).await.to_ascii_lowercase();
            for (name, value) in [
                ("authorization", expected_auth),
                ("x-api-key", expected_key),
            ] {
                match value {
                    Some(value) => assert!(
                        received.contains(&format!("{name}: {}\r\n", value.to_ascii_lowercase())),
                        "{received}"
                    ),
                    None => assert!(!received.contains(&format!("\r\n{name}:")), "{received}"),
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        assert!(
            request(proxy.local_addr(), "/", extra)
                .await
                .starts_with("HTTP/1.1 200")
        );
        mock.await.unwrap();
        proxy.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn sse_streams_incrementally_holds_limit_and_cancels_upstream() {
    let upstream = listener().await;
    let mut cfg = config(upstream.local_addr().unwrap());
    cfg.network.max_active_requests = 1;
    cfg.network.response_body_idle_timeout_secs = 0;
    let proxy = start(cfg).await;
    let (next_tx, next_rx) = oneshot::channel();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        headers(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n9\r\ndata: 1\n\n\r\n").await.unwrap();
        next_rx.await.unwrap();
        socket.write_all(b"9\r\ndata: 2\n\n\r\n").await.unwrap();
        closed(&mut socket).await;
        cancel_tx.send(()).unwrap();
        let (mut socket, _) = upstream.accept().await.unwrap();
        headers(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let mut client = connect(
        proxy.local_addr(),
        "GET /events HTTP/1.1\r\nHost: test\r\n\r\n",
    )
    .await;
    assert!(
        headers(&mut client)
            .await
            .to_ascii_lowercase()
            .contains("text/event-stream")
    );
    let mut first = [0; 14];
    timeout(Duration::from_secs(1), client.read_exact(&mut first))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&first, b"9\r\ndata: 1\n\n\r\n");
    assert!(
        request(proxy.local_addr(), "/busy", "")
            .await
            .starts_with("HTTP/1.1 503")
    );
    next_tx.send(()).unwrap();
    timeout(Duration::from_secs(1), client.read_exact(&mut first))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&first, b"9\r\ndata: 2\n\n\r\n");
    drop(client);
    timeout(Duration::from_secs(3), cancel_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(
        request(proxy.local_addr(), "/free", "")
            .await
            .starts_with("HTTP/1.1 200")
    );
    mock.await.unwrap();
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancellation_before_headers_reaches_upstream() {
    let upstream = listener().await;
    let mut cfg = config(upstream.local_addr().unwrap());
    cfg.network.response_header_timeout_secs = 30;
    let proxy = start(cfg).await;
    let (seen_tx, seen_rx) = oneshot::channel();
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        headers(&mut socket).await;
        seen_tx.send(()).unwrap();
        closed(&mut socket).await;
    });
    let client = connect(
        proxy.local_addr(),
        "GET /wait HTTP/1.1\r\nHost: test\r\n\r\n",
    )
    .await;
    seen_rx.await.unwrap();
    drop(client);
    mock.await.unwrap();
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn request_body_is_forwarded_before_upload_finishes() {
    let upstream = listener().await;
    let proxy = start(config(upstream.local_addr().unwrap())).await;
    let (seen_tx, seen_rx) = oneshot::channel();
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        headers(&mut socket).await;
        let mut bytes = [0; 3];
        socket.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"one");
        seen_tx.send(()).unwrap();
        socket.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"two");
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let mut client = connect(
        proxy.local_addr(),
        "POST /upload HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: 6\r\n\r\none",
    )
    .await;
    timeout(Duration::from_secs(1), seen_rx)
        .await
        .unwrap()
        .unwrap();
    client.write_all(b"two").await.unwrap();
    assert!(all(client).await.starts_with("HTTP/1.1 200"));
    mock.await.unwrap();
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn header_timeout_is_504_and_never_retries() {
    let upstream = listener().await;
    let proxy = start(config(upstream.local_addr().unwrap())).await;
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        headers(&mut socket).await;
        closed(&mut socket).await;
        assert!(
            timeout(Duration::from_millis(200), upstream.accept())
                .await
                .is_err()
        );
    });
    let response = request(proxy.local_addr(), "/private?secret=test", "").await;
    assert!(response.starts_with("HTTP/1.1 504"));
    assert!(!response.contains("private") && !response.contains("secret=test"));
    mock.await.unwrap();
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn body_idle_timeout_closes_without_second_response() {
    let upstream = listener().await;
    let proxy = start(config(upstream.local_addr().unwrap())).await;
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        headers(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\none\r\n")
            .await
            .unwrap();
        closed(&mut socket).await;
    });
    let response = request(proxy.local_addr(), "/", "").await;
    assert!(response.starts_with("HTTP/1.1 200"));
    assert_eq!(response.matches("HTTP/1.1").count(), 1);
    assert!(!response.contains("504"));
    mock.await.unwrap();
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_is_bounded_and_stops_accepting() {
    let upstream = listener().await;
    let mut cfg = config(upstream.local_addr().unwrap());
    cfg.network.response_body_idle_timeout_secs = 0;
    let proxy = start(cfg).await;
    let addr = proxy.local_addr();
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        headers(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await
            .unwrap();
        closed(&mut socket).await;
    });
    let mut client = connect(addr, "GET /events HTTP/1.1\r\nHost: test\r\n\r\n").await;
    headers(&mut client).await;
    let shutdown = tokio::spawn(proxy.shutdown());
    sleep(Duration::from_millis(100)).await;
    assert!(TcpStream::connect(addr).await.is_err());
    timeout(Duration::from_secs(3), shutdown)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    mock.await.unwrap();
}

#[tokio::test]
async fn authentication_health_and_unsupported_requests_do_not_touch_upstream() {
    let upstream = listener().await;
    let mut cfg = config(upstream.local_addr().unwrap());
    cfg.access.client_token_enabled = true;
    cfg.access.client_token = SecretString::new("test-client-token");
    cfg.keys.upstream_api_key = SecretString::new("test-upstream-key");
    // Loopback bypasses this whitelist, but must still authenticate.
    cfg.access.whitelist = vec!["192.0.2.0/24".parse().unwrap()];
    let proxy = start(cfg).await;
    let denied = request(proxy.local_addr(), "/", "X-Forwarded-For: 192.0.2.1\r\n").await;
    assert!(denied.starts_with("HTTP/1.1 407"));
    assert!(
        denied
            .to_ascii_lowercase()
            .contains("proxy-authenticate: bearer")
    );
    let health = request(proxy.local_addr(), "/__proxy/health", "").await;
    assert!(health.starts_with("HTTP/1.1 200"));
    assert!(health.contains("uptime_secs"));
    assert!(
        !health.contains("test-client-token")
            && !health.contains("test-upstream-key")
            && !health.contains("gateway")
    );
    for raw in [
        "CONNECT example.invalid:443 HTTP/1.1\r\nHost: example.invalid\r\nConnection: close\r\n\r\n",
        "GET http://example.invalid/ HTTP/1.1\r\nHost: example.invalid\r\nConnection: close\r\n\r\n",
        "GET / HTTP/1.1\r\nHost: test\r\nConnection: close, Upgrade\r\nUpgrade: websocket\r\n\r\n",
    ] {
        assert!(
            all(connect(proxy.local_addr(), raw).await)
                .await
                .starts_with("HTTP/1.1 501")
        );
    }
    assert!(
        timeout(Duration::from_millis(100), upstream.accept())
            .await
            .is_err()
    );
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn supported_features_start_and_invalid_secrets_fail_start() {
    for feature in 0..3 {
        let mut cfg = config("127.0.0.1:9".parse().unwrap());
        match feature {
            0 => cfg.cors.enabled = true,
            1 => cfg.proxy.forward_enabled = true,
            _ => cfg.proxy.connect_enabled = true,
        }
        proxy::start_on_listener(cfg, listener().await)
            .await
            .unwrap()
            .shutdown()
            .await
            .unwrap();
    }
    let mut cfg = config("127.0.0.1:9".parse().unwrap());
    cfg.keys.upstream_api_key = SecretString::new("test\r\nsecret");
    let result = proxy::start_on_listener(cfg, listener().await).await;
    let error = result.err().unwrap();
    assert!(!format!("{error:?} {error}").contains("test\r\nsecret"));
}

#[tokio::test]
async fn start_binds_configured_address_and_port_and_reports_conflict() {
    let reserved = listener().await;
    let mut cfg = config("127.0.0.1:9".parse().unwrap());
    cfg.listen.port = reserved.local_addr().unwrap().port();
    assert!(matches!(
        proxy::start(cfg.clone()).await,
        Err(proxy::ProxyError::Bind)
    ));
    drop(reserved);
    let proxy = proxy::start(cfg.clone()).await.unwrap();
    assert_eq!(proxy.local_addr().port(), cfg.listen.port);
    proxy.shutdown().await.unwrap();
}
