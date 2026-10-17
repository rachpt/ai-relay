//! Current-user Startup shortcut. Call on a worker thread, never the GUI thread.
//! No registry fallback, PowerShell, shell execution, or link resolution is used.
use std::{
    ffi::OsString,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use windows::{
    Win32::{
        Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize, IPersistFile, STGM_READ,
        },
        UI::Shell::{
            FOLDERID_Startup, IShellLinkW, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath, SLGP_RAWPATH,
            ShellLink,
        },
    },
    core::{Interface, PCWSTR, PWSTR},
};

pub const LINK_NAME: &str = "AI Relay.lnk";
/// Stable ownership marker, not a security boundary against the current user.
pub const LINK_MARKER: &str = "AI Relay Startup {C7B745D3-8A5A-46F6-916A-DF6479020FF1}";
const TEXT_LIMIT: usize = 32768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Disabled,
    /// An owned shortcut points to a missing executable or config directory.
    Lost,
    /// Foreign, malformed, or different from the supplied executable/configuration.
    Inconsistent,
    Enabled,
}

pub fn status(exe: &Path, config_dir: &Path) -> Result<Status, String> {
    status_in(&startup_directory()?, exe, config_dir)
}
pub fn enable(exe: &Path, config_dir: &Path) -> Result<Status, String> {
    enable_in(&startup_directory()?, exe, config_dir)
}
pub fn disable(exe: &Path, config_dir: &Path) -> Result<Status, String> {
    disable_in(&startup_directory()?, exe, config_dir)
}

