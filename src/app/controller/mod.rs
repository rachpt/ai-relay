//! 有界 GUI 桥接；本线程独占 Runtime、配置目录与代理句柄。
//!
//! 模块边界：
//! - `types`：命令、响应与只读快照；
//! - `results`：固定容量结果邮箱；
//! - `history`：独立历史控制线程；
//! - `session`：控制线程独占的状态与配置/日志/历史子域；
//! - `worker`：单入口命令循环与退出清理。
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
};

use super::{Options, instance::Control, instance::Signals};

mod history;
mod logging;
mod results;
mod session;
mod types;
mod util;
mod worker;

pub(crate) use results::ResultQueue;
pub(crate) use types::{
    AutostartResponse, Command, ConfigResponse, Event, HistoryResponse, LogResponse,
    LogViewSnapshot, OverviewSnapshot, WindowResponse,
};

use results::{event_overflow, overflow_message};
use worker::worker;

pub(crate) struct Controller {
    commands: SyncSender<Command>,
    pub events: ResultQueue<Event>,
    pub usage_events: Receiver<String>,
    pub overview_events: Receiver<OverviewSnapshot>,
    pub window_events: ResultQueue<WindowResponse>,
    pub log_snapshots: Receiver<LogViewSnapshot>,
    pub log_events: ResultQueue<LogResponse>,
    pub autostart_events: ResultQueue<AutostartResponse>,
    pub config_events: ResultQueue<ConfigResponse>,
    pub history_events: ResultQueue<HistoryResponse>,
    exit: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}
impl Controller {
    pub fn spawn(
        options: Options,
        ipc: Receiver<Control>,
        signals: Arc<Signals>,
    ) -> Result<Self, String> {
        let (commands, receiver) = mpsc::sync_channel(4);
        let events = ResultQueue::new(event_overflow);
        let event_receiver = events.clone();
        let (usage_events, usage_receiver) = mpsc::sync_channel(1);
        let (overview_events, overview_receiver) = mpsc::sync_channel(1);
        let window_events =
            ResultQueue::new(|_, count| WindowResponse::Error(overflow_message(count)));
        let window_receiver = window_events.clone();
        let (log_snapshots, log_snapshot_receiver) = mpsc::sync_channel(1);
        let log_events = ResultQueue::new(|_, count| LogResponse::Error(overflow_message(count)));
        let log_event_receiver = log_events.clone();
        let autostart_events =
            ResultQueue::new(|_, count| AutostartResponse::Error(overflow_message(count)));
        let config_events =
            ResultQueue::new(|_, count| ConfigResponse::Error(overflow_message(count)));
        let history_events =
            ResultQueue::new(|latest: &HistoryResponse, count| HistoryResponse::Error {
                directory: latest.directory(),
                message: overflow_message(count),
            });
        let action_events = (
            autostart_events.clone(),
            config_events.clone(),
            history_events.clone(),
        );
        let exit = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let worker_exit = exit.clone();
        let worker_finished = finished.clone();
        thread::Builder::new()
            .name("ai-relay-controller".into())
            .spawn(move || {
                struct Finished(Arc<AtomicBool>);
                impl Drop for Finished {
                    fn drop(&mut self) {
                        self.0.store(true, Ordering::Release);
                    }
                }
                let _finished = Finished(worker_finished);
                worker(
                    options,
                    receiver,
                    events,
                    usage_events,
                    worker_exit,
                    ipc,
                    signals,
                    (log_snapshots, log_events),
                    action_events,
                    overview_events,
                    window_events,
                );
            })
            .map_err(|_| "无法创建后台控制线程。".to_string())?;
        Ok(Self {
            commands,
            events: event_receiver,
            usage_events: usage_receiver,
            overview_events: overview_receiver,
            window_events: window_receiver,
            log_snapshots: log_snapshot_receiver,
            log_events: log_event_receiver,
            autostart_events,
            config_events,
            history_events,
            exit,
            finished,
        })
    }

    pub fn send(&self, command: Command) -> Result<(), String> {
        if self.exit.load(Ordering::Acquire) {
            return Err("程序正在退出。".into());
        }
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(_) => "后台命令队列已满，请稍后重试。".into(),
                TrySendError::Disconnected(_) => "后台线程已结束，请退出并重新启动程序。".into(),
            })
    }
    pub fn request_exit(&self) {
        self.exit.store(true, Ordering::Release);
    }
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.request_exit();
    }
}

#[cfg(test)]
mod tests;
