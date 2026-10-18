# 布局缓存、重绘事务、原生输入边框与 SVG 导航

日期：2026-10-18

## 目标

处理待提交重构的 review 发现：统计子视图坐标错误、日志首次预布局高度错误、缩放/分类切换残影、Edit 裁剪圆角与边框不一致；改善导航图标，补目录操作可用性和反馈。保留已有未提交模块拆分、原生控件、安全和线程边界。

## 实际变更

### 布局与重绘

- `src/ui/mod.rs` 的首次预布局按每页自身虚拟高度计算，日志至少848 DIP，其他页至少820 DIP；窗口树创建前不缓存空句柄的布局。
- 历史 VIEW/TABLE_VIEW/日期快捷范围/重置和实际子视图变化重新建立文档坐标，再统一应用当前滚动偏移；隐藏页缓存失效，不对已定位控件重复减偏移。
- 批量几何/显隐不立即绘制；最终 arrange/refresh/滚动排队 `RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_FRAME`，覆盖程序化 resize、分类和页面切换，不依赖 WM_EXITSIZEMOVE，也不在 State 借用期间同步 paint。
- 缓存 refresh 合法钳制滚动位置时同步移动可见控件。历史表格刷新仅对原本可见表格使用 WM_SETREDRAW，避免查询完成后意外显示隐藏 ListView；包括表头/边框的最终异步失效。
- 新增真实 State/HWND 回归，主窗口测试放到 `src/ui/tests.rs`，避免继续增大入口文件。

### 输入框与目录

- `rounded.rs` 不再对任何 Edit 安装圆角 region；单行、多行和密码输入保留完整原生矩形边框。仅清理本模块拥有的旧 region，不删除外部 region。按钮、闭合下拉与日期框继续6 DIP圆角，高对比度保持系统回退。
- 原生回归保留并扩展文本、选区、边距、样式、焦点、密码mask、字符上限和600轮 GDI 检查。
- 概览目录按钮使用已锁定、已建立scope且后台空闲的条件，命令执行再次检查并显示“正在打开”；后台仍返回真实 Shell 接受/失败结果，不宣称 Explorer 一定显示。

### SVG 导航资源

- 新增原创仪表盘、柱图、文档、齿轮 SVG；离线生成六档 alpha 遮罩，运行时内嵌并按主题着色。
- 使用公开 AlphaBlend，无新增 Cargo 依赖、浏览器或 SVG 运行时；缓存最多八对 DC/位图，退化为文字时保留原生Tab语义。
- 不是任意 SVG 渲染器，离线脚本只支持受限子集；见 ADR0010 和 `assets/navigation/README.md`。

### 自动化与文档

- compact smoke 按底部几何识别状态栏，单独验证品牌/菜单，适应头部新增 STATIC；rounded smoke 对 Edit 验证无region，其他支持控件仍验证圆角。
- 新增 `tools/smoke-ui-regressions.ps1`：只复用已有脚本的 C# 类型声明，不执行旧脚本 body；隔离配置、固定子进程归属、现有实例/mutex拒绝策略、120秒预算，覆盖坐标、残影物理差异、输入状态、图标像素和GDI。
- 更新 README、UI设计、STATUS、开发索引和 ADR索引；历史批次记录不重写。

## 验证与结果

工具链：Windows x64 MSVC，Rust/Cargo1.99.1；命令使用已发现工具链绝对路径及明确 PATH，不硬编码到源码或文档示例。

- `cargo fmt --check`：通过。
- `cargo check --all-targets --locked --offline`：通过。
- `cargo test --locked --offline -- --test-threads=1`：312项通过（160 lib + 152集成；图标GDI子进程是其中一项的隔离实现，不额外累加）。
- `cargo clippy --all-targets --locked --offline -- -D warnings`：通过。
- `cargo build --locked --offline`：通过，当前二进制为 `target/debug/ai-relay.exe`；未重打发行包或替换旧 `ai-relay-next.exe`。
- `powershell.exe -NoProfile -File tools/generate-navigation-icons.ps1 -ValidateOnly -SelfTest`：四组生成资源逐字节复核和9类拒绝用例通过。
- 新regressions、compact、rounded脚本 `-ValidateOnly`：声明/编译检查通过；compact/rounded旧脚本完整GUI body本批未执行。
- 新regressions最终GUI运行47.46秒，Complete=true只表示其96DPI短矩阵完成：
  - 日志首访96 DIP；历史/会话真实HWND坐标、滚动后切换及返回一致，草稿/选区保留。
  - 38个Edit均无裁剪region；浅深物理边缘采样连续，文本/样式/边距/密码状态不变；其他控件圆角通过。
  - 深色分类返回、日志960×740→800×560→900×700及缓存返回，物理截图与额外完整重绘后均0像素残差。
  - 四图标浅深8个采样逐像素匹配资源（色差容差2）；原生item边界内布局完整。
  - 预热后20轮主题/页面/缩放，GDI均91。
  - 目录按钮真实点击后状态如实返回，并只读确认临时目录Explorer窗口。

过程中的失败如实保留：首次all-targets check发现新增测试私有常量和缺少import，已修复；早期图标测试错误地使用已删除DC的GetObjectType判断释放，独立probe确认该方法无效，改用隔离进程真实GDI计数、析构审计和原生释放返回值；严格Clippy指出两个测试chunks迭代风格，已修复。GUI harness早期误以WS_BORDER/非客户区inset足以证明边框，已改查实际边缘像素，早期中止不算产品失败。

## 剩余限制

- 本批实际GUI为96DPI，未修改系统DPI；150%/200%/多屏、真实高对比度、物理键盘/IME、读屏、长稳和干净系统仍未执行。纯逻辑/程序化DPI测试不替代它们。
- Edit明确是原生矩形，不宣称圆角。SVG明确是设计期源，不支持运行时载入任意SVG。
- 目录操作仍使用既有后台Shell调用，没有扩大为Shell worker架构重构；慢Shell扩展可能延迟控制线程，未复现也未宣称消除。
- 网络测试仅本地mock/虚构凭据，无计费模型。GUI不启动代理、不保存配置，不改Startup/防火墙/系统代理；检测已有实例即停止。所有测试子进程正常退出，临时配置哈希未变，临时Explorer窗口不擅自关闭。
- 证据在 `target/review-ui/REGRESSIONS.md` 和 `regressions-20261018-060851-af1552/`；属于可丢弃验证产物，不纳入发行包，也不是完整验收证明。
