#![cfg(windows)]

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use ai_relay::config::*;

// Compile and exercise paths without requiring the GUI owner's mod.rs edit.
mod config {
    pub use ai_relay::config::*;
}
#[path = "../src/windows/paths.rs"]
mod paths;

struct TemporaryDirectory(PathBuf);
impl TemporaryDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..128 {
            let path = std::env::temp_dir().join(format!(
                "ai-relay-persistence-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            assert!(path.is_absolute());
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("temporary directory: {e}"),
            }
        }
        panic!("temporary directory allocation exhausted")
    }
    fn config(&self) -> PathBuf {
        self.0.join("config.toml")
    }
    fn backup(&self) -> PathBuf {
        self.0.join("config.toml.bak")
    }
}
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn changed(port: u16) -> Config {
    let mut config = Config::default();
    config.listen.port = port;
    config
}

#[test]
fn create_and_replace_keep_exact_recent_backup() {
    let dir = TemporaryDirectory::new();
    let first = Config::default();
    save_config_atomic(&dir.config(), &first).unwrap();
    assert_eq!(load_config(&dir.config()).unwrap(), first);
    assert!(!dir.backup().exists());
    let original = fs::read(dir.config()).unwrap();
    save_config_atomic(&dir.config(), &changed(8081)).unwrap();
    assert_eq!(fs::read(dir.backup()).unwrap(), original);
    let second = fs::read(dir.config()).unwrap();
    save_config_atomic(&dir.config(), &changed(8082)).unwrap();
    assert_eq!(fs::read(dir.backup()).unwrap(), second);
    assert_eq!(load_config(&dir.config()).unwrap(), changed(8082));
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn checked_save_rejects_unloaded_external_edits_without_touching_backup() {
    let dir = TemporaryDirectory::new();
    let baseline = Config::default();
    save_config_atomic_checked(&dir.config(), &baseline, None).unwrap();
    save_config_atomic(&dir.config(), &changed(8081)).unwrap();
    let external = fs::read(dir.config()).unwrap();
    let backup = fs::read(dir.backup()).unwrap();
    assert_eq!(
        save_config_atomic_checked(&dir.config(), &changed(8082), Some(&baseline)),
        Err(ConfigError::ExternalModification)
    );
    assert_eq!(fs::read(dir.config()).unwrap(), external);
    assert_eq!(fs::read(dir.backup()).unwrap(), backup);
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    let reloaded = load_config(&dir.config()).unwrap();
    save_config_atomic_checked(&dir.config(), &changed(8082), Some(&reloaded)).unwrap();
    assert_eq!(fs::read(dir.backup()).unwrap(), external);
}

#[test]
fn checked_save_distinguishes_missing_from_unloaded_and_deleted_configs() {
    let dir = TemporaryDirectory::new();
    let baseline = Config::default();
    save_config_atomic(&dir.config(), &baseline).unwrap();
    assert_eq!(
        save_config_atomic_checked(&dir.config(), &changed(8081), None),
        Err(ConfigError::ExternalModification)
    );
    assert!(!dir.backup().exists());
    fs::remove_file(dir.config()).unwrap();
    assert_eq!(
        save_config_atomic_checked(&dir.config(), &changed(8081), Some(&baseline)),
        Err(ConfigError::ExternalModification)
    );
    assert!(!dir.config().exists());
    save_config_atomic_checked(&dir.config(), &baseline, None).unwrap();
    // Semantic comparison deliberately permits formatting-only external edits.
    let mut text = fs::read_to_string(dir.config()).unwrap();
    text.push_str("\n# external formatting edit\n");
    fs::write(dir.config(), &text).unwrap();
    save_config_atomic_checked(&dir.config(), &changed(8081), Some(&baseline)).unwrap();
    assert_eq!(fs::read_to_string(dir.backup()).unwrap(), text);
}

#[test]
fn application_backup_slot_is_replaceable_without_parsing_its_contents() {
    let dir = TemporaryDirectory::new();
    save_config_atomic(&dir.config(), &Config::default()).unwrap();
    fs::write(dir.backup(), b"previous application backup").unwrap();
    save_config_atomic(&dir.config(), &changed(8081)).unwrap();
    assert_eq!(load_config(&dir.backup()).unwrap(), Config::default());
}

#[test]
fn malformed_and_future_configs_are_never_overwritten() {
    let dir = TemporaryDirectory::new();
    for content in ["broken synthetic-secret [", "schema_version = 999"] {
        fs::write(dir.config(), content).unwrap();
        fs::write(dir.backup(), b"keep backup").unwrap();
        assert!(save_config_atomic(&dir.config(), &Config::default()).is_err());
        assert_eq!(fs::read_to_string(dir.config()).unwrap(), content);
        assert_eq!(fs::read(dir.backup()).unwrap(), b"keep backup");
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }
}

#[test]
fn invalid_input_is_refused_and_dpapi_secrets_round_trip() {
    let dir = TemporaryDirectory::new();
    assert_eq!(
        save_config_atomic(Path::new("relative.toml"), &Config::default()),
        Err(ConfigError::AbsolutePathRequired)
    );
    assert!(save_config_atomic(&dir.config(), &changed(0)).is_err());
    let mut config = Config::default();
    config.keys.upstream_api_key = SecretString::new("synthetic-upstream-secret");
    save_config_atomic(&dir.config(), &config).unwrap();
    assert_eq!(load_config(&dir.config()).unwrap(), config);
    config.keys.upstream_api_key = SecretString::default();
    config.access.client_token = SecretString::new("synthetic-client-secret");
    save_config_atomic(&dir.config(), &config).unwrap();
    assert_eq!(load_config(&dir.config()).unwrap(), config);
    assert!(save_config_atomic(&dir.0.join("missing/config.toml"), &Config::default()).is_err());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn nonregular_backup_and_destination_are_refused() {
    let dir = TemporaryDirectory::new();
    save_config_atomic(&dir.config(), &Config::default()).unwrap();
    let before = fs::read(dir.config()).unwrap();
    fs::create_dir(dir.backup()).unwrap();
    assert!(save_config_atomic(&dir.config(), &changed(8081)).is_err());
    assert_eq!(fs::read(dir.config()).unwrap(), before);
    fs::remove_file(dir.config()).unwrap();
    fs::create_dir(dir.config()).unwrap();
    assert!(save_config_atomic(&dir.config(), &Config::default()).is_err());
    assert!(dir.config().is_dir());
}

#[test]
fn replace_failure_preserves_config_and_previous_backup() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
    let dir = TemporaryDirectory::new();
    save_config_atomic(&dir.config(), &Config::default()).unwrap();
    fs::write(dir.backup(), b"previous backup").unwrap();
    let before = fs::read(dir.config()).unwrap();
    // Allow validation reads but deny the delete sharing needed by ReplaceFileW.
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(dir.config())
        .unwrap();
    assert!(save_config_atomic(&dir.config(), &changed(8081)).is_err());
    assert_eq!(fs::read(dir.config()).unwrap(), before);
    assert_eq!(fs::read(dir.backup()).unwrap(), b"previous backup");
    drop(lock);
    save_config_atomic(&dir.config(), &changed(8082)).unwrap();
    assert_eq!(load_config(&dir.backup()).unwrap(), Config::default());
}

#[test]
fn backup_publication_failure_reports_commit_and_keeps_recovery() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
    let dir = TemporaryDirectory::new();
    save_config_atomic(&dir.config(), &Config::default()).unwrap();
    fs::write(dir.backup(), b"older backup").unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(dir.backup())
        .unwrap();
    let error = save_config_atomic(&dir.config(), &changed(8081)).unwrap_err();
    let ConfigError::BackupPublicationFailed { recovery_path, .. } = error else {
        panic!("expected committed-but-backup-failed: {error}");
    };
    assert_eq!(load_config(&dir.config()).unwrap(), changed(8081));
    assert_eq!(load_config(&recovery_path).unwrap(), Config::default());
    assert_eq!(fs::read(dir.backup()).unwrap(), b"older backup");
    drop(lock);
}

