# 0017：暗色统计表滚动的局部合成呈现

- 日期：2026-10-25
- 状态：已接受
- 关联：需求8.1／8.5；补充[ADR0014](0014-scrollbar-tracking-log-resize.md)，仅替代Report列表先原生绘制再覆盖的呈现方式

## 背景

接管非客户区鼠标跟踪能避开原生modal白条，但不能阻止原生ListView在`LVM_SCROLL`内部直接绘制系统色滚动条。原生下层探针在调用返回、上层暗色覆盖之前测得轨道`0xF0F0F0`，解释了滚动中仍闪白而最终截图正常的现象。

## 决策

1. 只给暗色Report ListView添加公开`WS_EX_COMPOSITED`；不扩大到共享viewport或整个窗口。排除不兼容的`CS_OWNDC/CS_CLASSDC/CS_PARENTDC`。不改原生内容、range/page、选区、键盘、复制或可访问性树，不新增依赖。
2. 在可重绘的原生滚动／绘制消息期间短暂持有window DC，原生绘制后在同一DC完成暗色条带，再释放。几何／禁绘事务不立即绘制；键盘仅导航键持有DC，不跨菜单键／Shift+F10模态菜单持有。
3. 对Report `WM_NCPAINT`复制更新region，扣除横纵条及交角后交给native，保留原生边框。不修改或接管消息借用region；分配／查询失败保持可读native降级。
4. 样式只清理本模块添加的位；外部预置COMPOSITED不移除。浅色／高对比度恢复原生绘制。保留presentation owner贯穿样式changing/changed同步重入，最新主题意图有界收敛后才卸载，避免待提交样式写入遗留无所有者位；同步销毁不继续访问旧实例。
5. 单元格复制对“仅COMPOSITED扩展样式位变化”保持活跃目标，其他原有失效条件不变。

## 不采用的方案

- 增加定时器或仅多补画几次：仍允许系统色中间帧。
- `DefWindowProc(WM_SETREDRAW)`禁绘：暂时清除WS_VISIBLE使重入Hide成为无效操作，恢复会错误显示，已由现有回归捕获。
- 正常ListView `WM_SETREDRAW`包围滚动：延后发布滚动范围，破坏ENSUREVISIBLE后的同步查询，已由巨大位移回归捕获。
- 自绘整个表格、伪造range、未文档暗色API：超出必要边界。

## 验证与限制

真实非owner-data Report横纵thumb拖动、轨道按住repeat、wheel及ENSUREVISIBLE，在native返回前、暗色覆盖前读取实际屏幕；初末／按住阶段严格检查窗口所有权。最终48个中间轨道样本均暗色。另有样式所有权／浅深与对比度恢复、局部NC region、禁绘／菜单键、12组主题交错／销毁、100轮GDI及单元格复制回归。

合成并不是跨任意长同步回调的无限期原子事务；公开API的窗口DC生命周期及系统兜底呈现仍适用。实际高DPI、多屏、Win10/11差异、物理硬件键鼠／读屏，以及最终Release统计页面完整GUI仍需验收。详见[开发记录](../development/2026-10-25-03-dark-report-scroll.md)。
