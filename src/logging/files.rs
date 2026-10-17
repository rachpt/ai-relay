use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(crate) struct Files {
    directory: PathBuf,
    current: Option<File>,
    // Windows 下固定目录祖先，拒绝运行期间的目录替换/重命名。
    #[cfg(windows)]
    _directory_guards: Vec<File>,
}
fn unsafe_metadata(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
fn checked(path: &Path, directory: bool) -> Result<fs::Metadata> {
    let meta = fs::symlink_metadata(path)?;
    if unsafe_metadata(&meta) || (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
        return Err(LogError::UnsafePath);
    }
    // Hard links can expose or modify data outside the managed directory.

    #[cfg(unix)]
    if !directory {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(LogError::UnsafePath);
        }
    }
    Ok(meta)
}
fn open(path: &Path, write: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    if write {
        options.append(true).create(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | WRITE | DELETE; OPEN_REPARSE_POINT 禁止跟随文件链接。
        options.share_mode(7).custom_flags(0x0020_0000);
    }
    let file = options.open(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // SAFETY: file owns a live handle; info points to correctly sized writable storage.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: successful API call initialized the entire information structure.
        if unsafe { info.assume_init() }.nNumberOfLinks != 1 {
            return Err(LogError::UnsafePath);
        }
    }
    if unsafe_metadata(&file.metadata()?) {
        return Err(LogError::UnsafePath);
    }
    checked(path, false)?;
    Ok(file)
}
pub(crate) fn file_index(name: &str) -> Option<u32> {
    if name == CURRENT_FILE {
        return Some(0);
    }
    let number = name.strip_prefix("ai-relay.")?.strip_suffix(".log")?;
    let index: u32 = number.parse().ok()?;
    (index > 0 && index <= 99 && number == index.to_string()).then_some(index)
}
fn identity(file: &File) -> Result<[u64; 3]> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // SAFETY: live owned handle and correctly sized writable output storage.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: successful call initialized info.
        let info = unsafe { info.assume_init() };
        Ok([
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            (u64::from(info.ftCreationTime.dwHighDateTime) << 32)
                | u64::from(info.ftCreationTime.dwLowDateTime),
        ])
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata()?;
        Ok([meta.dev(), meta.ino(), 0])
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = file;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "stable file identity unavailable",
        )
        .into())
    }
}
fn name(index: u32) -> String {
    if index == 0 {
        CURRENT_FILE.into()
    } else {
        format!("ai-relay.{index}.log")
    }
}
impl Files {
    pub fn start(directory: PathBuf) -> Result<Self> {
        if !directory.is_absolute()
            || directory
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(LogError::InvalidDirectory);
        }
        let mut path = PathBuf::new();
        #[cfg(windows)]
        let mut guards = Vec::new();
        for component in directory.components() {
            path.push(component);
            if matches!(component, Component::Prefix(_)) {
                continue;
            }
            match fs::symlink_metadata(&path) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&path)?,
                Err(e) => return Err(e.into()),
            }
            checked(&path, true)?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                let guard = OpenOptions::new()
                    .read(true)
                    .share_mode(3)
                    .custom_flags(0x0200_0000 | 0x0020_0000)
                    .open(&path)?;
                if unsafe_metadata(&guard.metadata()?) {
                    return Err(LogError::UnsafePath);
                }
                guards.push(guard);
            }
        }
        Ok(Self {
            directory,
            current: None,
            #[cfg(windows)]
            _directory_guards: guards,
        })
    }
    fn verify(&self) -> Result<()> {
        for path in self.directory.ancestors() {
            if !path.as_os_str().is_empty() {
                checked(path, true)?;
            }
        }
        Ok(())
    }
    fn path(&self, name: &str) -> Result<PathBuf> {
        self.verify()?;
        if file_index(name).is_none() {
            return Err(LogError::InvalidName);
        }
        let path = self.directory.join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                checked(&path, false)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(path)
    }
    pub fn list(&self) -> Result<Vec<LogFile>> {
        self.verify()?;
        let mut files = Vec::new();
        // Enumerate only the finite naming namespace, never an unbounded directory scan.
        for index in 0..100 {
            let name = name(index);
            let path = self.path(&name)?;
            match checked(&path, false) {
                Ok(meta) => files.push(LogFile {
                    name,
                    bytes: meta.len(),
                    modified_epoch: meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs()),
                }),
                Err(LogError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(files)
    }
    pub fn configure(&mut self, options: &LogOptions) -> Result<()> {
        self.current = None;
        self.prune(options)?;
        if options.file_enabled {
            self.ensure_open()?;
        }
        Ok(())
    }
    fn ensure_open(&mut self) -> Result<()> {
        let path = self.path(CURRENT_FILE)?;
        if self.current.is_none() {
            self.current = Some(open(&path, true)?);
        }
        Ok(())
    }
    pub fn prune(&mut self, options: &LogOptions) -> Result<()> {
        let now = SystemTime::now();
        for index in 0..100 {
            let path = self.path(&name(index))?;
            let meta = match checked(&path, false) {
                Ok(m) => m,
                Err(LogError::Io(e)) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let expired = options.retention_days.is_some_and(|days| {
                meta.modified()
                    .ok()
                    .and_then(|time| now.duration_since(time).ok())
                    .is_some_and(|age| age >= Duration::from_secs(u64::from(days) * 86400))
            });
            if (index > 0 && index >= options.retained_files) || expired {
                if index == 0 {
                    self.current = None;
                }
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }
    pub fn write(&mut self, entry: &LogEntry, options: &LogOptions) -> Result<()> {
        self.ensure_open()?;
        let mut bytes = serde_json::to_vec(entry).map_err(io::Error::other)?;
        bytes.push(b'\n');
        let size = self
            .current
            .as_ref()
            .ok_or(LogError::Closed)?
            .metadata()?
            .len();
        if size > 0 && size.saturating_add(bytes.len() as u64) > options.max_file_bytes {
            self.current = None;
            self.prune(options)?;
            let oldest = self.path(&name(options.retained_files - 1))?;
            if oldest.exists() {
                fs::remove_file(oldest)?;
            }
            for index in (0..options.retained_files - 1).rev() {
                let source = self.path(&name(index))?;
                let target = self.path(&name(index + 1))?;
                if source.exists() {
                    fs::rename(source, target)?;
                }
            }
            self.ensure_open()?;
        }
        self.current
            .as_mut()
            .ok_or(LogError::Closed)?
            .write_all(&bytes)?;
        Ok(())
    }
    pub fn tail(&self, name: &str, max_bytes: usize, max_lines: usize) -> Result<TailResult> {
        if !(1..=MAX_TAIL_BYTES).contains(&max_bytes) || !(1..=MAX_TAIL_LINES).contains(&max_lines)
        {
            return Err(LogError::InvalidTailBounds);
        }
        let mut file = open(&self.path(name)?, false)?;
        let size = file.metadata()?.len();
        let start = size.saturating_sub(max_bytes as u64);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::with_capacity(max_bytes);
        file.take(max_bytes as u64).read_to_end(&mut bytes)?;
        let mut offset = if start > 0 {
            bytes
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |p| p + 1)
        } else {
            0
        };
        let end = bytes
            .len()
            .saturating_sub(usize::from(bytes.last() == Some(&b'\n')));
        let mut lines = 1;
        for pos in (offset..end).rev() {
            if bytes[pos] == b'\n' {
                if lines == max_lines {
                    offset = pos + 1;
                    break;
                }
                lines += 1;
            }
        }
        // Lossy UTF-8 can expand input; cap output at the byte limit too.
        let mut text = String::from_utf8_lossy(&bytes[offset..]).into_owned();
        let mut end = text.len().min(max_bytes);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        Ok(TailResult {
            text,
            file_bytes: size,
            truncated: start > 0 || offset > 0 || end < bytes.len() - offset,
        })
    }
    pub fn read_page(
        &self,
        name: &str,
        cursor: PageCursor,
        bounds: ReadBounds,
    ) -> Result<ReadResponse> {
        let before = match cursor {
            PageCursor::Tail => None,
            PageCursor::Before(cursor) => Some(cursor),
        };
        self.read(name, before, bounds, false)
    }
    pub fn read_follow(
        &self,
        name: &str,
        cursor: FileCursor,
        bounds: ReadBounds,
    ) -> Result<ReadResponse> {
        self.read(name, Some(cursor), bounds, true)
    }
    fn read(
        &self,
        name: &str,
        cursor: Option<FileCursor>,
        bounds: ReadBounds,
        forward: bool,
    ) -> Result<ReadResponse> {
        bounds.validate()?;
        let index = file_index(name).ok_or(LogError::InvalidName)?;
        if cursor.as_ref().is_some_and(|c| c.index != index) {
            return Err(LogError::InvalidCursor);
        }
        let mut file = match open(&self.path(name)?, false) {
            Ok(file) => file,
            Err(LogError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(ReadResponse::Missing);
            }
            Err(e) => return Err(e),
        };
        let id = identity(&file)?;
        let size = file.metadata()?.len();
        if let Some(c) = &cursor {
            if c.identity != id {
                return Ok(ReadResponse::Changed(FileChange::Replaced));
            }
            if size < c.observed_len || size < c.offset {
                return Ok(ReadResponse::Changed(FileChange::Truncated));
            }
        }
        let edge = cursor.as_ref().map_or(size, |c| c.offset);
        let start = if forward {
            edge
        } else {
            edge.saturating_sub(bounds.max_bytes as u64)
        };
        let limit = if forward { size - start } else { edge - start };
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::with_capacity(bounds.max_bytes);
        (&mut file)
            .take(limit.min(bounds.max_bytes as u64))
            .read_to_end(&mut bytes)?;
        // A concurrent truncation invalidates this snapshot rather than advancing over missing bytes.
        if bytes.len() as u64 != limit.min(bounds.max_bytes as u64) || file.metadata()?.len() < size
        {
            return Ok(ReadResponse::Changed(FileChange::Truncated));
        }
        let mut lo = 0;
        let mut hi = bytes.len();
        let mut partial =
            forward && !bytes.is_empty() && cursor.as_ref().is_some_and(|c| c.partial_line);
        if forward {
            let mut lines = 0;
            for (pos, byte) in bytes.iter().enumerate() {
                if *byte == b'\n' {
                    lines += 1;
                    if lines == bounds.max_lines {
                        hi = pos + 1;
                        break;
                    }
                }
            }
            // Prefer whole lines; only split a line when no full line fits.
            if hi > 0 && bytes[hi - 1] != b'\n' {
                if let Some(pos) = bytes[..hi].iter().rposition(|b| *b == b'\n') {
                    hi = pos + 1;
                } else {
                    partial = true;
                }
            }
        } else {
            if start > 0 && !bytes.is_empty() {
                // The first byte is also a boundary probe. Unused prefix remains for the next page.
                if let Some(pos) = bytes.iter().position(|b| *b == b'\n') {
                    if pos + 1 < hi {
                        lo = pos + 1;
                    } else {
                        partial = true;
                    }
                } else {
                    partial = true;
                }
            }
            let end = hi.saturating_sub(usize::from(bytes.last() == Some(&b'\n')));
            let mut lines = 1;
            for pos in (lo..end).rev() {
                if bytes[pos] == b'\n' {
                    if lines == bounds.max_lines {
                        lo = pos + 1;
                        partial = false;
                        break;
                    }
                    lines += 1;
                }
            }
            partial |= hi > lo && bytes[hi - 1] != b'\n';
        }
        let selected = &bytes[lo..hi];
        let utf8_loss = std::str::from_utf8(selected).is_err();
        let mut text = String::from_utf8_lossy(selected).into_owned();
        let mut end = text.len().min(bounds.max_bytes);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = partial || end < text.len();
        text.truncate(end);
        let position = |offset, partial_line| FileCursor {
            index,
            identity: id,
            offset,
            observed_len: size,
            partial_line,
        };
        let begin = start + lo as u64;
        let end = start + hi as u64;
        Ok(ReadResponse::Page(LogPage {
            text,
            file_bytes: size,
            before: (begin > 0).then(|| position(begin, lo == 0)),
            follow: position(
                end,
                selected.last().map_or_else(
                    || cursor.as_ref().is_some_and(|c| c.partial_line),
                    |byte| *byte != b'\n',
                ),
            ),
            has_more: if forward { end < size } else { begin > 0 },
            truncated,
            utf8_loss,
        }))
    }
    pub fn clear(&mut self, all: bool, options: &LogOptions) -> Result<ClearReport> {
        self.verify()?;
        if all {
            self.current = None;
        }
        let mut report = ClearReport::default();
        for index in u32::from(!all)..100 {
            let name = name(index);
            let result = self.path(&name).and_then(|path| {
                let bytes = checked(&path, false)?.len();
                fs::remove_file(path)?;
                Ok(bytes)
            });
            match result {
                Ok(bytes) => {
                    report.deleted_files += 1;
                    report.deleted_bytes += bytes;
                }
                Err(LogError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => report.errors.push(FileFailure {
                    name,
                    kind: error_kind(&e),
                }),
            }
        }
        if all
            && options.file_enabled
            && let Err(e) = self.ensure_open()
        {
            report.errors.push(FileFailure {
                name: CURRENT_FILE.into(),
                kind: error_kind(&e),
            });
        }
        Ok(report)
    }
}
pub(crate) fn error_kind(error: &LogError) -> io::ErrorKind {
    match error {
        LogError::Io(e) => e.kind(),
        _ => io::ErrorKind::InvalidInput,
    }
}
