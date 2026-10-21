# 任务 — 编程任务可靠性修复

每项标注：目标文件 / 依赖 / 对应需求 / 验证。勾选只在改动**和**验证都完成后进行。
证据统一记录在 `acceptance.md` 对应小节；未验证项在 `acceptance.md` §6 如实列出。

## 基础

- [x] T001 复现并留档失败基线（`evals` 默认离线评测 exit 1、`202/204`、两条 FAIL 检查名）。验证：`/tmp/kodo-eval-before.txt`。依赖：无。（`acceptance.md` §0/§1）

## P0 评测套件目录误识别

- [x] T002 `evals/src/main.rs`：新增按磁盘形态判定套件的 `corpus_shape()`（计划名 `classify_corpus_shape()`，落码时缩短），`fixture_suite_names()` 只返回编程夹具套件，分类语料走独立检查。文件：`evals/src/main.rs`。需求：FR-A1–A5。验证：`cargo test -p kodo-evals`、`cargo run -p kodo-evals` exit 0。（`acceptance.md` §1）
- [x] T003 `evals/src/main.rs`：目录误识别回归测试（纯 `.jsonl` 目录 / 含夹具子目录 / 混合 / 空目录 / 案例行不合法 / 单一 split / 真实套件仍可发现 / 夹具门不落在校正语料上）。需求：FR-A6、AC-A2。依赖：T002。验证：`cargo test -p kodo-evals`。（`a_jsonl_only_corpus_is_a_classification_corpus_not_a_fixture_suite` 等 8 个测试）

## P1 任务路由

- [x] T004 `crates/agent/src/classify.rs`：文档短路改为"无代码产出命中时才生效"（FR-B3、FR-B4）。验证：`cargo test -p kodo-agent classify` + 既有测试全绿。（`acceptance.md` §2）
- [x] T005 `crates/agent/src/classify.rs`：新增 `route_label()` 统一输出含 `unknown` 的标签（FR-B1、FR-B2、FR-B5）。验证：单测 AC-B1/B2 + `labels_roundtrip_through_parse`。（`a_feature_that_names_its_output_format_is_not_a_document_task`、`route_label_names_uncertainty_instead_of_borrowing_the_default`）
- [x] T006 锁定显式选择优先级：模式/chip/权限/文件契约不被分类结果覆盖（FR-B6）。验证：`crates/agent` 单测。（`lib.rs` 的 `a_pinned_chip_outranks_the_classification`、`a_requested_file_output_outranks_the_route_label`）
- [x] T007 `evals/src/decision.rs`：评分器改用 `route_label()`，按类别与 tune/holdout 分列并打印分母（FR-B/C、AC-B4、AC-C1）。需求：AC-B4。依赖：T005。（`acceptance.md` §2 分类别表 + 分母）

## P1 失败归因

- [x] T008 `crates/agent/src/verify.rs`：新增分层证据 `FailureTriage` 与 `triage_verify_failure()`（退出码约定 → errno → 环境对象 → 网络瞬断 → 源码诊断 → 无证据），保留 `classify_verify_failure` 兼容（FR-C1–C6）。验证：AC-C3 单测。（`acceptance.md` §3）
- [x] T009 归因口径变更说明与新旧并列输出（FR-C7）。文件：`evals/src/decision.rs` + `acceptance.md`。依赖：T008。（`acceptance.md` §3 口径变更理由 + legacy 并列）
- [x] T010 `FailureTriage` 挂到 `VerificationEvidence`（`#[serde(default)]`），环境类阻断修复、瞬态类不阻断产品修复（FR-C2、FR-C4）。依赖：T008。（`verify.rs:269` 字段 + `triage_environment_faults_block_code_repair`、`triage_transient_faults_are_retried_not_repaired`、`evidence_written_before_the_triage_field_still_loads`）

## P1 上下文容量保护

