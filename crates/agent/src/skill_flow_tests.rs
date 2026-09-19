//! Fake-provider flow tests — two per task_type (12 total).
//!
//! Each scenario runs the real pipeline shape used by `run()`:
//! classify → SkillRegistry → `TaskPlan::from_skill` → filtered decisions →
//! skill tool gate → AgentMachine (scripted model replies) → acceptance.
//! No network, no real LLM: replies are a fixed script, tools are fakes.

use std::collections::VecDeque;

use crate::classify::{classify, TaskType};
use crate::plan::{Evidence, SubtaskKind, SubtaskStatus, TaskPlan};
use crate::protocol::{
    parse_model_turn, ModelTurn, ToolArgs, ToolCall, ToolError, ToolErrorCode, ToolInvocation,
    ToolName, ToolRegistry, ToolResult,
};
use crate::skill::{SkillRegistry, SkillSpec, VerificationPolicy};
use crate::state::{AgentEvent, AgentMachine, AgentState, Budget};

/// One scripted model turn.
enum Reply {
    /// JSON tool calls (validated through the standard registry, then gated).
    Tools(Vec<serde_json::Value>),
    /// Prose claim of completion (acceptance stays structural).
    Claim,
}

struct Outcome {
    task_type: TaskType,
    skill: SkillSpec,
    plan: TaskPlan,
    state: AgentState,
    acceptance_ok: bool,
    failures: Vec<String>,
    /// (tool label, error code) rejected by the skill gate.
    rejections: Vec<(String, ToolErrorCode)>,
    /// Tool labels that passed the gate and "executed".
    executed: Vec<String>,
    verify_entered: bool,
    files_written: Vec<String>,
    terminated: bool,
}

fn fake_execute(call: &ToolCall) -> ToolResult {
    match (&call.name, &call.args) {
        (ToolName::WriteFile, ToolArgs::WriteFile { path, .. })
        | (ToolName::CreateFile, ToolArgs::CreateFile { path, .. }) => {
            ToolResult::success(call.id.clone(), call.name.label(), path.clone(), "wrote")
        }
        (name, args) if name.is_mutation() => {
            ToolResult::success(call.id.clone(), name.label(), args.label(), "patched")
        }
        (ToolName::RunCommand, ToolArgs::RunCommand { command }) => {
            let lower = command.to_ascii_lowercase();
            // Reproduction / failing-test / error probes are scripted to fail
            // with a signature. That is what "reproduced the bug" means.
            let is_repro = lower.contains("repro")
                || lower.contains("failing")
                || lower.contains("npm test -- --bug")
                || lower.contains("cargo test repro")
                || lower.contains("500")
                || lower.contains("panic")
                || lower.contains("bug")
                || lower.contains("error")
                || lower.contains("curl login")
                || (lower.contains("cargo test")
                    && !lower.contains("workspace")
                    && !lower.contains("suite"))
                || (lower.contains("npm test")
                    && !lower.contains("suite")
                    && !lower.contains("--pass"));
            if is_repro {
                ToolResult::failure(
                    call.id.clone(),
                    call.name.label(),
                    command.clone(),
                    ToolError::execution(
                        "running test auth ... assertion failed left=1 right=0\nexit=101",
                    ),
                )
            } else if lower.contains("search") || lower.contains("rg ") {
                ToolResult::success(
                    call.id.clone(),
                    call.name.label(),
                    command.clone(),
                    "2 files match:\nsrc/auth.rs\nsrc/router.rs",
                )
            } else {
                ToolResult::success(call.id.clone(), call.name.label(), command.clone(), "ok")
            }
        }
        (ToolName::Search, ToolArgs::Search { query }) => ToolResult::success(
            call.id.clone(),
            call.name.label(),
            query.clone(),
            "2 files match:\nsrc/auth.rs\nsrc/router.rs",
        ),
        (ToolName::ReadFile, ToolArgs::ReadFile { path }) => ToolResult::success(
            call.id.clone(),
            call.name.label(),
            path.clone(),
            "fn auth() {}\n// relevant code",
        ),
        (name, args) => ToolResult::success(call.id.clone(), name.label(), args.label(), "ok"),
    }
}

