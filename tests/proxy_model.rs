//! Request-side observation through real streaming sockets and persisted history.
use ai_relay::{
    config::Config,
    history::{HistoryService, Model, Query},
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
    sync::oneshot,
    time::timeout,
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "relay-request-model-{}-{}",
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
async fn listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}
async fn config(addr: SocketAddr) -> Config {
    let reserved = listener().await;
    let mut cfg = Config::default();
    cfg.listen.address = reserved.local_addr().unwrap().ip();
    cfg.listen.port = reserved.local_addr().unwrap().port();
    cfg.upstream.url = format!("http://{addr}/");
    cfg.network.connect_timeout_secs = 2;
    cfg.network.response_header_timeout_secs = 10;
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
async fn service(path: PathBuf) -> HistoryService {
    tokio::task::spawn_blocking(move || HistoryService::start(path).unwrap())
        .await
        .unwrap()
}
async fn recorded(handle: &ai_relay::history::HistoryHandle) {
    timeout(Duration::from_secs(5), async {
        while handle.counters().queued < 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(handle.counters().queued, 1);
}

async fn roundtrip(payload: Vec<u8>, extra: &str, expected_model: Option<&str>, chunked: bool) {
    let temp = Temp::new();
    let service = service(temp.0.clone()).await;
    let history = service.handle();
    let upstream = listener().await;
    let proxy = proxy::start_with_telemetry(
        config(upstream.local_addr().unwrap()).await,
        Arc::new(UsageStore::new()),
        None,
        Some(history.clone()),
    )
    .await
    .unwrap();
    let metrics = proxy.metrics();
    let expected = payload.clone();
    let mock = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        let head = headers(&mut socket).await.to_ascii_lowercase();
        let mut actual = Vec::new();
        if chunked {
            assert!(head.contains("transfer-encoding: chunked"));
            loop {
                let mut line = Vec::new();
                while !line.ends_with(b"\r\n") {
                    line.push(socket.read_u8().await.unwrap());
                }
                let size =
                    usize::from_str_radix(String::from_utf8(line).unwrap().trim(), 16).unwrap();
                if size == 0 {
                    let mut trailers = Vec::new();
                    while !trailers.ends_with(b"\r\n") {
                        trailers.push(socket.read_u8().await.unwrap());
                    }
                    break;
                }
                let offset = actual.len();
                actual.resize(offset + size, 0);
                socket.read_exact(&mut actual[offset..]).await.unwrap();
                assert_eq!(socket.read_u8().await.unwrap(), b'\r');
                assert_eq!(socket.read_u8().await.unwrap(), b'\n');
            }
        } else {
            actual.resize(expected.len(), 0);
            socket.read_exact(&mut actual).await.unwrap();
        }
        assert_eq!(
            actual, expected,
            "observation must never rewrite any body byte"
        );
        let response = br#"{"model":"response-only-model","usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).as_bytes()).await.unwrap();
        socket.write_all(response).await.unwrap();
    });
    let mut client = TcpStream::connect(proxy.local_addr()).await.unwrap();
    let length = if chunked {
        "Transfer-Encoding: chunked\r\n".to_owned()
    } else {
        format!("Content-Length: {}\r\n", payload.len())
    };
    client.write_all(format!("POST /v1/chat/completions HTTP/1.1\r\nHost: test\r\nConnection: close\r\n{extra}{length}\r\n").as_bytes()).await.unwrap();
    // Small independent chunks exercise split UTF-8/escapes and raw-byte equality.
    for part in payload.chunks(17) {
        if chunked {
            client
                .write_all(format!("{:x}\r\n", part.len()).as_bytes())
                .await
                .unwrap();
        }
        client.write_all(part).await.unwrap();
        if chunked {
            client.write_all(b"\r\n").await.unwrap();
        }
    }
    if chunked {
        client.write_all(b"0\r\n\r\n").await.unwrap();
    }
    assert!(drain(&mut client).await.starts_with(b"HTTP/1.1 200"));
    mock.await.unwrap();
    recorded(&history).await;
    proxy.shutdown().await.unwrap(); // Join all guards before querying persisted history.
    let wanted = expected_model.map(|s| Model::new(s).unwrap());
    tokio::task::spawn_blocking(move || {
        service.barrier().unwrap();
        let result = service.query(Query::default()).unwrap();
        assert_eq!(result.summary.requests, 1);
        assert_eq!(result.details[0].requested_model, wanted);
        assert_eq!(
            result.details[0].report.model.as_deref(),
            Some("response-only-model")
        );
        if wanted.is_some() {
            let joined = service
                .query(Query {
                    requested_model: wanted,
                    model: Some(Model::new("response-only-model").unwrap()),
                    ..Query::default()
                })
                .unwrap();
            assert_eq!(joined.summary.requests, 1);
        }
        assert_eq!(history.counters().db_failures, 0);
        let mut service = service;
        service.shutdown().unwrap();
    })
    .await
    .unwrap();
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.total_requests, 1);
    assert_eq!(snapshot.errors, 0);
    assert_eq!(snapshot.upload_body_bytes, payload.len() as u64);
    assert_eq!(snapshot.active_requests, 0);
    assert_eq!(snapshot.inbound_connections, 0);
    let stored = std::fs::read(temp.0.join("history.sqlite3")).unwrap();
    assert!(!String::from_utf8_lossy(&stored).contains("prompt-never-persist"));
}

