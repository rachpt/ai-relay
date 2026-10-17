//! Bounded request metadata only; no raw URI, credential or body is retained.
use crate::{
    history::{HistoryHandle, Interface, Outcome, Protocol, ProviderOrigin, RequestRecord, now_ms},
    usage::UsageReport,
};
use hyper::{HeaderMap, Method, Request, Uri};
use std::net::SocketAddr;

pub(super) struct HistoryRecording {
    handle: HistoryHandle,
    record: RequestRecord,
}

/// Identity namespace v1: source header and normalized authentication scheme are
/// part of the HMAC input. Client precedence is Authorization, x-api-key, then
/// Proxy-Authorization; the upstream identity never uses the proxy token.
pub(super) fn credential(
    handle: &HistoryHandle,
    headers: &HeaderMap,
    client: bool,
) -> Option<crate::history::CredentialId> {
    let names: &[&str] = if client {
        &["authorization", "x-api-key", "proxy-authorization"]
    } else {
        &["authorization", "x-api-key"]
    };
    for name in names {
        let mut values = headers.get_all(*name).iter();
        let Some(value) = values.next() else { continue };
        // Ambiguous, invalid and oversized identities stay unknown, not partial.
        if values.next().is_some() {
            return None;
        }
        let value = value.to_str().ok()?;
        if value.is_empty() {
            continue;
        }
        if value.len() > 8192 {
            return None;
        }
        let (scheme, secret) = if *name == "x-api-key" {
            ("key".to_owned(), value)
        } else {
            let (scheme, secret) = value.split_once(' ')?;
            if scheme.is_empty()
                || scheme.len() > 32
                || !scheme
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return None;
            }
            (scheme.to_ascii_lowercase(), secret.trim_matches(' '))
        };
        if secret.is_empty() {
            return None;
        }
        return Some(handle.credential(&format!("proxy-identity/v1\0{name}\0{scheme}\0{secret}")));
    }
    None
}

impl HistoryRecording {
    pub(super) fn new<B>(handle: HistoryHandle, peer: SocketAddr, request: &Request<B>) -> Self {
        let interface = if request.method() == Method::CONNECT {
            Interface::Connect
        } else {
            match request.uri().path() {
                "/v1/chat/completions" => Interface::ChatCompletions,
                "/v1/responses" => Interface::Responses,
                "/v1/messages" => Interface::Messages,
                _ if request.uri().scheme().is_some() => Interface::Forward,
                _ => Interface::Unknown,
            }
        };
        let protocol = match interface {
            Interface::ChatCompletions | Interface::Responses => Protocol::OpenAi,
            Interface::Messages => Protocol::Anthropic,
            Interface::Forward | Interface::Connect => Protocol::Http,
            _ => Protocol::Unknown,
        };
        let record = RequestRecord {
            request_id: handle.request_id(),
            started_ms: now_ms(),
            peer_ip: peer.ip(),
            client: credential(&handle, request.headers(), true),
            upstream: None,
            provider: None,
            requested_model: None,
            protocol,
            interface,
            status: None,
            outcome: Outcome::Cancelled,
            latency_ms: 0,
            total_ms: 0,
            report: UsageReport::default(),
        };
        Self { handle, record }
    }
    pub(super) fn target(&mut self, uri: &Uri, headers: &HeaderMap) {
        // Construct only the origin, never copy path/query into metadata.
        self.record.provider = uri
            .scheme_str()
            .zip(uri.authority())
            .and_then(|(scheme, host)| ProviderOrigin::new(&format!("{scheme}://{host}")).ok());
        self.record.upstream = credential(&self.handle, headers, false);
    }
    pub(super) fn response_headers(&mut self, status: u16, elapsed: u64) {
        self.record.status = Some(status);
        self.record.latency_ms = elapsed;
    }
    pub(super) fn finish(
        mut self,
        status: Option<u16>,
        elapsed: u64,
        outcome: Outcome,
        mut report: UsageReport,
        requested_model: Option<crate::history::Model>,
    ) {
        if report
            .model
            .as_deref()
            .is_some_and(|s| crate::history::Model::new(s).is_err())
        {
            report.model = None;
        }
        // Independent request-side metadata only; never infer it from usage's
        // response model, even for cancellation, parse limits or missing JSON.
        self.record.requested_model = requested_model;
        self.record.status = status;
        self.record.total_ms = elapsed;
        self.record.outcome = outcome;
        self.record.report = report;
        // Full/closed counters belong to HistoryHandle. Never retry or wait here.
        let _ = self.handle.try_record(self.record);
    }
}
