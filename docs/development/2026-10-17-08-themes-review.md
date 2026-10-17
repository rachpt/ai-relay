# 2026-10-17-08：主题与代码审查修复

## 目标与实际变更

用户要求review并优化现代Windows外观，增加浅/深/系统主题。独立agent静态审查后台、安全和统计，另agent实现主题；随后分别修复有证据的问题。

- ui/theme：三模式、系统变化通知、高对比度优先、公开DWM标题栏/圆角和控件配色，画刷/字号RAII；设置预览保留草稿，正常保存恢复。
- config：ThemeMode默认System，旧配置兼容。manifest加入Common Controls v6；无新增crate，仅扩展windows-sys现有feature。
- proxy：10秒入站请求头读取超时，避免未认证慢连接永久耗尽槽，不限制SSE生命周期。
- persistence/controller：记录加载基线，外部合法修改/创建/删除后拒绝覆盖；比较配置语义，注释格式不作为冲突，不承诺防同用户恶意race。
- controller结果邮箱有界淘汰且丢失提示独立保留，不再阻塞IPC stop/quit。
- portable保存提交后先处理标记，日志失败不会跳过定位信息，各部分失败合并说明。
- usage：response.failed/incomplete提取已报usage/model并标终态，错误/截断状态独立，重复终态不覆盖。

## 验证

最终cargo fmt --check、cargo check --all-targets --locked、cargo test --locked、cargo clippy --all-targets --locked -- -D warnings、cargo build --release --locked通过。

新增回归验证慢header槽回收且SSE不中断、配置外部冲突、队列满IPC响应、便携日志失败标记、失败Responses用量、主题默认/高对比度优先/缩放布局。

Common Controls v6后主题smoke连续两次通过：预览不写盘、草稿保留、系统通知、保存/加载/重启，GDI40→41无持续增长迹象；不代表长期资源验收。日志RequireAccessLog和代理GUI烟测通过。仅临时目录和本地mock，不修改系统主题、高对比度、自启或防火墙。

曾出现主题脚本finally清理错误掩盖原始等待竞争，已修复保存原error、等待自己child退出、有限清理重试及初始化等待；最终退出清理成功。截图也暴露v6复选框/下拉框颜色问题，修复SetWindowTheme参数后已查看深色设置、浅色概览等截图确认可读。

## 剩余限制

当前只是更清晰的轻量Win32界面，不是完整WinUI。菜单、消息框、滚动条、下拉箭头和复选框方框仍保留系统/经典细节。实机100%DPI，150/200%仅布局单测，真实高对比度、多屏、键盘和读屏未验收。

review修复不意味着穷尽全部缺陷；性能、长稳及原有未实现功能见STATUS。未初始化Git或提交，未处理旧脚本秘密。
