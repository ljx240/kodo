# Secure Execution V1.1：沙箱能力与平台兼容

> 状态：P0/P1 已实现（macOS 实测通过；Linux 后端已实现，CI 中用真实 Bubblewrap 验证）
> 对应设计：`docs/design/Kodo_Secure_Execution_V1_Design.md`
> 实现位置：`crates/sandbox`（`kodo-sandbox`）

## 1. 为什么需要沙箱

审批只回答"这条命令能不能开始"，不回答"它能碰到什么"。
`npm test`、`cargo build`、`python setup.py` 都会执行项目里的任意代码，
它们派生的子进程可以读写项目外的文件、读取用户凭据、向外发送数据。

沙箱把第二个问题交给操作系统回答，并且这个约束对**所有后代进程**生效。

## 2. 两条互相独立的轴

| 轴 | 取值 | 决定什么 |
|---|---|---|
| 权限 `Permission` | `ask` / `auto` / `full` | 是否需要人批准这次调用 |
| 沙箱 `SandboxMode` | `read-only` / `workspace-write` / `unrestricted` | 进程在操作系统层面能访问什么 |

两条轴互不影响：

- `full` 权限**不会**放宽沙箱边界。`full` 的含义是"少打扰"，不是"不受限"。
- 沙箱放开到 `unrestricted` 需要显式用户操作，并在审计中记为 `sandbox_bypassed=true`。

默认组合：`ask` + `workspace-write`。

## 3. 越权：唯一的后门，而且必须有人点头

沙箱拒绝一条命令时，Kodo 可以请求**一次**越权：这条命令不带沙箱限制执行。
V1.1 把这条路径收紧为产品里最严格的一条规则：

| 规则 | 落地方式 |
|---|---|
| 三种权限模式都不能自动批准 | 独立的 `StepKind::SandboxEscalation`，`Permission::needs_approval` 对它恒为 `true` |
| GUI 必须展示风险与作用范围 | 审批卡片标注「沙箱越权」，写明风险与"仅本次生效，不改变会话沙箱设置" |
| CLI 必须执行同样的要求 | 独立提示块，只提供「仅本次允许 / 拒绝」；`s`/`a` 降级为单次 |
| 无头执行一律拒绝 | `StartArgs.headless = true`（调度器、后台触发）直接拒绝，不弹卡片 |
| 不复用 Session-Allow 缓存 | 越权指纹空间独立，且从不写入会话允许集合 |
| 已获会话批准的普通命令仍需单独审批 | 越权请求不查会话允许集合 |
| 拒绝或取消后保持原有边界 | 越权被拒时命令从未在沙箱外启动 |
| 只对精确匹配的本次操作生效 | 授权绑定 `operation_digest`，用后即焚 |
| 不隐式改变全局或会话配置 | 授权是一次性 grant，不修改策略 |

触发越权的条件是**结构化**的：`RunFailure::SandboxRefused(SandboxError)`。
退出码不参与判断——一个恰好返回 `126` 的普通程序只是普通失败。

## 4. 强制能力矩阵

能力由**功能探测**得出，不是由"二进制存在"推定。
`kodo --sandbox-status` 与桌面端 Settings → 沙箱状态显示同一份报告。

| 能力 | macOS（Seatbelt） | Linux（Bubblewrap） | 说明 |
|---|---|---|---|
| 文件写 | **完整** | **完整** | 严格白名单：只有工作区与私有临时目录可写 |
| 网络出站 | **完整** | **完整** | macOS `deny network*`；Linux `--unshare-all` 独立网络命名空间 |
| 文件读 | **部分** | 完整（按构造） | macOS 用拒绝清单（凭据目录、Keychain、Shell 历史）；Linux 靠挂载命名空间，用户主目录默认不挂载 |
| 子进程继承 | 是 | 是 | 限制由内核施加，脚本派生的子进程无法绕过 |
| 符号链接逃逸 | 被阻止 | 被阻止 | Seatbelt 在内核层解析路径；链接指向工作区外时直接失败 |
| 硬链接 / TOCTOU | 不保证 | 不保证 | 见 §8 剩余风险 |
| 域名白名单 | 不支持 | 不支持 | 请求该策略会被**拒绝**，不会用 `HTTP_PROXY` 冒充 |

