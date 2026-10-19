# 2026-10-19-08：修复待提交 review 的四项交互与冒烟问题

## 目标与范围

落实用户要求的上一轮独立 review 四项 P2：共享 viewport 跟踪跨页写错位置、键盘增高日志区后持焦 grip 出界、日志冒烟同步缩放可能阻塞，以及启动时托管进程 Path 尚未就绪的假失败。保留此前所有未提交内容，不顺带重构菜单、配置或代理。

## 实际变更

- `src/ui/scroll_theme.rs`：新增仅取消本模块自有交互的 `cancel`。复用保留 Rc 的清理路径，在释放 capture 前清除 tracking、timer、线程局部键盘 hook、hover 和 leave 状态；不向控件泛发取消消息，不释放原生 Edit 选区或其他窗口的捕获。
- `src/ui/viewport.rs`：收到活动页消息时，仅在 active 真正变化前取消旧跟踪。`ReleaseCapture` 可能同步重入或销毁宿主，返回后通过 proc/id/data 核验仍安装原 Page，再继续访问状态。同页 prepare/refresh 不取消正常拖拽，范围和位置仍由原生控件维护。
- `src/ui/viewport.rs`、`src/ui/logs/resize.rs`：提取异步 `reveal(child)`，复用原焦点揭示消息；日志 grip 的方向键/Home 调整完成同步布局后，仅当 grip 仍 attached 且持焦才排队揭示。消费时再次核对父窗口、焦点和可见性。鼠标拖拽不请求揭示，保持屏幕坐标锚点。
- 新增4项原生回归：同范围两页双向切换，在 thumb/line/page 跟踪期间清理捕获及 timer/hook，旧 timer/mouse 消息不覆盖两页偏移；取消不干扰选区/其他捕获；切页取消中 `WM_CAPTURECHANGED` 同步销毁真实 viewport；96/144/192参数下两种短窗口持续按方向键/Home，确认 grip 完整可见且正文、选区和焦点保持。
- `tools/smoke-logs-interaction-ui.ps1`：缩放改为 `SWP_ASYNCWINDOWPOS | SWP_NOZORDER | SWP_NOACTIVATE`，返回的是请求目标，不当作已完成。先有界观察完整外框，再在日志页核对宿主外框位置/宽高、client、scroll page/extent、editor/grip 和边距，包含仅宽度变化的情况；保留工作区钳制及总预算。
- 工具始终用原始 process handle 核验存活/PID/实际 EXE，并核对保留的 PID/handle/start time。仅托管 Path 为空时有界 Refresh 等待，非空不匹配立即失败；正常阶段最多5秒，清理阶段每次最多1秒且受总预算截止控制，退出不强杀。启动动态状态前缀改为一次有界 marshalled WM_GETTEXT，稳定日志正文仍严格检查长度、复制数量及 SHA256。
- ValidateOnly 增加异步 flags、完整目标矩形与 Path pending/match/mismatch 的纯函数自检，不启动进程/GUI/网络。更新使用说明与状态。
- 无新增依赖、配置迁移、需求默认值变化或系统设置修改。并行进入工作区的深色标题菜单属于07批，未覆盖或回退；不将其实现归为本批。未提交、建分支、改版本/标签或更新 dist。

## 验证

| 命令或验收项 | 结果 | 说明 |
|---|---|---|
| `cargo fmt --check` | 通过 | 最终工作区 |
| `cargo check --all-targets --locked` | 通过 | Windows x64 MSVC |
| `cargo test --locked --lib ui::scroll_theme:: -- --test-threads=1` | 通过 | 首轮19项；随后新增的同步销毁测试及切页测试2项定向均通过，最终库测试包含全部20项 |
| `cargo test --locked --lib ui::viewport:: -- --test-threads=1` | 通过 | 9项 |
| `cargo test --locked --lib ui:: -- --test-threads=1` | 通过 | 171项，含键盘可见性及4项本批回归；144/192是参数化原生测试，不冒称桌面DPI验收 |
| `cargo test --locked --lib -- --test-threads=1` | 通过 | 最终236项库测试 |
| 15个集成测试目标分段执行 | 通过 | 153项，无 skip；配置/日志/代理/usage 12目标与 history 3目标分别串行通过；合计389项 |
| `cargo clippy --all-targets --locked -- -D warnings` | 通过 | 最终独立执行，无告警 |
| `cargo build --locked` | 通过 | Debug已更新；后续并行菜单批次也更新同一程序 |
| 静态CRT `cargo build --release --locked` | 通过 | 当前进程编码flags及编译路径映射，无持久配置修改；最终同路径程序还含并行菜单修复 |
| Release PE32+/x64/导入检查 | 通过 | 未发现额外CRT/SQLite/OpenSSL DLL导入，不替代干净系统验收 |
| 日志工具 `-ValidateOnly` | 通过 | C#声明编译、PowerShell调用检查与新增纯逻辑自检 |
| Debug/Release原问题定向GUI | 通过 | 实际96 DPI、真实页面键盘选择与日志Tab焦点路径；旧页40/目标页120，切页即释放capture，松手仍120；grip保持456–468且offset从66变90，而非移至480–492 |
| 受阻线程异步缩放探针 | 通过 | 临时自有隐藏GUI线程有硬上限门闩，实际复用工具PostResize；目标阻塞时约1ms返回且未假报完成，恢复后16–19ms观察到目标矩形，无应用/用户配置操作 |
| Debug完整日志交互矩阵 | 定向功能通过，非全验收 | 最终96 DPI、约33.94秒，Passed=true/Complete=false；33张有效按住条带图271,524采样亮白像素0，3处遮挡跳过 |
| 最新Release完整日志交互矩阵 | 定向功能通过，非全验收 | 最终96 DPI、约35.69秒，Passed=true/Complete=false；33张有效按住条带图271,524采样亮白像素0，3处遮挡跳过；配置/闭合样本日志不变、默认高度恢复、正常退出 |
| 独立只读复核 | 已执行 | 确认4项根因修复、Page重入边界、捕获所有权和延后reveal；另指出等待宿主宽度的遗漏，补齐后复核未发现本修复范围新的确定P1/P2 |

