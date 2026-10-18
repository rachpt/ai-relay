//! Bounded, typed overview snapshots; clipboard and window placement are UI-only Win32 work.
use super::*;
use crate::{
    app::controller::{Command, OverviewSnapshot, WindowResponse},
    windows::preferences::WindowGeometry,
};
use std::{net::SocketAddr, path::PathBuf, time::Duration};
use windows_sys::Win32::{
    Foundation::GlobalFree,
    Graphics::Gdi::{
        GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
        MonitorFromWindow,
    },
    System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
};

pub(super) const RESTART: u32 = 2104;
pub(super) const COPY_URL: u32 = 2105;
pub(super) const OPEN_DIRECTORY: u32 = 2106;
const DETAILS: u32 = 2107;
const SUMMARY: u32 = 2108;

#[derive(Default)]
pub(super) struct Overview {
    details: HWND,
    summary: HWND,
    dashboard: HWND,
    timeline: overview_dashboard::Timeline,
    pub restart: HWND,
    copy_url: HWND,
    directory: HWND,
    snapshot: Option<OverviewSnapshot>,
    rendered: String,
    dirty: bool,
}
impl Overview {
    pub fn create(&mut self, hwnd: HWND) -> Result<(), String> {
        // SAFETY: Parent/module are live; each control copies its caption synchronously.
        unsafe {
            for (class, title, id, target) in [
                (
                    "STATIC",
                    "实际状态与会话指标：等待后台快照……",
                    DETAILS,
                    &mut self.details,
                ),
                ("STATIC", "等待运行状态……", SUMMARY, &mut self.summary),
                (
                    "STATIC",
                    "等待实际采样……",
                    overview_dashboard::CONTROL_ID,
                    &mut self.dashboard,
                ),
                ("BUTTON", "↻ 重启代理", RESTART, &mut self.restart),
                ("BUTTON", "复制 Base URL", COPY_URL, &mut self.copy_url),
                (
                    "BUTTON",
                    "打开数据目录",
                    OPEN_DIRECTORY,
                    &mut self.directory,
                ),
            ] {
                *target = CreateWindowExW(
                    0,
                    wide(class).as_ptr(),
                    wide(title).as_ptr(),
                    WS_CHILD
                        | if id == overview_dashboard::CONTROL_ID {
                            0x000d // SS_OWNERDRAW; native text remains available to accessibility.
                        } else if id == DETAILS || id == SUMMARY {
                            0
                        } else {
                            WS_TABSTOP
                        },
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    id as usize as HMENU,
                    GetModuleHandleW(null()),
                    null(),
                );
                if target.is_null() {
                    return Err(last_error("创建概览控件"));
                }
            }
        }
        Ok(())
    }
    pub fn show(&self, visible: bool) {
        for control in [
            self.summary,
            self.dashboard,
            self.restart,
            self.copy_url,
            self.directory,
        ] {
            // SAFETY: Live UI-owned child controls.
            unsafe {
                ShowWindow(control, if visible { SW_SHOW } else { SW_HIDE });
            }
        }
    }
    pub fn set_font(&self, font: windows_sys::Win32::Graphics::Gdi::HFONT) {
        for control in [
            self.details,
            self.summary,
            self.dashboard,
            self.restart,
            self.copy_url,
            self.directory,
        ] {
            // SAFETY: State retains both fonts until controls have switched.
            unsafe {
                SendMessageW(control, WM_SETFONT, font as usize, 0);
            }
        }
    }
    pub fn layout(&self, dpi: u32, width: i32, height: i32) {
        // SAFETY: Live sibling controls; no parent resizing or modal dispatch.
        unsafe {
            MoveWindow(
                self.summary,
                scale(24, dpi),
                scale(108, dpi),
                (width - scale(48, dpi)).max(1),
                scale(100, dpi),
                1,
            );
            for (control, x, y, w) in [
                (self.restart, 292, 220, 132),
                (self.copy_url, 432, 220, 140),
                (self.directory, 580, 220, 140),
            ] {
                MoveWindow(
                    control,
                    scale(24, dpi)
                        + (width - scale(48, dpi)) * (x - 24) / overview_dashboard::MIN_WIDTH_DIP,
                    scale(y, dpi),
                    ((width - scale(48, dpi)) * w / overview_dashboard::MIN_WIDTH_DIP).max(1),
                    scale(36, dpi),
                    1,
                );
            }
            MoveWindow(
                self.dashboard,
                scale(24, dpi),
                scale(282, dpi),
                (width - scale(48, dpi)).max(1),
                (height - scale(282 + 84, dpi)).max(1),
                1,
            );
        }
    }
    fn render(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let text = format_snapshot(snapshot);
        if text != self.rendered {
            settings::set_text(self.details, &text);
            self.rendered = text;
        }
        // Precise dashboard values/rates must not depend on the legacy summary's
        // rounded byte display or whole-second uptime deduplication.
        settings::set_text(self.summary, &format_header(snapshot));
        settings::set_text(self.dashboard, &self.timeline.description());
        // SAFETY: Queue drawing only while this page is visible; no I/O or
        // controller lock is acquired by the subsequent paint callback.
        unsafe {
            InvalidateRect(self.dashboard, null(), 0);
        }
    }
}

