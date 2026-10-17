use std::{
    fmt,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy)]
pub struct DirectoryInputs<'a> {
    pub explicit_directory: Option<&'a Path>,
    pub executable_directory: &'a Path,
    /// 已由宿主通过 Known Folder API 解析到应用专属目录。
    pub standard_directory: &'a Path,
    pub portable_marker_exists: bool,
    pub explicit_config_exists: bool,
    pub portable_config_exists: bool,
    pub standard_config_exists: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectorySource {
    Explicit,
    Portable,
    Standard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectorySelection {
    pub directory: PathBuf,
    pub source: DirectorySource,
    pub needs_initialization: bool,
}
impl DirectorySelection {
    pub fn config_path(&self) -> PathBuf {
        self.directory.join("config.toml")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryError;
impl fmt::Display for DirectoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("configuration directories must be resolved absolute paths")
    }
}
impl std::error::Error for DirectoryError {}

/// 纯选择：不检查磁盘、不创建目录、不以 cwd 补全路径、不在失败时回退。
/// 文件存在不代表有效；选中后必须单独 load_config，损坏也不得转用另一份。
pub fn select_directory(input: DirectoryInputs<'_>) -> Result<DirectorySelection, DirectoryError> {
    if !input.executable_directory.is_absolute()
        || !input.standard_directory.is_absolute()
        || input
            .explicit_directory
            .is_some_and(|path| !path.is_absolute())
    {
        return Err(DirectoryError);
    }
    let (directory, source, exists) = if let Some(path) = input.explicit_directory {
        (
            path,
            DirectorySource::Explicit,
            input.explicit_config_exists,
        )
    } else if input.portable_marker_exists {
        (
            input.executable_directory,
            DirectorySource::Portable,
            input.portable_config_exists,
        )
    } else {
        (
            input.standard_directory,
            DirectorySource::Standard,
            input.standard_config_exists,
        )
    };
    Ok(DirectorySelection {
        directory: directory.to_path_buf(),
        source,
        needs_initialization: !exists,
    })
}
