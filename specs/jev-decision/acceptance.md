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

## 3.1 OpenRouter 来源（FR-015）

2026-10-20 追加：决策请求可以走 OpenRouter，一次只启用一个来源。

| 验收点 | 证据 | 结论 |
| --- | --- | --- |
| 来源可配置，目的地/凭据/格式随之切换 | `the_openrouter_path_sends_its_own_shape_to_its_own_path`（请求打到 `/api/v1/chat/completions`，带 OpenRouter 自己的密钥，body 是 chat completions） | **通过** |
| 单字母作答，首 token 概率归一化 | `the_openrouter_request_asks_for_a_single_letter`、`an_openrouter_answer_becomes_a_probability_distribution` | **通过** |
| 没有概率分布就不采纳 | `an_openrouter_answer_without_logprobs_is_never_adoptable`（传输层）、`an_uncalibrated_answer_is_recorded_but_never_applied`（接入层，`applied=false`） | **通过** |
| 模型一致性 | `an_openrouter_answer_for_another_model_is_refused`、`the_openrouter_model_check_allows_a_resolved_id`（允许日期后缀与 provider 前缀，拒绝另一个模型） | **通过** |
| 凭据按来源隔离 | `decision.set_credential` / `clear_credential` 带来源参数；`clearing_the_decision_credential_leaves_the_others_alone` 扩展为"清一个来源、留另一个" | **通过** |
| 失败不自动换源 | 服务按 `settings.provider` 选一个客户端；没有任何跨来源回退分支 | **通过（代码级）** |

**未验证**：OpenRouter 的真实连通性、真实模型的字母服从率与 logprobs 可用性、各路由器的限流与价格 —— 都需要单独授权的联网评测。

## 3.2 默认模型与存储格式修正（2026-10-20 追加）

| 问题 | 原因 | 修正 | 证据 |
| --- | --- | --- | --- |
| OpenRouter 默认模型 | 按用户要求 | `typesafe/jev-1.14`（官方端点仍固定 `jev-1.13.0`） | `each_provider_has_its_own_default_model`、前端「切换服务来源」用例 |
| 保存配置报错 `jev-decision: EOF while parsing an object at line 1 column 1` | `settings.log` 是行式 `key=value`，而记录用 `to_string_pretty` 写成多行；读回时只剩第一行 `{` | 存盘前用 `DecisionSettings::to_json()` 归一化成**单行** JSON，并拒绝含换行的值 | `a_saved_record_can_be_read_back`、`a_pretty_printed_record_is_normalized_before_it_is_written`、`a_damaged_record_says_how_to_repair_it` |
| 修复后仍报错 | 旧的截断记录仍在日志里，读出来还是 `{` | 增加**恢复读取**：值解析失败时，把日志里紧跟其后的行重新拼回去直到 JSON 复原；复原后按紧凑格式回写一份 | `a_truncated_record_is_recovered_and_written_back`；用本机 `settings.log` 副本实测：`config_error=None`、`mode=assist`、`provider=openRouter`、`model=typesafe/jev-1.14` |

**用户现场的修复方式**：本机 `settings.log` 第 391–407 行仍完整留着那条被截断的记录（`mode=assist`、`provider=openRouter`、`model=typesafe/jev-1.14`），只是被切成了多行。恢复读取会把这些行重新拼回去 —— **设置值不丢**，并回写一份单行记录。以本机 `settings.log` 的副本实测：首次加载 `config_error=None`、三项设置原样恢复，记录被改写为单行，第二次加载无任何问题。旧的多行残留在日志里不再影响读取（`parse_line` 不含 `=` 的行直接忽略）。代码不重写用户既有数据，只追加一条正确的记录。

**接入 OpenRouter 带来的新风险（尚未评估）**：官方 Jev 给出的是服务自己标注的概率；OpenRouter 给出的是 chat 模型首 token 的 token 概率经选项字母归一化后的分布。两者都**不是**"准确率"，但后者对模型的服从度、分词器与提示词更敏感。因此 OpenRouter 路径在质量门评测里应当**单独统计**，不能与官方路径合并成一个指标。

## 3.3 腾讯 EdgeOne 来源（FR-015 扩展，2026-10-21）

