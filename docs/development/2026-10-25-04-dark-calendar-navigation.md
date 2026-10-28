# 2026-10-25-04：修复暗色月历翻月后变亮

## 目标与范围

用户截图指出日志日期月历翻到下一页后持续变成浅色。修复日志2224／2225和统计2302／2303共用日期组件，不改日期值、范围、时区、查询、代理或配置语义。保留所有前批未提交改动、不提交、不更改版本或dist，不访问真实模型／Key／用户日志或配置。

## 根因与调查证据

1. **原生动画绕过窗口过程**：新增screen-only回归初始深色，PageDown及鼠标next后仅约5%像素暗色，1秒后仍白，tint子类仍安装。队列取证确认`WM_TIMER.lParam != 0`的回调属于COMCTL32，DispatchMessage直接调用TimerProc，没有进入calendar子类；原生打印仍重新得到暗色栅格，因此旧WM_PRINT测试漏掉了真实屏幕失败。
2. `BufferedPaintStopAllAnimations`前后调用即使返回成功也无效，不能控制该原生TimerProc；最终代码不含这个实验或任何定时补画／timer拦截。
3. **经典布局裁切根因**：改为原生深色颜色后，calendar child为238×177，但独立DropDown宿主仅216×176。之前只重算child的minimum，右侧／页脚仍被较小父容器裁切。根据原生inset把宿主校正为244×183后可完整包含child；不是盲目放大月历。
4. **星期颜色耦合**：初版原生深色TITLEBK与MONTHBK同色，星期文字不可见。实际截图揭露后，按`MCHT_CALENDARDAY`新增七列ink断言，先失败（七列全0），再将TITLEBK改为accent、TITLETEXT改为on_accent通过。此前无星期图与报告保留，不能算最终可读性通过。

## 实际变更

- `src/ui/date_picker.rs`：日期框继续themed＋tint；只有深色、非高对比度的弹出monthcal切到经典原生六色并卸载tint，同时设置DTP缓存与当前MCM颜色。原生TimerProc直接画出的也是深色，不改原生导航。
- 最终字体后重算`MCM_GETMINREQRECT`及`MCM_GETMAXTODAYWIDTH`，再调整child及同线程、直接父级、DropDown＋WS_POPUP宿主，必要时按显示器工作区移动最终popup，不缩小minimum。浅色／对比度恢复默认主题和系统颜色。重复apply不不断增大。
- `src/ui/date_picker/tint/navigation_tests.rs`：四项新增回归：screen-only键盘／鼠标翻月、四日期×三字体参数的七列星期与日期ink／标题／页脚／命中／主题恢复、工作区右下边界、30轮开关导航及主题恢复GDI。旧字体／栅格／打印等测试保留。
- `tools/smoke-dark-date-picker-ui.ps1`：暗色翻月先独立popup screen-only，再打印检查，不让PrintWindow干预观察；采样计划0／50／350／1000ms，记录实际copy开始／结束，图片分析保存延后。0ms只检查暗色／白色上限，稳定帧还必须有标题／星期／日期／今天页脚。遮挡／前台请求拒绝必需样本失败，不当作通过。
- 新增`-NavigationOnly`定向入口：私有暗色停止配置，仅等待目标日期页可见，不读隐藏启动状态；四框翻月、16张物理图、4张后续稳定打印图、原生Right／Enter选日、配置哈希不变／正常退出。原完整模式断言和历史失败不改、不扩大超时。
- [ADR0018](../decisions/0018-native-dark-calendar-navigation.md)定点取代ADR0016的月历着色路线；日期框原方案不变。更新UI设计、开发指南、索引和STATUS，无新依赖／运行时／数据迁移。

独立只读review发现宿主放大后越出工作区的P2，已加工作区clamp及edge回归。最终针对颜色／布局复核无剩余确定P1/P2；不是全系统验收声明。

## 验证

Windows x64 MSVC，锁定离线依赖；GUI与native HWND测试串行，测试仅使用虚构本地数据。

