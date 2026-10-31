//! 历史页面控件创建、可见性、布局与字体。
use super::*;

impl History {
    pub(super) fn control(&self, id: u32) -> HWND {
        self.controls
            .iter()
            .find(|c| c.1 == id)
            .map_or(null_mut(), |c| c.0)
    }
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        parent: HWND,
        id: u32,
        class: &str,
        title: &str,
        style: u32,
        row: (i32, i32, i32, i32),
    ) -> Result<HWND, String> {
        // SAFETY: UI-owned parent/module and copied strings; ID is not a pointer.
        let h = unsafe {
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
                GetModuleHandleW(null()),
                null(),
            )
        };
        if h.is_null() {
            return Err(last_error("创建历史控件"));
        }
        self.controls.push((h, id, row.0, row.1, row.2, row.3));
        if class == "EDIT" {
            // SAFETY: Live edit, bounded user text.
            unsafe {
                let limit = match id {
                    2330 | 2331 | STORAGE_TEXT => 256 * 1024,
                    ALIAS_ID => 64,
                    ALIAS_LABEL => 128,
                    RETENTION | MAX_RECORDS => 10,
                    _ => 512,
                };
                SendMessageW(h, 0xc5, limit, 0);
            }
        }
        Ok(h)
    }
    pub fn combo(
        &mut self,
        parent: HWND,
        id: u32,
        items: &[&str],
        row: (i32, i32, i32, i32),
    ) -> Result<(), String> {
        if matches!(id, VIEW | ZONE | TREND | TABLE_VIEW | EXPORT_SCOPE | 2301) {
            let h = choices::create(parent, id, items)?;
            self.controls.push((h, id, row.0, row.1, row.2, 32));
            return Ok(());
        }
        let h = self.add(
            parent,
            id,
            "COMBOBOX",
            "",
            WS_TABSTOP | CBS_DROPDOWNLIST as u32 | WS_VSCROLL,
            row,
        )?;
        // SAFETY: Combo copies strings synchronously; first item exists.
        unsafe {
            for item in items {
                SendMessageW(h, CB_ADDSTRING, 0, wide(item).as_ptr() as isize);
            }
            SendMessageW(h, CB_SETCURSEL, 0, 0);
        }
        Ok(())
    }
    pub fn create(&mut self, parent: HWND) -> Result<(), String> {
        self.add(parent, OPERATION_STATE, "STATIC", "", 0, (0, 0, 1, 1))?;
        self.combo(
            parent,
            VIEW,
            &["▥ 历史统计", "◷ 当前会话"],
            (24, 60, 270, 180),
        )?;
        self.add(
            parent,
            RESET,
            "BUTTON",
            "重置筛选",
            WS_TABSTOP,
            (306, 60, 100, 30),
        )?;
        self.add(
            parent,
            ADVANCED,
            "BUTTON",
            "高级筛选 ▾",
            WS_TABSTOP,
            (418, 60, 140, 30),
        )?;
        self.combo(
            parent,
            2301,
            &["全部", "今日", "7 天", "30 天", "自定义"],
            (24, 102, 348, 32),
        )?;
        for (id, label, x, y, w) in [
            (2304, "来源 IP", 24, 164, 220),
            (2305, "响应模型（精确）", 256, 164, 220),
            (2306, "请求模型（精确）", 488, 164, 232),
            (2307, "客户端凭据 ID", 24, 226, 220),
            (2308, "实际上游凭据 ID", 256, 226, 220),
            (2309, "Provider origin", 488, 226, 232),
        ] {
            self.add(parent, 0, "STATIC", label, 0, (x, y, w, 22))?;
            self.add(
                parent,
                id,
                "EDIT",
                "",
                WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
                (x, y + 24, w, 28),
            )?;
        }
        for (id, label, x) in [(2302, "开始（含）", 24), (2303, "结束（不含）", 372)] {
            let picker = date_picker::create(parent, id, label)?;
            self.controls.push((picker, id, x, 136, 336, 28));
        }
        self.combo(
            parent,
            2310,
            &["全部协议", "未知", "OpenAI 格式", "Anthropic 格式", "HTTP"],
            (24, 292, 168, 180),
        )?;
        self.combo(
            parent,
            2311,
            &[
                "全部接口",
                "未知",
                "ChatCompletions",
                "Responses",
                "Messages",
                "Forward",
                "CONNECT",
            ],
            (204, 292, 200, 220),
        )?;
        self.combo(
            parent,
            2312,
            &[
                "全部结果",
                "Success",
                "HttpError",
                "Cancelled",
                "Timeout",
                "TransportError",
                "Rejected",
            ],
            (416, 292, 168, 220),
        )?;
        self.add(parent, 0, "STATIC", "HTTP", 0, (596, 286, 36, 26))?;
        self.add(
            parent,
            2313,
            "EDIT",
            "",
            WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            (636, 292, 84, 28),
        )?;
        self.add(
            parent,
            ADVANCED_LABEL,
            "STATIC",
            "三级分组使用所选时区；凭据仅填本地完整 HMAC ID，切勿填写密钥",
            0,
            (24, 322, 696, 24),
        )?;
        for (id, x) in [(2314, 24), (2315, 256), (2316, 488)] {
            self.combo(
                parent,
                id,
                &[
                    "不分组",
                    "日（所选时区）",
                    "小时（含偏移）",
                    "响应模型",
                    "IP",
                    "客户端凭据",
                    "上游凭据",
                    "接口",
                    "Provider",
                    "月（所选时区）",
                    "请求模型",
                    "协议格式",
                ],
                (x, 348, 220, 240),
            )?;
        }
        for (id, title, x, w) in [
            (QUERY, "⌕ 查询 / 应用", 24, 168),
            (PREV, "‹ 上一页", 204, 100),
            (NEXT, "下一页 ›", 316, 100),
            (EXPORT, "导出当前明细页…", 428, 168),
            (COUNTERS, "ⓘ 记录状态", 608, 112),
        ] {
            self.add(parent, id, "BUTTON", title, WS_TABSTOP, (x, 398, w, 30))?;
        }
        self.add(
            parent,
            2325,
            "STATIC",
            "历史尚未查询；不可用时将显示后台真实错误。",
            SS_NOPREFIX,
            (24, 436, 696, 44),
        )?;
        self.add(
            parent,
            CHART,
            "STATIC",
            "所选时区小时／日／月已报告总 Token 趋势",
            SS_OWNERDRAW,
            (24, 484, 696, 110),
        )?;
        let style = WS_TABSTOP
            | WS_BORDER
            | WS_VSCROLL
            | WS_HSCROLL
            | (ES_MULTILINE | ES_READONLY | ES_AUTOHSCROLL | ES_AUTOVSCROLL) as u32;
        self.add(
            parent,
            2330,
            "EDIT",
            "汇总／分组／趋势：等待真实历史查询",
            style,
            (24, 604, 696, 100),
        )?;
        self.add(
            parent,
            2331,
            "EDIT",
            "分页请求明细：等待真实历史查询",
            style,
            (24, 714, 696, 100),
        )?;
        self.add(
            parent,
            MANAGE,
            "BUTTON",
            "凭据／存储…",
            WS_TABSTOP,
            (570, 60, 150, 30),
        )?;
        self.combo(parent, ZONE, &["UTC", "本地"], (384, 102, 132, 32))?;
        // SAFETY: Live segmented choice; scalar selection, no notification/query.
        unsafe { SendMessageW(self.control(ZONE), CB_SETCURSEL, 1, 0) };
        self.combo(parent, TREND, &["日", "小时", "月"], (528, 102, 192, 32))?;
        self.combo(
            parent,
            DETAIL_SORT,
            &["明细：时间↓", "明细：时间↑", "明细：Token↓", "明细：Token↑"],
            (24, 548, 196, 180),
        )?;
        self.combo(
            parent,
            GROUP_SORT,
            &[
                "汇总：分组↑",
                "汇总：分组↓",
                "汇总：请求↓",
                "汇总：请求↑",
                "汇总：时间↓",
                "汇总：时间↑",
                "汇总：Token↓",
                "汇总：Token↑",
            ],
            (232, 548, 196, 240),
        )?;
        self.combo(
            parent,
            TABLE_VIEW,
            &["▤ 请求明细", "▥ 分组汇总", "▧ 汇总文本", "≡ 元数据"],
            (24, 548, 696, 32),
        )?;
        self.combo(
            parent,
            EXPORT_SCOPE,
            &["本页 CSV", "全量 CSV"],
            (440, 548, 280, 32),
        )?;
        // SAFETY: initialize the system-owned list-view class before creation.
        unsafe {
            let init = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            if InitCommonControlsEx(&init) == 0 {
                return Err(last_error("初始化历史表格"));
            }
        }
        let table = self.add(
            parent,
            TABLE,
            "SysListView32",
            "历史请求明细／分页汇总：点击单元格后 Ctrl+C，或右键复制；左右键切换列",
            WS_TABSTOP
                | WS_BORDER
                | LVS_REPORT
                | LVS_SHOWSELALWAYS
                | LVS_SINGLESEL
                | LVS_NOSORTHEADER,
            (24, 584, 696, 100),
        )?;
        cell_table::install(table)?;
        // SAFETY: live native list view; scalar style flags and no borrowed pointers.
        unsafe {
            SendMessageW(
                table,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                0,
                (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP) as isize,
            );
        }
        for (title, y) in [
            (
                "凭据：从已查询元数据选择或粘贴完整 HMAC ID；别名与 ID 分开，勿输入密钥或片段。",
                110,
            ),
            ("本地 HMAC ID（不可逆标识，不是 API Key）", 180),
            ("用户显示别名（1–64 字；不是凭据筛选值，不得使用密钥）", 246),
            (
                "存储策略：左侧保留天数 1–90；中间记录上限 1–100000。缩小策略可能立即删除旧记录。",
                362,
            ),
        ] {
            self.add(
                parent,
                MANAGEMENT_LABEL,
                "STATIC",
                title,
                0,
                (24, y, 696, 26),
            )?;
        }
        self.combo(
            parent,
            ALIAS_PICK,
            &["选择已查询凭据 ID"],
            (24, 142, 696, 260),
        )?;
        for (id, y, w) in [
            (ALIAS_ID, 210, 696),
            (ALIAS_LABEL, 276, 696),
            (RETENTION, 394, 160),
            (MAX_RECORDS, 394, 180),
        ] {
            let x = if id == MAX_RECORDS { 196 } else { 24 };
            self.add(
                parent,
                id,
                "EDIT",
                "",
                WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
                (x, y, w, 28),
            )?;
        }
        for (id, title, x, y, w) in [
            (ALIAS_SET, "保存别名", 24, 316, 160),
            (ALIAS_REMOVE, "移除别名", 196, 316, 180),
            (STORAGE, "刷新占用／策略", 388, 316, 220),
            (CONFIGURE, "确认并应用策略…", 388, 394, 220),
            (CLEAR, "预览清空历史…", 24, 488, 220),
            (CLEAR_CONFIRM, "确认清空…", 256, 488, 220),
        ] {
            self.add(parent, id, "BUTTON", title, WS_TABSTOP, (x, y, w, 30))?;
        }
        self.add(
            parent,
            REMOVE_ALIASES,
            "BUTTON",
            "清空时同时删除全部别名（默认保留）",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            (24, 446, 696, 30),
        )?;
        self.add(
            parent,
            STORAGE_TEXT,
            "EDIT",
            "点击刷新占用／策略获取后台真实状态。保留 HMAC 身份文件，不删除配置、密钥或日志。",
            style,
            (24, 530, 696, 120),
        )?;
        settings::set_text(self.control(RETENTION), "");
        settings::set_text(self.control(MAX_RECORDS), "");
        for id in 2304..=2309 {
            let hint = "留空表示全部";
            // SAFETY: Live single-line EDIT copies the cue string synchronously.
            unsafe {
                SendMessageW(self.control(id), 0x1501, 0, wide(hint).as_ptr() as isize);
            }
        }
        // SAFETY: Live single-line EDIT; no credential values are used in hints.
        unsafe {
            SendMessageW(
                self.control(2313),
                0x1501,
                0,
                wide("HTTP 状态码").as_ptr() as isize,
            );
        }
        Ok(())
    }
    pub fn show(&mut self, visible: bool) {
        self.visible = visible;
        let mut shows = ShowBatch::with_capacity(self.controls.len());
        for &(h, id, _, y, ..) in &self.controls {
            let advanced = (2307..=2316).contains(&id)
                || id == ADVANCED_LABEL
                || (id == 0 && (226..=380).contains(&y));
            let management = (ALIAS_PICK..=STORAGE_TEXT).contains(&id)
                || id == CLEAR_CONFIRM
                || id == MANAGEMENT_LABEL;
            let view = settings::selected(self.control(TABLE_VIEW));
            let content = match id {
                OPERATION_STATE => false,
                2302 | 2303 => settings::selected(self.control(2301)) == 4,
                2330 => view == 2,
                2331 => view == 3,
                TABLE => view <= 1,
                CHART => !self.advanced,
                _ => true,
            };
            let date_label = id == 0 && y == 102;
            let show = visible
                && (id == VIEW
                    || (!self.session
                        && !date_label
                        && content
                        && if management {
                            self.management
                        } else if id == MANAGE || id == 2325 {
                            true
                        } else {
                            !self.management && (!advanced || self.advanced)
                        }));
            shows.add(h, show);
        }
        shows.commit();
        self.enable();
    }
    pub(super) fn enable(&self) {
        self.publish_operation_state();
        for &(h, id, ..) in &self.controls {
            // SAFETY: Live child controls; no modal dispatch.
            unsafe {
                settings::EnableWindow(
                    h,
                    i32::from(
                        id == VIEW
                            || ((self.scope.is_some()
                                && self.pending.is_none()
                                && self.file.is_none()
                                && !self.modal)
                                && match id {
                                    PREV => self.applied.as_ref().is_some_and(|q| q.offset > 0),
                                    NEXT => self.report.as_ref().is_some_and(|r| {
                                        self.applied.as_ref().is_some_and(|q| {
                                            q.offset.saturating_add(q.limit) <= 100_000
                                                && (u64::from(q.offset) + (r.details.len() as u64)
                                                    < r.summary.requests
                                                    || r.groups.len() == q.limit as usize)
                                        })
                                    }),
                                    EXPORT => self.report.is_some(),
                                    CLEAR_CONFIRM => self.preview.as_ref().is_some_and(|p| {
                                        p.generation == self.generation
                                            && self.scope.as_ref() == Some(&p.scope)
                                    }),
                                    _ => true,
                                }),
                    ),
                );
            }
        }
    }
    pub fn set_font(&self, font: HFONT) {
        for &(h, ..) in &self.controls {
            // SAFETY: State retains font through child lifetime.
            unsafe {
                SendMessageW(h, WM_SETFONT, font as usize, 0);
            }
        }
    }
    pub(in crate::ui) fn page_height(&self, dpi: u32, height: i32, width: i32) -> i32 {
        let extra = if self.session {
            0
        } else {
            status_layout::height(self.control(2325), dpi, (width - scale(48, dpi)).max(1))
                - scale(status_layout::MIN_HEIGHT, dpi)
        };
        height.max(scale(820, dpi) + extra)
    }

    pub fn layout(&self, dpi: u32, width: i32, height: i32) {
        let status_height =
            status_layout::height(self.control(2325), dpi, (width - scale(48, dpi)).max(1));
        let geometry = status_layout::Geometry::new(dpi, self.advanced, status_height);
        let chart_top = geometry.chart_top;
        let bottom = viewport::content_bottom(dpi, height);
        let chart_height = scale(200, dpi);
        // Expanding filters temporarily replaces the chart, not the usable table.
        let sort_top = geometry.sort_top;
        let table_top = geometry.table_top;
        let editor_height = (bottom - table_top).max(scale(28, dpi));
        let mut moves = MoveBatch::with_capacity(self.controls.len());
        for &(h, id, x, y, w, ht) in &self.controls {
            let (top, height) = match id {
                CHART => (chart_top, chart_height),
                2302 | 2303 => (scale(136, dpi), scale(ht, dpi)),
                2330 | 2331 | TABLE => (table_top, editor_height),
                TABLE_VIEW => (sort_top + scale(36, dpi), scale(ht, dpi)),
                DETAIL_SORT | GROUP_SORT | EXPORT_SCOPE => (sort_top, scale(ht, dpi)),
                CLEAR_CONFIRM => (scale(y, dpi), scale(ht, dpi)),
                STORAGE_TEXT => (
                    scale(y, dpi),
                    (bottom - scale(y + 8, dpi) - status_height).max(scale(70, dpi)),
                ),
                _ if (ALIAS_PICK..=STORAGE_TEXT).contains(&id) || id == MANAGEMENT_LABEL => {
                    (scale(y, dpi), scale(ht, dpi))
                }
                QUERY | PREV | NEXT | EXPORT | COUNTERS => (
                    scale(if self.advanced { 386 } else { 232 }, dpi),
                    scale(ht, dpi),
                ),
                2325 => (
                    if self.management {
                        bottom - status_height
                    } else {
                        geometry.status_top
                    },
                    geometry.status_height,
                ),
                _ => (scale(if y == 292 { 284 } else { y }, dpi), scale(ht, dpi)),
            };
            // SAFETY: Owned live children only.
            moves.add(
                h,
                scale(24, dpi) + ((width - scale(48, dpi)) * (x - 24) / 696),
                top,
                ((width - scale(48, dpi)) * w / 696).max(1),
                height,
            );
        }
        moves.commit();
    }
    pub fn scope(&mut self, directory: PathBuf) {
        if self.scope.as_ref() != Some(&directory) {
            self.scope = Some(directory);
            // Keep the old operation in flight until it drains, but never accept it
            // after a directory transition, even if the path later changes back.
            self.generation = self.generation.wrapping_add(1);
            self.applied = None;
            self.report = None;
            self.credentials.clear();
            self.preview = None;
            settings::set_text(
                self.control(STORAGE_TEXT),
                "数据目录已变化；请刷新占用和策略。旧预览不可确认。",
            );
            for id in [ALIAS_ID, ALIAS_LABEL, RETENTION, MAX_RECORDS] {
                settings::set_text(self.control(id), "");
            }
            // SAFETY: live list view/combo; messages own no borrowed data.
            unsafe {
                SendMessageW(self.control(TABLE), LVM_DELETEALLITEMS, 0, 0);
                SendMessageW(self.control(ALIAS_PICK), CB_RESETCONTENT, 0, 0);
                SendMessageW(
                    self.control(REMOVE_ALIASES),
                    BM_SETCHECK,
                    BST_UNCHECKED as usize,
                    0,
                );
            }
            for id in [2307, 2308] {
                settings::set_text(self.control(id), "");
            }
            self.set_status("数据目录已变化；旧查询／导出结果不会用于当前目录。");
            settings::set_text(self.control(2330), "数据目录已变化；请重新查询历史。");
            settings::set_text(self.control(2331), "");
            self.invalidate();
            self.enable();
        }
    }
    pub(super) fn invalidate(&self) {
        // SAFETY: Invalidate only queues painting for a live child.
        unsafe {
            InvalidateRect(self.control(CHART), null(), 1);
        }
    }
}
