# 客户端头部程序图标与名称

日期：2026-10-18

## 目标

在程序顶部菜单按钮（☰）之后提供清晰的程序标识：应用图标 + `AI Relay` 名称，同时不改变现有标题栏菜单、Tab 导航和窗口生命周期。

## 实际变更

`src/ui/mod.rs`：

- 在客户端顶部新增品牌区：左侧为应用 HICON（来自现有 EXE 图标资源），右侧为 `AI Relay` 标题文字；使用现有 `heading_font`，不引入图片文件或新字体依赖。
- 品牌区是固定头部控件，不进入滚动视口；`Viewport::adopt` 将它和 Tab、状态栏一起保留在宿主层。
- `State::layout` 按 DPI 定位品牌图标/名称，并把四个 Tab 的起点从 24 DIP 右移到 220 DIP；新增布局回归 `brand_header_fits_before_all_four_tabs_at_minimum_width`。
- `update_dpi` 在替换标题字体时同步更新品牌名称，避免 125%/150%/200% 缩放后字号不一致。
- 没有改动原生标题栏的 ☰ 菜单和系统菜单接入；品牌区位于原生标题栏下方的客户端头部，避免与标题栏唯一的小图标槽争用。

`src/ui/tabs.rs`：

- `Tabs::layout` 增加 `left` 参数，由头部品牌宽度决定 Tab 起点；Tab 项尺寸仍为 112×36 DIP，控制高度 44 DIP。
- 四个 Tab 的图标与名称继续使用共享 `tabs::LABELS`。

## 视觉效果

- 浅色/深色下均显示 `[应用图标] AI Relay`，随后是 `● 概览`、`▥ 统计分析`、`≡ 实时日志`、`◆ 设置`。
- 800×680、96 DPI 时品牌区结束于约 214 DIP，Tab 从 220 DIP 开始，四个 Tab 在最小宽度内完整显示。

## 验证

- `cargo fmt --check`：通过。
- `cargo check --all-targets`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test -- --test-threads=1`：**299 项通过**（147 项 lib + 152 项集成）。
- 用临时配置启动最新 debug 构建，在浅色和深色 800×680 窗口抓图检查品牌图标、名称、Tab 间距和状态栏；未使用真实配置/Startup。

## 限制

- 原生标题栏仍保留 ☰ 菜单和空白 caption 文本，品牌标识位于客户端头部；不是完整自绘标题栏，也没有把 ☰ 移入客户端。
- 品牌图标使用现有 EXE 资源，若资源缺失会退化为空 STATIC；未增加备用图片资源。
- 未做 125%/150%/200% DPI 的实机截图、物理输入、多屏或读屏验收。
- 品牌区占用约 196 DIP 宽度，最小 800 DIP 窗口下 Tab 仍完整；更窄窗口不在当前最小尺寸承诺内。

## 改动文件

- `src/ui/mod.rs`
- `src/ui/tabs.rs`
- `docs/UI-DESIGN.md`
- 本记录、`STATUS.md`、开发记录索引
