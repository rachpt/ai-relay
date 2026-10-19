# 2026-10-19-06：普通打开自动启动代理与启动结果提示

## 目标与现状

用户要求存在有效配置时，打开软件即可自动启动代理。需求4.1/6.4及既有实现已经规定 `proxy.auto_start` 默认开启；本批核对并验证真实普通启动路径，完善设置说明及被覆盖的启动结果，不另装 Windows 服务，也不改动 Windows 登录启动项。

## 实际变更

- `src/app/controller/worker.rs`：保留原启动决策，区分首次无配置、显式关闭、空上游、非回环待手动确认、普通自动启动及CLI显式启动。排队启动时只说明“正在使用已保存配置自动启动代理”，实际监听成功后才显示运行中；失败由既有错误路径显示并请求窗口出现，不添加循环重试。
- `src/ui/settings/mod.rs`、`panel.rs`：为既有开关分配稳定ID2065，文案改为“打开软件后自动启动代理（使用已保存配置）”，仍位于“启动存储”，沿用显式保存/草稿规则。与登录Windows选项ID2031独立，不覆盖已有 `false`。
- `src/ui/settings/events.rs`：启动期间自动查询登录启动项的通用成功/失败事件不再覆盖代理主状态，具体查询结果仍显示于启动设置区域。过滤新增条件限定为实际“登录自启操作”前缀，不能吞掉“操作失败：保存应用不得改变登录自启；请单独操作。”等其他操作错误。
- `src/app/controller/tests/startup.rs`：新增6项真实Controller worker回归，通过普通Launch、事件、IPC状态及实际回环TCP监听验证缺省/显式开关、无配置/损坏/空上游、端口占用、CLI覆盖关闭及LAN保护。私有绝对目录、动态端口、配置字节不变；RAII在断言失败时也先退出并限时等待线程，超时保留目录。
- `src/ui/settings/tests.rs`：新增开关载入/草稿切换/切分类/丢弃恢复与登录设置独立回归，以及登录启动项结果不能覆盖代理启动结果的状态格式回归。
- `tools/smoke-auto-start-ui.ps1`：7组/10个私有fixture，只用普通 `Launch --config-dir <绝对目录>`，不以GUI/CLI Start冒充自动启动。只读检查开关、本地health和固定回环mock GET，检测手动停止及绑定失败后重复打开仅Show；配置哈希/配置产物数量在启动前、运行中、正常退出后保持不变。
- 工具拒绝已有实例，固定原始Process/handle/PID/启动时间/EXE及自有HWND树；隐藏采用异步投递与有界确认，退出仅走自有主窗口正常路由，不强杀。新建mock及端口占位socket清除并验证继承标志，避免Framework测试socket泄漏给被测子进程。LAN fixture仅用独占回环占位阻止意外绑定，不连接LAN，必须显示手动确认说明而不是仅绑定失败。
- 更新README及开发指南。无新增依赖、配置迁移、需求范围/默认值变更；未提交、建分支、改版本/标签、更新dist或再次清理target。

## 验证

| 命令或验收项 | 结果 | 说明 |
|---|---|---|
| `cargo fmt --check` | 通过 | 最终工作区 |
| `cargo check --all-targets --locked` | 通过 | Windows x64 MSVC |
| `cargo test --locked app::controller::tests::startup:: -- --test-threads=1` | 通过 | 6项新增真实worker回归，不发HTTP/POST、不连接上游 |
| `cargo test --locked --lib ui::settings::tests:: -- --test-threads=1` | 通过 | 8项，含2项新增状态/原生开关回归 |
| `cargo test --locked -- --test-threads=1` | 通过 | 380项：227库＋153集成，临时数据及本地mock |
| `cargo clippy --all-targets --locked -- -D warnings` | 通过 | 无告警 |
| `cargo build --locked` | 通过 | 更新Debug |
| 新工具 `-ValidateOnly` | 通过 | PowerShell AST/helper参数数目、C#编译、ABI、纯handle flags及释放守卫合成行；不调用Win32/网络/GUI |
| Debug普通启动专项 | 定向范围通过 | 7组/10fixture全部通过，约34.72秒，配置不变且全部正常退出 |
| 静态CRT `cargo build --release --locked` | 通过 | 当前进程编码Rustflags、home/项目编译路径映射，无持久构建配置修改 |
| Release x64 PE32+导入检查 | 通过 | 未见另装CRT/SQLite/OpenSSL导入，不能替代干净系统验收 |
| Release普通启动专项 | 定向范围通过 | 同样7组/10fixture通过，约34.56秒，配置不变且全部正常退出 |
| 独立只读review | 已修后复核 | 状态过滤、LAN边界、显式关闭、监听状态、工具身份/预算/继承及释放守卫；最终未发现新增确定P1/P2 |