### 4.1 探测怎么做的

能力不是"配置看起来对"，而是**被观察到**：

- **网络**：起一个真实 TCP 监听器，先在**沙箱外**连接成功（对照组），
  再在沙箱内连接同一地址。沙箱内被拒绝才算隔离生效。
  连接被拒绝 / 超时 / 网络不可达分别归类；对照组失败说明探测本身坏了，
  报告"无法验证"而不是"隔离成功"。
  覆盖 IPv4 回环与 IPv6 回环。
- **文件读**：先在沙箱外读到目标文件（对照组必须成功），
  再在沙箱内读同一文件，必须被操作系统拒绝或根本不存在。
- **文件写**：沙箱内写入工作区必须成功，写入工作区外必须失败且文件不存在。

探测由一个只依赖标准库的辅助程序 `kodo-sandbox-probe` 执行
（`crates/sandbox/src/bin/kodo-sandbox-probe.rs`）。
找不到辅助程序时能力报告为"不支持"，受限命令随之被拒绝——不猜、不放行。

### 4.2 按策略解析的最低强制能力

策略不满足最低强制能力时，受限命令被**拒绝**，不会降级运行。
最低要求由策略本身推导（`SandboxPolicy::required_enforcement`）：

| 轴 | 最低要求 | 原因 |
|---|---|---|
| 文件写 | 完整 | 写白名单必须由内核强制 |
| 文件读 | 部分 | 拒绝清单是可接受的强制模型（能力报告如实标注为"部分"） |
| 网络出站 | 完整 | `deny-all` 承诺"没有任何出站"，只有真正生效的隔离才达标 |

### 4.3 平台验证状态

| 平台 | 验证方式 | 状态 |
|---|---|---|
| macOS 27.0.1 | 本机真实进程执行（`cargo test -p kodo-sandbox --test confinement`） | **已验证** |
| Linux | GitHub Actions `sandbox-linux` 工作流：安装 bubblewrap、放开 unprivileged user namespaces，并要求后端报告 `network: full`，否则任务失败 | **CI 验证** |

> Linux 后端无法在 macOS 上实测，本仓库不虚报：CI 工作流是唯一的验证入口，
> 它在后端不可用时**直接失败**，而不是让测试静默跳过。

## 5. 使用方式

### 桌面端

**Settings → 工具** 新增「沙箱模式」与「当前强制能力」两行。
能力等级来自实测报告，后端不可用时明确显示"不支持"并说明受限命令会被拒绝。

审批卡片新增一行「沙箱：工作区写入 · 文件读=部分 · 文件写=完整 · 网络=完整」，
说明这条被批准的步骤在什么边界内执行。
越权审批卡片单独标注风险与作用范围，并且**不提供**「本会话允许」。

### CLI

```bash
kodo --sandbox-status              # 打印后端与实测强制能力
kodo --sandbox read-only            # 本次进程改为只读
kodo --sandbox workspace-write      # 默认值
kodo --sandbox unrestricted         # 关闭沙箱（会打印醒目告警）
```

CLI 与桌面端读取同一份 `settings.log`（键名 `sandbox-mode`），
解析逻辑共用 `kodo_shell::sandbox_mode_from_settings`。

## 6. 三种模式的行为

| 模式 | 执行语义 |
|---|---|
| `read-only` | 工作区只读；只有私有临时目录可写 |
| `workspace-write` | 工作区可写；项目外写入、`.git`、`.kodo` 被系统拒绝 |
| `unrestricted` | **显式**关闭沙箱：命令以当前用户权限执行，审计记为 `sandbox_bypassed=true`，CLI 与 GUI 都给出醒目告警 |

`unrestricted` 由策略表达，而不是"没有沙箱上下文"：
缺少上下文表示**调用方没有建立沙箱**，此时命令被拒绝，绝不隐式信任。
模型也无法通过普通工具参数切换模式——模式由宿主在启动时确定。

