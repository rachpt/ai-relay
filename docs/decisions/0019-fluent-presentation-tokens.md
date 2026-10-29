# 0019：明暗视觉层级与原生导航展示隔离

- 日期：2026-10-29
- 状态：已接受
- 关联：需求8.1；补充ADR0006／0013，仅取代ADR0013的6 DIP页内圆角值

## 背景

用户要求继续精修浅深主题并贴近当前Windows规范。既有蓝灰表面、逐处强调色和6 DIP页内圆角可进一步收敛；主Tab展示直接依赖应用State，增加悬停与原生焦点绘制时应避免扩大同步重入边界。

## 决策

- 延续原生控件＋有限owner/custom draw及公开DWM API，不引入WinUI、WebView、第三方主题库或联网资源。高对比度优先系统绘制／系统色，Windows10不支持的DWM属性失败不阻止运行。
- 参考微软 [Color](https://learn.microsoft.com/en-us/windows/apps/design/style/color)、[Geometry](https://learn.microsoft.com/en-us/windows/apps/design/style/rounded-corner)、[Typography](https://learn.microsoft.com/en-us/windows/apps/design/style/typography)指南：浅深均用中性画布和抬升表面，强调色集中于主动作／选中指示／数据曲线；页内按钮与闭合Combo为4 DIP圆角，卡片约8 DIP。Edit及日期框继续保留完整矩形边线，不以裁剪冒充圆角输入。
- 简体中文仍使用Microsoft YaHei UI及既有14／20 DIP层级，不仅为西文字体建议更改中文排版。固定应用强调色不声称自动使用系统强调色。
- 主Tab展示移入tabs模块，以独立Rc保存复制主题、hover及图标缓存；绘制前完成原生查询并校验实例／revision，绘制中不持应用State引用。原生选择、焦点、通知及消息先正常执行；过期快照或SaveDC失败不继续绘制。
- 不更改代理、配置、统计、确认、持久化或输入行为；主题切换不重置草稿、选区或重启代理。

## 备选方案

- 仅替换Palette：无法解决焦点、禁用、表头或层级细节，本批同时调整有限展示层。
- 完整WinUI／Mica／Acrylic或全自绘输入：部署和兼容性／可访问性成本超出当前范围，不采用。
- 直接裁剪Edit／日期框圆角：可能切断边线或影响输入，不采用。

## 影响与风险

圆角变更需同步真实HRGN与GUI工具契约；中性徽标及上传曲线不应再被旧“必须有彩色墨迹”断言拒绝，但仍须保留图标／文字／曲线和完整截图保护。状态不只通过颜色表达。图像粗检不是OCR、实际高DPI或读屏证明。

## 验证及未决事项

最终448项测试通过、2项物理测试默认ignored；fmt、全目标check、严格Clippy及Debug／静态CRT Release构建通过。两构建96 DPI四页双主题双尺寸及分类／统计子视图矩阵通过；概览定向检查通过限定范围。实机高DPI、多屏、真实高对比度、IME／读屏及既有专项限制保持未完成，详见[开发记录](../development/2026-10-29-01-fluent-theme-polish.md)。
