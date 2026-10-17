# 架构决策记录（ADR）

## 索引

- [0001：Windows 原生技术路线与模块边界](0001-windows-native-foundation.md)

- [0002：配置与原生宿主依赖路线](0002-foundation-dependencies.md)

- [0003：反向代理运行时与配置提交边界](0003-proxy-runtime-persistence.md)

- [0004：单实例信任边界与被动用量统计](0004-instance-usage.md)

- [0005：原生分页与有界日志后台](0005-tabs-logging.md)

- [0006：主题公开API与高对比度降级](0006-native-themes.md)

- [0007：DPAPI原子配置、SQLite历史与迁移](0007-secure-config-history-migration.md)

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
