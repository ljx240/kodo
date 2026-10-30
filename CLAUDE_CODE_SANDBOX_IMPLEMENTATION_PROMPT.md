# Claude Code 实施提示词：Kodo Secure Execution V1

你现在是 Kodo 仓库的首席 Rust 架构师、安全工程师和实施工程师。请在**现有项目中实际编码、测试并交付** Kodo Secure Execution V1，不要只给出设计分析或建立空接口。

## 0. 必须读取的资料

1. 项目根目录 `AGENTS.md`（最高优先级的项目操作约束），以及存在的 `CLAUDE.md`、相关设计/测试规范。
2. `docs/design/Kodo_Secure_Execution_V1_Design.md` —— 本次唯一需求设计基线。若该文件不存在，先报告并检查用户提供的文档位置；不要凭空猜测文档内容。
3. 当前 `Cargo.toml`、`crates/agent/src/process.rs`、`crates/agent/src/tools.rs`、`crates/agent/src/lib.rs`、`crates/shell/src/config.rs`、`crates/shell/src/approve.rs`、`crates/cli/src/args.rs`，以及 GUI 调用 Agent 的真实入口。

设计文档中的类型和文件路径是**建议/伪代码**，以仓库最新 HEAD 为准，不得机械照抄导致破坏现有运行时。

## 1. 总目标

在不大重构 Kodo Rust Core / Agent Loop / Shell / GUI 的前提下，增加可审计、可探测、可替换的 OS 级沙箱执行后端，让 Kodo 不可信 Shell 命令在默认模式下只在授权工作区内产生写效果、默认不可出站联网，并且在沙箱不可用时**fail closed**，绝不静默执行无沙箱命令。保持 CLI/GUI 使用同一策略逻辑。

保留现有 `Permission::{Ask,Auto,Full}`，**另建正交的 `SandboxMode::{ReadOnly,WorkspaceWrite,Unrestricted}`**；`Full` 审批档位不能自动解除 Sandbox 限制。对文件读取、文件写入、网络分别汇报 `Full/Partial/Unsupported` 级别。不得将仅有写入限制的实现标注为完整沙箱。

## 2. Git 与已有改动保护

- 开始时只读检查 `git status --porcelain=v1`、`git worktree list --porcelain`、`git branch --no-merged main`、`git stash list`、当前分支和 HEAD 基线，归属未确认的变更不得覆盖。
- 按根目录 `AGENTS.md` 从合适的本地 main 基线创建**唯一功能分支** `feat/secure-execution-v1`（已存在则检查后复用）；普通任务不要自动增加 worktree。
- 不使用 `git reset --hard`、`git clean -fd`、`checkout --`、`branch -D`、`stash drop/clear`、强推等破坏性操作。
- 未经用户明确授权，不 push、不建 PR、不合并 main、不清理他人的 worktree/stash。
- 修改已有文件前重新读取磁盘最新内容；发现冲突或他人改动及时停下受影响操作，保护现场。

## 3. 按以下顺序实施；能做的尽量一次落地

### Step A：执行入口审计

列出并追踪所有 Shell 子进程、Git 命令、验证命令、FileRead/Write/Patch/Delete、MCP stdio/HTTP、外部搜索及定时/非交互调用入口，指出真正受信控制平面与不可信项目执行平面。使用实际代码搜索，不要只看 README。记录可能绕过 `ProcessRunner` 或权限审批的路径。

### Step B：新建 `kodo-sandbox`

- 最小接口 `SandboxMode`、`SandboxPolicy`、`EnforcementCapabilities`、`SandboxBackend`、`PreparedCommand`、`SandboxError`；可以按现有架构调整命名。
- 检测可用后端和生效能力；建立真实功能探针，不仅判断外部命令存在。
- 严格参数化构造 `program + argv`，不得拼接用户命令到未转义的“外层 Shell 包装字符串”。
- 默认 `WorkspaceWrite` + 不可信 Shell 出站网络拒绝；未检测到可靠约束则阻止执行。

### Step C：接入进程生命周期

在现有 `ProcessSpec / ProcessRunner` 执行链路加入沙箱配置和后端包装，保留超时、取消、输出限制、进程树清理。覆盖 `command_run()`、Git 查询和 verify 路径，保证所有模型指令驱动的进程都通过统一边界。不要另写一套不可取消的 spawn 系统。

### Step D：macOS / Linux

