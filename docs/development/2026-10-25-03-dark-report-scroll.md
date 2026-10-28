# 2026-10-25-03：修复暗色统计表滚动期间的白色条带

## 目标与范围

按用户统计分析请求明细截图，修复滚动期间右侧原生滚动条区域闪白。仅改变暗色Report列表的呈现，保留原生滚动状态、单元格复制及浅色／高对比度回退；工作区已有日期边角、日志显隐及其他修改保留，不提交、不改版本、不覆盖dist或用户数据。

## 根因与实际变更

- 原来先`DefSubclassProc`再暗色overlay，接管鼠标并不能阻止ListView在`LVM_SCROLL`内部直接画系统色条。新增下层探针在native返回、暗色补画之前测得轨道`0xF0F0F0`，复现中间帧而非依赖释放后截图。
- `src/ui/scroll_theme/paint.rs`：只给暗色Report启用局部`WS_EX_COMPOSITED`，短暂持有window DC贯穿native处理与同DC暗色补画；不向原生发送内部禁绘，不改变范围／可见性。排除不兼容class style、几何事务及菜单快捷键，避免跨模态菜单长期持有DC。
- `WM_NCPAINT`复制更新region并扣除横纵条和交角，保留native边框及借用region。资源分配失败允许原生可读降级。
- `src/ui/scroll_theme.rs`：接入screen DC路径，仍检查redraw／defer／attachment；样式所有权、转换中guard及最新主题意图有界收敛，样式回调中嵌套浅深切换／销毁后不操作旧实例。只清本模块添加的COMPOSITED位。
- `src/ui/cell_table.rs`：仅COMPOSITED扩展样式位变化不清活跃单元格，真正样式／数据变更仍使旧复制目标失效。
- 新增`src/ui/scroll_theme/paint_tests.rs`六项回归（五项默认＋一项需独占桌面的显式physical），以及`src/ui/cell_table/tests.rs`两项复制目标回归。另保留原有20项滚动及其他测试。
- 设计及备选方案见[ADR0017](../decisions/0017-dark-report-scroll-presentation.md)。无新依赖／配置／数据迁移／代理改动。

## 验证

Windows x64 MSVC，依赖锁定离线；GUI与原生窗口测试串行运行，不访问真实模型或使用真实Key。

| 命令／范围 | 结果 |
|---|---|
| `cargo fmt --check`；额外`rustfmt --check --edition 2024 src/ui/scroll_theme/paint_tests.rs` | 通过 |
| `cargo check --all-targets --locked --offline` | 通过 |
| `cargo test --lib ui::scroll_theme::tests:: --locked --offline -- --test-threads=1` | 25项通过；physical默认ignored，另显式执行 |
| `cargo test --lib ui::cell_table::tests:: --locked --offline -- --test-threads=1` | 14项通过；注入writer不写系统剪贴板 |
| `cargo test --locked --offline -- --test-threads=1` | 全量通过：265库＋153集成，418项；1项physical默认ignored，doc 0项 |
| `cargo test --lib report_scroll_never_exposes_native_track_before_dark_overlay --locked --offline -- --ignored --test-threads=1 --nocapture` | 最终显式通过；48个native返回前物理轨道样本均暗色，含横纵thumb、按住repeat、wheel／定位；初末／按住窗口所有权严格匹配 |
| `cargo clippy --all-targets --locked --offline -- -D warnings` | 通过 |
| `cargo build --locked --offline` | Debug通过 |
| 静态CRT `cargo build --release --locked --offline` | 通过，123.867秒 |
| Release x64 PE32+／导入／`--version` | 通过；无额外CRT／SQLite／OpenSSL运行库，版本2026.10.18，退出码0 |
| `tools/smoke-history-ui.ps1 -ExePath <本批Release绝对路径>` | 未通过：首次StartGui切换Tab的1秒同步消息超时，尚未查询；未完成本轮Release统计页面GUI验收 |
| 新paint与cell文件诊断 | 无错误／告警 |

物理专项用普通Report ListView而非owner-data模拟，读取桌面DC而不是PrintWindow或会改变合成提交时机的ListView DC；在native返回后、上层补画前采样。默认ignored是避免普通并行／离线测试误操作桌面，不代表本批未执行。

## 调查失败与修复

- 最初native probe确实读到系统浅灰。`DefWindowProc`禁绘实验使DC裁剪为空并破坏重入Hide；正常ListView禁绘实验又使巨大ENSUREVISIBLE后的range延迟。两者均被现有回归拒绝，最终实现不含这些禁绘方案。
- 初期physical采样被其他窗口遮挡，WindowFromPoint并非自有列表，未视为通过。完成自有窗口前台就绪后严格检查所有权，最终通过；不关闭其他应用、不把遮挡样本改算成功。
- 独立只读review发现菜单键跨模态循环持DC、合成style变化清单元格、样式changing/changed中主题交错的P2，已定点修复并补回归；最终针对这些问题复核未发现剩余确定P1/P2，不冒称完整系统验收。
- cell回归揭露SetWindowLongPtr成功后的last-error会被同步回调污染，改为查询存活实例的实际样式位，恢复所有权判断。
- 最终Release历史GUI冒烟在`smoke-history-ui.ps1:665`调用SelectTab超时，状态为“历史尚未查询”。失败原因未进一步隔离，不断言与本修复无关、不以增加超时／重复执行掩盖；工具结束后查询无AI Relay残留进程。

## 产物

- 最新Debug：`target/debug/ai-relay.exe`。
- 最新静态CRT Release：`target/release/ai-relay.exe`，7,898,112字节。
- Release SHA256：`B3478B632625A1805F623652A86008E159D53FC0CCE509DCA85FC7F82512C277`。
- 构建／PE／版本报告：`target/release-build-2026-10-25-report-scroll.json`；临时脚本与版本输出同在忽略的target，不覆盖既有证据。
- 基线HEAD `7de0eda`＋工作区未提交日期／日志及本批修复，版本仍`2026.10.18`，未更新dist／显式target旧包。

## 剩余限制与下一步

- 最终Release统计页完整GUI仍未通过；物理中间帧证明来自Debug native回归，不冒称Release成品全部验收。
- 实际150%／200%、多屏、真实系统高对比度、Win10/11差异、硬件键鼠、读屏及连续视频仍待执行。
- 先前日志grip、菜单纯黑、浅色月历页脚等失败保持原记录，本批不改变其结论。
