# 2026-10-31-03：提交前UI复核与禁用时Key重掩码

## 目标与范围

按用户授权，对当前 `master` 全部未提交的源码、测试、工具和文档执行review、必要修复、验证及提交推送。包含前两批浅深滚动条／小图形／设置输入及统计状态自适应布局，不删除或回退已有工作，不改依赖、配置schema、代理协议、版本或dist。

## Review与实际修复

三个独立只读会话分别检查设置及生命周期、滚动条及小图形绘制、统计布局；统计会话另检查两个修改工具，主agent核对文档、调用链、暂存与数据边界。

- **确定P2，已修复：**`src/ui/settings/panel.rs` 的 `enable()` 原先只禁用控件；从上游分类通过全局菜单导入／导出时，`action_busy`／`file_task` 不经过 `disable()`，已显露Key仍保持明文，隐藏按钮却已禁用。现在统一在busy、action_busy、file_task或缺少配置导致Key操作不可用时先 `hide_secrets()`，再更新控件启禁。`modal`仍只拦截命令，不擅自改变其原有启禁语义。
- `src/ui/settings/tests/lifecycle.rs`：增加四项从已显露未保存Key出发的门控回归，以及modal对照；检查重掩码、stale command、解除门控不自动显露、UTF-16选区和完整草稿／已保存配置保留。四项修复前全部失败、修复后全部通过。
- **撤回一项静态误判：**初步认为显隐后按 `EM_GETSEL` 边界回写会改变反向选区活动端。首版 `EM_SETSEL(14,4)` → Shift+Right 探针连独立原生EDIT也得到 `(4,15)`，不能据此归因生产代码。进一步从collapsed caret 14经原生Shift+Left十次建立真实反向选择，再Shift+Right；原生掩码对照与现有三种显隐路径均得到 `(5,14)`。因此不修改生产 `secret.rs`，保留严格基线及四项有效回归；移除本轮临时诊断／打印／焦点探针，没有删减用户原有测试。
- `src/ui/control_theme.rs` 的switch测试及 `src/ui/status_bar/paint_tests.rs`：每条 `WM_PRINT`／`WM_PRINTCLIENT` 前独立清空完整私有bitmap，再恢复借用DC及复杂clip；每条分别验证前景、抗锯齿、裁剪洞及DC状态，避免另一条消息留下像素而假通过。生产绘制不变。
- 最终只读定向复核未发现本次修复及新增测试中的确定新增问题。全程由主工程统一构建，子agent未运行Cargo／GUI，不创建共享target源码快照。

## 验证

Windows x64 MSVC，Rust／Cargo 1.99.1；构建检查和测试使用 `CARGO_INCREMENTAL=0`、锁定离线依赖。集成测试仅本地mock／虚构凭据，无真实模型调用。

| 命令或范围 | 结果 |
|---|---|
| `cargo fmt --all --check` | 最终通过；新增测试首轮仅格式失败，`cargo fmt --all` 修正后重查 |
| `CARGO_INCREMENTAL=0 cargo check --all-targets --locked --offline` | 通过 |
| `CARGO_INCREMENTAL=0 cargo clippy --all-targets --locked --offline -- -D warnings` | 通过 |
| `CARGO_INCREMENTAL=0 cargo test --locked --offline -- --test-threads=1` | 完整命令退出0：356库测试＋153集成＝509项通过，另4项physical默认ignored，doc 0项无失败 |
| `native_enable_` 四项定向回归 | 修复前4项失败，修复后4项通过；完整测试再次覆盖 |
| `reverse_selection_active_end` | 首版无效探针及原生对照失败，作为误判证据保留；修正构造方式后4项通过，不放宽结果断言 |
| switch／status两项打印补强回归 | 分别通过，完整测试再次覆盖 |
| modern／ui-regressions `-ValidateOnly` | 两者均通过PowerShell解析、合并C#编译及纯helper自检；没有启动GUI |
| 编辑器diagnostics／`git diff --check` | 无错误警告／无空白错误；Git行尾规范提示另计 |
| 变更文件敏感模式检查 | 本地规则未发现私钥、常见Provider／Git令牌或本机用户路径；7处打印命中均为cfg(test)中的既有原生验证输出，不是新增生产调试残留。detect-secrets本次环境不可用，不将简易扫描视为完整秘密保证 |
| 远端同步前检查 | `git fetch --prune origin` 成功，HEAD与origin/master为0 ahead／0 behind，无需rebase；提交及推送结果以本次操作回执为准 |

收尾辅助测试枚举有一次未带 `CARGO_INCREMENTAL=0` 的命令触及30秒外部上限，未取得输出，不作验证通过；确认无残留构建后，带一致环境重新枚举成功，513项注册测试（360库含4项ignored＋153集成），与完整执行的509项通过一致。此枚举只核对数量，不替代此前完整测试。

## 未完成与限制

- 本批未重建Release、未重跑GUI矩阵或4项显式physical测试；前两批截图／Release验证不冒充本次修复后的成品验收。现有 `target/release/ai-relay.exe` 仍是02批构建，不包含本批重掩码修复，dist与显式target历史成品未变。
- 实际高DPI／多屏、真实高对比度／读屏、IME及物理键鼠、Win10／11干净系统和连续刷新视觉仍按STATUS待验收。线程局部键盘状态测试不等于物理键盘或焦点验收。
- 状态测高沿用 `settings::text()` 的16,384 UTF-16单元上限。完整状态写入未设置相同上限，极长导出路径是否可由当前系统文件对话框到达尚未确认；保留为边界疑点，不据此扩大生产修复范围或宣称任意路径文字已完整验收。
- 分配／驱动失败仅已有正常及超限回退覆盖，未注入真实OOM／驱动故障；既有Light物理遮挡、菜单／日期及Release历史工具超时限制不解除。

## 下一步

完成本批已验证变更的提交推送；后续重新构建含此修复的静态CRT Release，在独占桌面按上述限制继续验收，不因源码提交推断旧二进制已更新。
