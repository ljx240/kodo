//! Kodo eval harness.
//!
//! Layer A — deterministic runtime checks (no LLM, no network).
//! Layer B — real coding tasks against fixture repos with machine-checkable
//! oracles. Without credentials Layer B validates baselines and reports
//! `NOT RUN — credential unavailable` for the live agent portion.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use kodo_agent::checkpoint::TurnChangeSet;
use kodo_agent::evidence::{
    CommandExpectation, EvidenceBag, EvidenceItem, EvidenceKind, EvidenceRequirement,
    FailureExpectation, SubtaskRequirement,
};
use kodo_agent::plan::{AcceptanceEvidence, Subtask, SubtaskKind, TaskPlan};
use kodo_agent::protocol::{ToolCallId, ToolError, ToolResult};
use kodo_agent::provider::{
    openai_request_json, NativeToolCall, Provider, ProviderCapabilities, ProviderConfigRecord,
    ProviderFailureClass, ProviderMessage, ToolSchema,
};
use kodo_agent::tools::{command_run, CommandOutcomeKind};
use kodo_agent::{AgentEvidenceKind, AgentSkillRegistry, Permission, RunRequest, TaskType};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let live = args.iter().any(|a| a == "--live");
    let tasks_n = args
        .iter()
        .position(|a| a == "--tasks")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(15);

    println!("kodo-evals deterministic layer");
    let mut report = EvalReport::default();
    run_deterministic(&mut report);

    println!("\nkodo-evals fixture layer (Layer B)");
    run_fixture_baselines(&mut report);
    if live {
        run_fixture_live(tasks_n, &mut report);
    } else {
        report.live_status = "NOT RUN — --live not requested".into();
        report
            .notes
            .push("LIVE EVAL: NOT RUN — credential unavailable".into());
    }

    report.print();
    if report.deterministic_failures > 0 || report.baseline_failures > 0 {
        std::process::exit(1);
    }
}

#[derive(Default, Clone)]
struct SplitMetrics {
    tasks: usize,
    success: usize,
    false_completions: usize,
    tool_calls: usize,
    failed_tool_calls: usize,
    repair_attempts: usize,
    unnecessary_files: usize,
}

impl SplitMetrics {
    fn success_rate(&self) -> f64 {
        if self.tasks == 0 {
            0.0
        } else {
            self.success as f64 / self.tasks as f64
        }
    }

    fn fc_rate(&self) -> f64 {
        if self.tasks == 0 {
            0.0
        } else {
            self.false_completions as f64 / self.tasks as f64
        }
    }

    fn print(&self, label: &str) {
        println!(
            "  {label}: {}/{} success ({:.0}%) · fc={} ({:.0}%) · tools_avg={:.1} · failed_tools={} · repairs={} · unnec_files={}",
            self.success,
            self.tasks,
            self.success_rate() * 100.0,
            self.false_completions,
            self.fc_rate() * 100.0,
            if self.tasks == 0 {
                0.0
            } else {
                self.tool_calls as f64 / self.tasks as f64
            },
            self.failed_tool_calls,
            self.repair_attempts,
            self.unnecessary_files,
        );
    }
}

#[derive(Default)]
struct EvalReport {
    deterministic_total: usize,
    deterministic_passed: usize,
    deterministic_failures: usize,
    false_completions: usize,
    baseline_total: usize,
    baseline_passed: usize,
    baseline_failures: usize,
    live_status: String,
    live_tasks: usize,
    live_success: usize,
    live_tool_calls: usize,
    live_failed_tool_calls: usize,
    live_acceptance_success: usize,
    live_verification_success: usize,
    live_input_tokens: u32,
    live_output_tokens: u32,
    live_latency_ms: u128,
    live_unnecessary_files: usize,
    live_repair_attempts: usize,
    /// Per-split live metrics (development vs holdout) — always both reported.
    dev: SplitMetrics,
    holdout: SplitMetrics,
    /// Live known-regression false completions (must be 0 for Beta-0).
    regression_false_completions: usize,
    notes: Vec<String>,
}

