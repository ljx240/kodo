# 验收 — 编程任务可靠性修复

本轮只做本地/离线验证。所有命令均在 `/Users/lijianxue/work/WorkSpace/kodo` 以 `--offline` 执行。
证据文件：`/tmp/kodo-eval-before.txt`（改动前基线）、`/tmp/kodo-eval-final.txt`（改动后）。

## 0. 基线与现场漂移

- 规格书写的基线：本地 `main` = `origin/main` = `a7133b2`，任务分支 `fix/code-task-reliability`。
- 会话中途该现场发生变化（非本任务操作）：`fix/jev-decision-review` 已合入 `main`，`main` 连续推进到 `9fd1538`（`59afb97`、`9fd1538` 属 `jev-decision` 任务），工作树切回 `main`。本任务的未提交改动因此落在 `main` 工作树上，`fix/code-task-reliability`（`e5b8fa6`）无独有提交、已被 `main` 包含。
- 归属核对：那两个新提交只包含 `jev-decision` 的 8 个文件，未把本任务在制品扫入；本任务改动的 10 个文件（`crates/agent/src/{classify,compaction,lib,plan,provider,recovery,state,verify}.rs`、`evals/src/{decision,main}.rs`）+ `specs/code-task-reliability/` 仍未提交。
- 现场复核（交付时）：提交/合并前执行 `git fetch origin --prune`，`origin/main` = `9fd1538`、`HEAD..origin/main` 为空，确认共享基线未前进；合并后 `main` = `c8d4536`，`git branch --no-merged main` 为空，`git worktree list` 只有主工作目录，`git stash list` 仍为 7 条未变、未 pop 未 drop。
- 冻结语料未动：`git diff --stat` 与 `git status` 中没有任何 `evals/fixtures*/` 文件；`crates/agent/src/decision.rs`、`crates/shell/src/decision.rs`、`specs/jev-decision/` 亦未被本任务修改。

## 1. P0 评测套件目录误识别（FR-A1–A6 / AC-A1–A2）

根因：`evals/src/main.rs` 的 `suite_names()` 按 `fixtures-` 前缀枚举套件，把只含 `.jsonl` 的分类语料 `fixtures-decision/` 当作编程夹具套件，对其施加 `fixture_count_at_least_4_*` 与 `splits_defined_*` 两道编程门。

改动前（`/tmp/kodo-eval-before.txt`）：

```text
  FAIL  fixture_count_at_least_4_decision: only 0 fixtures
  FAIL  splits_defined_decision: dev=0 holdout=0
G_fixture_baselines: FAIL (202/204)
```

进程 exit 1。

改动后：按磁盘形态分类（含夹具子目录 → 编程夹具套件；只含 `.jsonl` → 分类语料），分类语料走自己的 4 道检查（可发现、案例行可解析、`id/label/split` 齐备、tune 与 holdout 都非空），编程门不再施加于它；`decision` 分类评测仍由 `decision::run()` 独立执行。

```text
EVAL_EXIT=0
  suite default — fixture suite, 30 fixture(s) — 168 check(s), 0 fail(s)
  suite decision — classification corpus, 2 case file(s) — 4 check(s), 0 fail(s)
  suite real — fixture suite, 4 fixture(s) — 32 check(s), 0 fail(s)
G_deterministic_100: PASS (73/73)
G_fixture_baselines: PASS (204/204)
```

阈值未降低（`default` 仍 30、`real` 仍 4），语料未删、未跳过、未吞错：`a_missing_root_is_unusable_rather_than_skipped`、`an_empty_corpus_is_unusable_rather_than_skipped`、`a_malformed_case_line_or_missing_field_fails_the_corpus_gate`、`a_dropped_corpus_line_is_counted`、`a_single_split_corpus_fails_the_split_gate` 把这些失败形态固定为测试。

AC-A2 目录形态回归测试（`cargo test --offline -p kodo-evals` → 21 passed）：
`a_jsonl_only_corpus_is_a_classification_corpus_not_a_fixture_suite`、`a_directory_of_fixture_repos_is_still_a_fixture_suite`、`mixed_shape_is_gated_as_a_fixture_suite_first`、`an_empty_corpus_is_unusable_rather_than_skipped`、`a_missing_root_is_unusable_rather_than_skipped`、`fixture_gates_are_never_applied_to_a_classification_corpus`、`the_real_programming_suites_are_still_discovered_as_suites`。

