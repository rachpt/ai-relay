//! 历史页面交互与后台结果轮询。
use super::*;

impl History {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some() || self.file.is_some() || self.modal
    }
    pub(super) fn publish_operation_state(&self) {
        settings::set_text(
            self.control(OPERATION_STATE),
            &format!(
                "idle={};sequence={};fence={};rejected={};generation={}",
                u8::from(!self.busy() && self.scope.is_some()),
                self.operation_sequence,
                self.queue_fence,
                self.rejected_commands,
                self.generation,
            ),
        );
    }
    pub(super) fn reject_busy_command(&mut self, id: u32) -> bool {
        if !self.busy() {
            return false;
        }
        self.rejected_commands = self.rejected_commands.wrapping_add(1);
        self.set_status(&format!(
            "历史操作仍在进行；命令 {id} 未执行，请等待完成后重试（未排队或重复发送）。"
        ));
        self.publish_operation_state();
        true
    }
    pub(super) fn send(&mut self, controller: &Controller, command: Command, key: String) {
        self.preview = None;
        match controller.send(command) {
            Ok(()) => {
                self.operation_sequence = self.operation_sequence.wrapping_add(1);
                self.pending = Some(Pending {
                    scope: self.scope.clone(),
                    generation: self.generation,
                    key,
                });
                self.set_status("历史后台操作中；筛选已冻结，单请求在途……");
            }
            Err(e) => self.set_status(&e),
        }
        self.invalidate();
        self.enable();
    }
    pub(super) fn handle(&mut self, controller: &Controller, id: u32) {
        if id == VIEW {
            self.session = settings::selected(self.control(VIEW)) == 1;
            self.show(self.visible);
            return;
        }
        if self.reject_busy_command(id) {
            return;
        }
        match id {
            2301 => self.show(self.visible),
            ZONE | TREND if self.report.is_some() => {
                // Direct choices requery only after a first successful report;
                // no request is sent merely by opening the page.
                self.handle(controller, QUERY);
            }
            MANAGE => {
                self.management = !self.management;
                settings::set_text(
                    self.control(MANAGE),
                    if self.management {
                        "返回统计表格"
                    } else {
                        "凭据／存储…"
                    },
                );
                self.show(self.visible);
                if self.management {
                    self.send(controller, Command::HistoryStorage, "storage".into());
                }
            }
            TABLE_VIEW => {
                self.render_table();
                self.show(self.visible);
            }
            ALIAS_PICK => {
                let index = settings::selected(self.control(ALIAS_PICK));
                if let Some(id) = self.credentials.get(index) {
                    settings::set_text(self.control(ALIAS_ID), id.as_str());
                    settings::set_text(
                        self.control(ALIAS_LABEL),
                        self.report.as_ref().map_or("", |r| {
                            r.aliases
                                .iter()
                                .find(|a| &a.credential == id)
                                .map_or("", |a| a.alias.as_str())
                        }),
                    );
                }
            }
            ALIAS_SET | ALIAS_REMOVE => {
                let operation = self.alias_command(id == ALIAS_REMOVE);
                match operation {
                    Ok(command) => self.send(controller, command, "alias".into()),
                    Err(error) => settings::set_text(self.control(STORAGE_TEXT), &error),
                }
            }
            STORAGE => self.send(controller, Command::HistoryStorage, "storage".into()),
            REMOVE_ALIASES => {
                self.preview = None;
                self.enable();
            }
            CLEAR => {
                self.preview = None;
                // SAFETY: scalar query to a live native checkbox.
                let remove_aliases = unsafe {
                    SendMessageW(self.control(REMOVE_ALIASES), BM_GETCHECK, 0, 0)
                        == BST_CHECKED as isize
                };
                self.send(
                    controller,
                    Command::HistoryClear {
                        confirmed: false,
                        remove_aliases,
                    },
                    "clear-preview".into(),
                );
            }
            ADVANCED => {
                self.advanced = !self.advanced;
                settings::set_text(
                    self.control(ADVANCED),
                    if self.advanced {
                        "收起高级筛选 ▴"
                    } else {
                        "高级筛选 ▾（条件保留）"
                    },
                );
                self.show(self.visible);
            }
            RESET => {
                // Reset only draft controls. The applied report/page remains labelled
                // until an explicit query; export always uses the applied query.
                for id in [
                    2301,
                    2310,
                    2311,
                    2312,
                    2314,
                    2315,
                    2316,
                    ZONE,
                    TREND,
                    DETAIL_SORT,
                    GROUP_SORT,
                ] {
                    // SAFETY: Live native combo, no pointer argument or notification.
                    unsafe {
                        SendMessageW(self.control(id), CB_SETCURSEL, usize::from(id == ZONE), 0);
                    }
                }
                for id in 2302..=2309 {
                    settings::set_text(self.control(id), "");
                }
                settings::set_text(self.control(2313), "");
                self.set_status(
                    "筛选草稿已重置；点击查询应用。图表／分页／导出仍使用上次已应用查询。",
                );
                self.show(self.visible);
            }
            QUERY | PREV | NEXT | DETAIL_SORT | GROUP_SORT => {
                let q = if matches!(id, QUERY | DETAIL_SORT | GROUP_SORT) {
                    self.draft()
                } else {
                    self.applied
                        .clone()
                        .ok_or("请先查询历史".into())
                        .map(|mut q| {
                            q.offset = if id == PREV {
                                q.offset.saturating_sub(q.limit)
                            } else {
                                q.offset.saturating_add(q.limit)
                            };
                            q
                        })
                };
                match q {
                    Ok(q) => {
                        let key = format!("{q:?}");
                        self.send(controller, Command::HistoryQuery(q), key);
                    }
                    Err(e) => self.set_status(&e),
                }
            }
            COUNTERS => self.send(controller, Command::HistoryCounters, "counters".into()),
            _ => {}
        }
    }
    pub fn poll(&mut self, controller: &Controller) {
        if self.modal {
            return;
        }
        if let Some(result) = self.file.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(files::Result::Error("导出文件选择线程已结束。".into()))
            }
            Err(_) => None,
        }) {
            self.file = None;
            match result {
                files::Result::HistoryPath(path) => {
                    if let Some((scope, generation, query, all)) = self.export_query.take() {
                        if self.scope.as_ref() != Some(&scope) || self.generation != generation {
                            self.set_status("数据目录已变化；已取消旧目录导出，未写文件。");
                            self.enable();
                            return;
                        }
                        self.send(
                            controller,
                            if all {
                                Command::HistoryExportAll {
                                    query,
                                    path,
                                    confirmed: true,
                                }
                            } else {
                                Command::HistoryExport {
                                    query,
                                    path,
                                    confirmed: true,
                                }
                            },
                            "export".into(),
                        );
                    }
                }
                files::Result::Error(e) => {
                    self.export_query = None;
                    self.set_status(&e);
                }
                _ => {
                    self.export_query = None;
                    self.set_status("已取消历史导出，未写文件。");
                }
            }
        }
        while let Ok(event) = controller.history_events.try_recv() {
            let Some(pending) = self.pending.take() else {
                continue;
            };
            self.invalidate();
            let directory = match &event {
                HistoryResponse::QueryResult { directory, .. }
                | HistoryResponse::Exported { directory, .. }
                | HistoryResponse::Counters { directory, .. }
                | HistoryResponse::Storage { directory, .. }
                | HistoryResponse::AliasChanged { directory, .. }
                | HistoryResponse::PolicyChanged { directory, .. }
                | HistoryResponse::ClearPreview { directory, .. }
                | HistoryResponse::Cleared { directory, .. } => Some(directory),
                HistoryResponse::Error { directory, .. } => directory.as_ref(),
            };
            if !pending.matches_scope(&self.scope, self.generation, directory) {
                self.set_status("已拒绝旧目录历史结果；请重新查询。");
                continue;
            }
            let key = pending.key;
            match event {
                HistoryResponse::QueryResult {
                    query,
                    report,
                    counters,
                    ..
                } if format!("{query:?}") == key => self.render(query, report, counters),
                HistoryResponse::Exported {
                    path,
                    rows,
                    page_only,
                    counters,
                    ..
                } if key == "export" => {
                    self.set_status(&status_text::exported(&path, rows, page_only, counters))
                }
                HistoryResponse::Counters { counters, .. } if key == "counters" => {
                    self.set_status(&status_text::counters(counters))
                }
                HistoryResponse::Storage { stats, .. } if key == "storage" => {
                    self.storage(&stats);
                    self.set_status("历史存储状态已刷新（后台真实值）。");
                }
                HistoryResponse::AliasChanged { .. } if key == "alias" => {
                    settings::set_text(
                        self.control(STORAGE_TEXT),
                        "别名已更新；ID与真实聚合数据不变。重新查询以刷新表格标签。",
                    );
                    if let Some(query) = self.applied.clone() {
                        let key = format!("{query:?}");
                        self.clear_report("别名已更新；正在重新查询真实历史。");
                        self.send(controller, Command::HistoryQuery(query), key);
                    } else {
                        self.set_status("别名已更新，请重新查询。");
                    }
                }
                HistoryResponse::PolicyChanged { report, .. } if key == "policy" => {
                    self.applied_policy = Some(report.options);
                    self.preview = None;
                    self.storage(&report.storage);
                    self.clear_report("策略已更新；旧报表已失效，请重新查询。");
                    self.set_status(&format!(
                        "历史策略已保存并应用；删除 {} 条记录；checkpoint {:?}。",
                        report.removed_records, report.checkpoint
                    ));
                }
                HistoryResponse::ClearPreview {
                    directory,
                    records,
                    remove_aliases,
                } if key == "clear-preview" => {
                    self.preview = Some(ClearPreview {
                        scope: directory,
                        generation: self.generation,
                        records,
                        remove_aliases,
                    });
                    settings::set_text(
                        self.control(STORAGE_TEXT),
                        &format!(
                            "清空预览：整个当前目录历史，约 {records} 条（不限当前筛选）；同时移除别名：{remove_aliases}。\r\n保留 HMAC 身份、配置、密钥、日志；不可撤销。后台写入可能增加实际删除数。\r\n请点击“确认清空…”进行最终确认；未确认不删除。"
                        ),
                    );
                    self.set_status("清空预览已收到；尚未删除任何记录。");
                }
                HistoryResponse::Cleared { remove_aliases, .. } if key == "clear" => {
                    self.preview = None;
                    if remove_aliases {
                        self.credentials.clear();
                        settings::set_text(self.control(ALIAS_ID), "");
                        settings::set_text(self.control(ALIAS_LABEL), "");
                        // SAFETY: live combo, scalar reset message.
                        unsafe {
                            SendMessageW(self.control(ALIAS_PICK), CB_RESETCONTENT, 0, 0);
                        }
                    }
                    self.clear_report(
                        "历史已清空；请重新查询。后续请求仍会写入，不保证目录永远为空。",
                    );
                    settings::set_text(
                        self.control(STORAGE_TEXT),
                        "清空已完成；HMAC 身份／配置／密钥／日志未删除。请刷新存储状态。",
                    );
                    self.set_status(&format!(
                        "历史已清空；移除别名 {remove_aliases}；后续请求仍会记录。"
                    ));
                }
                HistoryResponse::Error { message, .. } => {
                    self.preview = None;
                    self.set_status(&format!("历史不可用／操作失败：{message}"));
                    if self.management {
                        settings::set_text(self.control(STORAGE_TEXT), &message);
                    }
                }
                _ => self.set_status("已拒绝不匹配的过期历史结果；请重新查询。"),
            }
        }
        self.enable();
    }
}
