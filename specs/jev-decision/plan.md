# Jev 决策辅助 · 方案

日期：2026-10-20。规格见 `spec.md`，任务见 `tasks.md`，验收见 `acceptance.md`。

### 2026-10-21 修复方案（优先于下文旧实现描述）

修复 RF-001～005：将服务创建时的配置代次作为不可变授权边界，发送前及响应后复核，Agent 再复核；生产端当前没有已评估场景，Assist 有效模式为 Observe，UI 不开放该选项；OpenRouter 仅接受精确单字母和完整、未补齐的概率；恢复成功后调用现有单行序列化；将并发和熔断移至进程共享状态，按来源与凭据代次隔离熔断，运行级状态仅保留授权、快照和调用预算。测试注入独立运行状态以避免并行用例相互污染，不改变生产共享行为。不增加依赖、日志存储或真实网络调用。

一致性检查：R001～R005 分别覆盖 RF-001～005 和相关 AC，R006 记录实际验证；真实质量门和发布集成仍保持未完成，不能以模拟回归代替真实效果评测。

## 1. 架构与模块边界

依赖方向保持 `desktop/cli → shell → agent → core`。Agent 不导入 Shell，不自己读设置文件或打开凭据文件。

| 模块 | 职责 | 依赖 |
| --- | --- | --- |
| `crates/agent/src/decision.rs` | 场景类型、请求/响应 DTO、固定问题模板、响应校验、纯采纳策略、`DecisionAccess` trait、HTTP 客户端 | `serde`、`serde_json`、`ureq`（已有） |
| `crates/agent/src/classify.rs` | `classify_with_evidence()`：基线类别 + 规则分 + ambiguous 判定；保留 `classify()` 兼容入口 | — |
| `crates/agent/src/verify.rs` | `failure_evidence_origin()`：把既有失败判断分为 `hard_evidence` / `heuristic` / `default`（不改动既有枚举的持久化格式） | — |
| `crates/shell/src/decision.rs` | 运行级 `DecisionAccess` 实现：配置代次、独立凭据、最小状态、预览、授权、原子消费、额度、并发、熔断 | `kodo-agent`、`kodo-core::settings` |
| `apps/desktop/src-tauri/src/decision.rs` | 设置 IPC、预览/确认 IPC、运行级授权桥接（`RunEvent::DecisionPreview`） | `kodo-shell`、`kodo-agent` |
| `apps/desktop/src/decisionApi.ts`、`DecisionSettings.tsx` | 独立配置表单、发送预览确认界面 | 前端 |
| `crates/cli/src/decision.rs` | CLI 交互式预览（stderr）与非交互拒绝 | `kodo-shell` |
| `evals/src/decision.rs` + `evals/fixtures/decision/` | 虚构语料、标签、对照报告 | `kodo-agent` |

**Agent 侧不读设置、不读凭据**：`RunRequest` 只多一个可选的 `Option<DecisionHandle>`，所有配置、凭据、授权、网络都在壳层。

## 2. 配置与凭据

### 2.1 设置

服务来源是配置的一部分：`provider` 字段（`typeSafe` / `openRouter` / `tencent`）决定目的地、凭据与请求格式。

**存储格式**：记录以**单行** JSON 写入 `settings.log`。该日志是行式 `key=value`，多行值会在第一行被截断；存盘前统一用 `DecisionSettings::to_json()` 归一化，既保证单行，也保证存进去的形状和界面看到的形状一致。

**恢复读取**：万一日志里已经有被截断的旧记录（更早的写入方式留下的），读取时不直接判死：值解析失败就把紧跟其后的行重新拼回去，逐行尝试解析，直到 JSON 复原；复原后按紧凑格式**追加**一份新记录，下一次读取就是普通读取。日志是只追加的，既有内容不被改写。恢复出来的设置原样生效，功能状态与损坏前一致 —— 损坏期间功能本来就是关闭的（配置不可用 ⇒ 关闭），所以恢复不会扩大任何权限。

