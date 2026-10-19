# 2026-10-19-02：状态栏上方多余留白与滚动范围修复

## 目标

修复底部状态栏上方残留空白，统一各页尾部与滚动范围。保持40 DIP固定状态栏、44 DIP固定导航、控件值/选区及各页滚动位置，不改变代理和配置行为。

## 原因

- 状态栏已经缩为40 DIP，页面仍沿用84/92/104 DIP旧底部额度；大窗口概览、统计、日志尾部距离状态栏约44/52/64 DIP。
- 视口extent另减28 DIP，使小窗口滚到底与大窗口的间距不一致；单独修改任一公式会造成新的裁剪或空白。
- 普通设置分类始终使用至少820 DIP虚拟高度，短表单也存在大段空白滚动尾部。

## 实际变更

- `src/ui/viewport.rs`：统一 `BOTTOM_PADDING=16`、`content_bottom=virtual_height-footer-padding`、`content_extent=virtual_height-header-footer`。普通布局与缓存refresh共用计算，移除旧的额外28 DIP扣减。
- `overview.rs`、`history/layout.rs`、`settings/panel.rs`、`logs/mod.rs`：概览、统计明细/汇总/文本/管理、会话和日志尾部均按同一底边布局；长内容滚到底或大窗口无滚动时，最后控件到状态栏保留16 DIP。管理信息与状态文字之间继续保留8 DIP。
- 普通设置按当前分类动作行实际底边计算extent，不再允许滚入人为生成的空尾部。短表单下方的自然剩余空间保留，不把保存按钮强行固定到底部；长分类所有操作仍可滚动到达。
- 复核发现设置滚动条显隐改变宿主实际客户区宽度，但此前按旧宽度布局。新增 `Viewport::prepare` 先确定目标scroll range/page，再读取最终宽度；缓存键加入实际宿主宽度。该步骤不覆盖逻辑保存offset、不移动children，随后arrange/refresh统一完成一次定位。
- `scroll_theme.rs`：短设置页无 `WS_VSCROLL` 时仍保留已识别页面宿主的暗色子类，避免之后恢复滚动条出现白色条带。新增识别仅限子窗口 `Static + ID2400 + WS_EX_CONTROLPARENT`，其他控件支持范围不变；高对比度仍卸载。
- `src/ui/tests.rs`新增三个原生回归：96/144/192布局参数、大小及临界高度、四页/统计子视图/六分类、完整尾部到footer间距、缓存返回、草稿与实际宿主右边距。补prepare原生clamp不提前移动children及暗色宿主无条→有条像素回归，调整旧日志高度断言。
- `tools/smoke-modern-ui.ps1`新增完整尾部矩形间距/extent/24 DIP右边距检查，以及六分类与六历史子视图TOP/BOTTOM/缓存矩阵。不能使用已裁剪的矩形冒充尾部完整可见；management入口含存储I/O，不在该GUI矩阵点击，管理布局由隔离native回归覆盖。
- 无新增依赖、版本/标签/配置格式变化，无真实Key、用户配置、Startup、系统代理、防火墙或剪贴板变更。

## 验证

| 命令或验收项 | 结果 | 说明 |
|---|---|---|
| `cargo fmt --check` | 通过 | 最终工作区 |
| `cargo check --all-targets --locked` | 通过 | Windows x64 |
| `cargo test --locked ui:: -- --test-threads=1` | 通过 | 139项UI测试，含新增底边/宽度/暗色track回归 |
| `cargo test --locked -- --test-threads=1` | 最终通过，首轮有波动 | 351项；首轮既有proxy_usage连接失败测试返回504而非预期502，见下文 |
| `cargo clippy --all-targets --locked -- -D warnings` | 通过 | 最终验证 |
| `cargo build --locked` | 通过 | 最新Debug |
| modern `-ValidateOnly` | 通过 | PowerShell/C#编译及几何自检，不等同GUI验收 |
| Debug modern GUI | 通过 | 实际96 DPI，16组四页双主题双尺寸、24组设置分类及24组历史子视图TOP/BOTTOM/cache；严格16 DIP尾部与24 DIP实际客户区右边距 |
| `smoke-ui-regressions.ps1 -BudgetSeconds 120` | 已执行项通过，未完整 | 五项物理残影比较0像素差异；浅深边框/SVG/输入保护通过，20轮GDI恒91；目录状态截图因Explorer遮挡跳过，Complete=false |
| 静态CRT `cargo build --release --locked` | 通过 | 进程级编码Rustflags、home/项目路径映射，无持久构建配置变更 |
| Release x64 PE32+导入检查 | 通过 | 无需额外安装CRT/SQLite/OpenSSL的导入项，不能替代干净系统验收 |
| Release modern GUI | 通过 | 同一16/24/24矩阵，10条回环mock，输入/选区/开关/pending/配置字节保持，正常退出无遗留应用实例 |

### 失败与修复过程

- 初次check发现设置子模块的 `document_bottom` 可见性不足，改为仅在 `crate::ui` 范围可见后通过。
- 独立review指出滚动条显隐的旧宽度缓存问题；新增实际右边距断言先失败（分类2右边976，而实际宿主要求959，差17像素），随后增加prepare/宽度缓存处理，专项及全部UI回归通过。未通过放宽断言或仅增加裁剪掩盖问题。
- 全量首轮 `tests/proxy_usage.rs:355` 的 `connection_failure_is_counted_but_local_replies_are_not_and_session_survives_restart` 返回504而非预期502。本批没有代理路径改动；定向复跑通过，随后351项全量串行复跑通过。该次波动未确认根因，不将其描述为已修复代理缺陷。
- 最终独立只读复核未发现本批底边/prepare/缓存/宿主识别新增确定问题。

### 本地证据与产物

- Debug modern：`target/modern-ui-screenshots/20261018-212449-949de4079b7b489eb0c93377b9e20c16/results.json`；Release modern：`target/modern-ui-screenshots/20261018-214109-b90cd044ac894d4ab2f68d382448e854/results.json`。矩阵通过，截图为明确标注的PrintWindow离屏渲染；不能代替物理屏幕验收。
- 物理回归：`target/review-ui/regressions-20261018-213231-25b2d5/report.json`。分类返回、960×740/800×560/900×700日志缩放及缓存返回残影差异均0；目录状态被Explorer遮挡跳过。时间目录使用UTC。
- 更新 `target/debug/ai-relay.exe` 和 `target/release/ai-relay.exe`；Release为7,840,256字节，版本保持 `2026.10.18`。
- Release SHA256：`810466067CCE56FDFB0DC6A57E2124F139B996E92DCD84F585EB0C9575B20BB4`。
- 未更新dist或显式target目录下旧包，未提交、建分支或更改标签。

## 未完成与限制

- 本批真实GUI为96 DPI；144/192参数与几何打印不代表150%/200%实际桌面、多屏、真实高对比度、IME或读屏验收。
- 前一轮review的 `input_frame` 几何通知中立即overlay绕过no-redraw布局事务的P2仍待修复；不是本次空白区域的根因，也未作为本批修复成果。布局完成后的0差异截图不能排除瞬时中间帧。
- 管理页实际GUI末尾未在modern脚本执行，已由停止控制器的隔离原生布局测试覆盖；仍需补实际存储管理界面验收。
- 长稳、干净系统、完整性能验收及代理测试波动根因不在本批范围。
