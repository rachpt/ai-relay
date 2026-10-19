# 2026-10-19-04：日志高度拖拽与暗色滚动条跟踪修复

## 目标与需求

按用户要求为日志查看区增加手动拖拽高度，并解决暗色滚动条在拖动/滚动中退回原生白色大条的问题。保留原生只读文本、选区、复制、筛选、后台日志及Windows原生GUI路线。需求细化为v1.7第8.4节，交互边界见[ADR0014](../decisions/0014-scrollbar-tracking-log-resize.md)。

## 实际变更

- 新增 `src/ui/logs/resize.rs`：原生Button支持的细横grip（ID2230），中文caption及Tab焦点，屏幕Y增量换算DIP。默认240、最小120/最大720 DIP；↑/↓每次24、Home恢复默认、拖动Escape恢复起始高度。捕获丢失、隐藏、禁用、主题和主窗口尺寸/DPI变化取消拖动；高度仅在本次运行内保留，不改配置文件。
- `logs/mod.rs`、`ui/mod.rs`、`theme.rs`接线：父窗口验证固定通知后重排，不重写日志正文或选区；窗口缩放不覆盖所选高度。grip位于Edit底边+2 DIP、高12 DIP，下方控件整体跟随，页面extent由实际高度推导，尾部仍距固定底栏16 DIP；大窗口余量只扩展最后只读详情。grip优先分派绘制，不被通用按钮主题替换；高对比度优先系统文字色。
- `scroll_theme.rs`：根因是 `WM_NCLBUTTONDOWN → DefSubclassProc` 在原生modal scrollbar tracking期间画系统色，之前只能在松开返回后补画。改为Dark模式对已识别多行Edit、Report ListView及viewport2400处理NC命中/拇指捕获拖动/轨道分页/端部行滚动与按住repeat，不进入该原生鼠标回路。
- 使用公开API、真实完整条带/拇指/range/page适配；Edit垂直按行、水平按实际原生平均字符单位/像素位置换算；Report ListView按原生行/像素；viewport通过完整i32 scalar消息滚动，不打包16位thumb或伪造trackpos。文字、表格、caret、selection、键盘/滚轮、range/page仍由原生控件维护。浅色/高对比度使用原生交互。
- 捕获/timer/线程局部Escape hook仅在交互期存在，取消后清理；通过保留Rc与交互generation保护同步查询/通知期间取消、销毁或同HWND重开交互。未知控件保持native；不添加Cargo依赖/features或运行时。
- `input_frame.rs`修复前批review的P2：geometry/font/style/DPI/show/WM_SETREDRAW/EM_SETRECT处理期间只排队NC失效，深度守卫阻止原生处理/主题同步重入时提前overlay，不在no-redraw文档坐标阶段直接画边框。打印借用DC单独保护，不覆客户区/改region/styles。
- 更新UI原生回归和既有modern/regressions的默认高度/日志虚拟高度断言。新增 `tools/smoke-logs-interaction-ui.ps1`：私有停止配置、真实worker读取关闭的256行虚构JSONL，通过owned HWND鼠标/键盘驱动高度、按住阶段native位置/capture及物理CopyFromScreen白条采样；拒绝遮挡，不以PrintWindow或松开后截图替代。

## 验证

| 命令或验收项 | 结果 | 说明 |
|---|---|---|
| `cargo fmt --check` | 通过 | 最终工作区 |
| `cargo check --all-targets --locked` | 通过 | Windows x64 MSVC |
| `cargo test --locked ui:: -- --test-threads=1` | 通过 | 158项UI，含17滚动、12输入框专项及grip接线/正文选区/三DPI参数回归 |
| `cargo test --locked -- --test-threads=1` | 通过 | 370项；217库＋153集成，本地mock/临时目录 |
| `cargo clippy --all-targets --locked -- -D warnings` | 通过 | 无告警 |
| `cargo build --locked` | 通过 | 更新Debug |
| 专项/modern `-ValidateOnly` | 通过 | PowerShell/C#声明编译、纯几何/白条分析规则，不等同GUI验收 |
| Debug日志专项 | 功能通过，物理部分通过 | 实际96 DPI，高度范围/拖拽/Home/Escape/切页/主题/resize、原生正文/选区/位置及capture/repeat/取消通过。33张有效按住阶段条带截图，271,524像素采样中亮白像素0；3张后续截图被CursorVisualClass遮挡，Complete=false |
| Debug modern | 通过 | 四页×浅深×两尺寸16组，24设置/24历史子视图TOP/BOTTOM/cache；日志默认240及952最小虚拟高、尾部/右边距、输入/pending/开关回归通过；明确为PrintWindow离屏 |
| Debug regressions | 已执行项通过，未完整 | 5项分类/缩放/缓存返回物理残影差异0，38个Edit无region，SVG/矩形边框通过，20轮GDI恒91；目录状态因Explorer遮挡跳过，Complete=false |
| 静态CRT `cargo build --release --locked` | 通过 | 进程级编码Rustflags＋home/项目路径映射，不改持久配置 |
| Release x64 PE32+导入检查 | 通过 | 无额外CRT/SQLite/OpenSSL导入，不能替代干净系统验收 |
| Release日志专项 | 功能通过，视觉未验收 | 完整交互矩阵、默认恢复、正文/选区/文件哈希、正常退出通过；前台请求拒绝/Explorer遮挡导致有效held物理条带截图0，52条明确跳过/证据不足说明，Complete=false。没有用Debug证据冒充Release视觉通过 |

