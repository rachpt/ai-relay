# 2026-10-30-01：分段限宽、深色月历白边与概览稳定刷新

## 目标

- 设置页分类／主题分段不再随大窗口无限拉伸，窄窗口及DPI保持可用。
- 消除深色弹出月历外框白色立体边沿，不重写原生日期或导航。
- 定位概览采样更新的闪烁／跳动路径，并改善速率刻度和密集标记。

## 实际变更

### 设置分段

- `src/ui/settings/panel.rs`：分类最多864 DIP，主题最多456 DIP，分别保持24／180 DIP左锚点；宽度为可用空间与DPI上限的较小值。普通输入框及页面仍按原规则拉伸，不居中、不改草稿或预览／保存语义。
- `settings/tests.rs` 增加真实HWND／字体／选项矩形回归；`src/ui/tests.rs` 及 `tools/smoke-modern-ui.ps1` 同步限宽契约，继续用未限宽的2010检查实际客户区右侧24 DIP和滚动条显隐后的stale宽度，保留尾部／缓存／草稿检查。

### 月历外框

- `src/ui/date_picker/popup.rs` 新增局部展示；`date_picker.rs` 在当前popup接入并在浅色／高对比度恢复前卸载。只处理实际NC及原生DropDown客户区空隙，排除所有直接子窗口；不改样式或尺寸，不染月历正文，不取代原生箭头／TimerProc。原六色、工作区限制及日期输入语义保留。
- `date_picker/tint/{chrome_tests,chrome_physical_tests,navigation_tests}.rs` 覆盖打印局部／children、DC映射、卸载／同步销毁、主题恢复、资源及实际外框屏幕取证。
- 本机取证：calendar及DropDown均无NC，calendar在244×183宿主内偏移3像素。经典native宿主空隙有1,676／2,526个亮色像素，重绘calendar后仍在；适配后0／2,526，calendar客户区、箭头、颜色、几何和日期值不变。六张完整outer BMP及报告保留于忽略的 `target/calendar-outer-evidence-23440-1793302719045360500/`，已查看原生／适配后的DropDown实图。此前hidden fixture的NC=0／0不当作白边复现证据。

### 概览刷新及刻度

- `src/ui/overview_dashboard/surface.rs`：保留原生STATIC文字及owner绘制路由，但绕开 `WM_SETTEXT` 的同步擦除／绘制，更新只排队失效。真实回归原生为draw=1／background=1，改后0／0，最终paint只draw一次。`WM_PRINTCLIENT`检查client及可见性选项。
- `overview_dashboard.rs`：私有内存DC整帧绘制后一次拷贝；bitmap上限16M像素（常规32位约64 MiB）、失败直接绘制降级，资源按帧回收。固定88 DIP绘图区，未知提示显隐不推移曲线。流量刻度自动B／KiB／MiB等每秒单位、计数十进制单位、1／2／5刻度留顶；精度省略标记约值。普通主题只标端点／转折，高对比度第二序列保留逐样本空心方形。
- `overview.rs` 对头部／等价描述增加相等去重；精确B／请求数、累计、实际时间轴、隐藏冻结、未知断线及采样上限不变。未平滑、插值或补造数据。
- `overview_dashboard/painting/visual_tests.rs` 补整幅缓冲／直接像素对照及短高对比度线段；`tools/smoke-overview-dashboard-ui.ps1` 更新固定plot-top几何。
- 设计取舍见[ADR0020](../decisions/0020-bounded-dashboard-popup-chrome.md)。无新依赖、配置迁移、代理／安全／数据存储语义变更。

## 验证

在Windows x64、Rust／Cargo 1.99.1，Cargo使用 `--locked --offline`；原生HWND／GUI均串行，不访问真实模型或Key。

