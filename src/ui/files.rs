//! Native file selection and bounded file I/O run outside the window thread.
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

pub(super) enum Task {
    Import,
    Restore,
    Export(String),
    HistoryPath,
}
pub(super) enum Result {
    Import(String),
    Restore(PathBuf),
    Exported(PathBuf),
    HistoryPath(PathBuf),
    Cancelled,
    Error(String),
}

#[repr(C)]
#[allow(non_snake_case)]
struct OpenFileName {
    size: u32,
    owner: *mut core::ffi::c_void,
    instance: *mut core::ffi::c_void,
    filter: *const u16,
    custom_filter: *mut u16,
    max_custom_filter: u32,
    filter_index: u32,
    file: *mut u16,
    max_file: u32,
    file_title: *mut u16,
    max_file_title: u32,
    initial_dir: *const u16,
    title: *const u16,
    flags: u32,
    file_offset: u16,
    extension_offset: u16,
    default_extension: *const u16,
    custom_data: isize,
    hook: *mut core::ffi::c_void,
    template_name: *const u16,
    reserved: *mut core::ffi::c_void,
    reserved_size: u32,
    flags_ex: u32,
}
#[link(name = "comdlg32")]
unsafe extern "system" {
    fn GetOpenFileNameW(options: *mut OpenFileName) -> i32;
    fn GetSaveFileNameW(options: *mut OpenFileName) -> i32;
    fn CommDlgExtendedError() -> u32;
}

pub(super) fn spawn(task: Task) -> std::result::Result<Receiver<Result>, String> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("ai-relay-file-dialog".into())
        .spawn(move || {
            let result = run(task).unwrap_or_else(Result::Error);
            let _ = tx.send(result);
        })
        .map_err(|_| "无法创建文件选择工作线程。".to_string())?;
    Ok(rx)
}
fn run(task: Task) -> std::result::Result<Result, String> {
    let history = matches!(task, Task::HistoryPath);
    let export = history || matches!(task, Task::Export(_));
    let mut path = vec![0u16; 32768];
    let filter = super::wide(if history {
        "CSV 文件\0*.csv\0所有文件\0*.*\0"
    } else {
        "TOML 配置 / 备份\0*.toml;*.bak;*.backup\0所有文件\0*.*\0"
    });
    let title = super::wide(if history {
        "导出历史当前明细页（不含 IP／凭据 ID；请选择新文件）"
    } else if export {
        "导出脱敏配置（请选择新文件，不覆盖已有文件）"
    } else {
        "选择配置或备份文件"
    });
    let extension = super::wide(if history { "csv" } else { "toml" });
    // SAFETY: This C structure consists only of integer and raw-pointer fields; zero is valid.
    let mut options: OpenFileName = unsafe { core::mem::zeroed() };
    options.size = core::mem::size_of::<OpenFileName>() as u32;
    options.filter = filter.as_ptr();
    options.filter_index = 1;
    options.file = path.as_mut_ptr();
    options.max_file = path.len() as u32;
    options.title = title.as_ptr();
    options.default_extension = extension.as_ptr();
    // Explorer, no process-CWD changes, existing parent; imports require an existing file.
    options.flags = 0x80000 | 0x8 | 0x800 | if export { 0x2 } else { 0x1000 };
    // SAFETY: Correct repr(C) Win32 ABI, writable bounded buffer, strings live through call.
    // Dialog has no owner: no cross-thread window calls or UI State borrow is held.
    let accepted = unsafe {
        if export {
            GetSaveFileNameW(&mut options)
        } else {
            GetOpenFileNameW(&mut options)
        }
    };
    if accepted == 0 {
        // SAFETY: Reads this thread's common-dialog error without pointer arguments.
        return if unsafe { CommDlgExtendedError() } == 0 {
            Ok(Result::Cancelled)
        } else {
            Err("文件选择对话框失败。".into())
        };
    }
    use std::os::windows::ffi::OsStringExt;
    let length = path.iter().position(|c| *c == 0).ok_or("文件路径过长。")?;
    let path = PathBuf::from(std::ffi::OsString::from_wide(&path[..length]));
    if !path.is_absolute() {
        return Err("只接受绝对路径。".into());
    }
    match task {
        Task::HistoryPath => Ok(Result::HistoryPath(path)),
        Task::Restore => Ok(Result::Restore(path)),
        Task::Import => {
            let file = std::fs::File::open(&path).map_err(|_| "无法读取所选配置。")?;
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "读取配置失败。")?;
            if bytes.len() > 1024 * 1024 {
                return Err("配置超过 1 MiB 读取上限。".into());
            }
            Ok(Result::Import(
                String::from_utf8(bytes).map_err(|_| "配置必须是 UTF-8。")?,
            ))
        }
        Task::Export(text) => {
            // Never overwrite a configuration, backup, secret file, or other user data.
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|_| "无法创建导出文件：只允许新文件，不覆盖已有文件。")?;
            file.write_all(text.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|_| "导出写入未完成；所选新文件可能不完整，请检查。")?;
            Ok(Result::Exported(path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_dialog_structure_matches_windows_x64_abi() {
        assert_eq!(core::mem::size_of::<OpenFileName>(), 152);
        assert_eq!(core::mem::align_of::<OpenFileName>(), 8);
    }
}
