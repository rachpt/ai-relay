# 紧凑滚动页面与日志日期

日期：2026-10-18

## 目标

设置分类不再藏在下拉；统一按钮样式与颜色；降低窗口高度且可访问长统计/设置/日志页；处理日志暗色白色滚动条，日志日期改picker。无新依赖/运行时，不改网络/控制器/持久化语义。

## 实际变更

- `settings.rs`：分类2039改六项原生分段，保留索引/CB读取；动作行在当前分类末尾+24DIP，修正端口宽度、自启按钮重叠；分类切换布局、不保存/丢草稿。
- `logs.rs`：2224/2225原生可空日期picker，只接受ISO/空；UTC开始含零点、结束含23:59:59，逻辑epoch解析保留但GUI不再raw epoch。类别2226改三项分段。新增UTC首尾/次日排除、原生日期桥接回归，暂停/游标/缓存/安全测试保留。
- 新增 `viewport.rs`：原生STATIC+WS_EX_CONTROLPARENT中间裁剪区，顶部48DIP和底部56DIP固定。页面原控件reparent不销毁，WM_COMMAND/NOTIFY/DRAWITEM/颜色消息同步路由，主题刷新转发。布局保留现有820/848DIP虚拟高度，四页各自offset、原生scrollbar/滚轮、焦点自动揭示；保留原生顺序，不反转SetParent后的Zorder。跨DPI换算全部offset，丢弃隐藏/失焦控件过期reveal；host标题为空，修正会话usage底部18DIP越界。
- `mod.rs`：接host、默认800×680DIP、最小800×560DIP且工作区钳制；普通Tab显式GetNextDlgTabItem，避免只读多行框吞Tab；Ctrl+Tab不变。字体更新redraw=0、布局/主题排队，不在独占字体更新借用期间同步owner-draw。各模块set_font均不强制同步重绘。
- 新增 `scroll_theme.rs`：公开GetScrollBarInfo/GetScrollInfo，深色原生非客户区track/thumb/arrow/corner覆绘，支持多行Edit、ListView和host。不修改style/位置/input/选区，light/contrast恢复native。原生拖动动画可短暂覆盖；不处理系统菜单及下拉popup。
- `theme.rs`：递归访问host直属页面控件，应用所有已有主题/圆角及scrollbar；保存/应用蓝色、删除清空muted红色，不改确认流程。
- 新增 `tools/smoke-compact-pages-ui.ps1`：递归按ID定位、真实小窗口/各页scroll、分类/nativepicker/Tab/whitebar/color/GDI；已有实例安全拒绝。
- `smoke-proxy-ui.ps1`通用GetDlgItem helper递归查找，适配页面层级。`smoke-logs-ui.ps1`同步ISO日期/拒绝非法值保留和虚拟布局检查；未删暂停/分页/轮转/隐私验证。

## 验证与修复事实

使用独立target目录 `target/compact-ui-build`，主锁文件未改。

- `cargo fmt --check`、all-targets check：通过。
- 初始UI76项通过；额外viewport native回归6项通过：adopt顺序/固定控件/转发、TOP48与底部可达、四页offset、96/144/192DPI转换、stale focus丢弃。
- 全量串行 `cargo test --locked --offline --target-dir target/compact-ui-build -- --test-threads=1`：292项通过（final-test.log）。最后仅新增按钮语义颜色，之后严格Clippy、Debug build和GUI再次通过，未声称颜色后全量再次重跑。
- 严格Clippy：通过；Debug build通过。
- 原生scroll_theme独立5项测试/GDI600次10→10；项目全量包含这些测试。
- 初轮小窗口GUI发现Tab回退次序及readonly框占用，修复adoptZorder和主循环Tab后再次通过，不隐藏初轮失败。
- 最终GUI 41.69秒，当前DPI96，800×560/800×680/1100×800均准确outer尺寸，header/footer固定、viewport clip、四页offset/轮滚/底部可达通过；history table、日志apply/detail的Tab分别12/18/19步自动揭示。
- 六类鼠标/键盘切换、草稿、动作行通过；原生日历展开/键盘闰日/清空、UTC含首尾metadata组合筛选通过；配置退出前后字节不变。
- 暗色日志主区2200横竖白色采样均0；picker箭头约3.7%仅小字形。按钮2219浅深blue，2210/2211/2350/2352 mutedred+text颜色10项通过；2352仅预览启用截图、未点击确认删除。
- 30次浅深主题GDI79→75、峰79；自有子进程1002正常退出无残留，无kill/真实Startup/模型收费。
- 最终78图/报告 `target/compact-pages-ui-screenshots/20261018-012025-99f78eda94fd45e3b892a4e050792519/`。Passed=true、Complete=false：2221动态状态被100ms刷新替换，没有稳定长数据截图；剪贴板非文本格式，复制检查安全跳过，未覆盖用户剪贴板。均NOT RUN，不当成通过。
- 更新旧logs smoke只做PowerShell AST、C#编译及diff检查，未另跑其完整文件轮转场景；本批native日志逻辑测试及新mockmetadata冒烟不代替其全脚本。

## 限制与交接

当前实机小窗口仅DPI96；DPI144/192范围有native/逻辑测试，不替代真实多屏、高对比度/读屏。页面滚动会形成原生层级，外部自动化需递归按ID查找。默认/最小width800为保障固定表单字段可读，并非任意窄屏响应式。旧package/运行EXE未替换、未重打dist、未提交Git。先前日期/概览/圆角未提交工作保留。