- macOS：Seatbelt/sandbox-exec，实现文件写入限制、关键敏感文件读取保护、默认网络拒绝，包含子进程继承测试；探测 `sandbox-exec` 可用性及潜在系统兼容性。
- Linux：优先 Bubblewrap，设置合理 mount/user/network namespace、工作区可写、主机系统只读、敏感目录遮蔽、私有 TMP/HOME，使用真实探针验证；依赖缺失或 userns 不可用时 fail closed。
- 对系统命令、Node/npm、Rust/cargo、Git，以及项目已有测试做基本兼容性回归；不要因“安全”而允许整个 HOME 随意读取/写入。
- 若当前开发机只有一种平台，另一平台实现可以编译/静态审查，但必须在报告中标明**未在目标 OS 实测**，不能冒称验证通过。

### Step E：其他工具的能力边界与权限

统一 Agent 文件工具的策略与路径边界，修复已发现的 symlink/path race 等问题，必要时为安全写入引入更可靠的文件描述符操作。确保 `.git`（包含 worktree common dir）、`.kodo`、Kodo 凭据、用户私密目录受到保护。MCP stdio 等外部入口若当前无法可靠隔离，必须显式逐项授权/禁止，并准确报告为未覆盖，不能伪装安全。

区分模型 Provider 网络与沙箱 Shell 网络；后者默认拒绝。`AllowDomains` 如果没有强制代理与禁止直连能力，则只能作为未来 P1 预留，**不得开放假域名白名单**。

### Step F：配置、CLI/GUI 与审计

将沙箱设置贯通 `ShellConfig → RunRequest → ProcessSpec`；提供最小 CLI 帮助与清晰的 sandbox status 信息，GUI 如现有设置体系容易加入则只新增简洁选择与状态显示，不要大改布局。提供结构化执行审计、拒绝原因、单次越权授权机制。授权 token 必须绑定会话/操作/资源范围，并仅生效一次；没有人工输入的定时/无头任务不得自行提权。

### Step G：测试与文档

必须用真实执行验证：
1. 工作区写入成功，工作区外写入失败；`ReadOnly` 工作区修改失败。
2. 脚本/构建命令启动的子进程也无法跨目录写入。
3. 敏感目录读取保护按报告的能力成立；不成立时不能宣称完整。
4. HTTP/TCP/DNS/IPv6/localhost/Unix socket 绕路不破坏既定网络安全保证。
5. 软链接、硬链接、路径竞争、`.git`/common dir、`.kodo` 及私有运行时目录都不会让策略发生已知绕过。
6. 后端不可用、沙箱准备失败时**证明受限进程没有启动**。
7. 超时、取消、异常重试、CLI/GUI、MCP 和非交互任务不产生权限旁路。
8. 历史 `Ask/Auto/Full` 行为和 ChangeSet/Undo 回归。

执行与改动范围对应的 `cargo fmt --all -- --check`、`cargo check`、`cargo test`、必要的 `npm run typecheck/build` 与 Playwright 视觉测试。任何环境无法执行的测试必须逐一说明原因和覆盖缺口，不要写“全部通过”。更新 README、CLI 帮助、平台支持矩阵、安全模型和常见故障。

## 4. 约束条件（必须遵守）

- 极简 Rust 原生设计，CLI/TUI/GUI 共用 Core，不引入 Docker 作为 P0 强制依赖。
- 不新增另一套 Agent Loop、微内核或插件平台，不重写模型调用、Memory、Skills。
- 不默认把 `Unrestricted` 当后端失败时的替代模式。
- 不修改或读取用户真实密钥做安全测试；使用单独创建的假凭据和临时目录。
- 不用“只检查危险命令字符串”替代操作系统隔离。
- 不在 P0 实现未经验证的“安全域名白名单”；P1 再规划网络代理。
- Windows（特别是 Win7 32 位）不纳入本期完整沙箱交付；按设计文档标注后续事项。
- 不做无意义的 TODO 或空适配器占位，不让不可用后端静默跑普通 Shell。

## 5. 交付格式

先输出简短审计与变更计划，再执行开发。每完成一个阶段说明实际改动并继续。最终按下列结构交付：

1. **已落地能力**：按 P0-A/B/C/D 对照列出已实现与未实现项。
2. **代码改动**：关键文件、重要接口以及为何这样设计。
3. **安全矩阵**：macOS/Linux（以及未支持平台）的文件读/写/网络隔离能力与实际探针结果。
4. **测试证据**：真实命令、OS/版本、输出摘要、失败项/跳过项。
5. **兼容与回滚**：配置迁移、异常时 fail closed、恢复旧设置的安全方式。
6. **Git 报告**：基线 commit、当前分支、worktree/stash 状态、提交/未提交内容；是否涉及远程必须如实说明。
7. **后续工作**：仅列真正需要 P1/P2/P3 的任务，不要用未来计划冒充 P0 已交付。

现在开始读取设计文档和项目规范，审计真实仓库，然后按最小、可测试的增量完成实施。
