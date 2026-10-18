# 柔和圆角控件

日期：2026-10-18

## 目标与变更

按用户要求给界面适度小圆角，不修改业务或增加依赖。已有卡片/图表/分段选择/switch保留，按钮、单行Edit、闭合Combo及日期框采用6 DIP半径。

- 新增 `src/ui/rounded.rs`：按类与style严格筛选，用公开CreateRoundRectRgn/SetWindowRgn只裁剪目标控件外形，尺寸/DPI变化更新缓存区域。成功转移region所有权给OS、失败释放；Rc/Cell确保重入销毁安全，高对比度清除region并卸载。不处理多行Edit、表格、popup/菜单/复选开关/Tab，不修改值、选区、输入style、可见性或焦点。
- 提供GDI `surface` 共用小圆角表面，SaveDC/RestoreDC保护stock pen/brush；theme浅深普通按钮使用圆角owner-draw，control_theme深色闭合combo、date_picker深色未聚焦闭合态复用。高对比度仍用系统。
- `mod.rs`仅模块声明、theme遍历时调用rounded apply；不改代理/统计/设置逻辑。
- 初期尝试只补画Edit非客户区圆边，但review确认弧线进入客户区时会被排除裁掉，已删除此层补画；最终保留原生边框，仅外形clip，不为像素弧线覆盖文本或caret。
- 新增 `tools/smoke-rounded-ui.ps1`，临时无auto-start配置、无网络，PID/mutexsafe只自动化自有child。检查region corner/bounds、输入/caret/selection、native下拉和月历、query pending、主题/resize与GDI，物理DPI截图。

## 验证

隔离快照 `target/rounded-ui-validation`，不改主Cargo.lock/构建产物。

- all-targets check：通过。
- 全量串行tests：278项通过，`full-test.log`。随后移除Edit边框补画，最终再跑67项UI测试通过、严格Clippy通过、Debug构建通过，未将较早全量误称为最终逐行快照结果。
- rounded独立Windows 3项测试覆盖96/144/192DPI、native值/选区/焦点/显隐/resize/contrast恢复；600轮GDI10→10。
- 本批5个源文件rustfmt检查（清理unused frame遗留空行后）通过。
- 初次格式化曾遇Windows os1224短暂文件映射占用，未删除/替换文件强行写，稍后单文件操作正常。
- 最终GUI一次92.5秒、DPI192/200%通过：6DIP region corner外、中点内、resize bounds吻合完整window；文本/键盘/选区/nativecaret状态保留，设置文本及空password不变；原生dropdown选择和月历可用；真实pending→completion/0记录；disabled state正确；100次theme+resize GDI全程81；配置字节未变，child1002正常退出无残留。
- 最终38张PNG/报告：`target/rounded-ui-screenshots/20261017-221945-b3aebde8834e4eeca2dec70570a8d34b/`。浅深截图文本、下拉条目、月历及页脚未裁切。

## 限制

Edit边框仍为原生像素，不将外形region通过当成完整边框圆弧证明；caret状态/矩形验证不替代闪烁竖线的物理像素验收。原生月历可浅色，高对比度/多屏/读屏仍待真实验收。未改运行EXE或重打dist、未提交Git，先前未提交日期/概览工作均保留。
