use std::{fmt, net::IpAddr};

use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use url::Url;

pub const SCHEMA_VERSION: u32 = 1;

/// Debug 永不显示内容；显式序列化仍可能包含明文，不可用于日志或默认导出。
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString([REDACTED])")
    }
}

macro_rules! config_enum {
    ($name:ident { $default:ident, $($other:ident),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            #[default]
            $default,
            $($other),*
        }
    };
}

config_enum!(InjectionMode {
    Off,
    Always,
    Missing
});
config_enum!(ThemeMode {
    System,
    Light,
    Dark
});
config_enum!(SecretStorage { Dpapi, Plaintext });
config_enum!(StorageMode { Standard, Portable });
config_enum!(LogLevel {
    Info,
    Trace,
    Debug,
    Warn,
    Error,
    Off
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    // 缺失版本也必须失败，而不是由 Config::default 隐式补齐。
    #[serde(deserialize_with = "deserialize_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub upstream: UpstreamConfig,
    #[serde(default)]
    pub listen: ListenConfig,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub keys: KeyConfig,
    #[serde(default)]
    pub access: AccessConfig,
    #[serde(default)]
    pub cors: CorsConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub system: SystemConfig,
    #[serde(default)]
    pub storage: StorageConfig,
}

fn deserialize_version<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(d)?;
    if version != SCHEMA_VERSION {
        return Err(serde::de::Error::custom(
            "unsupported configuration version",
        ));
    }
    Ok(version)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            upstream: UpstreamConfig::default(),
            listen: ListenConfig::default(),
            proxy: ProxyConfig::default(),
            keys: KeyConfig::default(),
            access: AccessConfig::default(),
            cors: CorsConfig::default(),
            network: NetworkConfig::default(),
            logging: LoggingConfig::default(),
            ui: UiConfig::default(),
            system: SystemConfig::default(),
            storage: StorageConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UpstreamConfig {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ListenConfig {
    pub address: IpAddr,
    pub port: u16,
}
impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            address: IpAddr::from([127, 0, 0, 1]),
            port: 8080,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProxyConfig {
    pub auto_start: bool,
    pub forward_enabled: bool,
    pub rewrite_forward_to_upstream: bool,
    pub connect_enabled: bool,
    pub connect_allowed_ports: Vec<u16>,
}
impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            auto_start: true,
            forward_enabled: false,
            rewrite_forward_to_upstream: true,
            connect_enabled: false,
            connect_allowed_ports: vec![443],
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeyConfig {
    pub upstream_api_key: SecretString,
    pub injection_mode: InjectionMode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccessConfig {
    #[serde(deserialize_with = "deserialize_whitelist")]
    pub whitelist: Vec<IpNet>,
    pub client_token_enabled: bool,
    pub client_token: SecretString,
}
fn deserialize_whitelist<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<IpNet>, D::Error> {
    Vec::<String>::deserialize(d)?
        .into_iter()
        .map(|value| {
            value
                .parse::<IpNet>()
                .or_else(|_| value.parse::<IpAddr>().map(IpNet::from))
                .map_err(|_| serde::de::Error::custom("expected an IP address or CIDR"))
        })
        .collect()
}

impl AccessConfig {
    /// 回环放行仅适用于地址检查；调用者仍须独立检查客户端令牌。
    pub fn allows_ip(&self, ip: IpAddr) -> bool {
        let ip = normalize_ip(ip);
        ip.is_loopback()
            || self.whitelist.is_empty()
            || self.whitelist.iter().any(|net| {
                net.contains(&ip)
                    || match net {
                        IpNet::V6(net) if net.prefix_len() >= 96 => net
                            .network()
                            .to_ipv4_mapped()
                            .and_then(|address| {
                                ipnet::Ipv4Net::new(address, net.prefix_len() - 96).ok()
                            })
                            .is_some_and(|net| matches!(ip, IpAddr::V4(ip) if net.contains(&ip))),
                        _ => false,
                    }
            })
    }
}
fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        ip => ip,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CorsConfig {
    pub enabled: bool,
    pub allowed_origins: Vec<String>,
    pub allow_credentials: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    pub connect_timeout_secs: u32,
    pub response_header_timeout_secs: u32,
    /// 0 显式禁用空闲超时（TOML 无 null）。
    pub response_body_idle_timeout_secs: u32,
    pub tunnel_idle_timeout_secs: u32,
    pub shutdown_grace_secs: u32,
    pub max_active_requests: u32,
}
impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 10,
            response_header_timeout_secs: 120,
            response_body_idle_timeout_secs: 300,
            tunnel_idle_timeout_secs: 300,
            shutdown_grace_secs: 10,
            max_active_requests: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub access_log: bool,
    pub file_enabled: bool,
    pub retention_days: u32,
    pub max_file_bytes: u64,
    pub retained_files: u32,
    pub recent_entries: u32,
}
impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            access_log: true,
            file_enabled: true,
            retention_days: 7,
            max_file_bytes: 5 * 1024 * 1024,
            retained_files: 3,
            recent_entries: 1000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub theme: ThemeMode,
    pub close_to_tray: bool,
    pub remember_window_geometry: bool,
}
impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            close_to_tray: true,
            remember_window_geometry: true,
        }
    }
}