pub(super) unsafe fn draw(state: *mut State, l: LPARAM) -> bool {
    // SAFETY: WM_DRAWITEM's live item/DC and owner-thread State. All fonts and
    // report/timeline storage outlive this synchronous, read-only paint call.
    unsafe {
        let item = &*(l as *const windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT);
        if item.CtlID != overview_dashboard::CONTROL_ID {
            return false;
        }
        overview_dashboard::draw(
            item,
            (*state).dpi,
            (*state).font.0,
            (*state).heading_font.0,
            (*state).theme.palette,
            (*state).theme.kind,
            (*state).overview.snapshot.as_ref(),
            &(*state).overview.timeline,
        );
        true
    }
}

fn format_header(snapshot: &OverviewSnapshot) -> String {
    let seconds = snapshot.metrics.uptime.as_secs();
    format!(
        "{}    ·    本次会话 {:02}:{:02}:{:02}（含停止时间）\r\n监听：{}\r\n上游：{}\r\n本机 Base URL：{}",
        if snapshot.running {
            "● 正在运行"
        } else {
            "○ 已停止"
        },
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        snapshot
            .listen
            .map_or_else(|| "未监听".into(), |address| address.to_string()),
        snapshot
            .upstream
            .as_deref()
            .map(safe_upstream)
            .unwrap_or_else(|| "未配置／不可用".into()),
        snapshot
            .listen
            .filter(|_| snapshot.running)
            .map_or_else(|| "不可用".into(), base_url),
    )
}

