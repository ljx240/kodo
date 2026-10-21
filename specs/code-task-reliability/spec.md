# 编程任务可靠性修复 — 规格

基线：本地 `main` = `origin/main` = `a7133b2`。本轮分支 `fix/code-task-reliability`。
来源：对 `a7133b2` 的本地评估线索（评测非零退出、路由 60.8%、归因 38.3%、首轮无 usage 不压缩、早期约束丢失）在本轮重新核实。

## 1. 目标

让 Kodo 的 Code 通道在**离线可验证**的意义上更可靠：评测口径不再误伤、路由与归因输出真实类别、模型调用前有上下文容量保护、长任务跨轮不丢关键约束。

## 2. 范围与不做项

本轮做：`evals/`、`crates/agent/src/{classify.rs,verify.rs,state.rs,compaction.rs,context.rs,lib.rs,recovery.rs,plan.rs}` 及对应测试。`provider.rs` 仅允许改离线 fake 脚本分支（`fake_text`）以便测试驱动真实失败类别，网络请求体与工具协议不变（FR-D6）。

本轮不做（明确排除）：
- 不改 `crates/agent/src/decision.rs`、`crates/shell/src/decision.rs`、`specs/jev-decision/`、desktop decision 页面——这 12 个文件是本会话之前另一任务（`jev-decision`）的未提交现场，原样保留、不暂存、不提交。
- 不新增多代理运行时、不接新模型或外部决策服务、不引入 LSP/向量库/tokenizer 依赖、不重写 Agent 引擎、不改无关 UI。
- 不修改冻结语料 `evals/fixtures-decision/*.jsonl` 的任何一行。
- 不提交、不推送、不建 PR、不合并、不清理 stash/worktree/分支。

## 3. 场景与功能需求

### A. 评测套件目录误识别（P0）

**触发条件**：`evals/` 下存在名为 `fixtures-<x>/` 的目录，且 `<x>` 是分类语料（只含 `.jsonl`，无夹具子目录）。
**当前行为**：`evals/src/main.rs:1506 suite_names()` 按目录名前缀 `fixtures-` 无条件枚举为**编程夹具套件**，于是 `run_suite_baselines("decision")` 对它施加编程门：
- `fixture_count_at_least_4_decision`：该目录夹具子目录数 = 0 → FAIL（`only 0 fixtures`）；
- `splits_defined_decision`：`load_splits("decision")` 找 `evals/splits-decision.json`（不存在）→ dev=holdout=0 → FAIL；
- 默认离线评测 `G_fixture_baselines: FAIL (202/204)`，进程 exit 1。

**FR-A1** 评测必须按**磁盘形态**而非目录名判定套件类别：含夹具子目录 → 编程夹具套件；只含 `.jsonl` 案例文件 → 分类语料。
**FR-A2** 编程夹具门（`fixture_count_at_least_*`、`splits_*`、逐夹具 prompt/oracle/solution/scope 检查）只施加于编程夹具套件；`default` 阈值仍为 30、`real` 仍为 4，不降低。
**FR-A3** 分类语料不得被静默跳过：必须对它施加自己的一门检查（可发现、案例行可解析、`id`/`label`/`split` 齐备、tune 与 holdout 都非空），检查结果计入同一份报告。
**FR-A4** `decision` 分类评测继续由 `decision::run()` 独立执行，输出不因本次改动而减少。
**FR-A5** 形态判定不得吞掉错误：目录读不到、既无夹具子目录又无可解析案例文件时，必须以 FAIL 报出，不得当作"空套件"跳过。
**FR-A6** 必须有针对目录误识别的回归测试。
**AC-A1** `cargo run -p kodo-evals`（离线，无 `--live`）exit 0；`G_fixture_baselines` PASS；报告可见 `default`、`real` 各自检查数量与结果，以及 `decision` 语料的独立检查结果。
**AC-A2** 回归测试覆盖：`fixtures-x/` 只含 `.jsonl` → 判为分类语料；`fixtures-x/` 含夹具子目录 → 判为编程套件；混合目录 → 编程套件优先；空目录 → 报 FAIL 而非跳过。

### B. 任务路由（P1）

**触发条件 / 当前行为**（已核实，三套词表不一致）：
- 运行时 `TaskType`（`classify.rs:8-32`）= `bug-fix|feature|test|refactor|code-review|docs|work`，**无 `unknown`**；
- 决策组件选项（`decision.rs` `task_route_options`）有 `unknown`、无 `work`；
- 冻结语料标签含 `unknown` 20 条（tune 5 / holdout 15）、无 `work`；
- 评分器（`evals/src/decision.rs:158`）取 `evidence.baseline.label()`，**结构上不可能输出 `unknown`** → `task_route[unknown]: tp=0 fp=0 fn=20`，天花板 83.3%，实测 60.8%（73/120）。
- `is_document_output()`（`classify.rs:154`）是**短路**：命中"导出/保存 + .md/markdown" 直接判 `Docs`，跳过关键词打分。因此"新增导出为 Markdown 的入口"被判为 `docs`，而用户要的是**实现一个功能入口**。
- 零关键词命中时 `baseline` 仍返回 `Feature`（`classify.rs:170`），把无证据的猜测伪装成类别（`ambiguous` 已记录，但评分器与其他调用方只用 `baseline`）。

