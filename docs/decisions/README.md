# 架构决策记录（ADR）

## 索引

- [0001：Windows 原生技术路线与模块边界](0001-windows-native-foundation.md)

- [0002：配置与原生宿主依赖路线](0002-foundation-dependencies.md)

- [0003：反向代理运行时与配置提交边界](0003-proxy-runtime-persistence.md)

- [0004：单实例信任边界与被动用量统计](0004-instance-usage.md)

- [0005：原生分页与有界日志后台](0005-tabs-logging.md)

- [0006：主题公开API与高对比度降级](0006-native-themes.md)

- [0007：DPAPI原子配置、SQLite历史与迁移](0007-secure-config-history-migration.md)

- [0008：历史读写隔离、日历报表与窗口偏好](0008-history-read-isolation.md)

- [0009：原生标题栏紧凑菜单入口](0009-native-caption-menu.md)

- [0010：SVG设计源与内嵌原生导航图标](0010-native-navigation-assets.md)

- [0011：公开准备与历史卫生](0011-publication-history-hygiene.md)

- [0012：采用MIT项目许可证](0012-mit-license.md)

- [0013：原生控件视觉统一与输入框边界](0013-native-control-polish.md)

- [0014：暗色滚动条鼠标跟踪与日志高度调整](0014-scrollbar-tracking-log-resize.md)

- [0015：公开API深色标题／系统菜单](0015-dark-title-menu.md)

- [0016：保留原生布局的深色日期框与月历](0016-dark-native-date-picker.md)

- [0017：暗色统计表滚动的局部合成呈现](0017-dark-report-scroll-presentation.md)

- [0018：原生深色月历颜色与翻月呈现](0018-native-dark-calendar-navigation.md)

- [0019：明暗视觉层级与原生导航展示隔离](0019-fluent-presentation-tokens.md)

- [0020：概览有界整帧呈现与月历宿主边沿](0020-bounded-dashboard-popup-chrome.md)

- [0021：浅深滚动条、小图形抗锯齿与类型化设置输入](0021-light-scrollbars-small-shapes-settings.md)

## 使用规则

- 文件名使用递增四位编号，例如 `0002-config-discovery.md`。
- 记录背景、决策、备选方案、影响、状态与关联需求。
- 状态可为：提议、已接受、已取代。
- 未经确认的依赖路线和安全策略不能写成已经实现或验证。
- 取代旧决策时新增 ADR，并在两者之间建立链接；不静默重写历史依据。
- ADR 不能隐式改变需求；涉及功能或验收变化时同步修订需求文档。

## 模板

```text
# NNNN：标题

- 日期：YYYY-MM-DD
- 状态：提议 / 已接受 / 已取代
- 关联：需求章节、旧 ADR

## 背景
## 决策
## 备选方案
## 影响与风险
## 验证及未决事项
```
