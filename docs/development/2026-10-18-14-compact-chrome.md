# 紧凑顶栏、单行状态栏与细滚动条

日期：2026-10-18

## 目标

在保持现有控件/命令/无障碍语义的前提下，压缩窗口外框：

- 深色滚动条不再显示为大粗条；
- 底部状态栏改为单行，带状态图标，保留完整 tooltip；
- 顶部 Tab 紧贴标题栏，移除多余间隙，并给 Tab 与标题菜单加轻量图标。

## 实际变更

### 滚动条

`src/ui/scroll_theme.rs`：

- 仍保留原生 `WS_VSCROLL` 的命中区域、拖动、滚轮、键盘和 `SetScrollInfo` 状态。
- 深色模式下把原生滚动条条带填成页面背景色，仅在中间绘制约 6 DIP 的圆角胶囊滑块；不再填满整个 17 DIP 原生宽度，也不绘制粗箭头。
- 悬停/按下仍沿用原生 `STATE_SYSTEM_HOTTRACKED/PRESSED` 颜色。
- 新增 `slim_thumb_is_narrow_centered_and_keeps_native_track` 纯几何回归。
- 浅色继续使用系统滚动条；高对比度保持原生。

### 状态栏

`src/ui/status_bar.rs`、`src/ui/viewport.rs`：

- 高度从 56 DIP 降到 40 DIP，单行布局。
- 左侧按状态绘制图标：运行三角、停止方块、进行中三点、错误叹号；颜色继续区分运行/停止/忙碌/错误。
- 右侧显示省略后的详细消息，状态与消息之间用细竖线分隔；完整原文仍写入原生 STATIC 并保留 tooltip，因此无障碍文本不缩短。
- 顶部保留 1 DIP 分隔线，浅/深/高对比度沿用现有配色。
- `Viewport::TOP` 从 48 改为 44，`FOOTER` 引用状态栏高度；页面文档坐标/scroll extent 同步调整。

### 顶部 Tab 与菜单

`src/ui/tabs.rs`、`src/ui/theme.rs`、`src/ui/title_menu.rs`、`src/ui/mod.rs`：

- Tab 控件从 client y=12、36 DIP 高改为 y=0、44 DIP 高，紧贴标题栏并与页面视口衔接；不再保留标题栏下方的空白。
- 四个 Tab 使用共享 `tabs::LABELS`：`● 概览`、`▥ 统计分析`、`≡ 实时日志`、`◆ 设置`；原生/自绘两条绘制路径共用同一组标签，避免文案漂移。
- Tab item 宽度改为 112 DIP，容纳图标与中文。
- 标题菜单的“代理”“视图”项增加对应符号：启动/停止/重启以及四个页面。

## 验证

- `cargo fmt --check`：通过。
- `cargo check --all-targets`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test -- --test-threads=1`：**298 项通过**（146 项 lib + 152 项集成）。
- 用临时配置启动最新 debug 构建，800×680、96 DPI：
  - 浅色概览/设置页：Tab 图标、无标题栏间隙、单行状态栏；
  - 深色设置页：细胶囊滚动条、单行状态栏；
  - 切换页面与滚动后未见上一布局残影；窗口尺寸变化的残影回归见 13 记录。
- 未执行物理输入、多屏、高对比度截图、读屏或 125%/150%/200% DPI 专项验收；这些不能由上述截图替代。

## 限制

- 滚动条仍是原生 17 DIP 命中带，**视觉**滑块 6 DIP；没有替换为自绘滚动控件，因此保留原生拖动/键盘/无障碍，但不改变布局占用的条带宽度。
- 深色模式使用细胶囊覆盖；浅色沿用系统滚动条，Windows 10/11 外观可能不同。
- Tab/菜单图标为字体符号，极端系统字体或缺失符号时可能回退显示；没有引入图片资源或字体依赖。
- 状态栏详细消息被省略时依赖 tooltip；tooltip 的自动悬停/读屏验收仍按 STATUS 未完成项处理。

## 改动文件

- `src/ui/scroll_theme.rs`
- `src/ui/status_bar.rs`
- `src/ui/viewport.rs`
- `src/ui/tabs.rs`
- `src/ui/theme.rs`
- `src/ui/title_menu.rs`
- `src/ui/mod.rs`
- `docs/UI-DESIGN.md`
- 本记录、`STATUS.md`、开发记录索引