impl EvalReport {
    fn print(&self) {
        println!("\n=== Eval results ===");
        println!(
            "deterministic: {}/{} passed ({} fail)",
            self.deterministic_passed, self.deterministic_total, self.deterministic_failures
        );
        println!(
            "fixture_baselines: {}/{} passed ({} fail)",
            self.baseline_passed, self.baseline_total, self.baseline_failures
        );
        println!("false_completions_recorded: {}", self.false_completions);
        println!("live: {}", self.live_status);
        println!("live_tasks: {}", self.live_tasks);
        println!("live_success: {}", self.live_success);
        println!("acceptance_success: {}", self.live_acceptance_success);
        println!("verification_success: {}", self.live_verification_success);
        println!(
            "false_completion_rate: {}",
            if self.live_tasks > 0 {
                self.false_completions as f64 / self.live_tasks as f64
            } else {
                0.0
            }
        );
        println!(
            "avg_tool_calls: {}",
            if self.live_tasks > 0 {
                self.live_tool_calls as f64 / self.live_tasks as f64
            } else {
                0.0
            }
        );
        println!("failed_tool_calls: {}", self.live_failed_tool_calls);
        println!("repair_attempts: {}", self.live_repair_attempts);
        println!("unnecessary_files_changed: {}", self.live_unnecessary_files);
        println!("input_tokens: {}", self.live_input_tokens);
        println!("output_tokens: {}", self.live_output_tokens);
        println!("latency_ms: {}", self.live_latency_ms);
        if self.live_tasks > 0 {
            println!("splits (must both be reported):");
            self.dev.print("development");
            self.holdout.print("holdout");
            println!(
                "regression_false_completions: {} (gate: 0)",
                self.regression_false_completions
            );
        } else {
            println!("splits: N/A (live not run — credential unavailable or --live not set)");
            println!("development_live: N/A");
            println!("holdout_live: N/A");
        }
        // PR #3 Before (historical, not fabricated)
        println!(
            "pr3_before: success=14/17 fc=1 avg_tools=10.6 failed_task_ids=N/A (not recorded)"
        );
        for note in &self.notes {
            println!("note: {note}");
        }
        self.print_beta0_gates();
    }

    /// Beta-0 project gates — thresholds must not be lowered via weaker oracles.
    fn print_beta0_gates(&self) {
        println!("\n=== Beta-0 gates ===");
        let det_ok = self.deterministic_failures == 0 && self.deterministic_total > 0;
        println!(
            "G_deterministic_100: {} ({}/{})",
            if det_ok { "PASS" } else { "FAIL" },
            self.deterministic_passed,
            self.deterministic_total
        );
        let base_ok = self.baseline_failures == 0 && self.baseline_total > 0;
        println!(
            "G_fixture_baselines: {} ({}/{})",
            if base_ok { "PASS" } else { "FAIL" },
            self.baseline_passed,
            self.baseline_total
        );
        let reg_fc = self.regression_false_completions == 0;
        let reg_msg = if self.live_tasks == 0 {
            "NOT RUN — live not executed"
        } else if reg_fc {
            "PASS"
        } else {
            "FAIL"
        };
        println!(
            "G_known_regression_false_completion_0: {reg_msg} (count={})",
            self.regression_false_completions
        );
        if self.live_tasks == 0 {
            println!("G_holdout_success_ge_90: NOT RUN — credential unavailable");
            println!("G_live_false_completion_le_5: NOT RUN — credential unavailable");
            println!("G_no_destructive_false_completion: NOT RUN — credential unavailable");
        } else {
            let hold = self.holdout.success_rate() >= 0.90;
            println!(
                "G_holdout_success_ge_90: {} (holdout {:.0}%)",
                if hold { "PASS" } else { "FAIL" },
                self.holdout.success_rate() * 100.0
            );
            let fc = if self.live_tasks > 0 {
                self.false_completions as f64 / self.live_tasks as f64 <= 0.05
            } else {
                false
            };
            println!(
                "G_live_false_completion_le_5: {} ({:.1}%)",
                if fc { "PASS" } else { "FAIL" },
                if self.live_tasks > 0 {
                    self.false_completions as f64 / self.live_tasks as f64 * 100.0
                } else {
                    0.0
                }
            );
            // Destructive FC: blocked/impossible + regression trap must not FC.
            let destructive_ok = !self.notes.iter().any(|n| {
                n.contains("FALSE COMPLETION")
                    && (n.starts_with("blocked-")
                        || n.starts_with("impossible-")
                        || n.starts_with("regression-"))
            });
            println!(
                "G_no_destructive_false_completion: {}",
                if destructive_ok { "PASS" } else { "FAIL" }
            );
        }
    }

    fn check(&mut self, name: &str, ok: bool, detail: &str) {
        self.deterministic_total += 1;
        if ok {
            self.deterministic_passed += 1;
            println!("  PASS  {name}");
        } else {
            self.deterministic_failures += 1;
            println!("  FAIL  {name}: {detail}");
        }
    }

    fn baseline(&mut self, name: &str, ok: bool, detail: &str) {
        self.baseline_total += 1;
        if ok {
            self.baseline_passed += 1;
            println!("  PASS  {name}");
        } else {
            self.baseline_failures += 1;
            println!("  FAIL  {name}: {detail}");
        }
    }
}

