# 日期版本 Release 与标签

日期：2026-10-18

## 目标

用户授权完成后构建Release包、创建日期版本Git标签；随后要求补充常用macOS/Windows系统杂项忽略规则。整合此前未提交的MIT许可和忽略清理，不改变应用功能/验收范围，不推送远端。

## 实际变更

- Cargo版本由0.1.0改为2026.10.18，Cargo自然更新应用Cargo.lock条目，无依赖版本变化。Windows manifest使用2026.10.18.0，EXE数值版本相同，字符串版本为2026.10.18；标签 `v2026.10.18` 指向本批最终提交。
- 根目录MIT LICENSE、Cargo SPDX元数据、导航资源MIT说明及便携包许可整合入日期版本。
- .gitignore移除退休参考脚本专用条目，新增Finder/AppleDouble/Spotlight/Trashes、Explorer缩略图/desktop.ini/回收站、编辑器swap/backup；保留配置、日志、PID、环境秘密和Cargo.lock。
- 新增开发期标准库Python脚本生成第三方notices，按当前Windows normal/build依赖闭包收集134个crate/version、原始许可/版权及捆绑ring/SQLite等声明。未知许可或缺失文本失败，不访问网络、不编造法律条款。manifest/lock快照hash规范化UTF-8 BOM/LF，避免Git换行差异产生伪过期；输出不含本机目录。
- package.ps1检查并附带LICENSE、THIRD-PARTY-NOTICES，保留5文件白名单、不覆盖已有ZIP。静态CRT/路径remap参数通过CARGO_ENCODED_RUSTFLAGS避免空格路径拆分；编译器映射home与项目路径，不影响运行时配置定位。关闭非交互进度显示。
- 更新README、便携说明、开发指南、STATUS与索引。旧批次记录保留当时事实。

## 产物

- ZIP：`dist/ai-relay-2026.10.18-windows-x64-preview.zip`
- 校验：`dist/ai-relay-2026.10.18-windows-x64-preview.zip.sha256`
- ZIP大小：3,594,207 bytes（约3.43 MiB）。EXE：7,801,344 bytes（约7.44 MiB）。
- ZIP SHA256：`bfcf957092c745f9760522c3d43fb3fa451f5087505e9e815130d8b100ea5c91`
- EXE SHA256：`a011d5e073055508fee9b80681f8764db9f130b1d7be32e147843261ab104c35`
- ZIP只含 `ai-relay.exe`、`README.md`、`IMPORTS.txt`、`LICENSE`、`THIRD-PARTY-NOTICES.txt`；二进制/产物不提交Git，既有旧预览包不删除。
- 使用Release优化、Windows x64 MSVC、静态CRT，PE导入无独立安装的VC/SQLite/OpenSSL运行库要求。仍是开发预览，不宣称完成全部Windows验收或已签名。

## 验证

- `cargo fmt --check`、`cargo check --all-targets --locked --offline`：通过。
- `cargo test --locked --offline -- --test-threads=1`：312项通过（160 lib+152集成），图标隔离子进程测试不重复累计。
- `cargo clippy --all-targets --locked --offline -- -D warnings`：通过。
- `python tools/generate-third-party-notices.py --cargo <绝对Cargo> --check`：134项确定性/源文本证据检查通过；LF/CRLF及BOM快照等价验证通过。
- PowerShell语法、许可copy/ZIP白名单验证通过；macOS/Windows/editor18项忽略断言及Cargo.lock/源码未忽略断言通过。
- `tools/package.ps1`：最终静态CRT Release构建及PE检查通过。ZIP CRC、精确白名单、LICENSE/notices/EXE逐字节匹配及SHA256文件验证通过。
- `--version` 输出 `AI Relay 2026.10.18`；EXE数值版本2026.10.18.0。GUI子系统/x64头验证通过。
- 最终EXE检查本机home的普通/正斜线/UTF16形式命中均0，旧Key精确比对无命中；不把该检查当作绝对无泄密证明。
- 实际最终ZIP新目录解压后执行不改源码的 `smoke-ui-regressions.ps1`，45.6秒正常退出：60项真实HWND几何、草稿/选区/缓存、38个Edit无region、浅深图标离屏比对通过，20轮GDI均91，临时目录Explorer只读确认；配置哈希不变，无代理启动/真实模型请求/Startup/防火墙修改。
- **Release物理截图与残影比较未完成**：其他进程窗口遮挡，所有物理捕获/残差比较跳过，Complete=false；16张图仅为明确标记offscreen及裁剪，不以先前Debug物理结果代替。未操纵外国窗口或绕过前台限制。

## 过程中的问题

- 首次调用因显式PATH缺少PowerShell目录失败，改用已解析绝对PowerShell路径。
- 再打包时PowerShell压缩进度在编辑器终端中退出，trace定位到Write-Progress；禁用进度后完成，不伪称此前生成成功。
- 原Release发现356处编译期本机home字符串；加入路径映射重新构建，第一次120秒超时到最终链接，缓存重跑40.97秒完成。旧候选包仅本任务生成、未交付，已用最终私有路径无命中的包替换；旧发行产物不动。
- notices上游原文有两处行尾空格，保留法律文本，使用.gitattributes仅对该生成文件豁免行尾空格检查并固定LF，不改动原始条款。notices最初raw manifest hash在CRLF/LF checkout不一致，规范化后重新生成并打包。最终GUI验证对新EXE进行；旧候选EXE测试不冒充最终产物验证。

## 限制与下一步

真实高DPI/多屏/高对比度/IME/读屏、干净系统、登录自启、长稳、代码签名及完整第三方/工具链法律审查仍未完成。notices保守包含构建期/可能未链接组件，Fiat源码包未附AUTHORS文件的限制如实记录，不编造作者。

标签为本地附注标签，使用统一rachpt身份；没有远端push/GitHub Release上传。再次公开前继续对新增提交做禁联网秘密扫描，不上传私有备份、target和旧预览包。构建ZIP与标签代表当前日期开发预览，而非正式验收结论。
