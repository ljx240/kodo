# Work 可靠性实施记录（T001–T030）

分支：`fix/work-reliability`（会话工作树 `/Users/lijianxue/WorkBuddy/Worktrees/kodo/main-df55d246`）
基线：`c0f67a310cb7dea1761f14e017381407a0e07a81`（创建分支时的本地 main；`origin/main` 同值）
本轮**未**提交、未合并、未变基、未推送、未建 PR、未动远端。

状态口径：待开始 / 实施中 / 已实现待验证 / 已验证 / 阻塞。

## 一、M1 读写安全与交付契约

| 任务 | 状态 | 主要改动 | 验收 | 证据 |
| --- | --- | --- | --- | --- |
| T001 读取回执结构 | 已验证 | `crates/agent/src/read_receipt.rs`（新增）：`ReadReceipt`（表示形式/版本/区间/总量/截断/完整）、`ReadLedger`、`version_of`（FNV-1a64+字节长度） | AC-001 AC-002 | 8 个单测；`cargo test -p kodo-agent --lib read_receipt` 全绿 |
| T002 部分读取不得解锁整篇覆盖 | 已验证 | `dedup.rs` 用 `ReadLedger` 取代 `BTreeSet` 布尔；`lib.rs` `write_step` 改用 `overwrite_block` | AC-001 | `partial_read_cannot_unlock_whole_file_overwrite`（201 行文档） |
| T003 分段读取可累积为完整 | 已验证 | `tools::read_outcome` 把「窗口结束」与「提取截断」分开 | AC-002 | `contiguous_range_read_completes_the_overwrite_gate`（1-120 + 121-201） |
| T004 版本冲突拒绝 | 已验证 | `tools::file_version` + `write_project_file_checked` 双重前置校验；`stale_source:` 前缀；`stale_edit_block` 拦 apply_patch/replace_range | AC-003 | `external_edit_after_read_blocks_write_and_patch`、`version_precondition_refuses_a_stale_overwrite` |
| T005 失败写入保留原文件 | 已验证 | 同级临时文件 + rename；失败清理临时文件 | — | `failed_write_keeps_the_original_and_cleans_its_temp_file` |
| T006 输出契约解析 | 已验证 | `plan::parse_output_contract`：引号路径（含空格/中文）、动词后置、源/目标区分、问句识别、多目标 | AC-005 AC-006 | 6 个单测（`quoted_target_*`、`verb_after_*`、`source_and_target_*`、`plain_questions_*`、`multiple_targets_*`、`unnamed_file_request_*`） |
| T007 交付绑定本轮写入 | 已验证 | `validate_deliverables` 要求本轮写入证据（`Reuse` 除外）+ 格式回读 | AC-007 | `existing_old_file_is_not_this_turn_delivery`、`stale_file_is_not_a_delivered_update_end_to_end`（假 Provider 端到端） |
| T008 机器校验与人工评审分离 | 已验证 | `AcceptanceReport.pending_review`；答复 checks 输出 `需人工核对：…` | AC-008 | `deliverable_bar_claim_is_the_delivery_for_free_form_criteria` |

## 二、M2 调度执行可靠性

| 任务 | 状态 | 主要改动 | 验收 | 证据 |
| --- | --- | --- | --- | --- |
| T009 会话 ID 真实化 | 已验证 | `prepare_session` 使用 `session::open` 返回的 id；拆出 `prepare_session_in(dir,…)` 供测试 | AC-009 | `a_scheduled_run_uses_the_session_id_the_core_minted` |
| T010 run↔session 映射 | 已验证 | `SchedulerServiceInner.run_sessions`；取消/结果链接同一 id | AC-009 | 同上 + 资源锁测试断言 `run.session_id` |
| T011 结果取自真实终态 | 已验证 | `run::TurnTerminal` 可信终态通道；`terminal_outcome` 映射 | AC-010 | `a_ready_delivery_is_the_only_success`（9 组）、`a_deadline_outranks_a_late_ready_answer` |
| T012 `start` 返回 ≠ 成功 | 已验证 | 只有 `delivery == "ready"` 记为 Succeeded | AC-010 | 同上 |
| T013 快速任务不误判超时 | 已验证 | `run_with_timeout` 用 `recv_timeout`；协作式取消 + 宽限期参数化 | AC-011 | `a_fast_job_is_not_relabelled_timed_out`、`a_fast_run_does_not_wait_for_its_deadline`、`a_cooperative_job_stops_at_the_deadline`、`an_uncooperative_job_is_reported_as_unconfirmed` |
| T014 模式与 Persona 保留 | 已验证 | `TaskKind::Agent.mode`（`serde(default)` 兼容旧任务）；`map_mode`；`resolve_personas_in` 走同一注册表 | AC-012 | `scheduled_mode_survives_and_legacy_tasks_stay_on_code`、`scheduled_personas_resolve_through_the_same_registry` |
| T015 默认权限不升级 | 已验证 | `AGENT_RUN_PERMISSION = Permission::Ask` | AC-013 | `scheduled_runs_keep_the_ask_permission` |
| T016 异常路径 | 已验证 | 终态写失败保留映射并 stderr；panic→`job_panicked`；重启 `recover_inflight`；陈旧 generation 的 release 不驱逐新 run | AC-014 | `a_panicking_job_is_reported_as_failed`、`a_run_left_running_by_a_crash_is_recovered_as_interrupted`、`releasing_a_stale_generation_leaves_the_newer_run`、`a_missing_terminal_is_interrupted_never_success`、`the_resource_lock_is_held_until_the_terminal_row_is_written` |