#[test]
fn plaintext_is_explicit_and_unicode_paths_round_trip() {
    let dir = TemporaryDirectory::new();
    let path = dir.0.join("配置.toml");
    let mut config = Config::default();
    config.storage.secrets = SecretStorage::Plaintext;
    config.keys.upstream_api_key = SecretString::new("synthetic-secret");
    save_config_atomic(&path, &config).unwrap();
    assert_eq!(load_config(&path).unwrap(), config);
    save_config_atomic(&path, &Config::default()).unwrap();
    assert_eq!(load_config(&dir.0.join("配置.toml.bak")).unwrap(), config);
}

fn secret_config() -> Config {
    let mut c = Config::default();
    c.keys.upstream_api_key = SecretString::new("synthetic-upstream-only");
    c.keys.injection_mode = InjectionMode::Always;
    c.access.client_token = SecretString::new("synthetic-client-only");
    c.access.client_token_enabled = true;
    c
}

#[test]
fn protected_envelope_is_bounded_strict_and_never_exports_secrets() {
    let c = secret_config();
    let text = to_persistence_toml(&c).unwrap();
    assert!(!text.contains("synthetic-upstream-only"));
    assert!(!text.contains("synthetic-client-only"));
    assert_eq!(parse_toml(&text).unwrap(), c);
    assert_ne!(text, to_persistence_toml(&c).unwrap());
    let value: toml::Value = toml::from_str(&text).unwrap();
    for (field, replacement) in [
        ("version", toml::Value::Integer(2)),
        ("ciphertext", "00".into()),
        ("ciphertext", "zz".into()),
        ("ciphertext", "0".into()),
        ("ciphertext", "ab".repeat(65537).into()),
    ] {
        let mut bad = value.clone();
        bad["protected_secrets"][field] = replacement;
        let error = parse_toml(&toml::to_string(&bad).unwrap()).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("synthetic"));
        if field == "ciphertext" && bad["protected_secrets"][field].as_str() == Some("00") {
            assert!(
                matches!(error, ConfigError::Io { operation, .. } if operation.starts_with("DPAPI unprotect"))
            );
        }
    }
    for field in ["upstream_api_key", "client_token"] {
        let mut bad = value.clone();
        let group = if field == "client_token" {
            "access"
        } else {
            "keys"
        };
        bad[group]
            .as_table_mut()
            .unwrap()
            .insert(field.into(), "".into());
        assert_eq!(
            parse_toml(&toml::to_string(&bad).unwrap()),
            Err(ConfigError::CONFLICTING_SECRETS)
        );
    }
    let mut bad = value.clone();
    bad["storage"]["secrets"] = "plaintext".into();
    assert_eq!(
        parse_toml(&toml::to_string(&bad).unwrap()),
        Err(ConfigError::CONFLICTING_SECRETS)
    );
    assert_eq!(
        parse_toml("schema_version=1\n[keys]\nupstream_api_key='synthetic-forbidden'"),
        Err(ConfigError::PLAINTEXT_IN_DPAPI)
    );
    let mut tampered = value.clone();
    let mut hex = tampered["protected_secrets"]["ciphertext"]
        .as_str()
        .unwrap()
        .to_string();
    let end = hex.len();
    let replacement = if &hex[end - 2..] == "00" { "ff" } else { "00" };
    hex.replace_range(end - 2.., replacement);
    tampered["protected_secrets"]["ciphertext"] = hex.into();
    assert!(parse_toml(&toml::to_string(&tampered).unwrap()).is_err());
    let exported = export_redacted_config(&c).unwrap();
    let mut plain = c.clone();
    plain.storage.secrets = SecretStorage::Plaintext;
    let plain_export = export_redacted_config(&plain).unwrap();
    assert!(!plain_export.contains("synthetic"));
    assert!(!plain_export.contains("ciphertext"));
    assert!(
        import_config(&plain_export)
            .unwrap()
            .keys
            .upstream_api_key
            .is_empty()
    );
    for forbidden in [
        "synthetic",
        "protected_secrets",
        "ciphertext",
        "upstream_api_key",
        "client_token =",
    ] {
        assert!(!exported.contains(forbidden));
    }
    let draft = import_config(&exported).unwrap();
    assert!(draft.keys.upstream_api_key.is_empty());
    assert!(draft.access.client_token.is_empty());
    assert!(!draft.access.client_token_enabled);
    assert_eq!(draft.keys.injection_mode, InjectionMode::Off);
    let mut oversized = c;
    oversized.keys.upstream_api_key = SecretString::new("x".repeat(MAX_SECRET_BYTES + 1));
    assert_eq!(
        to_persistence_toml(&oversized),
        Err(ConfigError::SECRET_TOO_LARGE)
    );
    oversized.storage.secrets = SecretStorage::Plaintext;
    assert_eq!(
        to_persistence_toml(&oversized),
        Err(ConfigError::SECRET_TOO_LARGE)
    );
}