struct Apartment;
impl Apartment {
    fn new() -> Result<Self, String> {
        // SAFETY: The guard balances every successful initialization, including S_FALSE.
        // RPC_E_CHANGED_MODE fails without changing/uninitializing the caller's apartment.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(com_error)?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: This guard owns one successful COM initialization on this thread.
        unsafe { CoUninitialize() };
    }
}
fn com_error(error: windows::core::Error) -> String {
    format!(
        "Startup 快捷方式 COM 操作失败（HRESULT {:#x}）",
        error.code().0
    )
}
fn wide(path: &Path) -> Result<Vec<u16>, String> {
    let mut text: Vec<_> = path.as_os_str().encode_wide().collect();
    if text.contains(&0) || text.len() >= TEXT_LIMIT {
        return Err("快捷方式路径含 NUL 或超出长度限制。".into());
    }
    text.push(0);
    Ok(text)
}
fn text_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn validate(exe: &Path, config: &Path) -> Result<(), String> {
    if !exe.is_absolute() || !config.is_absolute() || exe.parent().is_none() {
        return Err("EXE 和配置目录必须为已解析的绝对路径。".into());
    }
    wide(exe)?;
    wide(config)?;
    if exe.components().chain(config.components()).any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    }) {
        return Err("路径不得含未解析的 . 或 ..。".into());
    }
    Ok(())
}
/// Quote with CommandLineToArgvW/CRT backslash rules, including trailing backslashes.
fn arguments(config: &Path) -> Result<Vec<u16>, String> {
    let path = wide(config)?;
    let mut result: Vec<_> = "--background --config-dir \"".encode_utf16().collect();
    let mut slashes = 0;
    for &unit in &path[..path.len() - 1] {
        if unit == b'\\' as u16 {
            slashes += 1;
        } else {
            result.extend(std::iter::repeat_n(
                b'\\' as u16,
                if unit == b'"' as u16 {
                    slashes * 2 + 1
                } else {
                    slashes
                },
            ));
            result.push(unit);
            slashes = 0;
        }
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.extend([b'"' as u16, 0]);
    if result.len() > TEXT_LIMIT {
        return Err("快捷方式参数超出长度限制。".into());
    }
    Ok(result)
}
fn startup_directory() -> Result<PathBuf, String> {
    // SAFETY: Startup is a documented Known Folder; None selects the current user.
    // DONT_VERIFY avoids creating/repairing the folder during status queries.
    let pointer = unsafe { SHGetKnownFolderPath(&FOLDERID_Startup, KF_FLAG_DONT_VERIFY, None) }
        .map_err(com_error)?;
    struct Allocated(PWSTR);
    impl Drop for Allocated {
        fn drop(&mut self) {
            // SAFETY: SHGetKnownFolderPath allocated this string with the task allocator.
            unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
        }
    }
    let owned = Allocated(pointer);
    // SAFETY: Successful Known Folder output is a live NUL-terminated UTF-16 string.
    let path = PathBuf::from(OsString::from_wide(unsafe { owned.0.as_wide() }));
    if !path.is_absolute() {
        return Err("Startup Known Folder 不是绝对路径。".into());
    }
    Ok(path)
}
fn present(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            // Never follow filesystem reparse points, including dangling links.
            if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                return Err("Startup 同名项目不是普通快捷方式文件；拒绝修改。".into());
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("无法读取 Startup 快捷方式元数据。".into()),
    }
}
struct Link {
    // Drop COM interfaces before the apartment guard.
    shell: IShellLinkW,
    persist: IPersistFile,
    _apartment: Apartment,
}
impl Link {
    fn new() -> Result<Self, String> {
        let apartment = Apartment::new()?;
        // SAFETY: COM initialized on this thread; documented in-process ShellLink class.
        let shell: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
                .map_err(com_error)?;
        let persist = shell.cast().map_err(com_error)?;
        Ok(Self {
            shell,
            persist,
            _apartment: apartment,
        })
    }
    fn load(path: &Path) -> Result<Self, String> {
        let link = Self::new()?;
        let path = wide(path)?;
        // SAFETY: NUL-terminated filename; read-only load. Never call Resolve or Save here.
        unsafe { link.persist.Load(PCWSTR(path.as_ptr()), STGM_READ) }.map_err(com_error)?;
        Ok(link)
    }
    fn fields(&self) -> Result<Fields, String> {
        let mut target = vec![0; TEXT_LIMIT];
        let mut args = vec![0; TEXT_LIMIT];
        let mut description = vec![0; TEXT_LIMIT];
        let mut working = vec![0; TEXT_LIMIT];
        // SAFETY: Each buffer is writable with the exact advertised capacity. RAWPATH
        // reads the stored target without resolving it or contacting a network target.
        unsafe {
            self.shell
                .GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
                .map_err(com_error)?;
            self.shell.GetArguments(&mut args).map_err(com_error)?;
            self.shell
                .GetDescription(&mut description)
                .map_err(com_error)?;
            self.shell
                .GetWorkingDirectory(&mut working)
                .map_err(com_error)?;
        }
        Ok(Fields {
            target: path_from_buffer(&target)?,
            args: units(&args)?.to_vec(),
            description: units(&description)?.to_vec(),
            working: path_from_buffer(&working)?,
        })
    }
}

