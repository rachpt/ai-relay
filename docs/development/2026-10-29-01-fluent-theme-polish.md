# 2026-10-29-01：明暗主题与 Windows 原生组件精修

## 目标

- 按用户要求由三个子 agent 并行实施主题／导航、原生交互控件、概览／统计图表；主 agent 负责状态栏、圆角、整合、串行验证与记录。
- 参考微软当前 Windows 配色、几何和字体指南，提升浅深模式的一致性、内容层级与状态可辨性；不改变四页结构、配置／代理／统计语义或需求验收范围。
- 延续 Rust + Win32、公开 API、高对比度优先及 Windows 10 降级，不增加依赖、浏览器／WinUI 运行时、联网字体、动画或完整自绘输入。

## 实际变更

- `src/ui/theme.rs`：浅色 `#F3F3F3` 画布／白色表面，深色 `#202020` 画布／`#2B2B2B` 表面；强调色 `#005FB8`／`#60CDFF`，hover 使用中性低强度叠加。现有 Palette 字段不变，字体仍为简体中文推荐的 Microsoft YaHei UI，14 DIP 正文／20 DIP 标题。
- `src/ui/tabs.rs`：主导航中性选中表面＋24×3 DIP 短强调指示条，增加被动 hover，焦点跟随原生 `TCM_GETCURFOCUS`。展示子类从 `theme` 移到 `tabs`，使用独立 Rc／复制的主题快照，不保留应用 State 指针；原生消息先正常分派，再处理展示失效。同步主题／销毁后拒绝过期快照，打印／擦背景完整保护 DC。
- `src/ui/buttons.rs`、`choices.rs`、`control_theme.rs`：次按钮／分段选择改为中性层级；主动作实心、危险动作淡红；按下时文字不再位移。禁用状态不显示 hover／按压反馈，switch 去强调色；Combo 箭头及表头统一启禁与悬停／按下状态，列表条纹更轻。原生值、命中、键盘、列宽／排序／顺序和通知保留。
- `src/ui/rounded.rs`：按钮及闭合 Combo 从6 DIP圆角收敛为4 DIP，几何与真实 HRGN 回归同步。Edit／日期框继续排除裁剪，不改 caret、选区、IME 或完整原生边线。
- `src/ui/overview_dashboard.rs`、`history_chart.rs`：卡片统一约8 DIP圆角、中性表面与轻边线；普通徽标和上传线使用中性色，非零错误徽标保留克制红色。数值／未知／查询中提示保持可读，网格弱于曲线；高对比度保留系统色、虚线和空心标记。120点／120秒、未知不补零、真实字体图例测量、68／88 DIP头部及等价文本不变。
- `src/ui/status_bar.rs`：40 DIP高度不变，左右24 DIP与页面对齐，状态正文去语义染色，仅小图标及淡底使用语义色；完整 STATIC 文本及 tooltip 合约不变。
- 新增27项测试，分布于相关模块及 `tabs/tests.rs`、`overview_dashboard/painting/visual_tests.rs`、`history_chart/paint_tests.rs`、`status_bar/paint_tests.rs`，覆盖状态色／对比度、96／144／192字体参数、原生消息、重入销毁、实际绘制、文字省略、DC恢复与GDI生命周期。
- `tools/smoke-rounded-ui.ps1` 同步4 DIP真实region断言；`smoke-overview-dashboard-ui.ps1` 同步中性徽标／上传图例，新增浅深合成自检，拒绝空白和普通徽标错误着色，保留完整裁图、空图、文字／曲线断言及身份／清理边界。
- README、UI设计、STATUS及索引同步。展示取舍见 [ADR0019](../decisions/0019-fluent-presentation-tokens.md)，仅取代 ADR0013 的6 DIP页内圆角值；原生输入和高对比度路线不变。

## 验证

所有 Cargo／原生 HWND 测试与 GUI 依次执行，不并发抢占焦点；仅普通构建与 GUI 工具有过并行，未在 GUI 期间运行原生测试。

