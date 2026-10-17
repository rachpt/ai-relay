# 分段选择、开关与暗色控件

日期：2026-10-18

## 目标及范围

减少不必要下拉框、缩短操作路径；修复用户截图中暗色表格/表头/箭头白底；增加简单图标、语义状态色、留白。保持 Rust + Win32，不新增依赖或运行时，不使用未文档化暗色 API。

仍有另一功能开发线程修改 app/history/proxy/logs。本批不改这些业务模块、锁文件和配置模型，不执行用户实例操作、不提交。展示模块单独实现；history/settings/mod/theme只做必要接线与页面显示调整，不回退既有功能。

## 实际变更

- 新增 `src/ui/choices.rs`：原生 SysTabControl32 支撑的圆角分段选择，保留原生选择、方向键、焦点、可访问性，复制轻量主题参数，不持 State。固定选项兼容 CB_GET/SETCURSEL、GETCOUNT、GETLBTEXT 和 WM_GETTEXT，旧查询读取方式不变；选择通知经标记校验后异步转为原ID的 CBN_SELCHANGE。非此组件的主 Tab 不被接管。高对比度原生绘制。
- `src/ui/history.rs`：统计来源、日期快捷项、时区、粒度、结果视图、导出范围六组改分段选择。协议/接口/结果/分组/排序等长列表保留下拉。自定义日期才显示起止框，重置刷新显隐。已有报告时切时区/粒度自动后台重查，不因打开页就查询；pending仍冻结草稿。工具栏分行，结果视图单行，HTTP标签不再盖住Provider，增加高级筛选与操作行留白；表格去网格线。中文操作保留，并加轻量符号图标。
- `src/ui/settings.rs`：主题改三段直接预览，保存与风险确认规则不变。
- 新增 `src/ui/control_theme.rs`：使用公开API及独立原生subclass。深色闭合ComboBox内容/箭头覆绘；列表背景/文字/选中行统一，列表表头独立绘制，首次插列后补主题覆盖延迟创建表头；仅已识别列表的 NM_CUSTOMDRAW 被正确解释，避免不同控件 style位碰撞。原生BS_AUTOCHECKBOX浅深色绘制为switch，BM_GET/SETCHECK/Space/点击/确认语义不变，高对比度恢复系统绘制。系统scrollbar/menu/dialog不强行自绘。
- `src/ui/theme.rs`：遍历控件时应用上述主题与分段参数；列表custom draw先于通用按钮逻辑。
- `src/ui/mod.rs`：模块声明、分段通知分派、DPI变更后排队刷新绘制。
- `src/ui/status_bar.rs`：状态圆点与文字颜色（运行绿、停止灰、进行中琥珀、错误红），只分类首行，未知不当运行；高对比度使用系统文本色，原有状态文字/tooltip不变。
- `tools/smoke-modern-ui.ps1`：新建物理DPI、原生点击/Space、草稿/冻结/主题无隐式写入、表格颜色/箭头/switch/布局检查和截图；只用临时配置与回环mock，已有实例安全停止。
- `tools/smoke-theme-ui.ps1`：主题标签有符号后旧精确文字断言改EndsWith，不更改测试行为。

## 验证

完整源快照在 `target/modern-ui-validation` 独立编译，不占用主线程构建产物、不写主锁文件。

- 本批展示快照文件 rustfmt --check：通过。交付前再次检查主工作区时，另一线程已向 history.rs 新增 busy 等接线/测试并产生格式差异；未全局格式化或覆盖其进行中代码，不能将最终主工作区格式检查列为通过。
- cargo check --all-targets --locked --offline：通过。
- cargo test --locked --offline --lib ui:: -- --test-threads=1：46项通过（含新分段6项、控件主题3项、状态分类2项）。最后修正重置显隐、延迟表头后再次通过。
- cargo clippy --all-targets --locked --offline -- -D warnings：通过，最后修正后再次通过。
- cargo build --locked --offline：Debug通过，最后修正后再次通过。
- 首次全量串行测试在另一线程新增的 history::reader::tests::read_deadline_interrupts_sql_and_the_same_actor_remains_usable 出现计时断言失败（1 != 0）；未改数据模块。单独重跑通过，再次完整串行重跑共254项通过。日志 `target/modern-ui-validation/full-test.log`。此全量重跑发生在最后两项小修正前；之后只重跑46项UI测试、Clippy和构建，不伪称最终全量已重跑。
- `tools/smoke-modern-ui.ps1` 实际通过：DPI192/200%；10次回环mock；7个分段native类型/旧消息兼容/真实鼠标；历史/会话与结果视图切换；自定义日期显隐；时区/粒度真实后台查询；实际pending冻结、草稿保留；暗色表格/表头/箭头无大片白底；switch原生check/click/Space；浅深主题预览配置字节不变；历史页可见直属控件无重叠。46张截图，LayoutFailures/CleanupErrors为空，子进程正常退出、临时数据清理。
- 截图：`target/modern-ui-screenshots/20261017-203053-4ee7126bcb86417db4e57307ab258dc6/`。
- 后续静态复核修复重置自定义显隐、延迟表头主题；高级操作行增16DIP留白。补充对应GUI重置回归脚本后，最后重跑检测到用户已有 `target/debug/ai-relay.exe` 实例而停止，未启动测试、不发送消息/IPC、不终止用户进程；最后这几项GUI回归未执行，截图为此前通过版。

## 限制及交接

- 控件与展示快照通过，不证明并行线程之后的业务改动通过。新增分段控件不再是ComboBox，依赖枚举class的外部自动化须按控件ID定位；原有CB选择读取兼容，但不支持动态增删选项。
- 高对比度恢复代码/native隔离测试不替代真实高对比度、物理键盘、读屏及多屏验收。
- 系统滚动条仍可能亮色，保留公开API降级边界，未伪称全WinUI/Web UI。
- 原生图标以简单符号配中文，可能随系统字体略有差异，不引入字体/图片运行时。
- 状态栏原有tooltip悬停稳定性问题不在本批修复范围，保持待验收。
- 未替换正在运行EXE、未重打dist、未提交Git。