#[tokio::test]
async fn fragmented_top_level_model_and_streaming_chunked_upload_preserve_bytes() {
    let body = br#"{"messages":[{"model":"nested-only","content":"prompt-never-persist"}],"mo\u0064el":"requested\u002dmodel"}"#;
    for chunked in [false, true] {
        roundtrip(
            body.to_vec(),
            "Content-Type: Application/json; charset=utf-8\r\nContent-Encoding: identity\r\n",
            Some("requested-model"),
            chunked,
        )
        .await;
    }
}

#[tokio::test]
async fn duplicate_non_string_invalid_json_and_invalid_model_remain_unknown() {
    for body in [
        br#"{"model":"one","model":"two"}"#.as_slice(),
        br#"{"model":"one","mo\u0064el":"one"}"#,
        br#"{"model":3}"#,
        br#"{"model":null}"#,
        br#"{"model":["one"]}"#,
        br#"{"model":{"model":"one"}}"#,
        br#"{"model":"bad model"}"#,
        br#"{"model":"one""#,
        br#"{"model":"one"}{}"#,
        br#"{"prompt":{"model":"nested-only"}}"#,
        b"{\"model\":\"one\",\"prompt\":\"\xff\"}",
        br#"{"model":"one","prompt":"\uD800"}"#,
    ] {
        roundtrip(
            body.to_vec(),
            "Content-Type: application/json\r\n",
            None,
            false,
        )
        .await;
    }
}