| 命令或验收项 | 结果 | 说明 |
|---|---|---|
| `cargo fmt --check` | 通过 | 整合时状态栏格式差异已用 rustfmt 修正，最终检查通过 |
| `cargo check --all-targets --locked --offline` | 通过 | Windows MSVC 全目标 |
| `cargo test --locked --offline -- --test-threads=1` | 通过 | 最终448项通过：295库＋153集成；2项需要无遮挡交互桌面的物理测试默认 ignored，doc 0项 |
| `cargo clippy --all-targets --locked --offline -- -D warnings` | 通过 | 首轮主Tab的 collapsible_match 已按等价 guard 修正，未抑制 lint |
| `cargo build --locked --offline` | 通过 | Debug更新 |
| `cargo build --release --locked --offline` | 通过 | 先验证普通Release；最终另以静态CRT和编译路径映射重建，见下文 |
| 三个UI工具 `-ValidateOnly` | 通过 | modern、overview、rounded；仅声明／辅助自检，不当作GUI验收 |
| Debug modern GUI | 通过限定范围 | 96 DPI，206.72秒，16组四页／浅深／两尺寸＋24设置分类＋24统计子视图TOP／BOTTOM／缓存返回，168张OFFSCREEN图，34项局部对比度证据，15个Edit草稿／选区／样式保留，配置字节不变、正常退出 |
| 普通Release modern GUI | 通过限定范围 | 同一矩阵202.59秒，无布局／清理失败；随后静态CRT成品再次验证，不以普通Release替代最终成品 |
| 最终静态CRT Release modern GUI | 通过限定范围 | 同一16／24／24矩阵204.37秒，168张OFFSCREEN图、34项局部对比度证据；配置字节不变、正常退出 |
| Debug overview GUI | 首轮旧视觉契约失败，更新断言后通过限定范围 | 首轮强制“每张卡片彩色徽标”失败；新设计本就为中性徽标，补合成拒绝检查后通过。54.36秒，21个本地mock请求含7个HTTP400，字节／速率、隐藏冻结／返回、启停／真实零、23张图及GDI短测通过；6轮计数88／86／88／88／86／88，最终88 |
| 独立只读review | 未发现确定新增P1／P2 | 两会话交叉审查主题／导航、控件／图表；另审查状态栏／圆角及工具断言。指出的4 DIP文案遗漏和状态栏实际绘制覆盖已补齐；静态结论不代替测试 |
| 编辑器 diagnostics、`git diff --check` | 通过 | 无错误／警告及差异空白问题；Git仅提示既有CRLF将规范为LF |

### 失败及修正边界

- 首轮 UI 回归227通过、2 ignored，圆角原生测试仍期待旧192／144 DPI直径24／18，失败；同步为16／12后4项圆角专项通过，最终全量通过。不是修改生产功能来回避断言。
- 概览工具首轮在普通卡片徽标的 Colored 阈值失败，原失败报告保留；此次明确改变了外观契约，因此改为“中性底面＋可读图标，非零错误才红色”，并为错误着色／无图标加入拒绝自检。未删除曲线、文字、完整裁图或空图保护。
- 本轮没有执行完整圆角旧工具 GUI、日期／菜单专项、真实系统高对比度切换或两项 ignored 物理测试。四页工具包含日期闭合外观，不代表弹出月历或按住滚动专项已通过。

### 成品与证据

- 最终 `target/release/ai-relay.exe`：静态CRT、编译路径映射，**7,913,984字节**；SHA256 `44EAFFB2A82D54EBFC0E45036174E2C0943929AB49A9AEF28919AF81D9C9AF7C`。
- x64 PE32+检查通过，导入仅系统DLL，未发现额外 VCRUNTIME／MSVCP／SQLite／OpenSSL运行时；`--version`输出 `AI Relay 2026.10.18`，退出码0。版本号、标签和 `dist` 未改；显式target下历史产物仍旧。
- 最终全量日志：`target/fluent-theme-tests.txt`。
- Debug modern：`target/modern-ui-screenshots/20261028-163709-1f09fd2381d34078b7297f9144cb6261/`。
- 最终静态CRT Release modern：`target/modern-ui-screenshots/20261028-172450-686b358e4b534bc8a7e5d3571df26232/`。
- 概览成功：`target/overview-dashboard-screenshots/20261028-170516-b9bb50f86e6749209ec7d8119e43a982/`；旧契约失败：同目录下 `20261028-164128-5264f7dc5a424cccb43f47f51b356536/`。
- 证据目录名由工具UTC时间生成，故为20261028；本批按本地2026-10-29记录。均为忽略的本地开发证据，不加入发布包。
- 已人工抽查浅深概览／完整图表、统计、设置／外观、日志和状态栏图片；不是逐图逐字验收。所有截图为 PrintWindow OFFSCREEN，不能声称物理屏幕无遮挡、无残影或输入设备验收。

## 未完成与限制

- 实际150%／200%、多屏DPI、Windows10／11干净系统、真实高对比度、IME／读屏及物理键鼠未执行。字体参数测试不等同真实高DPI。
- 概览最小800×560外框下完整dashboard高于viewport，两主题各保留限制，不将局部截图当完整图；默认和放大尺寸完整裁图通过。图表专项 `Complete=false`。
- 四页矩阵不点击会触发磁盘查询的统计管理入口，不执行导出、剪贴板、真实目录迁移、自启或系统设置变更。无真实上游、Key、计费请求。
- 以前日期启动超时、菜单纯黑捕获、日志高度／按住滚动及专项Release限制保留；本轮modern通过不宣称已隔离这些旧失败。
- 固定强调色和Win32展示不是自动系统强调色、Mica／Acrylic或完整WinUI。系统菜单、弹出控件及高对比度的既有降级边界保留。

## 下一步

- 在无遮挡桌面以最终静态CRT Release补真实高DPI／多屏／高对比度、键鼠／IME和读屏；检查长标签、禁用及焦点。
- 继续隔离既有日期／菜单／日志专项限制，不以此次常规四页通过替换旧失败。
- 版本与便携包是否更新由用户另行决定；本批不提交、不建分支、不打包覆盖 `dist`。
