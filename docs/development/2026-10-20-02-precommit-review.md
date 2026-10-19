# 2026-10-20-02：提交前独立 review 与原生绘制回归修复

## 范围与授权

用户要求 review 代码后协助 commit。本批检查相对HEAD的全部待提交源码、文档与原生tools，包括此前组件精修、布局／滚动、自动启动说明、标题菜单和日期适配；3个独立review会话按领域分工，写集互斥，最终对具体修复再次只读复核。用户数据、构建产物、截图、dist不属于提交范围；不建分支、不改版本或标签、不推送。

## Review 发现与实际修复

| 级别 | 问题 | 实际修复与回归 |
|---|---|---|
| P1 | 分段控件hover的TCM_HITTEST同步销毁后继续访问已释放Box | `choices.rs`改为Rc保活及Cell状态，命中返回核验attached及实例身份；外层一次性子类在命中中DestroyWindow并创建替代控件，验证旧状态回调期间保活、随后释放、替代状态不污染 |
| P2 | Edit的WM_NCACTIVATE(l=-1)仍screen overlay | `input_frame.rs`保留原生转发／返回值，对该消息及同步嵌套绘制保持PaintDeferral，跳过screen overlay；浅／深及正常恢复／打印回归 |
| P2 | 日期WM_PRINT整幅回写破坏零flags、PRF_CHECKVISIBLE及局部绘制区域 | `date_picker/tint.rs`检查隐藏与零flags；按真实window/client区域及调用方clip限制回写，保存源与目标DC；新增3项逐像素哨兵、CLIENT／NONCLIENT、偏移／2倍映射／带缺口裁剪、PRINTCLIENT语义回归 |
| P2 | 日期截图仅检查暗背景，纯色空图可假通过 | 日期工具PNG及物理图增加正文／月历标题、日期网格、今天页脚独立前景区域检查，排除整行边框；ValidateOnly用纯白／纯暗及逐区缺失图自检，不使用OCR冒称文字语义验收 |
| P2 | 系统菜单非ASCII助记键大小写不匹配 | `menu_theme.rs`用公开CompareStringOrdinal忽略Unicode大小写；新增俄文／希腊／é与禁用／转义／快捷键后缀隔离回归，不改系统语言或键盘 |
| P2 | 菜单工具启动时MainModule为空可立即假失败、清理缺少主窗口重找 | 原始进程handle＋QueryFullProcessImageNameW核验实际EXE，不依赖托管MainModule就绪；失败清理若HWND尚未找到最多5秒有界重找，仅正常Exit，不强杀 |
| P2 | 菜单dark截图可把内部纯黑当作深色通过 | 采样内部RGB是否完全相同，排除边框干扰；先保存PNG并记录，再对浅／深统一拒绝纯色，原暗／亮比例阈值不放宽 |

第一组及日期／菜单修复已由对应独立review会话只读复核，未再发现修复增量中新的确定P1/P2；不表示整个应用不存在问题。自动启动／overview／布局与滚动领域本次独立静态检查未发现其他新的确定问题；早轮日志白条等既有未决项不被后续通过覆盖。

## 验证

| 命令／验收 | 结果 | 说明 |
|---|---|---|
| `cargo fmt --check` | 通过 | 最终工作区 |
| `cargo check --all-targets --locked` | 通过 | Windows x64 MSVC |
| choices／input_frame／date_picker／menu_theme定向 | 通过 | 分别9／13／12／6项，含6项新增原生回归 |
| `cargo test --locked --lib -- --test-threads=1` | 通过 | 246项，无skip／ignored |
| 显式15个集成目标串行测试 | 通过 | 153项，无skip；合计399项 |
| `cargo test --locked --doc` | 通过 | 0项 |
| `cargo clippy --all-targets --locked -- -D warnings` | 通过 | 无告警 |
| `cargo build --locked` | 通过 | Debug更新 |
| 静态CRT `cargo build --release --locked` | 通过 | 当前进程编码flags、home／工程编译路径映射，不改持久环境 |
| Release x64 PE32+与导入检查 | 通过 | 未见额外CRT／SQLite／OpenSSL导入，非干净系统验收 |
| 7个工具ValidateOnly | 通过 | dark-date、dark-menu、modern、auto-start、logs-interaction、overview-dashboard、ui-regressions；仅相应声明／纯逻辑／自检，不当作实际GUI验收 |
| Debug完整modern UI矩阵 | 通过定向范围 | 96 DPI，16四页矩阵、24设置及24历史子视图TOP／BOTTOM／缓存，输入／选区／配置不变并正常退出；离屏非物理视觉证明 |
| Debug加强版日期专项 | 首轮失败，原构建复跑通过 | 84离屏内容检查通过、物理6通过6遮挡跳过、8轮GDI恒99、配置不变且正常退出；Complete=false |
| Release加强版日期专项 | 未通过 | 原生浅色翻月后的今天页脚栅格缺失，严格前景检查拒绝，未降低阈值或把失败改成成功 |
| 最新Debug菜单专项 | 未通过 | PrintWindow出现内部纯黑帧，统一纯色守卫拒绝；本批不声称菜单视觉重新验收通过 |
| 改动文件高信号秘密／隐私扫描 | 无未处理命中 | 只读本地规则，不联网验证；唯一URL userinfo为既有example.invalid＋synthetic-secret脱敏测试，已人工确认，不是绝对安全保证 |
| `git diff --check` | 通过 | 仅既有CRLF归一化提示 |

