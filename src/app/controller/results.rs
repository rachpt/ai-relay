//! 固定容量操作结果邮箱。
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
};

use super::types::{Event, LogResponse};

// 锁内只有有界内存操作，不等待消费者；溢出通知与计数独立于队列，不能被后续
// 结果挤掉。先通知缺口，再读取保留的结果。
pub(crate) struct ResultQueue<T> {
    state: Arc<Mutex<ResultState<T>>>,
    overflow: fn(&T, u64) -> T,
}
struct ResultState<T> {
    entries: VecDeque<T>,
    dropped: u64,
}
impl<T> Clone for ResultQueue<T> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            overflow: self.overflow,
        }
    }
}
impl<T> ResultQueue<T> {
    pub(crate) fn new(overflow: fn(&T, u64) -> T) -> Self {
        Self {
            state: Arc::new(Mutex::new(ResultState {
                entries: VecDeque::with_capacity(8),
                dropped: 0,
            })),
            overflow,
        }
    }
    pub(crate) fn publish(&self, value: T) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.entries.len() == 8 {
            state.entries.pop_front();
            state.dropped = state.dropped.saturating_add(1);
        }
        state.entries.push_back(value);
    }
    pub fn try_recv(&self) -> Result<T, mpsc::TryRecvError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.dropped != 0
            && let Some(latest) = state.entries.back()
        {
            let notice = (self.overflow)(latest, state.dropped);
            state.dropped = 0;
            return Ok(notice);
        }
        state.entries.pop_front().ok_or(mpsc::TryRecvError::Empty)
    }
    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> (usize, u64) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (state.entries.len(), state.dropped)
    }
}
pub(crate) fn overflow_message(count: u64) -> String {
    format!(
        "后台结果队列溢出，{count} 条操作结果已丢失；操作可能已执行，请重新加载配置或查询状态，勿直接重试保存或删除。"
    )
}
pub(crate) fn event_overflow(latest: &Event, count: u64) -> Event {
    Event {
        config: None,
        directory: latest.directory.clone(),
        directory_locked: latest.directory_locked,
        running: latest.running,
        message: overflow_message(count),
    }
}
pub(crate) fn publish(sender: &ResultQueue<Event>, event: Event, _exit: &AtomicBool) {
    sender.publish(event);
}
pub(crate) fn publish_log(
    sender: &ResultQueue<LogResponse>,
    response: LogResponse,
    _exit: &AtomicBool,
) {
    sender.publish(response);
}
