# 开发与验证指南

## 环境准备

目标平台为 Windows 10/11 x64。建议安装：

1. Rust stable，支持 2024 edition（Rust 1.85 或更高）。
2. `x86_64-pc-windows-msvc` 目标。
3. Visual Studio Build Tools 的 C++ 构建工具与 Windows SDK。
4. `rustfmt` 和 `clippy` 组件。

2026-10-17 已验证 Rust/Cargo 1.99.1、MSVC 目标、资源编译和 Release 链接可用。当前编辑器 shell 的 PATH 未包含 Cargo；可重启终端刷新环境，或从实际安装目录调用 `cargo.exe`。未将本机工具链绝对路径写入项目配置。

## 标准验证

在项目根目录执行：

```text
cargo fmt --check
cargo check --all-targets
cargo test
cargo clippy --all-targets -- -D warnings
```

- 使用 `cargo fmt` 修复格式后重新检查。
- 首次 Cargo 运行生成 `Cargo.lock`，将其纳入版本控制；不手写锁文件。
- 依赖引入时记录用途、版本兼容和分发影响；不默认加入不需要的 feature。
- 纯逻辑验证通过不能替代 Windows GUI、IPC、自启和便携测试。
- 自动测试已覆盖配置、原子保存、代理、usage、单实例及日志；完整系统验收仍须Windows手工验证，详见本批开发记录。

## target 空间管理

- `target/` 是构建产物，不是应用数据目录。先统计后清理，区分第三方依赖、项目测试EXE/PDB、增量缓存、可运行产物及验证截图；统计NTFS硬链接时不要重复计算为实际独占空间。
- 清理前确认没有Cargo/rustc/clippy或本项目测试正在执行，不主动结束用户进程。遇到配置、密钥、数据库、JSONL或重解析点应单独核查，不把整个目录一律视为可丢弃数据。
- 项目级开发缓存清理可先预览：

  ```text
  cargo clean --dry-run --verbose --package ai-relay --profile dev --locked --offline
  ```

- 不带 `--dry-run` 会同时删除本项目Debug EXE/PDB、库、测试和相关fingerprint/incremental。若需保留当前可运行版本，先将Debug EXE及匹配PDB移到清理范围外，操作成功后恢复并比对SHA256；不要直接清空整个 `target/`。包级清理仍会增加下一次项目编译/测试的耗时，但比全部依赖冷重建更节省开发时间。
- 默认保留Release、发布包、最新及历史验证证据；不把清理后未经重跑的测试宣传为再次通过。不为一次空间清理擅改 `profile.dev` / `profile.test` 或禁用增量编译。
- 本次清理明细及硬链接去重口径见[2026-10-19清理记录](development/2026-10-19-03-target-space-cleanup.md)。

## 当前程序入口

```text
cargo run
```

当前入口打开原生设置页，可保存配置及启动/停止反向代理。磁盘操作和 Tokio runtime 在后台线程执行；已有有效回环配置可自动启动。详见 README 操作说明。当前已接入单实例、四Tab、DPAPI、自启和多维历史。GUI子系统不弹控制台，CLI可附着父控制台及保留重定向；自启仅显式确认后更改。

## 发布验证

```text
cargo build --release --target x86_64-pc-windows-msvc
```

目前只设置 Release 的 thin LTO、单 codegen unit 及符号剥离作为初始优化，不构成体积或性能保证。

首次正式发布前：

- 验证运行库与 TLS 依赖，确认无需用户额外安装运行时。
- 图标、应用清单、GUI子系统和CLI重定向已接入，继续进行干净系统验收。
- 在干净的 Windows 环境测试免安装运行、中文路径及权限失败。
- 按需求执行目录迁移、单实例、自启、托盘、DPI、SSE、长稳及性能验收。
- ZIP 中不包含旧脚本、配置、密钥、日志、PID 或调试符号。

`tools/package.ps1` 可生成静态CRT x64便携预览ZIP，PE导入检查只发现系统DLL；不是完成签名/许可证/干净系统验收的正式版。Release EXE 为开发预览。`tools/smoke-proxy-ui.ps1` 使用临时配置和本地 mock 验证保存、启停、转发、重新加载与退出清理。`tools/smoke-native-host.ps1` 提供有限时 GUI 自动冒烟；`tools/generate-icon.ps1` 从 SVG 生成 ICO，可用 `-ValidateOnly` 检查现有图标。`tools/smoke-logs-ui.ps1 -RequireAccessLog` 验证实时/文件日志及安全清理。真实托盘菜单退出、Explorer 重启和多屏 DPI 仍需手工验收。