## 三、M3 附件可读性与格式边界

| 任务 | 状态 | 主要改动 | 验收 | 证据 |
| --- | --- | --- | --- | --- |
| T017 统一可读性状态 | 已验证（宿主 UI 未接线） | `crates/agent/src/attachment.rs`：`Readability`（complete/partial/clipped/unsupported/unreadable）、`describe`、`view_of`；`tools::read_outcome` 的尾注改为同一条句子 | AC-015 | `the_view_and_the_tool_result_use_the_same_sentence`、`a_window_is_partial_and_says_it_can_continue` 等 6 个 |
| T018 同版本分段续读 | 已验证（文本/DOCX/表格同一实现） | `read_outcome` 对三种表示统一按行窗口；版本来自源字节摘要 | AC-015 | `a_clipped_extraction_cannot_be_continued`、`accumulated_windows_project_as_complete` |
| T019 无项目会话受限句柄 | 部分完成 | `AttachmentScope`（roots 规范化、TTL、版本、目录枢轴拒绝）已实现并有测试；**未接入 `pinned_context` 解析路径** | AC-016 | `a_grant_outside_the_roots_is_refused`、`a_continuation_needs_a_live_handle_of_the_same_version`、`a_continuation_cannot_pivot_to_a_sibling_directory` |
| T020 续读拒绝三类原因 | 已验证（模块层） | `ContinuationError::{Unknown,Expired,VersionChanged,OutOfScope}` + 各自中文说明 | AC-016 | `every_refusal_explains_itself` |
| T021 DOCX 生成 vs 保格式编辑 | 已验证 | `plan::output_note`：Create 说明「生成新 DOCX」；Update 说明「整篇重写」；用户要求保格式时给出无法满足的具体理由 | AC-017 | `a_docx_create_is_not_an_edit_and_an_edit_is_a_rewrite`、`a_format_preserving_docx_edit_is_refused_instead_of_silently_reformatted` |
| T022 表格值为准，不声称公式重算 | 已验证 | 表格目标固定附注「写入的是单元格文本值；公式不会被计算或重算」 | AC-018 | `a_spreadsheet_write_never_claims_formula_results`、`notes_reach_the_answer_checks_without_becoming_passed_checks` |
| T023 不支持的格式提前说明 | 已验证 | `attachment::unsupported_format`（pdf/pptx/doc/xls）在计划期进入 `output_notes` 并渲染进 prompt；交付校验直接拒绝 | AC-019 | `an_unsupported_format_is_named_at_plan_time`、`an_unverifiable_output_fails_delivery_even_when_the_file_exists` |

## 四、M4 评测

