//! 有界、类型化日志后台。`LogService` 的阻塞方法只允许创建它的后台 controller
//! 线程调用，不能在 GUI / 网络线程创建或使用。网络侧只持有 `LogHandle`。
//! `directory` 是已解析的绝对 `data/logs` 路径；不读取或迁移旧根目录日志。
//! 文件是 JSONL，时间是 UTC Unix 秒。没有自由文本事件、URL、正文或凭据字段。
mod files;
mod service;
mod types;

pub use service::{LogHandle, LogService};
pub use types::*;
