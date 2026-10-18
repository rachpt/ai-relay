//! 独立历史控制线程：拥有阻塞服务，查询扫描和导出均不占 controller 或网络 runtime。
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::Duration,
};

use crate::{
    config,
    history::{self, HistoryHandle, HistoryService},
};

use super::{
    results::ResultQueue,
    types::{Command, HistoryResponse},
};

pub(super) struct HistoryOwner {
    pub(super) directory: PathBuf,
    pub(super) handle: HistoryHandle,
    pub(super) commands: SyncSender<Command>,
    worker: Option<thread::JoinHandle<()>>,
    stopped: Receiver<Result<(), String>>,
    pub(super) stop_requested: bool,
    pub(super) options: Option<history::Options>,
}
impl HistoryOwner {
    pub(super) fn start(
        directory: PathBuf,
        events: ResultQueue<HistoryResponse>,
        options: history::Options,
    ) -> Result<Self, String> {
        let (commands, receiver) = mpsc::sync_channel(4);
        let (ready, started) = mpsc::sync_channel(1);
        let (finished, stopped) = mpsc::sync_channel(1);
        let scope = directory.clone();
        let worker = thread::Builder::new()
            .name("ai-relay-history-control".into())
            .spawn(move || {
                let mut service = match HistoryService::start_with_options(&scope, options) {
                    Ok(service) => service,
                    Err(error) => {
                        let _ = ready.send(Err(history_error(error)));
                        return;
                    }
                };
                if ready.send(Ok(service.handle())).is_err() {
                    return;
                }
                let mut clear_preview = None;
                while let Ok(command) = receiver.recv() {
                    let result = match command {
                        Command::HistoryQuery(query) => service
                            .query(query.clone())
                            .map(|report| HistoryResponse::QueryResult {
                                directory: scope.clone(),
                                query,
                                report: Box::new(report),
                                counters: service.handle().counters(),
                            })
                            .map_err(history_error),
                        Command::HistoryExport {
                            query,
                            path,
                            confirmed,
                        } => export_history(&service, &scope, query, path, confirmed),
                        Command::HistoryExportAll { query, path, confirmed } => {
                            export_all_history(&service, &scope, query, path, confirmed)
                        }
                        Command::HistoryStorage => service
                            .stats()
                            .map(|stats| HistoryResponse::Storage {
                                directory: scope.clone(),
                                stats,
                            })
                            .map_err(history_error),
                        Command::HistorySetAlias { credential, alias } => {
                            let result = match alias {
                                Some(alias) => service.set_alias(credential, alias),
                                None => service.remove_alias(credential).map(|_| ()),
                            };
                            result
                                .map(|()| HistoryResponse::AliasChanged {
                                    directory: scope.clone(),
                                })
                                .map_err(history_error)
                        }
                        Command::HistoryPolicy { options, reply } => {
                            clear_preview = None;
                            let result = service.configure(options).map_err(|error| {
                                format!(
                                    "{}；策略或删除可能已经生效，请查询存储状态；未自动重试或回滚。",
                                    history_error(error)
                                )
                            });
                            let _ = reply.send(result);
                            continue;
                        }
                        Command::HistoryClear {
                            confirmed,
                            remove_aliases,
                        } => clear_history(
                            &service,
                            &scope,
                            &mut clear_preview,
                            confirmed,
                            remove_aliases,
                        ),
                        Command::HistorySnapshot { target, reply } => {
                            let _ = reply.send(service.snapshot_to(target).map_err(history_error));
                            continue;
                        }
                        Command::HistoryCounters => Ok(HistoryResponse::Counters {
                            directory: scope.clone(),
                            counters: service.handle().counters(),
                        }),
                        Command::Stop => break,
                        _ => Err("无效历史命令。".into()),
                    };
                    events.publish(result.unwrap_or_else(|message| HistoryResponse::Error {
                        directory: Some(scope.clone()),
                        message,
                    }));
                }
                let _ = finished.send(service.shutdown().map_err(history_error));
            })
            .map_err(|_| "无法创建历史控制线程。".to_string())?;
        match started.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(handle)) => Ok(Self {
                directory,
                handle,
                commands,
                worker: Some(worker),
                stopped,
                stop_requested: false,
                options: Some(options),
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("历史初始化超过 5 秒或线程失败；未关联历史服务。".into()),
        }
    }
    pub(super) fn configure(
        &mut self,
        options: history::Options,
    ) -> Result<history::PolicyReport, String> {
        let (reply, result) = mpsc::channel();
        self.commands
            .try_send(Command::HistoryPolicy { options, reply })
            .map_err(|_| "历史队列繁忙或关闭；未提交策略变更。".to_string())?;
        // 已入队后即可能提交；错误或超时不能继续信任旧策略缓存。
        self.options = None;
        let report = result.recv_timeout(Duration::from_secs(30)).map_err(|_| {
            "历史策略30秒内未确认；可能已生效，未保存配置，请查询状态，勿盲目重试。".to_string()
        })??;
        self.options = Some(report.options);
        Ok(report)
    }

