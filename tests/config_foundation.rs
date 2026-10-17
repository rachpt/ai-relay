use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use ai_relay::config::*;

fn configured() -> Config {
    let mut config = Config::default();
    config.upstream.url = "https://upstream.invalid/v1".into();
    config
}

#[test]
fn safe_defaults_cover_all_groups_and_require_setup() {
    let c = Config::default();
    assert_eq!(c.schema_version, 1);
    assert!(c.upstream.url.is_empty());
    assert_eq!(c.listen.address.to_string(), "127.0.0.1");
    assert_eq!(c.listen.port, 8080);
    assert!(c.proxy.auto_start && c.proxy.rewrite_forward_to_upstream);
    assert!(!c.proxy.forward_enabled && !c.proxy.connect_enabled);
    assert_eq!(c.proxy.connect_allowed_ports, [443]);
    assert_eq!(c.keys.injection_mode, InjectionMode::Off);
    assert!(c.keys.upstream_api_key.is_empty());
    assert!(c.access.whitelist.is_empty() && !c.access.client_token_enabled);
    assert!(c.access.client_token.is_empty());
    assert!(!c.cors.enabled && c.cors.allowed_origins.is_empty() && !c.cors.allow_credentials);
    assert_eq!(
        (
            c.network.connect_timeout_secs,
            c.network.response_header_timeout_secs,
            c.network.response_body_idle_timeout_secs,
            c.network.tunnel_idle_timeout_secs,
            c.network.shutdown_grace_secs,
            c.network.max_active_requests
        ),
        (10, 120, 300, 300, 10, 256)
    );
    assert_eq!(c.logging.level, LogLevel::Info);
    assert!(c.logging.access_log && c.logging.file_enabled);
    assert_eq!(c.logging.retention_days, 7);
    assert_eq!(
        (
            c.logging.max_file_bytes,
            c.logging.retained_files,
            c.logging.recent_entries
        ),
        (5 * 1024 * 1024, 3, 1000)
    );
    assert!(c.ui.close_to_tray && c.ui.remember_window_geometry);
    assert!(!c.system.login_auto_start && c.system.auto_start_background);
    assert_eq!(c.storage.mode, StorageMode::Standard);
    assert_eq!(c.storage.secrets, SecretStorage::Dpapi);
    assert!(c.validate_for_save().is_ok());
    assert!(c.validate_for_start().is_err());
    assert!(configured().validate_for_start().is_ok());
    assert_eq!(parse_toml("schema_version = 1").unwrap(), c);
    assert_eq!(parse_toml(&to_persistence_toml(&c).unwrap()).unwrap(), c);
}

#[test]
fn theme_is_backward_compatible_and_roundtrips() {
    for legacy in [
        "schema_version = 1",
        "schema_version = 1\n[ui]\nclose_to_tray = false",
    ] {
        assert_eq!(parse_toml(legacy).unwrap().ui.theme, ThemeMode::System);
    }
    for (name, mode) in [
        ("system", ThemeMode::System),
        ("light", ThemeMode::Light),
        ("dark", ThemeMode::Dark),
    ] {
        let config = parse_toml(&format!("schema_version = 1\n[ui]\ntheme = '{name}'")).unwrap();
        assert_eq!(config.ui.theme, mode);
        assert_eq!(
            parse_toml(&to_persistence_toml(&config).unwrap()).unwrap(),
            config
        );
    }
    assert!(parse_toml("schema_version = 1\n[ui]\ntheme = 'unknown'").is_err());
}

#[test]
fn legacy_logging_defaults_and_new_options_roundtrip() {
    let old = parse_toml("schema_version = 1\n[logging]\naccess_log = false\nretained_files = 4")
        .unwrap();
    assert!(!old.logging.access_log);
    assert!(old.logging.file_enabled);
    assert_eq!(old.logging.retention_days, 7);
    let mut updated = old;
    updated.logging.file_enabled = false;
    updated.logging.retention_days = 14;
    assert_eq!(
        parse_toml(&to_persistence_toml(&updated).unwrap()).unwrap(),
        updated
    );
}