## 2. P1 任务路由（FR-B1–B7 / AC-B1–B4）

根因是口径不一致，而非规则单调：运行时 `TaskType` 无 `unknown`、评分器取 `baseline.label()`，结构上永远猜不中语料里的 20 条 `unknown`；`is_document_output()` 又是短路，"新增导出为 Markdown 的入口"被格式词判成 `docs`。

改动：新增单一事实来源 `ClassificationEvidence::route_label()` 与 `ROUTE_LABELS`（含 `unknown`），`unknown` 只在"零关键词命中且非文档产出"时出现；文档短路改为只在代码产出类别无命中时生效；评分器改用 `route_label()`，按类别、tune/holdout 分列并打印分母。`classify()` 的兼容默认值（`feature`）保持不变，chip/模式/权限/文件契约的判定顺序未被改动。

行为锁定（`cargo test --offline -p kodo-agent --lib -- classify::` → 16 passed）：
- AC-B1 `a_feature_that_names_its_output_format_is_not_a_document_task`（"新增导出为 Markdown 的入口" → `feature`，"保存到 docs/架构.md" 仍 `docs`）
- AC-B2 `route_label_names_uncertainty_instead_of_borrowing_the_default`（"把那个东西弄好" / "do the usual thing" → `unknown` + `ambiguous=true`，而 `classify()` 仍返回 `Feature`）
- AC-B3 `labels_roundtrip_through_parse`、`the_compatibility_entry_point_agrees_with_the_evidence` 及全部原分类单测通过
- FR-B6 `tests::a_pinned_chip_outranks_the_classification`（同一句知识问话，钉住 `【技能：code-review】` 后走任务通道、计划里带上该技能规则）、`tests::a_requested_file_output_outranks_the_route_label`
- 语料侧：`the_route_vocabulary_covers_every_label_the_rule_can_name`、`a_request_the_rules_cannot_place_is_named_unknown`、`a_label_outside_the_vocabulary_is_reported_not_scored`（`work` 是模式标签，不计入分母）、`the_frozen_route_corpus_can_reach_unknown`

冻结语料指标（120 条，分母如实给出；新旧口径并列）：

| 指标 | 新口径 | 旧口径 |
| --- | --- | --- |
| 总体 accuracy | 0.692 (83/120) | 0.617 (74/120) |
| macro F1 | 0.709 | 0.611 |
| tune | 0.793 (46/58) | 0.724 (42/58) |
| holdout | 0.597 (37/62) | 0.516 (32/62) |
| bug-fix | 18/21 (tp18 fp1 fn3) | 18/21 |
| feature | 11/21 (tp11 fp2 fn10) | 16/21 (tp16 fp34 fn5) |
| test | 13/14 | 13/14 |
| refactor | 9/17 | 9/17 |
| code-review | 9/14 | 9/14 |
| docs | 9/13 | 9/13 |
| unknown | 14/20 (tp14 fp23 fn6) | 0/20 (tp0，结构性不可能) |

不可比说明：旧口径的 `unknown` 永远为 0，因此两行的差值主要来自这一类别被允许出现，不能读成"规则整体变强 7.5 个百分点"。`ambiguous` 计数在新旧口径下同为 112/120（93.3%），说明 `AMBIGUITY_MARGIN = 4` 下绝大多数样本仍被标为不确定——本轮未调该阈值，因为它直接决定 Jev 咨询调用是否发生，属用户决定项（见第 6 节）。

## 3. P1 失败归因（FR-C1–C8 / AC-C1–C3）

根因同样是口径：`classify_verify_failure()` 只做 24 条环境子串匹配、完全不看退出码，其余一律 `Product`；评分器又手工把 `Timeout`→`transient`、`Cancelled/Denied/Product`→`product`，并把 `command_category`（如 `generic`）当命令行传入，与运行时输入形态不符。

