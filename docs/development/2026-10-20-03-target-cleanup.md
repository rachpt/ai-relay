# 2026-10-20-03：清理可重建开发产物并核验最新Release

## 目标与范围

按用户要求清理 `target/` 废弃内容，并确认Release是否包含最新提交。本批不改源码、构建profile、依赖、应用数据或系统设置，不再次提交。

## 实际操作

- 确认工作区起始干净，HEAD为 `5ec6c367214c0cf54354ae7dbc710d1f97faab9c`；无运行中的Cargo／rustc／Clippy／应用／项目测试进程，不结束用户进程。
- 文件工具把Git忽略产物显示为空，初期判断不可靠；终端原生目录检查确认当前Debug／Release和显式target旧Release均存在。初次python调用命中WindowsApps启动别名，未成功执行，也未删除或构建任何内容；改用PowerShell／Cargo核验，不修改用户工具链或PATH。
- 使用与前批一致的进程级静态CRT flags及home／工程编译路径映射执行 `cargo build --release --locked --offline`：0.51秒完成，EXE哈希与提交前记录一致，确认 `target/release/ai-relay.exe`对应当前源码；不是仅凭版本字符串或mtime推断。
- 先递归统计，跳过并报告重解析点；本次未发现重解析点或配置／密钥／SQLite／JSONL等疑似应用数据。通过公开文件标识按NTFS硬链接去重，不把同一文件的多个目录项当作独占磁盘空间。
- Cargo dry-run确认清理路径均在已解析绝对 `target/debug/` 内。临时移动当前Debug EXE和匹配 `ai_relay.pdb`到清理范围外，执行 `cargo clean --package ai-relay --profile dev --locked --offline`，无论成功与否均恢复；未覆盖运行中或外部创建的文件。
- 仅清理ai-relay包的开发测试／库／增量／build／fingerprint等可重建产物，尽量保留第三方依赖缓存。当前Debug／Release／旧显式target的EXE及PDB均比对SHA256保持不变，历史截图及成功／失败报告全部保留。
- 清理临时脚本／暂存目录已移除。明细保留于忽略目录 `target/cleanup-2026-10-20.json`，版本输出保留在同目录两份小文本文件；未提交运行产物。

## 空间结果

GiB为1,073,741,824字节；统计不含临时保全目录及最终很小的报告／版本输出。

| 口径 | 清理前 | 清理后 | 减少 |
|---|---:|---:|---:|
| 文件目录项大小 | 9.190 GiB | 4.497 GiB | 4.693 GiB |
| 按文件标识去重 | 8.013 GiB | 4.251 GiB | 3.762 GiB |
| 文件数 | 15,479 | 10,071 | 5,408 |

文件标识去重约减少4,039,332,385字节，是硬链接去重数据量估计，不等同NTFS分配簇／全盘空闲变化。`du`清理后约4.3 GiB，与目录项求和口径不同，不混为精确物理回收量。

## 验证与产物

- 清理前／后6个EXE／PDB SHA256全部一致，当前 `target/release/ai-relay.exe`为7,889,408字节，SHA256 `5278316C8C4B25323765BA81406138910E8478C3B170F28C78AC3BE34CB33846`。
- 当前Release路径：`target/release/ai-relay.exe`，最新提交源码已验证；`target/x86_64-pc-windows-msvc/release/ai-relay.exe`仍是旧构建，本批保留但不宣传为最新；dist同样未更新。
- 全部验证证据2,472文件／135,820,272字节，按相对路径＋内容计算聚合SHA256前后均为 `1344A755A1ED24E737BC92D7D2AC8749C75261AAF7FA54B424E1DB63794A17E3`。
- 清理后分别重定向执行Debug／Release `--version`，均为 `AI Relay 2026.10.18`、退出码0，不启动GUI／代理或读取真实配置。版本号不用于证明构建新旧，依据是Cargo新鲜度检查及哈希。
- 首轮清理脚本在dry-run捕获Cargo stderr时遇到Windows PowerShell将正常摘要视为NativeCommandError，实际删除尚未开始；局部按退出码处理摘要后执行成功，无放宽路径／进程／数据检查。
- 无Rust行为变化，不重跑全套测试以免立即重建刚删除的缓存；前批399项测试及视觉未决项仍为历史事实，不冒称本批再次验收通过。

## 影响与下一步

当前两个主EXE和符号可直接使用；下次Debug编译／测试需重建本项目缓存，首次耗时会增加。Release及第三方缓存保留，所有报告／截图未删。菜单纯黑捕获、Release浅色月历翻月页脚缺失、高DPI／多屏／高对比度／读屏验收限制不因清理消失。
