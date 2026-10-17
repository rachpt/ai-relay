//! Blocking transfer APIs. Confirmation belongs to the caller; no UI is implied.
use super::{Config, ConfigError, MAX_CONFIG_BYTES};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

/// Exact, bounded snapshot for an explicit restore confirmation. No Debug or public
/// byte accessor: corrupt configuration bytes can themselves contain plaintext keys.
/// None in restore means the user observed a missing file, never "ignore current".
#[derive(Clone, PartialEq, Eq)]
pub struct RestoreBaseline(Vec<u8>);
impl std::fmt::Debug for RestoreBaseline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RestoreBaseline([REDACTED])")
    }
}

pub fn inspect_restore_target(path: &Path) -> Result<Option<RestoreBaseline>, ConfigError> {
    if !path.is_absolute() {
        return Err(ConfigError::AbsolutePathRequired);
    }
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error("inspect restore target", e)),
        Ok(metadata) => {
            #[cfg(windows)]
            let reparse = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x400 != 0
            };
            #[cfg(not(windows))]
            let reparse = metadata.file_type().is_symlink();
            if !metadata.is_file() || reparse {
                return Err(io_error(
                    "check restore target",
                    io::ErrorKind::InvalidInput.into(),
                ));
            }
        }
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| io_error("open restore target", e))?
        .take((MAX_CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error("read restore target", e))?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    Ok(Some(RestoreBaseline(bytes)))
}
fn io_error(operation: &'static str, error: io::Error) -> ConfigError {
    ConfigError::Io {
        operation,
        kind: error.kind(),
    }
}

/// Pure preparation: validates/decrypts the supported format, never writes a file,
/// runs legacy scripts, merges credentials or silently substitutes empty secrets.
/// Use export_redacted_config for sharing; this API accepts explicit plaintext mode.
pub fn import_config(text: &str) -> Result<Config, ConfigError> {
    super::parse_toml(text)
}

/// Writes an already prepared import only against the normal semantic baseline.
/// Damaged destinations remain refused; explicit restore is a separate operation.
pub fn import_config_atomic_checked(
    path: &Path,
    text: &str,
    baseline: Option<&Config>,
) -> Result<Config, ConfigError> {
    let config = import_config(text)?;
    super::save_config_atomic_checked(path, &config, baseline)?;
    Ok(config)
}

/// Explicitly restores the caller-selected, validated backup after confirmation of
/// BOTH paths and snapshots. The destination may be corrupt; unlike normal save it
/// is retained at the returned recovery path (if present), never discarded or put
/// over the selected backup. Neither path may alias the other. The caller must show
/// the returned path. Same-user concurrent hostile races still need coordination.
/// The source is re-read/decrypted and checked against backup_baseline before writing.
pub fn restore_backup_checked(
    path: &Path,
    backup_path: &Path,
    baseline: Option<&RestoreBaseline>,
    backup_baseline: &Config,
) -> Result<Option<PathBuf>, ConfigError> {
    if !path.is_absolute() || !backup_path.is_absolute() {
        return Err(ConfigError::AbsolutePathRequired);
    }
    // Canonicalize existing paths to reject ordinary relative-component aliases.
    // Reparse-point files are rejected below. Callers must use trusted application
    // paths and coordinate concurrent writers, as with normal atomic saves.
    if path == backup_path {
        return Err(io_error(
            "distinct restore paths required",
            io::ErrorKind::InvalidInput.into(),
        ));
    }
    let source = std::fs::canonicalize(backup_path).map_err(|e| io_error("resolve backup", e))?;
    if std::fs::canonicalize(path).is_ok_and(|target| target == source) {
        return Err(io_error(
            "distinct restore paths required",
            io::ErrorKind::InvalidInput.into(),
        ));
    }
    // Also reject reparse/nonregular backup files before loading.
    let source_snapshot = inspect_restore_target(backup_path)?
        .ok_or_else(|| io_error("open backup", io::ErrorKind::NotFound.into()))?;
    let backup = super::read_config(source_snapshot.0.as_slice())?;
    if &backup != backup_baseline || inspect_restore_target(path)?.as_ref() != baseline {
        return Err(ConfigError::ExternalModification);
    }
    #[cfg(windows)]
    {
        super::persistence::restore_confirmed(path, &backup)
    }
    #[cfg(not(windows))]
    {
        Err(io_error(
            "restore requires Windows",
            io::ErrorKind::Unsupported.into(),
        ))
    }
}