沿用 `settings.log` 的 append-only `key=value` 语义，新增独立键 `jev-decision`，值为 JSON：

```json
{
  "schemaVersion": 1,
  "provider": "typeSafe",
  "mode": "off",
  "model": "jev-1.13.0",
  "scenarios": { "taskRoute": true, "failureTriage": false },
  "requestTimeoutMs": 1500,
  "maxCallsPerTurn": 3,
  "maxRequestBytes": 32768,
  "maxStateBytes": 16384,
  "maxResponseBytes": 65536,
  "maxDecisionTimeMsPerTurn": 4500,
  "policyVersion": "kodo-jev-v1"
}
```

- 键缺失 → 默认值，`mode=off`。
- 未知字段忽略；`mode` 非法、`schemaVersion` 不是 1、`policyVersion` 不匹配、或任一额度越界 → **Jev 被禁用并显示配置问题**，不降级为自动开启，也不影响程序启动（FR-014）。
- `mode=off` 优先级最高：保存密钥不自动启用，场景开关只在模式开启后表示"允许考虑"。

### 2.2 凭据

沿用 `credentials.log` 的命名空间机制：命名空间 `decision`，ID 按来源分（`jev` = 官方，`openrouter` = OpenRouter，`tencent` = 腾讯 EdgeOne），见 `kodo_core::settings::{read_secret, write_secret, clear_secrets}`。与 `provider.*` / `mcp.*` 完全隔离；清除一个来源的密钥不触碰其他来源，也不触碰其他命名空间（AC-016）。

- 明文只存在于 Rust 壳层内存。普通设置、前端返回值、运行记录、错误消息都不含明文（FR-008）。
- 前端只拿到 `hasCredential: bool` 与掩码；空值 / 掩码 / 替换 / 显式清除四态区分。

### 2.3 配置代次

`DecisionSettings` 每次保存、密钥更新/清除、停用、切换模型都递增 `config_generation`。授权令牌绑定代次；代次变化后旧令牌与迟到响应立即失效（FR-007）。

## 3. 数据模型与协议

### 3.0 服务来源

`DecisionProvider` 决定目的地、凭据、请求格式与响应解码：

| | `typeSafe` | `openRouter` | `tencent` |
| --- | --- | --- | --- |
| 目的地 | `https://api.typesafe.ai/v1/systemone` | `https://openrouter.ai/api/v1/chat/completions` | `https://ai-gateway.edgeone.link/v1/systemone` |
| 凭据 id | `decision.jev` | `decision.openrouter` | `decision.tencent` |
| 模型 | 固定 `jev-1.13.0` | 用户指定，默认 `typesafe/jev-1.14` | 固定别名 `@makers/jev` |
| 请求 | `{model, state, questions}` | `{model, messages, temperature, logprobs, top_logprobs}` | `{model, state, questions}` |
| 概率来源 | 响应自带的 `probabilities` | 首 token 的 `top_logprobs` 在选项字母上归一化 | 响应自带的 `probabilities` |
| 无概率时 | 校验失败 → 回退 | `Uncalibrated`：只记录，不采纳 | 校验失败 → 回退 |

关键取舍：**OpenRouter 走单字母作答**。选项名是多词标签（`code-review`），会被切成多个 token，逐选项概率就取不到了；单字母在几乎所有分词器里都是一个 token，首 token 的候选概率才干净可用。**腾讯 EdgeOne 不需要这一步**：它返回的就是 System One 原生 `answers`，因此复用 TypeSafe 的请求体与解码，只换目的地、凭据与模型别名校验。

**内部形状不变**：OpenRouter 的响应被翻译成 TypeSafe 的内部 `DecisionResponse`；腾讯的响应本来就是该形状。所以校验顺序、采纳门槛、轨迹记录、授权流程对三种来源完全一致 —— 来源是传输细节，不是第二套策略。

