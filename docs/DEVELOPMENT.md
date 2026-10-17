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

- 根目录的旧 JS/CMD/PowerShell 保持原位，不自动运行。
- 旧实现含敏感部署配置；禁止把其密钥复制进测试、文档或示例。
- `.gitignore` 仅避免后续误加入部分本地数据，不会自动脱敏源码，也不能清除版本历史。
- 若后续需要初始化 Git，应先按用户授权处理旧文件中的敏感信息，再检查待提交文件。

## 开发记录

每个实质性批次使用 `docs/development/TEMPLATE.md` 写记录，同步更新 `STATUS.md`。重大技术取舍按 `docs/decisions/README.md` 新增 ADR。