| 命令／验收 | 结果 | 范围 |
|---|---|---|
| `cargo fmt --check` | 通过 | 最终代码格式 |
| `cargo check --all-targets --locked --offline` | 通过 | Windows所有目标 |
| `cargo test --locked --offline -- --test-threads=1` | 通过 | 305库＋153集成＝458项，doc通过，3项物理测试默认ignored |
| `cargo clippy --all-targets --locked --offline -- -D warnings` | 通过 | 最终严格检查 |
| 概览专项 `cargo test --lib ui::overview -- --test-threads=1 --nocapture` | 26项通过 | 字体／布局／采样／单位／打印／缓冲像素及GDI；96／192绘图参数不是实际缩放 |
| 设置 `ui::settings::tests::` | 9项通过 | 96／120／144／192字体参数及窄宽切换；更新后的共享滚动测试另定向通过 |
| 日期专项 | 最终通过 | 首次17过／1失败／1ignored，修复reference后含入最终全量；见下方失败记录 |
| `calendar_navigation_screen_stays_dark_without_print_or_forced_repaint --ignored` | 通过 | 15张screen-only导航帧；同步检查calendar／宿主outer边沿 |
| `calendar_popup_outer_screen_baseline_and_adapted_border_evidence --ignored` | 通过 | 独占桌面前后6张完整outer物理BMP，亮边1,676→0，calendar内容逐像素不变 |
| modern／overview／dark-date `-ValidateOnly` | 通过 | 解析、C# ABI及纯几何／图片自检，非GUI验收 |
| `smoke-dark-date-picker-ui.ps1 -NavigationOnly -ExePath target/debug/ai-relay.exe` | 通过限定scope | 四日期框16张screen-only＋4稳定打印及选日，配置不变／正常退出；不是完整浅深矩阵 |
| `smoke-overview-dashboard-ui.ps1 -ExePath target/debug/ai-relay.exe` | 两次通过限定scope | 96 DPI，21本地请求／7 HTTP400、真实字节／速率、隐藏／启停、每次23张OFFSCREEN图、6轮GDI最终88；首次54.33秒，固定88 DIP工具几何同步后的最终复验53.41秒，Complete=false |
| `smoke-modern-ui.ps1 -ExePath target/debug/ai-relay.exe` | 通过限定scope | 96 DPI浅深四页16／设置24／统计子视图24矩阵，168次OFFSCREEN捕获、保留165个独立PNG（3个旧输出名被复用），215.55秒；15 Edit草稿／选区／配置不变、正常退出；非物理全验收 |
| `cargo build --locked --offline` | 通过 | 最新可运行程序为 `target/debug/ai-relay.exe`；未重建Release／dist |
| 编辑器diagnostics | 无错误／警告 | 本批刷新一次 |

### 失败、纠正及复核

- 初次fmt／check发现新surface导出可见性及测试RECT导入遗漏，修正后通过；新增物理测试通过include引入的inner-doc编译错误已修正。
- 日期打印回归首轮在flags=0x10（`PRF_CHILDREN`）像素对照失败：reference与实际打印入口的DC文字／对齐属性不同；统一输入属性后定向及最终全量通过，保留严格sentinel和children内容比较，未放宽断言或改生产打印掩盖。
- 首次完整测试304过／1失败／3ignored，未执行后续集成；失败为旧共享测试强制分类填满宿主（888对959），按新864 DIP上限更新契约，并保留独立全宽输入检查。最终完整重跑全部通过。
- 独立只读review发现早期月历NC-only打印误裁children、概览高对比度短虚线退化及打印flags三项P2，均修复并加针对回归；后续定向只读复核未发现本范围确定新增P1／P2。日期方案已从早期导航覆盖收敛为仅外框，不保留透明导航子窗口和样式事务。
- 概览深色扩展完整图、native／adapted月历宿主实图已查看；不声称逐图逐字、OCR或连续物理刷新录像验收。

## 证据与限制

- 四日期导航：`target/dark-date-ui/20261029-191214-3268627a04a645c39f713974bde9de86/`。
- 概览首次：`target/overview-dashboard-screenshots/20261029-191410-e39e52bad7944b7f90c6e0088d5d4276/`；当时工具仍使用旧68 DIP预期plot区域，生产已固定88 DIP。同步工具后的最终复验：`target/overview-dashboard-screenshots/20261029-201431-1873d55fc71d4398a8d886c226f462c6/`，53.41秒通过、23图／相同功能范围，日志 `target/ui-refinements-overview-2026-10-30.log`。粗墨迹检查不作图表坐标精确证明，真实绘图区布局由进程内回归验证。提交前独立review发现旧raw-rates测宽／固定单位预算与新历史峰值选单位不一致；此两次旧工具通过不作新图例预算证明，工具修复及新验证见[提交前记录](2026-10-30-02-precommit-review-release.md)。
- modern：`target/modern-ui-screenshots/20261029-200341-f24483e1ba2747a99f86af2879799ed6/`；日志 `target/ui-refinements-modern-2026-10-30.log`。
- GUI脚本的UTC日期前缀与本地记录日期不同，均保留实际输出路径。普通测试／字体参数不能替代真实150%／200%、多屏、高对比度／Win10差异、读屏／IME或长稳。
- 概览最小窗口完整dashboard高于viewport，两处全图验收按既有规则跳过；保留分段／几何，不冒称完整图像通过。未做连续物理帧或整个页面无闪烁保证。
- 原生箭头仍为经典浅色按钮（本批仅优化外框）；未完成完整日期浅→深→浅GUI、Release及所有系统行为验收。原既有超时／菜单截图／滚动限制仍保留。
- 未提交Git，不覆盖现有Release、dist、用户配置、剪贴板、自启或系统主题；开始时已有开发记录及并行清理记录均保留。

## 下一步

- 在最新Debug复核实际大窗口、150%／200%及多屏，重点物理图表连续更新和月历宿主边沿；Release需另行构建再验收。
- 保持既有完整日期／历史／滚动专项限制，按STATUS继续补齐，不能由本批限定通过推定已解决。
