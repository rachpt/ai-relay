use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};

use super::{Config, ValidationError};

pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;

/// 不保留 TOML/URL 原始错误或源文本，Display、Debug 和 source 均不泄露秘密。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    AbsolutePathRequired,
    TooLarge,
    InvalidUtf8,
    InvalidToml,
    InvalidConfig(ValidationError),
    Serialization,
    ExternalModification,
    /// Compatibility for existing callers; production DPAPI paths never return this.
    DpapiNotImplemented,

    /// Windows may have partially moved files; retained paths require inspection.
    AtomicReplacementFailed {
        replacement_path: std::path::PathBuf,
        recovery_path: std::path::PathBuf,
        kind: io::ErrorKind,
    },
    /// The main config was committed; the old config remains at this recovery path.
    BackupPublicationFailed {
        recovery_path: std::path::PathBuf,
        kind: io::ErrorKind,
    },
    Io {
        operation: &'static str,
        kind: io::ErrorKind,
    },
}
impl ConfigError {
    pub const DPAPI_UNSUPPORTED: Self = Self::Io {
        operation: "DPAPI requires Windows; no plaintext fallback",
        kind: io::ErrorKind::Unsupported,
    };
    pub const INVALID_PROTECTED_SECRETS: Self = Self::InvalidConfig(ValidationError {
        field: "protected_secrets",
        reason: "invalid envelope, ciphertext or version",
    });
    pub const CONFLICTING_SECRETS: Self = Self::InvalidConfig(ValidationError {
        field: "protected_secrets",
        reason: "ciphertext cannot coexist with plaintext fields or plaintext storage",
    });
    pub const PLAINTEXT_IN_DPAPI: Self = Self::InvalidConfig(ValidationError {
        field: "storage.secrets",
        reason: "plaintext secrets forbidden in DPAPI storage; no plaintext fallback",
    });
    pub const SECRET_TOO_LARGE: Self = Self::InvalidConfig(ValidationError {
        field: "secrets",
        reason: "secret exceeds the 16 KiB limit",
    });
}
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AbsolutePathRequired => f.write_str("configuration file path must be absolute"),
            Self::TooLarge => f.write_str("configuration exceeds the 1 MiB limit"),
            Self::InvalidUtf8 => f.write_str("configuration must be UTF-8"),
            Self::InvalidToml => f.write_str("invalid TOML, field type or configuration version"),
            Self::InvalidConfig(error) => write!(f, "{error}"),
            Self::Serialization => f.write_str("configuration serialization failed"),
            Self::ExternalModification => {
                f.write_str("configuration changed since loading; reload before saving")
            }
            Self::DpapiNotImplemented => f.write_str("legacy DPAPI implementation error"),

            Self::AtomicReplacementFailed {
                replacement_path,
                recovery_path,
                kind,
            } => write!(
                f,
                "configuration replacement failed ({kind:?}); inspect retained replacement {} and recovery {}",
                replacement_path.display(),
                recovery_path.display()
            ),
            Self::BackupPublicationFailed {
                recovery_path,
                kind,
            } => write!(
                f,
                "configuration saved, but backup publication failed ({kind:?}); previous config retained at {}",
                recovery_path.display()
            ),
            Self::Io { operation, kind } => {
                write!(f, "configuration {operation} failed ({kind:?})")
            }
        }
    }
}
impl std::error::Error for ConfigError {}
impl From<ValidationError> for ConfigError {
    fn from(value: ValidationError) -> Self {
        Self::InvalidConfig(value)
    }
}
fn io_error(operation: &'static str, error: io::Error) -> ConfigError {
    ConfigError::Io {
        operation,
        kind: error.kind(),
    }
}

pub fn parse_toml(text: &str) -> Result<Config, ConfigError> {
    if text.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    let value: toml::Value = toml::from_str(text).map_err(|_| ConfigError::InvalidToml)?;
    super::secrets::decode(value)
}

/// 最多读取上限加一个检测字节；不依赖可能过期的文件 metadata。
pub fn read_config(reader: impl Read) -> Result<Config, ConfigError> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read", error))?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| ConfigError::InvalidUtf8)?;
    parse_toml(text)
}

pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    if !path.is_absolute() {
        return Err(ConfigError::AbsolutePathRequired);
    }
    let file = File::open(path).map_err(|error| io_error("open", error))?;
    read_config(file)
}

/// 显式持久化 API，不是脱敏导出。Plaintext 模式会包含所有秘密。
/// DPAPI 使用当前用户保护的内联版本化密文，失败绝不降级为明文。
pub fn to_persistence_toml(config: &Config) -> Result<String, ConfigError> {
    config.validate_for_save()?;
    serialize_value(&super::secrets::encode(config)?)
}

pub(super) fn serialize_value(value: &toml::Value) -> Result<String, ConfigError> {
    let text = toml::to_string_pretty(value).map_err(|_| ConfigError::Serialization)?;
    if text.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    Ok(text)
}

/// 仅新建，不创建父目录，不覆盖已有文件（含 create_new 竞争）。
/// 不是原子保存：写入/同步失败可能留下不完整的新文件，需宿主提示恢复，不能报告成功。
/// 需要原子新建或更新与备份时，使用 save_config_atomic。
pub fn create_new_config(path: &Path, config: &Config) -> Result<(), ConfigError> {
    if !path.is_absolute() {
        return Err(ConfigError::AbsolutePathRequired);
    }
    let text = to_persistence_toml(config)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error("create", error))?;
    file.write_all(text.as_bytes())
        .map_err(|error| io_error("write", error))?;
    file.sync_all().map_err(|error| io_error("sync", error))
}
