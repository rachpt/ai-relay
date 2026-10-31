# 2026-10-31-01：浅深滚动条、平滑图标与设置输入布局

## 目标与范围

按用户截图改善浅色滚动条、switch及状态图标锯齿，修正Release“关于”固定开发预览，增加上游Key眼睛显隐，并优化设置输入约束、宽度与布局。保持Rust＋Win32单包、原生输入／文本／表格、现有六分类及显式保存，不修改代理协议／配置schema、依赖／Cargo.lock、版本／标签或dist。

## 实际变更

- `src/ui/scroll_theme.rs`及`scroll_theme/`：Light／Dark共用窄胶囊、完整原生命中区、32位滚动和公开API capture／repeat适配；Report沿用局部COMPOSITED、NC排除条带及短DC呈现。Contrast卸载自有跟踪／样式位并恢复native；selection／keyboard／wheel／range／page不替换。跨模块复制与日志按需显隐测试同步新主题契约。
- `src/ui/raster.rs`、`control_theme.rs`、`status_bar.rs`：小图形使用4×4覆盖采样及32-bit DIB，switch38×20 DIP、状态图标保持16 DIP；文本仍用原生字体。无全局缓存，单边／总像素／图元／顶点／采样评估有硬上限，分配可失败，GDI可读回退。复杂clip、映射、借用打印DC和GDI生命周期有回归，高对比度保留native switch及系统色状态图形。
- `src/ui/settings/secret.rs`：上游Key旁增加眼睛＋中文可访问按钮，通过EM_SETPASSWORDCHAR切换，不读取／替换秘密正文、不保存或重启。默认掩码，切分类／主Tab、隐藏／最小化／关闭、populate／放弃草稿／禁用操作恢复掩码，保留UTF-16选区、undo及草稿。
- `src/ui/settings/numeric.rs`、`draft.rs`：端口／六个网络值共用范围元数据，字符／IME字符／选区替换／Ctrl+V／Shift+Insert／菜单粘贴拒绝非ASCII数字及超长结果，不截断、不clamp。空／范围外草稿可编辑，保存明确拒绝。端口1–65535，网络维持既有u32上限，只有两个idle允许0，未擅自收窄合法配置。
- `src/ui/settings/layout.rs`、`panel.rs`：短字段按类型限宽，URL／Key／文本列表伸展；IP最大256、端口108、注入／存储下拉320、密钥保存下拉440、CONNECT列表220 DIP。网络内容680 DIP起两列、以下单列，数值框148 DIP；动作区688 DIP以下两行，原生Tab顺序与表单对应。
- `src/ui/mod.rs`：接眼睛owner-draw及窗口遮罩生命周期，响应式设置高度在原生滚动条改变客户区宽后重算并参与缓存；关于使用Cargo版本＋Debug开发构建／Release构建，Release代码路径不再带固定开发预览／开发版标记。编译profile不代表正式发行验收。
- `tools/smoke-modern-ui.ps1`：将旧普通输入全宽断言替换为逐字段确切类型规则，保留URL实际客户区24 DIP右边距、无重叠／尾端／身份／超时guard；增加7数字框整段拒绝及空私有Key显隐／离页重掩码检查，不读取显露正文、不操作剪贴板、不隐式保存。
- 同步README、需求v1.8、UI设计与状态；[ADR0021](../decisions/0021-light-scrollbars-small-shapes-settings.md)只替代旧Light-native滚动主题范围，历史决策及失败证据保留。

## review及先失败后修复

三agent按scroll／settings／小图形分离写集，主agent接线及跨模块验证，交叉只读review。初轮没有确定新增P1／P2；末轮专项发现数字输入仍可能在同步销毁／延迟剪贴板回调后使用旧attached快照，作为P2修复：保留共享owner并在每次可重入查询后及最终分发前重查attached，不以IsWindow替代附件身份。回归15个查询／提示写入销毁场景＋2个模拟剪贴板回调，修复前提示仍收到1次WM_SETTEXT而失败，修复后0次通过；另一agent定向review无确定新增P1／P2。

初轮真实设置测试18／19，125%参数下输入框底231px、提示顶230px重叠1px，175%同样重叠1px。根因为top／height分别舍入与DIP总坐标舍入不一致；改按实际像素底边＋4 DIP定位，保留严格两两不重叠断言并增加上下间距与详细矩形错误。修复后19／19；重入补强后最终20／20。

另agent曾把隔离源码快照通过`--target-dir target`共享主构建目录，是不完整构建隔离。普通Cargo一度报告Fresh且只运行旧9项设置测试，EXE时间仍为前一天，与磁盘新源码不符；明确不计作本批新证据。共享fingerprint／incremental是已确认干扰因素，未单独证明唯一根因。后续统一主工程、`CARGO_INCREMENTAL=0`重编，真实新测试及最终Release均以此验证；未清理用户文件、强杀其他构建或修改持久环境。

## 验证命令与结果

本批在Windows x64 MSVC上使用锁定／离线依赖，网络集成仅回环mock，无真实Key或模型计费。

