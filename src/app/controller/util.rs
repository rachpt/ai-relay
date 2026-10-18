//! 控制器内部共享的路径、错误文本和概览快照辅助。
use std::{
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
};

use crate::{
    config::ConfigError,
    proxy::{ProxyError, ProxyMetrics},
};

use super::types::OverviewSnapshot;

pub(crate) type OverviewState = Arc<Mutex<(Option<SocketAddr>, Option<String>)>>;

pub(crate) fn same_directory(left: &Path, right: &Path) -> bool {
    // Canonicalization runs only on controller, never GUI. Fail closed for missing
    // paths except identical absolute spelling (normal first-run directory).
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(a), Ok(b)) => a == b,
        _ => left == right,
    }
}

pub(crate) fn open_directory(path: &Path) -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    if !path.is_absolute() || !path.is_dir() {
        return Err("目录不是已存在的绝对目录；未请求打开。".into());
    }
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err("目录含无效字符；未请求打开。".into());
    }
    wide.push(0);
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    // SAFETY: 两个字符串在同步调用期间有效且 NUL 结尾；无参数、无 CMD、无句柄所有权转移。
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        Err(format!("系统拒绝打开目录（ShellExecute 状态 {result}）。"))
    } else {
        Ok(format!(
            "系统已接受打开目录请求：{}；不代表资源管理器窗口已显示。",
            path.display()
        ))
    }
}

pub(crate) fn overview_snapshot(state: &OverviewState, metrics: &ProxyMetrics) -> OverviewSnapshot {
    let (listen, upstream) = state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    OverviewSnapshot {
        metrics: metrics.snapshot(),
        running: listen.is_some(),
        listen,
        upstream,
    }
}

pub(crate) fn validation_error(field: &str) -> String {
    format!("配置校验失败（字段 {field}）；请检查 URL、端口、密钥及注入策略。")
}

pub(crate) fn config_error(error: &ConfigError) -> String {
    match error {
        ConfigError::InvalidConfig(e) if e.field == "protected_secrets" => "DPAPI 密钥无法解密或密文格式无效；请使用原 Windows 用户，或重新输入密钥。未降级为明文。".into(),
        ConfigError::InvalidConfig(e) => validation_error(e.field),
        ConfigError::DpapiNotImplemented => "当前平台无法使用 Windows DPAPI，未降级为明文。".into(),
        ConfigError::BackupPublicationFailed { recovery_path, .. } => format!(
            "配置已保存，但备份发布失败；旧配置保留于 {}，请检查后再操作。",
            recovery_path.display()
        ),
        ConfigError::AtomicReplacementFailed {
            replacement_path,
            recovery_path,
            ..
        } => format!(
            "原子替换失败，请检查保留文件 {} 和 {}；不要直接覆盖。",
            replacement_path.display(),
            recovery_path.display()
        ),
        ConfigError::InvalidToml | ConfigError::InvalidUtf8 => {
            "配置格式或版本不受支持；原文件不会被覆盖，请手工修复或从备份恢复。".into()
        }
        ConfigError::TooLarge => "配置超过 1 MiB 上限。".into(),
        ConfigError::AbsolutePathRequired => "配置路径必须为绝对路径。".into(),
        ConfigError::Serialization => "配置序列化失败，未保存。".into(),
        ConfigError::ExternalModification => {
            "配置文件在加载后已被外部修改、创建或删除；未保存，请重新加载并确认修改。".into()
        }
        ConfigError::Io { kind, .. } => {
            format!("配置读写失败（{kind:?}），请检查显示的数据目录及权限。")
        }
    }
}

pub(crate) fn proxy_error(error: &ProxyError) -> String {
    match error {
        ProxyError::InvalidConfig(e) => validation_error(e.field),
        ProxyError::UnsupportedFeature => "请求的代理扩展尚不受支持，请检查配置。".into(),
        ProxyError::InvalidUpstream => "上游 URL 无效。".into(),
        ProxyError::InvalidCredential => "密钥无法用于 HTTP 认证头，请检查密钥格式。".into(),
        ProxyError::TlsRoots => "无法加载系统 TLS 信任根，未关闭证书校验。".into(),
        ProxyError::Bind => "无法监听指定 IP 和端口，请检查地址、端口占用及权限。".into(),
        ProxyError::Listener | ProxyError::Task => "代理监听或后台任务异常，请停止后重试。".into(),
    }
}
