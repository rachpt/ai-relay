# 2026-10-31-02：统计状态中文分段与窄屏自适应布局

## 目标与根因

用户反馈约800 DIP窄窗口的历史查询状态第二行截断，内容与下方摘要排列不清晰。根因为`2325`状态STATIC在布局中固定34 DIP，查询又拼入`Counters { queued: ... }`等Debug长句；实际字体两行／多行高度超过控件时，图表仍从固定312 DIP开始。

本批在上一批未提交UI改动上继续，保留所有已有工作；只修统计展示与对应布局／测试／工具，不修改统计口径、数据库schema、配置、代理重试或转发行为，无新增依赖。

## 实际修改

- `src/ui/history/status_text.rs`：查询状态分为中文查询摘要及时间／记录累计两段，明细改一基序号，空页显示0条；本地／协调世界时、小时／天／月中文化。保留全部五项精确指标：累计入队、已写入、队满丢弃、数据库失败次数、关闭丢弃。`db_failures`还涵盖读／维护／关闭失败，不误称仅写入失败或丢失条数；不把queued减written推算当前积压。导出状态明确当前页／筛选全量及文件路径，不变更CSV。计数边界使用u64或饱和计算，不回显筛选模型／分组值／凭据。
- `src/ui/history/status_layout.rs`：使用控件当前HFONT与最终可用宽度，`DrawTextW(DT_CALCRECT|DT_WORDBREAK|DT_NOPREFIX)`测高并加2 DIP余量，最低34 DIP；保存／恢复DC及字体、释放窗口DC。STATIC增加`SS_NOPREFIX`，使含`&`的错误／路径与计量行为一致。
- `history/layout.rs`：normal状态仍锚270 DIP，advanced仍424 DIP；图表／排序／视图分段／明细统一从状态底边＋8 DIP推导，图表200 DIP不缩小。统计minimum820 DIP加状态实际增高量，管理页storage与状态尾端保持8 DIP，session不加历史状态extra。长文字能完整纵向滚达，不靠减少字号或省略隐藏计数。
- `history/mod.rs`、`events.rs`、`table.rs`及`src/ui/mod.rs`：所有实际状态更新统一`set_status`，仅排队scalar状态变化消息；History借用结束后使统计布局缓存失效并重排，隐藏页只置脏，进入时重排。同宽的新文字也会刷新几何；最终客户区宽参与minimum计算，保留原生滚动／草稿／选区／查询状态。
- `history/status_tests.rs`、`history/tests.rs`和`ui/tests.rs`：增加真实字体96／120／144／168／192参数与800／1000／1400 DIP宽度的完整文字测量、normal／advanced间距、原生tail16 DIP、缓存返回、异步结果、隐藏页、管理与session独立尾端和100次计量GDI稳定回归。旧固定592改为状态底边关系，短状态仍专门验证312／592兼容锚点。
- `tools/smoke-modern-ui.ps1`：状态完整HWND高度推导extra及各行，独立计算统计minimum，不从观察到的scroll extent反推预期；保留旧PageTail接口供其他工具复用，新自检含extra0／20／60及错误旧820反例。查询断言映射中文并严格检查10条fixture两段完整状态，保留pending／reset／typed input及秘密保护。
- `tools/smoke-ui-regressions.ps1`：固定592改为状态底边关系，新增相关纯几何自检；未宣称本批执行该工具真实GUI。
- 同步README、UI-DESIGN、STATUS与本记录；属于既有GUI可读性修复，未改功能／验收范围或新增架构／依赖路线，不新建ADR。

## review与首轮失败

主agent实现计量／异步布局／回归；子agent单独新增纯格式模块并更新两个工具，随后只读复核。未发现确定新增生产P1／P2；指出新增测试把隐藏结果HWND的文档坐标与可见控件viewport坐标混比，按既有可见性契约修正测试，不改变生产隐藏行为。

- 初次check缺windows-sys未导出的`SS_NOPREFIX`，按公开winuser.h值本地定义0x0080，不增加feature或依赖。
- 初次统计测试17／18，旧固定592与新状态高度不符；更新为实际状态底边关系，保留即时不在borrow内重排和草稿／scroll断言。
- 新回归曾因兄弟测试helper私有未编译，以及最终误断隐藏TABLE顶部而20／21；修正可见性／坐标／helper范围后21／21通过，没有放松文字高度／间距／tail断言。
- 共享窗口测试10／11：日志键盘grip测试GetFocus返回null，串行定向复跑1／1通过；未修改日志、焦点断言或将首轮失败改写为成功，未确认焦点波动根因。

## 验证命令与结果

