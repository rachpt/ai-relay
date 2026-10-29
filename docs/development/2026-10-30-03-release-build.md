# 2026-10-30-03：提交推送后构建并核验Release

## 目标与范围

按用户明确授权，在独立review、修复及验证完成后commit／push，再执行Release构建；不改版本、依赖、Cargo.lock、profile，不打包、不覆盖dist或显式target历史产物。前置复核与测试见[02记录](2026-10-30-02-precommit-review-release.md)。

## 实际执行

1. 提交 `ec7295f836cfcd0b69be29898376503b692a2918` — `Stabilize charts and refine native UI layout`，包含已审查UI、回归、工具和文档，共25文件。已有两份Release／清理记录保留并一并纳入，不提交target／dist或用户数据。
2. 正常推送 `master` 至当前跟踪的 `origin/master`，未force、未推额外远端或标签；`git ls-remote` 确认远端哈希为该提交，构建开始前工作树干净。
3. 通过忽略目录内的本地辅助脚本，沿用单次进程的静态CRT及编译路径映射，执行 `cargo build --release --locked --offline`。环境仅本次进程生效，结束恢复原RUSTFLAGS／编码flags；不修改持久环境或运行时配置路径。
4. Cargo成功，166.049秒（终端显示约2分45秒）；本次重建若干缓存依赖，未联网更新锁文件或版本。
5. 复用既有package脚本的公开PE读取函数，仅提取函数不执行打包；核对x64／PE32+、无额外CRT／SQLite／OpenSSL／动态UCRT导入。以独立重定向进程执行 `--version`，不打开GUI或真实配置，不启动代理。

## 产物及验证

| 项目 | 实际结果 |
|---|---|
| 构建源提交 | `ec7295f836cfcd0b69be29898376503b692a2918` |
| Release入口 | `target/release/ai-relay.exe` |
| 命令 | `cargo build --release --locked --offline`，静态CRT／编译路径映射 |
| 文件大小 | **7,922,176字节**，约7.555 MiB |
| SHA256 | **`F5D47E6E85D9F20424511DB2A2079EAD9A6B8DDF70AF95515D80A705B734F8AB`** |
| PE架构 | x64 `0x8664`、PE32+ `0x20B`，通过 |
| 导入 | 仅Windows系统DLL；未见VCRUNTIME／MSVCP／CONCRT／SQLite／OpenSSL／动态UCRT依赖 |
| 版本 | `AI Relay 2026.10.18`，未提升版本号 |
| `--version` | 退出码0，stderr为空 |
| Git范围 | 源码提交已推送且远端核对一致；构建未改源码／Cargo文件，后续仅补本记录和状态索引 |

本地辅助脚本及报告保留于忽略的 `target/build-release-2026-10-30-reviewed.ps1`、`target/release-build-2026-10-30-reviewed.json` 和同名前缀version stdout／stderr文件，不纳入发布或提交。后续记录提交只改文档，运行代码仍对应上述构建源；未为文档变化强制重建相同程序。

## 既有验证与限制

- 提交前本轮fmt／全目标check／458项串行（305库＋153集成；3项physical默认ignored）／doc／严格Clippy及Debug概览GUI通过；这些是02记录的提交前结果，不能记为Release版GUI通过。
- 本批Release仅完成构建、PE／依赖／哈希和版本验证，**未执行Release GUI、完整日期／菜单／滚动、高DPI／多屏、真实高对比度／读屏、干净Win10／11或长稳**，既有限制不解除。
- `dist/`、显式target历史版本和版本号未更新，不创建Release标签、ZIP或GitHub发布。二进制哈希变化符合源码改变，但不声称可重复构建或相同版本字符串保证内容相同。

## 下一步

使用本批静态CRT Release继续既有Windows验收，重点连续图表更新及月历宿主边沿；完整系统验收后再决定便携打包及正式发布。
