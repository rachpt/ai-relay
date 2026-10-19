# 0014：暗色滚动条鼠标跟踪与日志高度调整

- 日期：2026-10-19
- 状态：已接受
- 关联：需求8.1/8.4（v1.7）；补充ADR0006/0013，保留原生文本/列表及高对比度回退

## 背景

原有滚动条仅在原生消息返回后补画暗色。`WM_NCLBUTTONDOWN`会进入Windows非客户区modal tracking，期间可反复画白色大滚动条，等松开才补画不能修复用户看到的拖动问题。日志原来随窗口高度分配空间，没有独立可调高度。

## 决策

1. 日志继续使用原生只读多行Edit。下方新增原生Button支持的高度grip（owner draw、中文可访问caption、Tab/方向键）；高度240 DIP默认、120–720有界，Home恢复默认、拖动Escape恢复起始高度。只在UI内存保留，不引入新配置/偏好文件或界面线程I/O。
2. 使用绝对屏幕Y增量换算DIP，避免grip自身移动后以旧client坐标重复累加；父窗口验证固定消息ID后同步重排，下方表单和滚动extent一起改变，不写正文/选区。主窗口resize/DPI/隐藏/主题/捕获丢失取消当前拖动；所选高度在本次运行中保留。
3. Dark模式对已识别的多行Edit、report ListView和页面宿主2400接管滚动条NC鼠标输入，避免进入原生白色modal循环。命中仍根据公开 `GetScrollBarInfo` 的真实完整条带/拇指矩形，不把6 DIP画面当成窄命中区；宿主Static仅在真实条带内修正HTVSCROLL/HTHSCROLL返回值。
4. 通过公开 `SetCapture`、`TrackMouseEvent` 和仅交互期间存在的timer处理thumb拖动、轨道分页、端部行滚动/按住重复。取消先清状态，再释放本交互的capture/timer/thread-local Escape hook；不窃取原生文本选区的捕获。hook仅当前GUI线程、只在capture期间存在，不做全局按键监听或遥测。
5. 不用 `SetScrollInfo`伪造trackpos或将位置打包到16位高字。Edit垂直由EM_LINESCROLL、水平按原生平均字符单位与真实像素range换算；ListView report使用原生行/像素适配；viewport用完整i32私有scalar消息驱动既有scroll路径。原生range/page、文本、表格、键盘/滚轮、selection/caret及MSAA结构保持。
6. 跨同步native查询/滚动可能取消、销毁或重开capture，使用保留Rc和交互generation验证后再继续；Report极大像素位移分两阶段时也不使用已取消的target/行高。
7. Light/Contrast卸载暗色鼠标适配，恢复原生系统行为；grip在高对比度始终使用系统前景/背景配对，不单用高亮色在Window底色画条。未知控件继续native，不为统一样式引入未文档API、WebView或新Cargo依赖。
8. geometry/font/style/DPI/WM_SETREDRAW/EM_SETRECT事务内，`input_frame`和scroll主题只排队NC失效，深度守卫抑制重入补画。借用打印DC单独保护，不改实际客户区/region/styles；正常paint后补画。由此修复前批no-redraw中间帧P2。

## 备选方案

- timer循环补画覆盖原生modal白条：不采用，会闪烁且不能保证每帧正确；timer只用于实际交互的repeat/取消检查。
- 完全自绘日志文本或新的列表/滚动容器：不采用，避免重做输入/选区/复制/可访问性。
- 调用未文档暗色API：不采用，保持Windows10/11公开API边界。

## 影响与验证边界

Dark鼠标捕获和repeat改由本模块驱动，属于有限交互适配，不再把它描述为纯颜色覆盖；原生内容、实际位置/range/page仍是唯一事实源。当前Edit超长行的水平范围存在原生上限，本批不扩大它；垂直/Report保留超过16位范围回归。

158项UI（17滚动、12输入边框专项）及370项全量串行测试、严格Clippy、Debug/静态CRT Release通过。Debug与Release完成高度/捕获/原生位移/选区保持交互矩阵，Debug取得按住阶段物理截图；Release物理图因Explorer遮挡跳过。保留高度会话语义，不宣称跨重启记忆或连续视频/物理键鼠/多屏/高DPI/读屏验收。详见[开发记录](../development/2026-10-19-04-log-resize-dark-scrollbars.md)。
