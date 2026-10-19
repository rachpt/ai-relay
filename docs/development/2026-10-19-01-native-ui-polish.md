# 2026-10-19-01：原生界面视觉与组件精修

## 目标

继续改善界面粗糙、控件像密集方框的问题，形成一致的浅深配色、操作层级和组件细节。保持 Rust + Win32、四页布局及原生输入/无障碍语义；不改变代理、配置、数据或权限行为。

## 实际变更

- `src/ui/theme.rs`：浅色冷灰背景/白色表面、深色蓝灰背景/抬升表面，统一文字、边线、强调色及中性/hover色；主要与次要文字有对比度回归。四个主导航使用圆角胶囊选中背景，保留内嵌 SVG 图标与原生 Tab 命中、键盘及焦点。
- 新增 `src/ui/buttons.rs`：实心主要动作、柔和中性次要动作、淡红危险动作；区分 hover/按下/禁用/键盘焦点。原生 Button 继续处理输入、命令和完整文本；绘制及打印通过 RAII 恢复 DC。
- `choices.rs`：一体式中性圆角轨道，选中项独立表面与轻微阴影；去掉逐项蓝边框，增加真实 hover，保留原生 item 几何、方向键及 CB 兼容消息。
- `control_theme.rs`：浅深闭合下拉统一边距/细箭头/圆角与 hover；switch 使用实心强调色轨道、对比旋钮和中性关闭态；浅深表头统一柔和底色/底线，表格增加微弱交替行与选中层级，不替换原生行/单元格选择。
- 新增 `input_frame.rs`：仅在实际非客户区绘制矩形细边框与聚焦色，明确排除整个真实客户区，不裁剪 Edit、不覆盖 caret/IME。设置/历史/日志的既有有边框 Edit 在创建时增加 `WS_EX_CLIENTEDGE`，确保真实非客户区存在；主题过程不修改输入样式、选区、密码掩码或窗口 region。详见 [ADR0013](../decisions/0013-native-control-polish.md)。
- `overview_dashboard.rs`、`history_chart.rs`、`status_bar.rs`：卡片层级、统计摘要圆角分块、细分隔线、状态栏配色；概览页标题改为“运行概览”，窗口仍明确为开发版。高对比度不使用次要文字色代替正常正文。
- `tools/smoke-modern-ui.ps1`：修正设置分类2039已是原生分段选择、页面位于宿主2400的旧假设；补四页×浅深×两种尺寸的截图/布局矩阵。明确 PrintWindow 仅为离屏渲染，不作为桌面残影验收。
- 更新回归工具的 SVG 图标 Palette 预期；新增原生按钮/输入框/真实应用创建接线、交互色对比度、鼠标状态、打印 DC 与 GDI 回归。无 Cargo 依赖/锁文件/版本号变更。

## 验证

| 命令或验收项 | 结果 | 说明 |
|---|---|---|
| `cargo fmt --check` | 通过 | 最终工作区 |
| `cargo check --all-targets --locked` | 通过 | Windows x64 MSVC |
| `cargo test --locked -- --test-threads=1` | 通过 | 347项；其中135项UI相关测试，使用本地模拟上游/临时目录 |
| `cargo clippy --all-targets --locked -- -D warnings` | 通过 | 无新增告警 |
| `cargo build --locked` | 通过 | 更新 `target/debug/ai-relay.exe` |
| modern / regressions `-ValidateOnly` | 通过 | PowerShell解析、C#声明/调用和辅助纯逻辑检查，不等同GUI验收 |
| `smoke-modern-ui.ps1 -ExePath target/debug/ai-relay.exe` | 通过 | 实际96 DPI；四页×浅深×800×680/1000×820 DIP共16组，pending/分段/switch/输入与配置不变检查；PrintWindow离屏 |
| `smoke-ui-regressions.ps1 -BudgetSeconds 120` | 已执行项通过，未完整 | 无失败；38个Edit均无裁剪region，浅深原生矩形边框及四图标像素匹配；20轮主题/切页GDI恒91；已完成的分类/800×560及900×700缩放/缓存返回物理残影比较为0像素差异。960×740和目录状态截图因其他窗口遮挡跳过，`Complete=false` |
| 静态CRT `cargo build --release --locked` | 通过 | 使用进程级编码Rustflags与home/项目路径映射，无持久配置变更 |
| Release x64 PE32+与导入检查 | 通过 | 未发现单独安装CRT/SQLite/OpenSSL导入，不能替代干净Windows验收 |
| `smoke-modern-ui.ps1 -ExePath target/release/ai-relay.exe` | 通过 | 同一16组矩阵，10条回环mock、输入/密码空值掩码/查询pending/switch与配置字节不变；正常退出，无遗留应用实例 |

### 验证中发现与修复

- 初次构建修正新增测试中 Win32 `OBJ_*` 常量的类型转换；初次UI测试发现次要文字在部分浅色表面不足4.5:1，已调深并保留严格断言。
- 隐藏 Edit 夹具在创建时被原生消费 `WS_BORDER`，不能证明真实非客户区；明确建立测试边框并增加真实应用创建/打印像素接线回归，生产创建使用 `WS_EX_CLIENTEDGE`。GDI暖机覆盖全部主题的实际绘制，未放宽稳定性断言。
- 按钮打印测试发现原生绘制会改变调用者DC；新增subclass打印RAII保护，高对比度卸载后明确由打印调用方保护原生DC。
- 第一轮modern矩阵失败于零尺寸只读概览元数据2107的误分类和细箭头采样错位。针对2107单独验证有界只读文本/非TabStop/零面积/不相交；按真实30 DIP箭头区逐像素采样，保留3:1阈值，原PNG离线复算约6.84:1。修正后Debug/Release完整复跑均通过，不把首轮失败改写为成功。

### 产物与本地证据

- 最新可运行：`target/release/ai-relay.exe`，7,838,208字节（约7.48 MiB）；版本保持 `2026.10.18`。
- SHA256：`3FE8377ECA70610804A6D43D2146CEB438DA9851DEDB2E24D548AD025FD5D0F0`。
- Debug modern报告：`target/modern-ui-screenshots/20261018-190604-0e8b7b0f17c543c7b66f2955eaf4315d/results.json`；Release报告：`target/modern-ui-screenshots/20261018-192120-f3c5098267f04f6eb311144290b7f49c/results.json`；两者 `Passed=true`、`PageMatrixComplete=true`、无UI/清理失败。每轮包含124张离屏渲染，人工查看了代表性浅/深概览、统计、设置图。
- 物理绘制报告：`target/review-ui/regressions-20261018-191403-3673fc/report.json`，无失败、正常退出，但存在明确遮挡跳过项。目录时间为UTC，不修改系统时间或DPI。
- 未打包、覆盖 `dist/`、修改标签、提交或建分支；显式target目录的旧静态包未更新。未改用户配置、Startup、系统代理、防火墙或剪贴板。

## 未完成与下一步

- 本批真实GUI仅96 DPI；纯几何/打印测试覆盖144/192 DPI不代表150%/200%实际桌面、多屏切换验收。
- 真实高对比度、IME组合输入中切主题/DPI、读屏、物理键鼠、状态栏悬停和长稳仍待完成；现有系统菜单/聚焦日期及日历保留原生样式。
- 在不被遮挡的桌面补960×740及目录状态的物理绘制项，再验收高DPI/多屏；发行前独立执行打包与干净系统检查。
