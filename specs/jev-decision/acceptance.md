# Jev 决策辅助 · 验收记录

日期：2026-10-20（Asia/Shanghai）。基线：`c0f67a310cb7dea1761f14e017381407a0e07a81`（本地 `main` = 缓存 `origin/main`；本次未 fetch，远程新鲜度未核验）。任务分支：`feat/jev-decision`。

**总体状态：本地实现已验证，待授权集成。** 代码路径、契约、授权机制与模拟验收都在本地完成；真实 Jev 推理请求、真实模型效果评测与 `main` 集成都没有做，也没有获得授权。

---

## 1. 交付范围

| 文件 | 内容 |
| --- | --- |
| `crates/agent/src/decision.rs` | 场景/模式/设置类型与解析、固定问题模板、请求/响应 DTO、响应校验顺序、纯采纳策略与阈值、HTTP 客户端、决策记录与轨迹载荷、Agent 侧接入（`consult_task_route` / `consult_failure_triage`） |
| `crates/agent/src/classify.rs` | `classify_with_evidence()`：基线类别、规则分、命中数、是否默认兜底、是否 ambiguous；`classify()` 保留为兼容包装 |
| `crates/agent/src/verify.rs` | `failure_evidence_origin()` 与 `failure_triage_eligible()`：把既有失败判断分为 `hard_evidence` / `heuristic` / `default`，不改动既有枚举的持久化格式 |
| `crates/agent/src/lib.rs` | `RunRequest.decision: Option<DecisionHandle>`；任务分类接入点在 `conversational` 判定之后、`skill` 选择之前；失败分流接入点在 `RepairDecision::from_evidence` 之后 |
| `crates/shell/src/decision.rs` | 运行级服务：独立配置、独立凭据命名空间、配置代次、最小状态、发送预览、一次性授权与原子消费、每轮额度、并发上限、熔断、敏感内容拦截 |
| `crates/shell/src/lib.rs` | 导出决策服务的公共 API |
| `crates/cli/src/decision.rs`、`turn.rs`、`main.rs` | CLI 的交互式预览（stderr）与非交互拒绝；无 TTY 时不注入句柄 |
| `apps/desktop/src-tauri/src/decision.rs` | 设置 IPC、凭据 IPC、预览/确认 IPC、运行级授权桥接（`RunEvent::DecisionPreview`） |
| `apps/desktop/src-tauri/src/run.rs`、`scheduler.rs`、`main.rs`、`view.rs` | 运行句柄的注入与释放、后台调度固定 `decision: None`、`DecisionPreview` 事件、停止运行时清理待批准项 |
| `apps/desktop/src/decisionApi.ts` | 前端 API 与 DTO；决策记录解析（无法识别的 schema 返回 `null`） |
| `apps/desktop/src/pages/DecisionSettings.tsx` | 设置小节：模式、模型版本、场景开关、密钥、显式外发说明 |
| `apps/desktop/src/conversation/DecisionPreviewDialog.tsx`、`decision.css` | 发送预览：接收方、模型、完整请求体、问题与答案空间、两个按钮（默认焦点为跳过） |
| `apps/desktop/src/inspector/DecisionRecords.tsx` | Inspector 的决策记录：建议、是否采纳、概率分布、耗时、等待确认、回退原因 |
| `apps/desktop/src/i18n.ts`、`api.ts`、`pages/SettingsPage.tsx`、`pages/ConversationPage.tsx`、`inspector/TraceInspector.tsx` | 文案、事件类型、设置入口、预览事件处理、Inspector 接入 |
| `evals/src/decision.rs`、`evals/fixtures-decision/*.jsonl` | 虚构语料（120 条任务分类 + 60 条失败分流）、本地规则基线、margin 敏感性扫描、离线 state 契约检查 |
| `specs/jev-decision/{spec,plan,tasks,acceptance}.md` | 规格、方案、任务、验收记录 |

**没有新增依赖**：`ureq 2.12`（kodo-agent 已有）、`serde` / `serde_json`（各 crate 已有）、`kodo-core::settings`（凭据与设置）、`kodo-core::redact`（敏感内容筛查）全部复用。

---

## 2. 验收对应（AC-001～AC-020）

