//! 有界原生历史分析。同一时刻最多一个在途操作；接收结果前校验目录范围与回显查询。
//! 会话用量是独立子视图。
//!
//! 页面入口只负责协调：筛选解析在 `filters`，表格与文本展示在 `table`，
//! 凭据/存储管理在 `management`。
use super::*;
use crate::{
    app::controller::{Command, HistoryResponse},
    history::{self as db, CredentialId, Query, QueryReport},
};
use std::path::PathBuf;
use windows_sys::Win32::{Graphics::Gdi::*, UI::Controls::*};
const SS_OWNERDRAW: u32 = 0x000d;
const SS_NOPREFIX: u32 = 0x0080;

pub(super) const VIEW: u32 = 2300;
const QUERY: u32 = 2320;
const PREV: u32 = 2321;
const NEXT: u32 = 2322;
const EXPORT: u32 = 2323;
const COUNTERS: u32 = 2324;
const CHART: u32 = 2332;
const RESET: u32 = 2317;
const ADVANCED: u32 = 2318;
const ZONE: u32 = 2333;
const TREND: u32 = 2334;
const DETAIL_SORT: u32 = 2335;
const GROUP_SORT: u32 = 2336;
const TABLE: u32 = 2337;
const TABLE_VIEW: u32 = 2338;
const MANAGE: u32 = 2339;
const ALIAS_PICK: u32 = 2340;
const ALIAS_ID: u32 = 2341;
const ALIAS_LABEL: u32 = 2342;
const ALIAS_SET: u32 = 2343;
const ALIAS_REMOVE: u32 = 2344;
const STORAGE: u32 = 2345;
const RETENTION: u32 = 2346;
const MAX_RECORDS: u32 = 2347;
const CONFIGURE: u32 = 2348;
const REMOVE_ALIASES: u32 = 2349;
const CLEAR: u32 = 2350;
const STORAGE_TEXT: u32 = 2351;
const CLEAR_CONFIRM: u32 = 2352;
const EXPORT_SCOPE: u32 = 2353;
const MANAGEMENT_LABEL: u32 = 2398;
const ADVANCED_LABEL: u32 = 2399;
// Read-only automation state and an owner-thread queue fence. Neither performs I/O.
const OPERATION_STATE: u32 = 2354;
const QUEUE_FENCE: u32 = 2397;
pub(in crate::ui) const STATUS_CHANGED: u32 = WM_APP + 96;

struct ClearPreview {
    scope: PathBuf,
    generation: u64,
    records: u64,
    remove_aliases: bool,
}

struct Pending {
    scope: Option<PathBuf>,
    generation: u64,
    key: String,
}
impl Pending {
    fn matches_scope(
        &self,
        scope: &Option<PathBuf>,
        generation: u64,
        directory: Option<&PathBuf>,
    ) -> bool {
        self.generation == generation
            && &self.scope == scope
            && directory.is_none_or(|directory| scope.as_ref() == Some(directory))
    }
}
#[derive(Default)]
pub(super) struct History {
    controls: Vec<(HWND, u32, i32, i32, i32, i32)>,
    pub session: bool,
    visible: bool,
    scope: Option<PathBuf>,
    generation: u64,
    pending: Option<Pending>,
    applied: Option<Query>,
    report: Option<Box<QueryReport>>,
    credentials: Vec<CredentialId>,
    file: Option<Receiver<files::Result>>,
    export_query: Option<(PathBuf, u64, Query, bool)>,
    modal: bool,
    advanced: bool,
    management: bool,
    preview: Option<ClearPreview>,
    applied_policy: Option<db::Options>,
    operation_sequence: u64,
    queue_fence: u64,
    rejected_commands: u64,
}

mod events;
mod filters;
mod layout;
mod management;
mod status_layout;
#[cfg(test)]
mod status_tests;
mod status_text;
mod table;
#[cfg(test)]
mod tests;

impl History {
    // Visibility changes reveal controls that were deliberately excluded from
    // viewport scrolling. Reset the whole page to document coordinates once,
    // then arrange once; never translate only the newly shown controls twice.
    fn layout_key(&self) -> (bool, bool, bool, bool, usize) {
        (
            self.session,
            self.advanced,
            self.management,
            settings::selected(self.control(2301)) == 4,
            settings::selected(self.control(TABLE_VIEW)),
        )
    }

    fn set_status(&self, text: &str) {
        let control = self.control(2325);
        // SAFETY: Owner-thread native status; defer layout until this History
        // borrow ends. Text changes never rearrange controls synchronously.
        let root = unsafe { GetAncestor(control, GA_ROOT) };
        settings::set_text(control, text);
        if !root.is_null() {
            // SAFETY: A scalar-only queued notification; the root validates its
            // current page/state before rearranging. No borrowed text is retained.
            unsafe { PostMessageW(root, STATUS_CHANGED, 0, 0) };
        }
    }

    pub(super) fn take_applied_policy(&mut self) -> Option<db::Options> {
        self.applied_policy.take()
    }
}

