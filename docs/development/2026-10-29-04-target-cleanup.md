# 2026-10-29-04：清理开发中间文件并核验保留产物

## 目标与范围

按用户要求回收 `target/` 废弃中间文件占用。仅清理可重建的本项目Debug开发产物，保留可运行程序／符号、Release、第三方依赖缓存、发布包及全部验证证据。源码HEAD为 `314ea76`；前批构建记录和STATUS等未提交文档保持，不修改Rust源码、依赖、构建profile或持久环境，不提交／推送。

## 实际操作

- 先统计目录项与NTFS文件标识去重大小，检查重解析点及疑似应用配置／密钥／数据库／JSONL；未发现重解析点或数据文件，未跟随链接清理。
- 执行 `cargo clean --dry-run --verbose --package ai-relay --profile dev --locked --offline`，预览列出5,417文件、约5.1 GiB，预览输出路径限定在解析后的 `target/debug/`。
- 首轮实际操作前发现Cargo进程，安全检查中止，未移动或删除文件，也未终止进程。随后检查确认Cargo／rustc／Clippy／应用／项目测试空闲，保留rust-analyzer运行；不能由此承诺后续永无并发构建。
- 将当前Debug EXE及匹配PDB移动到Debug清理范围外，执行 `cargo clean --package ai-relay --profile dev --locked --offline`。Cargo成功退出，报告删除5,408文件、约4.7 GiB；随后恢复Debug原路径，未覆盖外部创建的文件，空暂存目录已移除。
- 清理后对Debug／Release／显式target的6个EXE／PDB以及Release、非Debug证据、发布包等共8,145个保留文件校验。清理前后聚合SHA256均为 `D578503B5DD09CDBA9C6903BB6543A65FF47969B5F0D86D19BEE7FF21BF0B09A`；此比较发生在下述主动重建Release之前。
- 临时清理脚本的逐文件“删除集合必须逐项属于预览集合”后置断言未通过。原报告保持 `completed=false`，未将其改写为通过；另行只读复核保留文件聚合哈希及6个主产物均一致、Debug／Release版本检查正常。没有保存逐项预览及删除清单，故不追认该逐项断言通过，也不猜测失败原因。
- 该检查后未进一步清空依赖缓存或验证目录。临时清理脚本和空暂存／Python缓存目录已移除，原失败与独立复核报告保留。

## 空间结果

GiB按1,073,741,824字节换算；最终值包含恢复静态CRT Release后的小幅变化，少量报告／临时脚本增减对下列三位小数无影响。

| 统计口径 | 清理前 | 清理与静态CRT恢复后 | 收尾其他开发活动后 |
|---|---:|---:|---:|
| 全部文件目录项大小 | 9.412 GiB | 4.679 GiB | 5.046 GiB |
| 按文件标识去重的数据大小 | 8.095 GiB | 4.431 GiB | 4.638 GiB |

清理与静态CRT恢复后硬链接去重约回收3.66 GiB；收尾其他开发活动重新生成部分缓存，最后观察净回收约 **3.46 GiB**。这些是数据大小估算，不等同NTFS实际分配簇或整个磁盘空闲变化。Cargo实际删除5,408文件，不把后续重新生成文件或报告导致的净文件数变化混作Cargo删除数量。剩余主要为Debug／Release第三方依赖、Release构建缓存及旧显式target产物，避免为额外回收立即触发全依赖冷编。

## Release变化与最终核验

- 清理开始前观察到 `target/release/ai-relay.exe` 已不同于前批静态CRT构建：7,780,352字节，SHA256 `8923AB77…E2BE74E7`，导入包含 `VCRUNTIME140.dll`。只能确认清理前已变化，未定位其来源，不将其归因于本次Cargo清理。
- 该动态CRT版本在删除操作前后哈希一致。为保留前批交付的静态CRT部署边界，随后用同一源码及单次命令静态CRT／编译路径映射的 `cargo build --release --locked --offline` 重建，约1分钟完成；未重建Debug。
- 最终EXE为 `target/release/ai-relay.exe`，**7,913,984字节**，SHA256 **`4E814BFAF1C33F33678EDB64BFE931F759A6965AEFAA7BE3EFFE32AF30F6BD57`**。
- x64 `0x8664`／PE32+ `0x20B`及导入检查通过：仅系统DLL，无额外VCRUNTIME／MSVCP／CONCRT／SQLite／OpenSSL或动态UCRT导入。Debug／Release的 `--version` 均输出 `AI Relay 2026.10.18`，退出码0、stderr为空，不启动GUI／代理。
- 最终静态CRT产物哈希未重现前批的 `44EAFFB2…9C9AF7C`；未隔离位级差异原因，不宣称二进制可重复或本次成品与前批完全相同。当前入口与哈希以上述本批值为准。
- 收尾观察到另有未提交修改出现在 `src/ui/date_picker.rs`、`src/ui/settings/panel.rs`、`src/ui/settings/tests.rs`，其修改时间晚于本轮Release；未读取／修改／回退这些工作，也未在发现后继续构建或清理。当前Release未验证包含这些后续修改，不再宣称与当前工作区最新源码完全匹配。
- Debug EXE／PDB和旧显式target EXE／PDB在最终核验中仍与清理前相同；`dist`未打包／覆盖。未删除历史截图、成功／失败报告或开发脚本证据，不接触仓库外用户数据。

## 验证与证据

- Cargo包级dev清理：退出码0。
- 单独保留文件校验、Debug／Release版本、最终静态CRT Release构建和PE导入检查：通过上述限定范围。
- 原自动化逐项后置断言：失败保留，不冒称整条清理脚本通过。
- 报告均在Git忽略目录：
  - `target/cleanup-2026-10-29-blocked-1.json`：首轮进程保护中止。
  - `target/cleanup-2026-10-29.json`：Cargo成功、原逐项后置断言失败。
  - `target/cleanup-2026-10-29-verified.json`：独立保留文件复核。
  - `target/cleanup-2026-10-29-final.json`：恢复静态CRT后占用快照。
  - `target/release-build-2026-10-29-after-cleanup.json`：最终Release及版本／导入核验。
  - `target/cleanup-2026-10-29-last-observed.json`：其他开发活动后最后观察占用及新增源码修改边界。
- 不运行fmt／check／全套测试／Clippy或GUI，避免重新生成刚清理的Debug缓存；前批测试为历史结果，不记为本批重新通过。

## 影响与下一步

- 当前Debug／Release可直接运行；下次Debug编译／测试会重建本项目库、测试EXE／PDB及增量缓存，首次耗时可能增加。未关闭增量编译或减小调试信息。
- 日历／菜单／日志专项、高DPI／多屏／真实高对比度／IME／读屏及干净系统限制不因清理解除；最终重建Release仅完成PE／版本核验，未重跑GUI。
- 若后续需要再次自动清理，应先修正并保留逐项预览与删除清单，不能复用本次未通过后置断言的临时脚本。
