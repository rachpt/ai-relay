# 当前开发状态

最后更新：2026-10-31

## 当前结论

核心功能及本轮高级统计/日志/窗口偏好已接入，提供便携开发预览；**未完成全部Windows验收，不等同正式发行版**。旧参考脚本及日志/PID已按用户公开准备授权移除；原生版tools和其他用户数据保留，本地秘密不入库。

## 已实现

- [x] 统计查询／记录／导出状态改中文分段，五项累计精确计数不再显示Counters调试结构；状态按实际字体和最终宽度测高、最低34 DIP，普通／高级图表和结果栏按底边顺排，管理页尾与虚拟滚动范围随高度调整。状态更新排队重排，隐藏页只置脏；短状态旧锚点、草稿／选区／有效scroll保留。
- [x] 浅深主题日志／统计表／页面窄胶囊滚动条与完整原生命中区；Contrast恢复系统。38×20 DIP开关、状态／眼睛图标有界4×4覆盖采样抗锯齿，正文与输入仍原生；保留资源失败回退。
- [x] 上游Key显隐眼睛＋中文按钮，默认掩码；切分类／主Tab、隐藏／最小化／关闭、加载／放弃草稿或禁用操作时重掩码，保留文本／选区／保存语义。端口／网络数字仅ASCII、整段粘贴／替换校验、范围提示与保存共用元数据；空／越界草稿保留，不clamp。类型限宽、网络单／双列及窄窗动作换行，实际客户区宽参与滚动范围，六分类／旧ID保持。
- [x] 关于取Cargo版本，按编译配置显示Debug开发构建／Release构建；Release主标题去掉开发版，不把profile等同正式发行验收。
- [x] 设置分类／主题分段最大864／456 DIP，窄窗口及DPI自适应，原左锚点／草稿保留；普通输入按字段类型限宽或伸展。
- [x] 原生四Tab、浅/深/跟随系统、分段选择与switch、表格/下拉、固定状态栏和K/M图表摘要；浅灰／中性深灰层级、主导航中性选中面＋短强调指示条／被动hover、原生焦点与选择分开、一体分段、实心主要动作、4 DIP按钮／闭合下拉、8 DIP概览／统计卡片及柔和表格。禁用／按压／焦点细化，按钮文字按下不位移；状态栏24 DIP对齐、语义色限于小图标；Edit／日期框保留完整矩形输入边界。高对比度有系统回退，真实读屏／高对比度仍待验收。
- [x] 标题／系统菜单和四组子菜单公开API深色适配，保留原生文字／命令／键盘／助记／启禁和radio；浅色／高对比度恢复，托盘及其他popup仍可系统色。
- [x] 日志及统计四日期框深色栅格着色；弹出月历改为经典原生深色六色，避免翻月TimerProc直接绘制留下白帧。强调色标题及七列星期／日期、今天页脚可读；按最终字体minimum／today width同步校正calendar及原生DropDown宿主并限制到工作区。暗色局部适配实际NC及DropDown空隙白边，屏幕亮像素1,676→0，calendar正文／原生浅色箭头不改；浅色／高对比度卸载。聚焦／展开／禁用／未设置、输入和空值保留，浅色／高对比度恢复原生主题／系统色。日期框取消强制圆角region，保留完整原生边框，修复四角断边；按钮／闭合下拉仍保留圆角。
- [x] 用户/session单实例、受限IPC、GUI子系统和完整CLI启停/退出码。
- [x] 当前用户DPAPI、配置原子保存/冲突检测、导入导出/恢复、保存应用及确认回滚。
- [x] 当前用户Startup COM自启、实际状态与确认操作；LAN一次性授权。有效已保存回环配置普通打开默认自动监听，保留显式关闭；“启动存储”开关说明及启动中/未配置/LAN/失败提示已完善，登录启动查询不覆盖代理结果。
- [x] 流式反向/正向代理、CONNECT、CORS，超时/取消/背压/并发/认证和可信凭据边界。
- [x] 请求模型有界观察、响应usage完整度；不存正文，不把未知计零。
- [x] 历史IP/请求与响应模型/两类凭据ID及别名/origin/协议/接口/结果组合分析，UTC/Local小时/日/月、三级分组、排序原生表格、分页及本页/全量CSV；默认系统本地时区，重置恢复本地，保留UTC选项。
- [x] 统计明细/汇总表单元格选择及Ctrl+C/右键复制，完整Unicode文本、键盘切列和焦点框；刷新清除旧目标，实际系统剪贴板端到端验收仍待完成。浅深Report局部合成原生滚动及NC条带，避免浅色中间帧；浅深主题纯合成位切换保留活跃复制目标，原生range／visible语义不变。
- [x] 历史保留策略保存应用、占用查询、预览清理；schema v1→v2，HMAC身份保持。
- [x] 固定只读reader隔离报表与writer，4个读任务/60秒预算及取消；大查询不阻塞写入。
- [x] 日志实时/文件AND组合筛选、暂停冻结、分页/follow、安全清理与轮转；GUI默认本地时间及偏移、本地日期筛选，落盘UTC epoch不变。查看区横纵条仅在对应内容溢出时显示，正文／字体／尺寸变化自动隐藏／恢复，保留选区／底右位置且捕获期间延后显隐；支持120–720 DIP手动拖拽/方向键、Home恢复240 DIP，运行内保留；键盘调整后异步揭示焦点，跨页取消旧滚动捕获；浅深Edit/Report/页面滚动条鼠标跟踪适配避免原生系统modal大条，仅高对比度回退。
- [x] 概览四项大卡片、请求/错误及上下载两张实时趋势图、真实活动/错误/流量/会话指标、重启/复制Base URL/打开目录；图例按“标记＋名称＋最新速率”测宽紧凑排列，自动二进制流量／十进制计数速率单位、1／2／5留顶刻度，绘图区固定88 DIP避免未知提示跳动；STATIC文字更新不再同步擦除重画，局部有界整帧呈现，平直段简化标记且保留高对比度方形；有界120点/120秒、缺口不补零、隐藏抑制刷新。
- [x] 窗口几何/最大化记忆及工作区校正，关闭到托盘/退出；磁盘I/O全部后台。
- [x] 一致快照目录迁移、源保留、目标备份与失败尽力回退；静态CRT打包及PE检查。

## 本批review

2026-10-31提交前复核：三个独立只读会话覆盖设置／绘制／统计及工具，确认并修复全局导入／导出忙碌时已显露Key未重掩码的P2；四项门控回归先fail后pass，modal语义保持。反向选区静态疑点经原生对照排除：首版EM_SETSEL探针前提不成立，改用Shift+Left构造后现有显隐及native均通过，未误改生产代码。switch／status两种打印消息改为独立清屏和像素／clip／DC断言。最终定向只读复核无确定新增问题；完整串行509项通过，详见[本批记录](2026-10-31-03-precommit-ui-review.md)。

2026-10-31统计窄屏：主agent修正固定34 DIP状态区和Debug长句，agent提供纯中文格式及工具契约更新。只读review未发现确定新增生产P1／P2；指出新增测试把隐藏结果HWND与已滚动可见坐标混比，按既有文档／viewport坐标契约修正而不改生产行为。实际字体／窄宽／异步结果／隐藏页／管理和session tail／GDI回归21通过；旧固定592断言同步为状态底边关系。详见[本批记录](2026-10-31-02-history-status-layout.md)。

