# 2026-10-25-01：修复日期框圆角裁剪导致的断边

## 目标与根因

用户截图指出浅色日期框左右边角异常。`rounded.rs` 将 `SysDateTimePick32` 与按钮／闭合下拉一起设置6 DIP圆角window region，但日期框仍绘制原生矩形边框；裁剪删除四角直线，不能生成连接它们的圆弧。浅色原生绘制及深色栅格着色均没有补画圆弧，旧回归仅证明region形状，不能证明边框完整。

采用与Edit一致的最小修复：日期框保留完整原生边界，不增加全状态自绘或改动日期输入。需求8.1要求保留原生布局／交互及浅色／高对比度系统绘制，不要求日期子控件强制6 DIP圆角；无需变更功能验收范围、依赖路线或新增ADR。

## 实际变更

- `src/ui/rounded.rs`：排除所有日期框样式及大小写类名，移除Date资格。保留通用调用与旧subclass清理路径，仅清除本模块拥有的旧region，不触碰没有本模块state的外部region。按钮／闭合下拉继续圆角，高对比度回退不变。
- 扩展原有4项圆角回归：日期框无新region／subclass，外部region保持，自有旧region在apply／style／resize／DPI时移除，值／样式／焦点不变，600轮主题／尺寸／DPI资源检查涵盖Date。
- `src/ui/date_picker/tint/tests.rs`：日期着色回归加入真实`rounded::apply`调用，检查96／144／192字体和几何参数下聚焦、禁用、空值及浅色／高对比度回退均不设置region。不把这些参数测试称为实际桌面DPI矩阵。
- 三个GUI脚本同步改为四个日期框2224／2225／2302／2303无region断言，包含隐藏页；按钮／Combo仍验证真实圆角。深色日期工具附加浅色闭合／聚焦／空值态的实际外框及四角证据；遮挡／裁切／移动及500ms可见性预算明确skip，不冒充像素连续性断言。原有颜色／内容／月历今天页脚检查未放宽。
- 更新README、UI设计、开发索引及STATUS。无新增依赖、配置／数据迁移、代理或日期筛选语义变化；未提交、建分支、改版本或更新dist。保留起始工作区已有2026-10-20-04构建记录及索引／状态修改。

独立只读复核未发现本范围确定新增P1/P2。复核指出可选物理图全部skip也可能总体通过、逐像素命中检查比普通可见性严格，故仅作为额外证据，不宣称完整视觉验收。

## 验证结果

Windows x64 MSVC，Rust／Cargo1.99.1；依赖锁定并离线执行。

| 命令或范围 | 结果 |
|---|---|
| 新日期资格断言在修复前定向执行 | 按预期失败：旧实现仍返回Date资格，证明回归能拒绝原问题 |
| `cargo fmt --check` | 最终通过；初次发现移除分支遗留空行，已修复 |
| `cargo check --all-targets --locked --offline` | 通过 |
| `cargo test --locked --offline --lib ui::rounded:: -- --test-threads=1` | 4项通过，包含Date及600轮GDI |
| `cargo test --locked --offline --lib ui::date_picker:: -- --test-threads=1` | 12项通过 |
| `cargo test --locked --offline --lib -- --test-threads=1` | 246项通过，约137秒 |
| 全部15个集成测试目标，`--locked --offline -- --test-threads=1` | 153项通过；与库合计399项，分段执行，无skip |
| `cargo test --locked --offline --doc` | 通过，0项 |
| `cargo clippy --all-targets --locked --offline -- -D warnings` | 通过 |
| `cargo build --locked --offline` | Debug通过 |
| 三个修改脚本的`-ValidateOnly` | 通过，声明编译／解析／ABI等；不等同完整GUI执行 |
| 静态CRT `cargo build --release --locked --offline` | 通过，约164秒；沿用打包脚本的进程级编码flags及编译路径映射 |
| Release x64 PE32+／导入及`--version` | 通过，无额外CRT／SQLite／OpenSSL动态依赖，版本输出正确且退出码0 |
| Release `smoke-ui-regressions.ps1` | 定向功能通过、正常退出、配置不变；实际96 DPI，Complete=false，目录状态截图被Explorer遮挡 |
| 完整`smoke-dark-date-picker-ui.ps1` | 未通过，见下文；不能宣称四框全部状态／弹窗视觉验收完成 |
| 新改Rust文件诊断、`git diff --check` | 通过；Git保留既有CRLF归一化提示 |