集成目标为config_foundation、config_persistence、history_backend、history_reader、history_reports、logging_backend、logging_paging、proxy_core、proxy_features、proxy_history、proxy_logging、proxy_metrics、proxy_model、proxy_usage、usage_observer。GUI脚本及原生库测试串行，不与其他应用冒烟同时执行；集成使用本地模拟，不访问真实模型／Key、不产生计费。

## 失败与证据边界

- 新日期回归helper最初把flags参数声明为u32，而windows-sys的PRF常量为i32；改为i32后12项日期专项和最终全库通过，保留代码功能，未降低像素断言。
- 首轮本地隐私扫描规则转义错误导致工具异常，修正后扫描60个改动／未跟踪文件；URL userinfo唯一命中经定向读取确认是既有虚构安全回归，不打印凭据正文。提交前仍核对暂存清单，不把ignored的target或数据加入提交。
- 菜单旧dark比例可接受纯黑，本次实际捕获并复核，加入统一纯色拒绝。浅色根菜单及子菜单曾正常，后续黑帧的来源未确认，不能仅以native owner-draw flags正确或单测通过推出视觉正常。短暂尝试等待主题完成未解决黑帧，已撤回该延时，不为凑通过增加无界重试。
- 日期首轮Debug及Release均在原生浅色翻月后出现今天页脚缺失图；新前景断言正确失败，图片已查看。Debug原构建独立复跑通过且内容断言保持；Release本批最后一次仍失败。尚未隔离动画／PrintWindow／真实屏幕行为，不能将失败一律归为环境问题，更不能由后续成功证明根治。
- 高DPI／多屏／真实高对比度／IME／读屏和物理键鼠仍待验收；不修改系统主题、真实Startup、剪贴板或其他实例，不强杀自建GUI进程。

## 产物与报告

- 最新Debug和Release包含本批已确认源码修复；Release 7,889,408字节，SHA256 `5278316C8C4B25323765BA81406138910E8478C3B170F28C78AC3BE34CB33846`；dist未更新。
- modern通过：`target/modern-ui-screenshots/20261019-184845-ab347de331174a53a5734db98304a055/results.json`。
- 加强版Debug日期通过：`target/dark-date-ui/20261019-184608-c8ed8888d1cf4605abe6f32497cc64d2/report.json`；原首轮失败：`20261019-183731-f1d797e55f16405aaad01e8e01bb1555/report.json`。
- 加强版Release日期失败：`target/dark-date-ui/20261019-185508-17c2dfb3cdfb4ebd9a88020e6f77def2/report.json`。
- 菜单纯色拒绝失败：`target/dark-menu-ui/20261019-184416-d98e2bde08e64f17bbabadc3b9dd9887/report.json`；之前的183404／184012失败报告同目录保留，不改写。
- 用户授权提交所有已review的待提交开发变更；提交结果以Git日志为准，未包含运行数据／构建产物，也不推送远端。

## 下一步

在独占桌面隔离菜单纯黑捕获与浅色月历翻月页脚缺失，区分原生动画、打印API与实际像素；继续真实高DPI／系统主题／读屏验收。上述失败保持未完成，不阻止将已确认修复和诚实的验收记录保存为本地开发提交，但不作为正式发行就绪声明。