改动：新增分层证据 `FailureTriage{Product, Infrastructure, Transient, Unknown}` 与 `triage_verify_failure()`，优先级为 真实进程状态（取消/超时/拒绝）→ 退出码约定（127/126/137）→ errno/系统码（ENOSPC、EACCES、ECONNRESET 等）→ 环境对象故障 → 网络瞬断形态 → 源码诊断形态 → 无证据则 `Unknown`；`FailureTriage` 以 `#[serde(default)]` 挂到 `VerificationEvidence`（旧记录仍可读，见 `evidence_written_before_the_triage_field_still_loads`）；`classify_verify_failure` 保留兼容。环境类（`Infrastructure`）`blocks_code_repair()`，瞬态类（`Transient`）`is_retryable()` 且不阻断产品修复。未接入模型或外部服务，未修改语料。

口径变更原因（FR-C7）：`Temporary failure in name resolution`、`rate limit`、连接重置一类在旧口径里被环境子串清单收为"环境"，但它们的运维动作是重试而非停止，故改归 `transient`；旧口径结果同时在评测输出里打印（`failure_triage_legacy_*` 行）以便对比。

冻结语料指标（60 条）：

| 指标 | 新口径 | 旧口径 |
| --- | --- | --- |
| accuracy | 1.000 (60/60) | 0.383 (23/60) |
| macro F1 | 1.000 | 0.203 |
| tune | 1.000 (28/28) | 0.429 (12/28) |
| holdout | 1.000 (32/32) | 0.344 (11/32) |
| product | 20/20 (tp20 fp0 fn0) | 20/20 (但 fp34) |
| infrastructure | 16/16 | 3/16 (fn13) |
| transient | 15/15 | 0/15 (结构性 tp0) |
| unknown | 9/9 | 0/9 (类别不存在) |
| ambiguous | 9 (15.0%) | 34 (56.7%) |
| product_blocked | 0 (0/60) | 0 (0/60) |

必须如实说明：这 60 条判例在编写规则时是**可见**的，1.000 是"规则能解释自己所属语料"的拟合值，不是真实世界的归因成功率；可比的是 `transient`/`unknown` 的 tp 不再结构性为 0、以及 `product_blocked` 保持 0。真正的泛化检验需要 `model_comparison`（未运行）或新语料。

AC-C3 单测（`cargo test --offline -p kodo-agent --lib -- verify::tests::triage`）：`triage_reads_process_state_before_any_text`（超时/取消不降级为文本判断）、`triage_environment_faults_block_code_repair`（127/ENOSPC/`Cannot find module`/`missing script`/`could not find Cargo.toml`/EACCES）、`triage_transient_faults_are_retried_not_repaired`（ECONNRESET/name resolution/rate limit）、`triage_product_failures_stay_on_the_repair_path`（`test result: FAILED`、断言失败）、`triage_leaves_an_unattributable_failure_unknown`（非零退出且输出无诊断内容）。

## 4. P1 发送前上下文容量保护（FR-D1–D6 / AC-D1–D2）

根因：压缩触发只认**实测** usage（`last_input_tokens == 0` 直接不压缩），不返回 usage 的服务商与首轮请求整轮不压缩；发送前无收缩；`ContextTooLong` 不重试；`compact_history` 返回 `None` 时静默跳过、`anchors_ok` 被丢弃。

改动：`Budget` 区分估算与实测两个来源（`ContextSource{Measured, Estimated, Unknown}`），估算先行、实测兜底，UI/预算摘要标注"（估算，服务商未回报 usage）"；新增输出预留与 `usable_window()`；发送前 `fit_history_to_window()` 确定性裁剪（保留系统规则块、当前任务消息与约束、必要验收证据、最近工具调用/结果配对，并插入"[context budget: dropped N …]"标注）；压缩失败改为可见上报 + 回退到该裁剪；`ContextTooLong` 有界恢复上限 2 次。决策记录 D-D1：字符×(1/4) 估算，不引入 tokenizer；D-D2：发送前裁剪 + 有界重试。Provider 请求体与工具协议未变。