#[test]
fn versions_types_and_unknown_fields_are_strict() {
    for text in [
        "",
        "schema_version = 0",
        "schema_version = 2",
        "schema_version = -1",
        "schema_version = 4294967296",
        "schema_version = '1'",
        "schema_version = 1.0",
        "schema_version = 1\nunknown = true",
        "schema_version = 1\n[listen]\nport = 65536",
        "schema_version = 1\n[listen]\nadress = '127.0.0.1'",
        "schema_version = 1\n[keys]\ninjection_mode = 'unknown'",
        "schema_version = 1\n[access]\nwhitelist = ['bad address']",
    ] {
        assert!(parse_toml(text).is_err(), "{text}");
        assert!(toml::from_str::<Config>(text).is_err(), "{text}");
    }
    let config = Config {
        schema_version: 2,
        ..Config::default()
    };
    assert!(config.validate_for_save().is_err());
    assert!(to_persistence_toml(&config).is_err());
}

#[test]
fn urls_reject_unsafe_components_and_accept_base_paths() {
    for url in [
        "http://upstream.invalid",
        "https://upstream.invalid:8443/api/v1/",
        "http://[::1]:8081/v1",
        "HTTPS://upstream.invalid/a%20b",
    ] {
        assert!(validate_upstream_url(url).is_ok(), "{url}");
    }
    for url in [
        "",
        "ftp://upstream.invalid",
        "//upstream.invalid",
        "https://user:synthetic-secret@upstream.invalid",
        "https://@upstream.invalid",
        "https://upstream.invalid?",
        "https://upstream.invalid#",
        "https://upstream.invalid?key=synthetic-secret",
        "https://upstream.invalid/#fragment",
        "https://upstream.invalid:99999",
        "https://upstream.invalid\\evil",
        " https://upstream.invalid",
        "https://upstream.invalid\n",
        "https:///upstream.invalid",
    ] {
        assert!(validate_upstream_url(url).is_err(), "{url}");
    }
}

#[test]
fn credentials_are_separate_validated_and_debug_redacted() {
    let mut c = configured();
    c.keys.injection_mode = InjectionMode::Always;
    assert!(c.validate_for_save().is_err());
    c.keys.upstream_api_key = SecretString::new("synthetic-upstream-secret");
    assert!(c.validate_for_start().is_ok());
    c.access.client_token_enabled = true;
    assert!(c.validate_for_save().is_err());
    c.access.client_token = SecretString::new("synthetic-client-secret");
    assert!(c.validate_for_save().is_ok());
    let debug = format!("{c:?}");
    assert!(!debug.contains("synthetic-upstream-secret"));
    assert!(!debug.contains("synthetic-client-secret"));
    #[cfg(windows)]
    {
        let protected = to_persistence_toml(&c).unwrap();
        assert!(!protected.contains("synthetic-upstream-secret"));
        assert!(!protected.contains("synthetic-client-secret"));
        assert_eq!(parse_toml(&protected).unwrap(), c);
    }
    #[cfg(not(windows))]
    assert_eq!(to_persistence_toml(&c), Err(ConfigError::DPAPI_UNSUPPORTED));
    c.storage.secrets = SecretStorage::Plaintext;
    let text = to_persistence_toml(&c).unwrap();
    assert!(text.contains("synthetic-upstream-secret"));
    assert_eq!(parse_toml(&text).unwrap(), c);
    for invalid in ["\r\ninjected", "  ", "bad\0token"] {
        c.access.client_token = SecretString::new(invalid);
        assert!(c.validate_for_save().is_err());
    }
}

#[test]
fn validation_covers_ports_limits_timeouts_and_cors() {
    let mutations: &[fn(&mut Config)] = &[
        |c| c.listen.port = 0,
        |c| c.proxy.connect_allowed_ports = vec![0],
        |c| {
            c.proxy.connect_enabled = true;
            c.proxy.connect_allowed_ports.clear();
        },
        |c| c.network.connect_timeout_secs = 0,
        |c| c.network.response_header_timeout_secs = 0,
        |c| c.network.shutdown_grace_secs = 0,
        |c| c.network.max_active_requests = 0,
        |c| c.logging.max_file_bytes = 0,
        |c| c.logging.retained_files = 0,
        |c| c.logging.recent_entries = 0,
        |c| c.logging.max_file_bytes = 1023,
        |c| c.logging.max_file_bytes = 1024 * 1024 * 1024 + 1,
        |c| c.logging.retained_files = 101,
        |c| c.logging.recent_entries = 100_001,
        |c| c.logging.retention_days = 0,
        |c| {
            c.cors.allowed_origins = vec!["*".into()];
            c.cors.allow_credentials = true;
        },
    ];
    for mutate in mutations {
        let mut c = configured();
        mutate(&mut c);
        assert!(c.validate_for_save().is_err());
        assert!(c.validate_for_start().is_err());
    }
    for bad in [
        "null",
        "https://site.invalid/",
        "https://site.invalid/path",
        "https://site.invalid?x",
        "https://user@site.invalid",
        "file:///tmp",
    ] {
        let mut c = configured();
        c.cors.allowed_origins = vec![bad.into()];
        assert!(c.validate_for_save().is_err(), "{bad}");
    }
    let mut c = configured();
    c.cors.allowed_origins = vec![
        "https://site.invalid:8443".into(),
        "http://[::1]:8000".into(),
        "*".into(),
    ];
    c.network.response_body_idle_timeout_secs = 0;
    c.network.tunnel_idle_timeout_secs = 0;
    c.listen.port = 65535;
    assert!(c.validate_for_start().is_ok());
}