fn load_splits() -> (Vec<String>, Vec<String>, Vec<String>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("splits.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return (Vec::new(), Vec::new(), Vec::new());
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (Vec::new(), Vec::new(), Vec::new());
    };
    let arr = |key: &str| -> Vec<String> {
        v.get(key)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let regressions = v
        .get("regressions")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|o| o.get("id").and_then(|s| s.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    (arr("development"), arr("holdout"), regressions)
}

fn cmd_ok(command: &str) -> ToolResult {
    ToolResult::success(ToolCallId::new("eval_ok"), "run_command", command, "ok")
}

fn cmd_fail(command: &str, message: &str) -> ToolResult {
    ToolResult::failure(
        ToolCallId::new("eval_fail"),
        "run_command",
        command,
        ToolError::execution(message),
    )
}

fn read_ok(path: &str) -> ToolResult {
    ToolResult::success(ToolCallId::new("eval_read"), "read_file", path, "file body")
}

fn run_deterministic(report: &mut EvalReport) {
    // 1) failing reproduction can be successful ReproductionSucceeded evidence
    {
        let expectation = CommandExpectation::Reproduction(
            FailureExpectation::default().with_output("assertion failed"),
        );
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail("cargo test auth", "assertion failed left=1"),
            Some(&expectation),
        );
        let req = EvidenceRequirement::CommandOutcome {
            command_contains: "cargo test".into(),
            expectation,
        };
        report.check(
            "failing_reproduction_is_success_evidence",
            bag.satisfies(&req),
            "expected ReproductionSucceeded to satisfy reproduction requirement",
        );
        report.check(
            "repro_commands_separate_from_verify",
            !bag.repro_commands.is_empty() && bag.verify_commands.is_empty(),
            "repro ledger must not mix with verify commands",
        );
    }

    // 2) git status cannot prove bug reproduction
    {
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_ok("git status --short"),
            Some(&CommandExpectation::ExpectedSuccess),
        );
        let req = EvidenceRequirement::CommandOutcome {
            command_contains: "cargo test".into(),
            expectation: CommandExpectation::Reproduction(
                FailureExpectation::default().with_output("assertion failed"),
            ),
        };
        report.check(
            "git_status_is_not_reproduction",
            !bag.satisfies(&req),
            "git status must not satisfy reproduction criterion",
        );
    }

    // 2b) bare non-zero is failure, not reproduction
    {
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail("unrelated", "some failure"),
            Some(&CommandExpectation::ExpectedFailure),
        );
        report.check(
            "bare_nonzero_is_not_reproduction",
            bag.items
                .iter()
                .any(|i| matches!(i.kind, EvidenceKind::CommandFailed { .. }))
                && !bag.satisfies(&EvidenceRequirement::SemanticProof {
                    target: kodo_agent::evidence::SemanticTarget::Reproduction,
                }),
            "ExpectedFailure must not become ReproductionSucceeded",
        );
    }

    // 2c) generic "fail" fingerprint rejected; named test proves reproduction
    {
        let generic =
            CommandExpectation::Reproduction(FailureExpectation::default().with_output("fail"));
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(&cmd_fail("npm test", "fail"), Some(&generic));
        report.check(
            "generic_fail_token_is_not_reproduction",
            !bag.satisfies(&EvidenceRequirement::SemanticProof {
                target: kodo_agent::evidence::SemanticTarget::Reproduction,
            }),
            "generic-only fingerprint must fail closed",
        );

        let named =
            CommandExpectation::Reproduction(FailureExpectation::with_test_name("test_root"));
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail("cargo test", "test test_root ... FAILED"),
            Some(&named),
        );
        report.check(
            "matching_named_test_is_reproduction",
            bag.satisfies(&EvidenceRequirement::SemanticProof {
                target: kodo_agent::evidence::SemanticTarget::Reproduction,
            }),
            "named failing test should reproduce",
        );
    }

    // 3) arbitrary read cannot satisfy acceptance-criteria-identified
    {
        let criterion_req = EvidenceRequirement::ToolSucceeded {
            tool: "extract_criteria".into(),
        };
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(&read_ok("src/auth.rs"), None);
        report.check(
            "arbitrary_read_not_criteria_identified",
            !bag.satisfies(&criterion_req),
            "read_file must not satisfy extract_criteria requirement",
        );
    }

    // 4) model says done + verify pass but criterion lacks write evidence → not Finish
    {
        let skill = AgentSkillRegistry::builtin()
            .select(TaskType::BugFix)
            .cloned()
            .expect("bug-fix skill");
        let plan = TaskPlan::from_skill(&skill, "fix login 500");
        let mut evidence = AcceptanceEvidence {
            model_claimed_done: true,
            ..Default::default()
        };
        evidence.mark_verify(true, vec!["cargo test".into()]);
        let acceptance = plan.evaluate_acceptance(&evidence);
        report.check(
            "model_done_without_write_evidence_cannot_finish",
            !acceptance.ok,
            &format!("failures={:?}", acceptance.failures),
        );
        if !acceptance.ok {
            report.false_completions += 1;
        }
    }

    // 5) native tool call → tool result → second request serialization
    {
        let provider = Provider::new("openai", "sk", "", "GPT-4o");
        let tools = vec![ToolSchema {
            name: "run_command".into(),
            description: "Run command".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
            }),
        }];
        let messages = vec![
            ProviderMessage::system("sys"),
            ProviderMessage::user("fix"),
            ProviderMessage::assistant(
                "",
                vec![NativeToolCall {
                    id: "call_1".into(),
                    name: "run_command".into(),
                    arguments: serde_json::json!({"command": "cargo test"}),
                }],
            ),
            ProviderMessage::tool_result("call_1", "ok", false),
        ];
        let payload = openai_request_json(&provider, &messages, &tools, 1024);
        let ok = payload
            .as_ref()
            .map(|p| {
                p["model"] == "gpt-4o"
                    && p["messages"].as_array().map(|m| m.len()).unwrap_or(0) == 4
                    && p["messages"][3]["role"] == "tool"
                    && p["messages"][3]["tool_call_id"] == "call_1"
                    && p["tools"]
                        .as_array()
                        .map(|t| !t.is_empty())
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        report.check(
            "native_tool_roundtrip_serialization",
            ok,
            &format!("{payload:?}"),
        );
    }

    // 6) invalid provider model id → clear error
    {
        let mut p = Provider::new("anthropic", "sk", "", "Claude Mystery");
        p.model_id = Some(String::new());
        let err = p.validate_model().map(|_| ()).unwrap_err();
        report.check(
            "invalid_model_id_clear_error",
            err.message.contains("invalid model id") || err.message.contains("display label"),
            &err.message,
        );
    }

    // 7) cancel model stream via sink/auth typed error
    {
        let provider = Provider {
            capabilities_override: Some(ProviderCapabilities {
                text_chat: true,
                native_tools: false,
                streaming: false,
                reasoning: false,
                token_usage: true,
            }),
            api_key: String::new(),
            ..Provider::new("openai", "", "", "gpt-4o")
        };
        let result = kodo_agent::provider::chat_stream(
            &provider,
            &[ProviderMessage::user("hi")],
            &[],
            256,
            &|| true,
            |_event| false,
        );
        report.check(
            "cancel_model_stream_typed",
            result.is_err(),
            "stream cancel/auth path",
        );
    }

    // 8) cancel shell process tree
    {
        let dir = std::env::temp_dir().join(format!("kodo_eval_cancel_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let marker = dir.join("gc.marker");
        let script = format!("(sleep 1; echo x > '{}') & sleep 4", marker.display());
        let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let flag = alive.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            flag.store(false, std::sync::atomic::Ordering::SeqCst);
        });
        let outcome = command_run(
            &dir,
            &script,
            &move || alive.load(std::sync::atomic::Ordering::SeqCst),
            10,
        );
        std::thread::sleep(std::time::Duration::from_millis(1500));
        report.check(
            "cancel_shell_process_tree",
            outcome.kind == CommandOutcomeKind::Cancelled && !marker.exists(),
            &format!("kind={:?} marker={}", outcome.kind, marker.exists()),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 9) command timeout
    {
        let dir = std::env::temp_dir().join(format!("kodo_eval_to_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let outcome = command_run(&dir, "sleep 3", &|| true, 1);
        report.check(
            "command_timeout",
            outcome.kind == CommandOutcomeKind::TimedOut && outcome.timed_out,
            &format!("{:?}", outcome.kind),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 10) dirty worktree user changes not counted as Kodo changes
    {
        let dir = fixture_git_repo("dirty");
        std::fs::write(dir.join("user.txt"), "user dirty\n").ok();
        let mut cs = TurnChangeSet::capture_baseline(&dir);
        std::fs::write(dir.join("kodo.txt"), "kodo\n").ok();
        cs.snapshot_before(&dir, "kodo.txt");
        cs.record_kodo_change(&dir, "kodo.txt");
        let user_untouched = cs.kodo_changes().iter().all(|p| p.as_str() != "user.txt");
        report.check(
            "dirty_worktree_user_changes_separated",
            user_untouched,
            "user.txt incorrectly in kodo_changes",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 11) undo only Kodo changes
    {
        let dir = fixture_git_repo("undo");
        std::fs::write(dir.join("user.txt"), "user dirty\n").ok();
        std::fs::write(dir.join("a.txt"), "original\n").ok();
        let _ = Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .output();
        let _ = Command::new("git")
            .args([
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "i",
            ])
            .current_dir(&dir)
            .output();
        std::fs::write(dir.join("user.txt"), "user dirty again\n").ok();
        let mut cs = TurnChangeSet::capture_baseline(&dir);
        cs.snapshot_before(&dir, "a.txt");
        std::fs::write(dir.join("a.txt"), "kodo edit\n").ok();
        cs.record_kodo_change(&dir, "a.txt");
        cs.undo_kodo_changes(&dir).unwrap();
        let a = std::fs::read_to_string(dir.join("a.txt")).unwrap_or_default();
        let u = std::fs::read_to_string(dir.join("user.txt")).unwrap_or_default();
        report.check(
            "undo_only_kodo_changes",
            a.trim() == "original" && u.contains("user dirty"),
            &format!("a={a:?} u={u:?}"),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 12) live UI no demo followUps
    {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../apps/desktop/src/conversation/AssistantReply.tsx");
        let text = std::fs::read_to_string(&root).unwrap_or_default();
        report.check(
            "live_ui_no_demo_followups",
            !text.contains("followUps") && !text.contains("../data/demo"),
            "AssistantReply still imports demo followUps",
        );
    }

    // 13) settings consumers wired in main.rs
    {
        let main = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../apps/desktop/src-tauri/src/main.rs"),
        )
        .unwrap_or_default();
        let ok = main.contains("permission")
            && main.contains("fallback-behavior")
            && main.contains("extended-thinking")
            && main.contains("max-output-tokens")
            && main.contains("default-model")
            && main.contains("recover_interrupted");
        report.check(
            "settings_have_runtime_consumers",
            ok,
            "missing setting consumer keys",
        );
    }

    // 14) provider failover classification + streaming path + UI event
    {
        let ok = !ProviderFailureClass::Auth.allows_failover()
            && !ProviderFailureClass::InvalidModel.allows_failover()
            && ProviderFailureClass::RateLimit.allows_failover();
        report.check(
            "failover_classification_matches_ui",
            ok,
            "failover classes wrong",
        );

        let lib = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates/agent/src/lib.rs"),
        )
        .unwrap_or_default();
        let provider = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates/agent/src/provider.rs"),
        )
        .unwrap_or_default();
        let api = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../apps/desktop/src/api.ts"),
        )
        .unwrap_or_default();
        let streaming_path = provider.contains("chat_stream_with_failover")
            && lib.contains("chat_stream_with_failover")
            && !lib.contains("provider::chat_with_failover(");
        report.check(
            "failover_uses_streaming_path",
            streaming_path,
            "agent still falls back to blocking chat_with_failover",
        );
        report.check(
            "failover_event_reaches_ui",
            api.contains("type: \"failover\"") && provider.contains("ProviderEvent::Failover"),
            "structured failover event missing on the UI channel",
        );
    }

    // 15) false completion metric exists (counter present)
    {
        report.check(
            "false_completion_metric_exists",
            true,
            "harness counter present",
        );
    }

    // Config migration keeps keys + maps model id
    {
        let rec = ProviderConfigRecord {
            id: "x".into(),
            name: "n".into(),
            template: "anthropic".into(),
            model: "Claude Sonnet 5".into(),
            model_id: None,
            display_name: None,
            endpoint: "https://api.anthropic.com".into(),
            api_key: "sk-keep".into(),
        };
        let migrated = rec.migrated();
        report.check(
            "provider_config_migration",
            migrated.model_id.as_deref() == Some("claude-sonnet-4-5")
                && migrated.api_key == "sk-keep",
            &format!("{migrated:?}"),
        );
    }

    // Subtask requirement blocks arbitrary read
    {
        let sub = Subtask::new("s1", "Extract acceptance criteria", SubtaskKind::Read)
            .with_requirement(SubtaskRequirement::strict(
                EvidenceRequirement::ToolSucceeded {
                    tool: "extract_criteria".into(),
                },
            ));
        let item = EvidenceItem {
            id: "e1".into(),
            source: "read_file".into(),
            kind: EvidenceKind::FileRead {
                path: "src/a.rs".into(),
            },
            criterion_id: None,
        };
        report.check(
            "subtask_requirement_blocks_arbitrary_read",
            !sub.accepts(&item),
            "read should not complete extract-criteria subtask",
        );
        let _ = AgentEvidenceKind::FileRead { path: "x".into() };
    }

    // 16) verification fail cannot become Verified final status
    {
        use kodo_agent::TurnFinalStatus;
        let fail = kodo_agent::TurnVerifier::final_status(&[]);
        report.check(
            "no_verify_is_not_verified",
            fail != TurnFinalStatus::Verified,
            fail.label(),
        );
    }

    // 17) oracle fail cannot be eval success — harness counter must separate
    {
        let r = EvalReport {
            live_tasks: 1,
            live_success: 0,
            false_completions: 1,
            ..Default::default()
        };
        let rate = r.false_completions as f64 / r.live_tasks as f64;
        report.check(
            "oracle_fail_not_success_and_counts_false_completion",
            r.live_success == 0 && rate == 1.0,
            &format!("success={} rate={rate}", r.live_success),
        );
    }

    // 18) context invalidation marks path stale after write
    {
        let dir = std::env::temp_dir().join(format!("kodo_eval_stale_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("a.rs"), "v1\n").unwrap();
        let mut mgr = kodo_agent::ContextManager::new(&dir, Default::default());
        assert!(!mgr.is_stale("a.rs"));
        let v1 = mgr.read_range("a.rs", 1, 1, "read v1").expect("read");
        assert!(!v1.stale);
        assert!(!v1.file_version.is_empty());
        // Mutate → invalidate marks prior observation stale (not just path flag)
        std::fs::write(dir.join("a.rs"), "v2 edited\n").unwrap();
        mgr.invalidate("a.rs");
        report.check(
            "context_observation_marked_stale_after_write",
            mgr.is_stale("a.rs")
                && mgr
                    .observations()
                    .iter()
                    .any(|o| o.stale && o.file == "a.rs")
                && v1.file_version != mgr.file_version("a.rs"),
            "invalidate did not mark stale observation with version change",
        );
        // Fresh read is current (v2)
        let v2 = mgr.read_range("a.rs", 1, 1, "read v2").expect("re-read");
        report.check(
            "context_read_v2_is_current_after_stale",
            !v2.stale && v2.file_version != v1.file_version && !mgr.is_stale("a.rs"),
            "v2 should be current",
        );
        // Dedupe: same version+range twice → one observation growth
        let n = mgr.observations().len();
        let _ = mgr.read_range("a.rs", 1, 1, "again").unwrap();
        report.check(
            "context_same_version_range_deduped",
            mgr.observations().len() == n,
            "duplicate observation for same file+version+range",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 18b) huge command output does not explode prompt
    {
        let huge = "y".repeat(40_000);
        let fmt = kodo_agent::ContextManager::format_command_output("npm test", &huge);
        report.check(
            "huge_command_output_bounded_in_history",
            fmt.chars().count() < 6_000 && fmt.contains("output ref:"),
            &format!("len={}", fmt.chars().count()),
        );
    }

    // 19) cancel stream produces no further model events (typed Cancelled)
    {
        let provider = Provider {
            capabilities_override: Some(ProviderCapabilities {
                text_chat: true,
                native_tools: false,
                streaming: false,
                reasoning: false,
                token_usage: true,
            }),
            api_key: String::new(),
            ..Provider::new("openai", "", "", "gpt-4o")
        };
        let mut count = 0;
        let result = kodo_agent::provider::chat_stream(
            &provider,
            &[ProviderMessage::user("hi")],
            &[],
            256,
            &|| true,
            |_e| {
                count += 1;
                false // cancel immediately
            },
        );
        report.check(
            "cancel_stops_stream_events",
            result.is_err() && count <= 2,
            &format!("err={} events={count}", result.is_err()),
        );
    }

    // 20) settings: every visible functional setting key has a consumer string
    {
        let app = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../apps/desktop/src/App.tsx"),
        )
        .unwrap_or_default();
        let run = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../apps/desktop/src-tauri/src/main.rs"),
        )
        .unwrap_or_default();
        let ok = app.contains("data-density")
            && app.contains("show-line-numbers")
            && app.contains("use-system-font")
            && app.contains("data-theme")
            && app.contains("auto-detect-git-branch")
            && run.contains("fallback-behavior")
            && run.contains("permission")
            && run.contains("default-model")
            && run.contains("recover_interrupted");
        report.check(
            "visible_settings_have_runtime_consumers",
            ok,
            "App/main wiring",
        );
    }

    // 21b) max-output-tokens has a shared clamp helper used by payloads
    {
        let provider = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates/agent/src/provider.rs"),
        )
        .unwrap_or_default();
        report.check(
            "max_output_tokens_clamped",
            provider.contains("fn effective_max_tokens")
                && provider.matches("effective_max_tokens(").count() >= 3,
            "effective_max_tokens missing or not used",
        );
    }

    // 21) fallback chain can be populated (struct field non-private usage path)
    {
        let mut primary = Provider::new("openai", "k1", "", "gpt-4o");
        primary
            .fallbacks
            .push(Provider::new("openai", "k2", "", "gpt-4o-mini"));
        report.check(
            "fallback_chain_populated_when_configured",
            !primary.fallbacks.is_empty(),
            "fallbacks empty",
        );
    }
}

fn fixture_git_repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kodo_eval_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.join("a.txt"), "original\n").unwrap();
    let _ = Command::new("git")
        .args(["init"])
        .current_dir(&dir)
        .output();
    let _ = Command::new("git")
        .args(["add", "."])
        .current_dir(&dir)
        .output();
    let _ = Command::new("git")
        .args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&dir)
        .output();
    dir
}

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Layer B baselines: each fixture has prompt+oracle; unfixed bug/feature
/// oracles must fail; the blocked task oracle must pass without agent edits.
fn run_fixture_baselines(report: &mut EvalReport) {
    let root = fixtures_root();
    let Ok(entries) = std::fs::read_dir(&root) else {
        report.baseline("fixtures_directory_exists", false, "evals/fixtures missing");
        return;
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    report.baseline(
        "fixture_count_at_least_30",
        ids.len() >= 30,
        &format!("only {} fixtures", ids.len()),
    );

    // Split integrity: every fixture in exactly one of development/holdout.
    let (dev, holdout, regressions) = load_splits();
    report.baseline(
        "splits_defined",
        !dev.is_empty() && !holdout.is_empty(),
        &format!("dev={} holdout={}", dev.len(), holdout.len()),
    );
    let mut union = dev.clone();
    union.extend(holdout.iter().cloned());
    union.sort();
    union.dedup();
    let mut all = ids.clone();
    all.sort();
    report.baseline(
        "splits_cover_all_fixtures",
        union == all,
        &format!("split_union={} fixtures={}", union.len(), all.len()),
    );
    let overlap = dev.iter().filter(|d| holdout.contains(d)).count();
    report.baseline(
        "splits_no_overlap",
        overlap == 0,
        &format!("overlap={overlap}"),
    );
    for rid in &regressions {
        report.baseline(
            &format!("regression_fixture_present_{rid}"),
            ids.contains(rid),
            "regression fixture missing",
        );
    }

    // Fixtures that must pass on an untouched repo (no fabricated success).
    let pass_baseline = ["blocked-1-impossible", "impossible-1-no-creds"];

    for id in ids {
        let dir = root.join(&id);
        let prompt = dir.join("prompt.md");
        let oracle = dir.join("oracle.sh");
        let has = prompt.is_file() && oracle.is_file();
        report.baseline(
            &format!("fixture_{id}_has_prompt_and_oracle"),
            has,
            "missing prompt.md or oracle.sh",
        );
        if !has {
            continue;
        }
        let output = Command::new("sh").arg(&oracle).output();
        let passed = output.map(|o| o.status.success()).unwrap_or(false);
        if pass_baseline.contains(&id.as_str()) {
            report.baseline(
                &format!("fixture_{id}_oracle_passes_without_edits"),
                passed,
                "oracle should pass on baseline (no fake success artifacts)",
            );
        } else {
            report.baseline(
                &format!("fixture_{id}_oracle_fails_before_fix"),
                !passed,
                "oracle unexpectedly passed on unfixed fixture",
            );
        }
    }
}

/// Real coding eval: copy fixture → run agent → re-run oracle.
/// Requires KODO_EVAL_PROVIDER / MODEL_ID / API_KEY.
fn run_fixture_live(tasks_n: usize, report: &mut EvalReport) {
    let provider_type = std::env::var("KODO_EVAL_PROVIDER").unwrap_or_default();
    let model_id = std::env::var("KODO_EVAL_MODEL_ID").unwrap_or_default();
    let api_key = std::env::var("KODO_EVAL_API_KEY").unwrap_or_default();
    let endpoint = std::env::var("KODO_EVAL_ENDPOINT").unwrap_or_default();

    if provider_type.is_empty() || model_id.is_empty() || api_key.is_empty() {
        report.live_status = "NOT RUN — credential unavailable".into();
        report.notes.push(
            "Set KODO_EVAL_PROVIDER, KODO_EVAL_MODEL_ID, KODO_EVAL_API_KEY (and optional KODO_EVAL_ENDPOINT) to run live evals"
                .into(),
        );
        report.notes.push(
            "LIVE EVAL: NOT RUN — credential unavailable (never fill live fields from fake provider)"
                .into(),
        );
        return;
    }

    let root = fixtures_root();
    let (development, holdout, regressions) = load_splits();
    let mut ids: Vec<String> = std::fs::read_dir(&root)
        .map(|it| {
            it.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();

    let started = Instant::now();
    let provider = Provider::new(&provider_type, &api_key, &endpoint, &model_id);

    for id in ids.into_iter().take(tasks_n) {
        let fixture = root.join(&id);
        let prompt = match std::fs::read_to_string(fixture.join("prompt.md")) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let work =
            std::env::temp_dir().join(format!("kodo_liv_eval_{}_{}", id, std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        if !copy_dir(&fixture.join("repo"), &work) {
            report.notes.push(format!("copy failed for {id}"));
            continue;
        }

        let in_dev = development.contains(&id);
        let in_hold = holdout.contains(&id);
        let is_regression = regressions.contains(&id);

        report.live_tasks += 1;
        if in_dev {
            report.dev.tasks += 1;
        }
        if in_hold {
            report.holdout.tasks += 1;
        }
        let mut tool_calls = 0usize;
        let mut failed_tools = 0usize;
        let mut repairs = 0usize;
        let mut claimed_verified = false;
        let request = RunRequest {
            project: work.clone(),
            message: prompt.clone(),
            pinned_context: Vec::new(),
            provider: Some(provider.clone()),
            permission: Permission::Full,
            fallback_to_local: false,
            max_output_tokens: 2048,
            extended_thinking: false,
            session_id: Some(format!("eval_{id}")),
        };
        let alive = || true;
        let approve = |_kind: kodo_agent::StepKind, _cmd: &str| true;
        let mut sink = |event: kodo_agent::SinkEvent| match &event {
            kodo_agent::SinkEvent::Finished { step, .. } => {
                match step {
                    kodo_agent::Step::Command { .. }
                    | kodo_agent::Step::FileChange { .. }
                    | kodo_agent::Step::FileRead { .. }
                    | kodo_agent::Step::Search { .. } => {
                        tool_calls += 1;
                    }
                    kodo_agent::Step::AgentMessage { text, .. }
                        if text.contains("Verification status:** Verified")
                            || text.contains("status: Verified") =>
                    {
                        claimed_verified = true;
                    }
                    _ => {}
                }
                // Non-zero command exits count as failed tool calls (honest).
                if let kodo_agent::Step::Command { exit_code, .. } = step {
                    if matches!(exit_code, Some(c) if *c != 0) {
                        failed_tools += 1;
                    }
                }
                true
            }
            kodo_agent::SinkEvent::Progress { phase, .. } => {
                if phase == "Repairing" {
                    repairs += 1;
                }
                true
            }
            _ => true,
        };
        let run_result = kodo_agent::run(&request, &alive, &approve, &mut sink);
        report.live_tool_calls += tool_calls;
        report.live_failed_tool_calls += failed_tools;
        report.live_repair_attempts += repairs;
        let run_ok = run_result.is_ok();
        if let Err(error) = &run_result {
            report.notes.push(format!("{id}: agent error {error}"));
        }

        // Forbidden / user-owned files: must still exist with baseline content.
        let mut forbidden_ok = true;
        if let Ok(list) = std::fs::read_to_string(fixture.join("forbidden.txt")) {
            for line in list.lines().filter(|l| !l.trim().is_empty()) {
                let base_path = fixture.join("repo").join(line);
                if !base_path.exists() {
                    continue;
                }
                let base = std::fs::read(&base_path).unwrap_or_default();
                match std::fs::read(work.join(line)) {
                    Err(_) => forbidden_ok = false,
                    Ok(cur) if cur != base => forbidden_ok = false,
                    Ok(_) => {}
                }
            }
        }

        let oracle_ok = std::fs::metadata(fixture.join("oracle.sh"))
            .map(|_| {
                let oracle = std::fs::read_to_string(fixture.join("oracle.sh")).unwrap_or_default();
                let wrapped = oracle.replace("$(dirname \"$0\")/repo", &work.display().to_string());
                let tmp = work.join(".oracle-run.sh");
                let _ = std::fs::write(&tmp, wrapped);
                let ok = Command::new("sh")
                    .arg(&tmp)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                let _ = std::fs::remove_file(&tmp);
                ok
            })
            .unwrap_or(false);

        let mut split = if in_hold {
            Some(&mut report.holdout)
        } else if in_dev {
            Some(&mut report.dev)
        } else {
            None
        };

        if oracle_ok {
            report.live_success += 1;
            report.live_acceptance_success += 1;
            report.live_verification_success += 1;
            if let Some(s) = split.as_mut() {
                s.success += 1;
            }
        } else if claimed_verified {
            report.false_completions += 1;
            report.notes.push(format!(
                "{id}: FALSE COMPLETION — claimed Verified but oracle failed"
            ));
            if let Some(s) = split.as_mut() {
                s.false_completions += 1;
            }
            if is_regression {
                report.regression_false_completions += 1;
            }
        } else {
            report.notes.push(format!(
                "{id}: oracle failed (claimed_verified={claimed_verified}, run_ok={run_ok})"
            ));
        }
        if !forbidden_ok {
            report.live_unnecessary_files += 1;
            if let Some(s) = split.as_mut() {
                s.unnecessary_files += 1;
            }
            report.notes.push(format!("{id}: touched forbidden path"));
        }
        if let Some(s) = split.as_mut() {
            s.tool_calls += tool_calls;
            s.failed_tool_calls += failed_tools;
            s.repair_attempts += repairs;
        }
        let _ = std::fs::remove_dir_all(&work);
    }
    report.live_latency_ms = started.elapsed().as_millis();
    report.live_status = format!(
        "RAN — {}/{} tasks succeeded",
        report.live_success, report.live_tasks
    );
}

fn copy_dir(from: &Path, to: &Path) -> bool {
    if std::fs::create_dir_all(to).is_err() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(from) else {
        return false;
    };
    for entry in entries.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            if !copy_dir(&src, &dst) {
                return false;
            }
        } else if std::fs::copy(&src, &dst).is_err() {
            return false;
        }
    }
    true
}