#[tokio::test]
async fn large_requests_compressed_or_ambiguous_headers_stay_unknown_and_pass_through() {
    let mut exact = br#"{"model":"within-budget"}"#.to_vec();
    exact.resize(64 * 1024, b' ');
    roundtrip(
        exact.clone(),
        "Content-Type: application/json\r\n",
        Some("within-budget"),
        true,
    )
    .await;
    exact.push(b' ');
    roundtrip(exact, "Content-Type: application/json\r\n", None, true).await;
    let large = format!(
        "{{\"model\":\"early-model\",\"prompt\":\"{}\"}}",
        "prompt-never-persist".repeat(8000)
    );
    roundtrip(
        large.into_bytes(),
        "Content-Type: application/json\r\n",
        None,
        false,
    )
    .await;
    for extra in [
        "Content-Type: application/json\r\nContent-Encoding: gzip\r\n",
        "Content-Type: application/json\r\nContent-Encoding: br\r\nConnection: content-encoding\r\n",
        "Content-Type: application/json\r\nContent-Encoding: identity, identity\r\n",
        "Content-Type: application/json\r\nContent-Encoding: identity\r\nContent-Encoding: identity\r\n",
        "Content-Type: application/json\r\nContent-Type: application/json\r\n",
        "Content-Type: text/plain\r\n",
        "",
    ] {
        roundtrip(br#"{"model":"not-observed"}"#.to_vec(), extra, None, false).await;
    }
    roundtrip(
        vec![
            31, 139, 8, 0, 0, 0, 0, 0, 0, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
        "Content-Type: application/json\r\nContent-Encoding: gzip\r\n",
        None,
        true,
    )
    .await;
}

#[tokio::test]
async fn zero_preread_and_incomplete_uploads_never_publish_early_model() {
    for mode in ["cancel", "body-error", "early-response", "complete"] {
        let temp = Temp::new();
        let service = service(temp.0.clone()).await;
        let history = service.handle();
        let upstream = listener().await;
        let proxy = proxy::start_with_telemetry(
            config(upstream.local_addr().unwrap()).await,
            Arc::new(UsageStore::new()),
            None,
            Some(history.clone()),
        )
        .await
        .unwrap();
        let metrics = proxy.metrics();
        let (head_ready, head_seen) = oneshot::channel();
        let (prefix_ready, prefix_seen) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        let prefix = br#"{"model":"early-model","prompt":"prompt-never-persist"}"#;
        let mock = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            headers(&mut socket).await;
            head_ready.send(()).unwrap();
            let mut bytes = vec![0; prefix.len()];
            socket.read_exact(&mut bytes).await.unwrap();
            assert_eq!(bytes, prefix);
            prefix_ready.send(()).unwrap();
            wait.await.unwrap();
            if matches!(mode, "early-response" | "complete") {
                if mode == "complete" {
                    let mut suffix = [0; 9];
                    socket.read_exact(&mut suffix).await.unwrap();
                    assert_eq!(&suffix, b"         ");
                }
                socket
                    .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            } else {
                let mut rest = Vec::new();
                let _ = timeout(Duration::from_secs(5), socket.read_to_end(&mut rest))
                    .await
                    .unwrap();
            }
        });
        let mut client = TcpStream::connect(proxy.local_addr()).await.unwrap();
        client.write_all(format!("POST /v1/responses HTTP/1.1\r\nHost: test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", prefix.len() + 9).as_bytes()).await.unwrap();
        // Upstream receives headers before the client sends even one body byte.
        timeout(Duration::from_secs(3), head_seen)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(metrics.snapshot().upload_body_bytes, 0);
        client.write_all(prefix).await.unwrap();
        timeout(Duration::from_secs(3), prefix_seen)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.counters().queued, 0);
        assert_eq!(metrics.snapshot().active_requests, 1);
        release.send(()).unwrap();
        match mode {
            "cancel" => drop(client),
            "body-error" => {
                client.shutdown().await.unwrap();
                let _ = drain(&mut client).await;
            }
            "complete" => {
                client.write_all(b"         ").await.unwrap();
                assert!(drain(&mut client).await.starts_with(b"HTTP/1.1 204"));
            }
            _ => {
                assert!(headers(&mut client).await.starts_with("HTTP/1.1 204"));
                drop(client);
            }
        }
        mock.await.unwrap();
        recorded(&history).await;
        proxy.shutdown().await.unwrap();
        tokio::task::spawn_blocking(move || {
            service.barrier().unwrap();
            let result = service.query(Query::default()).unwrap();
            assert_eq!(result.summary.requests, 1, "{mode}");
            assert_eq!(
                result.details[0]
                    .requested_model
                    .as_ref()
                    .map(Model::as_str),
                if mode == "complete" {
                    Some("early-model")
                } else {
                    None
                },
                "{mode}"
            );
            let mut service = service;
            service.shutdown().unwrap();
        })
        .await
        .unwrap();
        assert_eq!(metrics.snapshot().active_requests, 0);
        assert_eq!(metrics.snapshot().inbound_connections, 0);
    }
}