## 7. 真实执行验证

```bash
# 沙箱 crate：真实进程，真实操作系统限制
cargo test -p kodo-sandbox --test confinement

# 失败关闭：后端不可用、网络隔离未验证时必须拒绝执行
cargo test -p kodo-sandbox --test failure

# 端到端：经过 Agent 自己的 ProcessRunner
cargo test -p kodo-agent --test sandbox_execution
```

覆盖的用例：

| 用例 | 期望 |
|---|---|
| 工作区内正常写入 | 成功 |
| 项目外写入 | 被操作系统拒绝，文件不存在 |
| 只读模式修改源码 | 被拒绝，原文件不变 |
| 脚本子进程跨目录写入 | 子进程同样被拒绝 |
| 符号链接逃逸 | 被拒绝，目标文件不存在 |
| `.git` / `.kodo` / 链接工作树的 common dir 写入 | 被拒绝 |
| 出站网络 / 回环 / IPv6 回环 | 被拒绝（真实 TCP 连接探测） |
| 正常构建（真实 `cargo build`） | 在沙箱内成功 |
| 私有 HOME 可写、工具链可见 | 真实工具链在沙箱内可用 |
| 后端故障 | 拒绝执行，命令从未启动 |
| 网络隔离未验证 | 拒绝执行 |
| 一次性授权 | 不可跨命令、不可跨会话复用 |
| 超时 / 取消 | 沙箱后代进程树仍被完整终止 |

平台确实没有后端时，测试打印 `skipped-unavailable` 并返回；
**若平台自带后端却无法生效，测试直接失败**——不允许用跳过掩盖失效的沙箱。

## 8. 剩余风险

- **硬链接与 TOCTOU**：路径校验与应用层文件工具有 TOCTOU 窗口；
  操作系统层的写入白名单仍然生效，但"同一 inode 的另一个名字"不在承诺范围内。
- **读取轴在 macOS 是拒绝清单**：主目录之外的系统路径（`/etc`、`/tmp`）仍可读。
  能力报告显示为"部分"，不做更强承诺。
- **Linux 读取可见的系统目录**：`/etc`、`/var`、`/opt` 只读可见；
  `/etc/ssh`、`/etc/ssl/private`、`/run`、`/var/run` 被遮蔽；
  工具链缓存只读，工具链内的凭据文件（`~/.cargo/credentials.toml`、
  `~/.cargo/config.toml`、`~/.m2/settings.xml`、`~/.npmrc`）被遮蔽为空文件。
- **MCP stdio 服务不受沙箱限制**：它们是 Kodo 自己启动的独立进程。
  审批卡片会明确标注「外部 MCP 服务不受沙箱限制」，不会被静默继承工作区权限。
  执行模型集中在 `kodo_agent::mcp::McpExecution` 一处，后续可在此接入沙箱化。
- **控制平面与执行平面分离**：Provider / WebSearch / 远程 MCP 的 HTTP 由 Kodo
  进程发出，属于受信控制平面；不可信 Shell 子进程的网络是另一条路径。
  沙箱内运行的子进程不继承 `HTTP_PROXY` 等代理变量——代理变量不是安全边界。
- **`sandbox-exec` 已被 Apple 标记为废弃**：当前 macOS 版本仍可用，
  后端会做功能探测，失效时报告不可用并拒绝受限执行。
- **Windows 无原生沙箱**：本阶段不承诺。缺少后端时受限命令会被拒绝。

## 9. 恢复与回退

- 临时放开：桌面端 Settings 改沙箱模式，或 CLI `--sandbox unrestricted`。
- 完全回退本特性：`git revert` 沙箱相关提交即可；
  `crates/sandbox` 是独立 crate，移除后 Agent 侧需要补回 `RunRequest.sandbox` 字段。
- 沙箱导致构建失败时，先看 `kodo --sandbox-status`：
  若后端报告"部分"，通常是工具链目录不在可读范围，可通过设置里的额外只读根解决。
