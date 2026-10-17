# 0006：主题公开API与高对比度降级

- 日期：2026-10-17
- 状态：已接受
- 关联：需求8.1（v1.4）

使用原生浅/深/系统三模式，serde默认System兼容旧配置；设置即时预览，持久化复用后台正常保存。通过公开Registry读取AppsUseLightTheme，WM_SETTINGCHANGE/WM_THEMECHANGED重查；系统高对比度优先GetSysColor，恢复原生主题。系统设置查询不修改系统配置，不进行应用文件读写。

公开SetWindowTheme、GDI画刷和必要Tab页头绘制搭配原生控件，DWM属性仅支持时使用，失败降级；不调用未文档化UxTheme函数，不引入WebView/WinUI运行时。Common Controls v6通过manifest声明。

主题资源RAII，回调避免持有可变State引用跨同步重入。原生控件保留键盘和可访问性结构。菜单/消息框/滚动条/下拉箭头/复选框方框等仍可保持系统外观，不能宣传全控件暗色。真实高对比度、多屏DPI和读屏仍须人工验证。