| 任务 | 状态 | 主要改动 | 证据 |
| --- | --- | --- | --- |
| T024 确定性复现 + 文件完整性断言 | 已验证 | `evals/src/work.rs`：每例跑两遍；断言源文件逐字节不变、既有目标不被截断/删除、无 `.kodo-tmp-` 残留、接受例需含标记且非空 | `cargo run -p kodo-evals` → Work scripted 30/30 |
| T025 dev/holdout 模板 | 部分完成 | 现有 30 例（dev 20 / holdout 10）已按 split 分别报告；计划目标 40 例未扩充 | 报告行 `Work scripted development: 20/20 … holdout: 10/10` |
| T026 成功率/误报完成/稳定性/延迟/成本报告 | 已验证（成本 N/A） | 每 split 输出 success/false_completions/unstable/latency_avg/latency_max，并打印 `work_report_json:` 单行机读汇总 | `{"cases":30,"attempts_per_case":2,"development":{…},"holdout":{…},"unstable_cases":[]}` |
| T027 真实 Provider 评测 | 阻塞（需单独授权） | 未运行任何真实模型；未产生任何真实分数 | `live quality NOT RUN` |

## 五、验证记录（实际执行）

```
cargo fmt --all -- --check                         → 无输出（干净）
cargo check --offline --locked --workspace --all-targets → 通过，无 error/unused warning
cargo test  --offline --locked -p kodo-agent --lib       → 610 passed, 0 failed
cargo test  --offline --locked -p kodo-desktop --bin kodo-desktop → 61 passed, 0 failed
cargo test  --offline --locked -p kodo-core --lib        → 134 passed, 0 failed
cargo test  --offline --locked -p kodo-shell --lib       → 72 passed, 0 failed
cargo test  --offline --locked -p kodo-knowledge --lib   → 12 passed, 0 failed
cargo run   --offline --locked -p kodo-evals             → deterministic 73/73；baselines 200/200；
                                                            Work scripted dev 20/20、holdout 10/10，
                                                            false_completions=0，unstable=0，
                                                            latency_avg≈380ms
npm run typecheck / test:imports / build                 → 未执行（工作树无 node_modules，本轮禁止安装依赖）
```

## 六、遗留问题与风险

1. **T019 宿主接线缺失**：`AttachmentScope` 尚未接入 `pinned_context` 的读取入口。当前无项目会话依赖「工具不运行、只读 pinned 附件」这一既有结构约束，句柄的过期/版本语义尚未在真实读取路径生效。
2. **T017 UI 投影**：统一句子已进入工具结果与计划，但桌面端附件卡片未消费 `ReadabilityView`。
3. **T021 仍是整篇重写**：本轮只做到「如实说明 + 用户要求保格式时拒绝」，没有实现保格式编辑（需要 OOXML 局部替换）。
4. **T025 规模**：30 例 < 计划的 40 例。
5. **T027 未跑**：真实 Provider 评测需单独授权，报告中的 live 指标一律 `NOT RUN`。
6. **T028–T030 未做**：兼容样本、迁移限制说明、回滚演练、原生客户端手动验收均未执行。
7. **既有抖动**：`context::tests::pinned_span_survives_collect` 在全量并行负载下曾失败一次，单跑与重跑均通过；与本轮改动无因果关系，未修改。
8. **分支落后 main**：本地 main 已前进 5 个提交（Jev decision assistant），本分支基于 `c0f67a3`；按约束未变基、未合并。

## 七、合并状态（2026-10-20 追加）

- 第 8 条已处理：分支先 rebase 到当时的 main（`a9304ea`），冲突 3 个文件按语义逐个解决——
  `run.rs` 的 `StartArgs` 同时保留 `decision` 与 `terminal` 两个字段，并补回 `impl TurnTerminal` 的闭合；
  `scheduler.rs` 保留 decision 注入（`app.state()` + 六参数 `run::start`）同时改用 `terminal_tx` 与
  `wait_for_terminal`；`evals/src/work.rs` 采用新 oracle 并补 `decision: None`。
- 提交拆为 6 个逻辑单元：读取回执/可读性、交付契约与格式边界、调度模式字段、桌面生命周期、评测 oracle、实施记录。
- 合并方式：`git merge --no-ff fix/work-reliability`，main 合并提交 `90f5228`；合并树与已验证分支树
  `git diff --stat` 为空，即 main 上的内容与验证过的内容逐字节一致。
- 合并后状态：`main` 领先 `origin/main`（未推送）；`git branch --no-merged main` 为空。
- **合并前既有红项**：在干净 main（`a9304ea`）上复现同样两条失败——
  `fixture_count_at_least_4_decision: only 0 fixtures`、`splits_defined_decision: dev=0 holdout=0`，
  属 decision 特性的夹具缺口，与本轮改动无关，本轮未修。
- 未推送、未建 PR；任务分支保留在会话工作树中（该工作树即本会话目录，未删除）。
