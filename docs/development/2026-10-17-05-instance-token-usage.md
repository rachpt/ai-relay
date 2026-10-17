# 2026-10-17-05：单实例与 Token 用量观察

## 目标与实际变更

继续单实例/IPC，并按用户新增要求加入 Token 用量统计。需求更新v1.2第8.5节，完整历史报表明确仍是后续范围。

- windows/instance：SID/session Mutex、受限DACL、拒绝远端管道、身份检查、版本/大小/超时限制和命名抢占拒绝。
- app/ui：重复启动显示、不同显式目录拒绝、background、show/status/stop/quit、help/version；start/restart明确拒绝，退出码尚为成功/失败。
- usage：OpenAI Chat/Responses、Anthropic JSON/SSE有界观察、快照去重、未知/截断/超限/错误、128组有界聚合。
- proxy/body：只读观察原数据帧，正常结束/错误/取消恰好记录一次；本地健康和拒绝不计。
- controller/ui：跨代理启停共享store，后台每秒格式化、独立有界通道刷新只读多行统计，保留编辑输入/状态、选区和滚动。
- 新增 parser 与真实代理用量回归、单实例冒烟，扩展GUI冒烟。依赖及信任边界见ADR0004。

## 验证

- `cargo fmt --check`、`cargo check --all-targets --locked`、`cargo test --locked`、`cargo clippy --all-targets --locked -- -D warnings`最终通过。
- 63项测试通过：9单元 +10配置 +9持久化 +15代理 +6代理用量 +14观察器。
- `cargo build --release --locked`通过。
- 整合发现单实例模块重复加载引发dead_code/duplicate_mod；统一由windows导出后严格检查通过，没有压制诊断。
- `smoke-instance.ps1 -Exe target/debug/ai-relay.exe`通过：后台隐藏、重复显示、不同目录拒绝、版本/半帧超时、命名抢占、连续控制与优雅退出。首次漏传必需Exe参数失败，补齐参数后通过。
- `smoke-proxy-ui.ps1`通过：JSON/SSE累计、未知及分组显示、跨启停保留、新进程清零，刷新不覆盖未保存输入和操作状态；实际转发和退出释放监听仍通过。

测试只使用本地mock、临时配置、虚构用量，不使用真实Key或计费API。GUI使用合成消息，不替代实际托盘和显示器验收。

## 限制与下一步

- Token统计不是完整账单：缺失usage、压缩、超限、取消可能不完整；字段完整与截断状态独立。仅当前程序会话，无历史持久化、日期/实际上游报表、导出或费用。
- 单实例未实际验证跨用户/session、提权组合与并发启动压力；同用户恶意进程在信任边界内。
- DPAPI、自启、配置迁移/回滚、完整日志、剩余代理协议、GUI子系统、性能和长稳仍未完成。
- 下一步优先DPAPI、自启和保存应用；Token历史存储需独立设计有界异步记录、恢复与隐私策略。
