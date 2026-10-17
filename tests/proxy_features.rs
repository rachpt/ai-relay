use ai_relay::{
    config::{Config, InjectionMode, SecretString},
    proxy::{self, ProxyHandle},
};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{sleep, timeout},
};
async fn listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}
fn config(addr: SocketAddr) -> Config {
    let mut c = Config::default();
    c.upstream.url = format!("http://{addr}/prefix");
    c.proxy.forward_enabled = true;
    c.proxy.connect_enabled = true;
    c.keys.injection_mode = InjectionMode::Always;
    c.keys.upstream_api_key = SecretString::new("synthetic-upstream-key");
    c.network.connect_timeout_secs = 1;
    c.network.response_header_timeout_secs = 1;
    c.network.response_body_idle_timeout_secs = 1;
    c.network.shutdown_grace_secs = 1;
    c
}
async fn start(c: Config) -> ProxyHandle {
    proxy::start_on_listener(c, listener().await).await.unwrap()
}
async fn head(s: &mut TcpStream) -> String {
    timeout(Duration::from_secs(4), async {
        let mut b = Vec::new();
        while !b.ends_with(b"\r\n\r\n") {
            let n = s.read_u8().await.unwrap();
            b.push(n);
        }
        String::from_utf8(b).unwrap()
    })
    .await
    .unwrap()
}
async fn send(addr: SocketAddr, raw: &str) -> TcpStream {
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(raw.as_bytes()).await.unwrap();
    s
}
async fn response(addr: SocketAddr, raw: &str) -> String {
    let mut s = send(addr, raw).await;
    let mut b = Vec::new();
    timeout(Duration::from_secs(4), s.read_to_end(&mut b))
        .await
        .unwrap()
        .unwrap();
    String::from_utf8(b).unwrap()
}
fn get(target: &str, extra: &str) -> String {
    format!("GET {target} HTTP/1.1\r\nHost: wrong.invalid\r\nConnection: close\r\n{extra}\r\n")
}
fn connect(target: &str, extra: &str) -> String {
    format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n{extra}\r\n")
}

