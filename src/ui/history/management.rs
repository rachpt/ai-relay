//! 历史凭据别名、保留策略、存储占用与清空状态。
use super::*;
use crate::history::{CredentialAlias, CredentialId, StorageStats};

impl History {
    pub(super) fn alias_command(&self, remove: bool) -> Result<Command, String> {
        let credential = CredentialId::parse(settings::text(self.control(ALIAS_ID)).trim())
            .map_err(|_| "必须选择或填写完整64位本地 HMAC ID，不能填写 API Key。")?;
        let alias = if remove {
            None
        } else {
            Some(
                CredentialAlias::new(settings::text(self.control(ALIAS_LABEL)).trim())
                    .map_err(|_| "别名需1–64字且不含控制字符或常见密钥格式；请勿粘贴密钥。")?,
            )
        };
        if !self.credentials.contains(&credential) {
            return Err("请先查询包含该凭据的历史，再从已查询的 HMAC ID 设置别名。".into());
        }
        Ok(Command::HistorySetAlias { credential, alias })
    }
    pub(super) fn policy(&self) -> Result<(u32, u32), String> {
        let days = settings::text(self.control(RETENTION))
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|v| (1..=90).contains(v))
            .ok_or("保留天数应为1–90")?;
        let records = settings::text(self.control(MAX_RECORDS))
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|v| (1..=100_000).contains(v))
            .ok_or("记录上限应为1–100000")?;
        Ok((days, records))
    }
    pub(super) fn storage(&self, stats: &StorageStats) {
        settings::set_text(
            self.control(RETENTION),
            &stats.options.retention_days.to_string(),
        );
        settings::set_text(
            self.control(MAX_RECORDS),
            &stats.options.max_records.to_string(),
        );
        settings::set_text(
            self.control(STORAGE_TEXT),
            &format!(
                "历史存储：{} 条记录；{} 个别名\r\n数据库 {} B；WAL {} B；共享内存 {} B；合计 {} B\r\n可复用数据库页 {} B（不代表已回收磁盘空间）。\r\n当前策略：{}天／{}条，先到先清理；不影响真实 usage，不删除配置、密钥或日志。",
                stats.records,
                stats.aliases,
                stats.database_bytes,
                stats.wal_bytes,
                stats.shared_memory_bytes,
                stats.total_bytes(),
                stats.free_database_bytes,
                stats.options.retention_days,
                stats.options.max_records,
            ),
        );
    }
    pub(super) fn clear_report(&mut self, message: &str) {
        self.applied = None;
        self.report = None;
        settings::set_text(self.control(2330), message);
        settings::set_text(self.control(2331), "");
        // SAFETY: live UI-owned list view; scalar delete message.
        unsafe {
            SendMessageW(self.control(TABLE), LVM_DELETEALLITEMS, 0, 0);
        }
        self.invalidate();
    }
}