| 验收 | 证据 | 结论 |
| --- | --- | --- |
| AC-001 | `crates/shell/tests/decision_transport.rs::off_makes_no_request_at_all`（模拟服务器收到 0 请求）；`decision_scenarios.rs::without_a_handle_nothing_is_asked_and_nothing_changes` | **通过** |
| AC-002 | `decision_scenarios.rs::observe_records_the_suggestion_without_changing_the_task`（记录 `status=observed`、`applied=false`，任务类别不变） | **通过** |
| AC-003 | `decision_scenarios.rs::assist_changes_only_the_task_category`（只改 `task_type`；预算仍取基线，见 `plan.md` 偏差 3） | **通过（模拟）** |
| AC-004 | `decision_scenarios.rs::work_mode_never_asks` / `a_conversation_never_asks` / `an_explicit_skill_never_asks` | **通过** |
| AC-005 | `crates/agent/src/decision.rs` 的 `a_model_mismatch_is_rejected`、`a_negative_probability_is_rejected`、`probabilities_that_do_not_sum_to_one_are_rejected`、`an_unknown_option_is_rejected`、`a_missing_option_in_the_probability_set_is_rejected`、`a_choice_that_is_not_the_top_option_is_rejected`、`an_unexpected_question_id_rejects_the_whole_response`、`a_confidence_outside_the_unit_interval_is_rejected`、`a_negative_usage_count_is_rejected` | **通过** |
| AC-006 | `crates/shell/src/decision.rs::a_refusal_is_remembered_for_the_rest_of_the_run`（第二次不弹窗、不发送） | **通过** |
| AC-007 | `a_changed_configuration_invalidates_the_grant`（`config_changed`）、`a_changed_payload_invalidates_the_grant`（`snapshot_changed`）、`another_sessions_grant_is_refused`（`consent_expired`） | **通过** |
| AC-008 | `a_grant_is_consumed_once`（令牌先取出再校验，第二次找不到）、`a_consent_is_answered_once`（桌面侧）、`a_consent_from_a_finished_run_cannot_be_answered` | **通过** |
| AC-009 | `verify.rs::failure_triage_eligible` 只放行"普通非零退出 + 默认 Product"；`decision_scenarios.rs::a_cancelled_decision_stops_the_round`（取消后 0 个命令步骤） | **通过（单元级）** |
| AC-010 | `decision::consult_failure_triage` 的四分支；`infrastructure` → 既有 `infra` 阻塞分支（不标完成），`transient` → 只记建议（见 `plan.md` 偏差 9） | **通过（代码级）** |
| AC-011 | `decision_transport.rs`：`an_authentication_failure_is_reported_and_not_retried`、`a_rate_limit_is_reported_and_not_retried`、`a_server_error_is_reported_and_not_retried`、`a_redirect_is_refused`、`an_invalid_body_is_refused`、`an_oversized_response_is_refused`、`a_slow_answer_times_out_without_a_retry`、`a_model_mismatch_is_refused` —— 全部断言"只发一次" | **通过** |
| AC-012 | 等授权时取消 → `a_cancelled_decision_stops_the_round`；发送前取消 → `a_cancelled_run_never_sends`；授权后配置/代次变化 → `a_changed_configuration_invalidates_the_grant` | **通过** |
| AC-013 | `crates/shell/src/decision.rs::the_per_turn_budget_stops_further_requests`（`budget_exhausted`）；并发上限 2 在服务内以原子计数实现，额度不足直接跳过不排队 | **通过（额度）**；并发上限本身**未单独实测** |
| AC-014 | 决策记录只进 `Step::Reasoning.diagnostics`；`decision_scenarios.rs` 断言记录里**不含**用户消息；`crates/shell/src/decision.rs::a_decision_record_never_reaches_the_memory_outbox`；密钥明文只存在于 Rust 内存 | **通过（代码级）**；`local_trace` 打开时的落盘文件内容**未做端到端落盘断言** |
| AC-015 | `crates/cli/src/turn.rs::decision_handle` 在无 TTY 时返回 `None`；`apps/desktop/src-tauri/src/scheduler.rs` 的后台运行固定 `decision: None` | **通过（代码级）**；CLI 的无 TTY 行为**未做真实管道运行验证** |
| AC-016 | `crates/shell/src/decision.rs::clearing_the_decision_credential_leaves_the_others_alone`（provider 与 MCP 凭据不受影响） | **通过** |
| AC-017 | `a_secret_shaped_state_is_never_sent`（`sensitive_input`，0 请求、0 弹窗）、`an_oversized_state_is_refused_before_the_prompt`（`payload_limit`） | **通过** |
| AC-018 | `a_consent_is_answered_once`（最多一次发送）、`a_consent_from_a_finished_run_cannot_be_answered`（旧结果不能采纳） | **通过** |
| AC-019 | 语料含否定式（tr-059～tr-062）、注入式（tr-063～tr-065）、混合中英文（tr-069、tr-070）；`evals` 的基线报告逐条列出误判 | **部分通过** —— 硬约束优先级已验证（本地规则优先、约束不被改写）；**误判率本身未评估**（需要真实模型） |
| AC-020 | 决策记录是普通 `ItemKind::Reasoning`，会话日志可持久化并在 Inspector 显示；`conversation_history` 只回放 `AgentMessage`（`crates/shell/src/turn.rs` 既有测试），Memory outbox 明确排除 Reasoning（新增断言） | **通过（代码级）** |