**模型一致性**：TypeSafe 要求请求与响应逐字一致；OpenRouter 允许路由解析出的 `日期后缀`（`-2024-07-18`）与 `provider 前缀`，`gpt-4o` 不接受 `gpt-4o-mini`；腾讯接受 `@makers/jev` 本身，或按别名解析出的 `jev-<版本>`（版本只允许数字与点），`jev-1.13.0-experimental` 与其他模型一律拒绝。腾讯的设置值也必须是 `@makers/jev`，其他值在保存时即被拒绝，不会发出请求。

**不自动换源**：一次只配置一个来源。换源等于把同一份内容发给用户没有批准过的第二个接收方。

### 3.1 请求

固定 POST，固定 origin 与路径，固定模型版本：

```text
POST https://api.typesafe.ai/v1/systemone
Authorization: Bearer <decision credential>
Content-Type: application/json
```

请求体（`serde_json` 序列化，`BTreeMap` 保证键序稳定 → 字节可复现）：

```json
{
  "model": "jev-1.13.0",
  "state": { "message": "…", "mode": "code" },
  "questions": {
    "task_type": {
      "type": "choice",
      "instructions": "…",
      "criteria": { "bug-fix": "…", "feature": "…", "test": "…", "refactor": "…", "code-review": "…", "docs": "…", "unknown": "…" }
    }
  }
}
```

失败分流的问题 ID 为 `failure_class`，选项为 `product` / `infrastructure` / `transient` / `unknown`。

### 3.2 首期允许进入预览的字段

| 场景 | 允许 | 不采集 |
| --- | --- | --- |
| 任务分类 | 当前用户消息、当前模式、已有基线类别 | 历史会话、源码、仓库规则全文、技能正文、附件、绝对路径 |
| 失败分流 | 命令类别、退出码、已计算失败标记、用户确认的短错误摘录 | 完整日志、完整命令环境、配置文件、凭据、生产数据、自动读取的项目文件 |

"命令类别"由本地解析生成（如 `rust_check`、`frontend_typecheck`），不发送原始 shell 命令。

### 3.3 敏感内容拦截

发请求前做本地检查：疑似密钥 / 私钥 / 令牌 / 受保护文件来源 → 直接跳过该次请求并走原路径（`sensitive_input`）。使用 `kodo_core::redact` 的既有脱敏工具辅助检查，不声称能检测全部敏感信息。不静默截断否定词或用户约束。

### 3.4 响应校验（顺序固定）

1. HTTP 成功、响应大小受控、JSON 解析成功。
2. `model` 与固定版本一致；别名漂移或不一致 → `invalid_response`。
3. 所有预期 question ID 存在且类型一致；出现意外 question ID → 拒绝整个响应；顶层无关扩展字段忽略。
4. Choice 的选项与概率键必须与请求一致；概率有限且在 0～1；概率和容差 ≤ 0.01；返回的 choice 必须是最高概率选项之一。
5. `confidence` 有限且在 0～1；不以其替代概率校验。`noul` 若存在也必须有限且在 0～1。
6. `usage` 若存在必须为非负可表示整数；缺失显示"用量未知"，不记作零成本。
7. 复核授权、配置、运行代次仍有效，再执行本地采纳策略。

### 3.5 采纳策略（纯函数）

| 场景 | 最高概率门槛 | 第一/第二概率差门槛 | 允许的采纳结果 |
| --- | --- | --- | --- |
| 任务分类 | 0.90 | 0.20 | 六种现有 `TaskType` 之一（`unknown` 不采纳） |
| 失败分流 | 0.95 | 0.30 | `product` / `infrastructure` / `transient`（`unknown` 不采纳） |

规则分是启发式分数，**不是概率**；`classify_with_evidence()` 返回的分数只用于"是否 ambiguous"的本地判断。

## 4. 授权（consent）流程

### 4.1 一次性令牌

令牌由壳层创建并保存在**内存**中，绑定：

```text
requestId + sessionId + runGeneration + configGeneration
+ scenario + origin + path + model + policyVersion
+ 冻结的请求字节 + 冻结的来源快照字节
```