struct Fields {
    target: PathBuf,
    args: Vec<u16>,
    description: Vec<u16>,
    working: PathBuf,
}
fn units(buffer: &[u16]) -> Result<&[u16], String> {
    let end = buffer
        .iter()
        .position(|&c| c == 0)
        .ok_or("快捷方式字段超出长度限制。")?;
    if end == buffer.len() - 1 {
        return Err("快捷方式字段可能已截断。".into());
    }
    Ok(&buffer[..end])
}
fn path_from_buffer(buffer: &[u16]) -> Result<PathBuf, String> {
    Ok(PathBuf::from(OsString::from_wide(units(buffer)?)))
}
fn owned(fields: &Fields, exe: &Path) -> bool {
    fields.description == LINK_MARKER.encode_utf16().collect::<Vec<_>>() && fields.target == exe
}
fn status_in(startup: &Path, exe: &Path, config: &Path) -> Result<Status, String> {
    validate(exe, config)?;
    let path = startup.join(LINK_NAME);
    if !present(&path)? {
        return Ok(Status::Disabled);
    }
    let fields = match Link::load(&path).and_then(|link| link.fields()) {
        Ok(fields) => fields,
        Err(_) => return Ok(Status::Inconsistent),
    };
    if fields.description != LINK_MARKER.encode_utf16().collect::<Vec<_>>() {
        return Ok(Status::Inconsistent);
    }
    if !fields.target.is_absolute() {
        return Ok(Status::Inconsistent);
    }
    match fields.target.try_exists() {
        Ok(false) => return Ok(Status::Lost),
        Err(_) => return Err("无法检查 Startup 目标是否存在。".into()),
        _ => {}
    }
    let args = arguments(config)?;
    if !owned(&fields, exe)
        || fields.working != exe.parent().ok_or("EXE 缺少父目录。")?
        || fields.args != args[..args.len() - 1]
    {
        return Ok(Status::Inconsistent);
    }
    if !config.is_dir() || !exe.is_file() {
        return Ok(Status::Lost);
    }
    Ok(Status::Enabled)
}
fn verify_owned(path: &Path, exe: &Path) -> Result<(), String> {
    let fields = Link::load(path)?.fields()?;
    if !owned(&fields, exe) {
        return Err("同名 Startup 项目的标记或 EXE 目标不属于当前应用；拒绝覆盖或删除。".into());
    }
    Ok(())
}
fn enable_in(startup: &Path, exe: &Path, config: &Path) -> Result<Status, String> {
    validate(exe, config)?;
    if !startup.is_absolute() || !startup.is_dir() || !exe.is_file() || !config.is_dir() {
        return Err("启用自启需要存在的 Startup 目录、EXE 和配置目录。".into());
    }
    let destination = startup.join(LINK_NAME);
    let existed = present(&destination)?;
    if existed {
        verify_owned(&destination, exe)?;
    }
    let link = Link::new()?;
    let target = wide(exe)?;
    let working = wide(exe.parent().ok_or("EXE 缺少父目录。")?)?;
    let args = arguments(config)?;
    let marker = text_wide(LINK_MARKER);
    // SAFETY: All inputs are NUL-terminated and live through the calls.
    unsafe {
        link.shell
            .SetPath(PCWSTR(target.as_ptr()))
            .map_err(com_error)?;
        link.shell
            .SetArguments(PCWSTR(args.as_ptr()))
            .map_err(com_error)?;
        link.shell
            .SetWorkingDirectory(PCWSTR(working.as_ptr()))
            .map_err(com_error)?;
        link.shell
            .SetDescription(PCWSTR(marker.as_ptr()))
            .map_err(com_error)?;
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temporary = startup.join(format!(
        ".ai-relay-{}-{}.lnk",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // Reserve rather than clobbering a pre-existing temporary filename.
    let reservation = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "无法创建 Startup 临时快捷方式。")?;
    drop(reservation);
    struct Temporary(PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let temporary = Temporary(temporary);
    let temp_wide = wide(&temporary.0)?;
    // SAFETY: Persist the new, locally owned link to the reserved temporary file only.
    unsafe { link.persist.Save(PCWSTR(temp_wide.as_ptr()), true) }.map_err(com_error)?;
    if status_at_link(&temporary.0, exe, config)? != Status::Enabled {
        return Err("临时快捷方式验证失败；未替换 Startup 项目。".into());
    }
    if existed {
        verify_owned(&destination, exe)?;
    }
    let destination_wide = wide(&destination)?;
    let flags = if existed {
        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
    } else {
        MOVEFILE_WRITE_THROUGH
    };
    // SAFETY: Absolute filenames; no replace flag when initially absent. Recheck ownership
    // before replacing. Same-user concurrent malicious mutation is outside this boundary.
    unsafe {
        MoveFileExW(
            PCWSTR(temp_wide.as_ptr()),
            PCWSTR(destination_wide.as_ptr()),
            flags,
        )
    }
    .map_err(com_error)?;
    let result = status_in(startup, exe, config)?;
    if result != Status::Enabled {
        return Err("自启项已写入，但验证不一致；请检查实际状态。".into());
    }
    Ok(result)
}
fn status_at_link(path: &Path, exe: &Path, config: &Path) -> Result<Status, String> {
    let fields = Link::load(path)?.fields()?;
    let args = arguments(config)?;
    Ok(
        if owned(&fields, exe)
            && fields.args == args[..args.len() - 1]
            && Some(fields.working.as_path()) == exe.parent()
        {
            Status::Enabled
        } else {
            Status::Inconsistent
        },
    )
}
fn disable_in(startup: &Path, exe: &Path, config: &Path) -> Result<Status, String> {
    validate(exe, config)?;
    let path = startup.join(LINK_NAME);
    if present(&path)? {
        verify_owned(&path, exe)?;
        std::fs::remove_file(&path).map_err(|_| "删除 Startup 快捷方式失败。")?;
    }
    if present(&path)? {
        return Err("删除后 Startup 快捷方式仍存在。".into());
    }
    Ok(Status::Disabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        exe: PathBuf,
        config: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "ai-relay-startup-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            let exe = root.join("中文 app.exe");
            let config = root.join("配置 data");
            std::fs::write(&exe, b"test fixture, never executed").unwrap();
            std::fs::create_dir(&config).unwrap();
            Self { root, exe, config }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn injected_startup_roundtrip_and_missing_target() {
        let f = Fixture::new();
        assert_eq!(
            status_in(&f.root, &f.exe, &f.config).unwrap(),
            Status::Disabled
        );
        assert_eq!(
            enable_in(&f.root, &f.exe, &f.config).unwrap(),
            Status::Enabled
        );
        assert_eq!(
            status_in(&f.root, &f.exe, &f.root).unwrap(),
            Status::Inconsistent
        );
        assert_eq!(
            enable_in(&f.root, &f.exe, &f.config).unwrap(),
            Status::Enabled
        );
        std::fs::remove_file(&f.exe).unwrap();
        assert_eq!(status_in(&f.root, &f.exe, &f.config).unwrap(), Status::Lost);
        assert_eq!(
            disable_in(&f.root, &f.exe, &f.config).unwrap(),
            Status::Disabled
        );
        assert_eq!(
            disable_in(&f.root, &f.exe, &f.config).unwrap(),
            Status::Disabled
        );
    }
    #[test]
    fn foreign_same_name_and_wrong_target_are_not_modified() {
        let f = Fixture::new();
        let path = f.root.join(LINK_NAME);
        std::fs::write(&path, b"foreign file").unwrap();
        assert!(enable_in(&f.root, &f.exe, &f.config).is_err());
        assert!(disable_in(&f.root, &f.exe, &f.config).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"foreign file");
        std::fs::remove_file(&path).unwrap();
        enable_in(&f.root, &f.exe, &f.config).unwrap();
        let other = f.root.join("other.exe");
        std::fs::write(&other, b"test").unwrap();
        assert!(enable_in(&f.root, &other, &f.config).is_err());
        assert!(disable_in(&f.root, &other, &f.config).is_err());
        assert_eq!(
            status_in(&f.root, &f.exe, &f.config).unwrap(),
            Status::Enabled
        );
    }
    #[test]
    fn a_valid_unmarked_shell_link_is_foreign() {
        let f = Fixture::new();
        let path = f.root.join(LINK_NAME);
        let link = Link::new().unwrap();
        let target = wide(&f.exe).unwrap();
        let filename = wide(&path).unwrap();
        // SAFETY: Test COM guard is live; all strings are NUL-terminated and this
        // filename is exclusively inside the test's temporary Startup directory.
        unsafe {
            link.shell.SetPath(PCWSTR(target.as_ptr())).unwrap();
            link.persist.Save(PCWSTR(filename.as_ptr()), true).unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        assert_eq!(
            status_in(&f.root, &f.exe, &f.config).unwrap(),
            Status::Inconsistent
        );
        assert!(enable_in(&f.root, &f.exe, &f.config).is_err());
        assert!(disable_in(&f.root, &f.exe, &f.config).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn quote_trailing_backslashes_and_reject_relative_paths() {
        let args = arguments(Path::new("C:\\配置 data\\")).unwrap();
        assert_eq!(
            String::from_utf16(&args[..args.len() - 1]).unwrap(),
            "--background --config-dir \"C:\\配置 data\\\\\""
        );
        let f = Fixture::new();
        assert!(enable_in(&f.root, Path::new("relative.exe"), &f.config).is_err());
    }
}