AC-D1/AC-D2 单测：
- 首轮大输入 + 无 usage：`tests::oversized_first_request_is_reduced_before_sending_without_any_usage`（窗口按实测体积反推；请求确实变小、摘要标记存在、尾部历史/当前请求/系统规则存活、脚本未被多消费一次、交付 `ready`）
- 压缩失败回退：`tests::compaction_failure_falls_back_to_the_deterministic_pre_send_trim`（上报含"发送前裁剪"与"估算"，历史里出现丢弃标注）
- 估算先行：`state::tests::estimate_leads_and_measurement_is_the_fallback`、`planned_context_sizes_the_first_request_without_usage`、`context_summary_labels_an_estimate_and_a_measurement_differently`、`overfits_window_only_when_input_plus_reserve_exceeds_the_window`
- 输出预留：`compaction::tests::output_reserve_trims_a_request_the_bare_window_would_accept`、`state::tests::output_reserve_shrinks_the_usable_window`、`output_reserve_stays_inside_the_window_and_above_the_floor`
- 配对完整：`compaction::tests::fit_never_separates_a_call_from_the_result_it_keeps`、`fit_keeps_system_block_task_constraints_and_tool_pairs`、`fit_charges_an_image_by_allowance_not_by_payload`、`fit_reports_still_over_without_touching_the_history`、`fit_drops_nothing_when_the_request_already_fits`
- 触发口径：`should_compact_triggers_from_an_estimate_without_any_usage`、`should_compact_follows_the_history_not_a_stale_measurement`。计划里写的"替换 `skips_when_window_unknown` 语义"没有照做：窗口本身未知时仍无从规划，该测试保留原义，真正修的是"窗口已知、usage 从未到达"。
- 有界恢复：`tests::context_too_long_recovery_retries_with_a_smaller_history`（重试确实带走更少的公开历史）、`tests::context_too_long_recovery_is_bounded_and_reported`（调用次数恰为 `MAX_CONTEXT_RECOVERY_ATTEMPTS + 1`，超限显式失败并携带服务商原因）
- 范围补充：`crates/agent/src/provider.rs` 也改了 14 行，规格 §2 的文件清单未列出它。改动只在 `fake_text()`（离线脚本分支）：识别 `{"error": "...", "status": N}` 并走与网络路径同一个 `classify_failure()`，否则有界恢复测试无法驱动真实的 `ContextTooLong` 类别。网络请求体、序列化格式与工具协议未动，FR-D6 仍成立。反向地，清单里的 `context.rs` 最终未改——发送前收缩全部落在 `compaction::fit_history_to_window()`。

## 5. P1 长任务与跨轮连续性（FR-E1–E6 / AC-E1）

根因：`TaskPlan.constraints` 只来自硬编码列表，用户口头约束从不进入，一旦超出历史窗口（最近 16 条 / 32k 字符）即不可恢复；同一轮内后续写入不会使此前的 `verify_ok` 失效。

改动（全部复用既有机制，无并行系统）：
- `plan.rs` 新增显式约束提取（引语式、带来源前缀"用户原话要求："、上限 6 条、单条 120 字符截断），接入 `conversational/from_task/summarize/from_skill` 与轮内模型计划替换处；`with_user_constraints()` 幂等合并且不丢模型自身约束。约束随 `## Task checkpoint` 系统块每轮重写，因而与可被裁剪的公开历史不同。
- `state.rs`：写入落在一次通过之后时，清除 `verify_ok` 结论并置 `verify_stale`，`after_tools()` 因此把任务送回 Verify；`record_verify()` 使新一轮运行替换而非叠加旧结论。`lib.rs` 的检查点块对模型明说"the last passing run predates these writes; verify again before finishing"。
- 检查点：`AcceptanceEvidence.verify_stale`、`VerificationEvidence.triage` 均 `#[serde(default)]`，旧格式记录仍可加载；`recovery::revalidate` 的既有行为（保留文字契约、清空全部证据、列出变更后文件）未改。
- 历史与摘要继续以 `user` 内容注入，未提升为系统指令；`AGENTS.md` 的作用域与优先级未触碰。

