# 2026-10-19-03：target 构建缓存空间清理

## 目标

按用户要求优化 `target/` 空间占用，清除可重新生成的本项目开发产物，保留当前可运行程序、符号、第三方依赖缓存及验证证据。不修改应用逻辑、构建profile或用户数据。

## 排查与实际操作

- 清理前递归统计文件，跳过并检查重解析点，不跟随目录链接；本次未发现重解析点或疑似应用配置/密钥/数据库/JSONL数据文件。
- Debug的主要占用来自 `deps/` 内多轮生成的本项目测试EXE/PDB/rlib及 `incremental/`；截图/报告约71 MiB，占比较小，全部保留。
- 执行 `cargo clean --dry-run --verbose --package ai-relay --profile dev --locked --offline`，确认列出的路径均位于解析后的 `target/debug/` 下；未执行不限定包的 `cargo clean`。
- 清理前将 `target/debug/ai-relay.exe` 和 `ai_relay.pdb` 暂存到 `target/` 下不属于Debug清理范围的独立目录，使用移动而非复制；成功清理后恢复原路径并移除空暂存目录。
- 执行 `cargo clean --package ai-relay --profile dev --locked --offline`：Cargo报告删除7,172个文件、约10.6 GiB的文件条目。该命令清理ai-relay本身的开发产物及配套fingerprint/build缓存，不主动清理全部第三方依赖。
- 清理时无应用/Cargo/rustc/clippy构建进程，编辑器rust-analyzer保留运行；未结束用户进程。只检查进程状态，不更改系统设置或启动项。
- 未删除截图、验证报告、Release或显式target目录产物；未改Cargo.toml、Cargo.lock、Rust源码、dist或仓库外数据。

## 空间结果

以下统计不含本批很小的清理报告；GiB按1,073,741,824字节换算。普通递归统计会重复计算Cargo的硬链接，故另按文件标识去重。

| 统计口径 | 清理前 | 清理后 | 减少 |
|---|---:|---:|---:|
| 全部文件条目大小 | 14.987 GiB | 4.437 GiB | 10.550 GiB |
| 按文件标识去重的数据大小 | 13.611 GiB | 4.191 GiB | 9.421 GiB |
| 文件条目数 | 15,682 | 8,510 | 7,172 |

去重数据大小是回收量估算，不等同NTFS实际分配簇或全磁盘空闲差值；未把硬链接重复条目宣称为实际额外占用。保留约4.44 GiB用于后续开发，避免为释放小量空间清空所有依赖缓存或重新编译Release。

## 验证

- 清理前后对以下6个文件计算SHA256，全部一致：Debug/Release/显式 `x86_64-pc-windows-msvc/release` 的 `ai-relay.exe` 和配套 `ai_relay.pdb`。
- 当前Release SHA256保持 `810466067CCE56FDFB0DC6A57E2124F139B996E92DCD84F585EB0C9575B20BB4`。
- 对 `target/modern-ui-screenshots/`、`target/review-ui/` 全部相对路径及文件内容计算聚合哈希，前后一致，历史验证证据未删除或改写。
- 当前Debug和Release分别执行 `--version`，均输出 `AI Relay 2026.10.18`，退出码0；没有启动GUI、代理或写配置。
- 明细保存在忽略目录内的 `target/cleanup-2026-10-19.json`，仅包含项目相对路径/摘要哈希/大小/操作结果，不记录用户数据内容。
- 本批无Rust代码/profile变更，未重跑fmt/check/test/clippy或重新构建，避免立即重建刚清理的缓存。上一批验证结果保留为历史事实，不宣称本批重新执行通过。

## 影响与后续

- 当前Debug/Release仍可直接运行与调试。下次Cargo编译/测试会重建本项目的库、测试EXE/PDB和增量缓存，因此首次耗时可能增加；第三方依赖缓存尽量保留，不需因此重新下载依赖。
- 不擅自关闭增量编译或减小调试信息级别，避免改变调试体验。后续若需要长期更小的开发产物，应单独权衡编译速度与调试信息，而不是把一次清理当作占用永不增长的保证。
- 维护方式补充到 `docs/DEVELOPMENT.md`；现有输入框中间帧P2及其他未完成验收不在本批修复范围。