2026-10-31三个分域agent实施scroll／settings／小图形，主agent整合About／生命周期／viewport／跨模块测试及文档；末轮只读review发现数字输入query／延迟剪贴板回调后使用旧attached快照的P2，以共享owner及逐调用重查修复。17个同步销毁场景先fail后pass，另一agent定向复核无确定新增P1／P2。新增原生测试捕获125%／175%参数下port hint与输入重叠1px，按实际像素底边＋间距修复，未放宽重叠断言；生命周期／完整字串／选区／禁用、边缘混色／clip／DC／GDI及浅色跟踪覆盖。普通Cargo一度误用旧9项测试产物，另agent源码快照确曾共享target，为明确干扰因素但根因未完全隔离；全部最终证据使用CARGO_INCREMENTAL=0重新编译，不把旧9项或快照记为整合通过。详见[本批记录](2026-10-31-01-ui-input-scroll-polish.md)。

2026-10-30提交前独立复核：三个新只读会话覆盖日期、概览／设置、工具文档；生产改动无确定新增P1／P2，发现并修复工具raw-rates预算不符新RateScale的P2。另一新独立会话追加确认ClearType彩边／marker冒充正文两项工具P2，修为精确色形状检测＋排除marker后的正文墨迹，最终复核无确定新增问题。完整矩阵与GUI分开后96正例／352反例及Debug概览55.37秒通过，458项串行／严格Clippy再次通过；两次120秒超时保留不作成功。modern统计纠正为168次捕获／165个PNG，既有验收边界不变。见[复核记录](2026-10-30-02-precommit-review-release.md)。

2026-10-30三处UI优化：并行落实设置限宽／月历边沿，主agent定位STATIC同步擦除及整帧呈现。独立review发现的月历打印children裁剪、概览短高对比度虚线／PRINTCLIENT flags共3项P2已修复并加回归，定向复核无确定新增P1／P2。初始日期reference DC不一致及共享测试旧填满契约失败均保留并修正；最终458项串行通过。独占桌面复现宿主3像素客户区空隙白边1,676→0，calendar内容不变；不是NC，旧hidden NC0／0不作复现证据。Debug modern／概览及四框暗色翻月通过限定scope，Release未重建、实际高DPI／多屏／读屏及连续刷新视觉仍待验收。见[本批记录](2026-10-30-01-ui-width-calendar-chart.md)。

2026-10-29提交前复核：按用户授权再次以三个独立只读会话分域review当前UI差异及测试，未发现确定新增P1／P2，无需改生产代码。主agent复核工具、文档与推送范围；本次fmt／全目标check／448项串行／严格Clippy、三工具ValidateOnly及编辑器诊断通过。表头实际模态拖拽、状态栏tooltip、复杂clip像素及真实高DPI／高对比度／读屏仍为覆盖限制；未重跑GUI，不将前批截图改写为本批验收。见[复核记录](2026-10-29-02-precommit-ui-review.md)。

2026-10-29明暗精修：三个子agent按主题／导航、原生控件、图表并行改代码，主agent整合状态栏／圆角；分域交叉只读review未发现确定新增P1／P2。review提示的4 DIP工具文案和状态栏实际draw测试已补齐。主题／Tab独立Rc展示不持应用State，新增重入销毁／原生焦点／打印DC回归；无新依赖或配置／代理语义变更。首轮旧圆角DPI期望和概览彩色徽标断言失败已分别按新几何／中性视觉契约修正并保留失败证据；最终448项测试、严格Clippy及Debug／最终静态CRT Release双主题四页矩阵通过限定范围，不替代物理高DPI／多屏／真实高对比度／读屏。见[本批记录](2026-10-29-01-fluent-theme-polish.md)。

2026-10-25月历翻月：screen-only复现COMCTL32 TimerProc绕过tint后持续变白；只对dark calendar改原生六色，日期框仍themed/tint。定位并修复calendar child大于DropDown父容器导致的历史裁切，独立review另发现放大后工作区越界并修复。实图发现TITLEBK兼星期前景的同色隐形，再以MCHT_CALENDARDAY七列ink先fail后修复，最终review无剩余确定颜色／布局P1/P2。Debug／Release四框最终16物理＋4稳定图/选日通过；完整日期启动超时仍未验收。

2026-10-25统计暗色滚动白帧：native LVM_SCROLL返回前测得系统浅灰，定位为“原生先画／暗色后盖”中间帧。Report局部COMPOSITED＋短持有DC，同DC补画后呈现，NC region排除横纵条／交角而保留native边框；不禁绘、不改range/visible。独立review发现的菜单DC／单元格清目标／样式交错P2均修复加回归，最终限定复核无剩余确定P1/P2。物理48个中间样本均暗色；Release统计GUI在初次切Tab消息超时，完整成品验收仍未完成。

2026-10-25日志按需滚动条：基于真实EDIT范围探测，不手工测量正文或伪造range；隐藏轴重新启用后查询，H→V→H消除条带互相挤占。独立review发现并修复刷新中断capture，回归另捕获底右clamp／重入禁绘generation及经典主题other-axis不一致；尺寸显隐移至deferred布局后的合并消息，原失败证据保留。最终独立只读复核未发现本范围确定新增P1/P2。12项新回归覆盖捕获／端点／重入／同步销毁／隐藏后暗色恢复，不等同全部系统验收。

2026-10-25日期边角修复：根因为原生矩形边框被圆角region裁掉却未绘制连接圆弧；与Edit一致排除日期框，仅清理自有旧region、保留外部region。原生资格／清理／样式／尺寸／DPI及600轮GDI回归扩展到Date，着色测试接入实际rounded调用。独立只读复核未发现本范围确定新增P1/P2；无region断言和四角证据不冒称逐像素连续性验收。Release浅色聚焦外框／四角实图已查看，定向浅深／缩放regressions通过；完整日期专项仍失败，详见最近验证。

提交前3个独立会话分域review发现并修复1项P1＋6项P2：分段hover同步销毁UAF、NCACTIVATE(-1)禁止重绘、日期局部打印flags、日期空图假通过、Unicode菜单助记键、菜单工具启动身份／清理及菜单纯黑假通过。修复后定向复核未发现新的确定问题；399项全量分段测试、严格Clippy通过。最新菜单GUI纯黑捕获和Release浅色月历翻月页脚缺失仍失败待隔离，不因native测试／Debug复跑通过改写为已验收。

此前深色日期框／月历保留视觉主题和原生布局，仅重映射有界原生栅格；代码自查及原生字体／状态／DC／资源回归通过，无独立review通过声明。翻月空帧经未着色原生对照确认为动画期，测试等待真实消息处理后验证稳定帧，不更改原生选日。Debug／Release四日期框定向截图和交互通过，不等同高DPI／读屏／真实高对比度完整验收。

此前待提交独立review的4项P2已落实：共享viewport换页前仅清理自有跟踪，键盘增高grip后异步揭示焦点，日志工具异步缩放及完整后代布局等待、原始handle身份核验下的空Path有界就绪。4项新增原生回归含取消中同步销毁宿主；独立复核补齐宽度only等待条件，最终未发现该修复范围新增确定P1/P2。早轮Debug真实按住repeat截图出现白条，未确认完整根因；后续通过不将这次失败改写为成功，视觉全验收仍未完成。

