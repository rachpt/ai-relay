# 2026-10-25-02：日志查看区按内容溢出显示滚动条

## 目标与范围

按用户截图要求，日志正文查看区2200的横／纵滚动条在对应方向内容没有溢出时隐藏，不再保留禁用的条带。内容增长、筛选／清空、字体及尺寸变化后自动恢复／隐藏；不改其他输入框、表格或页面滚动条规则。保留上一批日期修复及原有工作区修改。

## 根因与原生调查

Win32多行EDIT设置`WS_HSCROLL/WS_VSCROLL`后，内容适应时仍保留禁用条；只删样式或直接隐藏又会停止隐藏轴的范围维护，内容增长不能自动恢复，隐藏后的`GetScrollInfo`可能只是旧缓存。

使用自有隐藏窗口、真实项目字体及96／144／192字体和尺寸参数调查，不访问用户数据：

- 保留ES_AUTO语义，先无条，再H→V→H逐轴重新启用并查询原生RANGE/PAGE，适应则隐藏。不能一开始同时显示双轴，否则条带可能互相挤占形成假溢出。
- classic／高对比度回退的原生主题路线可能在show-H时自行加回V样式，客户区却没有扣V宽；须保持另一轴上次逻辑决定再查询。直接FRAMECHANGED虽能修页宽，却会重新引入临界双条，不采用。
- 临时无条扩大客户区会clamp底／右位置；保存原生首行及H位置，使用真实原生字符滚动单位及左右端点锚定恢复，不写伪造range/page。
- 在捕获期间显隐或发送SETREDRAW会中断暗色拖动／重复滚动，因此延后到释放；更新期间的native range仍由EDIT维护。

详细调查位于忽略目录`target/native-edit2200-{scroll,redraw,edge,frame}-probe-20261025/`。其中参数DPI不等于真实桌面缩放，隐藏窗口不等于物理视觉验收。

## 实际变更

- `src/ui/logs/scrollbars.rs`：仅日志2200安装owner-thread subclass；正文／字体变化后同步更新，SIZE合并到布局事务之后。两轴真实范围判断、空值直接隐藏、原生查询失败保持可滚动降级。Rc保活、深度／重入及dirty合并，内部／外部redraw marker与generation区分，不覆盖重入禁绘。显隐中屏蔽中间重绘，最终只排队重绘；无新的磁盘／网络工作。
- 捕获期间不改变条带／redraw，以50ms原生timer及释放消息进行有界合并重查；销毁清理timer与subclass。底端／近底及右端非整字符位置经原生消息恢复，选区不变。
- `src/ui/logs/mod.rs`接入；`src/ui/scroll_theme.rs`保留隐藏双轴日志框的暗色资格，使下一次溢出立即沿用暗色，并复用既有原生字符单位查询。
- `src/ui/logs/scrollbars/tests.rs`新增12项：范围溢出边界、浅深／对比度及字体／尺寸、临界相互诱发、替换／清空、禁绘／显隐／资源、capture内正文刷新及repeat、释放后合并、底右位置／原生baseline几何、嵌套正文与禁绘、同步销毁／Rc、隐藏后暗色tracking恢复。
- `tools/smoke-logs-interaction-ui.ps1`在既有私有暂停fixture加入浅深内容显隐和真实滚动矩阵，增高／加宽隐藏及缩回复原；排除动态WS_SCROLL位但继续校验readonly/multiline/auto-scroll语义。新增fit／clear物理参考图，保留held阶段亮白严格检查。工具不调用ShowScrollBar／SetScrollInfo，不伪造生产状态。
- 同步需求8.4、README、UI设计、开发索引与STATUS。无新依赖／配置／数据迁移／代理行为变化，不提交、不改版本、不更新dist；不擅自关闭之前存在的Explorer或其他窗口。

独立review曾发现正常刷新取消capture，已延后显隐；进一步测试发现外部redraw generation快照边界并修复。最终只读复核未发现本范围确定新增P1/P2。不是全部Windows行为已经验收的声明。

## 验证

Windows x64 MSVC，锁定离线依赖。

