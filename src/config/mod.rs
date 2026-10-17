//! 纯配置基础。文件操作是阻塞操作，宿主必须在 GUI 线程外调用。
//! Windows 支持当前用户 DPAPI、原子保存、备份恢复和安全导入导出；不实现目录迁移或系统自启。
mod directory;
mod file;
mod model;
mod persistence;
mod secrets;
mod transfer;

pub use directory::*;
pub use file::*;
pub use model::*;
pub use persistence::*;
pub use secrets::{MAX_SECRET_BYTES, export_redacted_config};
pub use transfer::*;