- TTL 5 分钟。
- 消费是**原子一次性**：`HashMap::remove` 先取出再校验，任何不匹配都是硬拒绝（与 `knowledge.rs::consume_turn` 同形）。
- 设计中的 `sourceSnapshotDigest` / `canonicalPayloadDigest` 用**字节等值比较**实现：比对冻结字节与当前重算字节，比摘要更严格，且不需要在壳层引入 sha2/uuid 新依赖（见第 8 节偏差 1）。
- 预览之后的任何字段变化（消息、配置、模型、来源、运行代次）都会让令牌失效；再次确认返回 `consent_expired` / `config_changed` / `snapshot_changed`，模拟服务器收到 0 请求。

### 4.2 预览内容

预览必须展示**实际发送的完整内容**：purpose、mode（Observe/Assist）、origin、path、模型版本、完整请求 JSON、问题与答案空间、请求字节数、state 字节数、TTL。两个按钮："允许本次发送" / "跳过 Jev"。默认焦点为跳过。关闭窗口等同跳过。

首期不提供永久允许、会话批量允许或后台自动允许。

### 4.3 拒绝后的记忆

用户跳过时，本轮记住该场景已拒绝（同一场景不重复弹窗）；下一轮不继承任何批准。

### 4.4 取消

- 等授权时取消 → 本轮停止，不允许以"回退"为由继续运行工具。
- 发送中取消 → 迟到响应不采纳。
- 响应后、采纳前取消 → 结果不采纳。

## 5. HTTP 契约与错误矩阵

### 5.1 约束

- 仅 HTTPS 官方固定地址；`redirects(0)`（不跟随 3xx）；不自动切换主机 / 模型 / 服务商；不自动重试（`ureq 2.12` 默认不重试）。
- 关闭环境代理读取（`try_proxy_from_env(false)`）：首期不能悄悄新增代理路径。生产端点为 HTTPS，客户端 `https_only(true)`；仅本地模拟服务用 HTTP loopback。
- 总请求体 ≤ 32 KiB，state ≤ 16 KiB，响应读取 ≤ 64 KiB。先本地检查后授权。
- 每轮最多 3 次请求；分类最多 1 次；失败分流最多 2 个不同快照；单轮决策累计传输等待预算 4500 ms（不含用户阅读预览时间）。额度在请求前原子预占。
- 进程内最多 2 个 Jev 在途请求；拿不到名额立即跳过，不阻塞任务队列。

### 5.2 错误矩阵

| 条件 | 行为 | 记录码 |
| --- | --- | --- |
| Off / 场景禁用 / 不适用入口 | 不构造远程请求 | `disabled` / `ineligible` |
| 无密钥、敏感内容、超限 | 不发送，回原路径 | `missing_credential` / `sensitive_input` / `payload_limit` |
| 拒绝、过期、来源变化 | 不发送，回原路径 | `consent_denied` / `consent_expired` / `snapshot_changed` |
| 用户停止 | 结束本轮，禁止任何后续工具 | `cancelled` |
| 超时、网络错误、429、529、5xx | 无自动重试，原路径继续 | `timeout` / `network` / `rate_limited` / `unavailable` |
| 401、403 | 当前凭据代次停止请求，提示检查设置 | `authentication` |
| 400、413、415、422 | 原路径继续，显示配置/契约问题，禁止原样重试 | `request_invalid` |
| 3xx、模型/答案/概率异常 | 拒绝响应，原路径继续 | `redirect` / `invalid_response` |
| 额度不足、并发已满 | 不发送，原路径继续 | `budget_exhausted` / `busy` |
| 用户已停用、切换配置 | 丢弃在途结果 | `config_changed` |

### 5.3 熔断

进程内状态：同一配置连续 3 次传输/服务失败后暂停 60 秒，之后允许一个探测请求；认证失败直至凭据更新。用户拒绝、不适用、低置信度不计故障。等待熔断恢复不阻塞原任务。