### 失败与review闭环

- 新滚动测试发现默认字体及比例字体的EM_LINESCROLL横向单位不能直接用 `tmAveCharWidth`。改为遵循原生默认SYSTEM_FONT/字母平均宽度：实测默认/系统字体17字符位移136px、GUI字体102px；半字符目标稳定舍入，重复坐标不来回移动。Edit长行原生水平range在本机上限8192，测试改为严格检查实际native范围夹紧，不人为扩大；纵向90000行和Report超过16位范围回归保留。
- 首次GDI计数包含未暖机的交互路径，补全横纵拖动/打印/repeat/释放/取消暖机后，100周期逐轮严格计数5→5；未改成容差或删除资源断言。
- 独立review发现Report巨大像素两阶段滚动在同步通知取消但仍attached后可能继续第二步；补交互generation/有效capture校验及仍attached取消回归，最终复核未发现新的确定问题。
- 独立review发现grip高对比度聚焦时单用COLOR_HIGHLIGHT/Window底色可能不可读，改为Contrast分支优先系统文字色。
- 输入框专项初次误把 `GetUpdateRect`包围矩形当真实client damage；改用 `GetUpdateRgn ∩ client`，并与同主题/同geometry、不挂子类的原生Edit用EqualRgn逐像素区域对照。12项专项通过，保留提前overlay为0、延后UpdateWindow实际绘制及客户区像素/打印DC保护。
- 首次专项脚本错误依赖普通WNDPROC窗口的WM_NEXTDLGCTL聚焦，改用owned grip正常鼠标down/up并验证GUI线程focus。第二次发现viewport Static在真实bar内没有返回scroll命中，生产子类限定实际可见条带返回HTV/H，native press测试先断言命中，未删验证。
- 首次Release专项观察到capture已建立但首步尚未完成的异步采样竞态；脚本从Post down前计时，严格要求200ms内观测首步（早于400ms repeat），随后另验真实timer继续移动。修正后功能矩阵通过；未把timer移动冒充初始点击或制造WM_TIMER。

## 产物与本地证据

- 最新 `target/debug/ai-relay.exe` 与 `target/release/ai-relay.exe` 均已更新；版本仍为 `2026.10.18`，未打包/覆盖dist、显式target旧包、提交、建分支或修改标签。
- Release为7,858,688字节；SHA256：`2CE506C9CD3A56465B7C4287DDA458572774DF6D73F5E99F9F3DCA4110536911`。
- Debug held专项：`target/logs-interaction-ui/20261019-015444-cd06c618/report.json`（Passed=true，Complete=false）；Release：`target/logs-interaction-ui/20261019-021738-ae29ec3b/report.json`（同标志，但held视觉均跳过）。文件名时间为UTC。
- Debug现代矩阵：`target/modern-ui-screenshots/20261019-015727-00c6afb9d8a14c96829a8f62c71e4b20/results.json`；物理稳定重绘：`target/review-ui/regressions-20261019-020517-e92f43/report.json`。
- 专项配置与关闭JSONL前后SHA256一致，无真实Key/模型计费、Startup/系统代理/防火墙/剪贴板改动；只退出自有子进程，不kill用户程序。临时fixture与截图保留供复核，编译/测试按需再生缓存，未擅自再次清空target。

## 未完成与限制

- 测试输入为HWND局部消息＋独立capture/native位置检查，物理截图来自CopyFromScreen，不等同硬件/SendInput/真实指针按键状态或物理Tab验收；采样不是连续视频，不能承诺每个子帧都不闪烁。
- Release无遮挡拖动绘制仍须复核；普通ListView适配有原生隐藏窗口回归，但本批实际桌面专项只覆盖Edit2200横纵和viewport2400纵条，不能宣传所有ListView物理验收。
- 实际150%/200%、多屏DPI、真实高对比度/读屏、IME组合中切主题及干净系统/长稳仍待完成；几何DPI参数测试不替代实机。
- 高度session-only，不跨重启保存；恢复默认通过Home，不改变日志策略或数据。