此前深色标题菜单已自查并通过原生绘制／状态恢复和Debug／Release定向GUI回归；修正MIIM_STRING查询清除其他字段、空快捷键测量及测试工具退出同步。未声称独立review（子agent连接不可用）。菜单命令／确认流程未改，无新增依赖；离屏图片不替代物理／高DPI／高对比度／读屏验收。

此前普通打开自动启动保持既有默认及安全边界；独立review发现并修复状态过滤吞错、工具同步隐藏阻塞及首份释放快照漏判。新增回滚真实错误/开关草稿/启动worker回归；工具对自建socket清除并验证继承位，APP存活时目标占位端口实际释放后才观察无重试。最终复核未发现本批新增确定P1/P2，定向HWND与有界监听采样不等同完整系统验收。

此前概览图例紧凑排版经独立只读复核未发现确定回归；真实字体/最窄面板/长标签/未知原因行及DC/GDI回归通过，Timeline采样、精确元数据和其他页面布局未改。工具的完整矩形裁图与外部字体契约预算明确区分实际观察，最小窗口跳过完整PNG不冒称全图验收。

此前日志高度/暗色跟踪及输入框no-redraw经独立只读复核：Report巨大位移同步通知取消后继续动作、grip高对比度着色均已修并加回归，最终复核未发现本批新增确定问题。暗色交互保留真实native范围、内容、选区及键盘/滚轮；取消使用capture/generation与RAII清理。前批 `input_frame`几何通知立即overlay的P2已以depth/redraw守卫、延后NC绘制和真实client-region对照回归修复；真实IME组合/读屏/多屏及Release无遮挡视觉仍待验收。

此前状态栏留白与滚动宽度缓存修复保持回归，末尾16 DIP及实际宿主宽度一致。

此前修复的旧草稿回写保留策略、大报表写入饥饿、偏好读取失败不重试保持回归。此前10万条高基数报表/导出期间各1500条2ms记录全写入、queue_full=0；约11–17秒报表耗时不代表本批重新执行完整性能验收。

## 最近验证

- 提交前复核（2026-10-31）：最终fmt／CARGO_INCREMENTAL=0全目标check／严格Clippy／完整 `cargo test --locked --offline -- --test-threads=1` 通过，356库＋153集成＝509项，另4项physical默认ignored，doc 0项无失败；modern／ui-regressions ValidateOnly、diagnostics及diff空白检查通过。四项Key门控先fail后pass；首版选区探针及原生对照失败已隔离，真实Shift+Left基线及四项回归通过；格式首轮失败已修正。敏感模式无确定秘密命中，detect-secrets本次不可用。未重建Release或运行GUI／physical，02批Release不包含本批重掩码修复，不继承旧截图为本批成品验收；见[本批记录](2026-10-31-03-precommit-ui-review.md)。

- 统计窄屏（2026-10-31）：fmt／CARGO_INCREMENTAL=0全目标check／严格Clippy通过；统计21项、共享窗口11项分段补齐、65非UI库／153集成及doc通过。共享窗口首轮日志焦点测试GetFocus为空失败，独立串行复跑通过，未改日志或放宽断言。实际字体参数96／120／144／168／192、800／1000／1400宽度、全状态文字测高／间距／底部16 DIP／草稿选区／异步重排／GDI覆盖。测试＋构建组合命令触及300秒，集成/doc已过而Release未完；忙碌保护拒绝后空闲时单独构建87.9秒通过。最新静态CRT Release7,993,856字节、SHA256 `3766D930…271FBAA`，x64 PE／系统DLL及版本退出0通过；版本2026.10.18、dist未改。最新Release modern完整限定矩阵两次通过（297.99／289.76秒）：各96 DPI、16四页／24设置／24统计子视图、168次OFFSCREEN捕获／165 PNG，中文状态／状态底边相对布局／extra独立范围／草稿选区／配置不变／正常退出通过；compact状态实高42px、chart320／table600，非原固定34／312／592。未运行物理／高DPI全验收，见[本批记录](2026-10-31-02-history-status-layout.md)，不继承上一批成品验收。

- 浅色滚动／设置精修（2026-10-31）：fmt／CARGO_INCREMENTAL=0全目标check／严格Clippy／doc通过。库分组335项通过（20设置＋11共享窗口＋239其他UI＋65非UI），另4项physical默认ignored；153项集成通过。整库命令触及240秒超时，改分组补齐，不改记该命令成功；初轮设置18／19的1px重叠失败已修，重叠修复19／19通过，重入修复后最终20／20，fmt／check／严格Clippy再次通过。首版静态CRT Release（7,995,904字节）modern 96 DPI 16四页／24设置／24统计子视图、168次OFFSCREEN捕获／165 PNG、15 Edit、7数字框拒绝和空Key显隐／配置不变／正常退出通过，284.69秒；该完整矩阵早于最终重入加固。最终静态CRT Release重建191.726秒、7,997,440字节、SHA256 `883C1326…A48E8F`，x64 PE32+／系统DLL及版本退出0通过，版本仍2026.10.18，dist未改。Dark物理48样本通过，Light两次因同一外窗遮挡被拒绝，不再重试／放宽guard；最终成品About实际显示`2026.10.18 · Release 构建`，浅色Key重掩码／六网络非法数字拒绝／配置不变／正常退出定向通过，4张OFFSCREEN图；未重跑完整modern。辅助脚本早轮关闭About用错按钮ID1（实际2），改原生WM_CLOSE后通过；原生主窗口caption读空保持未验收。细节见[本批记录](2026-10-31-01-ui-input-scroll-polish.md)，不据此宣称完整系统验收。

- 最新Release及推送（2026-10-30）：独立复核并修复工具3项P2后，UI代码提交 `ec7295f` 已正常推送origin/master，远端哈希确认一致；干净源执行静态CRT／编译路径映射的 `cargo build --release --locked --offline` 通过，166.049秒。`target/release/ai-relay.exe` 7,922,176字节，SHA256 `F5D47E6E…734F8AB`，x64 PE32+／仅系统DLL及 `--version` 退出0、stderr空通过；版本仍2026.10.18，未改dist／显式target历史产物。未重跑Release GUI，不等同全部Windows验收；后续仅文档补记不改运行代码，见[构建记录](2026-10-30-03-release-build.md)。

- 提交前复核（2026-10-30）：fmt／check／458项串行及doc／严格Clippy、diagnostics通过；三工具ValidateOnly通过，概览完整自检96正例／352反例76.74秒，修正后Debug GUI55.37秒、23次OFFSCREEN捕获／21本地请求／7 HTTP400／隐藏启停／GDI最终88／配置不变并正常退出，Complete=false。完整synthetic只在ValidateOnly执行，真实GUIguard不跳过。两次120秒外部超时保留；未重跑日期／modernGUI或physical，不继承前批为本次通过。该验证时点提交及Release尚待执行，后续实际结果已补[03构建记录](2026-10-30-03-release-build.md)；复核细节见[02记录](2026-10-30-02-precommit-review-release.md)。

