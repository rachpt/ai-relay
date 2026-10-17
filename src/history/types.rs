use ai_relay_usage::{ObservationFormat, Provider, TokenUsage, UsageReport};
use std::{
    net::IpAddr,
    path::PathBuf,
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

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
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
/// A user-chosen display name, never a raw key or a fragment of one. Pattern
/// checks reject common secret formats, but cannot identify every possible key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialAlias(String);
impl CredentialAlias {
    pub fn new(value: &str) -> Result<Self> {
        if !(1..=64).contains(&value.chars().count())
            || value.trim().is_empty()
            || value.chars().any(|c| {
                c.is_control()
                    || matches!(c, '\u{ad}' | '\u{61c}' | '\u{200b}'..='\u{200f}'
                        | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}'
                        | '\u{feff}' | '\u{fff9}'..='\u{fffb}')
            })
        {
            return Err(Error::Invalid("credential alias display text"));
        }
        let lower = value.to_ascii_lowercase();
        for (i, _) in lower.char_indices() {
            if i != 0
                && lower[..i]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                continue;
            }
            if [
                "sk-",
                "sk_",
                "rk-",
                "pk_",
                "aiza",
                "akia",
                "asia",
                "ghp_",
                "github_pat_",
                "bearer ",
            ]
            .iter()
            .any(|prefix| lower[i..].starts_with(prefix))
            {
                return Err(Error::Invalid("credential alias resembles a secret"));
            }
        }
        Ok(Self(value.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialAliasEntry {
    pub credential: CredentialId,
    pub alias: CredentialAlias,
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
            || chrono::DateTime::from_timestamp_millis(self.started_ms).is_none()
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupBy {
    Day,
    Hour,
    Month,
    Model,
    RequestedModel,
    Protocol,
    Ip,
    Credential,
    UpstreamCredential,
    Interface,
    Provider,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TimeZone {
    #[default]
    Utc,
    /// The OS local zone at each historical instant, including its DST rules.
    Local,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TrendGranularity {
    Hour,
    #[default]
    Day,
    Month,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenField {
    Input,
    Output,
    Total,
    CachedInput,
    Reasoning,
    CacheCreationInput,
    CacheReadInput,
}
impl TokenField {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Input => 0,
            Self::Output => 1,
            Self::Total => 2,
            Self::CachedInput => 3,
            Self::Reasoning => 4,
            Self::CacheCreationInput => 5,
            Self::CacheReadInput => 6,
        }
    }
}
/// Unknown token values sort last in both directions; ties use time/request ID.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DetailSort {
    TimeAsc,
    #[default]
    TimeDesc,
    TokensAsc(TokenField),
    TokensDesc(TokenField),
}
/// Keys break ties deterministically. TimeAsc uses the earliest request in a
/// group; TimeDesc uses the latest. Unknown token totals always sort last.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GroupSort {
    #[default]
    KeysAsc,
    KeysDesc,
    CountAsc,
    CountDesc,
    TimeAsc,
    TimeDesc,
    TokensAsc(TokenField),
    TokensDesc(TokenField),
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
    /// Filters remain UTC milliseconds with an inclusive start/exclusive end.
    /// This zone controls calendar grouping, trend buckets and export labels.
    pub time_zone: TimeZone,
    pub trend_granularity: TrendGranularity,
    pub detail_sort: DetailSort,
    pub group_sort: GroupSort,
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
            time_zone: TimeZone::default(),
            trend_granularity: TrendGranularity::default(),
            detail_sort: DetailSort::default(),
            group_sort: GroupSort::default(),
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
    /// Legacy calendar-day label as days since 1970-01-01, not an hour/month
    /// index. For Local this labels the local date; Month labels its first day.
    pub utc_day: i64,
    pub bucket_start_ms: i64,
    pub label: String,
    pub summary: Summary,
}
#[derive(Clone, Debug)]
pub struct QueryReport {
    pub summary: Summary,
    pub groups: Vec<Group>,
    pub details: Vec<RequestRecord>,
    pub trend: Vec<TrendPoint>,
    pub trend_truncated: bool,
    /// Only aliases referenced by this page's details/groups, at this snapshot.
    /// Group keys and records retain their irreversible credential identifiers.
    pub aliases: Vec<CredentialAliasEntry>,
}
impl QueryReport {
    /// Bounded page only, not a whole-database export. IPs and credential IDs are omitted.
    pub fn export_csv(&self) -> String {
        fn cell(s: &str) -> String {
            let safe = if super::csv::needs_formula_guard(s) {
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
impl Options {
    pub(super) fn validate(self) -> Result<()> {
        if !(1..=65536).contains(&self.queue_capacity)
            || !(1..=90).contains(&self.retention_days)
            || !(1..=100_000).contains(&self.max_records)
        {
            return Err(Error::Invalid("options"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClearOptions {
    /// Explicit opt-in: clear() preserves aliases as well as HMAC identity.
    pub remove_aliases: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointStatus {
    Complete,
    /// Policy/deletions committed, but an active report/export or external
    /// reader prevented truncation; writer admission is not paused.
    Busy,
}
#[derive(Clone, Debug)]
pub struct StorageStats {
    pub options: Options,
    pub records: u64,
    pub aliases: u64,
    pub database_bytes: u64,
    pub wal_bytes: u64,
    pub shared_memory_bytes: u64,
    /// Reusable SQLite pages, not a promise of reclaimed disk space.
    pub free_database_bytes: u64,
}
impl StorageStats {
    pub fn total_bytes(&self) -> u64 {
        self.database_bytes
            .saturating_add(self.wal_bytes)
            .saturating_add(self.shared_memory_bytes)
    }
}
#[derive(Clone, Debug)]
pub struct PolicyReport {
    pub options: Options,
    pub removed_records: u64,
    pub checkpoint: CheckpointStatus,
    pub storage: StorageStats,
}
#[derive(Clone, Debug)]
pub struct ExportSummary {
    pub path: PathBuf,
    pub rows: u64,
    pub bytes: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportFileState {
    NotCreated,
    /// A new file was created and retained; it is not a successful export.
    Incomplete,
    /// Worker completion was lost. Inspect the target; never blindly retry.
    Unknown,
}
#[derive(Debug)]
pub struct ExportError {
    pub cause: Error,
    pub path: PathBuf,
    /// Complete rows accepted before failure, not a durability guarantee.
    pub rows_written: u64,
    pub file_state: ExportFileState,
}
impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "history CSV export failed after {} rows ({:?}): {}",
            self.rows_written, self.file_state, self.cause
        )
    }
}
impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
pub type ExportResult<T> = std::result::Result<T, ExportError>;
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