---

## 3. 验证命令与原始结果摘要

全部在本机执行，没有调用真实 Jev 端点、没有读取真实凭据、没有连接用户状态库。

```text
cargo fmt --all -- --check
(exit 0)

cargo check --offline --locked --workspace --all-targets
    Finished `dev` profile [unoptimized + debuginfo] target(s)

cargo test --offline --locked -p kodo-agent --lib
test result: ok. 602 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.48s

cargo test --offline --locked -p kodo-agent --test decision_scenarios
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.74s

cargo test --offline --locked -p kodo-shell --lib
test result: ok. 92 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.29s

cargo test --offline --locked -p kodo-shell --test decision_transport
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s

cargo test --offline --locked -p kodo-desktop --bin kodo-desktop
test result: ok. 48 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s

cargo test --offline --locked -p kodo-core --lib
test result: ok. 134 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s

cargo test --offline --locked -p kodo-evals
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

npm --prefix apps/desktop run typecheck
> node ./node_modules/typescript/lib/tsc.js --noEmit
(exit 0)

npm --prefix apps/desktop run build
✓ built in 1.21s
（仍有项目原有的大包体积提示；本任务没有改动打包方案）

node apps/desktop/node_modules/@playwright/test/cli.js test \
  --config apps/desktop/playwright.config.ts --project=behaviour --workers=2
Running 359 tests using 2 workers
359 passed (2.5m)
（其中 2 项是本次新增：决策辅助设置小节、发送预览）
```

### 离线评测（`cargo run --offline -p kodo-evals`）原始输出

```text
kodo-evals decision layer (offline, local rules only)

--- decision · task route (local rule baseline) ---
task_route_samples: 120 (58 tune / 62 holdout)
task_route_accuracy: 0.608
task_route_macro_f1: 0.604
task_route_ambiguous: 110 (91.7% of the corpus)
task_route_ambiguous_correct: 65 (59.1% of the unsure cases)
  task_route[bug-fix]: tp=18 fp=1 fn=3
  task_route[feature]: tp=15 fp=34 fn=6
  task_route[test]: tp=13 fp=4 fn=1
  task_route[refactor]: tp=9 fp=2 fn=8
  task_route[code-review]: tp=9 fp=1 fn=5
  task_route[docs]: tp=9 fp=5 fn=4
  task_route[unknown]: tp=0 fp=0 fn=20
task_route_margin_sweep (ambiguous samples per margin):
  margin=0: 41 (34.2%)
  margin=2: 82 (68.3%)
  margin=4: 110 (91.7%)
  margin=8: 110 (91.7%)
  margin=12: 110 (91.7%)

--- decision · failure triage (local rule baseline) ---
failure_triage_samples: 60
failure_triage_accuracy: 0.383
failure_triage_macro_f1: 0.203
failure_triage_ambiguous: 34 (56.7% of the corpus)
failure_triage_product_blocked: 0 (0.0% — a product failure the local rule would treat as an environment failure)
  failure_triage[product]: tp=20 fp=34 fn=0
  failure_triage[infrastructure]: tp=3 fp=3 fn=13
  failure_triage[transient]: tp=0 fp=0 fn=15
  failure_triage[unknown]: tp=0 fp=0 fn=9

--- decision · offline contract check ---
state_contract_ok: 360/360 (the state carries only the allowed fields)

--- decision · model comparison ---
model_comparison: NOT RUN — a cloud evaluation needs a separate authorization
quality_gate: NOT EVALUATED — the offline numbers above are the local rule's, not the model's
```

