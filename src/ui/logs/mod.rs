//! 日志页面仅消费后台返回的文本和状态，不创建 LogService、不访问磁盘。
//! 设置即时应用不代表持久化；正常保存配置才写盘。
use super::*;
use crate::{
    app::controller::{Command, LogResponse, LogViewSnapshot},
    config::{LogLevel, LoggingConfig},
    logging::{FileCursor, LogEntry, LogFile, PageCursor, ReadResponse},
};
use windows_sys::Win32::{Foundation::HINSTANCE, Graphics::Gdi::HFONT};

mod filter;
mod resize;
mod scrollbars;
use filter::{
    Filters, LIMIT, MAX_LINES, bounded_file_text, filter, filter_live, local_bound, status_range,
};
pub(in crate::ui) use resize::CHANGED as HEIGHT_CHANGED;
pub(in crate::ui) use resize::{apply as apply_grip, draw};
#[cfg(test)]
mod tests;

// No State references may cross confirmation dialogs, which run nested message loops.
pub(super) unsafe fn poll(hwnd: HWND, state: *mut State) {
    // SAFETY: Stable owner-thread State pointer; borrows end before every modal call.
    unsafe {
        if let Ok(snapshot) = (*state).controller.log_snapshots.try_recv() {
            let logs = &mut (*state).logs;
            if !logs.initialized && snapshot.directory.is_some() {
                logs.populate_options(&snapshot.logging);
            }
            let first = snapshot.entries.first().map(|entry| entry.sequence);
            if logs.displayed_cursor != 0
                && first.is_some_and(|n| n > logs.displayed_cursor.saturating_add(1))
            {
                logs.gap = true;
            }
            logs.gap |= snapshot.gap;
            logs.initialized = snapshot.directory.is_some() && !snapshot.status.worker_stopped;
            logs.snapshot = Some(snapshot);
        }
        // Bounded result queue; one result per tick also limits modal confirmations.
        if let Ok(response) = (*state).controller.log_events.try_recv() {
            if let LogResponse::ClearPreview { all, files, bytes } = response {
                let message = format!(
                    "将删除{}：{files} 个文件 / {bytes} bytes。\n仅限当前日志目录内本应用管理的日志；不删除配置、密钥、统计或旧版日志。\n后台执行时范围可能因轮转变化。确认继续？",
                    if all {
                        "全部应用日志（含当前文件）"
                    } else {
                        "已关闭的历史日志（不含当前文件）"
                    }
                );
                let accepted = settings::confirm(hwnd, &message);
                let accepted = accepted
                    && (!all
                        || settings::confirm(
                            hwnd,
                            "再次确认清空全部应用日志？当前日志将由后台关闭／重开，新日志仍可继续产生。此操作不可撤销。",
                        ));
                if (*state).exiting.is_none() && accepted {
                    let command = if all {
                        Command::LogClearAll { confirmed: true }
                    } else {
                        Command::LogClearHistory { confirmed: true }
                    };
                    (*state).logs.send(&(*state).controller, command);
                } else {
                    (*state).logs.pending = false;
                    (*state).logs.message = "已取消删除，未发送执行命令".into();
                }
            } else if let Some(logging) = (*state).logs.response(response) {
                (*state).settings.sync_logging(logging);
            }
        }
        (*state).logs.visible = (*state).tabs.active == 2 && IsWindowVisible(hwnd) != 0;
        if (*state).exiting.is_none() {
            (*state).logs.tick(&(*state).controller);
        }
        (*state).logs.enable();
        (*state).logs.render();
    }
}

