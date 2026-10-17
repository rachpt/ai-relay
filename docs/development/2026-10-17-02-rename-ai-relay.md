# 2026-10-17-02：统一命名为 AI Relay

## 目标

按用户确认，将原生版产品命名为 AI Relay，保持工程、入口及需求中的名称一致，不改变代理功能范围。

## 实际变更

- 产品显示名统一为 `AI Relay`，定位为 AI Provider 流量转发工具。
- Cargo package 改为 `ai-relay`，默认 EXE 对应 `ai-relay.exe`；入口 crate 引用改为 `ai_relay`。
- 更新源码中的产品说明、README、AGENTS 和当前状态页。
- 需求文档更新至 v1.1，标准数据目录名改为 `AIRelay`，更新部署布局和命令行示例，产品名不再列为待确认项。
- 项目根目录仍保留 `ai-proxy`；旧 JS、CMD、PowerShell、日志和 PID 未修改。历史开发记录不追溯改写。
- 当前未实现实际配置存储或自启，因此本次不存在用户数据迁移或启动项修改。
- Windows 稳定应用标识及实际 Mutex、IPC、自启命名仍待系统集成阶段确定，本次未伪造为已经实现。

## 验证

- 对原生源码、当前文档及 Cargo 清单搜索旧显示名、旧 EXE 名和旧 crate 引用；仅保留描述旧环境变量 `AI_PROXY_REWRITE_HOST` 的兼容说明。
- `cargo --version` 与 `rustc --version` 均失败，终端找不到命令。
- `cargo fmt --check`、`cargo check --all-targets`、`cargo test`、`cargo clippy --all-targets -- -D warnings` 未执行，原因是工具链不可用。
- 未手工生成 Cargo.lock，未宣称构建或测试通过。

## 未完成与下一步

代理、GUI 及 Windows 集成功能仍未实现。工具链可用后完成首次验证并由 Cargo 生成锁文件，后续按 STATUS 中的配置与代理原型顺序实施。
