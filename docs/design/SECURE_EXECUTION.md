# Secure Execution V1：沙箱能力与平台兼容

> 状态：P0 已实现（macOS 实测通过；Linux 后端已实现但未在本机验证）
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

## 3. 强制能力矩阵

能力由**功能探测**得出，不是由"二进制存在"推定。
`kodo --sandbox-status` 与桌面端 Settings → 沙箱状态显示同一份报告。

| 能力 | macOS（Seatbelt） | Linux（Bubblewrap） | 说明 |
|---|---|---|---|
| 文件写 | **完整** | **完整** | 严格白名单：只有工作区与私有临时目录可写 |
| 网络出站 | **完整** | **完整** | macOS `deny network*`；Linux `--unshare-net` 独立网络命名空间 |
| 文件读 | **部分** | 完整（按构造） | macOS 用拒绝清单（凭据目录、Keychain、Shell 历史）；Linux 靠挂载命名空间，用户主目录默认不挂载 |
| 子进程继承 | 是 | 是 | 限制由内核施加，脚本派生的子进程无法绕过 |
| 符号链接逃逸 | 被阻止 | 被阻止 | Seatbelt 在内核层解析路径；链接指向工作区外时直接失败 |
| 硬链接 / TOCTOU | 不保证 | 不保证 | 见 §6 剩余风险 |
| 域名白名单 | 不支持 | 不支持 | P0 后端无法强制；请求该策略会被**拒绝**，不会用 `HTTP_PROXY` 冒充 |

> **验证状态**：macOS 27.0.1 上全部用真实进程执行验证（见 §5）。
> Linux 后端已实现，但**本机没有安装 `bwrap`**，因此未做功能验证，
> 平台上会报告 `bwrap 不可用` 并拒绝受限执行，而不是静默放行。

## 4. 使用方式

### 桌面端

**Settings → 工具** 新增「沙箱模式」与「当前强制能力」两行。
能力等级来自实测报告，后端不可用时明确显示"不支持"并说明受限命令会被拒绝。

审批卡片新增一行「沙箱：工作区写入 · 文件写=完整 · 网络=完整」，
说明这条被批准的步骤在什么边界内执行。

### CLI

```bash
kodo --sandbox-status              # 打印后端与实测强制能力
kodo --sandbox read-only            # 本次进程改为只读
kodo --sandbox workspace-write      # 默认值
kodo --sandbox unrestricted         # 关闭沙箱（会打印醒目告警）
```

CLI 与桌面端读取同一份 `settings.log`（键名 `sandbox-mode`），
解析逻辑共用 `kodo_shell::sandbox_mode_from_settings`。

## 5. 真实执行验证

```bash
# 沙箱 crate：真实进程，真实操作系统限制
cargo test -p kodo-sandbox --test confinement

# 失败关闭：后端不可用时必须拒绝执行
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
| `.git` / `.kodo` 写入 | 被拒绝 |
| 出站网络 / DNS | 被拒绝 |
| 回环端口（本机服务） | 不可达 |
| 正常构建（`cargo check`） | 在沙箱内成功 |
| 后端故障 | 拒绝执行，命令从未启动 |
| 一次性授权 | 不可跨命令、不可跨会话复用 |
| 超时 / 取消 | 沙箱后代进程树仍被完整终止 |

平台确实没有后端时，测试打印 `skipped-unavailable` 并返回；
**若平台自带后端却无法生效，测试直接失败**——不允许用跳过掩盖失效的沙箱。

## 6. 剩余风险

- **硬链接与 TOCTOU**：路径校验与应用层文件工具有 TOCTOU 窗口；
  操作系统层的写入白名单仍然生效，但"同一 inode 的另一个名字"不在承诺范围内。
- **读取轴在 macOS 是拒绝清单**：主目录之外的系统路径（`/etc`、`/tmp`）仍可读。
  能力报告显示为"部分"，不做更强承诺。
- **MCP stdio 服务不受沙箱限制**：它们是 Kodo 自己启动的独立进程。
  审批卡片会明确标注「外部 MCP 服务不受沙箱限制」，不会被静默继承工作区权限。
- **控制平面与执行平面分离**：Provider / WebSearch / 远程 MCP 的 HTTP 由 Kodo
  进程发出，属于受信控制平面；不可信 Shell 子进程的网络是另一条路径。
- **`sandbox-exec` 已被 Apple 标记为废弃**：当前 macOS 版本仍可用，
  后端会做功能探测，失效时报告不可用并拒绝受限执行。
- **Windows 无原生沙箱**：本阶段不承诺。缺少后端时受限命令会被拒绝。

## 7. 恢复与回退

- 临时放开：桌面端 Settings 改沙箱模式，或 CLI `--sandbox unrestricted`。
- 完全回退本特性：`git revert` 沙箱相关提交即可；
  `crates/sandbox` 是独立 crate，移除后 Agent 侧需要补回 `RunRequest.sandbox` 字段。
- 沙箱导致构建失败时，先看 `kodo --sandbox-status`：
  若后端报告"部分"，通常是工具链目录不在可读范围，可通过设置里的额外只读根解决。
