use ai_relay::{
    config::{Config, SecretString},
    logging::*,
    proxy,
    usage::UsageStore,
};
use std::{
    collections::HashSet,
    fs,
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
    time::{sleep, timeout},
};

const SECRET: &str = "synthetic-logging-secret";
const QUERY: &str = "synthetic-query-value";
const PAYLOAD: &str = "synthetic-body-content";
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ai-relay-proxy-logging-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
async fn listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}
async fn config(upstream: SocketAddr) -> Config {
    let reserved = listener().await;
    let addr = reserved.local_addr().unwrap();
    let mut cfg = Config::default();
    cfg.listen.address = addr.ip();
    cfg.listen.port = addr.port();
    cfg.upstream.url = format!("http://{upstream}/v1");
    cfg.keys.upstream_api_key = SecretString::new(SECRET);
    cfg.network.connect_timeout_secs = 1;
    cfg.network.response_header_timeout_secs = 1;
    cfg.network.response_body_idle_timeout_secs = 1;
    cfg.network.shutdown_grace_secs = 1;
    cfg
}
async fn head(socket: &mut TcpStream) -> String {
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
async fn request(addr: SocketAddr, method: &str, path: &str) -> TcpStream {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(format!(
        "{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\nAuthorization: Bearer {SECRET}\r\nContent-Length: {}\r\n\r\n{PAYLOAD}", PAYLOAD.len()
    ).as_bytes()).await.unwrap();
    stream
}
async fn drained(stream: &mut TcpStream) {
    timeout(Duration::from_secs(5), async {
        let mut buf = [0; 1024];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await
    .unwrap();
}
async fn entries(log: &LogHandle, count: usize) -> Vec<AccessMetadata> {
    timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = log.snapshot(None);
            assert_eq!(snapshot.status.dropped, 0);
            if snapshot.entries.len() >= count {
                assert_eq!(snapshot.entries.len(), count);
                return snapshot
                    .entries
                    .into_iter()
                    .map(|e| match e.event {
                        LogEvent::Access { metadata, .. } => metadata,
                        _ => panic!("unexpected runtime log"),
                    })
                    .collect();
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
fn safe(text: &str) {
    for forbidden in [
        SECRET,
        QUERY,
        PAYLOAD,
        "token=",
        "http://",
        "private-path",
        "CUSTOMSECRET",
    ] {
        assert!(
            !text.contains(forbidden),
            "sensitive log content: {forbidden}"
        );
    }
}

// LogService stays on its owning thread; only the Send handle enters the proxy.
#[test]
fn terminal_observations_are_once_sanitized_and_independent_of_file_logging() {
    for file_enabled in [true, false] {
        let temp = Temp::new();
        let mut service = LogService::start(
            temp.0.join("logs"),
            LogOptions {
                file_enabled,
                ..Default::default()
            },
        )
        .unwrap();
        let log = service.handle();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut expected = 0;
            let mut ids = HashSet::new();
            for mode in [
                "success",
                "empty",
                "http_error",
                "auth",
                "unsupported",
                "header_error",
                "header_timeout",
                "header_cancel",
                "body_error",
                "body_timeout",
                "body_cancel",
            ] {
                let upstream = listener().await;
                let mut cfg = config(upstream.local_addr().unwrap()).await;
                if mode == "auth" {
                    cfg.access.client_token_enabled = true;
                    cfg.access.client_token = SecretString::new("different-synthetic-token");
                }
                let usage = Arc::new(UsageStore::new());
                let proxy = proxy::start_with_observers(cfg, usage.clone(), Some(log.clone()))
                    .await
                    .unwrap();
                assert!(Arc::ptr_eq(&usage, &proxy.usage_store()));
                let (sent, mut received) = oneshot::channel();
                let (release, wait) = oneshot::channel();
                let local = matches!(mode, "auth" | "unsupported");
                let mock = tokio::spawn(async move {
                    if local {
                        return;
                    }
                    let (mut stream, _) = upstream.accept().await.unwrap();
                    let headers = head(&mut stream).await;
                    assert!(headers.contains(SECRET));
                    let mut body = vec![0; PAYLOAD.len()];
                    stream.read_exact(&mut body).await.unwrap();
                    assert_eq!(body, PAYLOAD.as_bytes());
                    if mode.starts_with("header_") {
                        sent.send(()).unwrap();
                        if mode != "header_error" {
                            drained(&mut stream).await;
                        }
                    } else if mode.starts_with("body_") {
                        stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 99999\r\n\r\nx")
                            .await
                            .unwrap();
                        sent.send(()).unwrap();
                        wait.await.unwrap();
                        if mode != "body_error" {
                            drained(&mut stream).await;
                        }
                    } else {
                        let status = if mode == "http_error" {
                            "503 Unavailable"
                        } else {
                            "200 OK"
                        };
                        if mode == "empty" {
                            stream
                                .write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
                                .await
                                .unwrap();
                        } else {
                            stream
                                .write_all(
                                    format!(
                                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{PAYLOAD}",
                                        PAYLOAD.len()
                                    )
                                    .as_bytes(),
                                )
                                .await
                                .unwrap();
                        }
                    }
                });
                // Health is intentionally absent from access logs, including auth-enabled mode.
                let mut health = request(proxy.local_addr(), "GET", "/__proxy/health").await;
                assert!(head(&mut health).await.starts_with("HTTP/1.1 200"));
                drained(&mut health).await;
                let path = if mode == "success" {
                    format!("/v1/chat/completions?token={QUERY}")
                } else {
                    format!("/private-path/{SECRET}?token={QUERY}")
                };
                let method = if mode == "unsupported" {
                    "CONNECT"
                } else if mode == "empty" {
                    "CUSTOMSECRET"
                } else {
                    "POST"
                };
                let mut client = request(proxy.local_addr(), method, &path).await;
                if mode.starts_with("header_") {
                    (&mut received).await.unwrap();
                }
                if mode == "header_cancel" {
                    drop(client);
                } else {
                    let response = head(&mut client).await;
                    let status = match mode {
                        "empty" => 204,
                        "http_error" => 503,
                        "auth" => 407,
                        "unsupported" => 501,
                        "header_error" => 502,
                        "header_timeout" => 504,
                        _ => 200,
                    };
                    assert!(
                        response.starts_with(&format!("HTTP/1.1 {status}")),
                        "{mode}: {response}"
                    );
                    if mode.starts_with("body_") {
                        (&mut received).await.unwrap();
                        assert_eq!(client.read_u8().await.unwrap(), b'x');
                        assert_eq!(log.snapshot(None).entries.len(), expected);
                        release.send(()).unwrap();
                    }
                    if mode == "body_cancel" {
                        drop(client);
                    } else {
                        drained(&mut client).await;
                    }
                }
                mock.await.unwrap();
                expected += 1;
                let all = entries(&log, expected).await;
                let entry = all.last().unwrap();
                assert!(ids.insert(entry.request_id.unwrap()));
                assert_eq!(entry.ip, Some("127.0.0.1".parse().unwrap()));
                assert_eq!(
                    entry.endpoint,
                    Some(if mode == "success" {
                        Endpoint::ChatCompletions
                    } else {
                        Endpoint::Redacted
                    })
                );
                assert_eq!(
                    entry.method,
                    Some(if mode == "empty" {
                        Method::Other
                    } else if mode == "unsupported" {
                        Method::Connect
                    } else {
                        Method::Post
                    })
                );
                let error = match mode {
                    "header_timeout" | "body_timeout" => Some(ErrorCode::Timeout),
                    "header_cancel" | "body_cancel" => Some(ErrorCode::Cancelled),
                    "header_error" => Some(ErrorCode::Connection),
                    "body_error" => Some(ErrorCode::Protocol),
                    _ => None,
                };
                assert_eq!(entry.error, error, "{mode}");
                let status = match mode {
                    "empty" => Some(204),
                    "http_error" => Some(503),
                    "auth" => Some(407),
                    "unsupported" => Some(501),
                    "header_error" => Some(502),
                    "header_timeout" => Some(504),
                    "header_cancel" => None,
                    _ => Some(200),
                };
                assert_eq!(entry.status, status, "{mode}");
                let has_headers = !local && !mode.starts_with("header_");
                assert_eq!(entry.first_byte_ms.is_some(), has_headers, "{mode}");
                if let Some(first) = entry.first_byte_ms {
                    assert!(entry.duration_ms.unwrap() >= first);
                }
                if mode.ends_with("timeout") {
                    assert!(entry.duration_ms.unwrap() >= 900);
                }
                assert!(entry.duration_ms.is_some());
                proxy.shutdown().await.unwrap();
                assert_eq!(log.snapshot(None).entries.len(), expected);
                assert_eq!(
                    usage.snapshot().aggregate.counters.reports,
                    u64::from(!local)
                );
                if mode.starts_with("body_") {
                    assert_eq!(usage.snapshot().aggregate.counters.truncated, 1);
                }
                if matches!(mode, "success" | "empty" | "http_error") {
                    assert_eq!(usage.snapshot().aggregate.counters.truncated, 0);
                }
            }
        });
        service.barrier().unwrap();
        let snapshot = log.snapshot(None);
        safe(&serde_json::to_string(&snapshot.entries).unwrap());
        if file_enabled {
            let text = service
                .read_tail(CURRENT_FILE, MAX_TAIL_BYTES, MAX_TAIL_LINES)
                .unwrap()
                .text;
            safe(&text);
            assert_eq!(text.lines().count(), snapshot.entries.len());
            let disk: Vec<serde_json::Value> = text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(
                serde_json::to_value(&snapshot.entries).unwrap(),
                serde_json::to_value(disk).unwrap()
            );
        } else {
            assert!(service.list_files().unwrap().is_empty());
            assert_eq!(snapshot.entries.len(), 11);
        }
        service.shutdown(Duration::from_secs(2)).unwrap();
    }
}