### 失败、取证及修正

- 首轮fmt报告新增条件换行，修正后通过；隐藏原生父窗口下 `BM_CLICK` 未改变测试复选框，测试改为明确设置草稿勾选状态后验证读取/切页保留，不把隐藏控件测试冒称真实点击验收。
- 独立review发现泛用 `contains("自启")` 会吞掉其他操作的真实失败，收窄为登录启动结果的明确前缀并加入真实错误原文回归；同步跨进程 `ShowWindow` 改为异步投递＋有界等待。
- 第一轮Debug工具在第二fixture因启动初期托管Path元信息检查失败。加入最多5秒的仅空Path刷新：从第一次循环起就用原始handle核对真实EXE，非空不匹配立即失败，PID/handle/启动时间不允许变化。后续运行不再出现该问题，未降低身份校验。
- 后续Debug前8fixture通过，但occupied-port关闭占位后TCP表仍有HARNESS监听。加入行级脱敏诊断及同批APP/端口判断；只允许原HARNESS IPv4回环项最多4秒等待消失，APP任何监听或目标端口OTHER均失败，之后才开始完整2500ms观察。4秒仍失败，未继续扩大等待预算或更改产品行为。
- 独立最小取证发现Framework新建私有TcpListener的socket继承位为1。对仅本工具新建的占位/mock socket执行Get→Set(mask1,flags0)→Get验证后，最终两构建均在APP仍存活时于Stop后的首份快照看到目标端口零监听，释放确认约10ms；与测试socket继承链路一致。没有复制/关闭子进程socket，未修改原借用helper脚本。
- review另指出首份Stop后快照只记不判会漏掉已观察到的违规行；改为同一 `Released` 纯守卫立即判定首份及后续行，合成APP同/异端口与OTHER行必须拒绝，验证通过。所有失败报告保留，不改写为成功。

## 产物与证据

- 最新可运行程序：`target/debug/ai-relay.exe`、`target/release/ai-relay.exe`；版本仍为 `2026.10.18`，Release为7,866,368字节。
- Release SHA256：`8490D227D108150B0DC80598139F5B49A5B838AE85D7B5A3CAF6A78E4DE24AAC`。
- Debug报告：`target/auto-start-ui/20261019-055618-5dec24f1bbfe46399ff2aa3e72865226/report.json`。
- Release报告：`target/auto-start-ui/20261019-055653-4d467afde57242b78e705b9b80dc014c/report.json`。
- 两者 `AutomatedScopeComplete=true`、`Complete=false`，全部10项配置未变、正常退出；`TempRetained=false`、清理错误为空。仅3个运行fixture各转发1次固定回环GET，health不访问上游；无真实模型/Key/费用，不修改剪贴板、Startup、防火墙或系统代理。
- 初期失败报告分别位于 `target/auto-start-ui/20261019-053248-2d4a174cbe4c4448b335e1f0f8cdddb0/`、`20261019-053654-bcdaa56acd0c45d6853013a5c9b5802c/`、`20261019-054456-1408cff90dbf4620acf0d3f49ef61bdc/`、`20261019-054644-3b377f75c7c945aeb762d1fc5a2a7cb3/`；均正常退出且配置未变。未保留测试进程。

## 未完成与限制

- 仅有效已保存回环配置且开关开启时自动监听；已有明确关闭不静默迁移。用户需在“设置 → 启动存储”开启并保存应用。非回环每次仍需GUI本次确认，不因本需求改为自动开放网络。
- 2.5秒无重试和重复打开观察是有界采样，不能证明任意延迟/瞬时行为；IPv4/IPv6表依次查询不是跨族原子快照。配置哈希不是瞬态文件系统写入审计。
- 定向GUI证据为实际原生HWND文本/启禁及本地监听，不是物理键鼠、截图、150%/200% DPI、高对比度、读屏或登录/注销重启验收。`Complete=false`不表示本批自动化失败，也不代表全部产品验收完成。
- 未安装Windows服务、未测试/修改真实登录启动项，既有跨用户/干净系统/长期运行及前批视觉限制保持未完成；dist仍为旧预览包。

## 下一步

- 使用最新Release在真实使用环境复核保存开关、普通打开、手动停止及局域网确认；登录启动项联动仍需独立授权与端到端验收。
- 在无遮挡桌面补前批日志滚动视觉、高DPI/多屏/IME/读屏验收；不以本批启动验证替代这些项目。
