//! Versioned on-disk envelope. Never attach this representation to runtime Config.
use super::{Config, ConfigError, InjectionMode, SecretStorage, SecretString};

pub const MAX_SECRET_BYTES: usize = 16 * 1024;
const MAX_CIPHERTEXT_BYTES: usize = 64 * 1024;
const MAGIC: &[u8] = b"AI Relay secrets\0\x01";

pub(super) fn check_lengths(config: &Config) -> Result<(), ConfigError> {
    if config.keys.upstream_api_key.expose_secret().len() > MAX_SECRET_BYTES
        || config.access.client_token.expose_secret().len() > MAX_SECRET_BYTES
    {
        return Err(ConfigError::SECRET_TOO_LARGE);
    }
    Ok(())
}

// Volatile clearing prevents these temporary plaintext buffers from being elided.
// Runtime SecretString ownership and OS paging are outside this buffer's guarantee.
struct Sensitive(Vec<u8>);
impl Drop for Sensitive {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            // SAFETY: byte is a live, uniquely borrowed byte in this allocation.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
    }
}

pub(super) fn encode(config: &Config) -> Result<toml::Value, ConfigError> {
    check_lengths(config)?;
    let mut safe = config.clone();
    if config.storage.secrets == SecretStorage::Dpapi {
        safe.keys.upstream_api_key = SecretString::default();
        safe.access.client_token = SecretString::default();
    }
    let mut value = toml::Value::try_from(&safe).map_err(|_| ConfigError::Serialization)?;
    if config.storage.secrets == SecretStorage::Dpapi
        && (!config.keys.upstream_api_key.is_empty() || !config.access.client_token.is_empty())
    {
        let mut plaintext = Sensitive(MAGIC.to_vec());
        for secret in [&config.keys.upstream_api_key, &config.access.client_token] {
            let bytes = secret.expose_secret().as_bytes();
            plaintext
                .0
                .extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            plaintext.0.extend_from_slice(bytes);
        }
        let ciphertext = protect(&plaintext.0)?;
        let mut hex = String::with_capacity(ciphertext.len() * 2);
        use std::fmt::Write;
        for byte in ciphertext {
            write!(&mut hex, "{byte:02x}").map_err(|_| ConfigError::Serialization)?;
        }
        let table = value.as_table_mut().ok_or(ConfigError::Serialization)?;
        for (group, field) in [("keys", "upstream_api_key"), ("access", "client_token")] {
            table
                .get_mut(group)
                .and_then(toml::Value::as_table_mut)
                .ok_or(ConfigError::Serialization)?
                .remove(field);
        }
        let mut blob = toml::Table::new();
        blob.insert("version".into(), 1.into());
        blob.insert("ciphertext".into(), hex.into());
        table.insert("protected_secrets".into(), toml::Value::Table(blob));
    }
    Ok(value)
}

pub(super) fn decode(mut value: toml::Value) -> Result<Config, ConfigError> {
    let table = value.as_table_mut().ok_or(ConfigError::InvalidToml)?;
    let blob = table.remove("protected_secrets");
    if blob.is_some() {
        for (group, field) in [("keys", "upstream_api_key"), ("access", "client_token")] {
            if table
                .get(group)
                .and_then(toml::Value::as_table)
                .is_some_and(|group| group.contains_key(field))
            {
                return Err(ConfigError::CONFLICTING_SECRETS);
            }
        }
    }
    let mut config: Config = value.try_into().map_err(|_| ConfigError::InvalidToml)?;
    if let Some(blob) = blob {
        if config.storage.secrets != SecretStorage::Dpapi {
            return Err(ConfigError::CONFLICTING_SECRETS);
        }
        let blob = blob
            .as_table()
            .ok_or(ConfigError::INVALID_PROTECTED_SECRETS)?;
        if blob.len() != 2 || blob.get("version").and_then(toml::Value::as_integer) != Some(1) {
            return Err(ConfigError::INVALID_PROTECTED_SECRETS);
        }
        let hex = blob
            .get("ciphertext")
            .and_then(toml::Value::as_str)
            .ok_or(ConfigError::INVALID_PROTECTED_SECRETS)?;
        if hex.is_empty() || hex.len() > MAX_CIPHERTEXT_BYTES * 2 || hex.len() % 2 != 0 {
            return Err(ConfigError::INVALID_PROTECTED_SECRETS);
        }
        let mut ciphertext = Vec::with_capacity(hex.len() / 2);
        for pair in hex.as_bytes().as_chunks::<2>().0 {
            let digit = |b: u8| match b {
                b'0'..=b'9' => Ok(b - b'0'),
                b'a'..=b'f' => Ok(b - b'a' + 10),
                _ => Err(ConfigError::INVALID_PROTECTED_SECRETS),
            };
            ciphertext.push(digit(pair[0])? * 16 + digit(pair[1])?);
        }
        let plaintext = Sensitive(unprotect(&ciphertext)?);
        let mut remaining = plaintext
            .0
            .strip_prefix(MAGIC)
            .ok_or(ConfigError::INVALID_PROTECTED_SECRETS)?;
        let mut next = || -> Result<SecretString, ConfigError> {
            let length = remaining
                .get(..4)
                .ok_or(ConfigError::INVALID_PROTECTED_SECRETS)?;
            let length = u32::from_le_bytes([length[0], length[1], length[2], length[3]]) as usize;
            if length > MAX_SECRET_BYTES {
                return Err(ConfigError::SECRET_TOO_LARGE);
            }
            remaining = &remaining[4..];
            let text = std::str::from_utf8(
                remaining
                    .get(..length)
                    .ok_or(ConfigError::INVALID_PROTECTED_SECRETS)?,
            )
            .map_err(|_| ConfigError::INVALID_PROTECTED_SECRETS)?;
            remaining = &remaining[length..];
            Ok(SecretString::new(text))
        };
        config.keys.upstream_api_key = next()?;
        config.access.client_token = next()?;
        if !remaining.is_empty() {
            return Err(ConfigError::INVALID_PROTECTED_SECRETS);
        }
    } else if config.storage.secrets == SecretStorage::Dpapi
        && (!config.keys.upstream_api_key.is_empty() || !config.access.client_token.is_empty())
    {
        return Err(ConfigError::PLAINTEXT_IN_DPAPI);
    }
    check_lengths(&config)?;
    config.validate_for_save()?;
    Ok(config)
}

