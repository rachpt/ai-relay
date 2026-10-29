# 2026-10-29-03：构建并核验最新Release

## 目标与范围

按用户要求，基于最新代码构建Windows x64 Release。起始工作区干净，HEAD为 `314ea7665b19527e1416fa536be1fcd034f8620f`，与本地跟踪的 `origin/master` 一致。本批不修改Rust源码、版本、依赖、Cargo.lock或构建profile，不打包、不覆盖 `dist/`，不提交或推送。

## 实际操作

- 沿用已有静态CRT路线，以单次命令的 `RUSTFLAGS` 传入 `-C target-feature=+crt-static`，并将已解析的用户home／工程绝对路径映射到 `/build/user`／`/build/ai-relay`；不修改持久环境或运行时数据目录。
- 执行 `cargo build --release --locked --offline`，Cargo耗时0.92秒、成功退出。Cargo判断已有产物最新，未为强制重编清理缓存。
- 本地解析PE签名、Machine、PE32+及导入表，检查是否含额外CRT／SQLite／OpenSSL动态库；执行重定向的 `--version` 并检查stdout、stderr与退出码。
- 仅执行版本查询，不打开GUI、不启动代理、不读取真实配置或改变应用／系统设置。
- 按项目约束新增本记录并更新开发索引、STATUS；源码与成品内容未因文档更新改变。

## 产物与验证

| 项目 | 结果 |
|---|---|
| EXE | `target/release/ai-relay.exe` |
| 源码提交 | `314ea76` — `Polish native light and dark UI` |
| 版本 | `2026.10.18`，未提升版本号 |
| 大小 | 7,913,984字节，约7.55 MiB |
| SHA256 | `44EAFFB2A82D54EBFC0E45036174E2C0943929AB49A9AEF28919AF81D9C9AF7C` |
| 静态CRT Release构建 | 通过，使用锁定依赖及离线缓存 |
| PE检查 | x64 `0x8664`、PE32+ `0x20B`，通过 |
| 导入检查 | 仅系统DLL；未发现额外VCRUNTIME／MSVCP／CONCRT／SQLite／OpenSSL运行时导入 |
| `--version` | `AI Relay 2026.10.18`；退出码0，stderr为空 |

成品SHA256与此前已验证的静态CRT Release一致。Cargo新鲜度检查与哈希核验共同确认现有产物匹配最新源码，不仅凭版本字符串或文件修改时间判断。本次无需强制重建。

核验报告位于忽略目录 `target/release-build-2026-10-29.json`，不包含本机绝对路径或应用数据，不纳入提交／发布包。

## 未完成与限制

- 本次仅构建和核验产物，未重跑fmt／全目标check／全套测试／Clippy或GUI；前批448项测试及明暗GUI结果仍为历史验证，不记为本批重新通过。
- 导入检查不替代干净Windows10／11、代码签名、许可证、真实高DPI／多屏／高对比度／IME／读屏及长稳验收。
- 仅 `target/release/ai-relay.exe` 是本批核验入口；显式target目录和 `dist/` 既有产物未更新。

## 下一步

- 使用本批Release继续既有Windows专项验收；若另需便携ZIP，以新BuildLabel打包，不覆盖旧包。
- 后续target清理时发现本入口已被其他构建替换，随后重新恢复静态CRT；当前成品与哈希见[清理记录](2026-10-29-04-target-cleanup.md)，本记录保留当时核验事实。