/// Run one scripted scenario to a terminal state (or fail the test on cap).
fn run_scenario(message: &str, script: Vec<Reply>) -> Outcome {
    run_scenario_with_verify(message, script, None)
}

fn run_scenario_with_verify(
    message: &str,
    script: Vec<Reply>,
    verify_command: Option<&str>,
) -> Outcome {
    let task_type = classify(message);
    let skill = SkillRegistry::builtin()
        .select(task_type)
        .unwrap_or_else(|| panic!("no skill selected for {task_type}"))
        .clone();

    // Planner hook: skill-shaped plan (mirrors `run()`).
    let plan = TaskPlan::from_skill(&skill, message);
    let registry = ToolRegistry::standard();
    let mut machine = AgentMachine::with_plan(message, plan, Budget::default());
    machine.handle(AgentEvent::TaskReceived);
    machine.handle(AgentEvent::PlanReady);
    machine.handle(AgentEvent::ContextGathered);

    let mut script: VecDeque<Reply> = script.into();
    let mut rejections: Vec<(String, ToolErrorCode)> = Vec::new();
    let mut executed: Vec<String> = Vec::new();
    let mut verify_entered = false;
    let mut terminated = false;

    for _ in 0..40 {
        if machine.state().is_terminal() {
            terminated = true;
            break;
        }
        match machine.state().clone() {
            // Verify gate — same decision point as `run()`, but scripted.
            AgentState::Verify => {
                verify_entered = true;
                if let Some(cmd) = verify_command {
                    machine.plan_mut().verify_commands.push(cmd.to_owned());
                }
                machine.handle(AgentEvent::VerifyFinished { ok: true });
            }
            AgentState::Repair => {
                machine.handle(AgentEvent::RepairApplied);
            }
            AgentState::Plan => {
                machine.handle(AgentEvent::ContextGathered);
            }
            AgentState::GatherContext | AgentState::Execute => {
                let reply = script.pop_front().unwrap_or(Reply::Claim);
                match reply {
                    Reply::Tools(calls) => {
                        let text = serde_json::json!({ "tool_calls": calls }).to_string();
                        let ModelTurn::Tools { calls: invocations } =
                            parse_model_turn(&text, &registry)
                        else {
                            panic!("scripted tools did not parse: {text}");
                        };
                        machine.handle(AgentEvent::ModelRequestedTools {
                            count: invocations.len(),
                        });
                        let mut results = Vec::new();
                        for inv in invocations {
                            match inv {
                                ToolInvocation::Ready(call) => {
                                    // Skill allowlist gate (before Permission — never instead of it).
                                    if let Some(denied) = skill.gate(
                                        call.id.clone(),
                                        call.name.label(),
                                        &call.args.label(),
                                    ) {
                                        let code = denied
                                            .error
                                            .as_ref()
                                            .map(|e| e.code)
                                            .unwrap_or(ToolErrorCode::PermissionDenied);
                                        rejections.push((denied.name.clone(), code));
                                        results.push(denied);
                                        continue;
                                    }
                                    executed.push(call.name.label().to_owned());
                                    results.push(fake_execute(&call));
                                }
                                ToolInvocation::Rejected(reject) => {
                                    let res = ToolResult::from_rejection(&reject);
                                    let code = res
                                        .error
                                        .as_ref()
                                        .map(|e| e.code)
                                        .unwrap_or(ToolErrorCode::UnknownTool);
                                    rejections.push((res.name.clone(), code));
                                    results.push(res);
                                }
                            }
                        }
                        machine.handle(AgentEvent::ToolsFinished { results });
                    }
                    Reply::Claim => {
                        machine.handle(AgentEvent::ModelClaimedDone);
                    }
                }
            }
            other => panic!("unexpected state in scenario: {other:?}"),
        }
    }

    let report = machine.last_acceptance().cloned();
    Outcome {
        task_type,
        skill,
        plan: machine.plan().clone(),
        state: machine.state().clone(),
        acceptance_ok: report.as_ref().map(|r| r.ok).unwrap_or(false),
        failures: report.map(|r| r.failures).unwrap_or_default(),
        rejections,
        executed,
        verify_entered,
        files_written: machine.evidence().files_written.clone(),
        terminated,
    }
}

