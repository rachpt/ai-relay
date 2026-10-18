# 核心模块二次拆分

日期：2026-10-18

## 目标

在不改变行为、线程模型和状态所有权的前提下，缩小上一轮只读检查中最集中的四个源文件：

- `src/app/controller.rs`
- `src/ui/history.rs`
- `src/ui/settings.rs`
- `src/ui/logs.rs`

约束是单个 Cargo package、不增加依赖、不拆 workspace、不引入新线程/锁，也不把 Win32 控件逻辑搬到阻塞路径。开始本批时工作区没有未提交改动；本批未与另一个 agent 并行修改。

## 实际变更

### `app::controller`

原 3433 行文件拆为：

| 文件 | 职责 |
|---|---|
| `mod.rs` | `Controller` 句柄、有界通道创建、退出标志 |
| `types.rs` | 命令、响应、快照和事件类型 |
| `results.rs` | 固定容量结果邮箱与溢出通知 |
| `history.rs` | 独立历史控制线程、查询/导出/清空辅助 |
| `session/mod.rs` | `Session` 状态、加载/保存/启停、窗口偏好 |
| `session/history_ops.rs` | 历史服务关联和策略命令 |
| `session/config_ops.rs` | 自启、导入导出、恢复、目录迁移 |
| `session/log_ops.rs` | 日志服务初始化和命令路由 |
| `logging.rs` | 日志选项、错误文本、有界视图格式化 |
| `util.rs` | 路径比较、打开目录、概览快照、错误文本 |
| `worker.rs` | 后台控制线程主循环与退出清理 |
| `tests.rs` | 原有控制器测试 |

`Session` 仍是控制线程独占状态，`worker` 仍是单入口；没有新增线程、通道或锁。`Controller` 对 UI 暴露的字段和命令类型保持原路径。

### `ui::history`

原 2045 行拆为：

- `mod.rs`：类型、控件 ID、命令入口和绘制分派；
- `layout.rs`：控件创建、显隐、布局、字体；
- `events.rs`：交互、在途操作、后台结果轮询；
- `filters.rs`：日期边界、筛选草稿解析；
- `table.rs`：表格、分组/趋势文本、详情单元格；
- `management.rs`：凭据别名、保留策略、存储与清空；
- `tests.rs`：原测试。

### `ui::settings`

原 1877 行拆为：

- `mod.rs`：控件句柄、常量、文本/选择/确认辅助、只读访问器；
- `panel.rs`：控件创建、分类布局、启用状态；
- `draft.rs`：控件到配置的草稿转换与字段校验；
- `commands.rs`：保存/启动/停止/导入导出命令路由；
- `events.rs`：文件任务、自启、配置和状态事件轮询；
- `tests.rs`：原测试。

### `ui::logs`

原 2001 行拆为：

- `mod.rs`：页面控件、渲染、交互和轮询；
- `filter.rs`：有界文本处理、搜索/级别/日期/状态筛选；
- `tests.rs`：原测试。

测试模块移入各目录的 `tests.rs` 只改善阅读与定位，未被当作架构完成度指标。搬运时未主动改写错误文本、确认文案、上限和校验规则；复核时对照 HEAD 原文检查了非测试函数体与字符串。

## 复核

- 对照 `HEAD` 的四个原文件，提取非测试函数体与字符串字面量检查：函数体均有对应实现，字符串无缺失；差异仅为模块路径、类型别名和格式化产生的等价变化。
- 复核发现并修正了两处可收敛的边界：删除 `controller` 中不必要的 `HistoryEvents` 类型别名；把 `Session`/`HistoryOwner` 的字段及仅模块内使用的方法从 `pub(crate)` 收窄到 `pub(super)`，把 `history/layout` 和 `settings/panel` 中仅内部使用的方法收窄到 `pub(super)`。
- 复核未发现命令顺序、线程归属、状态所有权或 UI 调用路径变化；但现有测试不能替代真实窗口、托盘、DPAPI/IPC 和 DPI 验收。

## 验证

工作区：Windows，Rust/Cargo 1.99.1，MSVC 目标。

```text
cargo fmt --check
cargo check --all-targets
cargo test -- --test-threads=1
cargo clippy --all-targets -- -D warnings
```

结果：

- `cargo fmt --check`：通过。
- `cargo check --all-targets`：通过。
- 全量串行测试：296 项通过，0 失败（144 项 lib 测试 + 152 项集成测试）。
- 严格 Clippy：通过，无 warning。
- 未运行 GUI 实机截图、DPI、托盘、真实 IPC 或自启验收；本批目标是无行为变化的模块搬运，`cargo check --all-targets` 只证明编译与类型边界，不替代 Windows GUI 验收。

说明：直接运行默认并行的 `cargo test` 时，4 项 GDI 资源计数测试因并行测试共享同一进程 GDI 计数而失败（基线快照同样如此）；项目既有验证约定是全量串行，本记录只报告 `--test-threads=1` 的结果。

## 未完成与限制

- `controller/tests.rs` 仍有约 1151 行，后续可按配置、历史、日志、IPC 分组继续拆；本次没有为拆测试而改动测试逻辑。
- `ui/logs/mod.rs` 仍同时承担控件创建、渲染和事件轮询，后续可再按 `layout`/`events` 分离。
- `ui/mod.rs`、`scroll_theme.rs`、`overview_dashboard.rs` 等未在本批处理；`scroll_theme` 的大体量主要来自内嵌测试。
- 更大的 `ui/history/layout.rs`、`ui/settings/panel.rs` 已低于原文件的一半，但仍可继续按控件组细分。
- 没有执行 Release 构建、GUI 冒烟、物理输入、多屏/高 DPI 或辅助功能验收，不能据此宣称正式发行质量提升。