- 三处UI优化（2026-10-30）：fmt／all-targets check／严格Clippy／305库＋153集成（458项）及doc通过，3项physical默认ignored；本批显式执行15帧翻月及6张outer前后物理取证通过。Debug四日期16screen-only＋4稳定打印／选日通过；概览21本地请求／7 HTTP400、23 OFFSCREEN图、隐藏／启停及6轮GDI最终88通过；modern 96 DPI 16四页／24设置／24统计子视图、168次OFFSCREEN捕获（165个独立PNG）／15 Edit草稿选区／配置不变及正常退出通过。真实STATIC文字更新draw／background 1／1→0／0、缓冲与直接绘图96／192像素对照通过；字体参数不替代真实缩放。最新构建为 `target/debug/ai-relay.exe`，本批未重建Release／dist，不解除旧验收限制。详细首轮失败、取证及工具范围见[记录](2026-10-30-01-ui-width-calendar-chart.md)。

- target清理（2026-10-29）：包级dev清理成功删除5,408个可重建文件；清理及静态CRT恢复时目录项9.412→4.679 GiB、文件标识去重8.095→4.431 GiB；收尾其他开发重新生成缓存后最后观察5.046／4.638 GiB，净回收估计3.457 GiB。6主产物及8,145个保留文件清理前后哈希一致；原逐项预览后置断言失败保留，独立保护内容复核通过，不宣传整条脚本通过。首轮Cargo忙保护中止；空闲后清理，不结束进程。清理前Release已被其他构建替换为动态CRT、来源未定位，随后恢复静态CRT构建约1分钟，最终EXE7,913,984字节，SHA256 `4E814BFA…0F6BD57`，x64 PE／系统DLL及Debug／Release版本退出0通过；静态哈希未重现前批值，不声称可重复构建。Debug／显式target符号、截图／报告／dist保留，不重跑全套或GUI。收尾另见日期／设置3个源码文件的新未提交修改，未触碰／回退或再次构建；当前Release未验证包含后续改动，见[清理记录](2026-10-29-04-target-cleanup.md)。

- 最新Release构建（2026-10-29）：基于已提交 `314ea76`，单次命令静态CRT／编译路径映射的 `cargo build --release --locked --offline` 通过（0.92秒），Cargo确认已有产物最新。`target/release/ai-relay.exe` 7,913,984字节、SHA256 `44EAFFB2…9C9AF7C` 与此前静态CRT成品一致；x64 PE32+／系统DLL导入及 `--version` 退出0、stderr空通过。版本仍2026.10.18，未改源码／依赖／profile／dist／显式target产物，未重跑全套或GUI，不解除既有验收限制。见[构建记录](2026-10-29-03-release-build.md)。

- 提交前复核（2026-10-29）：当前代码fmt／全目标check／448项串行（295库＋153集成，2项physical默认ignored）／严格Clippy再次通过；modern／overview／rounded的ValidateOnly通过，无GUI或Release重建声明。三个独立分域review无确定新增P1／P2；本地敏感模式扫描无确定敏感命中，detect-secrets本次不可用，不当作绝对秘密保证。见[复核记录](2026-10-29-02-precommit-ui-review.md)。

- 明暗精修（2026-10-29）：fmt／全目标check／严格Clippy／最终448项串行（295库＋153集成）及doc通过，2项physical默认ignored；新增27回归含状态栏实际绘制／省略／DC／GDI。Debug及最终静态CRT Release分别通过96 DPI modern 16四页／24设置／24统计子视图矩阵，各168 OFFSCREEN图、34局部对比度证据、15 Edit草稿／选区保留，配置字节不变并正常退出（206.72／204.37秒）。Debug概览21本地请求／7个HTTP400、真实速率／隐藏／启停／23图和6轮GDI最终88通过，最小完整图高于viewport两项保留限制，Complete=false。浅深图片已人工抽查，非逐图逐字或物理验收。最终Release 7,913,984字节，SHA256 `44EAFFB2…9C9AF7C`，x64 PE／系统DLL／版本退出0通过，版本仍2026.10.18，dist未改。见[精修记录](2026-10-29-01-fluent-theme-polish.md)。

- 月历翻月：fmt/check/严格Clippy/421项分段（268库＋153集成）及doc通过，2项physical默认ignored；本批月历另显式通过15张无print导航帧。四框×96/144/192字体参数的七列星期／日期ink、标题／页脚、native命中／minimum／父容器／边界／恢复及30轮popup GDI通过。Debug／最终静态CRT Release NavigationOnly四框各16screen-only＋4稳定打印、选日／配置不变／正常退出通过，无skip；计划0/50/350/1000ms记录实际采样时间，Complete=false。Release7,899,648字节，SHA256 `1496F414…BF005`，Debug/Release最新，dist未改。完整Debug工具仍启动Read text timeout；先前缺星期图不转记成功，见[翻月记录](2026-10-25-04-dark-calendar-navigation.md)。

- 统计暗色滚动：fmt/all-targets check/严格Clippy/全量418项（265库＋153集成）通过；1项physical默认ignored，另串行显式执行通过，横纵thumb／按住repeat／wheel／ENSUREVISIBLE共48个native返回前物理轨道样本均暗色。样式／region／no-redraw／菜单键／12组主题交错销毁／100轮GDI及复制回归通过。Debug与静态CRT Release／PE／版本通过；Release7,898,112字节，SHA256 `B3478B63…2C277`，dist未变。Release历史GUI在SelectTab同步消息超时、尚未查询，未声称完整通过，工具退出后无残留实例。见[修复记录](2026-10-25-03-dark-report-scroll.md)。

- 日志按需显隐：fmt/all-targets check/12新原生回归＋2项日志高度回归/258库＋153集成（411项分段）/doc/严格Clippy/Debug与静态CRT Release／PE／版本检查通过。Release7,894,528字节，SHA256 `E39454EF…C4790B`，包含日期及日志未提交修复，版本不变，dist未改。Debug96 DPI日志GUI完整定向矩阵通过（45.7秒、22项显隐、44帧、亮白0、无skip、配置／JSONL不变并正常退出）；仅表示该工具scope完成。Release已取得浅深fit／clear无条物理图并查看，但完整工具两轮分别在宽度后瞬时hidden读取及grip无位移高度断言失败，未标完整通过；有界等待显隐完成已补入工具，不扩大产品验收结论。首GUI在grip布局137/312失败，根因为SIZE内NC改动干扰batch，已移至排队更新，失败报告保留。见[日志显隐记录](2026-10-25-02-log-scrollbar-visibility.md)。

- 日期边角：fmt/all-targets check/16项日期与圆角专项/246库＋153集成（399项，分段）/doc/严格Clippy/Debug及静态CRT Release／PE／版本检查通过。最新Release含未提交日期修复，7,889,920字节，SHA256 `BA824AB3…AA2B4`，版本仍2026.10.18，dist未改。三个修改脚本ValidateOnly通过；Release96 DPI regressions定向功能通过、Complete=false（目录状态图被Explorer遮挡），四日期框浅深／缩放均无region，20轮GDI恒94。日期专项Debug两次启动Read text timeout；Release取得开始日期浅色闭合／聚焦物理外框与四角，随后月历捕获Foreign/missing HWND refused，完整专项未通过，无重试或放宽断言掩盖。见[边角修复记录](2026-10-25-01-date-picker-border.md)。

- 2026-10-20 Release构建（历史）：源码HEAD `7de0eda` 的静态CRT `cargo build --release --locked --offline` 通过，最终Cargo耗时0.39秒；`target/release/ai-relay.exe` 为7,889,408字节，SHA256保持 `5278316C…33846`，x64 PE32+／无额外CRT导入检查及 `--version` 退出码0通过。最近提交仅文档，Cargo确认产物新鲜；版本仍为 `2026.10.18`，未改源码／profile／dist／显式target旧产物，未重跑全套或GUI验收。见[构建记录](2026-10-20-04-release-build.md)。

