//! Blocking migration filesystem steps; the controller must stop the proxy and
//! create a coherent SQLite snapshot first. Source data is never deleted.
use crate::config::{self, Config, StorageMode};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const HISTORY: [&str; 2] = ["history.sqlite3", "history.identity"];

pub(super) struct Plan {
    pub source: PathBuf,
    pub target: PathBuf,
    pub executable_directory: PathBuf,
    pub config: Config,
    previous: Option<Config>,
}

pub(super) fn safe_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("迁移路径必须是已解析的绝对路径。".into());
    }
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(m) => {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    return Err("迁移路径含重解析点，拒绝跟随。".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("无法检查迁移路径权限。".into()),
        }
    }
    Ok(())
}
fn absent(path: &Path) -> Result<bool, String> {
    safe_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err("无法检查迁移目标。".into()),
    }
}
impl Plan {
    pub fn new(
        source: PathBuf,
        target: PathBuf,
        executable_directory: PathBuf,
        saved: &Config,
        mode: StorageMode,
    ) -> Result<Self, String> {
        safe_path(&source)?;
        safe_path(&target)?;
        safe_path(&executable_directory)?;
        if source == target
            || source
                .canonicalize()
                .ok()
                .zip(target.canonicalize().ok())
                .is_some_and(|(a, b)| a == b)
        {
            return Err("源目录与目标目录相同，无需迁移。".into());
        }
        let previous = match config::inspect_restore_target(&target.join("config.toml"))
            .map_err(|_| "目标配置不是可安全备份的普通文件。")?
        {
            Some(_) => Some(
                config::load_config(&target.join("config.toml"))
                    .map_err(|_| "目标配置损坏或版本不兼容；请先手工恢复，不覆盖。")?,
            ),
            None => None,
        };
        for name in [
            "history.sqlite3",
            "history.identity",
            "history.sqlite3-wal",
            "history.sqlite3-shm",
            "history.sqlite3-journal",
        ] {
            if !absent(&target.join(name))? {
                return Err("目标已有历史数据库或身份文件；不自动合并/覆盖，请先选择空目标或由用户备份移走。".into());
            }
        }
        let marker = executable_directory.join("portable.flag");
        if !absent(&marker)? && !marker.is_file() {
            return Err("便携标记不是普通文件，拒绝迁移。".into());
        }
        let mut config = saved.clone();
        config.storage.mode = mode;
        Ok(Self {
            source,
            target,
            executable_directory,
            config,
            previous,
        })
    }
    pub fn description(&self) -> String {
        format!(
            "来源：{}\n目标：{}\n将复制配置（含当前用户 DPAPI 密文）和历史数据库/凭据身份；日志不迁移，源数据全部保留。\n目标配置：{}。将更新便携标记及已启用的当前应用自启参数。确认迁移？",
            self.source.display(),
            self.target.display(),
            if self.previous.is_some() {
                "已有有效配置，确认后原子备份并替换"
            } else {
                "不存在，新建"
            }
        )
    }
    pub fn staging_path(&self) -> Result<PathBuf, String> {
        fs::create_dir_all(&self.target).map_err(|_| "无法创建目标目录。")?;
        safe_path(&self.target)?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Ok(self.target.join(format!(
            ".ai-relay-migration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
    pub fn publish(
        &self,
        staged: &Path,
        mut update_startup: impl FnMut(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        safe_path(staged)?;
        // Repeat all destination checks after the user confirmation/snapshot wait.
        for name in [
            "history.sqlite3",
            "history.identity",
            "history.sqlite3-wal",
            "history.sqlite3-shm",
            "history.sqlite3-journal",
        ] {
            if !absent(&self.target.join(name))? {
                return Err("迁移目标在预览后改变，未发布。".into());
            }
        }
        let current = config::inspect_restore_target(&self.target.join("config.toml"))
            .map_err(|_| "目标配置检查失败。")?;
        match (&self.previous, current) {
            (None, None) => {}
            (Some(expected), Some(_))
                if config::load_config(&self.target.join("config.toml"))
                    .ok()
                    .as_ref()
                    == Some(expected) => {}
            _ => return Err("目标配置在预览后改变，请重新预览。".into()),
        }
        let mut installed = Vec::new();
        let mut config_committed = false;
        let mut marker_created = false;
        let mut marker_backup = None;
        let mut startup_changed = false;
        let result: Result<(), String> = (|| {
            for name in HISTORY {
                move_new(&staged.join(name), &self.target.join(name))?;
                installed.push(name);
            }
            let save = config::save_config_atomic_checked(
                &self.target.join("config.toml"),
                &self.config,
                self.previous.as_ref(),
            );
            config_committed = save.is_ok()
                || matches!(
                    save,
                    Err(config::ConfigError::BackupPublicationFailed { .. })
                );
            save.map_err(|_| "目标配置提交或备份失败。")?;
            // A failed system update may be partial; attempt source repair on rollback.
            startup_changed = true;
            update_startup(&self.target)?;
            let marker = self.executable_directory.join("portable.flag");
            match self.config.storage.mode {
                StorageMode::Portable => {
                    match fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&marker)
                    {
                        Ok(f) => {
                            marker_created = true;
                            f.sync_all().map_err(|_| "便携标记同步失败。")?;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                            safe_path(&marker)?;
                            if !marker.is_file() {
                                return Err("便携标记不是普通文件。".into());
                            }
                        }
                        Err(_) => return Err("创建便携标记失败。".into()),
                    }
                }
                StorageMode::Standard => {
                    if !absent(&marker)? {
                        let retained = staged.join("portable.flag.source-backup");
                        move_new(&marker, &retained)?;
                        marker_backup = Some(retained);
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut recovery = Vec::new();
            if let Some(backup) = marker_backup
                && let Err(e) = move_new(&backup, &self.executable_directory.join("portable.flag"))
            {
                recovery.push(e);
            }
            if marker_created
                && fs::remove_file(self.executable_directory.join("portable.flag")).is_err()
            {
                recovery.push("新便携标记未能撤销。".into());
            }
            if startup_changed && let Err(e) = update_startup(&self.source) {
                recovery.push(format!("自启回退失败：{e}"));
            }
            if config_committed {
                let path = self.target.join("config.toml");
                let restored = if let Some(previous) = &self.previous {
                    config::save_config_atomic_checked(&path, previous, Some(&self.config))
                        .map_err(|_| "目标原配置回退失败。".to_string())
                } else if config::load_config(&path).ok().as_ref() == Some(&self.config) {
                    move_new(&path, &staged.join("config.unpublished.toml"))
                } else {
                    Err("目标配置被其他写入改变，未覆盖回退。".into())
                };
                if let Err(e) = restored {
                    recovery.push(e);
                }
            }
            for name in installed.into_iter().rev() {
                if let Err(e) = move_new(&self.target.join(name), &staged.join(name)) {
                    recovery.push(e);
                }
            }
            return Err(format!(
                "{error} 源数据未删除；暂存/恢复目录：{}。回退情况：{}",
                staged.display(),
                if recovery.is_empty() {
                    "已撤销本批发布".into()
                } else {
                    recovery.join("；")
                }
            ));
        }
        Ok(())
    }
}
fn move_new(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    safe_path(from)?;
    safe_path(to)?;
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: Both absolute NUL-terminated buffers are live. No replace flag:
    // publication/rollback never overwrites a racing destination.
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err("迁移文件移动失败；目标可能已存在或被占用。".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temporary() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ai-relay-migration-{}-{}-{}",
            std::process::id(),
            crate::history::now_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        dir
    }
    #[test]
    fn snapshot_publish_retains_source_and_identity() {
        let dir = temporary();
        let source = dir.join("source");
        let target = dir.join("target");
        let exe = dir.join("exe");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir(&exe).unwrap();
        let config = Config::default();
        config::save_config_atomic(&source.join("config.toml"), &config).unwrap();
        let mut history = crate::history::HistoryService::start(&source).unwrap();
        let id = history.handle().credential_id("synthetic-migration");
        let plan = Plan::new(
            source.clone(),
            target.clone(),
            exe.clone(),
            &config,
            StorageMode::Portable,
        )
        .unwrap();
        let stage = plan.staging_path().unwrap();
        history.snapshot_to(&stage).unwrap();
        history
            .flush_close(std::time::Duration::from_secs(5))
            .unwrap();
        plan.publish(&stage, |_| Ok(())).unwrap();
        assert!(source.join("config.toml").exists());
        assert!(exe.join("portable.flag").is_file());
        assert_eq!(
            config::load_config(&target.join("config.toml"))
                .unwrap()
                .storage
                .mode,
            StorageMode::Portable
        );
        let mut migrated = crate::history::HistoryService::start(&target).unwrap();
        assert_eq!(id, migrated.handle().credential_id("synthetic-migration"));
        migrated
            .flush_close(std::time::Duration::from_secs(5))
            .unwrap();
        drop(migrated);
        drop(history);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_system_update_rolls_back_and_preserves_target_config() {
        let dir = temporary();
        let source = dir.join("source");
        let target = dir.join("target");
        let exe = dir.join("exe");
        for p in [&source, &target, &exe] {
            fs::create_dir(p).unwrap();
        }
        let mut old = Config::default();
        old.listen.port = 9001;
        config::save_config_atomic(&target.join("config.toml"), &old).unwrap();
        let plan = Plan::new(
            source.clone(),
            target.clone(),
            exe,
            &Config::default(),
            StorageMode::Standard,
        )
        .unwrap();
        let mut history = crate::history::HistoryService::start(&source).unwrap();
        let stage = plan.staging_path().unwrap();
        history.snapshot_to(&stage).unwrap();
        history
            .flush_close(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(
            plan.publish(&stage, |path| if path == target {
                Err("synthetic failure".into())
            } else {
                Ok(())
            })
            .is_err()
        );
        assert_eq!(
            config::load_config(&target.join("config.toml")).unwrap(),
            old
        );
        assert!(!target.join("history.sqlite3").exists());
        assert!(stage.join("history.sqlite3").exists());
        drop(history);
        fs::remove_dir_all(dir).unwrap();
    }
}