### 检查中发现的问题

- **设计建议的 margin=4 让 91.7% 的语料被判为"含糊"。** 规则分是关键词权重，典型单类命中只有 1～6 分，因此"最高分减次高分 ≥ 4"几乎只在多关键词叠加时成立。margin 扫描显示 0/2/4 分别对应 34.2% / 68.3% / 91.7%。这不影响功能正确性（每次都要逐次授权），但它直接决定"适用样本覆盖率"与"弹窗频次"，必须在质量门评测里重新标定。本期按设计实现 4，并把扫描写进 `evals`。
- **概率差门槛在固定选项集下是冗余的。** n 个选项时次高概率不会超过 `1 - 最高概率`，所以最高概率 ≥ 0.90 必然推出概率差 ≥ 0.80。门槛仍按设计实现并有专门用例（`the_margin_rule_is_never_the_binding_constraint`），但实际起作用的只有最高概率门槛。
- **语料规模不足。** 设计要求 600 条任务分类 + 300 条失败分流；本次落地 120 + 60 条，够跑通离线对照与契约检查，不足以支撑质量门的统计结论。
- **本地规则没有 `unknown` 桶。** 规则无法归类时一律落到 `feature`，所以语料里 20 条 `unknown` 全部记为误判。这正是"模糊场景"的定义，不是语料错误。
- `evals/fixtures/` 的每个目录都会被 fixture 层当作"带 oracle 的任务仓库"扫描。语料最初放在 `evals/fixtures/decision/` 时破坏了 `splits_cover_all_fixtures` 门，已移到 `evals/fixtures-decision/`。
- **决策记录原本只发了 Started 事件**，会话日志里会留下一个永远"运行中"的悬空条目。已改为复用既有的 `simple_step`（Started + Finished），并补了 `a_decision_record_leaves_no_dangling_item` 用例。
- 设置页的既有视觉用例 `each Settings category swaps the detail pane` 断言了设置分类的完整列表，新增"决策辅助"后必须同步更新；已更新并重跑。
- 发送预览的"接收方"原本会在主机名中间断行（`overflow-wrap: anywhere`）。已改为独占一行 + `break-word`，URL 现在完整显示。

---

## 4. 尚未通过 / 未验证的项

| 项 | 状态 | 说明 |
| --- | --- | --- |
| 真实模型效果（质量门） | **未评估** | 需要一次单独授权的云端评测；模拟响应通过**不等于**模型效果达标 |
| 真实 Jev 端点连通性、地区延迟、有效价格、额度 | **未验证** | 本次授权不含真实推理请求 |
| 视觉回归基线（`toHaveScreenshot`） | **未补** | 新增了 2 项交互用例（`apps/desktop/tests/visual/decision.spec.ts`）并人工核对了 1586×992 与 900×900 两张截图；没有新增像素基线，避免在未确认基线的情况下锁定渲染 |
| Inspector 决策记录的实际渲染 | **未截屏核对** | 组件与数据接线完成，`typecheck`/`build` 通过；记录只在真实运行产生，本地没有产生该记录的界面状态 |
| `local_trace` 打开时的落盘内容断言 | **未做端到端断言** | 代码级保证：记录只进 `Reasoning.diagnostics`，内容不含请求正文 |
| CLI 无 TTY 的真实管道运行 | **未实测** | 代码级保证：`decision_handle` 在无 TTY 时返回 `None` |
| 并发上限（2 个在途）的实测 | **未单独实测** | 代码级保证：原子计数，拿不到名额直接 `busy` 跳过 |
| 取消回收时间（请求超时 + 250ms 目标） | **未实测** | 本地 ureq 行为需要专门测量；`ureq` 的阻塞读无法被立即打断 |
| 熔断的 60 秒冷却与探测请求 | **未做时间相关实测** | 代码级实现：`record_transport_failure` 累计 3 次后开路 60 秒 |