- target清理／Release核验：HEAD 5ec6c36静态CRT `cargo build --release --locked --offline` 0.51秒确认新鲜、EXE哈希保持5278316C…33846；包级dev清理5,408文件，目录项9.190→4.497 GiB、硬链接去重8.013→4.251 GiB（估计释放3.762 GiB）。当前Debug／Release及旧显式target共6个EXE/PDB哈希不变，2,472证据文件聚合哈希不变，Debug／Release `--version`均通过。未重跑全套、未改源码/profile/dist，最新程序是 `target/release/ai-relay.exe`，显式target和dist仍旧。见[清理记录](2026-10-20-03-target-cleanup.md)。

- 提交前review：fmt/check/246库＋153集成（399项）/严格Clippy/Debug与静态CRT Release／PE检查通过；7工具ValidateOnly与Debug modern 16／24／24矩阵通过。加强版日期Debug84离屏内容通过、物理6通过6遮挡跳过／GDI恒99；首轮Debug及最终Release因浅色翻月页脚缺失失败。菜单PrintWindow纯黑被新增守卫拒绝，未声称最新菜单视觉复验通过。原失败报告保留、秘密规则仅既有虚构fixture、未加入配置／target／dist。见[提交前review记录](2026-10-20-02-precommit-review.md)。

- 深色日期框／月历：最终fmt/all-targets check/9项日期（4新增）/240库＋153集成（393项，分段无skip）/严格Clippy/Debug及静态CRT Release与PE检查通过。两构建96 DPI浅→深→浅四日期框各84离屏＋12物理图均通过，聚焦／展开／翻月／原生PageDown/Right/Enter／清空及ISO值不变，配置哈希不变并正常退出；8轮GUI GDI恒99、30轮原生精确资源回归通过。144/192为字体参数测试，不代表实机缩放；Complete=false。最新Debug/Release更新，dist未变，见[日期修复记录](2026-10-20-01-dark-date-picker.md)。

- review四项修复：最终fmt/all-targets check/171项UI/236库＋153集成（389项，库与15集成目标分段无skip）/严格Clippy通过，最新Debug/静态CRT Release与PE检查通过。Debug/Release96 DPI实测切页capture立即释放、120偏移不被旧40覆盖；键盘增高grip仍456–468可见、offset66→90。自有阻塞GUI线程异步请求约1ms返回，恢复后16–19ms确认完整矩形。最终两构建日志完整功能矩阵Passed=true/Complete=false，各33张有效held条带/271,524采样亮白0，均有3处遮挡跳过；早轮白条3067像素失败保留，普通测试不替代视觉全验收。日志工具Path/resize复核完成，配置/样本不变且正常退出，dist未变。见[review修复记录](2026-10-19-08-review-interaction-fixes.md)。

- 深色标题菜单：fmt/all-targets check/5项新增菜单回归/236库＋153集成（389项，分段补齐）/严格Clippy/Debug及静态CRT Release与PE检查通过。整条全量命令触及240秒上限，236库已通过后终止，随后15集成目标无跳过单独通过。两构建96 DPI浅→深→浅各21张OFFSCREEN原生根／子菜单图、F10/Alt+M/Alt+Space/方向键切页/V→O助记/启禁radio/配置不变/正常退出通过；10轮GUI GDI无逐轮增长，进程内30轮严格相等。Release GUI与native测试并行首轮超时，串行复跑通过；Complete=false。最新Debug/Release已更新，dist未变。见[菜单修复记录](2026-10-19-07-dark-title-menu.md)。

- 普通打开自动启动：fmt/all-targets check/6项真实worker与8项设置/380项全量串行/严格Clippy、Debug与静态CRT Release及PE检查通过。两构建均通过7组/10个私有fixture：缺省/显式开关、首次无配置、无效/空上游、占用失败/释放后2500ms不重试、重复启动只Show及LAN手动确认保护；配置哈希不变、全部正常退出。初期工具Path就绪及Framework socket继承问题经取证修正，最终AutomatedScopeComplete=true、Complete=false；无真实Key/Startup/剪贴板/系统代理变更。最新Debug/Release已更新，dist未变。见[自动启动记录](2026-10-19-06-proxy-auto-start.md)。

- 概览紧凑图例：fmt/all-targets check/14项dashboard（2新增字体/几何/绘图资源）/372项全量串行/严格Clippy、Debug与静态CRT Release通过，PE导入检查通过。两构建概览定向96 DPI三尺寸/浅深/本地21请求与7个HTTP400/速率字节/隐藏冻结/启停重启/配置不变通过；默认/放大窗口完整图表离屏裁图通过，6轮GDI89不变。最小外框完整图高于视口及首样本窗口错过均明确跳过，Complete=false；实际高DPI/多屏/物理屏幕/读屏待验收。最新Debug/Release已更新，dist未变。见[紧凑图例记录](2026-10-19-05-compact-overview-legends.md)。

- 日志高度/暗色滚动：fmt/all-targets check/158项UI（17滚动、12输入框专项）/370项全量串行/严格Clippy及Debug/静态CRT Release通过；PE导入检查通过。Debug默认240、120–720拖拽/Home/Escape/切页/theme/resize、Edit横纵/host纵thumb与真实timer repeat功能通过，33张有效按住物理条带图271,524采样亮白像素0；3张后续遮挡跳过，Complete=false。Release完整交互通过，但Explorer遮挡无有效held图，视觉未验收。Debug16/24/24现代矩阵通过；5项物理残影差异0、GDI20轮恒91，目录图跳过。旧input-frame P2已修；无新依赖/数据迁移/Key/剪贴板写入，最新Debug/Release已更新、dist未变。见[本批记录](2026-10-19-04-log-resize-dark-scrollbars.md)。

- target空间清理：包级限定dev清理删除7,172个可重建文件，文件条目占用14.987→4.437 GiB；按硬链接文件标识去重减少约9.421 GiB。当前Debug/Release及显式target旧版EXE/PDB哈希不变，全部GUI验证证据哈希不变，Debug/Release `--version`均通过。保留依赖/Release缓存和截图，首次项目编译/测试需重建，未改profile/源码、未重跑全套测试。见[清理记录](2026-10-19-03-target-space-cleanup.md)。

- 状态栏留白：fmt/all-targets check/139项UI/最终351项全量串行/严格Clippy通过；首轮既有proxy_usage断言收到504非502，定向及全量复跑通过，未改代理且未确认波动根因。Debug与静态CRT Release已更新，PE导入检查通过；两构建均通过96 DPI的16四页/24设置/24历史子视图TOP/BOTTOM/cache矩阵，末尾16 DIP及实际客户区右边距24 DIP。物理回归五项残影差异0、20轮GDI恒91；目录状态因Explorer遮挡跳过，Complete=false。未更新dist/版本/标签，实际高DPI/多屏/管理GUI与前轮P2仍待完成。见[底部留白记录](2026-10-19-02-status-bar-spacing.md)。

