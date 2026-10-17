# AI Relay

Windows 原生 AI Provider 流量转发工具，**Rust + Win32**，支持托盘、单实例、流式代理和本地用量分析。

> 当前为功能开发预览，不是全部验收完成的正式版本。未完成项目与验证限制见 [STATUS](docs/development/STATUS.md)。旧 JS/CMD/PowerShell 保留，仅作参考，不属于原生版发布包。

## 运行

- 开发构建：`cargo run`。
- Release：`target/release/ai-relay.exe`。
- 静态 CRT 便携预览包：`dist/ai-relay-0.1.0-windows-x64-preview.zip`（构建后生成）。
- 普通双击不创建控制台；关闭主窗口隐藏至托盘，双击托盘恢复。再次启动唤醒同一用户会话的原实例。

### 首次设置

在“设置”页填写上游 URL、监听 IP/端口、Key 与注入策略，选择软件目录或 Windows LocalAppData。

- **保存并应用**：保存后启动，运行中受控重启；失败显示真实状态，可明确确认回滚上一配置。
- **启动已保存配置**：只使用已保存设置，不使用界面草稿。
- 默认仅回环；GUI 非回环监听每次单独确认，CLI/自动启动不复用授权。
- 默认 DPAPI 当前用户保护，密文与配置一起原子保存。不能保证跨用户/跨机器解密；明文便携需确认风险，备份同样敏感。
- 登录自启是当前用户登录后运行，不是服务；在设置中显式启用/关闭，显示系统实际状态。测试未修改真实 Startup。
- 迁移需先停止代理，预览并确认。复制配置及一致历史快照，保留源数据，日志不迁移；目标已有历史时拒绝自动合并。可识别的已启用自启同步更新。
- 支持脱敏配置导出、导入预览、备份恢复；不覆盖预览后外部改变的配置。

## 四个页面

### 概览

显示运行状态并提供启停。当前完整连接/流量仪表盘尚未完成。

### 统计分析

历史 SQLite 数据跨重启保留，与当前程序会话 usage 分开：

- IP、响应模型、UTC 日期/范围、客户端与实际发送的上游凭据 ID、实际目标 origin、协议、接口、状态与结果组合筛选。
- 最多三级分组、分页明细和原生每日 Token 趋势图。
- 输入/输出/总量、缓存/推理，以及未知、部分、截断、错误、队列丢失等覆盖状态。
- 凭据用本地 HMAC-SHA256 ID，不保存原始 Key；完整64位ID可直接筛选，不接受Key片段。
- CSV 当前只导出筛选后的**当前明细页**，默认移除 IP 和凭据 ID，不等于全量导出。
- 默认保留90天、最多10万条；当前尚未开放完整保留策略设置。

支持 OpenAI Chat/Responses 与 Anthropic JSON/SSE usage；不修改请求强制索取用量。未知不是0，不保存正文，不以统计冒充账单。请求模型目前不提取，别名管理、本地时区/月报及可排序表格仍待完善。

### 实时日志

实时查看、搜索/级别筛选、暂停/恢复、清空视图；当前/历史文件尾读、向前分页、自动跟随。文件替换/截断/缺失会停止跟随并提示重新读取。

日志目录为所选数据目录的 `logs`，JSONL，支持文件/访问日志开关、轮转、保留数量/天数及缓存限制。应用日志设置即时生效，正常保存配置后持久化。

清空视图不删文件；清理先预览确认，全部清理二次确认，部分失败逐项报告。不删除配置、历史数据库或旧版日志。未知路径脱敏，不记录Key、正文或查询参数。

### 设置

分类管理上游与监听、访问控制、正向代理/CONNECT/CORS、网络限制、启动/存储/配置、外观/日志。浅色、深色、跟随系统默认跟随；高对比度优先系统配色。原生菜单、滚动条等不保证完全暗色。

## 协议与安全

HTTP/HTTPS 反向代理及流式 SSE；可选 HTTP 正向代理、CONNECT 和 CORS。关闭正向改写时仅可信 origin 注入上游 Key；CONNECT 是不解密的 TCP 隧道。支持独立客户端令牌、IP/CIDR 白名单、超时、并发上限和取消。

不关闭 TLS 校验，不跟随上游重定向，不自动重试可能发出的 POST；不自动修改系统代理或防火墙。

## 命令行

```text
ai-relay.exe [--background] [--config-dir <绝对目录>]
ai-relay.exe start|restart|status|stop|show|quit
ai-relay.exe --help
ai-relay.exe --version
```

无主实例时 start/restart 创建后台实例，限时等待真实监听。退出码：0成功、1未运行/监听、2参数/配置错误、3操作或IPC失败。quit回执仅表示已接受优雅退出。

## 开发与验证

Windows x64，当前验证 Rust/Cargo 1.99.1 MSVC，需要 C++ Build Tools、Windows SDK、rustfmt、Clippy。最低Rust/Windows构建号尚待完整验证。

```text
cargo fmt --check
cargo check --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
```

Windows PowerShell 冒烟使用临时目录、本地mock和虚构凭据，不产生模型费用：

```powershell
.\tools\smoke-proxy-ui.ps1
.\tools\smoke-history-ui.ps1
.\tools\smoke-settings-ui.ps1
.\tools\smoke-logs-ui.ps1 -RequireAccessLog
.\tools\smoke-theme-ui.ps1
.\tools\smoke-cli.ps1
.\tools\package.ps1
```

图标源为 `assets/ai-relay.svg`，`tools/generate-icon.ps1` 生成 ICO。打包仅纳入白名单文件，不包含旧脚本、配置、密钥、日志或数据库；静态CRT+PE导入检查不能代替干净系统、签名和许可证验收。

## 文档

[需求](REQUIREMENTS.md) · [约束](AGENTS.md) · [导航](docs/README.md) · [状态](docs/development/STATUS.md) · [架构](docs/ARCHITECTURE.md) · [开发指南](docs/DEVELOPMENT.md) · [便携说明](docs/PORTABLE.md)