| 命令／范围 | 结果 |
|---|---|
| `cargo fmt --check` | 最终通过 |
| `cargo check --all-targets --locked --offline` | 通过 |
| `cargo test --locked --offline --lib ui::logs::scrollbars:: -- --test-threads=1` | 12项通过 |
| 既有scroll_theme／input_frame专项 | 20／13项通过 |
| 既有native_log高度与键盘专项 | 2项通过 |
| `cargo test --locked --offline --lib -- --test-threads=1` | 258项通过，约164秒 |
| 全部15集成目标，`--locked --offline -- --test-threads=1` | 153项通过；全工作区与库合计411项，分段无skip |
| `cargo test --locked --offline --doc` | 通过，0项 |
| `cargo clippy --all-targets --locked --offline -- -D warnings` | 最终通过 |
| `cargo build --locked --offline` | Debug通过 |
| 修改日志工具`-ValidateOnly` | 最终解析／静态调用／C#编译／纯规则自检通过 |
| Debug实际96 DPI日志GUI矩阵 | 通过，Passed=true／Complete=true仅指工具scope；45.712秒，22项显隐，44物理帧，无skip，亮白0，配置／JSONL哈希不变、正常退出 |
| 静态CRT `cargo build --release --locked --offline` | 通过，约169秒 |
| Release x64 PE32+／导入／`--version` | 通过，无额外CRT／SQLite／OpenSSL动态依赖，版本输出正确、退出码0 |
| Release实际96 DPI日志GUI | 部分通过，完整工具未通过；见失败记录，不用Debug通过替代 |
| 新模块诊断及`git diff --check` | 无错误／告警；Git既有CRLF归一化提示保留 |

集成目标为config_foundation、config_persistence、history_backend、history_reader、history_reports、logging_backend、logging_paging、proxy_core、proxy_features、proxy_history、proxy_logging、proxy_metrics、proxy_model、proxy_usage、usage_observer；仅本地mock，不访问真实模型或计费。

## 失败与处理

- 初始严格Clippy指出inclusive表达式int_plus_one，改为等价signed64比较，最终通过。
- 新底右位置回归揭露临时client扩容clamp，加入原生位置恢复；classic主题进一步触发另一轴样式／client不一致，以上述逻辑轴修正而非FRAMECHANGED解决，原失败／诊断断言保留。
- 首Debug完整工具在grip拖动中出现editor外框137而期望312，原因为SIZE内同步改NC干扰deferred window-position batch；改为SIZE后排队更新。随后12项新测试、原有高度专项及Debug完整GUI通过。初次前台请求被拒绝及Explorer遮挡如实记录，未强抢前台或关闭其他窗口。
- Release首轮已通过浅深fit、H-only、V-only、both、clear、恢复和height矩阵，并保存浅深fit／clear4张物理图；宽度resize后工具立即读visible时遇到内部禁绘的瞬间，拒绝hidden目标。新增1500ms有界等待visible/enabled，不跳过持续失败、不改变生产。
- Release第二轮在旧grip键盘流程的“No-motion grip focus click must not change viewer height”断言失败，并有前台请求被拒绝。未隔离根因、不声称与系统或本修复无关；停止继续重试，完整Release工具保持未通过。
- 两次Release均正常退出，配置／闭合JSONL哈希不变，查询无残留AI Relay进程；没有强杀或删除失败证据。

## 产物与证据

- Debug：`target/debug/ai-relay.exe`；Release：`target/release/ai-relay.exe`。
- Release7,894,528字节，SHA256 `E39454EF955C29A8E58EAF93A8690167FC3A86894A9C693D1EEE4243DFC4790B`；源码为HEAD `7de0eda`＋日期／日志未提交修复，版本仍`2026.10.18`，dist及显式target旧目录未改。
- 构建／PE／版本报告：`target/release-build-2026-10-25-scrollbars.json`；SourceCommit仅为基线，不表示本补丁已提交。
- 初Debug失败：`target/logs-interaction-ui/20261025-064922-b43fc9e6/report.json`。
- Debug最终通过：`target/logs-interaction-ui/20261025-081315-31175b8d/report.json`。
- Release部分显隐／物理图及瞬时hidden失败：`target/logs-interaction-ui/20261025-082925-78cd132a/report.json`；同目录`light-fits-no-scrollbars-REFERENCE.png`与`dark-fits-no-scrollbars-REFERENCE.png`已查看，正文框内没有横纵条，外围页面纵条因页面实际溢出仍保留。
- Release第二轮失败：`target/logs-interaction-ui/20261025-083140-4d75ee84/report.json`。

工具按原规则保留私有fixture及截图，不含真实Key或上游，不把temporary证据加入发行包。

## 剩余限制

- Release完整grip／held矩阵仍需在稳定独占桌面隔离并复核；Debug工具Complete=true仅表示其96 DPI定向scope完成。
- 真实150%／200%、多屏、系统高对比度、硬件鼠标／键盘、IME及读屏未执行；字体参数／隐藏原生测试不替代。
- 捕获期间延后显隐是有意的交互保护，释放后更新；不承诺任意退化到不足一行高度的EDIT尺寸。
- 上批日期弹窗及历史菜单／月历／早轮白条未完成项保持原记录，未在本批解决或改写。
