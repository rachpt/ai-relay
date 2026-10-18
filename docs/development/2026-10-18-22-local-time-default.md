# 2026-10-18-22：默认使用系统本地时区

## 目标

- 软件时间默认使用用户的系统时区，不再要求用户先手动切换本地。
- 保留 UTC 存储和统计中的显式 UTC 选项，不迁移或改写已有历史、日志。

## 实际变更

- `src/history/types.rs`：`TimeZone` 和 `Query` 默认使用 `Local`，日历规则仍取各历史时刻的系统时区，非固定当前偏移。
- `src/ui/history/layout.rs`、`events.rs`：首次创建选中“本地”，重置筛选恢复本地；保持 UTC/本地既有选项顺序，重置只改草稿，不覆盖已应用结果或隐式查询。
- `src/ui/logs/filter.rs`、`mod.rs`：日志日期按系统本地整日解释，起止均包含；独立解析首尾边界，兼容 DST 23/25 小时、午夜缺失/重复和整日不存在。数值 epoch 解析保持绝对 UTC 秒，GUI 仍仅接受 ISO 日期或空值。
- 实时日志增加本地 RFC3339 时间及偏移，保留原始 epoch/序号/事件文本；文件展示副本增加 `timestamp_local`，保留原有 JSON 字段和数值精度，复制后仍为 JSON 但包含展示字段。元数据筛选始终使用原始 UTC 值；同时支持原始文本和本地时间搜索。无效时间保持原文，有界显示计入新增字符。
- 新增默认/重置的原生控件测试、默认查询本地日历分组测试；日志通过可注入时区覆盖正负/半小时偏移、DST、午夜边界、缺失日期、显示/搜索及容量。既有明确验证 UTC 的测试改为显式指定 UTC，不依赖主机时区。
- 同步历史、日志、modern 冒烟脚本的默认值/本地日期和实时行格式假设。日志 fixture 从本地日界生成 epoch，保留文件字节不变检查。
- 更新 `README.md`、`REQUIREMENTS.md`、`docs/UI-DESIGN.md`、状态与记录索引。未新增依赖、配置字段或数据库 schema。

## 验证

工具链不在 shell PATH，使用实际安装的 Cargo 可执行文件；本机 Windows、China Standard Time，未修改系统时区或时钟。

| 命令或验收项 | 结果 | 说明 |
|---|---|---|
| 首轮 `cargo check --all-targets` | 失败后修复 | 新测试误用未实现的 LogEntry 反序列化及另一 crate 的 TimeZone 类型；改为 JSON Value 验证与测试模块自身类型 |
| `cargo fmt --check` | 通过 | 先执行 fmt，最终检查通过 |
| `cargo check --all-targets` | 通过 | Windows 全目标 |
| 定向默认/重置及日志测试 | 通过 | 原生 HWND 与筛选/格式化回归 |
| `cargo test -- --test-threads=1` | 通过 | 全部 318 项，无失败 |
| `cargo clippy --all-targets -- -D warnings` | 通过 | 严格警告检查 |
| `cargo build` | 通过 | 更新 `target/debug/ai-relay.exe` |
| `smoke-history-ui.ps1 -ExePath target/debug/ai-relay.exe` | 通过 | 53 条本地 mock 请求，UTC/本地范围、排序、分组、导出和管理回归 |
| `smoke-logs-ui.ps1 -ExePath target/debug/ai-relay.exe -RequireAccessLog` | 通过 | 96 DPI 最小/放大窗口，真实本地 mock 日志、本地标签/日期、组合筛选、暂停、分页/跟随、UTC epoch 和文件字节不变 |
| `smoke-modern-ui.ps1 -ExePath target/debug/ai-relay.exe` | 部分通过后失败 | 初始 Local、重置 Local、显式 UTC/Local 自动查询、粒度和 pending 冻结通过；随后设置预览阶段因脚本仍将分类2039视为 ComboBox 而失败。该分类已为分段 Tab，此无关旧假设未在本批修改，不能称整脚本通过 |
| `git diff --check` | 通过 | 无空白错误；部分文档/脚本有 CRLF→LF 规范化提示 |

集成验证只使用临时目录、本地 mock 和虚构凭据，不访问真实模型、不改真实 Startup、防火墙或用户配置。测试计数通过测试清单单独核对；一次 PowerShell 计数命令因 shell 引号失败，改用 Python 调用 Cargo 清单成功，不影响测试结果。

## 未完成与限制

- Release、静态 CRT 发布包及 `dist/` 未重建；新行为目前在源码和 Debug EXE。
- 多种实际系统时区、高 DPI、多屏、物理输入和读屏未在本批验收；可注入时区单元测试不能替代这些验收。
- 综合 modern 冒烟的设置分类控件假设仍待独立更新；本批不修改无关设置行为。
- “本地”指运行本软件的 Windows 系统时区，不从客户端 IP 推断时区，不提供任意 IANA 时区选择。

## 下一步

- 在其他实际系统时区及高 DPI 环境复核，再按需要重建预览包。
- 独立更新 modern 冒烟的设置分类控件类型假设并重跑完整脚本。