日志高度/暗色按住滚动专项：`powershell.exe -NoProfile -NonInteractive -File tools/smoke-logs-interaction-ui.ps1 -ValidateOnly` 先检查声明/纯逻辑；随后不带该开关在独占桌面执行，默认使用Debug，显式 `-ExePath` 必须是绝对路径。脚本使用私有停止配置/关闭的虚构JSONL文件，不保存配置或修改剪贴板/Startup；按住未释放阶段检查capture、native位置和实际CopyFromScreen条带，不以PrintWindow代替。遮挡或前台请求拒绝使Complete=false，即使功能Passed=true也不能宣称视觉全验收。保留临时fixture和报告供复核。缩放以异步请求、完整外框和活动日志后代布局的有界观察确认完成（含仅宽度变化）；进程原始handle/PID/start/实际EXE始终严格核验，仅空托管Path有界等待。不要与原生HWND测试或其他GUI冒烟并行执行；整条测试命令超时应分段补齐，不把未执行项记为通过。最新修复及一次早轮Debug白条失败的保留边界见[08记录](development/2026-10-19-08-review-interaction-fixes.md)。

概览图表定向验证：`tools/smoke-overview-dashboard-ui.ps1 -ValidateOnly` 先检查声明及辅助自检，随后以 `-ExePath target/debug/ai-relay.exe` 或Release执行。工具使用真实回环mock/私有配置，检查计数、字节、速率和隐藏/启停语义。最小/默认/放大窗口按实际工作区测量；完整dashboard装不进viewport时仅记录分段图/几何及限制，能完整放入时从整窗PrintWindow作严格完整矩形1:1裁图。图像均为OFFSCREEN，不代表物理可见性。视觉检查只使用固定整行header／legend／status及88 DIP plot区域，不从最新WM_GETTEXT速率推算由历史峰值决定的显示单位或第二图例位置；图例标记按主题精确色和短线／点／空心方形几何检测，排除标记后检查中性正文墨迹。ValidateOnly覆盖96组合成正例与352组缺标记／ClearType彩边／marker-only无正文等反例，不依赖旧截图；完整矩阵仅在ValidateOnly执行，普通GUI不重复矩阵但保留基础自检及全部真实截图guard，报告明确未执行矩阵。启动阶段输出编译／基础自检／矩阵耗时；不读取跨进程HFONT，不声称OCR、每个标签／单位／数值或实际文字fit验证。真实字体／精确布局由独立进程内回归覆盖，工具不替代实际高DPI、系统高对比度或物理刷新验收。

普通打开自动启动专项：先执行 `powershell.exe -NoProfile -File tools/smoke-auto-start-ui.ps1 -ValidateOnly` 检查声明，再以 `-ExePath target/debug/ai-relay.exe` 或Release运行。工具只以普通 `Launch --config-dir <绝对私有临时目录>` 打开，7组/10个fixture覆盖缺省与显式开关、首次无配置、无效/空上游、占用端口及LAN保护；自动监听不靠CLI/GUI Start触发。只读检查设置开关、健康接口与固定回环mock GET，配置哈希在启动前/过程中/正常退出后比较。已有实例时拒绝执行；原始进程handle/EXE/HWND树固定，退出只走自有窗口正常路由，不强杀，异常保留fixture。未改真实Startup/剪贴板/系统代理/防火墙；有限观察窗的无重试、定向HWND状态不代表物理输入或系统完整验收。

深色标题菜单专项：先运行 `powershell.exe -NoProfile -NonInteractive -File tools/smoke-dark-menu-ui.ps1 -ValidateOnly`，再以默认Debug或 `-ExePath target/release/ai-relay.exe` 执行。工具只用私有停止配置，验证浅→深→浅、F10/Alt+M/Alt+Space、四子菜单／助记键／方向键切页／Escape及原生命令和启禁／radio，保存21张OFFSCREEN PrintWindow菜单图与像素采样，内部纯RGB空图统一拒绝；配置哈希不变、正常退出、不强杀、不改剪贴板／Startup／系统主题。实际高对比度／高DPI／读屏／物理键鼠仍需验收，Complete=false。不要与其他GUI冒烟或大量原生HWND测试并行执行，以免窗口激活／焦点干扰；旧 `smoke-title-menu-ui.ps1` 包含已过时的caption hamburger／布局断言，不能替代本专项。

深色日期专项：先运行 `powershell.exe -NoProfile -NonInteractive -File tools/smoke-dark-date-picker-ui.ps1 -ValidateOnly`，再用默认Debug或 `-ExePath target/release/ai-relay.exe` 执行，不与其他GUI测试并行。私有停止配置／无网络，浅→深→浅遍历日志与统计四日期框，检查闭合／聚焦／展开／翻月／选日／清空、原生日期值与最小尺寸；84张OFFSCREEN及最多12张有遮挡保护的物理图，正文及月历标题／日期网格／今天页脚逐区检查前景；ValidateOnly拒绝纯色和逐区缺失合成图，8轮主题＋弹窗GDI及正常退出／配置不变。遮挡会跳过物理图，不当作通过；实际高DPI／高对比度／读屏保持待验收。旧 `smoke-date-picker-ui.ps1` 有过时的父窗口／设置分类及允许白色弹窗断言，不替代本专项。

日期翻月专项可额外使用 `tools/smoke-dark-date-picker-ui.ps1 -NavigationOnly -ExePath target/release/ai-relay.exe`：私有暗色停止配置，四日期框各自新开popup，在PageDown后只做实际屏幕采样再执行稳定内容／选日检查；不在观察段打印／强制重绘。0ms只证明暗色栅格与亮白上限，50/350/1000ms还要求标题／星期／日期／今天页脚完整；报告记录实际开始／结束时间，计划时间不冒称精确采样时间。遮挡或前台请求拒绝使必需采样失败，`NavigationOnly`不替代完整浅深／状态／GDI矩阵；历史失败保留。原生物理回归亦须显式串行执行，不与其他GUI测试并行。