fn base_url(mut address: SocketAddr) -> String {
    if address.ip().is_unspecified() {
        address.set_ip(if address.is_ipv4() {
            std::net::Ipv4Addr::LOCALHOST.into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        });
    }
    format!("http://{address}/v1")
}
fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut number = value as f64;
    let mut unit = 0;
    while number >= 1024.0 && unit < UNITS.len() - 1 {
        number /= 1024.0;
        unit += 1;
    }
    format!("{number:.1} {}", UNITS[unit])
}
fn format_snapshot(snapshot: &OverviewSnapshot) -> String {
    let m = &snapshot.metrics;
    let seconds = m.uptime.as_secs();
    let listen = snapshot
        .listen
        .map_or_else(|| "未监听".into(), |address| address.to_string());
    let upstream = snapshot
        .upstream
        .as_deref()
        .map(safe_upstream)
        .unwrap_or_else(|| "未配置／不可用".into());
    let local = snapshot
        .listen
        .filter(|_| snapshot.running)
        .map_or_else(|| "不可用".into(), base_url);
    format!(
        "实际监听：{listen}    状态：{}\r\n上游：{upstream}\r\n本机 Base URL：{local}\r\n活动请求：{}    活动隧道：{}    入站连接：{}\r\n累计请求：{}    错误：{}\r\n上传正文／隧道：{}    下载正文／隧道：{}\r\n程序会话时长：{} 天 {:02}:{:02}:{:02}{}\r\n本次程序会话累计，包含停止时间；流量不含 HTTP 头／帧，不等于账单。",
        if snapshot.running {
            "运行中"
        } else {
            "已停止"
        },
        m.active_requests,
        m.active_tunnels,
        m.inbound_connections,
        m.total_requests,
        m.errors,
        bytes(m.upload_body_bytes),
        bytes(m.download_body_bytes),
        seconds / 86400,
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
        if m.saturated {
            "（计数已达上限）"
        } else {
            ""
        },
    )
}
fn safe_upstream(value: &str) -> String {
    // Defense in depth: never render userinfo, query credentials or fragments.
    let Ok(mut url) = url::Url::parse(value) else {
        return "无效地址（已隐藏）".into();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

#[link(name = "user32")]
unsafe extern "system" {
    fn OpenClipboard(owner: HWND) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CloseClipboard() -> i32;
}
fn copy(hwnd: HWND, text: &str) -> Result<(), String> {
    let text = wide(text);
    // SAFETY: Movable shared memory is populated before transfer. Every failure frees
    // our allocation; successful SetClipboardData transfers ownership to the system.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, text.len() * size_of::<u16>());
        if memory.is_null() {
            return Err(last_error("分配剪贴板内存"));
        }
        let target = GlobalLock(memory);
        if target.is_null() {
            GlobalFree(memory);
            return Err(last_error("锁定剪贴板内存"));
        }
        std::ptr::copy_nonoverlapping(text.as_ptr(), target.cast::<u16>(), text.len());
        GlobalUnlock(memory);
        if OpenClipboard(hwnd) == 0 {
            GlobalFree(memory);
            return Err("剪贴板正忙，请重试。".into());
        }
        let result = if EmptyClipboard() != 0 && !SetClipboardData(13, memory).is_null() {
            Ok(())
        } else {
            GlobalFree(memory);
            Err(last_error("写入剪贴板"))
        };
        CloseClipboard();
        result
    }
}

pub(super) unsafe fn command(hwnd: HWND, state: *mut State, id: u32) {
    // SAFETY: Stable owner-thread state; no borrow survives show_error's modal loop.
    unsafe {
        let result = match id {
            RESTART if (*state).settings.overview_available() => {
                match (*state).controller.send(Command::Restart) {
                    Ok(()) => {
                        (*state).settings.disable();
                        settings::set_text(
                            (*state).status_label,
                            "代理重启中：仅使用已保存配置；非回环监听须重新明确授权。\r\n未保存草稿不参与本次重启。",
                        );
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            COPY_URL => {
                let address = (*state)
                    .overview
                    .snapshot
                    .as_ref()
                    .filter(|snapshot| snapshot.running)
                    .and_then(|snapshot| snapshot.listen);
                match address {
                    Some(address) => copy(hwnd, &base_url(address)),
                    None => Err("当前没有实际运行的监听地址。".into()),
                }
            }
            OPEN_DIRECTORY => (*state).controller.send(Command::OpenDataDirectory),
            _ => Ok(()),
        };
        if let Err(error) = result {
            show_error(hwnd, &error);
        }
    }
}

/// All placement operations below use screen pixels. WINDOWPLACEMENT uses workspace
/// coordinates for ordinary top-level windows, so capture converts its normal rect.
#[derive(Default)]
pub(super) struct Preferences {
    directory: Option<PathBuf>,
    loaded: Option<(PathBuf, Option<WindowGeometry>)>,
    ready: bool,
    restored: bool,
    pub maximize_on_show: bool,
    dirty: Option<Instant>,
    in_flight: Option<WindowGeometry>,
    saved: Option<WindowGeometry>,
    pub exit_flush_deadline: Option<Instant>,
}
impl Preferences {
    pub fn changed(&mut self) {
        if self.ready {
            self.dirty = Some(Instant::now() + Duration::from_millis(400));
        }
    }
    pub fn needs_flush(&self) -> bool {
        self.dirty.is_some() || self.in_flight.is_some()
    }
}

pub(super) fn work_area(hwnd: HWND) -> Option<RECT> {
    // SAFETY: Live HWND; monitor handle is borrowed and info is correctly sized.
    unsafe {
        monitor_info(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)).map(|info| info.rcWork)
    }
}
pub(super) fn rect_work_area(rect: &RECT) -> Option<RECT> {
    // SAFETY: Borrowed rectangle and monitor, correctly sized writable monitor info.
    unsafe { monitor_info(MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST)).map(|info| info.rcWork) }
}
unsafe fn monitor_info(
    monitor: windows_sys::Win32::Graphics::Gdi::HMONITOR,
) -> Option<MONITORINFO> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: Correctly sized writable MONITORINFO and borrowed monitor handle.
    (unsafe { GetMonitorInfoW(monitor, &mut info) } != 0).then_some(info)
}
fn capture(hwnd: HWND, dpi: u32) -> Option<WindowGeometry> {
    let mut placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: Live window and sized writable placement; monitor data is copied.
    unsafe {
        if GetWindowPlacement(hwnd, &mut placement) == 0 {
            return None;
        }
        let r = placement.rcNormalPosition;
        let monitor = monitor_info(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST))?;
        Some(WindowGeometry {
            left: r
                .left
                .saturating_add(monitor.rcWork.left.saturating_sub(monitor.rcMonitor.left)),
            top: r
                .top
                .saturating_add(monitor.rcWork.top.saturating_sub(monitor.rcMonitor.top)),
            width: r.right.saturating_sub(r.left),
            height: r.bottom.saturating_sub(r.top),
            dpi,
            maximized: IsZoomed(hwnd) != 0
                || (IsIconic(hwnd) != 0 && placement.flags & WPF_RESTORETOMAXIMIZED != 0),
        })
    }
}

