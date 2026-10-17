use super::{Error, Result};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}
pub(super) fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| Error::Invalid("random source unavailable"))?;
    Ok(bytes)
}
/// Requires a directory controlled by the current user. Rejects symlinks/reparse
/// points, including all existing ancestors; not a defense against an attacker
/// who can concurrently replace entries in that trusted directory.
pub(super) fn check_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Error::Invalid("absolute trusted path required"));
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(m) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        return Err(Error::Invalid("reparse point"));
                    }
                }
                if m.file_type().is_symlink() {
                    return Err(Error::Invalid("symlink"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
/// Creates a fresh private snapshot directory, never adopts an existing one.
#[cfg(windows)]
pub(super) fn create_private_dir(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
        Storage::FileSystem::CreateDirectoryW,
    };
    check_path(path)?;
    // Protected DACL: only the owner, inherited by files and subdirectories.
    let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)\0".encode_utf16().collect();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: NUL-terminated SDDL and valid output pointer. Allocation freed below.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: Descriptor and NUL-terminated absolute path live across this call.
    let success = unsafe { CreateDirectoryW(path.as_ptr(), &attributes) };
    let error = std::io::Error::last_os_error();
    // SAFETY: Conversion returned a LocalAlloc allocation, exclusively owned here.
    unsafe {
        LocalFree(descriptor);
    }
    if success == 0 {
        return Err(error.into());
    }
    Ok(())
}
#[cfg(not(windows))]
pub(super) fn create_private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    check_path(path)?;
    std::fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}
pub(super) fn save_key(dir: &Path, key: &[u8; 32]) -> Result<()> {
    let stored = protect(key)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(dir.join("history.identity"))?;
    file.write_all(&stored)?;
    file.sync_all()?;
    Ok(())
}
pub(super) fn load_key(dir: &Path) -> Result<[u8; 32]> {
    let path = dir.join("history.identity");
    check_path(&path)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            let key = random::<32>()?;
            let stored = protect(&key)?;
            file.write_all(&stored)?;
            file.sync_all()?;
            Ok(key)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let m = fs::symlink_metadata(&path)?;
            if !m.is_file() || m.len() > 4096 {
                return Err(Error::Invalid("identity file"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if m.permissions().mode() & 0o077 != 0 {
                    return Err(Error::Invalid("identity permissions"));
                }
            }
            let mut b = Vec::new();
            fs::File::open(path)?.take(4097).read_to_end(&mut b)?;
            let key = unprotect(&b)?;
            key.try_into()
                .map_err(|_| Error::Invalid("identity length"))
        }
        Err(e) => Err(e.into()),
    }
}
#[cfg(not(windows))]
fn protect(b: &[u8]) -> Result<Vec<u8>> {
    Ok(b.to_vec())
}
#[cfg(not(windows))]
fn unprotect(b: &[u8]) -> Result<Vec<u8>> {
    Ok(b.to_vec())
}
#[cfg(windows)]
fn protect(b: &[u8]) -> Result<Vec<u8>> {
    crypt(b, true)
}
#[cfg(windows)]
fn unprotect(b: &[u8]) -> Result<Vec<u8>> {
    crypt(b, false)
}
#[cfg(windows)]
fn crypt(bytes: &[u8], protecting: bool) -> Result<Vec<u8>> {
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
                // SAFETY: DPAPI owns this LocalAlloc output until this guard frees it.
                unsafe {
                    for i in 0..self.0.cbData as usize {
                        std::ptr::write_volatile(self.0.pbData.add(i), 0);
                    }
                    LocalFree(self.0.pbData.cast());
                }
            }
        }
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes
            .len()
            .try_into()
            .map_err(|_| Error::Invalid("identity size"))?,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = Output(CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    });
    // SAFETY: Input is live for the call; output is writable and guarded. No UI,
    // no machine-wide protection, no plaintext fallback on failure.
    let ok = unsafe {
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
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if output.0.pbData.is_null() || output.0.cbData > 4096 {
        return Err(Error::Invalid("protected identity"));
    }
    // SAFETY: Successful DPAPI initialized cbData bytes; guard stays alive.
    Ok(unsafe { std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize) }.to_vec())
}
