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

补充冒烟：`smoke-history-ui.ps1`、`smoke-settings-ui.ps1`、`smoke-cli.ps1`，使用临时目录和本地mock，真实Startup未修改。迁移目前覆盖文件/历史快照及失败回退单测，未对真实软件/标准目录执行破坏性迁移。

## 数据与旧实现

- 2026-10-18 用户为公开准备明确授权移除根目录旧参考 JS/CMD/PowerShell 和关联日志/PID；默认保护其他用户数据，保留原生版 `tools/` 脚本。
- 旧实现含敏感部署配置；禁止恢复到公开仓库或把其密钥复制进测试、文档、示例。仓库外受限恢复备份不属于源码发布内容。
- `.gitignore` 仅避免后续误加入部分本地数据，不会自动脱敏源码，也不能清除版本历史。
- 旧脚本本来就在忽略列表，历史审计未发现这些路径。仍保留忽略规则，防止以后误加入。

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
- 尚无 LICENSE：由维护者选择后添加完整许可证、Cargo license元数据，审查依赖的第三方声明，并更新打包白名单。不要把“可以公开仓库”误写成“已完成开源许可/正式发行审核”。

## 开发记录

每个实质性批次使用 `docs/development/TEMPLATE.md` 写记录，同步更新 `STATUS.md`。重大技术取舍按 `docs/decisions/README.md` 新增 ADR。
