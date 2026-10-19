# 2026-10-19-07：修复深色模式下的浅色标题菜单

## 目标与原因

用户截图中的文件／代理／视图／帮助及系统窗口菜单在深色界面上仍为浅色。既有主题只设置DWM标题栏和客户端控件，`HMENU`内容仍由Windows按系统默认样式绘制；不属于配置未保存或主题选择失效。修复限于标题／系统菜单，不顺带替换其他弹出控件。

## 实际变更

- 新增 `src/ui/menu_theme.rs`：仅深色使用公开 `MFT_OWNERDRAW`／`SetMenuInfo(MIM_BACKGROUND)`，统一背景、文字、选中／禁用、分隔线、当前页面radio、系统命令图标和快捷键。系统菜单字体及粗体由公开DPI API读取，真实文字测宽；菜单仍由原生HMENU负责展开、导航和命令。
- 保留 `MIIM_STRING`、ID和子菜单；itemData只存有界标量索引，不解引用原菜单数据。处理 `WM_MENUCHAR`，保留单项助记执行、同助记键循环和禁用保护。绘制用SaveDC/RestoreDC和item rect裁剪，正常／异常资源按RAII清理。
- `src/ui/title_menu.rs`：接入主题生命周期，即使应用使用客户端☰且不替换caption图标，也会更新菜单。浅色／高对比度恢复原数据、位图和背景，保留期间更新的启禁／radio状态；原菜单命令及执行前权限校验不变。分配失败保持可用系统菜单。
- `src/ui/mod.rs`：只对菜单相关三类消息先路由，避免菜单ID被控件owner-draw处理误判；`theme.rs`补充可选适配失败的降级注释。
- `src/ui/menu_theme/tests.rs`：新增5项Windows原生回归，覆盖层级／Unicode文字／位图／数据／实时flags恢复，助记键与重复／禁用，96/144/192 DPI度量和绘制裁剪／DC恢复，真实宿主路由及主题恢复，30轮资源／销毁后清理。旧标题菜单GDI测试增加一次冷启动菜单字体缓存预热，不放宽循环后资源相等断言。
- 新增 `tools/smoke-dark-menu-ui.ps1`：独立私有停止fixture，拒绝已有实例，固定自建进程handle／PID／EXE及HWND；实际原生popup的F10／Alt+M／Alt+Space、四子菜单／方向键／Enter切页／助记键／Escape、启禁／radio及浅→深→浅回归。21张OFFSCREEN PrintWindow图片有界像素检查，10轮主题循环，配置哈希不变及正常退出；无真实Key／HTTP／计费、剪贴板、Startup或系统主题修改，不强杀。
- 同步README、需求8.1、UI设计／开发指南；新增ADR0015并链接ADR0009与索引。不添加依赖、配置迁移、版本／标签、提交或dist变更；保留原有工作区其他批次修改。

## 验证

| 命令或验收项 | 结果 | 范围 |
|---|---|---|
| `cargo fmt --check` | 通过 | 最终工作区 |
| `cargo check --all-targets --locked` | 通过 | Windows x64 MSVC |
| `cargo test --locked --lib ui::menu_theme:: -- --test-threads=1` | 通过 | 新增5项 |
| `cargo test --locked --lib ui::title_menu:: -- --test-threads=1` | 通过 | 既有5项 |
| `cargo test --locked -- --test-threads=1` | 未整条完成 | 240秒预算届满；236项库测试全通过，随后在历史reader集成阶段被终止，不能称该命令完整通过 |
| 显式15个 `--test` 集成目标，`-- --test-threads=1` | 通过 | config_foundation/persistence、history_backend/reader/reports、logging_backend/paging、proxy_core/features/history/logging/metrics/model/usage、usage_observer，共153项，无跳过 |
| `cargo test --locked --doc` | 通过 | 0项；结合上项共覆盖236库＋153集成，389项 |
| `cargo clippy --all-targets --locked -- -D warnings` | 通过 | 最终无告警 |
| `cargo build --locked` | 通过 | 更新Debug |
| `powershell.exe -NoProfile -NonInteractive -File tools/smoke-dark-menu-ui.ps1 -ValidateOnly` | 通过 | PowerShell AST／helper参数数目、C#编译、x64 ABI |
| Debug菜单专项 | 定向范围通过 | 96 DPI，21张根／子菜单浅深图片，F10／Alt+M／Alt+Space／方向键切页及V→O助记执行；配置不变、正常退出 |
| 静态CRT `cargo build --release --locked` | 通过 | 当前进程编码Rustflags＋home／工程编译路径映射，无持久环境改动 |
| Release x64 PE32+导入检查 | 通过 | 未见需另装CRT／SQLite／OpenSSL导入，不能替代干净系统验收 |
| Release菜单专项 | 定向范围通过 | 同样96 DPI／21张／原生导航与恢复；正常退出、配置不变 |
| `git diff --check` | 通过 | 仅既有CRLF归一化提示 |

