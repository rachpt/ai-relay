//! CONNECT stays inside its owning HTTP connection task, including upgrade wait.
use super::{AccessRecording, Body, State, reply, secs};
use crate::logging::ErrorCode;
use hyper::{Request, Response, StatusCode, Uri, body::Incoming, upgrade::OnUpgrade};
use hyper_util::rt::TokioIo;
use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    sync::{OwnedSemaphorePermit, watch},
    time::{Instant, timeout},
};

pub(super) struct Tunnel {
    upgrade: OnUpgrade,
    upstream: TcpStream,
    _permit: OwnedSemaphorePermit,
    access: AccessRecording,
    idle_secs: u32,
}

fn destination(uri: &Uri) -> Option<(String, u16)> {
    if uri.scheme().is_some() || uri.path_and_query().is_some() {
        return None;
    }
    let authority = uri.authority()?;
    if authority.as_str().contains('@') {
        return None;
    }
    let port = authority.port_u16().filter(|port| *port != 0)?;
    let host = authority.host();
    if host.starts_with('[') {
        host.strip_prefix('[')?
            .strip_suffix(']')?
            .parse::<std::net::Ipv6Addr>()
            .ok()?;
    } else if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return None;
    }
    Some((
        host.trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned(),
        port,
    ))
}

pub(super) async fn prepare(
    state: &Arc<State>,
    request: &mut Request<Incoming>,
    permit: OwnedSemaphorePermit,
    mut access: AccessRecording,
) -> Result<Tunnel, Box<Response<Body>>> {
    let Some((host, port)) = destination(request.uri()) else {
        return Err(Box::new(access.reply(
            StatusCode::BAD_REQUEST,
            None,
            "invalid CONNECT target",
        )));
    };
    if !state.config.proxy.connect_allowed_ports.contains(&port) {
        return Err(Box::new(access.reply(
            StatusCode::FORBIDDEN,
            None,
            "CONNECT port denied",
        )));
    }
    let upstream = match timeout(
        secs(state.config.network.connect_timeout_secs),
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(_)) => {
            return Err(Box::new(access.reply(
                StatusCode::BAD_GATEWAY,
                Some(ErrorCode::Connection),
                "CONNECT connection failed",
            )));
        }
        Err(_) => {
            return Err(Box::new(access.reply(
                StatusCode::GATEWAY_TIMEOUT,
                Some(ErrorCode::Timeout),
                "CONNECT connection timeout",
            )));
        }
    };
    access.response_headers(StatusCode::OK);
    Ok(Tunnel {
        upgrade: hyper::upgrade::on(request),
        upstream,
        _permit: permit,
        access,
        idle_secs: state.config.network.response_body_idle_timeout_secs,
    })
}

impl Tunnel {
    pub(super) async fn run(mut self) {
        // Hyper preserves any bytes read beyond the CONNECT headers in Upgraded.
        let upgraded = match (&mut self.upgrade).await {
            Ok(upgraded) => upgraded,
            Err(_) => {
                self.access.finish(Some(ErrorCode::Cancelled));
                return;
            }
        };
        // Opaque TCP is deliberately not passed to the AI usage observer.
        let (activity, mut changes) = watch::channel(Instant::now());
        let mut client = Active {
            inner: TokioIo::new(upgraded),
            activity: activity.clone(),
        };
        let mut upstream = Active {
            inner: self.upstream,
            activity,
        };
        let copy = tokio::io::copy_bidirectional(&mut client, &mut upstream);
        tokio::pin!(copy);
        let error = loop {
            let deadline = *changes.borrow_and_update() + secs(self.idle_secs);
            tokio::select! {
                result = &mut copy => break result.err().map(|_| ErrorCode::Connection),
                _ = changes.changed(), if self.idle_secs != 0 => {},
                _ = tokio::time::sleep_until(deadline), if self.idle_secs != 0 => {
                    // A ready I/O operation may have refreshed activity this turn.
                    if Instant::now().duration_since(*changes.borrow()) >= secs(self.idle_secs) {
                        break Some(ErrorCode::Timeout);
                    }
                }
            }
        };
        self.access.finish(error);
    }
}

struct Active<T> {
    inner: T,
    activity: watch::Sender<Instant>,
}
impl<T: AsyncRead + Unpin> AsyncRead for Active<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            this.activity.send_replace(Instant::now());
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for Active<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_write(cx, buf);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            this.activity.send_replace(Instant::now());
        }
        result
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

pub(super) fn established() -> Response<Body> {
    reply(StatusCode::OK, "")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authority_requires_explicit_valid_port_and_unambiguous_host() {
        for target in ["example.invalid:443", "127.0.0.1:443", "[::1]:443"] {
            assert!(destination(&target.parse().unwrap()).is_some(), "{target}");
        }
        for target in [
            "example.invalid",
            "example.invalid:0",
            "example.invalid:65536",
            "user@host:443",
            "http://host:443/",
            "[not-ip]:443",
        ] {
            if let Ok(uri) = target.parse() {
                assert!(destination(&uri).is_none(), "{target}");
            }
        }
    }
}