提交前复核中菜单纯黑捕获／浅色月历翻月页脚缺失曾被严格检查拒绝，最新Release日期专项未通过；不以早轮成功或普通测试代替，当前限制及失败证据见[提交前review记录](development/2026-10-20-02-precommit-review.md)。

补充冒烟：`smoke-history-ui.ps1`、`smoke-settings-ui.ps1`、`smoke-cli.ps1`，使用临时目录和本地mock，真实Startup未修改。迁移目前覆盖文件/历史快照及失败回退单测，未对真实软件/标准目录执行破坏性迁移。

## 数据与旧实现

- 2026-10-18 用户为公开准备明确授权移除根目录旧参考 JS/CMD/PowerShell 和关联日志/PID；默认保护其他用户数据，保留原生版 `tools/` 脚本。
- 旧实现含敏感部署配置；禁止恢复到公开仓库或把其密钥复制进测试、文档、示例。仓库外受限恢复备份不属于源码发布内容。
- `.gitignore` 仅避免后续误加入部分本地数据，不会自动脱敏源码，也不能清除版本历史。
- 历史审计未发现旧参考脚本路径；文件及专用忽略条目已移除，仅保留配置、日志、PID和环境秘密等通用忽略规则。

### 公开前的历史检查

本次本地扫描使用 `detect-secrets==1.5.0`，必须指定 `--no-verify`，禁止为确认候选凭据有效性而调用 Provider API。工具安装在忽略的临时目录，不是应用依赖。当前树检查示例：

```text
detect-secrets scan --no-verify
```

该命令只检查当前 Git 跟踪文件，**不能代替历史检查**。本批另以 `git rev-list --all` / `git ls-tree -r` 枚举每个提交，并按 blob 去重、读取原始内容，扫描所有历史文本版本；使用相同禁联网策略。另扫描提交/标签说明及路径，核对旧密钥/上游值是否存在，检查个人目录、私钥块、常见令牌和异常地址。报告只输出规则、路径和行号，不打印命中内容。

- 当前/历史候选须人工分类；本批命中的是虚构测试凭据和 Windows Common Controls 清单常量，不能删除安全回归测试来获得零告警，也不应全局屏蔽测试目录。
- 检查全部 refs、旧提交对象和 reflog，清理可恢复的旧身份/敏感对象时先做仓库外受限备份；备份及 filter-repo 审计映射都不发布。
- 全部 author/committer 应为 `rachpt <rachpt@126.com>`；本地 user.name/email 设为同一身份，不改全局配置。历史重写使哈希变化；任何旧克隆或外部副本仍独立存在，不由本地清理撤回。
- 发布前再扫描新提交并启用 GitHub secret scanning/push protection（适用时）；本项目目前没有自动秘密扫描 CI，不把一次检查当作长期保证。
- 项目按用户选择采用MIT，根目录LICENSE及Cargo `license = "MIT"` 一致。发布脚本在构建前检查LICENSE存在，将原文加入ZIP白名单；不得据此声称第三方依赖声明/许可证或正式发行审核已完成。第三方notices已从当前Windows目标锁定依赖的原始许可/版权文本收集，包含构建期依赖的保守清单并加入发行包；完整法律及SDK/CRT/工具链许可审核仍需单独完成。

### 日期版本与第三方声明

当前版本为 `2026.10.18`，对应标签 `v2026.10.18`；日期号不改变开发预览的验收状态。修改Cargo版本后由Cargo自然更新应用锁文件，不手工编造锁文件。

```text
python tools/generate-third-party-notices.py --cargo <Cargo绝对路径>
python tools/generate-third-party-notices.py --cargo <Cargo绝对路径> --check
```

生成器只在开发机运行，用Python标准库及离线锁定Cargo metadata读取本地registry源码，按Windows normal/build依赖闭包收集原始许可和署名，未知/缺失许可失败，不下载或编造法律文本。输出不含本机路径；manifest/lock快照SHA按UTF-8去BOM、LF规范化，跨Git换行设置可复核。Cargo依赖变化后必须重新生成并审查。

发布脚本检查LICENSE与THIRD-PARTY-NOTICES存在，ZIP白名单仅含EXE、README、IMPORTS、LICENSE和第三方声明。静态CRT编译通过编码的RUSTFLAGS传入当前home/项目路径映射，避免编译器路径泄露本机目录；不会改变运行时数据目录定位。禁用非交互进度条以兼容编辑器终端。实际Release验证须针对解压后的二进制；物理截图被其他窗口遮挡时标为跳过，不能用offscreen绘制冒充实际屏幕无残影。

## 开发记录

每个实质性批次使用 `docs/development/TEMPLATE.md` 写记录，同步更新 `STATUS.md`。重大技术取舍按 `docs/decisions/README.md` 新增 ADR。