pub(super) unsafe fn command(hwnd: HWND, state: *mut State, id: u32) {
    // SAFETY: Stable UI State; no references cross the modal confirmation.
    unsafe {
        if id == QUEUE_FENCE {
            (*state).history.queue_fence = (*state).history.queue_fence.wrapping_add(1);
            (*state).history.publish_operation_state();
            return;
        }
        if matches!(id, CONFIGURE | CLEAR_CONFIRM) {
            if (*state).history.reject_busy_command(id) {
                return;
            }
            let Some(scope) = (*state).history.scope.clone() else {
                return;
            };
            let generation = (*state).history.generation;
            let (message, action, key) = if id == CONFIGURE {
                let (retention_days, max_records) = match (*state).history.policy() {
                    Ok(value) => value,
                    Err(error) => {
                        settings::set_text((*state).history.control(STORAGE_TEXT), &error);
                        return;
                    }
                };
                (
                    format!(
                        "将历史策略保存为 {retention_days} 天／{max_records} 条，先到先清理。后台可能立即删除超出范围的历史记录，不可撤销。保留别名、HMAC身份、配置密钥和日志。确认保存并应用？"
                    ),
                    Command::HistoryConfigure {
                        retention_days,
                        max_records,
                        confirmed: true,
                    },
                    "policy",
                )
            } else {
                let Some(preview) = (*state).history.preview.as_ref() else {
                    return;
                };
                if preview.generation != generation || preview.scope != scope {
                    return;
                }
                let remove_aliases =
                    SendMessageW((*state).history.control(REMOVE_ALIASES), BM_GETCHECK, 0, 0)
                        == BST_CHECKED as isize;
                if remove_aliases != preview.remove_aliases {
                    (*state).history.preview = None;
                    (*state).history.enable();
                    return;
                }
                (
                    format!(
                        "后台预览：当前目录全部历史约 {} 条（不受筛选限制）；同时删除全部别名：{}。新写入也可能被删除。保留HMAC身份、配置、密钥和日志。此操作不可撤销，确认清空？",
                        preview.records, remove_aliases
                    ),
                    Command::HistoryClear {
                        confirmed: true,
                        remove_aliases,
                    },
                    "clear",
                )
            };
            (*state).history.modal = true;
            (*state).history.enable();
            let accepted = settings::confirm(hwnd, &message);
            (*state).history.modal = false;
            if accepted
                && (*state).exiting.is_none()
                && (*state).history.scope.as_ref() == Some(&scope)
                && (*state).history.generation == generation
            {
                (*state).history.preview = None;
                (*state)
                    .history
                    .send(&(*state).controller, action, key.into());
            }
            (*state).history.enable();
        } else if id == EXPORT {
            if (*state).history.reject_busy_command(id) {
                return;
            }
            let Some(query) = (*state).history.applied.clone() else {
                return;
            };
            let Some(scope) = (*state).history.scope.clone() else {
                return;
            };
            let generation = (*state).history.generation;
            let all = settings::selected((*state).history.control(EXPORT_SCOPE)) == 1;
            let range = if all {
                "已应用筛选后的全部匹配请求（忽略分页，不是仅当前页）".to_string()
            } else {
                format!(
                    "已应用查询的当前明细页：offset {}，最多 {} 行；不是全量导出",
                    query.offset, query.limit
                )
            };
            (*state).history.modal = true;
            (*state).history.enable();
            let accepted = settings::confirm(
                hwnd,
                &format!(
                    "导出{range}。CSV排除来源IP及客户端／上游凭据HMAC ID，不含密钥或请求正文；全量CSV包含用户别名、请求ID、时间、模型、Provider和用量等可关联元数据。别名不能当作匿名数据，请仅存放到可信位置。\n后台重新读取当前已应用查询，新记录可能影响结果。只创建新文件，不覆盖已有文件。确认后选择路径？"
                ),
            );
            (*state).history.modal = false;
            if !accepted
                || (*state).exiting.is_some()
                || (*state).history.scope.as_ref() != Some(&scope)
                || (*state).history.generation != generation
            {
                (*state).history.enable();
                return;
            }
            match files::spawn(files::Task::HistoryPath) {
                Ok(rx) => {
                    (*state).history.file = Some(rx);
                    (*state).history.export_query = Some((scope, generation, query, all));
                }
                Err(e) => (*state).history.set_status(&e),
            }
            (*state).history.enable();
        } else {
            // Choice selection has already changed before WM_COMMAND. These
            // commands can reveal stale hidden HWNDs even when the new selection
            // equals the last one (automation/keyboard compatibility messages).
            let before = (*state).history.layout_key();
            (*state).history.handle(&(*state).controller, id);
            let changed = before != (*state).history.layout_key()
                || matches!(id, VIEW | TABLE_VIEW | 2301 | RESET);
            let visible = (*state).tabs.active == 1 && (*state).history.session;
            (*state).settings.show_session(visible);
            if changed {
                (*state).page_geometry[1] = false;
                (*state).page_positioned[1] = false;
                if (*state).tabs.active == 1 {
                    (*state).layout(hwnd, false);
                }
            }
        }
    }
}
pub(super) unsafe fn draw(state: *mut State, l: LPARAM) -> bool {
    // SAFETY: WM_DRAWITEM supplies a live DRAWITEMSTRUCT/DC; all GDI objects restored/freed.
    unsafe {
        let item = &*(l as *const DRAWITEMSTRUCT);
        if item.CtlID != CHART {
            return false;
        }
        super::history_chart::draw(
            item,
            (*state).dpi,
            (*state).font.0,
            (*state).heading_font.0,
            &(*state).theme,
            (*state).history.report.as_deref(),
            (*state).history.pending.is_some(),
        );
        true
    }
}