#[test]
fn ipnet_whitelist_normalizes_mapped_addresses_and_allows_loopback() {
    let c = parse_toml("schema_version = 1\n[access]\nwhitelist = ['192.0.2.0/24', '2001:db8::/32', '198.51.100.5']").unwrap();
    for ip in [
        "127.0.0.1",
        "::1",
        "::ffff:127.0.0.2",
        "192.0.2.42",
        "::ffff:192.0.2.42",
        "2001:db8::1",
        "198.51.100.5",
    ] {
        assert!(c.access.allows_ip(ip.parse().unwrap()), "{ip}");
    }
    for ip in ["203.0.113.1", "2001:db9::1", "::ffff:203.0.113.1"] {
        assert!(!c.access.allows_ip(ip.parse().unwrap()), "{ip}");
    }
    let mapped =
        parse_toml("schema_version = 1\n[access]\nwhitelist = ['::ffff:192.0.2.0/120']").unwrap();
    assert!(mapped.access.allows_ip("192.0.2.1".parse().unwrap()));
    assert!(mapped.access.allows_ip("::ffff:192.0.2.1".parse().unwrap()));
    assert!(!mapped.access.allows_ip("192.0.3.1".parse().unwrap()));
    assert!(
        Config::default()
            .access
            .allows_ip("203.0.113.1".parse().unwrap())
    );
}

#[test]
fn errors_never_echo_source_or_parser_secrets() {
    for text in [
        "schema_version = 1\n[keys]\nupstream_api_key = 'synthetic-hidden'\nbroken = [",
        "schema_version = 1\n[upstream]\nurl = 'https://synthetic-hidden@upstream.invalid'",
        "schema_version = 1\n[listen]\nport = 'synthetic-hidden'",
    ] {
        let err = parse_toml(text).unwrap_err();
        assert!(!format!("{err} {err:?}").contains("synthetic-hidden"));
        assert!(std::error::Error::source(&err).is_none());
    }
}

#[test]
fn bounded_reads_check_actual_bytes_and_utf8() {
    let mut exact = b"schema_version = 1\n#".to_vec();
    exact.resize(MAX_CONFIG_BYTES, b'x');
    assert!(read_config(exact.as_slice()).is_ok());
    exact.push(b'x');
    assert_eq!(read_config(exact.as_slice()), Err(ConfigError::TooLarge));
    assert_eq!(
        parse_toml(std::str::from_utf8(&exact).unwrap()),
        Err(ConfigError::TooLarge)
    );
    assert_eq!(read_config(&[0xff][..]), Err(ConfigError::InvalidUtf8));
    struct Infinite {
        bytes: usize,
    }
    impl Read for Infinite {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            assert!(self.bytes + buf.len() <= MAX_CONFIG_BYTES + 1);
            buf.fill(b'x');
            self.bytes += buf.len();
            Ok(buf.len())
        }
    }
    assert_eq!(
        read_config(Infinite { bytes: 0 }),
        Err(ConfigError::TooLarge)
    );
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("synthetic-hidden"))
        }
    }
    let err = read_config(Broken).unwrap_err();
    assert!(!format!("{err} {err:?}").contains("synthetic-hidden"));
}

fn absolute(name: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!("C:/config-test/{name}"))
    } else {
        PathBuf::from(format!("/config-test/{name}"))
    }
}

