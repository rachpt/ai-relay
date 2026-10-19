# 2026-10-20-01：修复深色日期选择框与弹出月历

## 目标与原因

用户指出日志页日期框聚焦／展开后与弹出月历仍为浅色。旧实现仅给未聚焦、闭合且启用的日期框覆绘，聚焦／禁用／展开直接原生回退；视觉主题月历也忽略部分MCSC颜色。此前直接关闭月历视觉主题曾出现200% DPI七列／页脚裁切，本批不重复该路线。

## 实际变更

- 新增 `src/ui/date_picker/tint.rs`：保留原生视觉主题和布局，使用公开原生打印消息将当前控件绘制到有界32位DIB，GDI同步后做256级灰度／强调色查表映射，再提交到借用绘制DC。颜色覆盖日期框聚焦、展开、禁用、未设置和月历；原生字段／日期选区、命中、键盘和通知仍由Windows处理。
- 32位DIB最多4,194,304像素／16 MiB，无跨帧文本或屏幕缓存；DC/bitmap RAII、SaveDC/RestoreDC、重入／销毁保活、WM_SETREDRAW守卫及非客户区原生边界查询。分配失败保留原生可读绘制，不以空实现冒充成功。
- `src/ui/date_picker.rs`：在原生主题选择后安装着色，月历创建通知及延期尺寸校正中接入同一着色。保留主题度量、原字体及MCM_GETMINREQRECT规则；浅色／高对比度卸载着色，MCSC使用原生颜色作为映射源。旧闭合覆绘只保留为未安装着色时的降级，不与新路径二次绘制。
- 新增 `src/ui/date_picker/tint/tests.rs`：4项回归，连同原日期模块5项共9项。覆盖色阶与对比度、96/144/192字体／控件参数、聚焦／禁用／空值、字段方向键、原生日期／样式不变、浅色／对比度回退、两种print的DC恢复、原生月历最小尺寸／翻月及日期导航，以及30轮GDI精确相等、WM_SETREDRAW和销毁保活。测试激活应用Common Controls v6清单，不把普通字体缩放冒称桌面DPI矩阵。
- 新增 `tools/smoke-dark-date-picker-ui.ps1`：仅提取已有菜单工具C#声明，不执行其他工具body；私有停止配置／无网络、拒绝已有PID／用户session mutex，原始handle核验PID／真实EXE。日志2224/2225及统计2302/2303浅→深→浅，检查闭合／聚焦／展开、七列最小尺寸、真实PageDown／Right／Enter、清空及ISO／原生值一致；84张离屏截图和12张有遮挡保护的实际物理截图、8轮theme＋popup的GDI采样。配置哈希不变、正常退出，不强杀，不改剪贴板／Startup／系统主题。
- 更新README、需求8.1、UI设计／开发指南，新增ADR0016及索引和当前状态。无新增依赖／数据迁移／日期筛选语义或代理行为变更；无提交、分支、版本／标签或dist变更。保留并行08批review修复及其他原有工作区修改。

## 验证

| 命令或验收项 | 结果 | 说明 |
|---|---|---|
| `cargo fmt --check` | 通过 | 最终工作区 |
| `cargo check --all-targets --locked` | 通过 | Windows x64 MSVC |
| `cargo test --locked --lib ui::date_picker:: -- --test-threads=1` | 通过 | 9项，4新增 |
| `cargo test --locked --lib -- --test-threads=1` | 通过 | 240项，0失败，约90秒 |
| 15个集成测试目标 `-- --test-threads=1` | 通过 | 153项，无skip；与库测试合计393项 |
| `cargo test --locked --doc` | 通过 | 0项 |
| `cargo clippy --all-targets --locked -- -D warnings` | 通过 | 最终无告警 |
| `cargo build --locked` | 通过 | Debug已更新 |
| `powershell.exe -NoProfile -NonInteractive -File tools/smoke-dark-date-picker-ui.ps1 -ValidateOnly` | 通过 | 声明提取、C#编译、x64 ABI；不启动GUI |
| Debug日期GUI专项 | 定向通过 | 实际96 DPI，四日期框×浅深浅；84张离屏／12张物理均通过，原生翻月／选日／确认／清空，8轮GDI恒99 |
| 静态CRT `cargo build --release --locked` | 通过 | 当前进程编码flags及home／工程编译路径映射，不改持久环境 |
| Release PE32+／x64／导入检查 | 通过 | 未发现额外CRT／SQLite／OpenSSL动态依赖，非干净系统验收 |
| Release日期GUI专项 | 定向通过 | 同样96 DPI／84离屏＋12物理／原生交互／GDI恒99；配置不变且正常退出 |
| `git diff --check` 与新模块诊断 | 通过 | 无错误／告警；既有CRLF归一化提示保留 |