AC-E1 单测：
- 早期约束超出窗口仍在：`plan::tests::explicit_user_constraints_ride_the_task_checkpoint` + `tests::an_early_user_constraint_survives_the_context_trim`（发送前裁剪后，系统块内仍含"用户原话要求：不要改动公开 API"，且 `## Task checkpoint` 块仍在请求里）
- 不猜测、有上限：`plan::tests::descriptive_requests_yield_no_constraints`、`user_constraints_are_capped_and_shortened`
- 模型计划不得丢弃用户规则：`plan::tests::a_model_plan_cannot_drop_the_users_rule`
- 中断恢复 / 恢复前文件被改动：`recovery::tests::checkpoint_preserves_contract_but_invalidates_completed_evidence`（既有，仍通过）
- 旧验证结果失效：`state::tests::a_write_after_a_passing_run_must_be_verified_again`
- 旧格式记录兼容：`recovery::tests::a_checkpoint_from_before_the_new_fields_still_loads`（剥掉 `output_operation/output_sources/output_notes/triage` 四个新字段后仍加载，并按遗留语义读：无操作标记、无归因）

## 6. 验证命令与结果

```text
cargo test --offline -p kodo-agent            # exit 0
  696 passed; 0 failed（lib）+ 9 + 1 + 13 passed，2 ignored（需真实网络的用例）
cargo test --offline -p kodo-evals            # exit 0，21 passed; 0 failed
cargo run --offline -q -p kodo-evals          # EVAL_EXIT=0
  G_deterministic_100: PASS (73/73) · G_fixture_baselines: PASS (204/204)
cargo fmt --all -- --check                    # FMT_EXIT=0
git diff --check                              # exit 0
cargo check --offline -p kodo-desktop         # exit 0（依赖 crate 编译；本任务未改 TS/接口，
                                              #   故未跑 npm typecheck/build/visual）
```

以上每条命令在收敛阶段（代码定稿后）重跑过一次，结果与首次记录一致；合并到 `main` 后又整套重跑一次，仍全部 exit 0。原始输出留在 `/tmp/kodo-agent-test-final.txt`、`/tmp/kodo-evals-test-final.txt`、`/tmp/kodo-eval-final.txt`、`/tmp/kodo-fmt-final.txt`、`/tmp/kodo-diffcheck-final.txt`、`/tmp/kodo-desktop-check-final.txt`，合并后一次在 `/tmp/kodo-postmerge-{agent,evals,eval,fmt}.txt`。

明确未验证（不得当作通过）：
- `live` 评测、`G_holdout_success_ge_90`、`G_live_false_completion_le_5`、`G_no_destructive_false_completion`、`G_known_regression_false_completion_0`：`NOT RUN — --live not requested`（需真实模型授权）。
- `model_comparison: NOT RUN`，`quality_gate: NOT EVALUATED`。
- 桌面 `npm run typecheck / build / test:visual`：本任务未改 `apps/desktop` 与协议，未运行。
- 真实编程成功率提升：本轮只用离线夹具与冻结语料，未以任何分类指标宣称真实成功率变化。

## 7. 遗留与需用户决定

1. `AMBIGUITY_MARGIN = 4` 使 112/120（93.3%）路由样本被判为不确定，而 `evidence.ambiguous` 正是 Jev 咨询调用的前置门。收紧或放宽该门槛会改变是否发起云调用，属用户决定，本轮不动。
2. 归因 1.000 是拟合值（见第 3 节），需要新语料或已授权的模型对比才能给出泛化结论。
3. 检查点仍写入工具输出片段（既有设计），未新增密钥类字段；`FR-E6` 的"不含密钥"依赖 engine-state 的字段集合与既有 0600/符号链接防护，无专门测试断言"内容里没有凭据"。
4. 交付状态（2026-10-21 收尾）：任务分支 `fix/code-task-reliability` 提交 `3078dd5`，以 `--no-ff` 合入 `main` 得 `c8d4536`（父提交 `9fd1538` + `3078dd5`），推送后 `origin/main` = `c8d4536`。合并后在 `main` 上重跑：agent 696+9+1+13 passed（2 ignored）、evals 21 passed、默认评测 exit 0（204/204、73/73、三套件 0 fail）、fmt 与 diff check 通过。`fix/code-task-reliability` 保留未删除。
