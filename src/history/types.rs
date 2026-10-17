use ai_relay_usage::{ObservationFormat, Provider, TokenUsage, UsageReport};
use std::{
    net::IpAddr,
    time::{SystemTime, UNIX_EPOCH},
};
// The parent supplies the application's existing, non-serializable usage types.
use super::ai_relay_usage;

#[derive(Debug)]
pub enum Error {
    Invalid(&'static str),
    Io(std::io::Error),
    Database(rusqlite::Error),
    Closed,
    Busy,
    Worker,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) => write!(f, "invalid history {s}"),
            Self::Io(e) => write!(f, "history I/O: {}", e.kind()),
            Self::Database(_) => f.write_str("history database operation failed"),
            Self::Closed => f.write_str("history worker closed"),
            Self::Busy => f.write_str("history queue full"),
            Self::Worker => f.write_str("history worker failed"),
        }
    }
}
impl Error {
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Io(e) if e.kind() == std::io::ErrorKind::TimedOut)
    }
    pub(super) fn timeout() -> Self {
        Self::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "history closure is not proven",
        ))
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialId(pub(super) String);
impl CredentialId {
    /// Accept only the complete local HMAC identifier, never an API key or prefix.
    pub fn parse(value: &str) -> Result<Self> {
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid("credential identifier"));
        }
        Ok(Self(value.to_ascii_lowercase()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestId(pub(super) String);
impl RequestId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Model(pub(super) String);
impl Model {
    pub fn new(s: &str) -> Result<Self> {
        if s.is_empty()
            || s.len() > 128
            || !s
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.:/@".contains(&b))
        {
            return Err(Error::Invalid("model"));
        }
        Ok(Self(s.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderOrigin(pub(super) String);
impl ProviderOrigin {
    pub fn new(s: &str) -> Result<Self> {
        let url = url::Url::parse(s).map_err(|_| Error::Invalid("provider origin"))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(Error::Invalid("provider origin"));
        }
        let origin = url.origin().ascii_serialization();
        if origin.len() > 512 {
            return Err(Error::Invalid("provider origin"));
        }
        Ok(Self(origin))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
macro_rules! categories { ($name:ident {$($variant:ident => $value:literal),+}) => {
#[derive(Clone,Copy,Debug,Eq,PartialEq)] pub enum $name {$($variant),+}
impl $name { pub(super) fn text(self)-> &'static str { match self {$(Self::$variant=>$value),+} } pub(super) fn parse(s:&str)->Result<Self> {match s {$($value=>Ok(Self::$variant),)+ _=>Err(Error::Invalid(stringify!($name)))}} }
}; }
categories!(Protocol { Unknown=>"unknown", OpenAi=>"openai", Anthropic=>"anthropic", Http=>"http" });
categories!(Interface { Unknown=>"unknown", ChatCompletions=>"chat_completions", Responses=>"responses", Messages=>"messages", Forward=>"forward", Connect=>"connect" });
categories!(Outcome { Success=>"success", HttpError=>"http_error", Cancelled=>"cancelled", Timeout=>"timeout", TransportError=>"transport_error", Rejected=>"rejected" });
#[derive(Clone, Debug)]
pub struct RequestRecord {
    pub request_id: RequestId,
    pub started_ms: i64,
    pub peer_ip: IpAddr,
    pub client: Option<CredentialId>,
    pub upstream: Option<CredentialId>,
    pub provider: Option<ProviderOrigin>,
    pub requested_model: Option<Model>,
    pub protocol: Protocol,
    pub interface: Interface,
    pub status: Option<u16>,
    pub outcome: Outcome,
    pub latency_ms: u64,
    pub total_ms: u64,
    pub report: UsageReport,
}
impl RequestRecord {
    pub(super) fn validate(&self) -> Result<()> {
        if self.started_ms < 0
            || self.status.is_some_and(|s| !(100..=599).contains(&s))
            || self.latency_ms > self.total_ms
            || self.total_ms > i64::MAX as u64
        {
            return Err(Error::Invalid("record"));
        }
        if let Some(m) = &self.report.model {
            Model::new(m)?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub enum GroupBy {
    Day,
    Hour,
    Model,
    Ip,
    Credential,
    UpstreamCredential,
    Interface,
    Provider,
}
impl GroupBy {
    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::Day => "started_ms / 86400000",
            Self::Hour => "started_ms / 3600000",
            Self::Model => "response_model",
            Self::Ip => "ip",
            Self::Credential => "client",
            Self::UpstreamCredential => "upstream",
            Self::Interface => "interface",
            Self::Provider => "origin",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Query {
    pub time_from: Option<i64>,
    pub time_to: Option<i64>,
    pub ip: Option<IpAddr>,
    pub requested_model: Option<Model>,
    pub model: Option<Model>,
    pub client: Option<CredentialId>,
    pub upstream: Option<CredentialId>,
    pub protocol: Option<Protocol>,
    pub provider: Option<ProviderOrigin>,
    pub status: Option<u16>,
    pub outcome: Option<Outcome>,
    pub interface: Option<Interface>,
    pub group_by: Vec<GroupBy>,
    pub limit: u32,
    pub offset: u32,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            time_from: None,
            time_to: None,
            ip: None,
            requested_model: None,
            model: None,
            client: None,
            upstream: None,
            protocol: None,
            provider: None,
            status: None,
            outcome: None,
            interface: None,
            group_by: vec![],
            limit: 100,
            offset: 0,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub requests: u64,
    pub complete: u64,
    pub partial: u64,
    pub unknown: u64,
    pub truncated: u64,
    pub parse_errors: u64,
    pub parse_limits: u64,
    pub provider_errors: u64,
    pub arithmetic_overflows: u64,
    pub unsupported: u64,
    pub tokens: TokenUsage,
    pub coverage: [u64; 7],
}
#[derive(Clone, Debug)]
pub struct Group {
    pub keys: Vec<Option<String>>,
    pub summary: Summary,
}
#[derive(Clone, Debug)]
pub struct TrendPoint {
    pub utc_day: i64,
    pub summary: Summary,
}
#[derive(Clone, Debug)]
pub struct QueryReport {
    pub summary: Summary,
    pub groups: Vec<Group>,
    pub details: Vec<RequestRecord>,
    pub trend: Vec<TrendPoint>,
    pub trend_truncated: bool,
}
impl QueryReport {
    /// Bounded page only, not a whole-database export. IPs and credential IDs are omitted.
    pub fn export_csv(&self) -> String {
        fn cell(s: &str) -> String {
            let safe = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
                format!("'{s}")
            } else {
                s.into()
            };
            format!("\"{}\"", safe.replace('"', "\"\""))
        }
        let mut s = String::from(
            "started_utc_ms,requested_model,response_model,protocol,interface,status,outcome,input_tokens,output_tokens,total_tokens\r\n",
        );
        for r in &self.details {
            let fields = [
                r.started_ms.to_string(),
                r.requested_model
                    .as_ref()
                    .map(|v| v.0.clone())
                    .unwrap_or_default(),
                r.report.model.clone().unwrap_or_default(),
                r.protocol.text().into(),
                r.interface.text().into(),
                r.status.map(|v| v.to_string()).unwrap_or_default(),
                r.outcome.text().into(),
                r.report
                    .usage
                    .input_tokens
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                r.report
                    .usage
                    .output_tokens
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                r.report
                    .usage
                    .total_tokens
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ];
            s.push_str(&fields.iter().map(|v| cell(v)).collect::<Vec<_>>().join(","));
            s.push_str("\r\n");
        }
        s
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub queue_capacity: usize,
    pub retention_days: u32,
    pub max_records: u32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            queue_capacity: 1024,
            retention_days: 90,
            max_records: 100_000,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    pub queued: u64,
    pub written: u64,
    pub queue_full: u64,
    pub db_failures: u64,
    pub closed: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TryRecord {
    Queued,
    Full,
    Closed,
    Invalid,
}
pub(super) fn usage_values(u: &TokenUsage) -> [Option<u64>; 7] {
    [
        u.input_tokens,
        u.output_tokens,
        u.total_tokens,
        u.cached_input_tokens,
        u.reasoning_tokens,
        u.cache_creation_input_tokens,
        u.cache_read_input_tokens,
    ]
}
pub(super) fn usage_from(v: [Option<u64>; 7]) -> TokenUsage {
    TokenUsage {
        input_tokens: v[0],
        output_tokens: v[1],
        total_tokens: v[2],
        cached_input_tokens: v[3],
        reasoning_tokens: v[4],
        cache_creation_input_tokens: v[5],
        cache_read_input_tokens: v[6],
    }
}
pub(super) fn provider_text(p: Provider) -> &'static str {
    match p {
        Provider::Unknown => "unknown",
        Provider::OpenAiChat => "chat",
        Provider::OpenAiResponses => "responses",
        Provider::Anthropic => "anthropic",
    }
}
pub(super) fn format_text(f: ObservationFormat) -> &'static str {
    match f {
        ObservationFormat::UnsupportedContentType => "type",
        ObservationFormat::UnsupportedContentEncoding => "encoding",
        ObservationFormat::Json => "json",
        ObservationFormat::Sse => "sse",
    }
}
