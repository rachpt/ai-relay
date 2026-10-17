use std::{net::SocketAddr, sync::Arc, time::Duration};

use ai_relay::{
    config::{Config, SecretString},
    proxy::{self, ProxyHandle},
    usage::{MAX_JSON_BYTES, MAX_SSE_BYTES, UsageStore},
};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, client::conn::http1};
use hyper_util::rt::TokioIo;
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
    let mut cfg = Config::default();
    cfg.upstream.url = format!("http://{addr}/");
    cfg.network.connect_timeout_secs = 1;
    cfg.network.response_header_timeout_secs = 1;
    cfg.network.response_body_idle_timeout_secs = 1;
    cfg.network.shutdown_grace_secs = 1;
    cfg
}

async fn start(cfg: Config, store: Arc<UsageStore>) -> ProxyHandle {
    proxy::start_on_listener_with_usage(cfg, listener().await, store)
        .await
        .unwrap()
}

async fn headers(socket: &mut TcpStream) -> String {
    timeout(Duration::from_secs(4), async {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            let byte = socket.read_u8().await.unwrap();
            bytes.push(byte);
        }
        String::from_utf8(bytes).unwrap()
    })
    .await
    .unwrap()
}

async fn raw(addr: SocketAddr) -> TcpStream {
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(b"GET / HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    socket
}

async fn closed(socket: &mut TcpStream) {
    timeout(Duration::from_secs(4), async {
        let mut bytes = [0; 1024];
        loop {
            match socket.read(&mut bytes).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await
    .expect("upstream exchange was not cancelled");
}

async fn get(addr: SocketAddr, path: &str) -> (u16, hyper::HeaderMap, Bytes) {
    timeout(Duration::from_secs(5), async {
        let socket = TcpStream::connect(addr).await.unwrap();
        let (mut sender, connection) = http1::handshake(TokioIo::new(socket)).await.unwrap();
        let task = tokio::spawn(connection);
        let request = Request::builder()
            .uri(path)
            .header("host", "test")
            .header("connection", "close")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let response = sender.send_request(request).await.unwrap();
        let (parts, body) = response.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        task.await.unwrap().unwrap();
        (parts.status.as_u16(), parts.headers, bytes)
    })
    .await
    .unwrap()
}

async fn reports(store: &UsageStore, expected: u64) {
    timeout(Duration::from_secs(4), async {
        while store.snapshot().aggregate.counters.reports < expected {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(store.snapshot().aggregate.counters.reports, expected);
}

// A real upstream and proxy, not a direct parser test. Chunk boundaries and
// trailers are transport framing; the decoded payload must match byte for byte.
async fn roundtrip(
    content_type: &str,
    encoding: Option<&str>,
    payload: Vec<u8>,
    chunked: bool,
) -> Arc<UsageStore> {
    let upstream = listener().await;
    let store = Arc::new(UsageStore::new());
    let proxy = start(config(upstream.local_addr().unwrap()), store.clone()).await;
    assert!(Arc::ptr_eq(&store, &proxy.usage_store()));
    let mut head = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n");
    if let Some(encoding) = encoding {
        head.push_str(&format!("Content-Encoding: {encoding}\r\n"));
    }
    if chunked {
        head.push_str("Transfer-Encoding: chunked\r\nTrailer: x-end\r\n\r\n");
    } else {
        head.push_str(&format!("Content-Length: {}\r\n\r\n", payload.len()));
    }
    let expected = payload.clone();
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        headers(&mut socket).await;
        socket.write_all(head.as_bytes()).await.unwrap();
        for chunk in payload.chunks(997) {
            if chunked {
                socket
                    .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                    .await
                    .unwrap();
            }
            socket.write_all(chunk).await.unwrap();
            if chunked {
                socket.write_all(b"\r\n").await.unwrap();
            }
        }
        if chunked {
            socket.write_all(b"0\r\nx-end: done\r\n\r\n").await.unwrap();
        }
    });
    let (status, head, received) = get(proxy.local_addr(), "/").await;
    assert_eq!(status, 200);
    assert_eq!(received.as_ref(), expected);
    assert_eq!(
        head.get("content-encoding").map(|v| v.to_str().unwrap()),
        encoding
    );
    mock.await.unwrap();
    reports(&store, 1).await;
    proxy.shutdown().await.unwrap();
    assert_eq!(store.snapshot().aggregate.counters.reports, 1);
    store
}

#[tokio::test]
async fn json_and_chunked_sse_preserve_payload_and_record_once() {
    let json = br#"{"model":"test-model","usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
    let store = roundtrip(
        "application/json; charset=utf-8",
        None,
        json.to_vec(),
        false,
    )
    .await;
    let s = store.snapshot();
    assert_eq!(s.aggregate.counters.complete, 1);
    assert_eq!(s.aggregate.counters.truncated, 0);
    assert_eq!(s.aggregate.observed_tokens.total_tokens, Some(5));
    assert_eq!(s.groups[0].model.as_deref(), Some("test-model"));
    let sse = format!(
        ": {}\r\n\r\ndata: {}\r\n\r\ndata: {}\n\ndata: [DONE]\n\n",
        "心跳".repeat(700),
        String::from_utf8_lossy(json),
        String::from_utf8_lossy(json)
    );
    let store = roundtrip(
        "text/event-stream",
        Some("identity"),
        sse.into_bytes(),
        true,
    )
    .await;
    let s = store.snapshot();
    assert_eq!(s.aggregate.counters.complete, 1);
    assert_eq!(s.aggregate.counters.truncated, 0);
    assert_eq!(s.aggregate.observed_tokens.total_tokens, Some(5));
}

#[tokio::test]
async fn compressed_unsupported_and_oversized_payloads_pass_through() {
    // Valid gzip member containing an empty payload, with binary bytes.
    let gzip = vec![
        31, 139, 8, 0, 0, 0, 0, 0, 0, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    let store = roundtrip("application/json", Some("gzip"), gzip, false).await;
    assert_eq!(store.snapshot().aggregate.counters.unsupported, 1);
    assert_eq!(store.snapshot().aggregate.counters.unknown, 1);
    let store = roundtrip("application/octet-stream", None, vec![0, 255, 13, 10], true).await;
    assert_eq!(store.snapshot().aggregate.counters.unsupported, 1);
    for (media, body) in [
        ("application/json", vec![b' '; MAX_JSON_BYTES + 1]),
        ("text/event-stream", vec![b'x'; MAX_SSE_BYTES + 1]),
    ] {
        let store = roundtrip(media, None, body, true).await;
        let s = store.snapshot();
        assert_eq!(s.aggregate.counters.parse_limits, 1);
        assert_eq!(s.aggregate.counters.unknown, 1);
        assert_eq!(s.aggregate.counters.truncated, 0);
    }
}

#[tokio::test]
async fn already_ended_body_is_reported_before_keepalive_connection_drops() {
    let upstream = listener().await;
    let store = Arc::new(UsageStore::new());
    let proxy = start(config(upstream.local_addr().unwrap()), store.clone()).await;
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        headers(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
            .await
            .unwrap();
        closed(&mut socket).await;
    });
    let mut socket = TcpStream::connect(proxy.local_addr()).await.unwrap();
    socket
        .write_all(b"GET / HTTP/1.1\r\nHost: test\r\n\r\n")
        .await
        .unwrap();
    assert!(headers(&mut socket).await.starts_with("HTTP/1.1 204"));
    reports(&store, 1).await;
    assert_eq!(store.snapshot().aggregate.counters.truncated, 0);
    drop(socket);
    proxy.shutdown().await.unwrap();
    mock.await.unwrap();
    assert_eq!(store.snapshot().aggregate.counters.reports, 1);
}

#[tokio::test]
async fn body_cancel_error_and_idle_timeout_keep_observed_prefix_and_record_once() {
    for mode in ["cancel", "error", "timeout"] {
        let upstream = listener().await;
        let store = Arc::new(UsageStore::new());
        let proxy = start(config(upstream.local_addr().unwrap()), store.clone()).await;
        let prefix = b"data: {\"usage\":{\"prompt_tokens\":3}}\n\n";
        let (sent, received) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        let mock = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            headers(&mut socket).await;
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 999999\r\n\r\n").await.unwrap();
            socket.write_all(prefix).await.unwrap();
            sent.send(()).unwrap();
            wait.await.unwrap();
            if mode != "error" {
                closed(&mut socket).await;
            }
        });
        let mut client = raw(proxy.local_addr()).await;
        assert!(headers(&mut client).await.starts_with("HTTP/1.1 200"));
        let mut bytes = vec![0; prefix.len()];
        timeout(Duration::from_secs(4), client.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bytes, prefix);
        received.await.unwrap();
        assert_eq!(store.snapshot().aggregate.counters.reports, 0);
        release.send(()).unwrap();
        if mode == "cancel" {
            drop(client);
        } else {
            closed(&mut client).await;
        }
        reports(&store, 1).await;
        let s = store.snapshot();
        assert_eq!(s.aggregate.counters.truncated, 1, "{mode}");
        assert_eq!(s.aggregate.observed_tokens.input_tokens, Some(3), "{mode}");
        mock.await.unwrap();
        proxy.shutdown().await.unwrap();
        assert_eq!(store.snapshot().aggregate.counters.reports, 1);
    }
}

#[tokio::test]
async fn preheader_failure_timeout_and_cancellation_are_unknown_once() {
    for mode in ["error", "timeout", "cancel"] {
        let upstream = listener().await;
        let store = Arc::new(UsageStore::new());
        let proxy = start(config(upstream.local_addr().unwrap()), store.clone()).await;
        let (sent, received) = oneshot::channel();
        let mock = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            headers(&mut socket).await;
            sent.send(()).unwrap();
            if mode != "error" {
                closed(&mut socket).await;
            }
        });
        let mut client = raw(proxy.local_addr()).await;
        received.await.unwrap();
        if mode == "cancel" {
            drop(client);
        } else {
            let response = headers(&mut client).await;
            let expected = if mode == "error" { "502" } else { "504" };
            assert!(response.starts_with(&format!("HTTP/1.1 {expected}")));
        }
        reports(&store, 1).await;
        mock.await.unwrap();
        proxy.shutdown().await.unwrap();
        let s = store.snapshot();
        assert_eq!(s.aggregate.counters.reports, 1);
        assert_eq!(s.aggregate.counters.unknown, 1);
        assert_eq!(s.aggregate.counters.truncated, 1);
    }
}

#[tokio::test]
async fn connection_failure_is_counted_but_local_replies_are_not_and_session_survives_restart() {
    let upstream = listener().await;
    let mut cfg = config(upstream.local_addr().unwrap());
    drop(upstream);
    cfg.access.client_token_enabled = true;
    cfg.access.client_token = SecretString::new("synthetic-client-token");
    let store = Arc::new(UsageStore::new());
    let proxy = start(cfg.clone(), store.clone()).await;
    assert_eq!(get(proxy.local_addr(), "/__proxy/health").await.0, 200);
    assert_eq!(get(proxy.local_addr(), "/").await.0, 407);
    assert_eq!(
        get(proxy.local_addr(), "http://example.invalid/").await.0,
        501
    );
    assert_eq!(store.snapshot().aggregate.counters.reports, 0);
    proxy.shutdown().await.unwrap();
    cfg.access.client_token_enabled = false;
    // Exercise the configured-listener public API as well as the injected one.
    let bound = listener().await;
    cfg.listen.address = bound.local_addr().unwrap().ip();
    cfg.listen.port = bound.local_addr().unwrap().port();
    drop(bound);
    let proxy = proxy::start_with_usage(cfg.clone(), store.clone())
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&store, &proxy.usage_store()));
    assert_eq!(get(proxy.local_addr(), "/").await.0, 502);
    proxy.shutdown().await.unwrap();
    reports(&store, 1).await;
    let proxy = start(cfg, store.clone()).await;
    assert_eq!(get(proxy.local_addr(), "/").await.0, 502);
    proxy.shutdown().await.unwrap();
    let s = store.snapshot();
    assert_eq!(s.aggregate.counters.reports, 2);
    assert_eq!(s.aggregate.counters.unknown, 2);
    assert_eq!(s.aggregate.counters.truncated, 2);
}