fn tools(calls: serde_json::Value) -> Reply {
    Reply::Tools(calls.as_array().expect("array of calls").clone())
}

fn read_call(id: &str, path: &str) -> serde_json::Value {
    serde_json::json!({"id": id, "name": "read_file", "arguments": {"path": path}})
}

fn write_call(id: &str, path: &str) -> serde_json::Value {
    serde_json::json!({"id": id, "name": "write_file", "arguments": {"path": path, "content": "x\n"}})
}

fn command_call(id: &str, command: &str) -> serde_json::Value {
    serde_json::json!({"id": id, "name": "run_command", "arguments": {"command": command}})
}

fn patch_call(id: &str, path: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "name": "apply_patch",
        "arguments": {"path": path, "old": "a", "new": "b"}
    })
}

fn assert_done_evidence(plan: &TaskPlan) {
    for s in &plan.subtasks {
        if s.status == SubtaskStatus::Done {
            assert!(
                !s.evidence.is_empty(),
                "subtask {} is done without evidence",
                s.id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// bug-fix
// ---------------------------------------------------------------------------

#[test]
fn bug_fix_fake_provider_1_classifies_and_reproduces_first() {
    // Scripted model gathers context and claims done without ever fixing.
    let out = run_scenario(
        "Fix the panic when opening an empty project",
        vec![
            tools(serde_json::json!([
                command_call("c1", "cargo test repro_panic"),
                read_call("r1", "src/open.rs"),
            ])),
            Reply::Claim,
        ],
    );
    assert_eq!(out.task_type, TaskType::BugFix);
    assert_eq!(out.skill.name, "bug-fix");
    assert_eq!(out.skill.verification_policy, VerificationPolicy::Full);
    assert!(
        out.plan.requires_verify,
        "bug-fix must require verification"
    );
    // Workflow: reproduce (command) before anything else.
    assert_eq!(out.plan.subtasks[0].kind, SubtaskKind::Command);
    assert!(
        out.plan.subtasks[0]
            .title
            .to_lowercase()
            .contains("reproduce"),
        "first step should reproduce: {}",
        out.plan.subtasks[0].title
    );
    // Claim without fix/verification must never Finish.
    assert_ne!(
        out.state,
        AgentState::Finish,
        "claim without fix must not finish"
    );
    assert!(out.terminated, "scenario must reach a terminal state");
    assert!(
        !out.acceptance_ok,
        "acceptance must reject the bare claim: {:?}",
        out.failures
    );
    assert_done_evidence(&out.plan);
}

#[test]
fn bug_fix_fake_provider_2_repro_fix_verify_finishes_with_evidence() {
    let out = run_scenario_with_verify(
        "修复登录接口 500 报错",
        vec![
            tools(serde_json::json!([command_call("c1", "curl login → 500")])),
            tools(serde_json::json!([read_call("r1", "src/login.rs")])),
            tools(serde_json::json!([write_call("w1", "src/login.rs")])),
        ],
        Some("cargo test --workspace --quiet"),
    );
    assert_eq!(out.task_type, TaskType::BugFix);
    assert_eq!(out.skill.name, "bug-fix");
    assert_eq!(out.state, AgentState::Finish, "failures={:?}", out.failures);
    assert!(out.acceptance_ok, "failures={:?}", out.failures);
    assert!(
        out.verify_entered,
        "bug-fix must pass through the Verify gate"
    );
    assert!(out.plan.subtasks.iter().all(|s| s.is_done()));
    assert_done_evidence(&out.plan);
    // Evidence kinds: changed file for the fix, passed verification for the gate.
    let evidence: Vec<&Evidence> = out
        .plan
        .subtasks
        .iter()
        .flat_map(|s| s.evidence.iter())
        .collect();
    assert!(evidence
        .iter()
        .any(|e| matches!(e, Evidence::ChangedFile { path } if path == "src/login.rs")));
    assert!(evidence.iter().any(|e| matches!(
        e,
        Evidence::PassedVerification { command } if command.contains("cargo test")
    )));
    assert!(out.files_written.iter().any(|p| p == "src/login.rs"));
}

// ---------------------------------------------------------------------------
// feature
// ---------------------------------------------------------------------------

#[test]
fn feature_fake_provider_1_acceptance_criteria_come_first() {
    let out = run_scenario(
        "Add a dark mode toggle to settings",
        vec![
            tools(serde_json::json!([
                read_call("r1", "src/settings.rs"),
                read_call("r2", "src/theme.rs"),
            ])),
            Reply::Claim,
        ],
    );
    assert_eq!(out.task_type, TaskType::Feature);
    assert_eq!(out.skill.name, "feature");
    assert_eq!(out.skill.context_strategy.label(), "acceptance-first");
    assert!(
        out.plan.subtasks[0]
            .title
            .to_lowercase()
            .contains("acceptance criteria"),
        "first step must identify acceptance criteria: {}",
        out.plan.subtasks[0].title
    );
    assert!(out.plan.requires_verify);
    assert_eq!(out.plan.acceptance_criteria, out.skill.completion_criteria);
    // Reads alone cannot satisfy a feature claim.
    assert_ne!(out.state, AgentState::Finish);
    assert!(out.terminated);
    assert!(!out.acceptance_ok, "failures={:?}", out.failures);
    assert_done_evidence(&out.plan);
}

#[test]
fn feature_fake_provider_2_implements_and_verifies_to_finish() {
    let out = run_scenario_with_verify(
        "实现导出 CSV 的功能",
        vec![
            tools(serde_json::json!([
                read_call("r1", "src/export.rs"),
                read_call("r2", "src/main.rs"),
            ])),
            tools(serde_json::json!([write_call("w1", "src/export.rs")])),
        ],
        Some("cargo test --workspace --quiet"),
    );
    assert_eq!(out.task_type, TaskType::Feature);
    assert_eq!(out.state, AgentState::Finish, "failures={:?}", out.failures);
    assert!(out.acceptance_ok);
    assert!(out.verify_entered);
    assert!(out.files_written.contains(&"src/export.rs".to_owned()));
    assert_done_evidence(&out.plan);
    let evidence: Vec<&Evidence> = out
        .plan
        .subtasks
        .iter()
        .flat_map(|s| s.evidence.iter())
        .collect();
    assert!(evidence
        .iter()
        .any(|e| matches!(e, Evidence::ChangedFile { .. })));
    assert!(evidence
        .iter()
        .any(|e| matches!(e, Evidence::PassedVerification { .. })));
}

// ---------------------------------------------------------------------------
// test
// ---------------------------------------------------------------------------

#[test]
fn test_fake_provider_1_classifies_with_tests_only_policy() {
    let out = run_scenario(
        "Add unit tests for the task classifier",
        vec![
            tools(serde_json::json!([read_call("r1", "src/classify.rs")])),
            Reply::Claim,
        ],
    );
    assert_eq!(out.task_type, TaskType::Test);
    assert_eq!(out.skill.name, "test");
    assert_eq!(out.skill.verification_policy, VerificationPolicy::TestsOnly);
    assert!(out.skill.verification_policy.needs_run());
    assert!(out.plan.requires_verify);
    assert_eq!(
        out.plan.subtasks.iter().map(|s| s.kind).collect::<Vec<_>>(),
        vec![SubtaskKind::Read, SubtaskKind::Edit, SubtaskKind::Verify],
        "test plan: locate → write tests → run suite"
    );
    // Claiming tests pass without writing/running them must fail.
    assert_ne!(out.state, AgentState::Finish);
    assert!(out.terminated);
    assert!(!out.acceptance_ok);
    assert_done_evidence(&out.plan);
}

#[test]
fn test_fake_provider_2_adds_tests_and_suite_passes() {
    let out = run_scenario_with_verify(
        "给 plan 模块补测试用例",
        vec![
            tools(serde_json::json!([read_call("r1", "src/plan.rs")])),
            tools(serde_json::json!([write_call("w1", "tests/plan_spec.rs")])),
        ],
        Some("cargo test plan --quiet"),
    );
    assert_eq!(out.task_type, TaskType::Test);
    assert_eq!(out.state, AgentState::Finish, "failures={:?}", out.failures);
    assert!(out.acceptance_ok);
    assert!(
        out.verify_entered,
        "test skill runs the suite via the Verify gate"
    );
    assert_done_evidence(&out.plan);
    let evidence: Vec<&Evidence> = out
        .plan
        .subtasks
        .iter()
        .flat_map(|s| s.evidence.iter())
        .collect();
    assert!(evidence.iter().any(|e| matches!(
        e,
        Evidence::PassedVerification { command } if command.contains("cargo test plan")
    )));
    assert!(evidence
        .iter()
        .any(|e| matches!(e, Evidence::ChangedFile { path } if path == "tests/plan_spec.rs")));
}

// ---------------------------------------------------------------------------
// refactor
// ---------------------------------------------------------------------------

#[test]
fn refactor_fake_provider_1_classifies_minimal_change_plan() {
    let out = run_scenario(
        "Refactor the verify module into smaller functions",
        vec![
            tools(serde_json::json!([read_call("r1", "src/verify.rs")])),
            Reply::Claim,
        ],
    );
    assert_eq!(out.task_type, TaskType::Refactor);
    assert_eq!(out.skill.name, "refactor");
    assert_eq!(out.skill.context_strategy.label(), "minimal-change");
    assert!(
        out.plan.requires_verify,
        "refactor must prove tests still pass"
    );
    assert_eq!(
        out.plan.subtasks.iter().map(|s| s.kind).collect::<Vec<_>>(),
        vec![SubtaskKind::Read, SubtaskKind::Edit, SubtaskKind::Verify]
    );
    assert!(
        !out.skill.allows(ToolName::DeleteFile),
        "refactor skill must not allow delete_file"
    );
    // A claim after only reading cannot finish a refactor.
    assert_ne!(out.state, AgentState::Finish);
    assert!(out.terminated);
    assert!(!out.acceptance_ok);
    assert_done_evidence(&out.plan);
}

#[test]
fn refactor_fake_provider_2_patches_and_verifies_to_finish() {
    let out = run_scenario_with_verify(
        "重构上下文扫描逻辑",
        vec![
            tools(serde_json::json!([read_call("r1", "src/context.rs")])),
            tools(serde_json::json!([patch_call("p1", "src/context.rs")])),
        ],
        Some("cargo test --workspace --quiet"),
    );
    assert_eq!(out.task_type, TaskType::Refactor);
    assert_eq!(out.state, AgentState::Finish, "failures={:?}", out.failures);
    assert!(out.acceptance_ok);
    assert!(out.verify_entered);
    assert!(out.executed.contains(&"apply_patch".to_owned()));
    assert!(out.files_written.contains(&"src/context.rs".to_owned()));
    assert_done_evidence(&out.plan);
}

// ---------------------------------------------------------------------------
// code-review
// ---------------------------------------------------------------------------

#[test]
fn code_review_fake_provider_1_read_only_plan_finishes_without_writes() {
    let out = run_scenario(
        "Please review the patch in crates/agent",
        vec![
            tools(serde_json::json!([read_call("r1", "src/lib.rs")])),
            tools(serde_json::json!([read_call("r2", "src/plan.rs")])),
            tools(serde_json::json!([command_call("c1", "git diff --stat")])),
            Reply::Claim,
        ],
    );
    assert_eq!(out.task_type, TaskType::CodeReview);
    assert_eq!(out.skill.name, "code-review");
    assert_eq!(out.skill.verification_policy, VerificationPolicy::None);
    assert!(
        !out.plan.requires_verify,
        "review must not run project verification"
    );
    assert!(
        !out.plan
            .subtasks
            .iter()
            .any(|s| s.kind == SubtaskKind::Edit),
        "review plan must contain no edit steps"
    );
    assert!(
        !out.executed
            .iter()
            .any(|t| { ToolName::parse(t).map(|n| n.is_mutation()).unwrap_or(false) }),
        "no mutation tool may execute under code-review: {:?}",
        out.executed
    );
    assert!(
        !out.verify_entered,
        "review must never enter the Verify gate"
    );
    assert_eq!(out.state, AgentState::Finish, "failures={:?}", out.failures);
    assert!(out.acceptance_ok);
    assert!(out.files_written.is_empty());
    assert!(out.rejections.is_empty());
    assert_done_evidence(&out.plan);
}

#[test]
fn code_review_fake_provider_2_write_attempt_is_gated_and_review_still_completes() {
    let out = run_scenario(
        "审查一下这个 PR 的改动",
        vec![
            // Model misbehaves: tries to edit under a read-only skill.
            tools(serde_json::json!([write_call("w1", "src/lib.rs")])),
            tools(serde_json::json!([read_call("r1", "src/lib.rs")])),
            tools(serde_json::json!([command_call(
                "c1",
                "git status --short"
            )])),
            Reply::Claim,
        ],
    );
    assert_eq!(out.task_type, TaskType::CodeReview);
    assert_eq!(out.skill.name, "code-review");
    // The skill gate rejected the write before any Permission approval.
    assert_eq!(out.rejections.len(), 1, "rejections={:?}", out.rejections);
    assert_eq!(out.rejections[0].0, "write_file");
    assert_eq!(out.rejections[0].1, ToolErrorCode::PermissionDenied);
    assert!(!out.executed.contains(&"write_file".to_owned()));
    assert!(
        out.files_written.is_empty(),
        "no file may be written: {:?}",
        out.files_written
    );
    assert!(!out.verify_entered);
    assert_eq!(out.state, AgentState::Finish, "failures={:?}", out.failures);
    assert!(
        out.acceptance_ok,
        "read-only review can still complete honestly: {:?}",
        out.failures
    );
    assert_done_evidence(&out.plan);
}

// ---------------------------------------------------------------------------
// docs
// ---------------------------------------------------------------------------

#[test]
fn docs_fake_provider_1_updates_docs_without_running_compile() {
    let out = run_scenario(
        "Update the README installation section",
        vec![
            tools(serde_json::json!([read_call("r1", "README.md")])),
            tools(serde_json::json!([write_call("w1", "README.md")])),
            tools(serde_json::json!([read_call("r2", "README.md")])),
            Reply::Claim,
        ],
    );
    assert_eq!(out.task_type, TaskType::Docs);
    assert_eq!(out.skill.name, "docs");
    assert_eq!(out.skill.verification_policy, VerificationPolicy::None);
    assert!(!out.skill.verification_policy.needs_run());
    assert!(
        !out.plan.requires_verify,
        "docs must not require a full verify run"
    );
    assert!(
        out.plan.subtasks[0]
            .title
            .to_lowercase()
            .contains("documentation"),
        "first step locates docs: {}",
        out.plan.subtasks[0].title
    );
    assert!(
        !out.skill.allows(ToolName::RunCommand),
        "docs skill forbids shell commands"
    );
    assert!(
        !out.verify_entered,
        "docs task must never run a full compile"
    );
    assert_eq!(out.state, AgentState::Finish, "failures={:?}", out.failures);
    assert!(out.acceptance_ok);
    assert!(out.files_written.contains(&"README.md".to_owned()));
    assert_done_evidence(&out.plan);
}

#[test]
fn docs_fake_provider_2_shell_attempt_is_gated_and_no_fake_success() {
    let out = run_scenario(
        "补充 API 使用文档",
        vec![
            // Model tries to compile — the skill forbids run_command entirely.
            tools(serde_json::json!([command_call(
                "c1",
                "cargo test --workspace"
            )])),
            Reply::Claim,
        ],
    );
    assert_eq!(out.task_type, TaskType::Docs);
    assert_eq!(out.skill.name, "docs");
    assert_eq!(out.rejections.len(), 1, "rejections={:?}", out.rejections);
    assert_eq!(out.rejections[0].0, "run_command");
    assert_eq!(out.rejections[0].1, ToolErrorCode::PermissionDenied);
    assert!(!out.executed.contains(&"run_command".to_owned()));
    assert!(!out.verify_entered, "docs never enters the Verify gate");
    assert!(out.files_written.is_empty());
    // Cannot claim docs done without actually writing docs — honest failure.
    assert_ne!(out.state, AgentState::Finish, "must not fake success");
    assert!(out.terminated);
    assert!(!out.acceptance_ok);
    assert_done_evidence(&out.plan);
}
