# 2026-10-19-05：概览趋势图与图例紧凑排版

## 目标

按用户要求收紧概览两张趋势图，重点减少图例与速率摘要的分散留白。仅改变绘制排版，不修改采样、代理、配置、页面位置或统计口径。

## 实际变更

- `src/ui/overview_dashboard.rs`：标题与“区间均值 · 次/s／B/s”共用第一行；第二行将短线/点标记、曲线名称与最新区间速率直接组合。两项按实际正文HFONT测宽后相邻排列，项间12 DIP，不再各占面板一半。
- 正常绘图区顶部从面板内108 DIP提前至68 DIP，减少40 DIP头部开销；最新区间未知且有原因时保留18 DIP独立提示，绘图区顶部为88 DIP。不把未知变成零，不将旧曲线连接到未知当前区间。
- 保留原文字大小、曲线颜色、第二序列虚线/方形高对比度标记、120秒时间轴及单位；时间刻度文字框限制在面板内，每个面板独立保存DC及裁剪，长标签有界省略而不侵入邻图。
- 四项卡片、图表外框和dashboard布局不改；释放的内部头部空间用于曲线，不删除精确WM_GETTEXT元数据或改变数据缓存。
- 新增两项回归：96/120/144/192 DPI布局参数、最小348 DIP面板及长标签不重叠；真实应用字体测量、浅深/系统色路径、已知/首样本/无快照/缺口绘制、字体/文字色/clip恢复和反复绘图GDI稳定。
- `tools/smoke-overview-dashboard-ui.ps1` 更新过时的760×880最小窗口、main直接子枚举及旧图例区域。改为800×560/800×680/1000×820 DIP、递归宿主2400发现、完整文档/实际宿主宽度/16 DIP尾部与24 DIP右边距检查，保留真实回环mock请求、精确计数/字节/速率、隐藏冻结、主题、启停/重启和配置不变验证。
- 无新增依赖、配置迁移、版本或需求验收范围变化。未执行git提交、建分支或修改标签。

## 验证

| 命令或验收项 | 结果 | 说明 |
|---|---|---|
| `cargo fmt --check` | 通过 | 最终工作区 |
| `cargo check --all-targets --locked` | 通过 | Windows x64 MSVC |
| `cargo test --locked ui::overview_dashboard:: -- --test-threads=1` | 通过 | 14项，含2项新增布局/字体/绘图回归 |
| `cargo test --locked -- --test-threads=1` | 通过 | 372项；219库＋153集成，使用临时数据与本地mock |
| `cargo clippy --all-targets --locked -- -D warnings` | 通过 | 测试模块移至绘制函数后，无告警 |
| `cargo build --locked` | 通过 | 更新Debug |
| dashboard `-ValidateOnly` | 通过 | PowerShell/C#声明及裁图/字体契约辅助自检，不等同目标字体或GUI验收 |
| Debug概览专项 | 已执行项通过，非全验收 | 实际96 DPI；三尺寸几何/浅深/本地21请求及7个HTTP400/字节/区间速率/隐藏冻结/启停/重启/正常退出及配置不变通过；6轮GDI恒89。默认与放大窗口取得完整图表离屏裁图，人工检查代表性浅深图 |
| 静态CRT `cargo build --release --locked` | 通过 | 进程级编码Rustflags及home/项目路径映射，无持久构建配置变更 |
| Release x64 PE32+导入检查 | 通过 | 无额外CRT/SQLite/OpenSSL导入，不能替代干净系统验收 |
| Release概览专项 | 已执行项通过，非全验收 | 同一三尺寸/浅深/真实mock/计数及生命周期检查通过；Complete=false，限制见下 |

### 失败与工具修正

- 新布局测试在125%参数下组合坐标独立取整产生1物理像素差异，最小绘图区高度断言明确容许这一差值；不改变控件布局或降低文字尺寸。
- 定向工具启动时可能先发现main HWND、随后页面控件才创建/重挂。新增完整树readiness，未发现返回零只供等待，严格Control不缓存零；遍历中同一owned HWND因重挂出现两次时去重，所有权和数量上限仍检查。
- 小窗口下直接PrintWindow被父视口裁剪的dashboard输出48.55%洋红未绘制区；保留失败，没有放宽完整性阈值。800×560下改为仅TOP/BOTTOM整窗分段图与完整几何检查，明确跳过完整PNG像素验收，不把它称为文本fallback。
- 默认/放大窗口先滚动揭示完整dashboard，确认其完整原生矩形位于宿主client内，再从整窗PrintWindow按屏幕坐标/窗口非客户区偏移进行1:1裁图；捕获前后几何必须一致，不使用Intersection/缩放/拼图掩盖缺失。
- 跨进程WM_GETFONT句柄不能直接选入harness DC。工具改为只检查目标WM_GETFONT非零；harness拥有的CreateFontW按应用Font::new契约测“预期预算”，不冒充真实目标HFONT身份/内部绘图矩形测量。应用真实字体适配由进程内原生绘图测试验证；harness字体测量资源自检GDI3→3。
- 独立只读review未发现本批排版/未知语义/高对比度标记/DC裁剪确定回归；实际高DPI仍待验收。

## 产物与证据

- 更新 `target/debug/ai-relay.exe` 与 `target/release/ai-relay.exe`；Release为7,860,224字节，版本保持 `2026.10.18`。
- Release SHA256：`FF0C50C7A53298070F16E18904DF5FBFE1B79DE146790DE96C70B690F6F46EE1`。
- Debug报告：`target/overview-dashboard-screenshots/20261019-035822-f03f1fe849d14a1f8f9ec2f2585597cc/results.json`；Release报告：`target/overview-dashboard-screenshots/20261019-041951-96da40a0410d40b9b4249c87f5461360/results.json`。两者Passed=true、Complete=false，非完整产品验收。
- 人工查看Debug默认浅色及放大深色的 `WHOLEWINDOWOFFSCREEN-COMPLETEVIEWPORT-CONTAINEDCROP.png`，图例按名称/速率紧凑排列，单位同行；图像只是离屏证据，不是无遮挡桌面截图。
- 未打包或覆盖dist、显式target旧包；未触碰真实Key、真实模型、剪贴板、自启、防火墙或系统代理。使用本地mock和私有配置，执行后正常退出；未为排版任务改动其他页面。

## 限制与下一步

- 首次≤1秒的未知显示窗口在最终Debug/Release专项运行时已经过去，未强制制造该状态，报告明确跳过；原生绘图回归覆盖该状态。
- 800×560最小外框的实际viewport高437 DIP，小于dashboard完整482 DIP，完整PNG像素检查跳过；正常页面可滚动到达，TOP/BOTTOM及尾部几何通过。默认/放大窗口完整裁图通过。
- 真实150%/200%、多屏、系统高对比度、读屏/物理键鼠及完整图表无障碍仍待验收；120/144/192参数与字体回归不能替代实际桌面DPI。
- 图例数值可能随采样改变宽度；长数字在不足时有界省略，精确数值仍保留在原生等价文本。未增加动画、tooltip或新统计能力。