#[test]
fn encrypted_backups_restore_only_after_explicit_matching_confirmation() {
    let dir = TemporaryDirectory::new();
    let original = secret_config();
    save_config_atomic(&dir.config(), &original).unwrap();
    let baseline = load_config(&dir.config()).unwrap();
    let mut updated = original.clone();
    updated.access.client_token = SecretString::new("synthetic-updated-token");
    save_config_atomic_checked(&dir.config(), &updated, Some(&baseline)).unwrap();
    assert_eq!(load_config(&dir.backup()).unwrap(), original);
    assert_eq!(
        save_config_atomic_checked(&dir.config(), &original, Some(&baseline)),
        Err(ConfigError::ExternalModification)
    );
    let backup_bytes = fs::read(dir.backup()).unwrap();
    fs::write(dir.config(), b"damaged synthetic-private").unwrap();
    let confirmation = inspect_restore_target(&dir.config()).unwrap().unwrap();
    assert!(!format!("{confirmation:?}").contains("synthetic"));
    assert!(save_config_atomic(&dir.config(), &original).is_err());
    assert_eq!(
        restore_backup_checked(&dir.config(), &dir.backup(), None, &original),
        Err(ConfigError::ExternalModification)
    );
    assert_eq!(
        restore_backup_checked(&dir.config(), &dir.backup(), Some(&confirmation), &updated),
        Err(ConfigError::ExternalModification)
    );
    fs::write(dir.config(), b"another external edit").unwrap();
    assert_eq!(
        restore_backup_checked(&dir.config(), &dir.backup(), Some(&confirmation), &original),
        Err(ConfigError::ExternalModification)
    );
    let confirmation = inspect_restore_target(&dir.config()).unwrap().unwrap();
    let retained =
        restore_backup_checked(&dir.config(), &dir.backup(), Some(&confirmation), &original)
            .unwrap()
            .unwrap();
    assert_eq!(fs::read(retained).unwrap(), b"another external edit");
    assert_eq!(fs::read(dir.backup()).unwrap(), backup_bytes);
    assert_eq!(load_config(&dir.config()).unwrap(), original);
    assert!(restore_backup_checked(&dir.config(), &dir.config(), None, &original).is_err());
    fs::remove_file(dir.config()).unwrap();
    assert_eq!(
        restore_backup_checked(&dir.config(), &dir.backup(), None, &original).unwrap(),
        None
    );
    assert_eq!(load_config(&dir.config()).unwrap(), original);
}

