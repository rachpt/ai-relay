# 2026-10-20-04：构建并核验最新Release

## 目标与范围

按用户要求构建最新Windows x64 Release。起始工作区干净，源码HEAD为 `7de0eda125265141e01bdd48e81a46f46f0e5909`。本批不改Rust源码、版本、依赖、构建profile或持久环境，不打包、不覆盖 `dist/`，不提交或推送。

## 实际操作

- 沿用 `tools/package.ps1` 的静态CRT与编译路径映射设置，以进程级 `CARGO_ENCODED_RUSTFLAGS` 传递 `-C target-feature=+crt-static` 及动态解析的home／工程路径映射，结束后恢复原环境。
- 执行 `cargo build --release --locked --offline`，使用现有锁定依赖，不联网下载。首次Cargo耗时0.78秒，最终核验复跑0.39秒，均成功；Cargo判定现有产物为最新，无需强制清缓存重编。
- 通过PowerShell检查无并行Cargo／rustc／Clippy进程，且目标Release EXE未运行；不结束用户进程。不启动GUI或代理，不读取真实配置，不修改应用数据或系统设置。
- 仅提取现有打包脚本的 `Imports` 函数检查PE，不执行打包脚本主体；确认x64 PE32+，未发现额外安装的CRT／SQLite／OpenSSL动态库依赖。
- 使用重定向的 `--version` 核验启动及退出，确认输出 `AI Relay 2026.10.18`、退出码0、stderr为空。

## 产物与验证

| 项目 | 结果 |
|---|---|
| EXE | `target/release/ai-relay.exe` |
| 版本 | `2026.10.18`，未提升版本号 |
| 大小 | 7,889,408字节，约7.52 MiB |
| SHA256 | `5278316C8C4B25323765BA81406138910E8478C3B170F28C78AC3BE34CB33846` |
| 静态CRT Release构建 | 通过 |
| x64 PE32+／导入检查 | 通过；不代表干净系统验收 |
| `--version` | 通过；未启动GUI／代理 |

EXE与构建前哈希一致。最新提交仅新增清理记录及文档索引／状态，没有Rust行为变化；Cargo新鲜度检查与哈希共同确认当前产物匹配最新源码，不以版本字符串或mtime推断构建新旧。

临时构建／核验脚本和JSON报告保留在忽略目录 `target/`，报告为 `target/release-build-2026-10-20.json`。首轮 `Start-Process` 封装的版本检查报失败，但重定向stdout内容正确且stderr为空；改用直接持有 `System.Diagnostics.Process` 的方式重新启动并读取退出码，最终确认为0，EXE哈希未变。首次 `tasklist` 检查受Git shell参数转换影响未执行成功，后续改用PowerShell进程检查；不把工具失败记为应用失败，也不隐藏初次检查失败。

## 限制与下一步

- 未改Rust代码，本批不重复执行fmt／all-targets check／全套测试／Clippy或GUI冒烟；前批测试仅为历史结果，不声称本批重新通过。
- 仅使用 `target/release/ai-relay.exe` 作为本批最新产物；显式target目录和 `dist/` 既有文件未更新。
- 菜单纯黑捕获、浅色月历翻月页脚缺失、高DPI／多屏／真实高对比度／读屏、干净系统和长稳验收仍未完成；构建成功不解除这些限制。
- 后续在独占桌面使用本批Release继续已有GUI专项；若需ZIP，应另行使用新BuildLabel打包，避免覆盖旧包。