#[tokio::test]
async fn forward_rewrite_and_trust_boundary_strip_proxy_credentials() {
    for rewrite in [true, false] {
        for trusted in [true, false] {
            let upstream = listener().await;
            let other = listener().await;
            let upstream_addr = upstream.local_addr().unwrap();
            let other_addr = other.local_addr().unwrap();
            let target = if trusted { upstream_addr } else { other_addr };
            let mut c = config(upstream_addr);
            c.proxy.rewrite_forward_to_upstream = rewrite;
            c.access.client_token_enabled = true;
            c.access.client_token = SecretString::new("synthetic-client-token");
            let p = start(c).await;
            let expected_key = rewrite || trusted;
            let selected = if rewrite || trusted { upstream } else { other };
            let mock = tokio::spawn(async move {
                let (mut s, _) = selected.accept().await.unwrap();
                let h = head(&mut s).await.to_ascii_lowercase();
                assert!(
                    h.starts_with(if rewrite {
                        "get /prefix/a%2fb?q=%2f "
                    } else {
                        "get /a%2fb?q=%2f "
                    }),
                    "{h}"
                );
                assert!(
                    !h.contains("proxy-authorization") && !h.contains("synthetic-client-token")
                );
                assert_eq!(h.contains("synthetic-upstream-key"), expected_key);
                if !expected_key {
                    assert!(h.contains("authorization: bearer client-key"));
                }
                s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                    .await
                    .unwrap();
            });
            let r = response(p.local_addr(), &get(&format!("http://{target}/a%2fb?q=%2f"), "Proxy-Authorization: Bearer synthetic-client-token\r\nAuthorization: Bearer client-key\r\n")).await;
            assert!(r.starts_with("HTTP/1.1 200"));
            mock.await.unwrap();
            p.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn absolute_https_and_disabled_modes_never_open_upstream() {
    let upstream = listener().await;
    let mut c = config(upstream.local_addr().unwrap());
    c.proxy.forward_enabled = false;
    c.proxy.connect_enabled = false;
    let p = start(c).await;
    for raw in [
        get("http://example.invalid/", ""),
        get("https://example.invalid/", ""),
        connect("example.invalid:443", "Connection: close\r\n"),
    ] {
        assert!(
            response(p.local_addr(), &raw)
                .await
                .starts_with("HTTP/1.1 501")
        );
    }
    assert!(
        timeout(Duration::from_millis(100), upstream.accept())
            .await
            .is_err()
    );
    p.shutdown().await.unwrap();
    let p = start(config(upstream.local_addr().unwrap())).await;
    assert!(
        response(p.local_addr(), &get("https://example.invalid/", ""))
            .await
            .starts_with("HTTP/1.1 501")
    );
    p.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_absolute_targets_never_reach_upstream_even_when_rewriting() {
    for rewrite in [true, false] {
        let upstream = listener().await;
        let mut c = config(upstream.local_addr().unwrap());
        c.proxy.rewrite_forward_to_upstream = rewrite;
        let p = start(c).await;
        for target in [
            "http://user@host.invalid/",
            "http://host.invalid:0/",
            "http://%6cocalhost/",
            "http://host.invalid:99999/",
        ] {
            let r = response(
                p.local_addr(),
                &get(target, "Proxy-Authorization: Bearer synthetic-token\r\n"),
            )
            .await;
            assert!(r.starts_with("HTTP/1.1 400"), "{target}: {r}");
        }
        assert!(
            timeout(Duration::from_millis(100), upstream.accept())
                .await
                .is_err()
        );
        p.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn connect_preserves_head_bytes_and_client_half_close_without_usage() {
    for host in ["127.0.0.1", "localhost"] {
        let upstream = listener().await;
        let port = upstream.local_addr().unwrap().port();
        let mut c = config(upstream.local_addr().unwrap());
        c.proxy.connect_allowed_ports = vec![port];
        c.network.connect_timeout_secs = 5;
        let p = start(c).await;
        let usage = p.usage_store();
        let metrics = p.metrics();
        let mock = tokio::spawn(async move {
            let (mut s, _) = upstream.accept().await.unwrap();
            let mut b = Vec::new();
            s.read_to_end(&mut b).await.unwrap();
            assert_eq!(b, b"early opaque bytes");
            s.write_all(b"after client FIN").await.unwrap();
            s.shutdown().await.unwrap();
        });
        let mut s = send(
            p.local_addr(),
            &(connect(
                &format!("{host}:{port}"),
                "Proxy-Authorization: Bearer never-forward\r\n",
            ) + "early opaque bytes"),
        )
        .await;
        assert!(head(&mut s).await.starts_with("HTTP/1.1 200"));
        s.shutdown().await.unwrap();
        let mut b = Vec::new();
        timeout(Duration::from_secs(3), s.read_to_end(&mut b))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(b, b"after client FIN");
        mock.await.unwrap();
        assert_eq!(usage.snapshot().aggregate.counters.reports, 0);
        p.shutdown().await.unwrap();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_requests, 1);
        assert_eq!(snapshot.errors, 0);
        assert_eq!(
            snapshot.upload_body_bytes,
            b"early opaque bytes".len() as u64
        );
        assert_eq!(
            snapshot.download_body_bytes,
            b"after client FIN".len() as u64
        );
        assert_eq!(snapshot.active_requests, 0);
        assert_eq!(snapshot.active_tunnels, 0);
        assert_eq!(snapshot.inbound_connections, 0);
    }
}

#[tokio::test]
async fn connect_auth_ports_failure_and_malformed_targets() {
    let upstream = listener().await;
    let addr = upstream.local_addr().unwrap();
    let mut c = config(addr);
    c.proxy.connect_allowed_ports = vec![addr.port()];
    c.access.client_token_enabled = true;
    c.access.client_token = SecretString::new("synthetic-client-token");
    let p = start(c).await;
    assert!(
        response(
            p.local_addr(),
            &connect(&addr.to_string(), "Connection: close\r\n")
        )
        .await
        .starts_with("HTTP/1.1 407")
    );
    for (target, status) in [
        ("example.invalid:443", "403"),
        ("example.invalid:0", "400"),
        ("user@host:443", "400"),
    ] {
        let r = response(
            p.local_addr(),
            &connect(
                target,
                "Connection: close\r\nProxy-Authorization: Bearer synthetic-client-token\r\n",
            ),
        )
        .await;
        assert!(r.starts_with(&format!("HTTP/1.1 {status}")), "{r}");
    }
    assert!(
        timeout(Duration::from_millis(100), upstream.accept())
            .await
            .is_err()
    );
    drop(upstream);
    let r = response(
        p.local_addr(),
        &connect(
            &addr.to_string(),
            "Connection: close\r\nProxy-Authorization: Bearer synthetic-client-token\r\n",
        ),
    )
    .await;
    // Windows may keep a refused connect pending until the configured deadline.
    assert!(
        r.starts_with("HTTP/1.1 502") || r.starts_with("HTTP/1.1 504"),
        "{r}"
    );
    p.shutdown().await.unwrap();
}

#[tokio::test]
async fn connect_idle_limit_cancellation_and_bounded_shutdown() {
    for mode in ["idle", "cancel", "shutdown"] {
        let upstream = listener().await;
        let addr = upstream.local_addr().unwrap();
        let mut c = config(addr);
        c.proxy.connect_allowed_ports = vec![addr.port()];
        c.network.max_active_requests = 1;
        if mode == "shutdown" {
            c.network.response_body_idle_timeout_secs = 0;
        }
        let p = start(c).await;
        let metrics = p.metrics();
        let mut client = send(p.local_addr(), &connect(&addr.to_string(), "")).await;
        let (mut remote, _) = upstream.accept().await.unwrap();
        assert!(head(&mut client).await.starts_with("HTTP/1.1 200"));
        assert_eq!(metrics.snapshot().active_requests, 0);
        assert_eq!(metrics.snapshot().active_tunnels, 1);
        assert_eq!(metrics.snapshot().inbound_connections, 1);
        assert!(
            response(p.local_addr(), &get("/", ""))
                .await
                .starts_with("HTTP/1.1 503")
        );
        if mode == "cancel" {
            drop(client);
        } else if mode == "shutdown" {
            timeout(Duration::from_secs(3), p.shutdown())
                .await
                .unwrap()
                .unwrap();
            let snapshot = metrics.snapshot();
            assert_eq!(snapshot.total_requests, 2);
            assert_eq!(snapshot.errors, 2); // Limit refusal + cancelled tunnel.
            assert_eq!(snapshot.active_requests, 0);
            assert_eq!(snapshot.active_tunnels, 0);
            assert_eq!(snapshot.inbound_connections, 0);
            let mut b = Vec::new();
            timeout(Duration::from_secs(2), remote.read_to_end(&mut b))
                .await
                .unwrap()
                .unwrap();
            continue;
        } else {
            let mut b = Vec::new();
            timeout(Duration::from_secs(3), client.read_to_end(&mut b))
                .await
                .unwrap()
                .unwrap();
        }
        let mut b = Vec::new();
        timeout(Duration::from_secs(3), remote.read_to_end(&mut b))
            .await
            .unwrap()
            .unwrap();
        p.shutdown().await.unwrap();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_requests, 2);
        assert_eq!(snapshot.errors, 2, "{mode}");
        assert_eq!(snapshot.active_requests, 0);
        assert_eq!(snapshot.active_tunnels, 0);
        assert_eq!(snapshot.inbound_connections, 0);
    }
}

#[tokio::test]
async fn cors_preflight_is_local_validated_and_can_bypass_token_only() {
    let upstream = listener().await;
    let mut c = config(upstream.local_addr().unwrap());
    c.cors.enabled = true;
    c.cors.allowed_origins = vec!["https://browser.invalid".into()];
    c.cors.allow_credentials = true;
    c.access.client_token_enabled = true;
    c.access.client_token = SecretString::new("synthetic-client-token");
    let p = start(c).await;
    let metrics = p.metrics();
    for (origin, method, requested, status) in [
        (
            "https://browser.invalid",
            "POST",
            "authorization, content-type, x-api-key",
            "204",
        ),
        ("https://evil.invalid", "POST", "authorization", "403"),
        ("https://browser.invalid", "CONNECT", "authorization", "403"),
        (
            "https://browser.invalid",
            "POST",
            "proxy-authorization",
            "204",
        ),
        ("https://browser.invalid", "POST", "connection", "403"),
    ] {
        let raw = format!(
            "OPTIONS /v1 HTTP/1.1\r\nHost: test\r\nConnection: close\r\nOrigin: {origin}\r\nAccess-Control-Request-Method: {method}\r\nAccess-Control-Request-Headers: {requested}\r\n\r\n"
        );
        let r = response(p.local_addr(), &raw).await.to_ascii_lowercase();
        assert!(r.starts_with(&format!("http/1.1 {status}")), "{r}");
        assert_eq!(
            r.contains("access-control-allow-origin:"),
            origin != "https://evil.invalid"
        );
        assert!(
            r.contains("access-control-request-method")
                && r.contains("access-control-request-headers")
        );
    }
    let r = response(
        p.local_addr(),
        &get("/", "Origin: https://browser.invalid\r\n"),
    )
    .await
    .to_ascii_lowercase();
    assert!(r.starts_with("http/1.1 407") && r.contains("access-control-allow-credentials: true"));
    assert!(
        timeout(Duration::from_millis(100), upstream.accept())
            .await
            .is_err()
    );
    p.shutdown().await.unwrap();
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.total_requests, 6);
    assert_eq!(snapshot.errors, 4); // Three refused preflights + auth; two 204s succeed.
    assert_eq!(snapshot.active_requests, 0);
    assert_eq!(snapshot.inbound_connections, 0);
    assert_eq!(snapshot.upload_body_bytes, 0);
    assert_eq!(snapshot.download_body_bytes, 0);
}

#[tokio::test]
async fn cors_merges_vary_and_overrides_upstream_policy_without_touching_body() {
    for allowed in [true, false] {
        let upstream = listener().await;
        let mut c = config(upstream.local_addr().unwrap());
        c.cors.enabled = true;
        c.cors.allowed_origins = vec!["https://browser.invalid".into()];
        let p = start(c).await;
        let mock = tokio::spawn(async move {
            let (mut s, _) = upstream.accept().await.unwrap();
            head(&mut s).await;
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nVary: Accept-Encoding\r\nVary: Origin\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Credentials: true\r\nProxy-Authorization: forbidden\r\n\r\nbody").await.unwrap();
        });
        let origin = if allowed {
            "https://browser.invalid"
        } else {
            "https://evil.invalid"
        };
        let r = response(p.local_addr(), &get("/", &format!("Origin: {origin}\r\n")))
            .await
            .to_ascii_lowercase();
        assert_eq!(r.contains("access-control-allow-origin:"), allowed);
        assert!(
            !r.contains("access-control-allow-credentials") && !r.contains("proxy-authorization")
        );
        assert!(
            r.contains("vary: accept-encoding")
                && r.matches("vary: origin").count() == 1
                && r.ends_with("body")
        );
        mock.await.unwrap();
        p.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn connect_upstream_half_close_keeps_upload_open_and_activity_resets_idle() {
    let upstream = listener().await;
    let addr = upstream.local_addr().unwrap();
    let mut c = config(addr);
    c.proxy.connect_allowed_ports = vec![addr.port()];
    let p = start(c).await;
    let metrics = p.metrics();
    let mock = tokio::spawn(async move {
        let (mut s, _) = upstream.accept().await.unwrap();
        s.write_all(b"hello").await.unwrap();
        s.shutdown().await.unwrap();
        let mut b = Vec::new();
        s.read_to_end(&mut b).await.unwrap();
        assert_eq!(b, vec![7; 4]);
    });
    let mut s = send(p.local_addr(), &connect(&addr.to_string(), "")).await;
    assert!(head(&mut s).await.starts_with("HTTP/1.1 200"));
    let mut b = Vec::new();
    s.read_to_end(&mut b).await.unwrap();
    assert_eq!(b, b"hello");
    assert_eq!(metrics.snapshot().active_tunnels, 1);
    assert_eq!(metrics.snapshot().active_requests, 0);
    for _ in 0..4 {
        sleep(Duration::from_millis(350)).await;
        s.write_all(&[7]).await.unwrap();
    }
    s.shutdown().await.unwrap();
    mock.await.unwrap();
    p.shutdown().await.unwrap();
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.total_requests, 1);
    assert_eq!(snapshot.errors, 0);
    assert_eq!(snapshot.upload_body_bytes, 4);
    assert_eq!(snapshot.download_body_bytes, 5);
    assert_eq!(snapshot.active_tunnels, 0);
    assert_eq!(snapshot.inbound_connections, 0);
}

#[tokio::test]
async fn connect_ipv6_streams_large_bidirectional_payload_with_backpressure() {
    let upstream = TcpListener::bind("[::1]:0").await.unwrap();
    let addr = upstream.local_addr().unwrap();
    let mut c = config(addr);
    c.proxy.connect_allowed_ports = vec![addr.port()];
    c.network.response_body_idle_timeout_secs = 5;
    let p = start(c).await;
    let metrics = p.metrics();
    let mock = tokio::spawn(async move {
        let (socket, _) = upstream.accept().await.unwrap();
        let (mut read, mut write) = socket.into_split();
        let upload = async {
            sleep(Duration::from_millis(100)).await;
            let mut b = Vec::new();
            read.read_to_end(&mut b).await.unwrap();
            assert_eq!(b, vec![23; 2 * 1024 * 1024]);
        };
        let download = async {
            write.write_all(&vec![42; 2 * 1024 * 1024]).await.unwrap();
            write.shutdown().await.unwrap();
        };
        tokio::join!(upload, download);
    });
    let mut s = send(p.local_addr(), &connect(&addr.to_string(), "")).await;
    assert!(head(&mut s).await.starts_with("HTTP/1.1 200"));
    let (mut read, mut write) = s.into_split();
    timeout(Duration::from_secs(10), async {
        tokio::join!(
            async {
                write.write_all(&vec![23; 2 * 1024 * 1024]).await.unwrap();
                write.shutdown().await.unwrap();
            },
            async {
                sleep(Duration::from_millis(100)).await;
                let mut b = Vec::new();
                read.read_to_end(&mut b).await.unwrap();
                assert_eq!(b, vec![42; 2 * 1024 * 1024]);
            }
        );
        mock.await.unwrap();
    })
    .await
    .unwrap();
    p.shutdown().await.unwrap();
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.total_requests, 1);
    assert_eq!(snapshot.errors, 0);
    assert_eq!(snapshot.upload_body_bytes, 2 * 1024 * 1024);
    assert_eq!(snapshot.download_body_bytes, 2 * 1024 * 1024);
    assert_eq!(snapshot.active_tunnels, 0);
    assert_eq!(snapshot.inbound_connections, 0);
}

#[tokio::test]
async fn cors_wildcard_no_credentials_and_invalid_wildcard_credentials_rejected() {
    let upstream = listener().await;
    let mut c = config(upstream.local_addr().unwrap());
    c.cors.enabled = true;
    c.cors.allowed_origins = vec!["*".into()];
    let p = start(c.clone()).await;
    let r = response(p.local_addr(), "OPTIONS / HTTP/1.1\r\nHost: test\r\nConnection: close\r\nOrigin: https://browser.invalid\r\nAccess-Control-Request-Method: POST\r\n\r\n").await.to_ascii_lowercase();
    assert!(r.starts_with("http/1.1 204") && r.contains("access-control-allow-origin: *"));
    assert!(!r.contains("access-control-allow-credentials"));
    p.shutdown().await.unwrap();
    c.cors.allow_credentials = true;
    assert!(matches!(
        proxy::start_on_listener(c, listener().await).await,
        Err(proxy::ProxyError::InvalidConfig(_))
    ));
}

#[test]
fn connect_terminal_access_logs_are_once_and_never_contain_opaque_payload() {
    use ai_relay::logging::*;
    use std::sync::Arc;
    let dir = std::env::temp_dir().join(format!("ai-relay-connect-log-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut logs = LogService::start(
        dir.clone(),
        LogOptions {
            file_enabled: false,
            ..Default::default()
        },
    )
    .unwrap();
    let log = logs.handle();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        for mode in ["finish", "idle", "shutdown"] {
            let upstream = listener().await;
            let addr = upstream.local_addr().unwrap();
            let mut c = config(addr);
            c.proxy.connect_allowed_ports = vec![addr.port()];
            if mode == "shutdown" {
                c.network.response_body_idle_timeout_secs = 0;
            }
            let reserved = listener().await;
            c.listen.port = reserved.local_addr().unwrap().port();
            drop(reserved);
            let p = proxy::start_with_observers(
                c,
                Arc::new(ai_relay::usage::UsageStore::new()),
                Some(log.clone()),
            )
            .await
            .unwrap();
            let mut s = send(
                p.local_addr(),
                &(connect(&addr.to_string(), "") + "opaque-secret"),
            )
            .await;
            let (mut remote, _) = upstream.accept().await.unwrap();
            assert!(head(&mut s).await.starts_with("HTTP/1.1 200"));
            let mut b = [0; 13];
            remote.read_exact(&mut b).await.unwrap();
            if mode == "finish" {
                s.shutdown().await.unwrap();
                remote.shutdown().await.unwrap();
            }
            if mode == "idle" {
                let mut b = Vec::new();
                timeout(Duration::from_secs(3), s.read_to_end(&mut b))
                    .await
                    .unwrap()
                    .unwrap();
            }
            p.shutdown().await.unwrap();
        }
        timeout(Duration::from_secs(3), async {
            loop {
                if log.snapshot(None).entries.len() == 3 {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let snapshot = log.snapshot(None);
        let mut ids = std::collections::HashSet::new();
        let mut errors = Vec::new();
        for entry in &snapshot.entries {
            if let LogEvent::Access { metadata, .. } = entry.event {
                assert_eq!(metadata.method, Some(Method::Connect));
                assert_eq!(metadata.status, Some(200));
                assert!(ids.insert(metadata.request_id));
                errors.push(metadata.error);
            } else {
                panic!("unexpected log");
            }
        }
        assert!(
            errors.contains(&None)
                && errors.contains(&Some(ErrorCode::Timeout))
                && errors.contains(&Some(ErrorCode::Cancelled))
        );
        assert!(!format!("{snapshot:?}").contains("opaque-secret"));
    });
    logs.shutdown(Duration::from_secs(3)).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