- 原生组件精修：fmt/all-targets check/347项串行测试/严格Clippy/Debug及静态CRT Release构建通过，`target/release/ai-relay.exe`已更新；x64 PE/无额外CRT导入检查通过。Debug/Release各通过96 DPI四页×浅深×800×680/1000×820共16组modern离屏矩阵（2039/宿主2400旧断言已修正），保留输入/选区/switch/pending及配置不变。96 DPI regressions已执行项通过：38个Edit无region、浅深矩形边框/四SVG图标匹配、分类及两尺寸缩放/缓存返回物理残影差异0、20轮GDI恒91；960×740及目录状态截图因遮挡跳过，Complete=false。未改版本/标签或dist；高DPI/多屏/真实高对比度/IME/读屏仍待验收。见 [组件精修记录](2026-10-19-01-native-ui-polish.md)。

- 本地Release重建：静态CRT `cargo build --release --locked`通过，更新`target/release/ai-relay.exe`（包含默认本地时区及单元格复制）；x64 PE/无额外CRT依赖检查和53条本地mock历史GUI冒烟通过。未改版本/标签、未打包或覆盖dist；剪贴板、干净系统和完整GUI验收限制不变。见 [Release重建记录](2026-10-18-24-release-rebuild.md)。

- 统计单元格复制：新增独立ListView子类，复用Unicode剪贴板写入，支持点击/Ctrl+C/原生右键菜单、键盘切列、浅深/系统色边框及重建失效；无新依赖。fmt/all-targets check/330项串行测试/严格Clippy/Debug构建通过。历史GUI默认及保护剪贴板模式均运行，明细/汇总点击、导航、菜单出现/启禁/取消、刷新失效通过；非文本剪贴板触发保护跳过真实写入，Ctrl+C及菜单命令执行未验收。Release/dist未更新。见 [单元格复制记录](2026-10-18-23-history-cell-copy.md)。

- 默认用户时区：历史查询/首次打开/重置默认Local，日志显示本地时间及偏移并按本地整日筛选，UTC存储不变；无新增依赖或数据迁移。fmt/all-targets check/318项串行测试/严格Clippy/Debug构建通过；历史53条本地mock与日志96DPI专用GUI冒烟通过。综合modern脚本通过本地默认/重置/切换及pending，随后在无关的设置分类2039旧ComboBox假设失败，不能称全脚本通过。Release/dist未更新，多时区实际系统/高DPI本批未验收。见 [本地时区记录](2026-10-18-22-local-time-default.md)。

- 日期Release `2026.10.18`：MIT及系统杂项忽略规则已整合；Cargo/清单日期版本一致，第三方原始许可文本134项已收集并随包附带。fmt/all-targets check/312项串行测试/严格Clippy及x64静态CRT Release构建通过；解压EXE版本与5文件白名单/CRC/许可匹配通过。Release 96DPI定向冒烟布局/输入/图标离屏/GDI通过，但其他程序遮挡导致物理截图/残影比较跳过，Complete=false。发布包与标签详见 [日期发布记录](2026-10-18-21-date-release.md)；未推送，不等同正式全验收。

- 许可证：用户选定MIT，新增根目录LICENSE（2026 rachpt）、Cargo SPDX元数据、README/便携说明和导航资源授权说明；发布脚本检查并附带LICENSE。第三方notices和完整许可审核仍未完成；既有dist包未重建。见 [MIT记录](2026-10-18-20-mit-license.md)。

- GitHub公开准备：4个既有提交author/committer已统一为 `rachpt <rachpt@126.com>`，保留内容/提交拓扑，本地Git身份一致；无远端、无推送。仓库外受限备份及bundle验证后移除8个忽略的旧参考脚本/日志/PID。全部264个历史blob经本地detect-secrets（禁联网验证）与精确旧部署值/隐私规则交叉检查，命中均为已核对的虚构测试或清单常量；未发现真实凭据，非绝对安全保证。许可证选择、第三方notices/打包仍待维护者处理。见 [清理记录](2026-10-18-19-publication-cleanup.md)。

- UI回归修复与SVG导航：修正统计子视图/会话坐标、日志848 DIP预布局、批量事务后完整异步重绘；Edit恢复原生矩形边框，保留其他控件圆角；原创SVG离线生成多DPI遮罩内嵌，运行时无新依赖。fmt/check/312项全量串行/严格Clippy/Debug构建通过；96DPI浅深GUI短矩阵完成，分类/缩放残影物理差异0，38个Edit无裁剪、图标像素匹配、20轮GDI均91，目录按钮实际临时Explorer验证通过。未执行实际高DPI/多屏/IME/读屏；当前构建是 `target/debug/ai-relay.exe`，旧 `ai-relay-next.exe` 和发布包未更新。见 [本批记录](2026-10-18-18-ui-regressions-svg.md)。

- 缩放/图标修复：拖动结束强制整页 `RedrawWindow` 清除旧帧；标题栏不再占用小图标，客户端新增☰菜单，任务栏/标题栏保留应用图标；新增 300 项全量串行测试通过。旧实例锁定主 EXE，最新包为 `target/debug/ai-relay-next.exe`，新二进制的物理拖拽/任务栏尚未实机复核。见 [缩放与任务栏图标记录](2026-10-18-17-resize-taskbar-icon.md)。

- UI 交互性能：切页改为页面布局缓存 + 批量移动/显隐，滚动只移动可见控件并批量提交；同步消息测量切页约14.7ms/最大30.7ms、单行滚动约6.3ms/最大13.9ms，fmt/check/299项全量串行/严格Clippy通过。同步清理 target 旧验证构建/截图及 incremental，释放约59GB（剩余约9.7GB）；保留当前 debug/release。见 [交互性能记录](2026-10-18-16-ui-interaction-performance.md)。

- 程序标识：客户端头部增加应用图标 + `AI Relay` 名称，四个Tab右移并保持最小宽度内完整；fmt/check/299项全量串行/严格Clippy通过，浅/深色临时debug截图已检查。见 [品牌头部记录](2026-10-18-15-brand-header.md)。

- 紧凑外框：顶部Tab紧贴标题栏并加图标，深色滚动条改为约6 DIP胶囊滑块，底部状态栏改为40 DIP单行并加状态图标；fmt/check/298项全量串行/严格Clippy通过，浅/深色临时debug截图已检查，未做多屏/物理输入/读屏验收。见 [紧凑外框记录](2026-10-18-14-compact-chrome.md)。

- 滚动残影：页面布局先按文档坐标定位、`arrange` 再移入视口时缺少宿主失效，导致 48 DIP 中间帧残影和重复绘制；已改为布局不立即重绘、最终统一失效，新增原生 update-region 回归。fmt/check/297项全量串行/严格Clippy通过；临时 debug 实例切换分类/滚动/调整大小后未见重复行，仍不等同完整 GUI 验收。见 [滚动残影记录](2026-10-18-13-viewport-ghosting.md)。

- 模块拆分：`controller` 拆为 types/results/history/session/worker，`history`/`settings`/`logs` 页面按布局、筛选、管理、事件和草稿拆分，测试移入各目录 `tests.rs`；fmt/check/296项全量串行/严格Clippy通过，未运行GUI实机验收。见 [模块拆分记录](2026-10-18-12-module-splits.md)。

- 标题栏☰菜单：原生系统菜单插入文件/代理/视图/帮助，F10/Alt+M及原生Alt+Space；保留原生窗口边框与任务栏图标。all-targets check、85项UI/296项全量串行、严格Clippy通过；最后显式单击路由后fmt/Clippy/构建和96DPI专项GUI通过，30次主题GDI79→79。仅定向HWND自动化，不代表物理输入/多屏/读屏。见 [标题菜单记录](2026-10-18-11-title-menu.md)。

