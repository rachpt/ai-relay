//! Resolved Windows paths only; discovery never creates directories or reads config contents.
use std::{
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use crate::config::{DirectoryInputs, DirectorySelection, select_directory};
use windows_sys::Win32::{
    System::Com::CoTaskMemFree,
    UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath},
};

pub fn standard_data_directory() -> Result<PathBuf, String> {
    let mut pointer = std::ptr::null_mut();
    // SAFETY: The folder GUID is valid, token is null (current user), and pointer
    // receives the task-allocator-owned NUL-terminated string on success.
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_LocalAppData,
            0,
            std::ptr::null_mut(),
            &mut pointer,
        )
    };
    struct KnownFolder(*mut u16);
    impl Drop for KnownFolder {
        fn drop(&mut self) {
            // SAFETY: SHGetKnownFolderPath allocates this pointer with the COM task
            // allocator; CoTaskMemFree also accepts null. Exactly one owner frees it.
            unsafe { CoTaskMemFree(self.0.cast()) };
        }
    }
    let owned = KnownFolder(pointer);
    if result < 0 || owned.0.is_null() {
        return Err(format!("获取 LocalAppData 失败（HRESULT {result:#x}）"));
    }
    let mut length = 0;
    // SAFETY: A successful SHGetKnownFolderPath returns a NUL-terminated UTF-16
    // allocation, kept alive by owned until after the path is copied.
    let path = unsafe {
        while *owned.0.add(length) != 0 {
            length += 1;
        }
        PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(
            owned.0, length,
        )))
    };
    if !path.is_absolute() {
        return Err("LocalAppData 必须为已解析的绝对路径".into());
    }
    Ok(path.join("AIRelay"))
}

pub fn portable_data_directory() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|e| format!("获取 EXE 路径失败（{e}）"))?;
    if !executable.is_absolute() {
        return Err("EXE 路径必须为已解析的绝对路径".into());
    }
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "EXE 路径缺少父目录".into())
}

fn exists(path: &Path) -> Result<bool, String> {
    // symlink_metadata deliberately treats malformed/dangling entries as present:
    // discovery must not turn a broken config into permission to initialize over it.
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("检查路径 {} 失败（{e}）", path.display())),
    }
}

pub fn discover_directory(explicit: Option<&Path>) -> Result<DirectorySelection, String> {
    // Do not probe lower-priority locations: an unrelated Known Folder/access
    // failure must not invalidate an explicit or portable selection.
    if let Some(path) = explicit {
        if !path.is_absolute() {
            return Err("配置目录必须为已解析的绝对路径".into());
        }
        return select_directory(DirectoryInputs {
            explicit_directory: Some(path),
            executable_directory: path,
            standard_directory: path,
            portable_marker_exists: false,
            explicit_config_exists: exists(&path.join("config.toml"))?,
            portable_config_exists: false,
            standard_config_exists: false,
        })
        .map_err(|e| e.to_string());
    }
    let portable = portable_data_directory()?;
    if exists(&portable.join("portable.flag"))? {
        return select_directory(DirectoryInputs {
            explicit_directory: None,
            executable_directory: &portable,
            standard_directory: &portable,
            portable_marker_exists: true,
            explicit_config_exists: false,
            portable_config_exists: exists(&portable.join("config.toml"))?,
            standard_config_exists: false,
        })
        .map_err(|e| e.to_string());
    }
    let standard = standard_data_directory()?;
    select_directory(DirectoryInputs {
        explicit_directory: None,
        executable_directory: &portable,
        standard_directory: &standard,
        portable_marker_exists: false,
        explicit_config_exists: false,
        portable_config_exists: false,
        standard_config_exists: exists(&standard.join("config.toml"))?,
    })
    .map_err(|e| e.to_string())
}