## 6. 场景接入点

### 6.1 任务分类

`crates/agent/src/lib.rs` 中 `task_type` 的计算顺序（现行代码，磁盘实测）：

```text
is_direct_conversation 早退（3025）
  → split_skill_chip（3050）
  → task_type = Work ? Work : chip ? chip.applicable_task_types[0] : classify(classify_text)（3056-3066）
  → project_summary（3073）
  → file_contract + 无项目早退（3074-3080）
  → conversational（3081-3086）
  → skill = chip_skill.or_else(|| skill_registry.select(task_type))（3087-3091）
```

Jev 接入点在 **`conversational` 判定之后、`skill` 选择之前**（即 3086 与 3087 之间）：

- 前提全部满足才请求：`request.decision` 存在、`mode == Code`、`chip_skill` 为空、`conversational == false`（已排除无项目、总结意图、普通对话、咨询问题）、`file_contract.requires_file_output` 为假、`has_project` 为真。
- Observe：只产生 `Step::Reasoning` 记录，**不改变** `task_type`。
- Assist：仅当 `classify_with_evidence()` 判定 ambiguous、响应合法且达到门槛时，把 `task_type` 换成 Jev 建议的类别。
- **预算家族固定用基线 `TaskType` 计算**（`budget_for_task(baseline_task_type, …)`），满足"不改预算家族"；文件产出契约、模式、权限全部不受影响。
- 采纳后重新执行硬约束复核：显式技能、模式、文件契约、咨询/执行通道判定优先于 Jev 建议。

`classify_with_evidence()` 返回：基线类别、各类别规则分、最高/次高规则分、命中类别数、是否默认兜底、是否 ambiguous。ambiguous 判据：无命中 / 最高分并列 / 最高与次高分差 < 4。`classify()` 保留为兼容包装（`classify_with_evidence().baseline`）。

### 6.2 失败分流

`crates/agent/src/verify.rs` 的 `classify_verify_failure()` 命中明确的 INFRA 标记时返回 `Infrastructure`，否则返回 `Product`（默认）。新增 `failure_evidence_origin()` 把既有判断分为：

| origin | 判据 | 可否进入 Jev |
| --- | --- | --- |
| `hard_evidence` | `Denied`（用户拒绝）/ `Cancelled`（用户取消）/ `Infrastructure`（命令不存在、权限故障、网络、限流等明确标记）/ `Timeout` | **否** |
| `heuristic` | 命中 INFRA 标记但被 `Product` 覆盖的混合情况 | 否 |
| `default` | 无明确标记，默认 `Product` | **是**（唯一可问的情况） |

接入点在 `lib.rs` 的 `RepairDecision::from_evidence()` 之后、`infra` 分支之前：

- 前提：非零退出、无 `Denied`/`Cancelled`/`Timeout`/`Infrastructure` 硬证据、`!budget_exhausted`、同一失败快照最多问一次。
- Observe：只记录建议，路径不变。
- Assist：`infrastructure` → 走既有 `infra` 阻塞分支（不标完成、不删证据、不自动装依赖）；`transient` → 只报告，首期不自动重试；`product` → 走既有修复路径；`unknown` → 原路径。

## 7. 轨迹与持久化

复用 `Step::Reasoning { summary, phase: "analyze", diagnostics }`。`diagnostics` 放带 schema 标识的有限 JSON：

```json
{"schema":"decision.v1","requestId":"…","scenario":"taskRoute","mode":"observe",
 "model":"jev-1.13.0","policyVersion":"kodo-jev-v1","status":"observed",
 "baseline":"feature","suggestion":"refactor","applied":false,
 "probabilities":{"feature":0.1,"refactor":0.86},"confidence":0.83,
 "durationMs":412,"consentWaitMs":3200,"inputTokens":null,
 "skipCode":null,"fallbackCode":null}
```

