# 公开准备、历史身份与旧实现清理

日期：2026-10-18

## 目标与授权

用户明确要求统一历史Git身份为 `rachpt <rachpt@126.com>`，删除旧JS/CMD等参考文件并检查敏感历史，以准备GitHub公开。此次许可仅覆盖已识别的旧参考实现和关联日志/PID，不扩展到原生tools脚本、配置或其他用户数据。

## 实际变更

- 初始工作区干净，仅master一个本地ref、4个可达提交、两组身份，无远端或标签。
- 仓库外创建关闭权限继承、仅当前用户/System可访问的备份目录，复制完整.git、8个拟删除文件并生成/验证history bundle。确切位置仅在本地验证记录中，不把私有备份或内容加入仓库。
- 使用git-filter-repo2.47.0统一author/committer的name/email，设置仓库本地user.name/email；不改全局配置。核对4个提交的树OID、顺序、日期、说明均保持，提交哈希改变。
- 对旧参考路径使用排除过滤；原可达历史未含这些路径，未发现需要替换的真实秘密。
- 删除根目录 `ai-proxy.js`、`proxy.cmd`、`allow-firewall.cmd`、`install-shortcut.ps1`、`start-proxy.cmd`、`ai-proxy.pid`、`proxy.log`、`proxy.err.log`。这些均为忽略文件，因此删除不表现为Git跟踪文件的D状态。保留tools、target、dist和其他用户数据。
- 清理reflog及不可达对象；发现并清除一个额外不可达旧身份提交，原.git完整备份中保留恢复能力。filter-repo旧提交映射同样只保留于私有备份。
- 更新AGENTS、需求13.3、README、开发指南/导航、STATUS和索引；新增ADR0011，只取代ADR0001的旧脚本保留决策，不重写其他历史记录。

## 扫描方法与结果

- 枚举全部ref和每个可达提交的tree，按blob去重，扫描264个独立文件版本，共4,355,332字节；另扫描commit/tag对象的说明与路径。
- 使用detect-secrets1.5.0及默认检测器，显式禁用凭据联网验证；当前跟踪树和全部历史分别执行。本地工具位于忽略目录，不是应用依赖，不上传源码或候选。
- 当前树检测6处、历史独立blob检测8处候选：Windows Common Controls公钥标识常量、无效URL用户密码和明确synthetic的安全回归测试。人工复核均非真实凭据，保留测试，不全局忽略tests目录。
- 额外规则检查私钥块、常见provider令牌、URL凭据、秘密赋值、个人home路径和需复核IP。版本号被IP规则命中、公共地址否定测试等均人工分类；没有发现应清理的真实部署值或个人目录。
- 从私有旧实现仅在内存提取5个精确候选（默认Key、令牌、旧上游及两类部署IP），与全部对象比对无命中。候选原文不输出、不写入新源码/报告。
- Gitleaks8.30.1官方release下载多次超时，未执行该工具，不声称其通过；改用成功安装的detect-secrets和独立规则交叉验证。git-filter-repo和检测器从PyPI安装到本地忽略目录，不要求最终用户安装Python。
- 重写后再次扫描、核对全部原始author/committer、旧路径不可达、旧对象不可读及 `git fsck --full --no-reflogs`。清理本身不修改Rust/工具业务代码，不以旧GUI结果冒充本批重跑。
- 新增文档随本次清理提交后再执行最终扫描和完整性检查；新增清理提交使用同一身份。

## 验证命令

```text
git bundle create <仓库外私有备份> --all
git bundle verify <仓库外私有备份>
git-filter-repo --force --name-callback <统一姓名> --email-callback <统一邮件> --invert-paths --path-regex <旧参考路径>
detect-secrets scan --no-verify
git reflog expire --expire=now --expire-unreachable=now --all
git gc --prune=now
git fsck --full --no-reflogs
git diff --check
cargo fmt --check
cargo check --all-targets --locked --offline
```

历史逐blob扫描、备份权限验证和树/元数据对比使用本地一次性脚本；公开开发指南说明其范围与禁联网策略，不把原文或旧身份写入仓库。`cargo fmt --check`、`cargo check --all-targets --locked --offline` 和 `git diff --check` 已通过。提交后继续最终重扫/完整性验证；无Rust业务改动，不重跑GUI、代理计费接口或修改Startup/系统代理/防火墙。

## 未完成与限制

- 自动规则与人工检查不能保证绝对没有秘密。旧部署Key建议轮换；即使本地未跟踪，附件、旧克隆、聊天、同步目录或其他副本不因本次清理撤回。
- 无LICENSE和Cargo许可元数据，尚未完成第三方notices审核或打包白名单更新；必须由维护者选定许可，不擅自选择MIT/Apache等。项目可见性与开源授权是不同事项。
- 没有GitHub远端、没有push；以后若外部已持有旧历史，应协调改用新克隆，而非直接合并旧分支。只发布已审查的Git提交，不上传整个本地目录、私有备份或旧dist/target产物。
- 尚无自动秘密检测CI；后续提交需复扫，适用时启用GitHub secret scanning/push protection。既有Windows实机/DPI/读屏/长稳验收限制不变。