#[test]
fn directory_priority_is_exhaustive_and_does_not_probe_disk() {
    let exe = absolute("exe");
    let standard = absolute("standard");
    let explicit = absolute("explicit");
    for flags in 0..32 {
        let has_explicit = flags & 1 != 0;
        let marker = flags & 2 != 0;
        let explicit_exists = flags & 4 != 0;
        let portable_exists = flags & 8 != 0;
        let standard_exists = flags & 16 != 0;
        let selected = select_directory(DirectoryInputs {
            explicit_directory: has_explicit.then_some(explicit.as_path()),
            executable_directory: &exe,
            standard_directory: &standard,
            portable_marker_exists: marker,
            explicit_config_exists: explicit_exists,
            portable_config_exists: portable_exists,
            standard_config_exists: standard_exists,
        })
        .unwrap();
        let (path, source, exists) = if has_explicit {
            (&explicit, DirectorySource::Explicit, explicit_exists)
        } else if marker {
            (&exe, DirectorySource::Portable, portable_exists)
        } else {
            (&standard, DirectorySource::Standard, standard_exists)
        };
        assert_eq!(selected.directory, *path);
        assert_eq!(selected.source, source);
        assert_eq!(selected.needs_initialization, !exists);
        assert_eq!(selected.config_path(), path.join("config.toml"));
    }
    let base = DirectoryInputs {
        explicit_directory: None,
        executable_directory: &exe,
        standard_directory: &standard,
        portable_marker_exists: false,
        explicit_config_exists: false,
        portable_config_exists: false,
        standard_config_exists: false,
    };
    assert!(
        select_directory(DirectoryInputs {
            explicit_directory: Some(Path::new("relative")),
            ..base
        })
        .is_err()
    );
    assert!(
        select_directory(DirectoryInputs {
            executable_directory: Path::new("."),
            ..base
        })
        .is_err()
    );
    assert!(
        select_directory(DirectoryInputs {
            standard_directory: Path::new("relative"),
            ..base
        })
        .is_err()
    );
    #[cfg(windows)]
    for relative in ["C:relative", "\\rooted-without-drive"] {
        assert!(
            select_directory(DirectoryInputs {
                explicit_directory: Some(Path::new(relative)),
                ..base
            })
            .is_err()
        );
    }
}

struct TempDirectory(PathBuf);
impl TempDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir();
        assert!(root.is_absolute());
        for _ in 0..100 {
            let path = root.join(format!(
                "ai-relay-config-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("temporary directory: {e}"),
            }
        }
        panic!("could not allocate temporary directory")
    }
}
impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn create_load_never_overwrite_or_create_missing_parents() {
    let dir = TempDirectory::new();
    let path = dir.0.join("配置.toml");
    let c = Config::default();
    create_new_config(&path, &c).unwrap();
    assert_eq!(load_config(&path).unwrap(), c);
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        create_new_config(&path, &configured()),
        Err(ConfigError::Io {
            kind: io::ErrorKind::AlreadyExists,
            ..
        })
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::write(&path, b"corrupt synthetic-hidden").unwrap();
    assert!(load_config(&path).is_err());
    assert!(create_new_config(&path, &c).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupt synthetic-hidden");
    let missing = dir.0.join("missing/config.toml");
    assert!(create_new_config(&missing, &c).is_err());
    assert!(!dir.0.join("missing").exists());
    assert_eq!(
        create_new_config(Path::new("relative.toml"), &c),
        Err(ConfigError::AbsolutePathRequired)
    );
    assert_eq!(
        load_config(Path::new("relative.toml")),
        Err(ConfigError::AbsolutePathRequired)
    );
    let mut invalid = c.clone();
    invalid.listen.port = 0;
    let invalid_path = dir.0.join("invalid.toml");
    assert!(create_new_config(&invalid_path, &invalid).is_err());
    assert!(!invalid_path.exists());
    invalid = c;
    invalid.keys.upstream_api_key = SecretString::new("synthetic-hidden");
    #[cfg(windows)]
    {
        create_new_config(&invalid_path, &invalid).unwrap();
        assert_eq!(load_config(&invalid_path).unwrap(), invalid);
        assert!(
            !std::fs::read_to_string(&invalid_path)
                .unwrap()
                .contains("synthetic-hidden")
        );
    }
    #[cfg(not(windows))]
    {
        assert_eq!(
            create_new_config(&invalid_path, &invalid),
            Err(ConfigError::DPAPI_UNSUPPORTED)
        );
        assert!(!invalid_path.exists());
    }
}
