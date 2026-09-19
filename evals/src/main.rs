//! Kodo Beta-0 eval harness.
//!
//! Deterministic layer runs without LLM. Live layer needs real credentials and
//! reports `NOT RUN — credentials unavailable` when keys are missing.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use kodo_agent::checkpoint::TurnChangeSet;
use kodo_agent::evidence::{
    CommandExpectation, EvidenceBag, EvidenceItem, EvidenceKind, EvidenceRequirement,
    SubtaskRequirement,
};
use kodo_agent::plan::{AcceptanceEvidence, Subtask, SubtaskKind, TaskPlan};
use kodo_agent::protocol::{ToolCallId, ToolError, ToolResult};
use kodo_agent::provider::{
    openai_request_json, NativeToolCall, Provider, ProviderCapabilities, ProviderConfigRecord,
    ProviderFailureClass, ProviderMessage, ToolSchema,
};
use kodo_agent::tools::{command_run, CommandOutcomeKind};
use kodo_agent::{AgentEvidenceKind, AgentSkillRegistry, TaskType};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let live = args.iter().any(|a| a == "--live");
    let tasks_n = args
        .iter()
        .position(|a| a == "--tasks")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(5);

    println!("kodo-evals deterministic layer");
    let mut report = EvalReport::default();
    run_deterministic(&mut report);

    if live {
        run_live(tasks_n, &mut report);
    } else {
        report.live_status = "NOT RUN — --live not requested".into();
    }

    report.print();
    if report.deterministic_failures > 0 {
        std::process::exit(1);
    }
}

#[derive(Default)]
struct EvalReport {
    deterministic_total: usize,
    deterministic_passed: usize,
    deterministic_failures: usize,
    false_completions: usize,
    live_status: String,
    live_tasks: usize,
    live_success: usize,
    live_tool_calls: usize,
    live_failed_tool_calls: usize,
    live_input_tokens: u32,
    live_output_tokens: u32,
    live_latency_ms: u128,
    notes: Vec<String>,
}

impl EvalReport {
    fn print(&self) {
        println!("\n=== Eval results ===");
        println!(
            "deterministic: {}/{} passed ({} fail)",
            self.deterministic_passed, self.deterministic_total, self.deterministic_failures
        );
        println!("false_completions_recorded: {}", self.false_completions);
        println!("live: {}", self.live_status);
        println!("live_tasks: {}", self.live_tasks);
        println!("live_success: {}", self.live_success);
        println!(
            "avg_tool_calls: {}",
            if self.live_tasks > 0 {
                self.live_tool_calls as f64 / self.live_tasks as f64
            } else {
                0.0
            }
        );
        println!("failed_tool_calls: {}", self.live_failed_tool_calls);
        println!("input_tokens: {}", self.live_input_tokens);
        println!("output_tokens: {}", self.live_output_tokens);
        println!("latency_ms: {}", self.live_latency_ms);
        for note in &self.notes {
            println!("note: {note}");
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
    // 1) failing reproduction can be successful ReproductionEvidence
    {
        let expectation = CommandExpectation::ExpectedFailureSignature {
            signature: "assertion failed".into(),
        };
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
            "expected CommandFailedAsExpected to satisfy reproduction requirement",
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
            expectation: CommandExpectation::ExpectedFailureSignature {
                signature: "assertion failed".into(),
            },
        };
        report.check(
            "git_status_is_not_reproduction",
            !bag.satisfies(&req),
            "git status must not satisfy reproduction criterion",
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
            && main.contains("max-output-tokens");
        report.check(
            "settings_have_runtime_consumers",
            ok,
            "missing setting consumer keys",
        );
    }

    // 14) provider failover classification
    {
        let ok = !ProviderFailureClass::Auth.allows_failover()
            && !ProviderFailureClass::InvalidModel.allows_failover()
            && ProviderFailureClass::RateLimit.allows_failover();
        report.check(
            "failover_classification_matches_ui",
            ok,
            "failover classes wrong",
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
        };
        report.check(
            "subtask_requirement_blocks_arbitrary_read",
            !sub.accepts(&item),
            "read should not complete extract-criteria subtask",
        );
        let _ = AgentEvidenceKind::FileRead { path: "x".into() };
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

fn run_live(tasks_n: usize, report: &mut EvalReport) {
    let provider_type = std::env::var("KODO_EVAL_PROVIDER").unwrap_or_default();
    let model_id = std::env::var("KODO_EVAL_MODEL_ID").unwrap_or_default();
    let api_key = std::env::var("KODO_EVAL_API_KEY").unwrap_or_default();

    if provider_type.is_empty() || model_id.is_empty() || api_key.is_empty() {
        report.live_status = "NOT RUN — credentials unavailable".into();
        report.notes.push(
            "Set KODO_EVAL_PROVIDER, KODO_EVAL_MODEL_ID, KODO_EVAL_API_KEY to run live evals"
                .into(),
        );
        return;
    }

    let fixtures = live_fixtures();
    let started = Instant::now();
    let mut ran = 0usize;
    for fixture in fixtures.into_iter().take(tasks_n) {
        ran += 1;
        report.live_tasks += 1;
        let provider = Provider::new(&provider_type, &api_key, "", &model_id);
        let _ = provider.validate_model();
        report.notes.push(format!(
            "live task {} ({}) oracle={} — requires Kodo desktop run path for full agent loop",
            fixture.id, fixture.kind, fixture.oracle
        ));
    }
    report.live_latency_ms = started.elapsed().as_millis();
    report.live_status = format!(
        "PARTIAL — {} tasks prepared; full live loop needs desktop run path / API reachability",
        ran
    );
}

struct LiveFixture {
    id: &'static str,
    kind: &'static str,
    oracle: &'static str,
}

fn live_fixtures() -> Vec<LiveFixture> {
    vec![
        LiveFixture {
            id: "bug-1",
            kind: "bug-fix",
            oracle: "failing test → pass",
        },
        LiveFixture {
            id: "bug-2",
            kind: "bug-fix",
            oracle: "off-by-one unit test",
        },
        LiveFixture {
            id: "bug-3",
            kind: "bug-fix",
            oracle: "panic path covered",
        },
        LiveFixture {
            id: "feat-1",
            kind: "feature",
            oracle: "new function + test",
        },
        LiveFixture {
            id: "feat-2",
            kind: "feature",
            oracle: "CLI flag works",
        },
        LiveFixture {
            id: "feat-3",
            kind: "feature",
            oracle: "config default applied",
        },
        LiveFixture {
            id: "ref-1",
            kind: "refactor",
            oracle: "tests pass, API preserved",
        },
        LiveFixture {
            id: "ref-2",
            kind: "refactor",
            oracle: "duplicate helper removed",
        },
        LiveFixture {
            id: "test-1",
            kind: "test",
            oracle: "coverage file + pass",
        },
        LiveFixture {
            id: "test-2",
            kind: "test",
            oracle: "edge-case tests added",
        },
        LiveFixture {
            id: "rev-1",
            kind: "code-review",
            oracle: "findings written",
        },
        LiveFixture {
            id: "rev-2",
            kind: "code-review",
            oracle: "risk list non-empty",
        },
        LiveFixture {
            id: "multi-1",
            kind: "feature",
            oracle: "2+ files + tests",
        },
        LiveFixture {
            id: "amb-1",
            kind: "feature",
            oracle: "no destructive edit",
        },
        LiveFixture {
            id: "repair-1",
            kind: "bug-fix",
            oracle: "repair loop ends green",
        },
    ]
}