- 不新增 Core `ItemKind`、SinkEvent、日志文件、数据库或远程监控。
- 不写入用户消息、源码、错误摘录、请求正文、认证头、错误响应正文、授权令牌。
- 已有 `local_trace` 不绕过：不捕获 Jev 的 I/O 原文。
- `ItemKind::Reasoning` 已被 `kodo_shell::memory_outbox::event_for_item` 明确排除，不进入 Memory outbox；`kodo_shell::turn::conversation_history` 只回放 `AgentMessage`，决策记录不进入下一轮普通对话历史（AC-020）。
- 前端无法识别 diagnostics 的 schema 时显示原 Reasoning 摘要即可。

## 8. 入口接入矩阵

| 入口 | 位置 | 决策句柄 | 说明 |
| --- | --- | --- | --- |
| 桌面人工发送 | `apps/desktop/src-tauri/src/main.rs::send_message` | `Some` | 交互式预览通道 |
| 桌面定时/后台 | `apps/desktop/src-tauri/src/scheduler.rs::spawn_agent_run` | `None` | 首期一律不注入（FR-012 / AC-015） |
| CLI 交互 | `crates/cli/src/turn.rs::run_turn` | `Some`（TTY 时） | 显示完整预览并要求明确选择，默认拒绝 |
| CLI 非交互 | 同上 | `None` | 无 TTY / 管道输入 / 无人值守一律跳过 |
| `evals` | `evals/src/main.rs`、`evals/src/work.rs` | `None` | 评测默认不触发网络 |
| 单元测试 | `crates/agent/src/lib.rs` 内的 `RunRequest {…}` | `None` | 默认无句柄 ⇒ 行为与基线逐字节一致 |

`RunRequest` 新增 `pub decision: Option<DecisionHandle>`，缺省 `None`；**关闭功能、无授权或不适用入口时不产生 Jev 请求**。

## 9. 与设计的偏差