---

## 5. 真实模型评测仍需的授权、样本与步骤

### 授权

1. 对 `https://api.typesafe.ai/v1/systemone` 发起真实推理请求的单独授权（含固定模型 `jev-1.13.0`）。
2. 一个可用的 TypeSafe 凭据，写入 `credentials.log` 的 `decision` 命名空间（`decision.jev`）。
3. 允许把**虚构语料**发送到该端点；不允许上传真实项目资料、历史会话或日志。

### 样本

- 任务分类：设计要求 600 条（200 调参 + 400 冻结留出），中文 ≥ 60%，覆盖六个类别 + unknown + 多意图 + 否定 + 恶意指令。本次已有 120 条，需扩到 600 条。
- 失败分流：设计要求 300 条（100 调参 + 200 留出），覆盖产品/环境/短暂/证据不足。本次已有 60 条，需扩到 300 条。
- 标签需人工确定；争议样本单列，不把基线输出当真值。留出集不得用于提示词微调。

### 步骤

1. 用现有 `evals/fixtures-decision/` 跑通离线基线（已完成）。
2. 补齐语料并冻结；冻结后不得再改标签。
3. 补一条 `--decision-live` 路径：同一批样本、同一份合法输入，对照三组——现有规则 / 现有主模型的结构化判断 / 规则加 Jev；记录全部实际请求成本与端到端耗时。
4. 分别计算两个场景的门槛指标：任务分类留出集 Macro-F1 相对规则的提升（≥ 3 个百分点）、采纳子集准确率（≥ 100 个样本、≥ 97%，报告置信区间）、适用样本覆盖率（≥ 30%）；失败分流模糊样本准确率（≥ 95%）与产品问题误阻塞率（≤ 1%，必须报告分母与区间）。
5. 中文结果单独统计与报告。
6. 同时给出 hard-constraint 用例的通过情况（AC-003/004/007/008/009/012/014/015/017/018/020）——这些不受评分控制。
7. 依据结果决定：维持 Observe、开启单个场景的 Assist，或整体停用。达不到就维持观察，**不通过继续放宽阈值来掩盖退化**。
8. margin 常量与采纳阈值在冻结后改动，必须重跑对应留出评测。

---

## 6. Git 现场

**已合入本地 `main`**（用户明确授权"合并至main"）。**未推送**：本次授权只到本地合并，`git push` 与创建 PR 均未执行。

```text
当前分支：main
main HEAD：91eb96b merge(decision): integrate the optional Jev decision assistant
本任务提交：
  aaa9a84 docs(decision): add the Jev decision specification
  bd5bd60 feat(decision): add the optional Jev decision assistant
  c4e52b3 docs(decision): add the Jev decision acceptance record
合并前基线：c0f67a310cb7dea1761f14e017381407a0e07a81（= 合并前 main = 合并前缓存 origin/main）
缓存 origin/main：c0f67a310cb7dea1761f14e017381407a0e07a81 —— 本次未 fetch，远程新鲜度未核验
main 相对 origin/main：ahead 5（3 个任务提交 + 1 个合并提交 + 1 个本记录提交；全部未推送）
git branch --no-merged main：（空）
任务分支 feat/jev-decision：已合入后按 AGENTS.md §2 删除（三个提交均保留在 main 的历史里）
git stash list：7 条，全部为会话开始前既有，未 pop / drop / clear
```

合并方式：`--no-ff`，无冲突，保留任务边界。合并后重跑了改动范围对应的验证（见下）。

worktree：

```text
/Users/lijianxue/work/WorkSpace/kodo                    91eb96b [main]
/Users/lijianxue/WorkBuddy/Worktrees/kodo/main-df55d246 c0f67a3 [fix/work-reliability]
```

**归属未确认的既有现场（本任务未触碰）**：第二个 worktree 有未提交改动 —— `crates/agent/src/dedup.rs`、`crates/agent/src/lib.rs`、`crates/agent/src/tools.rs` 已修改，`crates/agent/src/read_receipt.rs` 未跟踪。它们在另一个目录，本任务不覆盖、不暂存、不提交、不清理。注意 `crates/agent/src/lib.rs` 与本任务改动的是同一个文件的不同区域：若该 worktree 的改动后续要合入 `main`，需要逐段核对冲突。

