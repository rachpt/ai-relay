//! Blocking persistence. Call off the GUI thread; the data directory must be trusted.
//! Reparse points are rejected, but this is not a sandbox against a process with the
//! same user's write access racing directory entries. No migration/overwrite consent
//! is implied by this API: an existing config must already be valid.
#[cfg(windows)]
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use std::{io, path::Path};

#[cfg(windows)]
use super::load_config;
use super::{Config, ConfigError, to_persistence_toml};

fn error(operation: &'static str, error: io::Error) -> ConfigError {
    ConfigError::Io {
        operation,
        kind: error.kind(),
    }
}

#[cfg(windows)]
fn regular_file(path: &Path) -> Result<bool, ConfigError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[cfg(windows)]
            let reparse = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x400 != 0
            };
            #[cfg(not(windows))]
            let reparse = metadata.file_type().is_symlink();
            if !metadata.is_file() || reparse {
                return Err(error(
                    "check regular file",
                    io::ErrorKind::InvalidInput.into(),
                ));
            }
            Ok(true)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(error("inspect file", e)),
    }
}

#[cfg(windows)]
struct Temporary {
    path: PathBuf,
    remove: bool,
}
#[cfg(windows)]
impl Temporary {
    fn create(parent: &Path) -> Result<(Self, File), ConfigError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..128 {
            let path = parent.join(format!(
                ".ai-relay-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Self { path, remove: true }, file)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(error("create temporary file", e)),
            }
        }
        Err(error(
            "allocate temporary file",
            io::ErrorKind::AlreadyExists.into(),
        ))
    }
}
#[cfg(windows)]
impl Drop for Temporary {
    fn drop(&mut self) {
        if self.remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Atomically saves a validated config without creating parent directories.
/// Existing valid files get a recent backup at `<filename>.bak` (an application-owned
/// slot, replaceable without another prompt). Non-regular/reparse backup slots are
/// refused. Existing malformed/unknown-version configs are never overwritten.
///
/// Windows only: first creation never replaces a racing destination. Updates use
/// ReplaceFileW with a separate recovery slot, leaving the previous .bak untouched
/// until replacement succeeds. A failed replacement retains recovery files when
/// Windows may have moved the original; it never deletes a possibly unique copy.
/// `BackupPublicationFailed` means the config WAS saved but the previous config is
/// in `recovery_path`, not necessarily in .bak. Do not retry as if nothing changed.
/// Use `save_config_atomic_checked` to reject normal external edits since loading.
/// Concurrent external editors/writers still require coordination by the caller.
#[cfg(windows)]
pub fn save_config_atomic(path: &Path, config: &Config) -> Result<(), ConfigError> {
    save_inner(path, config, false).map(|_| ())
}

#[cfg(windows)]
fn save_inner(
    path: &Path,
    config: &Config,
    restoring: bool,
) -> Result<Option<PathBuf>, ConfigError> {
    if !path.is_absolute() {
        return Err(ConfigError::AbsolutePathRequired);
    }
    let text = to_persistence_toml(config)?;
    let parent = path.parent().ok_or(ConfigError::AbsolutePathRequired)?;
    let existing = regular_file(path)?;
    let mut name = path
        .file_name()
        .ok_or(ConfigError::AbsolutePathRequired)?
        .to_os_string();
    name.push(".bak");
    let backup = parent.join(name);
    regular_file(&backup)?;
    if existing && !restoring {
        load_config(path)?;
    }
    let (temporary, mut file) = Temporary::create(parent)?;
    file.write_all(text.as_bytes())
        .map_err(|e| error("write temporary file", e))?;
    file.sync_all()
        .map_err(|e| error("sync temporary file", e))?;
    drop(file);
    if !existing {
        return move_file(&temporary.path, path, false)
            .map(|()| None)
            .map_err(|e| error("publish new file", e));
    }

    // Reserve a unique name so ReplaceFileW can only overwrite our own empty slot.
    let (mut recovery, file) = Temporary::create(parent)?;
    file.sync_all()
        .map_err(|e| error("sync recovery slot", e))?;
    drop(file);
    if let Err(e) = replace_file(path, &temporary.path, &recovery.path) {
        // ReplaceFileW documents partial failure states. Keep both temporary names
        // rather than deleting a copy which might now be the only old configuration.
        recovery.remove = false;
        let mut temporary = temporary;
        temporary.remove = false;
        if matches!(fs::symlink_metadata(path), Err(ref e) if e.kind() == io::ErrorKind::NotFound)
            && load_config(&recovery.path).is_ok()
        {
            // Never overwrite a destination created while attempting recovery.
            let _ = move_file(&recovery.path, path, false);
        }
        return Err(ConfigError::AtomicReplacementFailed {
            replacement_path: temporary.path.clone(),
            recovery_path: recovery.path.clone(),
            kind: e.kind(),
        });
    }
    // The original is now in recovery. Never delete it if backup publication fails.
    recovery.remove = false;
    if restoring {
        // Keep the selected valid .bak and retain the replaced (possibly corrupt)
        // file separately. Recovery is never an implicit discard of user data.
        return Ok(Some(recovery.path.clone()));
    }
    if let Err(e) = regular_file(&backup) {
        return Err(ConfigError::BackupPublicationFailed {
            recovery_path: recovery.path.clone(),
            kind: match e {
                ConfigError::Io { kind, .. } => kind,
                _ => io::ErrorKind::Other,
            },
        });
    }
    move_file(&recovery.path, &backup, true)
        .map(|()| None)
        .map_err(|e| ConfigError::BackupPublicationFailed {
            recovery_path: recovery.path.clone(),
            kind: e.kind(),
        })
}

#[cfg(windows)]
pub(super) fn restore_confirmed(
    path: &Path,
    config: &Config,
) -> Result<Option<PathBuf>, ConfigError> {
    save_inner(path, config, true)
}

/// Saves only if the on-disk config still matches the caller's loaded baseline.
/// `None` records a missing file, not permission to overwrite an unseen config.
/// Comparison uses the parsed config, so formatting-only edits are allowed. This
/// detects ordinary external edits, not hostile same-user races during replacement.
/// A committed `BackupPublicationFailed` requires advancing the caller's baseline.
pub fn save_config_atomic_checked(
    path: &Path,
    config: &Config,
    baseline: Option<&Config>,
) -> Result<(), ConfigError> {
    if !path.is_absolute() {
        return Err(ConfigError::AbsolutePathRequired);
    }
    match std::fs::symlink_metadata(path) {
        Ok(_) => {
            let current = super::load_config(path)?;
            if baseline != Some(&current) {
                return Err(ConfigError::ExternalModification);
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if baseline.is_some() {
                return Err(ConfigError::ExternalModification);
            }
        }
        Err(e) => return Err(error("inspect baseline", e)),
    }
    save_config_atomic(path, config)
}

/// No portable rename fallback: cross-platform builds do not claim Windows durability.
#[cfg(not(windows))]
pub fn save_config_atomic(path: &Path, config: &Config) -> Result<(), ConfigError> {
    if !path.is_absolute() {
        return Err(ConfigError::AbsolutePathRequired);
    }
    to_persistence_toml(config)?;
    Err(error(
        "atomic save requires Windows",
        io::ErrorKind::Unsupported.into(),
    ))
}

#[cfg(windows)]
fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    wide.push(0);
    Ok(wide)
}

#[cfg(windows)]
fn move_file(from: &Path, to: &Path, replace: bool) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let from = wide_path(from)?;
    let to = wide_path(to)?;
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    // SAFETY: Both buffers are NUL-terminated and live throughout the call.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn replace_file(path: &Path, replacement: &Path, backup: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
    let path = wide_path(path)?;
    let replacement = wide_path(replacement)?;
    let backup = wide_path(backup)?;
    // SAFETY: All paths are live NUL-terminated buffers; reserved arguments are null.
    // REPLACEFILE_WRITE_THROUGH is unsupported by Windows, so sync the input first.
    if unsafe {
        ReplaceFileW(
            path.as_ptr(),
            replacement.as_ptr(),
            backup.as_ptr(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
