//! Windows 原生 AI Relay：代理、配置、历史分析与宿主的可测试模块边界。

pub mod app;
pub mod config;
pub mod history;
pub mod logging;
pub mod proxy;
pub mod usage;

#[cfg(windows)]
pub mod ui;
#[cfg(windows)]
pub mod windows;
