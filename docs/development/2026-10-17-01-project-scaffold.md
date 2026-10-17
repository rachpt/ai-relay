# 2026-10-17-01：项目结构与文档骨架

## 目标

依据 Rust + Win32 技术路线搭建后续实施结构，新增 AGENTS 约束和开发记录体系，不改造旧脚本或提前实现业务功能。

## 实际变更

- 新增 `Cargo.toml`，使用 Rust 2024 edition，保持单 package，不增加第三方依赖。
- 新增 `src/main.rs`、`src/lib.rs` 和六个职责模块。
- `app::run` 明确返回尚未实现/非 Windows 不支持的错误，避免把骨架当成已启动代理。
- 新增 `.gitignore`、项目 README 和集成测试约定。
- 新增 `AGENTS.md`，约束需求读取、代码边界、安全、依赖、验证、开发记录及提交行为。
- 新增文档导航、架构说明、开发指南、ADR 0001、状态页和批次模板。
- 保留现有需求文档、JS/CMD/PowerShell、PID 和日志，不移动、删除或执行。

## 验证

| 命令或验收项 | 结果 | 说明 |
|---|---|---|
| `git --no-optional-locks status --short` | 失败 | 当前目录不是 Git 仓库，未主动初始化 |
| `rustc --version` | 失败 | 终端找不到命令 |
| `cargo --version` | 失败 | 终端找不到命令 |
| `cargo fmt --check` | 未执行 | 缺可用 Cargo 工具链 |
| `cargo check --all-targets` | 未执行 | 同上 |
| `cargo test` | 未执行 | 同上，且当前尚无可执行测试 |
| `cargo clippy --all-targets -- -D warnings` | 未执行 | 缺可用 Cargo 工具链 |

## 未完成与限制

- 未生成 `Cargo.lock`；必须待 Cargo 可用后生成，不能手工编造。
- 尚未验证编译、链接、Windows GUI 和发布运行库条件。
- 未实现代理、配置读写、系统集成和可执行集成测试。
- 当前未设置 GUI 子系统；后续有真实界面及命令行输出方案后再配置。
- 旧脚本仍可能含有效密钥；本次没有擅自修改，后续入库或分享前须单独处理。

## 下一步

按 `STATUS.md` 准备工具链、执行首次验证，再实施配置模型和本地模拟上游测试。