未提交文件清单：

```text
修改（20）：
  apps/desktop/src-tauri/src/{main,run,scheduler,view}.rs
  apps/desktop/src/{api.ts,i18n.ts}
  apps/desktop/src/inspector/TraceInspector.tsx
  apps/desktop/src/pages/{ConversationPage,SettingsPage}.tsx
  apps/desktop/tests/visual/behaviour.spec.ts        （设置分类列表新增一项）
  crates/agent/src/{classify,lib,verify}.rs
  crates/agent/tests/{agent_benchmark,online_benchmark}.rs
  crates/cli/src/{main,turn}.rs
  crates/shell/src/lib.rs
  evals/src/{main,work}.rs
  （已跟踪文件合计 +752 / -27）

新增（17）：
  apps/desktop/src-tauri/src/decision.rs                       393 行
  apps/desktop/src/conversation/DecisionPreviewDialog.tsx      118 行
  apps/desktop/src/conversation/decision.css                   258 行
  apps/desktop/src/decisionApi.ts                             229 行
  apps/desktop/src/inspector/DecisionRecords.tsx                81 行
  apps/desktop/src/pages/DecisionSettings.tsx                  253 行
  apps/desktop/tests/visual/decision.spec.ts
  crates/agent/src/decision.rs                                1809 行
  crates/agent/tests/decision_scenarios.rs                     512 行
  crates/cli/src/decision.rs                                   106 行
  crates/shell/src/decision.rs                                1439 行
  crates/shell/tests/decision_transport.rs                     474 行
  evals/src/decision.rs                                        528 行
  evals/fixtures-decision/task_route.jsonl                     120 条
  evals/fixtures-decision/failure_triage.jsonl                 60 条
  specs/jev-decision/{spec,plan,tasks,acceptance}.md
```

**本任务没有执行**：提交、合并、推送、创建 PR、删除分支/worktree/stash、修改远程。

---

## 7. 剩余工作与恢复路径

### 剩余工作

1. **远程交付**：`main` 已在本地包含本任务，但**尚未推送**，也没有 PR。远程同步与 PR 需要单独授权；本地合并的"远程新鲜度未核验"状态仍然成立。
2. **视觉与交互回归**：按 `docs/design/UI_ACCEPTANCE.md` 为设置小节、发送预览、Inspector 决策记录补 `apps/desktop/tests/visual/` 用例，并在参考视口（1586×992 / 1440×900 / 1920×1080 / 2560×1440）截图核对。
3. **语料扩量与冻结**：120→600、60→300，并冻结标签。
4. **真实模型评测**：见第 5 节，需要单独授权。
5. **margin 常量重新标定**：当前 4 使 91.7% 的语料判为含糊，应在冻结语料上重标。
6. **取消回收与并发上限的专项实测**：受控网络下测量 ureq 的取消回收行为，决定是否保留远程路径。
7. **连接检查按钮**：设计 §11.1 要求"用户显式点击才发起"；本次没有实现该命令，真实外发需要单独授权。

### 合并后复验（合并树上重跑）

```text
cargo fmt --all -- --check                → FMT OK
cargo check --offline --locked --workspace --all-targets → Finished
cargo test --offline --locked --workspace → total passed: 1027  failed: 0
npm --prefix apps/desktop run typecheck   → (exit 0)
npm --prefix apps/desktop run build       → ✓ built in 2.39s
playwright --project=behaviour --grep "决策辅助设置小节|发送预览展示完整请求内容|each Settings category swaps the detail pane" → 3 passed
```

### 恢复路径

- 全部实现已提交并合入本地 `main`（`91eb96b`），三个提交在 main 的历史里可追溯；`git stash` 未使用。
- 分支 `feat/jev-decision` 已删除；要单独查看任务边界可用 `git log c0f67a3..91eb96b`。
- 7 条 stash 与第二个 worktree 的未提交改动保持原样，未被本任务改动。
- 文档：`specs/jev-decision/` 四件套是下一次续接的入口；续接时先重新读 `crates/agent/src/decision.rs` 与 `crates/shell/src/decision.rs` 的当前内容（本文档记录的行号可能已过时）。
