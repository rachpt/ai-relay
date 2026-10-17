#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    if !std::env::args_os().any(|arg| arg == "--background") {
        attach_parent_console();
    }
    #[cfg(windows)]
    prevent_standard_handle_inheritance();
    match ai_relay::app::run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            error.exit_code()
        }
    }
}

#[cfg(windows)]
fn prevent_standard_handle_inheritance() {
    use windows_sys::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation},
        System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    };
    for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: Inspect only this process's standard handles. Children explicitly
        // configure their own stdio; parent redirection must never outlive the CLI.
        unsafe {
            let handle = GetStdHandle(id);
            if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::{
        Foundation::{HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::GetFileType,
        System::Console::{
            ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE,
            STD_OUTPUT_HANDLE, SetStdHandle,
        },
    };
    fn usable(handle: HANDLE) -> bool {
        // SAFETY: GetFileType only inspects the handle; invalid handles are rejected first.
        !handle.is_null() && handle != INVALID_HANDLE_VALUE && unsafe { GetFileType(handle) } != 0
    }
    // SAFETY: These APIs inspect process standard handles without transferring ownership.
    let (output, error) = unsafe {
        (
            GetStdHandle(STD_OUTPUT_HANDLE),
            GetStdHandle(STD_ERROR_HANDLE),
        )
    };
    let (keep_output, keep_error) = (usable(output), usable(error));
    if keep_output && keep_error {
        return;
    }
    // SAFETY: Attach only to the parent; never allocate a console for Explorer launches.
    // AttachConsole can replace inherited redirection handles, so restore those below.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
        if keep_output {
            SetStdHandle(STD_OUTPUT_HANDLE, output);
        }
        if keep_error {
            SetStdHandle(STD_ERROR_HANDLE, error);
        }
    }
}
