# UI 切页与滚动卡顿修复

日期：2026-10-18

## 现象

上一批修复了中间布局帧残留，但实际运行中滚动、切换 Tab 仍有一卡一卡和残影。表现为：

- 每次切页都重新布局全部四个页面，切到任意页都会触发大量子窗口移动；
- 滚动和切页逐个 `SetWindowPos` 移动所有子控件，隐藏页面也被移动；
- `ShowWindow` 逐个显示/隐藏控件，切页时约上百次同步窗口状态变更；
- 概览页标题被错误地当成固定窗口文字重置，滚动时覆盖页面内容；概览 `details` 控件未参与隐藏；
- `标签页` 的“启动/停止代理”按钮在缓存刷新时被重新放回文档坐标，和卡片重叠。

## 实际变更

### 页面布局缓存

`src/ui/mod.rs`：

- 首次布局在窗口显示前一次性预排四个页面；之后记录每页的 `page_geometry`、`page_positioned` 和窗口尺寸/DPI。
- 已定位页面切回时走 `Viewport::refresh`：只更新活动页和滚动信息，不再移动标题、Tab、品牌区、页面控件或状态栏。
- 尺寸或 DPI 变化时清空布局缓存；类别/高级筛选变化通过 `layout(..., true)` 只重排当前页。
- 切页只对离开页和进入页执行显示/隐藏，不再遍历其他页面。
- 标题随概览页滚动，不再作为固定覆盖层；启动/停止按钮只在概览真正需要 arrange 时定位。
- 主窗口增加 `WS_CLIPCHILDREN`，减少背景重绘穿透子控件。

### 批量窗口操作

- 新增 `MoveBatch`：页面布局用 `BeginDeferWindowPos`/`DeferWindowPos` 一次提交几何，使用 `SWP_NOREDRAW`，最终由 arrange 统一重绘。
- 新增 `ShowBatch`：页面显示/隐藏批量提交 `SWP_SHOWWINDOW`/`SWP_HIDEWINDOW`，跳过状态未变化的控件。
- `Viewport` 的滚动/arrange 只移动 `WS_VISIBLE` 子控件；隐藏页面和隐藏分类不参与移动，并使用批量移动。
- 概览页 `show` 补上遗漏的 `details` 标题控件。

## 验证与测量

环境：Windows，Rust/Cargo 1.99.1，800×680、96 DPI，临时配置 debug 实例。

- 24 次连续 Tab 点击 + 滚轮行滚动（同一进程，直接 `SendMessage` 计时）：
  - 切页平均约 **14.7 ms**，最大约 **30.7 ms**；
  - 单行滚动平均约 **6.3 ms**，最大约 **13.9 ms**。
- 对比本轮开始的同类测量：切页平均约 57 ms、最大约 111 ms；滚动平均约 8.4 ms、最大约 22.6 ms。此前还测得切页约 76 ms 的中间版本。
- 压力切换后浅/深色截图未见重复标题、旧页面控件或按钮重叠；概览标题滚动到视口外时不再作为固定层覆盖。
- `cargo fmt --check`、`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test -- --test-threads=1`：**299 项通过**（147 lib + 152 集成）。

## 限制

- 上述毫秒数来自同步 `SendMessage` 的处理器耗时，不等同于 DWM 帧调度、真实鼠标滚轮或触控板手感；仍需在实际窗口、物理输入和多屏 DPI 上验收。
- 首次进入某页仍可能需要一次 arrange；本次压力测试最大值约 30.7 ms，未达到所有帧都低于 16 ms。
- “本次会话”历史子视图会重新布局 history + settings 两个 owner，仍有额外布局成本。
- 页面仍是同一 viewport 下的原生子控件，不是每页一个容器窗口；进一步降低切页成本需要页面容器化，风险更大。
- 未执行 125%/150%/200% DPI、读屏或高对比度 GUI 验收。

## target 清理

本批结束同时清理 `target/` 中旧的验证构建与截图：

- 删除 `compact-ui-build`、各 `*-validation`、`*-validation-build`、`title-menu-build`、`tmp`、`theme-smoke` 等旧目录；
- 删除旧验证截图目录及 `target` 根目录下本轮临时截图/旧测试可执行文件；
- 共删除 55 个旧验证目录/截图/临时文件，并额外删除 `target/debug/incremental`，合计释放约 **59 GB**；`target` 剩余约 **9.7 GB**；
- 保留当前 `target/debug/ai-relay.exe`、`target/release`、`target/package` 和 `target/x86_64-pc-windows-msvc`，未执行 `cargo clean`，避免删掉可直接运行的产物和当前依赖构建缓存。

## 改动文件

- `src/ui/mod.rs`
- `src/ui/viewport.rs`
- `src/ui/overview.rs`
- `src/ui/settings/panel.rs`
- `src/ui/history/layout.rs`
- `src/ui/logs/mod.rs`
- 本记录、`STATUS.md`、开发记录索引