用户提供 [腾讯云 EdgeOne Makers 文档](https://cloud.tencent.com/document/product/1552/138731)，要求把腾讯加入 Jev 的服务来源。文档描述的是 System One 原生协议，模型别名 `@makers/jev`，与 TypeSafe 的请求/响应形状一致。

| 验收点 | 证据 | 结论 |
| --- | --- | --- |
| 来源可配置，目的地/凭据/请求形状随之切换 | `the_tencent_path_sends_the_native_shape_to_its_own_path`：请求打到 `/v1/systemone`，带 `Bearer synthetic-tencent-key`，body 的 `model` 是 `@makers/jev`，含 `questions`、不含 `messages` | **通过** |
| 别名解析后的模型被接受 | `a_tencent_answer_resolving_the_alias_is_accepted`：请求 `@makers/jev`、响应 `jev-1.13.0`，采纳记录的模型是解析后的 `jev-1.13.0` | **通过** |
| 其他模型被拒绝 | `a_tencent_answer_for_another_model_is_refused`（传输层，`model mismatch`，无重试）、`the_tencent_model_check_accepts_only_the_resolved_alias`（拒绝 `jev`、`jev-1.13.0-extra`、其他模型与裸模型 id） | **通过** |
| 设置只接受固定别名 | `tencent_rejects_a_different_model_on_save`；前端 `腾讯来源固定使用 @makers/jev 并锁定模型输入`（模型框禁用，切换来源后恢复） | **通过** |
| 凭据按来源隔离 | `clearing_one_providers_credential_keeps_the_other_one` 扩展为三条密钥：清 TypeSafe 后 OpenRouter 与腾讯的密钥都保留 | **通过** |
| 设置页展示第三个来源 | 视觉用例断言 `decision-provider-tencent` 可见、外发说明包含「腾讯 EdgeOne」 | **通过** |
| 失败不自动换源 | 服务按 `settings.provider` 选一个客户端；没有跨来源回退分支 | **通过（代码级）** |

**未验证**：腾讯 EdgeOne 的真实连通性、`Authorization: Bearer $MAKERS_MODELS_KEY` 鉴权头是否与文档一致、实际解析出的版本号、限流与价格、以及 `@makers/jev` 指向的构建是否与评估所用版本相同 —— 都需要单独授权的真实请求。上述实现全部使用回环模拟服务与虚构凭据验证。

## 4. 尚未通过 / 未验证的项

| 项 | 状态 | 说明 |
| --- | --- | --- |
| 真实模型效果（质量门） | **未评估** | 需要一次单独授权的云端评测；模拟响应通过**不等于**模型效果达标 |
| 真实 Jev 端点连通性、地区延迟、有效价格、额度 | **未验证** | 本次授权不含真实推理请求 |
| 视觉回归基线（`toHaveScreenshot`） | **未补** | 现有 4 项交互用例（`apps/desktop/tests/visual/decision.spec.ts`）已通过；没有新增像素基线，避免在未确认基线的情况下锁定渲染 |
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

**已合入并推送到 `main`**（用户先授权"合并至main"，随后授权"全部代码 分支 worktree 都合并到main并提交推送"）。推送结果：`c0f67a3..f3073b7  main -> main`；`main` 与 `origin/main` 同为 `f3073b7`。**未创建 PR**，未删除远端分支。

> **并发记录（2026-10-20 22:40）**：本任务进行期间，另一个会话把 `fix/work-reliability` 分支合并进了 `main`（合并提交 `46b9f53 merge(work): …`），其合并基点正是本任务的 `a9304ea`。本任务的提交全部保留在 `main` 历史中，未被覆盖；本轮修复在合并后的树上重新验证通过。

```text
当前分支：main（唯一分支）
main HEAD：05c0ca8 docs(decision): record the consolidated and pushed state
origin/main：与本地一致（已推送）
本任务提交：
  aaa9a84 docs(decision): add the Jev decision specification
  bd5bd60 feat(decision): add the optional Jev decision assistant
  c4e52b3 docs(decision): add the Jev decision acceptance record
  0b1f96e docs(decision): record the local merge into main
  e8381f6 feat(decision): let the decision service talk to OpenRouter
  15c8718 fix(decision): store the settings record on one line and pick a new default model
  9a92f1f fix(decision): recover a truncated settings record instead of failing
本任务合并前的基线：c0f67a310cb7dea1761f14e017381407a0e07a81
推送结果：c0f67a3..f3073b7  main -> main
git branch --no-merged main：（空）
git branch -r --no-merged main：（空 —— 远端的 feat/scheduled-task-experience
  与 fix/output-token-limit 也都已包含在 main 里）
本地分支：仅 main
git stash list：7 条，全部为会话开始前既有，未 pop / drop / clear
```

合并方式：`--no-ff`，无冲突，保留任务边界。合并后重跑了改动范围对应的验证（见下）。

worktree（清理后）：

```text
/Users/lijianxue/work/WorkSpace/kodo  05c0ca8 [main]
```

`fix/work-reliability` 与 `workbuddy/main-df55d246` 两个已合并的本地分支已删除；
`fix/work-reliability` 所在的 worktree 已移除（它的代码早已随 `46b9f53 merge(work)` 进入 `main`）。

**那个 worktree 里唯一剩下的未跟踪内容**是另一个会话的笔记
`.workbuddy-ai/memory/2026-10-20.md`（非产品代码）。它已逐字节校验后**保全**在
`/Users/lijianxue/WorkBuddy/Worktrees/kodo/main-df55d246-workbuddy-notes/`，
worktree 内的那份副本才被清理。

本任务改动过的文件清单（已全部提交并推送）：

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

**本任务执行了**：提交、合并到 `main`、推送 `main`、删除已合并的本地分支、移除已完成的 worktree。**未执行**：创建 PR、删除远端分支、清理 stash、修改远端的任何其他内容。

---

## 7. 剩余工作与恢复路径

### 剩余工作

1. **PR**：没有创建。`main` 已推送（`f3073b7`），但按项目惯例仍应走 PR 流程由他人评审；如需 PR，请单独授权。
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

- 全部实现已提交并合入本地 `main`（`4189423`），提交在 main 的历史里可追溯；`git stash` 未使用。
- 任务分支 `feat/jev-decision`、`feat/jev-openrouter`、`fix/jev-settings-record`、`fix/jev-recover-truncated-record` 均已在合入后删除；要单独查看任务边界可用 `git log c0f67a3..4189423`。
- 7 条 stash 与第二个 worktree 的未提交改动保持原样，未被本任务改动。
- 文档：`specs/jev-decision/` 四件套是下一次续接的入口；续接时先重新读 `crates/agent/src/decision.rs` 与 `crates/shell/src/decision.rs` 的当前内容（本文档记录的行号可能已过时）。

## 8. 2026-10-21 评审问题修复（最新记录）

状态：**本地修复与验证已完成，用户已明确授权本地提交、以 --no-ff 合入 main，并删除已合并任务分支。** 本节 Git 哈希及文件清单是提交前快照，最终提交和合并结果见本轮交付说明及 Git 历史。此状态只针对 `fix/jev-decision-review`，不改写上文既往交付的历史。本轮从本地 main `a7133b220cefce56c3da760c3096f3c5f9b76507` 创建唯一任务分支，工作目录仍是原仓库，7 条既有 stash 原样保留。

### 8.1 问题与证据

| 修复项 | 行为 | 新增或增强的验证 |
| --- | --- | --- |
| RF-001 | 授权绑定服务配置代次，预览前和响应后失效检查；Agent 采纳前复核 | `stale_configuration_never_reaches_the_server`、`configuration_changed_during_http_discards_the_answer`、`cancellation_during_http_discards_the_answer`、`the_agent_rechecks_host_validity_before_adopting_a_result` |
| RF-002（历史，已被 RF-006/R007 覆盖） | 当时真实质量门未评估，生产 Assist 有效模式强制 Observe；设置返回 `assistAvailable=false`，界面禁用并解释原因 | 历史用例 `unevaluated_assist_is_effectively_observe`、`saved_assist_is_observe_even_with_a_high_probability_response`；2026-10-21 用户要求开放 Assist 后，改为 `assist_is_selectable_and_the_default_stays_off`、`saved_assist_applies_a_high_probability_response` |
| RF-003 | 正文和生成 Token 精确单字母匹配；不补齐缺失选项、不重分配缺失概率质量 | `openrouter_refusals_and_explanations_are_not_option_letters`、`openrouter_generated_token_must_match_the_answer`、`openrouter_does_not_invent_missing_option_probabilities` |
| RF-004 | 恢复仅回写校验成功后的单行 JSON，写入失败显示配置错误；配置指纹使用规范化内容 | 增强 `a_truncated_record_is_recovered_and_written_back`，断言第二次 recovered=false、字节内容和配置代次不变 |
| RF-005 | 生产服务使用进程共享并发槽和来源/凭据代次熔断，运行级授权/预算不共享；作用域释放名额及探测位 | `three_runs_share_the_same_two_network_slots`、`multiple_runs_share_two_slots_and_release_them_on_drop`、`authentication_block_is_shared_until_the_credential_changes`、`cooldown_and_single_probe_are_shared_across_runs`、`production_services_default_to_the_same_runtime` |

之前 AC-007/012/018 的“通过”只覆盖部分时序，本轮补充了真实 loopback 在途换代与取消用例。AC-013 本轮已验证三运行并发：只有两个请求到达本机模拟服务器。原场景测试中的假句柄仍可验证有界采纳策略，不代表真实质量门达标。

**2026-10-21 追加（RF-006 / R007）**：按用户明确要求，`ASSIST_AVAILABLE` 改为 `true`，设置页“采纳”可选，保存后的 Assist 会实际应用高概率建议；未评估提示不再展示。质量门仍未通过，真实模型效果仍未验证。证据：`assist_is_selectable_and_the_default_stays_off`、`saved_assist_applies_a_high_probability_response`，以及 Playwright 用例“决策辅助设置小节…”中的采纳可选用例。

### 8.2 本轮命令和结果

全部使用本机已安装工具与离线 Cargo 依赖，没有新增依赖或调用真实端点。

```text
cargo fmt --all -- --check
exit 0

cargo check --offline --locked --workspace --all-targets
Finished `dev` profile [unoptimized + debuginfo] target(s) in 37.40s

cargo test --offline --locked -p kodo-agent -p kodo-shell --lib
kodo-agent: 652 passed; 0 failed
kodo-shell: test result: ok. 103 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.29s

cargo test --offline --locked -p kodo-desktop --bin kodo-desktop
test result: ok. 66 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s

cargo test --offline --locked -p kodo-agent --test decision_scenarios
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.82s

cargo test --offline --locked -p kodo-shell --test decision_transport
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.49s

/opt/homebrew/bin/node node_modules/typescript/lib/tsc.js --noEmit
exit 0

/opt/homebrew/bin/node node_modules/vite/bin/vite.js build
✓ built in 1.00s

/opt/homebrew/bin/node node_modules/@playwright/test/cli.js test --config <仓库外的本轮配置>
3 passed (3.8s)

git diff --check
exit 0
```

前端命令在 `apps/desktop` 运行，显式使用已安装的 `/opt/homebrew/bin/node`，避免 npm 子进程误用 Node 18。浏览器配置在仓库外 `jev-fix-2026-10-21/playwright.config.cjs`，使用独占 `127.0.0.1:1439`，不复用其他任务的服务、不更新截图基线。Rust 不重复计数的本轮测试合计 852 项，另有 3 项浏览器用例；36/31 项定向决策单元测试是上述 lib 套件的子集。

### 8.3 Git 现场、范围与恢复

修改文件：`crates/agent/src/decision.rs`、`crates/shell/src/decision.rs`、对应两个 Rust 集成测试、`apps/desktop/src/{decisionApi.ts,i18n.ts,pages/DecisionSettings.tsx}`、`apps/desktop/tests/visual/decision.spec.ts` 和本功能四份规格文件，共 12 个文件。

```text
git worktree list --porcelain
worktree /Users/lijianxue/work/WorkSpace/kodo
HEAD a7133b220cefce56c3da760c3096f3c5f9b76507
branch refs/heads/fix/jev-decision-review

git branch --no-merged main
（尚无独有提交，无输出）
```

HEAD、本地 main、缓存 origin/main 均为 `a7133b220cefce56c3da760c3096f3c5f9b76507`。未 fetch，远程新鲜度未核验；本轮没有新提交、远程操作或 PR。未 pop/drop/clear stash，未创建 worktree、删除分支或清理其他现场。

提交前恢复位置为 `fix/jev-decision-review` 的上述 12 个文件。用户已授权随后的本地提交、main 合并和任务分支清理，仍未授权远程操作；最终恢复入口为 main 的本轮合并提交。真实模型效果评测尚未完成。本轮未验证真实模型效果、真实端点连通性、取消后的底层网络瞬时中断、完整视觉回归和安装包；已有大 chunk 构建警告保留。质量门保持未评估，没有以模拟测试冒充收益证明。
