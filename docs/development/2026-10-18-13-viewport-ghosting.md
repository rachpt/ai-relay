# 滚动页面残影与重复绘制修复

日期：2026-10-18

## 现象

设置页在滚动、切换分类或点击主题后，会保留上一帧的控件内容：截图中“外观主题”行和“保存并应用”按钮各出现两次，偏移约 48 DIP，且有文字被上一帧覆盖。历史、日志和概览页共享同一个滚动宿主，因此同样存在风险。

## 原因

问题不在模块拆分，也不在控件自身的绘制代码，而在滚动宿主的两次定位：

1. 各页面 `layout()` 先把控件摆到文档坐标；这一步使用 `MoveWindow(..., bRepaint = TRUE)`，会立即重绘。
2. `Viewport::arrange()` 再把控件从文档坐标平移到视口坐标：减去固定 48 DIP 顶部和当前滚动 offset。
3. 第二次移动后没有让宿主背景失效。第一次重绘留下的是 48 DIP 偏移的中间帧，因此形成清晰、持久的“残影”。

另外，所有页面布局在第一次定位时都强制重绘，随后又被 `arrange()` 移动一次，造成每次布局/切页/改变分类时的重复绘制，放大闪烁和性能开销。

## 修复

- `src/ui/viewport.rs`：`WM_APP + 82` 完成最终定位后调用 `InvalidateRect(hwnd, null(), 1)`，由滚动宿主擦除旧的中间帧。
- `src/ui/overview.rs`、`src/ui/settings/panel.rs`、`src/ui/logs/mod.rs`、`src/ui/history/layout.rs`：页面布局的 `MoveWindow` 改为不立即重绘，最终位置由 `arrange()` 统一移动并重绘。
- 新增原生回归 `native_arrange_invalidates_host_after_position_pass`：离屏显示测试窗口后，验证 `arrange` 会留下非空 update region，防止后续再次删除宿主失效逻辑。

## 字体

未改字号。正文目前是设计稿明确的 14 DIP、标题 20 DIP（`Font::new`/`Font::heading`），并按每显示器 DPI 缩放；因此 125% 下正文约 17.5 px，150% 下约 21 px，比 Windows 默认 9pt 正文更大。这是既定设计，不是 DPI 重复缩放。若需要更接近系统默认，应作为独立批次把正文调整到 13/12 DIP，并完整回归布局与可读性。

## 验证

- `cargo fmt --check`：通过。
- `cargo check --all-targets`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test -- --test-threads=1`：**297 项通过**（145 项 lib 测试 + 152 项集成测试）。
- 新增的 viewport 原生测试覆盖 `arrange` 后的 update region。
- 用临时配置目录启动最新 debug 构建，切换到设置页、切换分类、滚动并调整窗口大小后抓取窗口截图；本次未再出现重复行或上一布局残影。临时进程已结束，未使用真实配置、Startup 或用户数据。
- 该截图验证仍不等同完整 GUI 验收：物理滚动/点击、多屏、DPI 与可访问性仍需在本机继续确认。

## 改动文件

- `src/ui/viewport.rs`
- `src/ui/overview.rs`
- `src/ui/settings/panel.rs`
- `src/ui/logs/mod.rs`
- `src/ui/history/layout.rs`
- `docs/UI-DESIGN.md`
- 本记录及 `STATUS.md`、开发记录索引