集成目标：config_foundation、config_persistence、history_backend、history_reader、history_reports、logging_backend、logging_paging、proxy_core、proxy_features、proxy_history、proxy_logging、proxy_metrics、proxy_model、proxy_usage、usage_observer。仅本地mock／虚构凭据，不访问真实模型或产生计费。

## GUI实际观察与失败

- Debug日期专项两次均在启动就绪检查读取隐藏状态文本时`Read text timeout`，尚无截图／日期region证据。未延长预算或放宽断言掩盖失败；失败后已查询无残留AI Relay进程，未强杀。
- Release日期专项通过启动及四日期框无region检查，取得开始日期2224浅色闭合／聚焦物理外框、四角及展开离屏图；随后捕获月历时`Foreign/missing HWND refused`，弹窗句柄已失效。未确认原因，不将其归因本补丁或当作完整通过。已查看聚焦外框和四角实图，左右顶底边线完整，无原先强制裁掉的缺口。其余日期框全部状态尚无同等物理证据。
- Release通用regressions先因传入相对ExePath被工具前置拒绝，改用已知绝对路径后运行通过。四日期框在浅深主题／缩放中无region，按钮／Combo真实6 DIP region仍符合；5项重绘残影比较均0差异，38个Edit保持无region，20轮GDI恒94，配置不变且正常退出。目录按钮按脚本既有行为打开临时Explorer，目录状态图因其遮挡明确skip；未擅自关闭Explorer。
- 历史菜单纯黑、月历翻月今天页脚缺失等其他未完成项保持原记录，本批未解决或宣称已验收。

## 产物与证据

- 最新Debug：`target/debug/ai-relay.exe`。
- 最新Release：`target/release/ai-relay.exe`，7,889,920字节，SHA256 `BA824AB31BC1EFD5DD7E7BB59B6C3FCCFD3FD55D5A20D5DAA707D44C63CAA2B4`。
- 源码为HEAD `7de0eda`加本批未提交修改；版本仍为`2026.10.18`。显式target旧目录及dist未更新。
- 构建／PE／版本报告：`target/release-build-2026-10-25.json`；该报告的SourceCommit只标识基线，不代表已提交本修复。
- Debug日期失败：`target/dark-date-ui/20261025-050950-833b532e8acf46708d72ca78a1a3d2f8/report.json`及`20261025-051302-ee1481399f46457d89be937d03b532bd/report.json`。
- Release日期部分证据／失败：`target/dark-date-ui/20261025-051649-fb529ecc74764b4c9f3da8aa295827b3/report.json`，同目录`light-0-2224-focused-physical.png`及`light-0-2224-focused-physical-corners.png`已查看。
- Release定向regressions：`target/review-ui/regressions-20261025-051804-e3940d/report.json`，Failure=null／NormalExit=true／Complete=false。

失败日期工具按原规则保留其私有临时配置／证据，未清除或覆盖失败报告；所有产物／截图均在忽略目录，不入发行包。

## 未完成与下一步

- 隔离日期专项启动文本超时及弹窗句柄生命周期，再补齐四框浅深／空值／禁用／展开的实际截图；不以本批原生测试或单个开始框照片替代完整视觉矩阵。
- 实际150%／200%、多屏、真实高对比度／主题通知、物理键鼠、IME及读屏尚未执行。
- 本修复恢复完整原生边框，不宣称实现自绘日期圆弧；保留原有原生输入、可空复选、键盘选日及月历布局。
