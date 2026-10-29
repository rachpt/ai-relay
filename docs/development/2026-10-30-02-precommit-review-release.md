# 2026-10-30-02：独立复核、工具回归加固与提交前验证

## 目标与范围

按用户授权，对本轮分段限宽、月历白边和概览刷新改动独立review；处理确定问题并验证后提交／推送，再执行Release构建。不擅自改版本、依赖、运行时或打包覆盖dist。

起始HEAD为 `314ea76`，当前分支 `master` 跟踪 `origin/master`。本轮另有两份已存在的Release／清理记录及其索引，已只读审阅并保留，不回退用户工作。推送目标只使用当前跟踪的 `origin`，不向额外远端自动推送。

## 独立review与实际修正

三个全新只读会话分别审查日期popup／生命周期、概览／设置生产与回归、工具／文档／安全范围；新增未跟踪模块均纳入，不仅查看git diff。

- 日期及概览／设置生产代码未发现确定新增P1／P2；真实系统高对比度、高DPI、多屏、读屏、资源耗尽及非零NC等仍为未实测边界。
- 工具审查确认1项P2：概览工具仍由raw最新速率及固定单位计算图例／单位区域，但新生产显示受120秒历史峰值影响。修为固定header／legend／status行和88 DIP plot区域，不再从WM_GETTEXT重建显示单位、第二图例坐标或声称精确字体fit；原始metadata数值检验独立保留。
- 另一全新独立会话复核发现2项工具P2：整带Red／Blue计数可被真实ClearType文字彩边掩盖，marker本身也可能充当正文墨迹。修为精确主题色及14 DIP短线＋点／空心方形几何，第二marker沿固定y有界扫描定位，排除两个marker矩形后检查中性正文；不靠宽松颜色数量证明marker。
- 同一独立复核会话对5份旧真实完整crop的10条legend再计算，原图均符合新条件，移除第一marker／第二marker／只保留markers共30个反例均拒绝；未修改旧PNG，不将其当作新GUI。
- 最终限定复核确认上述3项P2已解决，没有确定新增P1／P2；并复核后续仅将完整合成矩阵移至ValidateOnly的启动调整，不误记GUI执行过矩阵。
- 文档统计修正：前批modern为168次离屏捕获、实际保留165个独立PNG，3个既有输出名被复用；不是168个独立图片。历史报告保留，不静默改成另一轮测试。

仅 `tools/smoke-overview-dashboard-ui.ps1` 及有关文档新增修正，生产Rust代码未因本轮review改变。`docs/DEVELOPMENT.md` 同步工具真实覆盖范围。精确字体／单位／每个标签或连续物理刷新仍未宣称验收。

## 本轮验证

| 命令／项目 | 结果 | 范围与证据 |
|---|---|---|
| `cargo fmt --check` | 通过 | 当前生产／回归代码 |
| `cargo check --all-targets --locked --offline` | 通过 | Windows全目标 |
| `cargo test --locked --offline -- --test-threads=1` | 通过 | 305库＋153集成＝458项；3项physical默认ignored；doc通过。日志 `target/precommit-ui-refinements-tests-2026-10-30.log` |
| `cargo clippy --all-targets --locked --offline -- -D warnings` | 通过 | 严格检查 |
| modern／dark-date `-ValidateOnly` | 通过 | parser／C# ABI及纯辅助自检，不执行GUI |
| overview `-ValidateOnly` | 通过 | 96组合成正例、352组反例；包含近零、约值、B／KiB／K单位、历史峰值替代状态、未知，以及保留ClearType式彩边但缺marker、marker-only无正文、空header／legend／已知plot。日志 `target/precommit-overview-validate-2026-10-30.log`，完整矩阵76.74秒 |
| 修正后的Debug overview GUI | 通过限定scope | 96 DPI，21回环请求／7 HTTP400、真实字节／速率、隐藏冻结／返回、启停重启、23次OFFSCREEN捕获；6轮GDI最终88→88、配置不变及正常退出。55.37秒、Complete=false，完整矩阵本次未运行且报告明确标注 |
| 编辑器diagnostics | 无错误／警告 | 本轮刷新一次 |
| `git diff --check` | 通过 | 仅既有CRLF正规化提示 |
| 本地新增／修改文本敏感模式检查 | 未发现确定命中 | 私钥／常见凭据／私网地址／本机用户路径规则，不输出匹配内容；detect-secrets不可用，不作绝对秘密保证，不访问凭据验证API |
| Git远端基线 | 已fetch确认 | 提交前HEAD与origin/master无前后差异；用户已授权正常提交／推送，禁止force |

最终GUI证据：`target/overview-dashboard-screenshots/20261029-214855-0de076c4d0a841c5a50771286592c2ec/`；日志 `target/precommit-overview-ui-reviewed-2026-10-30.log`。最小窗口两处全图因dashboard高于viewport按既有规则跳过，仍保留几何及分段图，不当作完整像素通过。

### 超时与执行分离

两次120秒终端调用超时：首次串联ValidateOnly及GUI时未启动应用／GUI日志为空；第二次单独GUI已进入部分真实截图检查，但外部时限终止前未完成退出／报告，不记通过。未发现应用残留，未手工结束用户进程。

代码路径证实GUI入口原来无条件重跑完整合成矩阵，`ValidateBands` 有重复完整扫描成本；随后只将该完整矩阵移入ValidateOnly，保留GUI基础自检及全部真实crop／marker／正文守卫，并添加编译／基础／矩阵阶段输出及耗时。最终分别运行均在原120秒时限内完成，未扩大时限或减少矩阵／放宽断言。两次失败日志及部分证据保留，不用成功覆盖失败。

## 提交与Release计划

本记录写入时review和提交前验证已完成，commit／push及其后的Release尚待执行，不将计划记为成功。Release沿用既有单次进程静态CRT及编译路径映射，以 `cargo build --release --locked --offline` 更新 `target/release/ai-relay.exe`，不改Cargo profile、版本、dist或显式target历史产物；构建后核对x64 PE32+／动态依赖、SHA256与 `--version`。真实执行结果另补记录。

## 限制与下一步

- 本轮未重跑日期／modern完整GUI或三项物理测试；前批通过仍是历史结果。
- Debug截图与合成DPI矩阵不代替真实150%／200%、多屏、Win10／11差异、高对比度、IME／读屏及连续刷新／长稳。
- 保留此前完整日期启动超时、菜单／滚动等独立限制；本轮review未证实其根因已修。
- 只有源码／文档／工具进入提交；target／dist、真实配置／密钥／数据库、截图报告不入库，不创建发布标签或ZIP。
