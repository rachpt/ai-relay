//! Window-only preferences. Call load/save on the controller worker, never the GUI thread.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

pub const FILE_NAME: &str = "window.json";
const MAX_BYTES: u64 = 4096;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Physical screen pixels for the restored (not maximized/minimized) window.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WindowGeometry {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    #[serde(default = "default_dpi")]
    pub dpi: u32,
    #[serde(default)]
    pub maximized: bool,
}
const fn default_dpi() -> u32 {
    96
}
impl WindowGeometry {
    fn validate(&self) -> io::Result<()> {
        if self.width <= 0 || self.height <= 0 || !(48..=960).contains(&self.dpi) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "无效窗口尺寸或 DPI",
            ));
        }
        Ok(())
    }

    /// Keep the entire restored rectangle inside the selected monitor's work area.
    /// Even the preferred minimum must fit small/high-DPI work areas.
    pub(crate) fn fit(self, work: [i32; 4], dpi: u32) -> Self {
        let [left, top, right, bottom] = work;
        let available_width = right.saturating_sub(left).max(1);
        let available_height = bottom.saturating_sub(top).max(1);
        let resize = |value: i32| {
            (i64::from(value) * i64::from(dpi) / i64::from(self.dpi.max(1)))
                .clamp(1, i64::from(i32::MAX)) as i32
        };
        let width =
            resize(self.width).clamp(super::scale(760, dpi).min(available_width), available_width);
        let height = resize(self.height).clamp(
            super::scale(880, dpi).min(available_height),
            available_height,
        );
        Self {
            left: self.left.clamp(left, right.saturating_sub(width).max(left)),
            top: self.top.clamp(top, bottom.saturating_sub(height).max(top)),
            width,
            height,
            dpi,
            maximized: self.maximized,
        }
    }
}

fn absolute(path: &Path) -> io::Result<()> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "窗口偏好必须使用绝对文件路径",
        ));
    }
    Ok(())
}

/// Missing file is a normal first run; malformed/oversized files are not overwritten here.
pub fn load(path: &Path) -> io::Result<Option<WindowGeometry>> {
    absolute(path)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "窗口偏好超过 4 KiB",
        ));
    }
    let geometry: WindowGeometry = serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "窗口偏好格式无效"))?;
    geometry.validate()?;
    Ok(Some(geometry))
}

/// Same-directory, flushed temporary file followed by an atomic name replacement.
/// Does not create directories or remove the previous file before replacement.
pub fn save(path: &Path, geometry: &WindowGeometry) -> io::Result<()> {
    absolute(path)?;
    geometry.validate()?;
    let bytes = serde_json::to_vec_pretty(geometry).map_err(io::Error::other)?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_file_name(format!(".window-{}-{sequence}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        replace(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: Both absolute paths are terminated and live for the synchronous call.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn replace(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn geometry() -> WindowGeometry {
        WindowGeometry {
            left: -1800,
            top: 40,
            width: 800,
            height: 900,
            dpi: 96,
            maximized: true,
        }
    }
    #[test]
    fn fit_removed_monitor_and_high_dpi_work_area() {
        let fitted = geometry().fit([0, 0, 1920, 1040], 192);
        assert_eq!(fitted.left, 0);
        assert_eq!(fitted.top, 0);
        assert_eq!(fitted.width, 1600);
        assert_eq!(fitted.height, 1040);
        assert!(fitted.maximized);
        let small = geometry().fit([-1280, -720, 0, 0], 192);
        assert_eq!(
            [small.left, small.top, small.width, small.height],
            [-1280, -720, 1280, 720]
        );
    }
    #[test]
    fn defaults_and_invalid_values() {
        let old: WindowGeometry =
            serde_json::from_str(r#"{"left":0,"top":0,"width":800,"height":900}"#).unwrap();
        assert_eq!(old.dpi, 96);
        assert!(!old.maximized);
        assert!(WindowGeometry { width: 0, ..old }.validate().is_err());
        assert!(WindowGeometry { dpi: 0, ..old }.validate().is_err());
        assert!(load(Path::new("relative.json")).is_err());
        assert!(save(Path::new("relative.json"), &old).is_err());
    }
    #[test]
    fn atomic_round_trip_and_failure_preserves_previous_file() {
        let directory = std::env::temp_dir().join(format!(
            "ai-relay-window-test-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join(FILE_NAME);
        assert_eq!(load(&path).unwrap(), None);
        save(&path, &geometry()).unwrap();
        assert_eq!(load(&path).unwrap(), Some(geometry()));
        let changed = WindowGeometry {
            maximized: false,
            ..geometry()
        };
        save(&path, &changed).unwrap();
        assert_eq!(load(&path).unwrap(), Some(changed));
        assert!(
            save(
                &path,
                &WindowGeometry {
                    height: -1,
                    ..changed
                }
            )
            .is_err()
        );
        assert_eq!(load(&path).unwrap(), Some(changed));
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::write(&path, "not-json").unwrap();
        assert!(load(&path).is_err());
        fs::write(&path, vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert!(load(&path).is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