- 紧凑页面：六项设置导航、默认800×680/最小800×560 DIP、裁剪滚动/独立页位置/Tab揭示、日志日期picker与类别分段、暗色原生滚动条。fmt/check/292项全量串行/严格Clippy通过；最后按钮语义色调整后构建/Clippy和96DPI三档尺寸浅深GUI已执行项通过。日志主区白条采样为0，30次切主题GDI79→75。底部动态状态长文与受保护剪贴板复制未验证，报告Complete=false；详见 [紧凑页面记录](2026-10-18-10-compact-pages.md)。

- 小圆角：按钮、单行Edit、闭合Combo和日期框6 DIP区域；高对比度恢复。隔离快照278项全量串行通过，去掉Edit非客户区补画后67项UI测试/严格Clippy/构建及200% DPI最终GUI通过；100次theme+resize GDI恒81，输入/选区/原生caret状态/下拉/月历/查询未受影响。Edit保留原生边框，非逐像素完整圆弧保证。见 [圆角记录](2026-10-18-09-rounded-controls.md)。

- 概览仪表盘：隔离快照275项全量串行测试通过，最后排版修正后64项UI测试、严格Clippy、构建及200% DPI浅/深最小/放大GUI冒烟通过；实测请求/错误/字节及速率增量一致，隐藏冻结/返回更新、停止累计保留、配置不变。详见 [仪表盘记录](2026-10-18-08-overview-dashboard.md)。

- 日期选择专项：原生月历、严格ISO/清空/闰日、真实键盘选日、显隐/切页/时区保留、pending冻结与配置不变通过；200% DPI浅深模式四个弹窗尺寸与截图完整性通过。弹窗保留系统视觉主题（暗色下允许浅色），详见 [日期控件记录](2026-10-18-07-calendar-date-picker.md)。最终隔离快照263项全量串行测试、check、Clippy及构建通过。

- 格式、all-targets check、263项全量测试、严格Clippy通过。
- 历史、日志、窗口、代理、设置、主题、CLI和现代UI冒烟通过；静态版历史/日志/窗口亦通过。
- 实机DPI192/200%及浅深物理截图已检查，不能替代多屏/读屏。
- 全部临时目录、本地mock和虚构凭据，无真实模型计费、不改真实Startup/防火墙/系统时区。
- 并行展示批次的历史事实见02–05记录；其隔离测试不作为全工作区证明，当前以上为统一整合结果。

## 未完成与验收限制

- [ ] 本批统计状态实际150%／200%、多屏DPI／读屏／真实系统高对比度及物理输入验收；native字体／打印参数回归不等同实屏。超长单词／任意导出路径水平完整性、真实内存／GDI耗尽尚未专项验证；测高读取16,384 UTF-16单元上限与极长导出状态来源是否可达仍待核实，不扩大本批状态纵向排布验收结论。
- [ ] 2026-10-31浅色滚动／抗锯齿小图形／Key与数字输入实际150%／200%、多屏、Win10／11差异、真实高对比度／读屏、IME组合及物理键鼠；参数／离屏／native测试不替代。分配或驱动失败只静态／超限回退测试，未实际OOM注入；不声称秘密内存擦除。Release及物理后续结果见本批记录，旧遮挡／超时保留。
- [ ] 本批限宽／图表／月历边沿实际150%／200%、多屏、Win10／11差异及真实系统高对比度／读屏；未进行整页连续刷新录像或资源耗尽验证。原生箭头保留经典浅色。Debug 96 DPI限定矩阵及物理外框取证不替代完整日期浅深、Release及系统验收。

- [ ] 本轮明暗精修实际150%／200%、多屏DPI、真实高对比度、Win10／11干净系统、IME／读屏／物理键鼠；96 DPI Debug／静态CRT Release modern矩阵及字体参数／离屏绘制通过不能替代。完整圆角旧工具GUI、日期／菜单／按住滚动专项本批未跑。

- [ ] 此前Release统计历史专项在初次SelectTab消息超时，原因仍待隔离；本轮modern统计查询／pending／明细及汇总矩阵通过不证明旧专项根因已修。Debug native真实Report的48中间帧证明仍不替代完整Release滚动专项、实际高DPI／多屏／高对比度／硬件输入／读屏验收。

- [ ] 日志按需显隐Release完整GUI复核：已见浅深fit／clear无条截图，完整工具分别遇到瞬时hidden读取和无位移grip高度断言失败；Debug完整矩阵通过不能替代Release。实际高DPI／多屏／系统高对比度／硬件指针及读屏未执行。

- [ ] 完整日期状态／浅深矩阵仍待完成；本批NavigationOnly暗色翻月已在Debug/Release四框通过，不等同完整模式。月份／年份高级视图、真实高DPI／多屏／高对比度／硬件输入／读屏仍待验收。

- [ ] 日期边角本批完整GUI专项：Debug启动读取文本超时、Release月历截图时句柄失效待隔离；目前仅浅色开始框闭合／聚焦物理边角取证及四框无region主题／缩放回归通过，不等同四框全部状态视觉验收。

- [ ] 本轮菜单PrintWindow纯黑帧与浅色月历翻月后今天页脚缺失的隔离取证；严格截图检查已拒绝，Release日期最新一轮未通过，不以单测或复跑覆盖失败。
- [ ] 深色日期框／月历实际150%/200%／多屏、真实系统高对比度／主题变化、月份／年份高级导航、物理键鼠／IME和读屏；96 DPI截图及字体参数回归不替代这些项。
- [ ] 深色标题菜单实际150%/200%／多屏、系统高对比度／主题通知、物理键鼠及读屏；96 DPI离屏与DPI绘图回归不替代这些项。
- [ ] 日志暗色滚动早轮Debug按住repeat白条的隔离取证及根因确认；本批已单独确认并修复Report原生滚动中间帧，48个实际桌面采样通过，但不推定历史Edit失败同根因。仍需完整Release／物理键鼠与高DPI/多屏/真实高对比度/读屏，普通native测试不替代。
- [ ] 状态栏tooltip自动悬停、图表悬停/完整无障碍；原生日历专项已在单屏200% DPI通过，其他DPI/真实高对比度/读屏仍待验收。
- [ ] 未保存草稿退出提示等成熟度细节、同维度多选/任意IANA时区（当前仅UTC/Local）。
- [ ] 真实目录迁移/自启联动端到端、登录/注销重启、跨用户/提权DPAPI/IPC。
- [ ] 受信任HTTPS成功握手、干净Win10/11、物理托盘/键盘、Explorer、多屏DPI/真实高对比度/读屏。
- [ ] 完整性能基线、24小时长稳、第三方许可证和签名审核。

请求模型仅64KiB内无压缩合法JSON且完整EOS；图表最多显示部分桶、精确数据在表格/文本。全量CSV去IP/ID但别名仍可关联，不是公开匿名数据。长读可能延迟WAL checkpoint；迁移需关闭成功证明，失败保留恢复路径，不保证跨文件系统断电事务。

## 下一步

- 项目MIT许可证及Cargo元数据已补齐；GitHub公开/二进制发行前第三方notices已随包附带，继续完成其人工法律及工具链许可审核；新提交重扫，旧部署Key建议轮换，不上传仓库外备份或未审查发布包。