pub(super) const CONTENT: u32 = 2200;
#[derive(Default)]
pub(super) struct Logs {
    controls: Vec<(HWND, i32, i32, i32, i32)>,
    content: HWND,
    resize_grip: HWND,
    viewer_height: Option<i32>,
    snapshot: Option<LogViewSnapshot>,
    live_text: String,
    live_entries: Vec<LogEntry>,
    files: Vec<LogFile>,
    file_text: String,
    file_name: Option<String>,
    file_mode: bool,
    before: Option<FileCursor>,
    follow: Option<FileCursor>,
    read_kind: Option<u32>,
    last_follow: Option<Instant>,
    paused: bool,
    clear_cursor: u64,
    displayed_cursor: u64,
    gap: bool,
    pending: bool,
    initialized: bool,
    visible: bool,
    rendered: String,
    message: String,
    detail: String,
}
impl Logs {
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        parent: HWND,
        instance: HINSTANCE,
        class: &str,
        title: &str,
        style: u32,
        id: u32,
        row: (i32, i32, i32, i32),
    ) -> Result<HWND, String> {
        // SAFETY: Live parent/module; strings are copied during creation; id is a child identifier.
        let control = unsafe {
            CreateWindowExW(
                if class == "EDIT" && style & WS_BORDER != 0 {
                    WS_EX_CLIENTEDGE
                } else {
                    0
                },
                wide(class).as_ptr(),
                wide(title).as_ptr(),
                WS_CHILD | style,
                0,
                0,
                0,
                0,
                parent,
                id as usize as HMENU,
                instance,
                null(),
            )
        };
        if control.is_null() {
            return Err(last_error("创建日志控件"));
        }
        self.controls.push((control, row.0, row.1, row.2, row.3));
        Ok(control)
    }
    pub fn create(&mut self, parent: HWND, instance: HINSTANCE) -> Result<(), String> {
        self.add(
            parent,
            instance,
            "STATIC",
            "等待后台日志状态；未初始化时请先正常保存配置。",
            0,
            2220,
            (24, 64, 700, 40),
        )?;
        let button = WS_TABSTOP;
        for (title, id, x, w) in [
            ("实时视图", 2201, 24, 100),
            ("暂停显示", 2202, 132, 100),
            ("文件视图", 2203, 240, 100),
            ("清空视图", 2204, 348, 100),
            ("↻ 刷新文件列表", 2205, 456, 132),
            ("读取尾部", 2206, 596, 124),
        ] {
            self.add(
                parent,
                instance,
                "BUTTON",
                title,
                button,
                id,
                (x, 108, w, 30),
            )?;
        }
        self.add(
            parent,
            instance,
            "STATIC",
            "搜索 / 级别 / 文件",
            0,
            0,
            (24, 150, 160, 26),
        )?;
        self.add(
            parent,
            instance,
            "EDIT",
            "",
            WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            2207,
            (184, 146, 210, 30),
        )?;
        for (id, x, w, items) in [
            (
                2208,
                404,
                104,
                vec!["全部级别", "trace", "debug", "info", "warn", "error"],
            ),
            (2209, 518, 202, vec!["尚未加载文件列表"]),
        ] {
            let combo = self.add(
                parent,
                instance,
                "COMBOBOX",
                "",
                WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32,
                id,
                (x, 146, w, 180),
            )?;
            // SAFETY: Live COMBOBOX copies each string; selection index exists.
            unsafe {
                for item in items {
                    SendMessageW(combo, CB_ADDSTRING, 0, wide(item).as_ptr() as isize);
                }
                SendMessageW(combo, CB_SETCURSEL, 0, 0);
            }
        }
        self.add(
            parent,
            instance,
            "STATIC",
            "本地日期：开始（左）/ 结束（右），含首尾整日；取消勾选不限",
            0,
            0,
            (24, 182, 696, 22),
        )?;
        for (id, x, label) in [(2224, 24, "开始（含）"), (2225, 372, "结束（含）")] {
            let picker = date_picker::create(parent, id, label)?;
            self.controls.push((picker, x, 208, 336, 28));
        }
        let category = choices::create(parent, 2226, &["全部", "运行", "访问"])?;
        self.controls.push((category, 24, 244, 156, 28));
        self.add(
            parent,
            instance,
            "STATIC",
            "状态码 / 2xx / 400-499",
            0,
            0,
            (192, 248, 208, 24),
        )?;
        let status = self.add(
            parent,
            instance,
            "EDIT",
            "",
            WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            2227,
            (404, 244, 128, 28),
        )?;
        // SAFETY: Live EDIT; no retained pointers.
        unsafe {
            SendMessageW(status, 0x00c5, 32, 0);
        }
        self.add(
            parent,
            instance,
            "BUTTON",
            "仅错误（AND）",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            2228,
            (544, 244, 176, 28),
        )?;
        self.add(parent, instance, "STATIC", "", 0, 2229, (24, 280, 696, 40))?;
        self.content = self.add(parent, instance, "EDIT",
            "等待后台快照。只读内容支持选中及 Ctrl+C。\r\n文件视图每秒异步跟随追加，支持向前分页；暂停时不请求追加。",
            WS_TABSTOP | WS_BORDER | WS_VSCROLL | WS_HSCROLL |
                (ES_READONLY | ES_MULTILINE | ES_AUTOVSCROLL | ES_AUTOHSCROLL) as u32,
            CONTENT, (24, 324, 696, 96))?;
        // SAFETY: Live EDIT; fixed input limit, no retained pointers.
        unsafe {
            SendMessageW(self.content, 0x00c5, LIMIT, 0);
            scrollbars::attach(self.content)?;
        }
        self.resize_grip = resize::create(parent, instance)?;
        self.controls.push((self.resize_grip, 24, 422, 696, 12));
        for (title, id, x, w) in [
            ("删除历史日志…", 2210, 24, 180),
            ("清空全部应用日志…", 2211, 216, 220),
            ("← 向前一页", 2222, 448, 124),
            ("打开日志目录", 2223, 584, 136),
        ] {
            self.add(
                parent,
                instance,
                "BUTTON",
                title,
                button,
                id,
                (x, 432, w, 30),
            )?;
        }
        self.add(
            parent,
            instance,
            "STATIC",
            "暂停不停止记录；向前分页会暂停跟随。打开目录由后台执行，不阻塞界面。",
            0,
            0,
            (24, 468, 696, 30),
        )?;
        self.add(
            parent,
            instance,
            "STATIC",
            "日志设置（暂放本页）：应用仅即时生效，正常保存设置后才持久化",
            0,
            0,
            (24, 504, 696, 26),
        )?;
        for (title, id, x) in [("文件日志", 2212, 24), ("访问日志", 2213, 180)] {
            self.add(
                parent,
                instance,
                "BUTTON",
                title,
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                id,
                (x, 538, 144, 28),
            )?;
        }
        self.add(
            parent,
            instance,
            "STATIC",
            "级别 / 单文件 MiB / 文件数（含当前）/ 保留天数 / 缓存条数",
            0,
            0,
            (24, 578, 696, 26),
        )?;
        for (id, title, x, w) in [
            (2214, "info", 24, 104),
            (2215, "5", 140, 104),
            (2216, "3", 256, 104),
            (2217, "7", 372, 104),
            (2218, "1000", 488, 104),
        ] {
            self.add(
                parent,
                instance,
                "EDIT",
                title,
                WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
                id,
                (x, 612, w, 30),
            )?;
        }
        self.add(
            parent,
            instance,
            "BUTTON",
            "应用日志设置",
            button,
            2219,
            (24, 656, 180, 30),
        )?;
        self.add(
            parent,
            instance,
            "EDIT",
            "目录、占用、落盘错误、丢弃及缓存缺口：等待后台状态。",
            WS_TABSTOP
                | WS_BORDER
                | WS_VSCROLL
                | (ES_READONLY | ES_MULTILINE | ES_AUTOVSCROLL) as u32,
            2221,
            (24, 698, 696, 46),
        )?;

        self.enable();
        Ok(())
    }
    fn control(&self, id: u32) -> HWND {
        self.controls
            .iter()
            .find(|(h, ..)| {
                // SAFETY: All handles are live child controls on this thread.
                unsafe { GetDlgCtrlID(*h) == id as i32 }
            })
            .map_or(null_mut(), |(h, ..)| *h)
    }
    pub fn populate_options(&mut self, options: &LoggingConfig) {
        for (id, value) in [
            (2214, format!("{:?}", options.level).to_lowercase()),
            (2215, (options.max_file_bytes / (1024 * 1024)).to_string()),
            (2216, options.retained_files.to_string()),
            (2217, options.retention_days.to_string()),
            (2218, options.recent_entries.to_string()),
        ] {
            settings::set_text(self.control(id), &value);
        }
        // SAFETY: Live checkbox controls, no pointers retained.
        unsafe {
            SendMessageW(
                self.control(2212),
                BM_SETCHECK,
                usize::from(options.file_enabled),
                0,
            );
            SendMessageW(
                self.control(2213),
                BM_SETCHECK,
                usize::from(options.access_log),
                0,
            );
        }
    }
    fn draft(&self) -> Result<LoggingConfig, String> {
        let number = |id| {
            settings::text(self.control(id))
                .trim()
                .parse::<u32>()
                .map_err(|_| "日志设置数值无效。".to_string())
        };
        let level = match settings::text(self.control(2214))
            .trim()
            .to_lowercase()
            .as_str()
        {
            "trace" => LogLevel::Trace,
            "debug" => LogLevel::Debug,
            "info" => LogLevel::Info,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            "off" => LogLevel::Off,
            _ => return Err("日志级别须为 trace/debug/info/warn/error/off。".into()),
        };
        // SAFETY: Live checkboxes; messages have no pointer arguments.
        let (file_enabled, access_log) = unsafe {
            (
                SendMessageW(self.control(2212), BM_GETCHECK, 0, 0) != 0,
                SendMessageW(self.control(2213), BM_GETCHECK, 0, 0) != 0,
            )
        };
        Ok(LoggingConfig {
            level,
            file_enabled,
            access_log,
            max_file_bytes: u64::from(number(2215)?) * 1024 * 1024,
            retained_files: number(2216)?,
            retention_days: number(2217)?,
            recent_entries: number(2218)?,
        })
    }
    fn enable(&self) {
        for &(control, ..) in &self.controls {
            // SAFETY: Owner-thread child controls; no modal UI or State access from notifications.
            unsafe {
                let id = GetDlgCtrlID(control);
                if (2201..=2219).contains(&id) || id == 2222 || id == 2223 {
                    let enabled = match id {
                        2201 | 2202 | 2204 | 2207 | 2208 => true,
                        2209 => !self.pending && !self.files.is_empty(),
                        2206 => !self.pending && !self.files.is_empty() && self.initialized,
                        2222 => !self.pending && self.file_mode && self.before.is_some(),
                        2223 => self.initialized,
                        _ => !self.pending && self.initialized,
                    };
                    settings::EnableWindow(control, i32::from(enabled));
                }
            }
        }
    }
    fn send(&mut self, controller: &Controller, command: Command) {
        match controller.send(command) {
            Ok(()) => {
                self.pending = true;
                self.message = "日志后台操作中……".into();
            }
            Err(error) => self.message = error,
        }
        self.enable();
    }
    fn selected_file(&self) -> Option<String> {
        self.files
            .get(settings::selected(self.control(2209)))
            .map(|f| f.name.clone())
    }
    // IDs 2224–2228 are sampled by the existing UI timer, so no parent dispatch
    // expansion or nested message-loop callbacks are required.
    fn filters(&self) -> Result<Filters, String> {
        let start = local_bound(&settings::text(self.control(2224)), false)?;
        let end = local_bound(&settings::text(self.control(2225)), true)?;
        let status = status_range(&settings::text(self.control(2227)))?;
        // SAFETY: Live checkbox owned by this page; message contains no pointers.
        let errors_only = unsafe { SendMessageW(self.control(2228), BM_GETCHECK, 0, 0) != 0 };
        Ok(Filters {
            search: settings::text(self.control(2207)).to_lowercase(),
            level: settings::selected(self.control(2208)),
            start,
            end,
            status,
            errors_only,
            category: settings::selected(self.control(2226)),
        })
    }
    fn clear_view(&mut self) {
        self.clear_cursor = self.snapshot.as_ref().map_or(0, |s| s.cursor);
        self.file_text.clear();
        self.live_text.clear();
        self.live_entries.clear();
        self.rendered.clear();
        settings::set_text(self.content, "");
        self.message = "已清空视图，未删除文件".into();
    }
    fn render(&mut self) {
        if !self.visible {
            return;
        }
        if !self.file_mode && !self.paused {
            self.live_text = self
                .snapshot
                .as_ref()
                .map_or_else(String::new, |s| bounded_file_text("", &s.text, true));
            self.live_entries = self.snapshot.as_ref().map_or_else(Vec::new, |s| {
                s.entries[s.entries.len().saturating_sub(MAX_LINES)..].to_vec()
            });
        }
        let result = self.filters().and_then(|filters| {
            if self.file_mode {
                filter(&self.file_text, &filters)
            } else {
                filter_live(
                    &self.live_text,
                    &self.live_entries,
                    self.clear_cursor,
                    &filters,
                )
            }
        });
        let error = result
            .as_ref()
            .err()
            .map(|e| format!("筛选无效：{e}；保留原显示"));
        let hint = error.as_deref().unwrap_or("本地时间含偏移，保留 UTC epoch；JSON 增加 timestamp_local。条件 AND；仅错误含 error/错误码/4xx/5xx。上限 1000 行/256 KiB，文件仅筛已加载窗口。");
        if settings::text(self.control(2229)) != hint {
            settings::set_text(self.control(2229), hint);
        }
        if let Ok(filtered) = result
            && filtered != self.rendered
        {
            // SAFETY: Live readonly EDIT. Preserve selection and vertical viewport; never force follow.
            unsafe {
                let mut start = 0u32;
                let mut end = 0u32;
                SendMessageW(
                    self.content,
                    0xB0,
                    (&mut start as *mut u32) as usize,
                    (&mut end as *mut u32) as isize,
                );
                let line = SendMessageW(self.content, 0xCE, 0, 0);
                settings::set_text(self.content, &filtered);
                SendMessageW(self.content, 0xB1, start as usize, end as isize);
                let current = SendMessageW(self.content, 0xCE, 0, 0);
                SendMessageW(self.content, 0xB6, 0, line - current);
            }
            self.rendered = filtered;
        }
        let snapshot = self.snapshot.as_ref();
        if !self.paused && !self.file_mode && error.is_none() {
            self.displayed_cursor = snapshot.map_or(0, |s| s.cursor);
        }
        let status = format!(
            "{}；{}；{}",
            if self.file_mode {
                if self.paused {
                    "文件分页／跟随已暂停"
                } else {
                    "文件每秒自动跟随（不强制滚动）"
                }
            } else if self.paused {
                "实时已暂停显示（后台继续记录）"
            } else {
                "实时视图（保留选区，不自动滚动）"
            },
            if self.gap {
                "缓存缺口"
            } else {
                "有界缓存"
            },
            error.as_deref().unwrap_or(&self.message)
        );
        if settings::text(self.control(2220)) != status {
            settings::set_text(self.control(2220), &status);
        }
        let detail = snapshot.map_or_else(|| "等待日志快照".into(), |s| format!(
            "目录：{}；文件列表 {} 个 / {} bytes\r\n丢弃 {}；磁盘错误 {} ({:?})；已落盘 {:?} / cursor {}；停止 {}；{}",
            s.directory.as_ref().map_or_else(|| "未初始化".into(), |p| p.display().to_string()),
            self.files.len(), self.files.iter().map(|f| f.bytes).sum::<u64>(), s.status.dropped,
            s.status.disk_errors, s.status.last_disk_error, s.status.last_persisted_sequence,
            s.cursor, s.status.worker_stopped, self.detail));
        if settings::text(self.control(2221)) != detail {
            settings::set_text(self.control(2221), &detail);
        }
    }
    pub fn show(&mut self, visible: bool) {
        self.visible = visible;
        let mut shows = ShowBatch::with_capacity(self.controls.len());
        for &(control, ..) in &self.controls {
            shows.add(control, visible);
        }
        shows.commit();
        self.render();
    }
    fn response(&mut self, response: LogResponse) -> Option<LoggingConfig> {
        self.pending = false;
        match response {
            LogResponse::Files(files) => {
                let selected = self.selected_file();
                self.files = files;
                // SAFETY: Live COMBOBOX; string copies complete synchronously.
                unsafe {
                    let combo = self.control(2209);
                    SendMessageW(combo, CB_RESETCONTENT, 0, 0);
                    for file in &self.files {
                        SendMessageW(combo, CB_ADDSTRING, 0, wide(&file.name).as_ptr() as isize);
                    }
                    let index = self
                        .files
                        .iter()
                        .position(|f| Some(&f.name) == selected.as_ref())
                        .unwrap_or(0);
                    SendMessageW(combo, CB_SETCURSEL, index, 0);
                }
                self.message = format!("已刷新 {} 个文件；选中文件后读取尾部", self.files.len());
            }
            LogResponse::Read { name, response } => {
                if self.file_name.as_ref() == Some(&name) {
                    match response {
                        ReadResponse::Page(page) => {
                            let kind = self.read_kind.take().unwrap_or(2206);
                            if kind == 2222 {
                                self.file_text =
                                    bounded_file_text(&page.text, &self.file_text, false);
                                self.before = page.before;
                            } else if kind == 2224 {
                                self.file_text =
                                    bounded_file_text(&self.file_text, &page.text, true);
                                self.follow = Some(page.follow);
                            } else {
                                self.file_text = bounded_file_text("", &page.text, true);
                                self.before = page.before;
                                self.follow = Some(page.follow);
                            }
                            self.message = format!(
                                "{name} / {} bytes；更多 {}；片段裁剪 {}；UTF-8 损失 {}",
                                page.file_bytes, page.has_more, page.truncated, page.utf8_loss
                            );
                        }
                        ReadResponse::Changed(change) => self.file_changed(format!(
                            "文件身份／长度变化：{change:?}；旧游标已清除，请重新读取尾部"
                        )),
                        ReadResponse::Missing => {
                            self.file_changed("文件已删除／缺失；旧游标已清除，请刷新列表".into())
                        }
                    }
                }
            }
            LogResponse::FileChanged { name, change } => {
                if self.file_name.as_ref() == Some(&name) {
                    self.file_changed(format!(
                        "{name} 已变化：{change:?}；停止跟随，请重新读取尾部"
                    ));
                }
            }
            LogResponse::FileMissing { name } => {
                if self.file_name.as_ref() == Some(&name) {
                    self.file_changed(format!("{name} 已缺失；停止跟随，请刷新列表"));
                }
            }
            LogResponse::Tail { name, tail } => {
                if self.file_name.as_ref() == Some(&name) {
                    self.file_text = bounded_file_text("", &tail.text, true);
                    self.message = format!(
                        "{} / {} bytes；尾部截断 {}；未自动跟随",
                        name, tail.file_bytes, tail.truncated
                    );
                }
            }
            LogResponse::Cleared { all, report } => {
                self.file_text.clear();
                self.file_name = None;
                self.before = None;
                self.follow = None;
                self.message = format!(
                    "{}：已删除 {} 文件 / {} bytes；失败 {} 项；允许新日志继续产生",
                    if all {
                        "全部应用日志清理"
                    } else {
                        "历史清理"
                    },
                    report.deleted_files,
                    report.deleted_bytes,
                    report.errors.len()
                );
                self.detail = report
                    .errors
                    .iter()
                    .map(|f| format!("{}: {:?}", f.name, f.kind))
                    .collect::<Vec<_>>()
                    .join("；");
                self.files.clear();
                // SAFETY: Live COMBOBOX; deletion invalidates all file selections.
                unsafe {
                    SendMessageW(self.control(2209), CB_RESETCONTENT, 0, 0);
                }
            }
            LogResponse::OptionsApplied { logging, persisted } => {
                self.message = if persisted {
                    "日志设置已持久化"
                } else {
                    "日志设置已应用，未持久化；请到设置页正常保存"
                }
                .into();
                self.enable();
                return Some(logging);
            }
            LogResponse::Error(error) => {
                if self.read_kind.take().is_some() {
                    self.paused = true;
                    settings::set_text(self.control(2202), "恢复显示");
                }
                self.message = format!("日志操作失败：{error}；读取跟随已暂停，可手动重试");
            }
            LogResponse::ClearPreview { .. } => {}
        }
        self.enable();
        None
    }
    pub fn handle(&mut self, controller: &Controller, id: u32) {
        if self.pending && ![2201, 2202, 2204, 2207, 2208].contains(&id) {
            return;
        }
        match id {
            2201 => {
                self.file_mode = false;
                self.paused = false;
                settings::set_text(self.control(2202), "暂停显示");
            }
            2202 => {
                self.paused = !self.paused;
                settings::set_text(
                    self.control(2202),
                    if self.paused {
                        "恢复显示"
                    } else {
                        "暂停显示"
                    },
                );
            }
            2204 => self.clear_view(),
            2203 | 2205 => self.send(controller, Command::LogListFiles),
            2206 | 2209 => {
                if let Some(name) = self.selected_file() {
                    self.file_mode = true;
                    self.file_name = Some(name.clone());
                    self.file_text.clear();
                    self.before = None;
                    self.follow = None;
                    self.paused = false;
                    self.read_kind = Some(2206);
                    self.last_follow = Some(Instant::now());
                    settings::set_text(self.control(2202), "暂停显示");
                    self.send(
                        controller,
                        Command::LogReadPage {
                            name,
                            cursor: PageCursor::Tail,
                        },
                    );
                }
            }
            2222 => {
                if let (Some(name), Some(cursor)) = (self.file_name.clone(), self.before.clone()) {
                    self.paused = true;
                    settings::set_text(self.control(2202), "恢复显示");
                    self.read_kind = Some(2222);
                    self.send(
                        controller,
                        Command::LogReadPage {
                            name,
                            cursor: PageCursor::Before(cursor),
                        },
                    );
                }
            }
            2223 => {
                if let Err(error) = controller.send(Command::OpenLogDirectory) {
                    self.message = error;
                }
            }
            2210 => self.send(controller, Command::LogClearHistory { confirmed: false }),
            2211 => self.send(controller, Command::LogClearAll { confirmed: false }),
            2219 => match self.draft() {
                Ok(logging) => self.send(controller, Command::LogApplyOptions { logging }),
                Err(error) => self.message = error,
            },
            _ => {}
        }
        self.render();
    }
    fn file_changed(&mut self, message: String) {
        self.before = None;
        self.follow = None;
        self.read_kind = None;
        self.paused = true;
        settings::set_text(self.control(2202), "恢复显示");
        self.message = message;
    }
    fn tick(&mut self, controller: &Controller) {
        if !self.visible
            || !self.file_mode
            || self.paused
            || self.pending
            || self
                .last_follow
                .is_some_and(|last| last.elapsed().as_secs() < 1)
        {
            return;
        }
        if let (Some(name), Some(cursor)) = (self.file_name.clone(), self.follow.clone()) {
            self.last_follow = Some(Instant::now());
            self.read_kind = Some(2224);
            self.send(controller, Command::LogReadFollow { name, cursor });
        }
    }
    pub fn set_font(&self, font: HFONT) {
        for &(control, ..) in &self.controls {
            // SAFETY: State owns the live font until controls are destroyed or updated.
            unsafe {
                SendMessageW(control, WM_SETFONT, font as usize, 0);
            }
        }
    }
    pub fn cancel_resize(&self) {
        // SAFETY: Owner-thread UI child; no controller/disk/network work.
        unsafe {
            resize::cancel(self.resize_grip);
        }
    }

    pub fn viewer_height(&self) -> i32 {
        self.viewer_height.unwrap_or(resize::DEFAULT_HEIGHT)
    }

    pub fn set_viewer_height(&mut self, height: i32) -> bool {
        let height = resize::clamp(height);
        if self.viewer_height() == height {
            return false;
        }
        self.viewer_height = Some(height);
        true
    }

    pub fn page_height(&self, dpi: u32, height: i32) -> i32 {
        height.max(scale(
            656 + self.viewer_height() + status_bar::HEIGHT + viewport::BOTTOM_PADDING,
            dpi,
        ))
    }

    pub fn layout(&self, dpi: u32, width: i32, height: i32) {
        // Resizing the main window must not silently replace the user's chosen
        // viewer height. Extra space expands only the final read-only detail.
        let viewer_height = self.viewer_height();
        let shift = scale(viewer_height - 96 + 8, dpi);
        let document_bottom = scale(656 + viewer_height, dpi);
        let detail_extra = (viewport::content_bottom(dpi, height) - document_bottom).max(0);
        // SAFETY: Live grip, scalar presentation update without native callbacks.
        unsafe {
            resize::set_height(self.resize_grip, viewer_height, dpi);
        }
        let mut moves = MoveBatch::with_capacity(self.controls.len());
        for &(control, x, y, w, h) in &self.controls {
            let wide = w == 696 || w == 700;
            let (y, h) = if control == self.content {
                (scale(324, dpi), scale(viewer_height, dpi))
            } else if control == self.resize_grip {
                (scale(324 + viewer_height + 2, dpi), scale(12, dpi))
            } else {
                (
                    scale(y, dpi) + if y >= 432 { shift } else { 0 },
                    scale(h, dpi)
                        + if unsafe { GetDlgCtrlID(control) } == 2221 {
                            detail_extra
                        } else {
                            0
                        },
                )
            };
            moves.add(
                control,
                scale(x, dpi),
                y,
                if wide {
                    (width - scale(48, dpi)).max(1)
                } else {
                    scale(w, dpi)
                },
                h,
            );
        }
        moves.commit();
    }
}
