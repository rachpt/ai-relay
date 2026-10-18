# 2026-10-18-24：本地Release重建

## 目标与实际变更

- 按用户要求编译当前源码的Windows x64 Release，包含默认系统本地时区及统计单元格复制。
- 使用现有发布路线的静态CRT和编译器路径映射，运行 `cargo build --release --locked`；通过进程环境传递 `-C target-feature=+crt-static`，不改项目配置或依赖。
- 产物为 `target/release/ai-relay.exe`，版本保持 `2026.10.18`；7,823,360字节（约7.46 MiB）。
- EXE SHA256：`DED781C009FB27411EF3EC3B5B5D5AE4A18E6A25F993ACD1564E7C12EF803125`。
- 没有改版本、提交、标签、打包或覆盖dist。仅同步状态与本记录索引。

## 验证

- Release构建通过，耗时约1分44秒。
- 复用现有打包脚本的PE导入检查函数，确认x64 PE32+，未发现需要额外安装的CRT、SQLite或OpenSSL动态库依赖；该项不能代替干净Windows验证。
- 针对该Release执行 `tools/smoke-history-ui.ps1 -ExePath target/release/ai-relay.exe`：53条本地mock的历史统计、UTC/本地范围、明细/汇总导航和单元格菜单出现/启禁/取消、刷新失效、查询/导出/管理回归通过。
- 本次未修改Rust代码，不重复执行上一批已通过的330项测试、all-targets check及Clippy。

## 限制与下一步

- 没有启用剪贴板写入验证，不声称实际Ctrl+C或菜单复制端到端验收通过；未执行物理输入、多DPI/多屏、读屏、干净系统或长稳。
- `dist/`及带显式target目录的既有静态发布产物未重建，请使用本记录指定的EXE；若需ZIP另行执行带新BuildLabel的打包。