### 最终分段测试命令

```text
cargo test --locked --lib -- --test-threads=1
cargo test --locked --test config_foundation --test config_persistence --test logging_backend --test logging_paging --test proxy_core --test proxy_features --test proxy_history --test proxy_logging --test proxy_metrics --test proxy_model --test proxy_usage --test usage_observer -- --test-threads=1
cargo test --locked --test history_backend --test history_reader --test history_reports -- --test-threads=1
```

以上236库＋153集成均实际通过；不是将全量命令超时记为成功。

## 失败、取证与验证限制

- 初次切页新增测试把 page-after 场景设置为小范围，原生一次分页已到末端并合法取消捕获，测试因此失败。仅该场景改用4000虚拟高度，thumb/line保持820，同页不取消与切页清理断言均未降低。
- 并行菜单新增测试曾短暂有 Win32 常量类型编译错误，后由其批次修正；本批未改菜单文件。一次Cargo测试在自有Debug GUI仍运行时无法替换EXE，随后改为不与GUI冒烟并行。
- 整组fmt/check/test/clippy曾分别触及240/300秒上限，未把剩余项当作通过，也未无限加长重跑。最终按库及15集成目标分段执行并补独立Clippy，全部通过。
- GUI曾因其他实例存在拒绝启动；未操作该实例。一次临时探针的DPI检查不符合前提而正常中止，后使用线程级DPI感知及实际测量核实96 DPI，不修改系统缩放。另一次临时探针启动状态WM_GETTEXT超时，记录为失败，不能算最终再次复验通过。
- 异步外框已更新但日志子控件布局尚未完成，曾使工具几何断言失败；新增真实后代布局等待，独立复核补齐仅宽度变化条件。启动状态多次读取文本变化曾使严格读取失败，改用一次有界前缀读取；没有放宽稳定日志内容校验或像素阈值。
- **早轮Debug真实截图 `arrow-2200-1-timer-repeat-HELD` 捕获到白色条带，约3,067亮白像素，报告保持失败。** 当时已检查图片，并非以离屏图替代或提升阈值；本批未改变原生绘制实现，也未确认这次白条的完整根因。后续Debug和Release串行运行通过且采样无白，不据此宣称任意时序白条已根治。该偶发视觉现象仍须独占桌面定向取证。
- GUI矩阵仍有其他窗口遮挡跳过；物理键鼠、连续视频、高DPI/多屏、真实高对比度/IME/读屏保持未验收。异步缩放假设输入队列独立，工具不附加队列，不承诺对恶意附加队列硬实时抢占。

## 产物与证据

- 最新程序：`target/debug/ai-relay.exe`、`target/release/ai-relay.exe`，版本仍为2026.10.18；Release 7,881,216字节，SHA256 `356CAA0CCBBA8D4B998DBE603B350AF772861A2437A61EADC520670F4E473162`。
- Debug最终报告：`target/logs-interaction-ui/20261019-152324-c2e99b77/report.json`。
- 最新Release最终报告：`target/logs-interaction-ui/20261019-155640-8b431bca/report.json`；此前Release通过报告为 `target/logs-interaction-ui/20261019-151010-739c7b84/report.json`。
- 白条失败报告：`target/logs-interaction-ui/20261019-150252-23a670da/report.json`，未覆盖；早期布局与动态文本失败亦保留在日志专项输出目录。
- 两项原问题和受阻线程探针曾在 `target/review-ui/` 生成临时报告，完成后本会话后段检查发现该目录已被外部操作清空。这里保留真实终端观察结果，不能再将已不存在的临时文件作为可点击证据；长期回归已在源码测试中，未为收尾重新引入被移除的临时脚本。
- 本批自建应用实例均正常退出；未改真实配置、Key、模型调用、剪贴板、Startup、防火墙或系统代理，无新增费用。测试私有fixture按工具既有规则保留。

## 下一步

- 在无其他GUI测试/窗口遮挡的桌面隔离早轮Debug按住repeat的白条现象，记录原生绘制消息时序，确认根因后再决定修复；不把本批4项修复与未确认视觉问题混为同一结论。
- 按既有矩阵继续实际高DPI/多屏/高对比度/读屏验收，dist仍为旧预览包。