| # | 设计 | 实际 | 理由 / 影响 |
| --- | --- | --- | --- |
| 1 | 授权绑定 `sourceSnapshotDigest` / `canonicalPayloadDigest` | 壳层内存冻结**字节**并做等值比较 | `kodo-shell` 未依赖 sha2/uuid；字节等值比摘要更严格，且不新增依赖。不影响对外契约。 |
| 2 | `DecisionAccess` 放在 `crates/agent`，HTTP 客户端也在 agent | DTO/模板/校验/纯策略/HTTP 客户端在 `crates/agent/src/decision.rs`；运行级状态、授权、额度、熔断在 `crates/shell/src/decision.rs` | 与设计一致；壳层只做字节等值，不引入摘要依赖。 |
| 3 | 预算可能随采纳类别变化 | 预算固定由**基线** `TaskType` 计算 | 满足"不改预算家族"的硬边界；采纳只影响技能选择与任务通道内类别。 |
| 4 | 决策轨迹用 `Step::Reasoning` + diagnostics | 同设计 | 未扩大会话协议。 |
| 5 | 配置 JSON 存 `settings.log` | 同设计，`key=jev-decision` | 复用 append-only 语义；不新增 SQLite 迁移。 |
| 6 | 决策轨迹三态：Started(pending_consent) → Started(requesting) → Finished(observed/applied/skipped/fallback) | 单条目：Started(pending_consent) → Finished(最终态) | 三态会产生两个"已 started 但未完成"的条目，在会话日志里留下悬空项。改为单条目，`consentWaitMs` 与 `durationMs` 分别记录人工等待与服务耗时。 |
| 7 | 任务分类 state 可含"已有基线类别"（§6.1 表格） | state 只含 `message` 与 `mode` | §6.1 是**允许集合**（上限），§7.2 的规范请求体只含 message 与 mode。最小化原则优先，基线只留本地对照。离线契约检查断言 state 恰好是这两个键。 |
| 8 | 请求 ID 由 uuid 生成 | `jev-<pid>-<nanos>-<seq>` | `kodo-shell` 没有 uuid 依赖；令牌是一次性内存态，且消费时还要校验会话与运行代次，可猜测的 ID 不构成越权路径。 |
| 9 | 失败分流 Assist 的 `transient` 建议 | 只记录建议，沿用既有路径（不进入基础设施阻塞分支） | 设计只对 `infrastructure` 规定"转为阻塞报告"，对 `transient` 只要求"不自动重试"。把 `transient` 也变成阻塞会放大"产品问题误阻塞率"，与设计的 1% 上限相悖。 |
| 10 | 语料放在 `evals/fixtures/` 下的独立目录 | 放在 `evals/fixtures-decision/` | `evals/fixtures/` 的每个目录都被 fixture 层当作"带 oracle 的任务仓库"扫描；把语料放进去会破坏 `splits_cover_all_fixtures` 门。 |
| 11 | 首期固定一个官方接收方 | 支持三个来源（官方 Jev / OpenRouter / 腾讯 EdgeOne），一次只启用一个 | 用户先后要求接入 OpenRouter 与腾讯 EdgeOne。仍然固定目的地、固定凭据、失败不换源；新增的是"用户选哪一个"，不是"系统替用户选"。 |
| 12 | 概率由服务直接给出 | OpenRouter 的概率由首 token 候选归一化得到；没有概率时只记录不采纳；TypeSafe / 腾讯直接使用响应的 `probabilities` | 单字母作答让 OpenRouter 的逐选项概率可取；没有分布就意味着"不知道"，而不是"很确定"。门槛假设的是真实概率，不能拿模型自评冒充。 |
| 13 | 设置记录按可读 JSON 存盘 | 存盘前统一归一化成**单行** JSON | `settings.log` 是行式 `key=value` 日志：多行记录会在第一行被截断，读回来是 `{`，于是报 `EOF while parsing an object`。归一化同时兜住了用户直接提交多行 JSON 的情况。 |
| 14 | 官方端点固定 `jev-1.13.0` | OpenRouter 端点默认 `typesafe/jev-1.14`；腾讯固定别名 `@makers/jev`，响应解析为 `jev-<版本>` | 用户指定。三者是不同传输上的不同版本；官方与腾讯路径的版本仍受冻结评测约束，OpenRouter 路径的版本在质量门里单独统计。 |
| 15 | 模型名由请求方自由指定 | 腾讯只接受 `@makers/jev`，保存其他值即配置错误 | EdgeOne Makers 对非该别名的模型返回 400。在保存时拒绝比发出一个必然失败的请求更清楚；前端同时锁定输入框。 |

**无新增依赖**：`kodo-agent` 已有 `ureq 2.12` / `serde` / `serde_json`；`kodo-shell` 已有 `serde` / `serde_json`；凭据与设置复用 `kodo-core::settings`；脱敏复用 `kodo-core::redact`；轨迹复用 `kodo-shell::turn` 的 Reasoning 映射。

## 10. 验证方式

### 10.1 Rust

```bash
cargo fmt --all -- --check
cargo check --offline --locked --workspace --all-targets
cargo test --offline --locked -p kodo-agent --lib
cargo test --offline --locked -p kodo-shell --lib
cargo test --offline --locked -p kodo-desktop --bin kodo-desktop
cargo test --offline --locked -p kodo-core --lib
```

新增的本地模拟服务只绑定 loopback（`127.0.0.1`），使用虚构凭据与临时目录，不连接实际用户状态库。

### 10.2 前端

```bash
cd apps/desktop && npm run typecheck && npm run build
```

视觉/交互用例遵守 `docs/design/UI_ACCEPTANCE.md` 的参考视口与验收口径。

### 10.3 行为回归

- 默认（无 `jev-decision` 设置键、mode=off）时，本地模拟服务器收到 0 请求，且既有路由与终态与基线一致（AC-001）。
- 关键回归：显式技能 / Work 模式 / 普通对话 / 无项目 / 知识库轮次不请求（AC-004）。