**FR-B1** 路由必须有一个单一事实来源的输出标签函数，覆盖语料词表（含 `unknown`），并由真实证据字段推导，不在评测器里另写一套手工映射。
**FR-B2** `unknown` 只允许来自"没有任何关键词命中且不是文档产出"这一无证据状态；不得为凑分伪造。
**FR-B3** 同时出现功能动词与文档格式词时，按用户要求的**实际行为**判定：存在代码产出意图时不得被文档短路判为 `docs`。
**FR-B4** 文档短路不得再吞掉打分：改为在代码产出类别无命中时才生效。
**FR-B5** `classify()` 兼容入口的行为契约保持稳定；含糊输入的不确定性通过证据字段与新标签函数表达，不把 `Feature` 默认值改掉来"修指标"。
**FR-B6** 用户显式选择（Work/Code 模式、技能 chip、权限、文件产出契约）优先级高于分类结果，必须有测试锁定。
**FR-B7** 不通过增补宽泛关键词清单提升命中率。
**AC-B1** "新增导出为 Markdown 的入口"路由为 `feature`。
**AC-B2** "把那个东西弄好"/"do the usual thing" 路由标签为 `unknown`，且 `ambiguous=true`。
**AC-B3** 原有 `classify.rs` 全部单测保持通过（含"保存到 docs/架构.md" → `Docs` + `DocumentOutput`）。
**AC-B4** 评测按**类别**和 **tune/holdout** 分别报告，且报告分母。

### C. 失败归因（P1）

**触发条件 / 当前行为**（已核实）：
- 运行时 `FailureClass`（`verify.rs:56-78`）= `product|infrastructure|timeout|cancelled|denied`；语料词表 = `product|infrastructure|transient|unknown`；
- `classify_verify_failure()`（`verify.rs:311-350`）只有 24 条 INFRA 子串匹配，**完全不看退出码**，其余一律 `Product`；`transient` 与 `unknown` 在运行时不存在；
- 评分器（`evals/src/decision.rs:210-215`）把 `Timeout` 手工映射成 `transient`、把 `Cancelled/Denied/Product` 全塌成 `product`，且把 `command_category`（如 `generic`）当作**命令行**传入，与运行时输入形态不符；
- 实测 38.3%（23/60）：`transient` tp=0、`unknown` tp=0、`infrastructure` fn=13。

**FR-C1** 归因必须按证据分层，优先级为：真实进程状态（取消/超时/拒绝）→ 退出码约定（127 命令不存在、126 不可执行、137 信号/OOM 击杀）→ 结构化 errno/系统码 → 环境对象故障 → 网络瞬断形态 → 源码诊断形态 → 无诊断内容则不归属。
**FR-C2** `transient`（可重试的网络/限流/中断）必须与 `infrastructure`（持续性的工具链/环境/配置故障）区分，因为二者运维动作不同：前者重试，后者停止并阻断。
**FR-C3** `unknown` 只允许来自"非零退出但输出里没有任何可识别证据"；`unknown` 在修复决策上按可修复处理，不得因为不归属就停止修复产品问题。
**FR-C4** 磁盘空间不足（ENOSPC/`no space left on device`/`disk quota exceeded`）、明确缺失工具（127/`command not found`/`linker ... not found`）、环境配置故障（`Cannot find module`、`Cannot find type definition file`、`missing script`、`could not find Cargo.toml`、`EACCES`）必须归为环境，且不得驱动无关代码修改。
**FR-C5** `product_blocked`（把产品错误标成环境问题）必须继续测量并保持 0。
**FR-C6** 归因规则不得靠无限堆砌宽泛关键词；每条判据必须绑定一个可复核的证据族（errno 名、退出码约定、源码位置/诊断号、网络错误码）。
**FR-C7** 口径调整（例如 `Temporary failure in name resolution` 与 `rate limit` 从"环境"改为"瞬态"）必须在评测口径变更说明里记录原因，并同时保留旧口径结果以便对比。
**FR-C8** 冻结语料不变；不接新模型或外部服务。
**AC-C1** 60 条样本的 4 类各自 tp/fp/fn 与 tune/holdout 分列可见。
**AC-C2** `unknown` 与 `transient` 的 tp 不再结构性为 0。
**AC-C3** 归因有针对退出码 127/126/137、ENOSPC、ECONNRESET、`test result: FAILED`、空输出的单测。

### D. 模型调用前的上下文容量保护（P1）