#[test]
fn encrypted_replacement_failure_keeps_decryptable_original_and_backup() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
    let dir = TemporaryDirectory::new();
    let first = secret_config();
    save_config_atomic(&dir.config(), &first).unwrap();
    let mut second = first.clone();
    second.keys.upstream_api_key = SecretString::new("synthetic-replacement");
    save_config_atomic(&dir.config(), &second).unwrap();
    let before = fs::read(dir.config()).unwrap();
    let backup = fs::read(dir.backup()).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(dir.config())
        .unwrap();
    assert!(save_config_atomic_checked(&dir.config(), &first, Some(&second)).is_err());
    assert_eq!(fs::read(dir.config()).unwrap(), before);
    assert_eq!(fs::read(dir.backup()).unwrap(), backup);
    assert_eq!(load_config(&dir.config()).unwrap(), second);
    assert_eq!(load_config(&dir.backup()).unwrap(), first);
    drop(lock);
}

#[test]
fn safe_import_never_overwrites_unseen_or_damaged_destination() {
    let dir = TemporaryDirectory::new();
    let c = secret_config();
    let text = to_persistence_toml(&c).unwrap();
    assert_eq!(
        import_config_atomic_checked(&dir.config(), &text, None).unwrap(),
        c
    );
    assert_eq!(
        import_config_atomic_checked(&dir.config(), &text, None),
        Err(ConfigError::ExternalModification)
    );
    let before = fs::read(dir.config()).unwrap();
    assert!(import_config_atomic_checked(&dir.config(), "schema_version=999", Some(&c)).is_err());
    assert_eq!(fs::read(dir.config()).unwrap(), before);
    fs::write(dir.config(), b"corrupt").unwrap();
    assert!(import_config_atomic_checked(&dir.config(), &text, Some(&c)).is_err());
    assert_eq!(fs::read(dir.config()).unwrap(), b"corrupt");
}

#[test]
fn explicit_discovery_only_probes_temporary_directory() {
    let dir = TemporaryDirectory::new();
    let selection = paths::discover_directory(Some(&dir.0)).unwrap();
    assert_eq!(selection.directory, dir.0);
    assert_eq!(selection.source, DirectorySource::Explicit);
    assert!(selection.needs_initialization);
    fs::write(dir.config(), b"invalid config").unwrap();
    assert!(
        !paths::discover_directory(Some(&dir.0))
            .unwrap()
            .needs_initialization
    );
    assert!(paths::discover_directory(Some(Path::new("relative"))).is_err());
    assert!(paths::discover_directory(Some(Path::new("C:relative"))).is_err());
    // Only resolve these paths: never probe/read/write the user's real config.
    let standard = paths::standard_data_directory().unwrap();
    assert!(standard.is_absolute());
    assert_eq!(standard.file_name().unwrap(), "AIRelay");
    assert_eq!(
        paths::portable_data_directory().unwrap(),
        std::env::current_exe().unwrap().parent().unwrap()
    );
}