- [x] T011 `crates/agent/src/state.rs`：估算 vs 实测两个来源、输出预留、剩余空间（FR-D1、FR-D2）。验证：`state.rs` 单测。（`estimate_input_tokens_charges_chars_plus_framing`、`estimate_leads_and_measurement_is_the_fallback`、`output_reserve_shrinks_the_usable_window`、`context_summary_labels_an_estimate_and_a_measurement_differently`；`acceptance.md` §4）
- [x] T012 `crates/agent/src/compaction.rs`：`should_compact` 无 usage 时用估算（FR-D2）。验证：`should_compact_triggers_from_an_estimate_without_any_usage`、`should_compact_follows_the_history_not_a_stale_measurement`。**与计划的偏差**：原写"替换 `skips_when_window_unknown` 的语义"，实际保留该测试（窗口本身未知时无从规划，仍应跳过），新增覆盖的是"窗口已知但 usage 从未到达"这一真实缺陷。
- [x] T013 `crates/agent/src/{lib.rs,compaction.rs}`：发送前 `fit_history_to_window()`，保留系统块/任务消息/约束/最近工具配对（FR-D3）。验证：AC-D1 配对完整性单测。（`fit_keeps_system_block_task_constraints_and_tool_pairs`、`fit_never_separates_a_call_from_the_result_it_keeps`）
- [x] T014 `crates/agent/src/lib.rs`：`ContextTooLong` 有界恢复（上限 2 次）（FR-D4、AC-D2）。依赖：T013。（`MAX_CONTEXT_RECOVERY_ATTEMPTS` + `context_too_long_recovery_is_bounded_and_reported`、`context_too_long_recovery_retries_with_a_smaller_history`）
- [x] T015 `crates/agent/src/lib.rs`：压缩失败可见 + 回退到有标注的截断，`anchors_ok` 不再丢弃（FR-D5）。（`fit_reports_still_over_without_touching_the_history`、`verify_anchors_*` 三个测试 + 摘要中的 `[context budget: dropped N …]` 标注）

## P1 长任务连续性

- [x] T016 `crates/agent/src/plan.rs`：从用户消息提取显式约束进入 `constraints`，带来源与上限（FR-E2、AC-E1）。（`extract_user_constraints()` + `MAX_USER_CONSTRAINTS` / `MAX_USER_CONSTRAINT_CHARS`；测试 `explicit_user_constraints_ride_the_task_checkpoint`、`descriptive_requests_yield_no_constraints`、`user_constraints_are_capped_and_shortened`、`a_model_plan_cannot_drop_the_users_rule`）
- [x] T017 `crates/agent/src/recovery.rs`：确认旧格式检查点在新增字段下仍可加载（FR-E6）。验证：serde 往返单测。（`a_checkpoint_from_before_the_new_fields_still_loads`）
- [x] T018 轮内文件写入使既有验证通过结论标记需重验（FR-E4）。依赖：T016。**实际落点**：`crates/agent/src/state.rs`（`record_verify()` + `absorb()` 置 `verify_stale`）与 `crates/agent/src/plan.rs`（`VerificationEvidence.verify_stale`），`lib.rs` 只负责把该标记写进 `## Task checkpoint`；原计划只写 `lib.rs`，按状态归属修正。验证：`a_write_after_a_passing_run_must_be_verified_again`、`an_early_user_constraint_survives_the_context_trim`。

## 收敛

- [x] T019 全量验证：`cargo test -p kodo-agent`、`cargo test -p kodo-evals`、`cargo run -p kodo-evals`、`cargo fmt --all -- --check`、`git diff --check`；桌面侧仅当接口有变化时跑 `npm run typecheck`。（`acceptance.md` §6 命令与退出码；接口未变，桌面侧只跑 `cargo check -p kodo-desktop`）
- [x] T020 按验收场景复核实际行为，把证据写入 `acceptance.md`；如实标注未验证项。（`acceptance.md` §0–§7）