**触发条件 / 当前行为**（已核实）：
- `should_compact()`（`compaction.rs:83-92`）在 `last_input_tokens == 0` 时直接 `false`；`percent_used()`（`state.rs:178-186`）同样返回 0；`last_input_tokens` 只由**实测** usage 写入（`state.rs:144` ← `lib.rs:4439`）；
- 因此首轮请求（尚无 usage）与**不返回 usage 的服务商**（`token_usage: false`，`provider.rs:60/75`；`ProviderEvent::Usage` 仅在 `input_tokens > 0` 时发出）整轮永不压缩，UI 进度条恒为 0%；
- `initial_history()`（`lib.rs:2670-2757`）只把**会话历史**限到 32k 字符，对 system 规则、`request.message`、observations、memory/knowledge 块**没有任何上限**；
- `ContextTooLong` 不可重试、不 failover（`provider.rs:804-809`、`:849-851`），残余错误立刻 `break`（`lib.rs:5060-5067`）落到 offline_answer——**没有收缩重试**；
- `compact_if_needed()`（`lib.rs:2925`）在 `compact_history` 返回 `None` 时无 `else` 分支，静默跳过；`report.anchors_ok` 算出后被丢弃；
- 文本回退协议下工具结果以纯文本 user 消息追加（`lib.rs:2664`），`tool_group_boundary` 看不见，配对可能断裂。

**FR-D1** 发送前必须估算将要发送的输入量，并按服务商窗口减去输出预留。
**FR-D2** 无实测 usage 时使用保守估算（字符→token 启发式），并在预算与 UI 摘要中**明确区分估算与实测**，不得把估算当实测。
**FR-D3** 收缩必须保留：系统规则块、当前任务消息与任务约束、必要验收证据、最近工具调用与其结果的**完整配对**；配对不得被切开。
**FR-D4** 超窗恢复必须有次数上限（本轮取 2 次），超限后按现有终局路径处理；不得无限重试。
**FR-D5** 压缩失败不得静默：必须可见地上报失败原因并回退到有标注的截断路径，不得悄悄丢掉关键约束。
**FR-D6** 保持现有 Provider 请求体与工具协议兼容；不新增 tokenizer 依赖。
**AC-D1** 单测覆盖：首轮大输入、无 usage 服务商、工具结果持续增长、压缩失败回退、输出预留、调用/结果配对完整。
**AC-D2** 恢复次数上限有测试锁定（第 3 次不再尝试收缩）。

### E. 长任务与跨轮连续性（P1）

**触发条件 / 当前行为**（已核实）：
- `TaskPlan.constraints`（`plan.rs:151-158`）存在，但只来自硬编码列表（`plan.rs:994-1002` 等）；**用户口头约束从不进入**，因此"不要动 X"这类要求在超出最近 16 条 / 32k 字符历史窗口（`lib.rs:2679-2702`）后即不可恢复；
- `RecoveryCheckpoint`（`recovery.rs:70-127`）已持久化 plan/evidence/state/`file_versions`，`revalidate`（`recovery.rs:147-169`）保留文字契约、清空全部证据并列出变更文件——机制可复用；
- 同一轮内，后续文件写入不会使此前 `verify_ok` 失效（只有 resume 才强制重验）；
- 摘要与历史已按 `user` 消息注入（`compaction.rs:211-213`），不变量"Never promote history to system rules"（`lib.rs:2677-2678`）存在。

**FR-E1** 复用既有 `TaskPlan.constraints` + `RecoveryCheckpoint`，不新建并行机制。
**FR-E2** 从用户消息中提取**显式约束**（否定式禁令、保留要求、必须项）进入 `constraints`，带上来源标记；无显式标记时不猜测，提取数量有上限。
**FR-E3** 继续任务时重新装载约束与未完成项，并按 `file_versions` 核实文件证据是否过期；过期证据不得作为当前事实呈现。
**FR-E4** 同一轮内在文件被修改后，此前的验证通过结论必须标记为需重验，不得被当作仍然有效。
**FR-E5** 历史回答与摘要继续作为 `user` 内容注入，不得提升为系统指令；`AGENTS.md` 的作用域与优先级不变。
**FR-E6** 检查点不写入密钥或无关敏感内容；旧格式检查点记录（无新字段）必须仍可加载。
**AC-E1** 单测覆盖：早期约束超出历史窗口后仍在、任务中断恢复、恢复前文件被改动、旧验证结果失效、旧格式记录兼容。

## 4. 成功标准

1. 默认离线评测 exit 0，且 `default`/`real`/`decision` 三类检查数量与结果清晰可见。
2. 路由与归因在**冻结语料**上按类别与 tune/holdout 分列报告，给出分母与口径变更说明。
3. 上下文保护在无 usage 服务商与首轮大输入下确实生效，恢复有上限，配对不破坏。
4. 所有新增行为有对应回归单测；`cargo check`/`cargo test`/`cargo fmt --check`/`git diff --check` 通过。
5. 不以分类指标宣称真实编程成功率提升；真实模型评测本轮不运行，如实标 `NOT RUN`。