| 命令／范围 | 实际结果 |
|---|---|
| `cargo fmt --all --check` | 通过，最终补强后复查 |
| `CARGO_INCREMENTAL=0 cargo check --all-targets --locked --offline` | 通过，最终补强后复查 |
| `CARGO_INCREMENTAL=0 cargo test --lib ui::settings:: --locked --offline -- --test-threads=1` | 最终20通过，14.09秒 |
| `CARGO_INCREMENTAL=0 cargo test --lib ui::tests:: --locked --offline -- --test-threads=1` | 11通过，88.69秒 |
| `CARGO_INCREMENTAL=0 cargo test --lib ui:: --locked --offline -- --test-threads=1 --skip ui::tests:: --skip ui::settings::` | 239通过、4项physical默认ignored，167.40秒 |
| `CARGO_INCREMENTAL=0 cargo test --lib --locked --offline -- --test-threads=1 --skip ui::` | 65通过 |
| `CARGO_INCREMENTAL=0 cargo test --tests --locked --offline -- --test-threads=1 --skip ui::` | 所有集成目标153通过；非UI库亦复跑通过 |
| `CARGO_INCREMENTAL=0 cargo test --doc --locked --offline` | 0项，无失败 |
| `CARGO_INCREMENTAL=0 cargo clippy --all-targets --locked --offline -- -D warnings` | 通过，最终补强后复查 |
| 编辑器diagnostics／`git diff --check` | 无错误警告／无空白错误；Git另提示部分文本CRLF将规范为LF，不视为代码诊断 |

最终按分组共**335库＋153集成＝488项通过**，库另4项默认ignored，不声称未跑的physical通过。第一次整库串行命令触及240秒上限，在设置测试中途终止；保持该次超时事实，随后分组补齐，未把原命令改记为成功。scroll专项36通过、2physical ignored亦单独执行。

### 实屏滚动

显式串行执行两个Report物理专项：Dark48个native返回前轨道样本通过；Light在初始屏幕所有权检查被同一外窗遮挡拒绝，单独复核仍同样遮挡，**两次均失败，Light物理未验收**。没有放宽WindowFromPoint、换离屏证据或继续盲重试。普通Light双轴拖拽／hover／repeat／Contrast／selection／32位range／禁绘／重入／GDI回归通过，不替代这项实际屏幕限制。

### Release与GUI

1. 第一份静态CRT Release构建193.612秒，7,995,904字节，SHA256 `9696B990FC838B9984890DEAD49BC2AC354A3DEA1D61A9FC614F0FBE01C83965`，x64 PE32+／系统DLL导入及版本退出0通过。
2. `tools/smoke-modern-ui.ps1 -ValidateOnly`通过PowerShell解析、合并C#编译及纯helper自检；新增cap反例初次误落在既有±1px容差，改正反例而未放宽guard。
3. 对第一份Release完整modern GUI通过，96 DPI、284.69秒：16四页／24设置分类／24统计子视图TOP／BOTTOM／cache，168次PrintWindow捕获、165个独立PNG、34组局部对比证据、15个Edit草稿／选区／属性保持、7数字框拒绝、空私有Key显隐／重掩码、回环10请求历史查询／pending、配置字节不变及正常退出。主agent抽看上游／网络／switch／统计浅色及网络深色图，不声称逐图逐字审阅。该完整矩阵**早于最终数字重入加固**。
4. 重入修复后最终Release再次静态CRT／编译路径映射构建，191.726秒：
   - 文件：`target/release/ai-relay.exe`，**7,997,440字节**。
   - SHA256：**`883C1326F2A31E4928DE0C7E9346350CE5AC0445B4971A270098729243A48E8F`**。
   - x64 PE32+／只导入Windows系统DLL，无外置CRT／SQLite／OpenSSL；`--version`为`AI Relay 2026.10.18`、退出0、stderr空。
5. 最终成品About／浅色设置定向检查通过：真实菜单打开关于、读取及截图确认`AI Relay 2026.10.18 · Release 构建`；空Key显隐及切分类自动掩码、六网络数字非法整次拒绝、浅色上游／网络／switch截图，配置哈希不变、代理始终停止、本人测试进程正常退出。未重跑整份modern矩阵。
6. 定向辅助脚本初轮有解析错误、创建期控件尚未出现的错误读取、原生主窗口caption为空的严格断言失败及关闭About超时，均未作成功证据。关于截图与文字已正确，关闭超时定位为系统按钮ID实际**2**而非脚本假定的IDOK=1；修为向已验证属主的对话框发送原生WM_CLOSE后通过。主窗口native caption读到空值，未声称exact caption初始化验收通过，未扩展修理该历史边界；代码分支中Release开发标记移除与About实际验证分开记录。

验证报告／截图保留在忽略的`target/modern-ui-screenshots/20261031-021542-533d4990748246f284c3071b6d0470cd/`与`target/final-ui-2026-10-31-73ef4a5cee6141b8b5d42a85e8f43e1e/`，最终构建报告`target/release-build-2026-10-31-ui-polish-final.json`。这些工具证据不纳入源码发布。初次Release构建被忙碌保护拒绝，空闲后执行成功，未停止其他进程。

## 剩余限制与下一步

- 实际150%／200%、多屏DPI、Win10／11干净系统、真实高对比度／读屏、物理键鼠／IME组合、连续刷新视觉及全部Release专项未完成。参数／离屏不能替代。
- Light物理Report采样因遮挡未验收；日历／菜单／日志历史限制按STATUS保持，不因本批通过扩大结论。
- 新数字生产剪贴板只读有界，测试使用一次性注入源，未改用户剪贴板或用真实延迟剪贴板Provider验收；不声称秘密从内存擦除。资源失败有静态／超限回退覆盖，未注入真实OOM／驱动故障或实测最坏绘制耗时。
- 最终静态CRT Release包含本批未提交源码；未提交／推送、建分支、打包或改dist／显式target历史成品，版本仍2026.10.18。旧Debug并不保证最终加固，不建议据同版本号推断内容相同。
- 后续使用最新Release在无遮挡桌面与真实高DPI完成视觉／交互验收，再决定提交及打包。