1. 先重新构建包含03批Key禁用重掩码修复的静态CRT Release；现有 `target/release/ai-relay.exe` 仍为02批构建（7,993,856字节，SHA256 `3766D930…271FBAA`，版本 `2026.10.18`），不包含03批生产修复。再以新成品在无遮挡桌面复核连续图表刷新／月历宿主边沿、实际高DPI／多屏、日志grip／暗色按住滚动、tooltip／自启／跨用户／干净系统；本批Release构建／PE／版本及后续限定GUI结果见2026-10-31记录，不替代完整Release验收，dist／显式target历史产物未改。继续隔离完整日期工具Read text timeout及此前Release历史专项SelectTab超时；本轮modern通过不替代旧专项复现和原因确认。补regressions物理截图跳过项；仅在用户自行准备的空／纯文本剪贴板下复核Ctrl+C及菜单复制，不覆盖现有非文本剪贴板；继续复核不同系统时区。
2. 补退出草稿提示和辅助功能细节，优化大数据报表耗时。
3. 执行长稳/完整基准和许可证审核，再制定正式发布基线。
4. 对最新一体分段/按钮/switch/下拉/表格及Edit非客户区边框补150%/200%实际GUI、多屏DPI、真实高对比度与IME组合/读屏验收；纯几何/打印测试不替代实机，保持未执行项未完成。
5. 在独占桌面复现并隔离菜单纯黑捕获、浅色月历翻月页脚缺失；保持空图／前景断言严格，不用延时重试掩盖失败。
6. 在保持行为不变的前提下，继续按领域拆分 `controller`/`logs` 的测试与轮询代码，并补必要的模块边界回归。

## 最近记录

- [2026-10-31-03：提交前UI复核与禁用时Key重掩码](2026-10-31-03-precommit-ui-review.md)

- [2026-10-31-02：统计状态中文分段与窄屏自适应布局](2026-10-31-02-history-status-layout.md)

- [2026-10-31-01：浅深滚动条、平滑图标与设置输入布局](2026-10-31-01-ui-input-scroll-polish.md)

- [2026-10-30-03：提交推送后构建并核验Release](2026-10-30-03-release-build.md)

- [2026-10-30-02：独立复核、工具回归加固与提交前验证](2026-10-30-02-precommit-review-release.md)

- [2026-10-30-01：分段限宽、深色月历白边与概览稳定刷新](2026-10-30-01-ui-width-calendar-chart.md)

- [2026-10-29-04：清理开发中间文件并核验保留产物](2026-10-29-04-target-cleanup.md)

- [2026-10-29-03：构建并核验最新Release](2026-10-29-03-release-build.md)

- [2026-10-29-02：明暗UI精修提交前独立复核](2026-10-29-02-precommit-ui-review.md)

- [2026-10-29-01：明暗主题与 Windows 原生组件精修](2026-10-29-01-fluent-theme-polish.md)

- [2026-10-25-04：修复暗色月历翻月后变亮](2026-10-25-04-dark-calendar-navigation.md)

- [2026-10-25-03：修复暗色统计表滚动期间的白色条带](2026-10-25-03-dark-report-scroll.md)

- [2026-10-25-02：日志查看区按内容溢出显示滚动条](2026-10-25-02-log-scrollbar-visibility.md)

- [2026-10-25-01：修复日期框圆角裁剪导致的断边](2026-10-25-01-date-picker-border.md)

- [2026-10-20-04：构建并核验最新Release](2026-10-20-04-release-build.md)

- [2026-10-20-03：清理可重建开发产物并核验最新Release](2026-10-20-03-target-cleanup.md)

- [2026-10-20-02：提交前独立review与原生绘制回归修复](2026-10-20-02-precommit-review.md)

- [2026-10-20-01：修复深色日期选择框与弹出月历](2026-10-20-01-dark-date-picker.md)

- [2026-10-19-08：修复待提交 review 的四项交互与冒烟问题](2026-10-19-08-review-interaction-fixes.md)

- [2026-10-19-07：修复深色模式下的浅色标题菜单](2026-10-19-07-dark-title-menu.md)

- [2026-10-19-06：普通打开自动启动代理与启动结果提示](2026-10-19-06-proxy-auto-start.md)

- [2026-10-19-05：概览趋势图与图例紧凑排版](2026-10-19-05-compact-overview-legends.md)

- [2026-10-19-04：日志高度拖拽与暗色滚动条跟踪修复](2026-10-19-04-log-resize-dark-scrollbars.md)

- [2026-10-19-03：target 构建缓存空间清理](2026-10-19-03-target-space-cleanup.md)

- [2026-10-19-02：状态栏上方多余留白与滚动范围修复](2026-10-19-02-status-bar-spacing.md)

- [2026-10-19-01：原生界面视觉与组件精修](2026-10-19-01-native-ui-polish.md)

- [2026-10-18-24：本地Release重建](2026-10-18-24-release-rebuild.md)

- [2026-10-18-23：统计表格单元格复制](2026-10-18-23-history-cell-copy.md)

- [2026-10-18-22：默认使用系统本地时区](2026-10-18-22-local-time-default.md)

- [2026-10-18-21：日期版本Release与标签](2026-10-18-21-date-release.md)

- [2026-10-18-20：采用MIT许可证](2026-10-18-20-mit-license.md)

- [2026-10-18-19：公开准备、历史身份与旧实现清理](2026-10-18-19-publication-cleanup.md)

- [2026-10-18-18：布局重绘回归、原生输入边框与SVG导航](2026-10-18-18-ui-regressions-svg.md)

- [2026-10-18-17：拖动缩放残影与任务栏图标修复](2026-10-18-17-resize-taskbar-icon.md)

- [2026-10-18-16：UI 切页与滚动卡顿修复](2026-10-18-16-ui-interaction-performance.md)

- [2026-10-18-15：客户端头部程序图标与名称](2026-10-18-15-brand-header.md)

- [2026-10-18-14：紧凑顶栏、单行状态栏与细滚动条](2026-10-18-14-compact-chrome.md)

- [2026-10-18-13：滚动页面残影与重复绘制修复](2026-10-18-13-viewport-ghosting.md)

- [2026-10-18-12：核心模块二次拆分](2026-10-18-12-module-splits.md)

- [2026-10-18-11：标题栏紧凑菜单](2026-10-18-11-title-menu.md)

- [2026-10-18-10：紧凑滚动页面与日志日期](2026-10-18-10-compact-pages.md)

- [2026-10-18-09：柔和圆角控件](2026-10-18-09-rounded-controls.md)

- [2026-10-18-08：概览统计卡片与实时大图](2026-10-18-08-overview-dashboard.md)
- [2026-10-18-07：原生日历日期选择](2026-10-18-07-calendar-date-picker.md)

- [2026-10-18-06：高级分析、日志与窗口偏好](2026-10-18-06-advanced-analysis-window.md)
- [2026-10-18-05：分段选择、开关与暗色控件](2026-10-18-05-modern-native-controls.md)
- [2026-10-18-04：固定底部状态栏](2026-10-18-04-status-bar.md)
- [2026-10-18-03：K/M单位](2026-10-18-03-token-units.md)
- [2026-10-18-02：简洁原生外观](2026-10-18-02-ui-clarity.md)
- [2026-10-18-01：安全配置与核心整合](2026-10-18-01-complete-core-history.md)
- 更早记录见本目录README。