    pub(super) fn snapshot(&self, target: PathBuf) -> Result<(), String> {
        let (reply, result) = mpsc::channel();
        self.commands
            .try_send(Command::HistorySnapshot { target, reply })
            .map_err(|_| "历史队列繁忙，未创建迁移快照。".to_string())?;
        result.recv_timeout(Duration::from_secs(30)).map_err(|_| {
            "快照30秒内未确认；后台可能仍在执行，禁止发布或重试覆盖暂存目录。".to_string()
        })?
    }
    pub(super) fn shutdown(&mut self) -> Result<(), String> {
        if self.worker.is_none() {
            return Ok(());
        }
        if !self.stop_requested {
            self.commands.try_send(Command::Stop).map_err(|_| {
                "历史队列繁忙或已关闭；尚未确认排空/checkpoint，不得复制数据库。".to_string()
            })?;
            self.stop_requested = true;
        }
        self.stopped
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| {
                "历史关闭未在 5 秒内确认；不得复制数据库，后台可能仍在排空。".to_string()
            })??;
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "历史控制线程异常。".to_string())?;
        }
        Ok(())
    }
}

impl Drop for HistoryOwner {
    fn drop(&mut self) {
        // 曾等待超时则不再次阻塞退出；断开 sender 后后台自行排空，绝不宣称已 checkpoint。
        if !self.stop_requested {
            let _ = self.shutdown();
        }
    }
}

pub(super) fn history_error(error: history::Error) -> String {
    match error {
        history::Error::Io(error) => {
            format!("历史读写失败（{:?}）；请检查数据目录和权限。", error.kind())
        }
        history::Error::Database(_) => "历史数据库操作失败；未清空或重建数据库。".into(),
        history::Error::Busy => "历史队列已满，请稍后查询；未自动重试。".into(),
        history::Error::Closed | history::Error::Worker => "历史服务已关闭或线程失败。".into(),
        history::Error::Invalid(_) => "历史配置、查询或文件身份无效；未静默替换历史身份。".into(),
    }
}

fn clear_history(
    service: &HistoryService,
    directory: &Path,
    preview: &mut Option<(u64, bool)>,
    confirmed: bool,
    remove_aliases: bool,
) -> Result<HistoryResponse, String> {
    if !confirmed {
        *preview = None;
        let records = service.stats().map_err(history_error)?.records;
        *preview = Some((records, remove_aliases));
        return Ok(HistoryResponse::ClearPreview {
            directory: directory.to_path_buf(),
            records,
            remove_aliases,
        });
    }
    let expected = preview
        .take()
        .ok_or("必须先请求清空预览并明确确认；未删除历史。")?;
    if expected.1 != remove_aliases {
        return Err("别名清理范围与预览不同；请重新预览，未删除历史。".into());
    }
    if service.stats().map_err(history_error)?.records != expected.0 {
        return Err("历史记录数已与预览不同；请重新预览并确认，未删除历史。".into());
    }
    service
        .clear_with_options(history::ClearOptions { remove_aliases })
        .map_err(|error| {
            format!(
                "{}；清空可能已部分完成，请查询状态，勿盲目重试。",
                history_error(error)
            )
        })?;
    Ok(HistoryResponse::Cleared {
        directory: directory.to_path_buf(),
        remove_aliases,
    })
}

fn export_all_history(
    service: &HistoryService,
    directory: &Path,
    query: history::Query,
    path: PathBuf,
    confirmed: bool,
) -> Result<HistoryResponse, String> {
    if !confirmed {
        return Err("全量导出必须明确确认路径及全部筛选结果范围；未写入。".into());
    }
    if !path.is_absolute() {
        return Err("历史导出必须使用绝对路径。".into());
    }
    let summary = service.export_all(query, &path).map_err(|error| {
        format!(
            "全量历史导出失败：{}；文件状态 {:?}，已处理 {} 行；请检查目标，未覆盖、删除或自动重试。",
            history_error(error.cause),
            error.file_state,
            error.rows_written
        )
    })?;
    Ok(HistoryResponse::Exported {
        directory: directory.to_path_buf(),
        path: summary.path,
        rows: usize::try_from(summary.rows).map_err(|_| "导出已完成，但行数超出平台表示范围。")?,
        page_only: false,
        counters: service.handle().counters(),
    })
}

pub(super) fn export_history(
    service: &HistoryService,
    directory: &Path,
    query: history::Query,
    path: PathBuf,
    confirmed: bool,
) -> Result<HistoryResponse, String> {
    use std::io::Write;
    if !confirmed {
        return Err("导出历史必须明确确认路径及当前查询页范围；未写入。".into());
    }
    if !path.is_absolute() {
        return Err("历史导出必须使用绝对路径。".into());
    }
    // 先完成查询再创建文件；create_new 拒绝现有文件、链接及数据库覆盖。
    let report = service.query(query).map_err(history_error)?;
    let text = report.export_csv();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("无法新建导出文件（{:?}）；已有目标绝不覆盖。", e.kind()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| {
            format!(
                "导出写入失败（{:?}）；部分文件保留于 {}，未报告成功。",
                e.kind(),
                path.display()
            )
        })?;
    Ok(HistoryResponse::Exported {
        directory: directory.to_path_buf(),
        path,
        rows: report.details.len(),
        page_only: true,
        counters: service.handle().counters(),
    })
}

pub(super) fn history_options(config: &config::HistoryConfig) -> history::Options {
    history::Options {
        queue_capacity: 1024,
        retention_days: config.retention_days,
        max_records: config.max_records,
    }
}
