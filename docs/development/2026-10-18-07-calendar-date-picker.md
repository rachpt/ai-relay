# 原生日历日期选择

日期：2026-10-18

## 评估与目标

自定义日期输入改为可点击月历，减少格式输入与错误，有必要且可用现有Win32依赖完成。暂不增加全年热力图：小时/月趋势不能直接复用为每日格，无记录/未知不能混同零；新覆盖口径和额外查询会扩大并行业务改动及挤占页面。

## 实际变更

- 新增 `src/ui/date_picker.rs`：原生 SysDateTimePick32，DTS_SHOWNONE 可空日期，固定 yyyy-MM-dd，范围1970-01-01..9999-12-31，开始含/结束不含前缀；原生键盘、复选框和月历选择。
- 桥接 WM_SETTEXT ISO/空及 WM_GETTEXT，原ID2302/2303和查询读取方式保留；非法输入拒绝并保留旧值，不静默正规化日期。文本仅展示前缀，不混入实际值。
- history 创建两日期选择器替代 Edit，宽336 DIP、y136，只自定义显示。起止可分别清空，时区转换、DST、左闭右开、scope/pending等现有逻辑不变；mod 注册模块/日期dropdown通知，theme更新复制外观，不改历史数据层。
- 暗色闭合未聚焦态用应用色；编辑/禁用/高对比度原生回退。弹出月历最终保留 Windows 视觉主题，只做公开颜色best-effort及字体更新。按原生 MCM_GETMINREQRECT 和nonclient尺寸校正，dropdown返回下一UI轮再次校正，避免原生覆盖初始尺寸；不使用循环重试或未公开API。
- 新增 `tools/smoke-date-picker-ui.ps1`：PID/mutex安全、临时配置、无网络，真实原生popup及键盘选日，跨进程SYSTEMTIME/RECT读取使用目标缓冲区，严格最小尺寸/物理截图/绘制完整性断言。
- README/UI-DESIGN/STATUS/开发索引更新。本批无新依赖、无数据迁移、不提交/不重打包；另一线程在本批期间已整合提交部分日期初始接线，未改写其提交，仅保留后续样式修正与测试。

## 验证结果

使用 `target/date-picker-validation` 独立源码快照，避免共用功能线程构建产物。

- 本批接线及模块rustfmt检查通过。
- `cargo check --all-targets --locked --offline`：最终通过。
- `cargo test --locked --offline --lib ui:: -- --test-threads=1`：52项通过，其中日期模块5项覆盖严格日期/空/范围/闰年/非法值保留/原生HWND及主题通知。
- `cargo clippy --all-targets --locked --offline -- -D warnings`：最终通过。
- `cargo build --locked --offline`：最终Debug通过。
- `cargo test --locked --offline -- --test-threads=1`：最终263项通过，0失败，日志 `target/date-picker-validation/final-test.log`。
- GUI：200% DPI，日期2024-02-15实际月历PageDown/Right/Enter选到2024-03-16；清空复选、ISO桥接、闰日、非法值保留、初始空、边界、重置显隐、切Tab/时区保留、pending冻结通过。配置字节不变，仅自有子进程1002退出，临时数据清理无错误，无kill。
- 初期深色尝试在dropdown时关闭popup主题导致字体/几何不一致，420×365最小尺寸但实际376×294；尺寸延期修复后仍有绘制缺口。未掩盖失败，最终取消强制classic主题。最终浅/深模式四个popup客户端均378×296，满足原生最小尺寸且绘制完整；四张弹窗人工复核标题、导航、七列及今天页脚均完整。
- 最终18张截图和报告：`target/date-picker-screenshots/20261017-210907-c4b695d0503c412f858979bee80060f8/`，Passed=true。

## 限制

- 暗色下聚焦/弹出月历可能为浅色原生控件，已明确降级；不宣称全暗色。
- 仅当前显示器DPI192，不代替多屏、其他DPI、真实高对比度及读屏验收。
- 本GUI测试无mock记录，未另测日期边界的实际记录归属；既有DST/区间测试保留并全量通过。
- 只读review发现既有范围边界：UTC正偏移的Local选择1970-01-01会转换为负毫秒，而后端query validate拒绝负时间戳。未擅改并行数据层；该极端日期仍可能得到真实查询错误，需后续协调处理，不能静默钳制结束边界为0。
- 最终日期样式修正未重打EXE/dist，不等于当前运行/既有发布包已包含。