| 命令／范围 | 结果 |
|---|---|
| `cargo fmt --check`；`rustfmt --check --edition 2024 src/ui/date_picker/tint/navigation_tests.rs` | 通过 |
| `cargo check --all-targets --locked --offline` | 通过 |
| `cargo clippy --all-targets --locked --offline -- -D warnings` | 最终通过；初期测试表达式precedence告警已明确括号修复 |
| `cargo test --lib ui::date_picker:: --locked --offline -- --test-threads=1` | 15项通过，1项physical默认ignored，另显式执行 |
| `cargo test --lib --locked --offline -- --test-threads=1` | 最终268项通过，2项physical默认ignored；177.82秒 |
| 全部15个集成目标，`--locked --offline -- --test-threads=1` | 153项通过；与库共421项分段通过 |
| `cargo test --doc --locked --offline` | 通过，0项 |
| 显式`calendar_navigation_screen_stays_dark_without_print_or_forced_repaint --ignored --test-threads=1 --nocapture` | 最终通过；15帧，无Print／强制重绘，键盘＋鼠标next跨年导航；原生星期／日期／页脚存在 |
| 修改工具两个模式`-ValidateOnly` | 通过；解析／C#编译／静态调用／内容缺失及亮白上限负例 |
| Debug NavigationOnly | 最终通过，四框16物理帧＋4稳定打印／选日／正常退出；无skip，配置不变 |
| Debug完整日期工具 | 未通过，启动读取状态文本超时，尚未日期验收，不以定向通过覆盖 |
| `cargo build --locked --offline` | Debug通过 |
| 静态CRT `cargo build --release --locked --offline` | 最终通过，120.542秒 |
| Release PE／导入／`--version` | x64 PE32+／无额外CRT等运行库／输出2026.10.18及退出码0通过 |
| 最终Release NavigationOnly | 通过，DPI96，四框16物理帧＋4稳定打印，星期／日期／今天页脚／选日值／配置不变／正常退出均通过；Complete=false，不代替完整模式 |

最终全量组合命令触及300秒上限，在proxy_model目标中被终止；随后独立运行库、全部15集成目标及doc补齐，不把超时记为整条命令通过。两项默认ignored分别为前批滚动条与本批月历物理测试，本批显式执行月历，前批滚动条不声称重跑。

**采样时间边界**：0／50／350／1000ms是调度计划，不是精确帧时间；桌面CopyFromScreen可能花费数十到数百ms，报告保存实际时间。0ms允许原生瞬时局部内容，但不能白背景；稳定样本和随后独立打印必须内容完整。有限帧不证明连续视频的每一帧。

## 失败与处理

- 原native screen-only变白的失败、StopAllAnimations无效、父容器裁切及星期隐藏均先复现后修复，不移除相关断言。
- 初期physical基线有一次尚未完成原生／DWM呈现（全亮0暗色）；仅初始就绪改为2秒有界screen-only观察，不请求补画；导航样本不重试，最终通过。
- 完整Debug工具启动Read text timeout：`target/dark-date-ui/20261025-111151-c4ea724ca0574f1ba2cd947347f95066/report.json`，原失败保留。
- 首NavigationOnly的2225在约8ms只有标题与部分日期、暗色背景，50ms后完整：`20261025-112225-90314aa4a67c4098a0b8f13359bb630e`。图已查看；明确区分瞬时栅格／稳定内容，暗色65%及亮白12%上限不放松，新增纯规则自测。
- 下一次screen-only被其他窗口遮挡拒绝：`20261025-112841-4c4419c4504344409afbf30601a51638`；工具增加一次正常前台请求，拒绝则失败，不关闭其他应用或忽略遮挡。
- 星期修复前Release`20261025-113209-07831c2d295547f29350a28dfda2ff08`虽然当时旧断言通过，但截图缺星期，不能当成最终成功；已增强native与工具断言并重新构建、验证最终配色。

## 最终产物与证据

- Debug：`target/debug/ai-relay.exe`；最终静态CRT Release：`target/release/ai-relay.exe`。
- Release7,899,648字节，SHA256 `1496F414FF135CE0B0D3A35E1861688B489DCE7FDFC36D50873014F768ABF005`。
- 构建／PE／版本：`target/release-build-2026-10-25-calendar-weekdays.json`；先前实验构建calendar-nav报告不覆盖。
- 最终Debug：`target/dark-date-ui/20261025-114429-cd778becaea840d194c3b03d36351126/report.json`。
- 最终Release：`target/dark-date-ui/20261025-115530-bbc48060cf8e4b8cbd1983f7ccb4d8c8/report.json`。日志开始及统计结束的1000ms实际屏幕图已查看：强调色标题、七列星期／日期及完整今天页脚可见。
- 基线HEAD `7de0eda`＋前批及本批未提交修复；版本仍2026.10.18、dist与显式target旧包不变。

## 未完成与下一步

- 完整日期工具的启动状态读取超时及旧浅色页脚失败未被定向暗色翻月通过覆盖。
- 真正150%／200%桌面缩放、多屏／DPI切换、Win10/11差异、真实高对比度、硬件键鼠／IME／读屏仍待验收；本批三字体参数不替代这些项。
- 经典原生箭头可保留系统按钮色，未改成全自绘；月份／年份高级视图继续需要独立验收。