全部实际构建／测试在主项目进行，沿用`CARGO_INCREMENTAL=0`避免前批共享target快照干扰；无新快照、无清缓存或用户数据操作。

| 验证 | 结果 |
|---|---|
| `cargo fmt --all --check` | 通过 |
| `CARGO_INCREMENTAL=0 cargo check --all-targets --locked --offline` | 通过 |
| `CARGO_INCREMENTAL=0 cargo test --lib ui::history:: --locked --offline -- --test-threads=1` | 21通过，15.04秒；含9格式／3新native布局及既有统计回归 |
| `CARGO_INCREMENTAL=0 cargo test --lib ui::tests:: --locked --offline -- --test-threads=1` | 首轮10通过、1日志焦点失败；该项定向复跑通过，17.67秒；11项分段补齐 |
| `CARGO_INCREMENTAL=0 cargo test --tests --locked --offline -- --test-threads=1 --skip ui::` | 非UI库65及15个集成目标153通过；本批不把未重跑的其他UI记为再次通过 |
| `CARGO_INCREMENTAL=0 cargo test --doc --locked --offline` | 0项，无失败 |
| `CARGO_INCREMENTAL=0 cargo clippy --all-targets --locked --offline -- -D warnings` | 通过 |
| modern／ui-regressions `-ValidateOnly` | PowerShell解析、合并C#编译、public helper及纯自检通过 |
| diagnostics／`git diff --check` | 无错误／警告，diff无空白错误；Git行尾规范提示另计 |

测试、doc和Release构建的组合命令触及300秒总时限：集成及doc已通过，Release仍编译，**该组合命令不记为通过**。随后忙碌保护曾拒绝；未停止其他构建，待空闲后单独Release成功。

### 最终Release

通过忽略的`target/build-release-2026-10-31-history-status.ps1`单次进程设置静态CRT／编译路径映射、锁定离线构建：

- 构建87.9秒。
- `target/release/ai-relay.exe`：**7,993,856字节**。
- SHA256：**`3766D930A46433F6478C302250322B4A24AD6404C1D91FB246EBA4F67271FBAA`**。
- x64 PE32+、仅Windows系统DLL导入，无需另装CRT／SQLite／OpenSSL；`--version`为`AI Relay 2026.10.18`、退出0、stderr空。
- 版本／profile／Cargo.lock／dist／显式target历史成品未改，无提交或推送。构建源包含前批及本批未提交修复，不由相同版本号保证内容相同。

### Release GUI

最新Release完整modern限定矩阵两次均通过，分别297.99秒及289.76秒；没有调整300秒内部预算或放宽guard。

- 实际96 DPI，800×680／1000×820两种外框；每次16四页、24设置分类、24统计子视图TOP／BOTTOM／cache，168次PrintWindow捕获／165独立PNG。
- 10个私有回环mock请求，100／50／150 Token；native筛选／pending／中文状态／新状态相对坐标、状态extra独立范围、table／text切换、16 DIP底边、15 Edit草稿／选区及输入规则保持、配置字节不变、本人测试进程正常退出，无真实Key／计费／剪贴板／Startup／系统代理修改。
- compact查询状态实测高度42px（原34px），图表top320、sort528、table600；UTC较长中文可三行，状态继续增高不与图表重叠。宽窗／短状态使用较少高度，完整结果区通过滚动可达。主agent查看浅色800统计与深色查询图片，未声称逐图逐字或所有文案的视觉验收。
- 本批图是OFFSCREEN，不能证明物理遮挡／实际高DPI／读屏。management GUI入口会访问存储，工具仍明确不进入；管理布局仅native fixture验证。

证据位于忽略的：

- `target/modern-ui-screenshots/20261031-050841-fa236aef666e4d608ca91a51219d7a79/`
- `target/modern-ui-screenshots/20261031-051432-c24e6a0c5e2a4e58bd308c15c69c1919/`
- `target/history-status-modern-2026-10-31.log`、`target/history-status-modern-final-2026-10-31.log`
- `target/release-build-2026-10-31-history-status.json`

## 剩余限制

- 实际150%／200%／多屏、Win10／11差异、真实高对比度／读屏、物理输入、连续刷新视觉、资源耗尽未专项执行。字体参数不等于物理DPI验收。
- 本批保证常规中文状态纵向计量与下方排布；超长无断点单词／任意导出路径横向完整性未专项验收，不把可见性关系测试当作每字OCR。
- 前批Light物理滚动采样遮挡、菜单／日期／窗口caption及其他STATUS未完成项保持，不因本批界面矩阵通过解除。
- 下一步在真实窄屏／高DPI复核最新Release；未主动提交、建分支或打包，不改用户配置及历史数据。
