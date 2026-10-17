# 0002：配置与原生宿主依赖路线

- 日期：2026-10-17
- 状态：已接受（本批实施方案）
- 关联：需求第 6、8、12 节；ADR 0001

## 背景

配置基础和原生窗口需要可靠解析、校验及 Win32 FFI；图标和 DPI/权限声明应随 EXE 内嵌。本次工具链已验证为 Rust/Cargo 1.99.1 MSVC。

## 决策

- `serde 1` + `toml 0.8`：显式版本配置、严格字段类型及序列化。
- `url 2`：HTTP(S) 上游与 Origin 语法校验。
- `ipnet 2`：IPv4/IPv6 CIDR 校验和访问判断。
- 仅 Windows 使用 `windows-sys 0.61`：直接调用原生窗口、托盘及 DPI API，按 feature 限制范围。
- `winresource 0.1` 仅用于构建：使用 Windows SDK 资源工具嵌入 ICO 和 manifest，不成为部署运行时。
- 图标用 SVG 保存几何源，PowerShell/System.Drawing 脚本生成多尺寸 ICO；用户运行成品无需 PowerShell 生成图标。
- manifest 声明 asInvoker 和 PerMonitorV2，不默认申请管理员权限。
- 依赖精确版本由 Cargo.lock 固定，锁文件由 Cargo 生成。

## 备选方案

- 手工解析 TOML/URL/CIDR：增加协议边界错误，不采用。
- 更高层 GUI 框架：暂不需要；直接 Win32 保持当前轻量路线。
- 构建时自动生成图标：会引入构建宿主绘图要求；改为提交生成资源，脚本仅维护时使用。

## 影响与风险

- url 的国际域名支持带来传递依赖；需观察正式核心加入后的体积，不以当前小 EXE 推断最终体积。
- windows-sys 是低层绑定，必须显式处理返回值、句柄所有权和消息回调重入。
- 最低 Rust 版本未针对完整依赖验证；目前只确认 1.99.1。
- 当前控制台子系统暂保留，后续 GUI 子系统需与 CLI 输出和启动错误处理一起实现。
- DPAPI、配置原子替换、迁移、TLS 和异步网络依赖尚未实施，不由本 ADR 隐式确定。

## 验证

整项目格式、检查、测试、Clippy 和 Release 构建通过。实际 EXE 已验证图标与 manifest 资源存在。真实托盘交互、多屏 DPI 及干净系统运行库验收尚未完成。
