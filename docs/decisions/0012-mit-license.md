# 0012：采用 MIT 项目许可证

- 日期：2026-10-18
- 状态：已接受（用户明确选择 MIT）
- 关联：ADR0011 的许可证未决事项；需求14.4；开发记录2026-10-18-20

## 背景

公开准备时项目未选择许可证，不能把公开可见误认为已授予使用、修改和分发权。用户现明确选择MIT。

## 决策

- 根目录 `LICENSE` 使用完整标准MIT文本，版权声明为 `Copyright (c) 2026 rachpt`。
- Cargo包使用SPDX标识 `license = "MIT"`，保留既有 `publish = false`；GitHub开源不要求发布到crates.io。
- README链接许可证，项目原创导航SVG及生成资源同样适用MIT。第三方组件依然遵循自身许可，不被项目MIT重新授权。
- 发布脚本在构建前检查LICENSE存在，并显式复制原文到ZIP白名单。已有dist包不自动覆盖或重建。

## 影响与风险

使用、复制、修改、分发等权利与无担保条款以LICENSE英文原文为准；复制或分发全部或重要部分时需保留版权及许可声明。项目许可选择不等于依赖许可证审查完成，不替代第三方版权/通知、干净系统或正式发行验收。

## 验证及未决事项

Cargo metadata确认MIT；许可证关键条款/版权声明、PowerShell语法及LICENSE复制/白名单验证通过，fmt和all-targets check通过。本批没有依赖或Rust行为变化，不修改Cargo.lock；未执行release打包、GUI或全量测试，未新增第三方notices。