集成目标为config_foundation、config_persistence、history_backend、history_reader、history_reports、logging_backend、logging_paging、proxy_core、proxy_features、proxy_history、proxy_logging、proxy_metrics、proxy_model、proxy_usage、usage_observer。仅临时配置／本地模拟，无真实模型或计费。

## 失败与取证

- 初期编译使用了bindings未提供的HGDI_ERROR名，改为检查SelectObject真实错误句柄；无API／依赖变更。
- 工具首轮沿用托管Process.MainModule立即取Path，启动瞬间为空导致假失败；改为原始进程handle＋公开QueryFullProcessImageNameW直接核验。原失败报告保留，未把PID本身当作足够身份，不强杀。
- 原生月历测试曾连续出现纯背景截图。经原生未着色对照取证，翻月键已更新真实日期，但动画尚未处理，未着色源同样没有内容；不是着色映射丢文字。测试改为有界处理350ms原生消息后再验稳定栅格，保留内容非空／对比度断言；GUI也在翻月后采样。期间WM_PAINT／WM_PRINT替代尝试已撤回，生产最终仍使用原消息和参数，不为测试放宽绘制语义。
- GUI第一轮声明编译需将引用其他Add-Type类型的C#合并编译；工具未执行旧smoke body。UTF-8 BOM保留以兼容Windows PowerShell中文。
- 实际桌面为96 DPI；本地日期2026-10-20而UTC目录名仍20261019，记录按当前本地日2026-10-20新增，不回写前批历史。

## 产物与证据

- 最新程序：`target/debug/ai-relay.exe`、`target/release/ai-relay.exe`；版本仍2026.10.18，dist未更新。
- Release：7,888,384字节；SHA256 `959804590859C1AAE8686403309D867D86B9ECC950ED6D2BFBBB83DB39D5A90F`。
- Debug报告：`target/dark-date-ui/20261019-164122-575f86155f8e41b1810a7579f29d33db/report.json`。
- Release报告：`target/dark-date-ui/20261019-164609-e2651185e5bb4f439cb8c30bbd497ca3/report.json`。
- 两者Passed=true、NormalExit=true、Complete=false；每构建84离屏＋12物理，四日期框展开、月历原月／翻月后共12项实际物理图均通过且未遮挡跳过。日志结束日期月历及展开框图片已查看；原生标题／七列／今天页脚／选中标记完整。8轮GUI GDI均恒99，进程内30轮更严格相等测试通过。
- 物理截图为控件外框区域，前后核对9个采样点属于自建进程，遮挡则拒绝截图；仍不是连续视频或所有像素无遮挡证明。

## 未完成与下一步

- 实际150%／200%桌面、跨屏DPI、真实系统高对比度／主题通知、真实物理键鼠及读屏仍待验收；参数化字体／DPI绘制不替代这些项。
- 原生动画瞬间、月份／年份高级导航子控件、IME组合和未来Common Controls主题变化尚未完整验收，不宣传所有弹出控件完全暗色。
- 日期范围、UTC／Local转换及日志起止含整日／统计结束不含语义不变；未重跑有真实记录归属的日期专项mock，不据零数据GUI推出所有时区边界均验收。
- 配置／代理／系统集成未改；托盘／表格右键菜单和消息框仍可能保留原生浅色。