/// Safe default export: excludes plaintext AND DPAPI ciphertext. Credential-dependent
/// switches are disabled so the result is a valid draft, not an authenticated setup.
/// This is not a general scrubber for secrets placed in unrelated free-text fields.
pub fn export_redacted_config(config: &Config) -> Result<String, ConfigError> {
    config.validate_for_save()?;
    check_lengths(config)?;
    let mut safe = config.clone();
    safe.keys.upstream_api_key = SecretString::default();
    safe.keys.injection_mode = InjectionMode::Off;
    safe.access.client_token = SecretString::default();
    safe.access.client_token_enabled = false;
    let mut value = toml::Value::try_from(&safe).map_err(|_| ConfigError::Serialization)?;
    let table = value.as_table_mut().ok_or(ConfigError::Serialization)?;
    for (group, field) in [("keys", "upstream_api_key"), ("access", "client_token")] {
        table
            .get_mut(group)
            .and_then(toml::Value::as_table_mut)
            .ok_or(ConfigError::Serialization)?
            .remove(field);
    }
    super::file::serialize_value(&value)
}

#[cfg(not(windows))]
fn protect(_: &[u8]) -> Result<Vec<u8>, ConfigError> {
    Err(ConfigError::DPAPI_UNSUPPORTED)
}
#[cfg(not(windows))]
fn unprotect(_: &[u8]) -> Result<Vec<u8>, ConfigError> {
    Err(ConfigError::DPAPI_UNSUPPORTED)
}

#[cfg(windows)]
fn protect(input: &[u8]) -> Result<Vec<u8>, ConfigError> {
    windows::crypt(input, true)
}
#[cfg(windows)]
fn unprotect(input: &[u8]) -> Result<Vec<u8>, ConfigError> {
    windows::crypt(input, false)
}

#[cfg(windows)]
mod windows {
    use super::*;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };

    struct Output(CRYPT_INTEGER_BLOB);
    impl Drop for Output {
        fn drop(&mut self) {
            if !self.0.pbData.is_null() {
                // SAFETY: DPAPI returned a LocalAlloc allocation of cbData bytes,
                // owned exclusively here. Clear it before releasing with LocalFree.
                unsafe {
                    for index in 0..self.0.cbData as usize {
                        std::ptr::write_volatile(self.0.pbData.add(index), 0);
                    }
                    LocalFree(self.0.pbData.cast());
                }
            }
        }
    }

    pub(super) fn crypt(input: &[u8], protecting: bool) -> Result<Vec<u8>, ConfigError> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: input
                .len()
                .try_into()
                .map_err(|_| ConfigError::SECRET_TOO_LARGE)?,
            pbData: input.as_ptr().cast_mut(),
        };
        let mut output = Output(CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        });
        // SAFETY: Input is live for the synchronous call and DPAPI does not modify
        // it. Output is writable and subsequently owned by Output. Optional pointers
        // are null; UI is forbidden. No LOCAL_MACHINE flag: current-user protection.
        let success = unsafe {
            if protecting {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output.0,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output.0,
                )
            }
        };
        if success == 0 {
            return Err(ConfigError::Io {
                operation: if protecting {
                    "DPAPI protect; secrets not written; no plaintext fallback"
                } else {
                    "DPAPI unprotect; re-enter secrets or restore decryptable backup; no plaintext fallback"
                },
                kind: std::io::Error::last_os_error().kind(),
            });
        }
        if output.0.pbData.is_null()
            || output.0.cbData == 0
            || output.0.cbData as usize > MAX_CIPHERTEXT_BYTES
        {
            return Err(ConfigError::INVALID_PROTECTED_SECRETS);
        }
        // SAFETY: A successful DPAPI call initialized cbData readable bytes. The
        // RAII guard remains live while copying and clears/frees the OS allocation.
        Ok(
            unsafe { std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize) }
                .to_vec(),
        )
    }
}