### 过程中发现并修正

- 新增代码最初错误复用 `MENUITEMINFO` 做 `MIIM_STRING` 查询：API会清除未请求的其他字段，导致子菜单遍历／恢复身份丢失。改为独立保存ID／子菜单／原数据快照；原生恢复回归通过。
- 空快捷键切片传入GDI测量曾触发访问异常，增加空切片短路；所有原生绘制回归通过。测试先修复Win32常量类型和DPI像素采样坐标，旧GDI精确测试补冷缓存预热；这些初期失败不冒称成功。
- 工具初期修正C# `Main` 命名警告、零参数AST检查、Windows PowerShell UTF-8 BOM、启动控件尚未创建／后台未idle的等待。无需前台的定向HWND＋离屏范围不把前台请求失败冒称物理输入通过。
- Alt+Space取消曾出现popup已销毁但系统菜单跟踪尚未结束，改为同时等待真实 `GetGUIThreadInfo` 状态，继续向自有主HWND发送Escape；失败清理使用WM_CANCELMODE＋正常Exit，不强杀。旧失败fixture／报告保留，不覆盖改写。
- 首轮Release GUI与大量原生HWND单测并行时，浅色页面选择超时；无法由该次确定产品故障。停止并行桌面验证后，原构建单独重跑通过；工具说明要求不要与其他GUI／原生窗口测试同时执行。
- 大组Cargo命令240秒超时后没有把未执行的后续Clippy／构建当作通过；独立执行最终fmt/check/Clippy/build/doc及无跳过的15目标集成测试补齐验证。曾用skip过滤的诊断性运行不作为完整集成证明。

## 产物与证据

- 最新可运行程序：`target/debug/ai-relay.exe`、`target/release/ai-relay.exe`；版本仍为2026.10.18，dist未更新。
- Release为7,881,216字节；SHA256：`356CAA0CCBBA8D4B998DBE603B350AF772861A2437A61EADC520670F4E473162`。
- 最终Debug报告：`target/dark-menu-ui/20261019-152638-d8b9210298c64fd6a57fb0806a143a54/report.json`；此前通过报告为 `target/dark-menu-ui/20261019-145738-48175c2a103645ebb29478fe8412285b/report.json`。
- Release报告：`target/dark-menu-ui/20261019-151708-51445740312e428a8598136aa34f7e2d/report.json`。
- 两构建 `AutomatedScopeComplete=true`、`Complete=false`、正常退出、配置哈希不变。深色根菜单与四子菜单图片已查看；原生边框／展开箭头细节仍由OS参与，不宣传逐像素完整WinUI。Release暖态GDI基线93、循环94–95、最终94，无逐轮增长；更严格进程内30轮资源相等回归通过。

## 未完成与下一步

- 图片全部为OFFSCREEN，不代表物理桌面无遮挡／真实鼠标点击。GUI只执行当前96 DPI；144／192是原生度量及绘图测试，不替代实机150%／200%／多屏验收。
- 高对比度回退经传入Resolved::Contrast回归验证，未更改真实系统高对比度。真实读屏、系统主题变更、跨屏移动与物理键鼠仍待完成。
- 托盘／表格右键菜单、消息框、文件对话框和日历仍可能浅色，不在截图所指标题菜单修复范围。
- 不新增独立review通过结论：本批尝试的子agent连接不可用；已完成代码自查和上述回归，但不冒称独立审核。