/// 仅表示期望配置；不代表已创建或核实系统启动项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SystemConfig {
    pub login_auto_start: bool,
    pub auto_start_background: bool,
}
impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            login_auto_start: false,
            auto_start_background: true,
        }
    }
}

/// DPAPI 仅保留模式，尚无加解密实现；目录选择不信任该字段。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    pub mode: StorageMode,
    pub secrets: SecretStorage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationError {
    pub field: &'static str,
    pub reason: &'static str,
}
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}
impl std::error::Error for ValidationError {}

fn require(ok: bool, field: &'static str, reason: &'static str) -> Result<(), ValidationError> {
    if ok {
        Ok(())
    } else {
        Err(ValidationError { field, reason })
    }
}

/// 不返回原始 URL，避免错误链暴露认证信息。
pub fn validate_upstream_url(value: &str) -> Result<(), ValidationError> {
    let valid = !value.contains('\\')
        && !value.chars().any(|c| c.is_whitespace() || c.is_control())
        && value.split_once("://").is_some_and(|(_, rest)| {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            !authority.is_empty() && !authority.contains('@')
        })
        && Url::parse(value).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        });
    require(
        valid,
        "upstream.url",
        "expected an HTTP(S) URL without userinfo, query or fragment",
    )
}

impl Config {
    /// 可保存首次初始化草稿；空上游不能据此启动代理。
    pub fn validate_for_save(&self) -> Result<(), ValidationError> {
        require(
            self.schema_version == SCHEMA_VERSION,
            "schema_version",
            "unsupported version",
        )?;
        if !self.upstream.url.is_empty() {
            validate_upstream_url(&self.upstream.url)?;
        }
        require(self.listen.port != 0, "listen.port", "must be nonzero")?;
        require(
            !self.proxy.connect_allowed_ports.contains(&0),
            "proxy.connect_allowed_ports",
            "ports must be nonzero",
        )?;
        require(
            !self.proxy.connect_enabled || !self.proxy.connect_allowed_ports.is_empty(),
            "proxy.connect_allowed_ports",
            "enabled CONNECT requires allowed ports",
        )?;
        require(
            self.keys.injection_mode == InjectionMode::Off
                || !self.keys.upstream_api_key.is_empty(),
            "keys.injection_mode",
            "injection requires an upstream key",
        )?;
        require(
            !self.access.client_token_enabled || !self.access.client_token.is_empty(),
            "access.client_token",
            "enabled authentication requires a token",
        )?;
        for (field, secret) in [
            ("keys.upstream_api_key", &self.keys.upstream_api_key),
            ("access.client_token", &self.access.client_token),
        ] {
            require(
                secret.is_empty()
                    || (!secret.expose_secret().trim().is_empty()
                        && !secret.expose_secret().chars().any(char::is_control)),
                field,
                "secret must be nonblank and contain no control characters",
            )?;
        }
        for origin in &self.cors.allowed_origins {
            if origin == "*" {
                require(
                    !self.cors.allow_credentials,
                    "cors.allowed_origins",
                    "wildcard cannot allow credentials",
                )?;
            } else {
                let valid = validate_upstream_url(origin).is_ok()
                    && Url::parse(origin).is_ok_and(|url| {
                        url.path() == "/"
                            && origin
                                .split_once("://")
                                .is_some_and(|(_, rest)| !rest.contains('/'))
                    });
                require(
                    valid,
                    "cors.allowed_origins",
                    "expected an HTTP(S) origin without path",
                )?;
            }
        }
        // 第一批采用可表达秒数范围：必需超时非零；仅两个 idle 超时允许 0。
        for (field, value) in [
            (
                "network.connect_timeout_secs",
                self.network.connect_timeout_secs,
            ),
            (
                "network.response_header_timeout_secs",
                self.network.response_header_timeout_secs,
            ),
            (
                "network.shutdown_grace_secs",
                self.network.shutdown_grace_secs,
            ),
            (
                "network.max_active_requests",
                self.network.max_active_requests,
            ),
        ] {
            require(value > 0, field, "must be nonzero")?;
        }
        require(
            (1024..=1024 * 1024 * 1024).contains(&self.logging.max_file_bytes),
            "logging.max_file_bytes",
            "must be between 1 KiB and 1 GiB",
        )?;
        require(
            (1..=100).contains(&self.logging.retained_files),
            "logging.retained_files",
            "must be between 1 and 100",
        )?;
        require(
            (1..=100_000).contains(&self.logging.recent_entries),
            "logging.recent_entries",
            "must be between 1 and 100000",
        )?;
        require(
            self.logging.retention_days > 0,
            "logging.retention_days",
            "must be nonzero",
        )?;
        Ok(())
    }

    pub fn validate_for_start(&self) -> Result<(), ValidationError> {
        self.validate_for_save()?;
        require(
            !self.upstream.url.is_empty(),
            "upstream.url",
            "configure upstream before starting",
        )
    }
}
