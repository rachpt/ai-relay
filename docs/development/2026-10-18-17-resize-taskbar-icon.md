# 拖动缩放残影与任务栏图标修复

日期：2026-10-18

## 现象

1. 拖动改变窗口大小后，设置页出现重复“监听 IP / 端口”标签、按钮被空白区遮住；日志页出现重复按钮行、筛选区文字重叠。
2. 任务栏按钮显示成标题栏的 ☰ 汉堡图标，而不是应用图标。

## 原因

### 缩放残影

- 页面布局使用批量 `SWP_NOREDRAW` 移动控件，滚动/arrange 只在最终位置失效宿主背景；
- 拖动期间控件不断换位，旧的子控件像素没有被完整擦除；
- `WM_EXITSIZEMOVE` 只保存窗口偏好，没有做一次整页重绘，因此拖动结束后残影仍留在屏幕上。

### 任务栏图标

- `title_menu::CaptionIcon` 把窗口 `WM_SETICON ICON_SMALL` 改成汉堡图标，供标题栏菜单使用；
- Windows 任务栏使用了这个 small icon，于是任务栏也变成 ☰。

## 修复

- `WM_EXITSIZEMOVE` 调用 `State::repaint_all`：先强制布局当前页（`layout(..., true)`），再对 viewport 执行
  `RedrawWindow(..., RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN)`，并重绘主窗口固定区；只拖动期间保留批量无重绘，结束后强制完整刷新。
- 标题栏不再安装汉堡 caption icon：`TitleMenu` 增加可选 `install_caption_icon`，应用设为 `false`；测试默认仍为 `true` 以保留原有图标回归。
- 在客户端头部左侧新增 `☰` 静态按钮（ID 2110），点击走同一 `OPEN_TITLE_MENU` 路径；品牌图标/名称和 Tab 起点相应右移。
- 窗口 `ICON_BIG` 显式设置为应用图标；标题栏/任务栏保留应用图标，菜单入口移到客户端按钮。
- 新增 `client_menu_mode_keeps_window_and_taskbar_icons` 回归：关闭 caption icon 后，`ICON_SMALL`/`ICON_BIG` 不因 attach/refresh/drop 改变。

## 验证

- `cargo fmt --check`、`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`：通过（源码验证；运行中的旧实例锁定 `target\debug\ai-relay.exe`，未强行结束）。
- 使用独立 `--target-dir` 构建并执行全量串行测试：**300 项通过**（148 lib + 152 集成）。
- 最新二进制已生成到 `target\debug\ai-relay-next.exe`，因为旧实例仍占用 `ai-relay.exe`。
- 未能在旧实例运行期间做新二进制的真实拖拽/任务栏截图；需关闭旧实例后运行 `ai-relay-next.exe` 或重新 `cargo build`，再确认物理拖拽和多屏/高 DPI。

## 限制

- `WM_EXITSIZEMOVE` 覆盖鼠标拖拽；键盘/程序化 resize 若未发送该消息，仍依赖后续布局/repaint。
- 客户端 ☰ 使用字体符号，极端系统字体可能回退；没有引入图片资源。
- 原生标题栏标题文本仍为空；应用图标由 caption/class 图标提供，产品名在客户端品牌区。
- 未执行物理多屏、150%/200% DPI、读屏和任务栏固定/分组场景验收。

## 改动文件

- `src/ui/mod.rs`
- `src/ui/title_menu.rs`
- `docs/UI-DESIGN.md`
- 本记录、`STATUS.md`、开发记录索引