unsafe fn restore(hwnd: HWND, state: *mut State, geometry: WindowGeometry) {
    let rect = RECT {
        left: geometry.left,
        top: geometry.top,
        right: geometry.left.saturating_add(geometry.width),
        bottom: geometry.top.saturating_add(geometry.height),
    };
    // SAFETY: Copied rectangle and valid monitor info; no State reference crosses resize.
    unsafe {
        if let Some(info) = monitor_info(MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST)) {
            let work = info.rcWork;
            let fitted = geometry.fit([work.left, work.top, work.right, work.bottom], (*state).dpi);
            (*state).preferences.maximize_on_show = fitted.maximized;
            SetWindowPos(
                hwnd,
                null_mut(),
                fitted.left,
                fitted.top,
                fitted.width,
                fitted.height,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            // Moving to a different monitor can synchronously change the HWND's
            // DPI. Refit from the original dimensions, not the already-scaled rect.
            let target_dpi = GetDpiForWindow(hwnd).max(96);
            if target_dpi != fitted.dpi {
                let fitted =
                    geometry.fit([work.left, work.top, work.right, work.bottom], target_dpi);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    fitted.left,
                    fitted.top,
                    fitted.width,
                    fitted.height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            // Never show a background-started/hidden window or steal foreground just to restore.
            if fitted.maximized && IsWindowVisible(hwnd) != 0 && GetForegroundWindow() == hwnd {
                (*state).preferences.maximize_on_show = false;
                ShowWindow(hwnd, SW_MAXIMIZE);
            }
        }
    }
}

pub(super) unsafe fn flush(hwnd: HWND, state: *mut State, force: bool) {
    // SAFETY: UI-owned state; controller.send is a nonblocking bounded try_send.
    unsafe {
        if !(*state).preferences.ready || !(*state).settings.remember_geometry() {
            return;
        }
        if (*state).preferences.in_flight.is_some() {
            if force {
                (*state).preferences.changed();
            }
            return;
        }
        if !force
            && (*state)
                .preferences
                .dirty
                .is_none_or(|due| Instant::now() < due)
        {
            return;
        }
        let Some(directory) = (*state).preferences.directory.clone() else {
            return;
        };
        let Some(mut geometry) = capture(hwnd, (*state).dpi) else {
            return;
        };
        geometry.maximized |= (*state).preferences.maximize_on_show;
        if (*state).preferences.saved == Some(geometry) {
            (*state).preferences.dirty = None;
            return;
        }
        match (*state).controller.send(Command::SaveWindowGeometry {
            directory,
            geometry,
        }) {
            Ok(()) => {
                (*state).preferences.in_flight = Some(geometry);
                (*state).preferences.dirty = None;
            }
            Err(error) => {
                (*state).preferences.dirty = Some(Instant::now() + Duration::from_secs(1));
                settings::set_text((*state).status_label, &format!("窗口位置尚未保存：{error}"));
            }
        }
    }
}

pub(super) unsafe fn poll(hwnd: HWND, state: *mut State) {
    // SAFETY: Stable owner-thread state, short-lived borrows; resize may reenter the WndProc.
    unsafe {
        if let Ok(snapshot) = (*state).controller.overview_events.try_recv() {
            (*state).overview.timeline.record(&snapshot.metrics);
            (*state).overview.snapshot = Some(snapshot);
            (*state).overview.dirty = true;
        }
        if (*state).tabs.active == 0 && IsWindowVisible(hwnd) != 0 {
            (*state).overview.render();
            settings::EnableWindow(
                (*state).overview.restart,
                i32::from((*state).settings.overview_available() && (*state).exiting.is_none()),
            );
            settings::EnableWindow(
                (*state).overview.copy_url,
                i32::from(
                    (*state)
                        .overview
                        .snapshot
                        .as_ref()
                        .is_some_and(|s| s.running && s.listen.is_some())
                        && (*state).exiting.is_none(),
                ),
            );
            settings::EnableWindow(
                (*state).overview.directory,
                i32::from((*state).settings.scope().is_some() && (*state).exiting.is_none()),
            );
        }
        let scope = (*state).settings.scope().cloned();
        if scope != (*state).preferences.directory {
            (*state).preferences.directory = scope.clone();
            (*state).preferences.ready = false;
            (*state).preferences.dirty = None;
            (*state).preferences.in_flight = None;
            (*state).preferences.saved = None;
        }
        if let Ok(response) = (*state).controller.window_events.try_recv() {
            match response {
                WindowResponse::Loaded {
                    directory,
                    geometry,
                } => {
                    // Queues are independent: retain a load that arrives before
                    // the main config event establishes its new actual directory.
                    if !(*state).preferences.ready || scope.as_ref() != Some(&directory) {
                        (*state).preferences.loaded = Some((directory, geometry));
                    }
                }
                WindowResponse::Saved => {
                    (*state).preferences.saved = (*state).preferences.in_flight.take();
                }
                WindowResponse::Error(error) => {
                    // Keep the prior saved baseline; failures are visible and a later
                    // drag/exit can retry, without a one-second disk-error loop.
                    (*state).preferences.in_flight = None;
                    settings::set_text(
                        (*state).status_label,
                        &format!("窗口偏好未保存／加载失败：{error}"),
                    );
                }
            }
        }
        if (*state)
            .preferences
            .loaded
            .as_ref()
            .is_some_and(|(directory, _)| Some(directory) == scope.as_ref())
            && (*state).settings.has_config()
        {
            let loaded = (*state).preferences.loaded.take();
            if let Some((_, geometry)) = loaded {
                if !(*state).preferences.restored
                    && (*state).settings.remember_geometry()
                    && let Some(geometry) = geometry
                {
                    restore(hwnd, state, geometry);
                }
                (*state).preferences.restored = true;
                (*state).preferences.saved = capture(hwnd, (*state).dpi).map(|mut geometry| {
                    geometry.maximized |= (*state).preferences.maximize_on_show;
                    geometry
                });
                (*state).preferences.ready = true;
            }
        }
        if !(*state).settings.remember_geometry() {
            (*state).preferences.dirty = None;
        }
        flush(hwnd, state, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_urls_use_loopback_and_ipv6_brackets() {
        for (input, expected) in [
            ("0.0.0.0:8080", "http://127.0.0.1:8080/v1"),
            ("[::]:8080", "http://[::1]:8080/v1"),
            ("[::1]:9000", "http://[::1]:9000/v1"),
            ("127.0.0.1:42", "http://127.0.0.1:42/v1"),
        ] {
            assert_eq!(base_url(input.parse().unwrap()), expected);
        }
    }
    #[test]
    fn upstream_summary_removes_credentials() {
        assert_eq!(
            safe_upstream("https://user:synthetic-secret@example.invalid/v1?key=secret#secret"),
            "https://example.invalid/v1"
        );
        assert!(!safe_upstream("invalid-secret").contains("invalid-secret"));
    }
    #[test]
    fn counters_keep_their_actual_meaning() {
        let snapshot = OverviewSnapshot {
            metrics: crate::proxy::ProxyMetricsSnapshot {
                total_requests: 7,
                errors: 2,
                active_requests: 3,
                active_tunnels: 1,
                upload_body_bytes: 2048,
                download_body_bytes: 4096,
                uptime: Duration::from_secs(90061),
                saturated: true,
                ..Default::default()
            },
            running: false,
            listen: None,
            upstream: None,
        };
        let text = format_snapshot(&snapshot);
        for expected in [
            "累计请求：7",
            "错误：2",
            "活动请求：3",
            "活动隧道：1",
            "2.0 KiB",
            "4.0 KiB",
            "1 天 01:01:01",
            "计数已达上限",
            "未监听",
        ] {
            assert!(text.contains(expected), "{text}");
        }
    }
}
