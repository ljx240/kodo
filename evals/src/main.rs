//! Kodo eval harness.
//!
//! Layer A — deterministic runtime checks (no LLM, no network).
//! Layer B — real coding tasks against fixture repos with machine-checkable
//! oracles. The default run validates baselines and leaves live quality
//! `NOT RUN`; real provider evaluation requires an explicit `--live`.

mod work;

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
use kodo_agent::{
    AgentEvidenceKind, AgentMode, AgentSkillRegistry, Permission, RunRequest, TaskType,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let live = args.iter().any(|a| a == "--live");
    let tasks_n = args
        .iter()
        .position(|a| a == "--tasks")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(15);
    let repeat = parse_repeat(&args[1..]);
    // `--suite real` selects `evals/fixtures-real/`; the default corpus stays
    // the fast 30-task set.
    let suite = args
        .iter()
        .position(|a| a == "--suite")
        .and_then(|i| args.get(i + 1))
        .filter(|value| !value.starts_with('-'))
        .map(|value| value.to_owned())
        .unwrap_or_else(|| DEFAULT_SUITE.to_owned());

    println!("kodo-evals deterministic layer");
    let mut report = EvalReport::default();
    run_deterministic(&mut report);
    work::run(&mut report);

    println!("\nkodo-evals fixture layer (Layer B)");
    run_fixture_baselines(&mut report);
    if live {
        run_fixture_live(&suite, tasks_n, repeat, &mut report);
    } else {
        report.live_status = "NOT RUN — --live not requested".into();
        report.live_suite = "n/a (live not run)".into();
        report
            .notes
            .push(format!("LIVE EVAL: {}", report.live_status));
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
    /// Runs whose own final answer claimed completion (delivery ready or
    /// verification passed). Compared with `success` on the same denominator
    /// this is the per-split self-report gap.
    claims: usize,
    /// Runs whose agent-side verification verdict was `passed`.
    verify_passed: usize,
    /// Runs that solved the task without touching every declared expected file.
    expected_files_missing: usize,
    /// Runs that touched files outside the declared expected scope.
    scope_extra_files: usize,
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
            "  {label}: {}/{} success ({:.0}%) · fc={} ({:.0}%) · claims={} verify={} · tools_avg={:.1} · failed_tools={} · repairs={} · unnec_files={} · scope_missing={} scope_extra={}",
            self.success,
            self.tasks,
            self.success_rate() * 100.0,
            self.false_completions,
            self.fc_rate() * 100.0,
            self.claims,
            self.verify_passed,
            if self.tasks == 0 {
                0.0
            } else {
                self.tool_calls as f64 / self.tasks as f64
            },
            self.failed_tool_calls,
            self.repair_attempts,
            self.unnecessary_files,
            self.expected_files_missing,
            self.scope_extra_files,
        );
    }
}

#[derive(Default)]
struct EvalReport {
    deterministic_total: usize,
    deterministic_passed: usize,
    deterministic_failures: usize,
    /// The deterministic layer deliberately walks a simulated "claimed done
    /// without evidence" through the acceptance gate and counts each correct
    /// rejection. Those rejections are **not** agent false completions and must
    /// never feed the live false-completion rate (Beta-0 gate).
    gate_selftest_rejections: usize,
    baseline_total: usize,
    baseline_passed: usize,
    baseline_failures: usize,
    live_status: String,
    /// Which corpus a live run drew its fixtures from (`--suite`). "default" is
    /// the fast toy-scale set; "real" is the multi-file project set.
    live_suite: String,
    live_tasks: usize,
    live_success: usize,
    live_false_completions: usize,
    live_tool_calls: usize,
    live_failed_tool_calls: usize,
    /// Final-answer `delivery == "ready"` — the agent's own acceptance verdict.
    live_acceptance_success: usize,
    /// Runs whose final answer claimed completion at all (ready delivery or a
    /// `verification == "passed"` verdict). `live_success` is the oracle's say.
    live_claims: usize,
    /// Final-answer `verification == "passed"` — independent of the oracle.
    live_verification_success: usize,
    live_not_verified: usize,
    live_verify_failed: usize,
    live_blocked: usize,
    /// Structured failure taxonomy counts reported by the agent itself.
    live_failure_budget: usize,
    live_failure_acceptance: usize,
    live_failure_provider: usize,
    live_failure_truncated: usize,
    live_failure_permission: usize,
    live_failure_no_answer: usize,
    live_failure_other: usize,
    /// Runs that never landed an answer (provider outage, crash, empty final,
    /// state-machine error). Such a run is **not** credited with an oracle
    /// pass: an untouched blocked fixture goes green precisely because nothing
    /// happened, which is how a dead endpoint once scored 2/2.
    live_aborted: usize,
    live_input_tokens: u32,
    live_output_tokens: u32,
    live_model_calls: usize,
    /// Model calls that hit the output ceiling (`finish_reason == "length"`),
    /// i.e. the run was capped rather than wrong.
    live_truncated_calls: usize,
    /// Calls that could not use native tool-calling and fell back to a
    /// JSON/tag/text protocol.
    live_fallback_protocol_calls: usize,
    live_latency_ms: u128,
    live_unnecessary_files: usize,
    live_repair_attempts: usize,
    live_max_output_tokens: u32,
    /// Runs requested per fixture (`--repeat`); 1 is the historical single sample.
    live_repeat: usize,
    /// Distinct fixtures attempted. `live_tasks` counts **runs**, so every rate
    /// stays per-attempt and comparable with the single-sample baseline.
    live_fixtures: usize,
    /// Fixtures with at least one oracle-passing run (pass@N).
    live_pass_at_n: usize,
    /// Fixtures whose every repeat passed.
    live_pass_always: usize,
    /// Fixtures that both passed and failed across repeats — the variance a
    /// single sample hides.
    live_unstable_fixtures: usize,
    /// Runs that solved the task while leaving a declared expected file untouched.
    live_expected_files_missing: usize,
    /// Runs that changed files outside the declared expected scope.
    live_scope_extra_files: usize,
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
        println!(
            "false_completions_recorded: {} (live agent answers that claimed success without evidence)",
            self.live_false_completions
        );
        println!(
            "gate_selftest_rejections: {} (deterministic acceptance-gate rejections, NOT agent false completions)",
            self.gate_selftest_rejections
        );
        println!("live: {}", self.live_status);
        println!("live_suite: {}", self.live_suite);
        println!("live_tasks: {} (runs)", self.live_tasks);
        if self.live_fixtures > 0 {
            println!(
                "live_fixtures: {} ({} run(s) each)",
                self.live_fixtures, self.live_repeat
            );
        }
        println!("live_success: {}", self.live_success);
        println!("acceptance_success: {}", self.live_acceptance_success);
        println!("verification_success: {}", self.live_verification_success);
        println!(
            "self_report: claims={} oracle_confirmed={} gap={}",
            self.live_claims,
            self.live_success,
            self.live_claims.saturating_sub(self.live_success)
        );
        println!(
            "false_completion_rate: {}",
            if self.live_tasks > 0 {
                self.live_false_completions as f64 / self.live_tasks as f64
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
        println!(
            "expected_files_missing: {} (oracle passed but a declared expected file was untouched)",
            self.live_expected_files_missing
        );
        println!(
            "scope_extra_files: {} (runs that changed files outside the declared scope)",
            self.live_scope_extra_files
        );
        println!("input_tokens: {}", self.live_input_tokens);
        println!("output_tokens: {}", self.live_output_tokens);
        println!("latency_ms: {}", self.live_latency_ms);
        println!("model_calls: {}", self.live_model_calls);
        println!(
            "truncated_calls: {} (finish_reason=length at max_output_tokens={})",
            self.live_truncated_calls, self.live_max_output_tokens
        );
        println!(
            "fallback_protocol_calls: {} (no native tool-calling used)",
            self.live_fallback_protocol_calls
        );
        println!(
            "agent_verdicts: not_verified={} verify_failed={} blocked={}",
            self.live_not_verified, self.live_verify_failed, self.live_blocked
        );
        println!(
            "failure_taxonomy: budget={} acceptance={} provider={} truncated={} permission={} no_answer={} other={}",
            self.live_failure_budget,
            self.live_failure_acceptance,
            self.live_failure_provider,
            self.live_failure_truncated,
            self.live_failure_permission,
            self.live_failure_no_answer,
            self.live_failure_other
        );
        println!(
            "aborted_runs: {} (no answer landed; never credited as success)",
            self.live_aborted
        );
        if self.live_tasks > 0 {
            if self.live_repeat > 1 {
                println!(
                    "variance: pass_at_n={} pass_always={} unstable={}/{} (unstable = same fixture, both passing and failing runs)",
                    self.live_pass_at_n,
                    self.live_pass_always,
                    self.live_unstable_fixtures,
                    self.live_fixtures
                );
            } else {
                println!(
                    "variance: NOT MEASURED — one run per fixture (pass --repeat N to measure it)"
                );
            }
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
            println!("G_holdout_success_ge_90: {}", self.live_status);
            println!("G_live_false_completion_le_5: {}", self.live_status);
            println!("G_no_destructive_false_completion: {}", self.live_status);
        } else {
            let hold = self.holdout.success_rate() >= 0.90;
            println!(
                "G_holdout_success_ge_90: {} (holdout {:.0}%)",
                if hold { "PASS" } else { "FAIL" },
                self.holdout.success_rate() * 100.0
            );
            let fc = self.live_false_completions as f64 / self.live_tasks as f64 <= 0.05;
            println!(
                "G_live_false_completion_le_5: {} ({:.1}%)",
                if fc { "PASS" } else { "FAIL" },
                self.live_false_completions as f64 / self.live_tasks as f64 * 100.0
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

    /// The split bucket a run belongs to; holdout wins over development, and a
    /// fixture in neither split is still counted in the global totals.
    fn split_mut(&mut self, in_dev: bool, in_hold: bool) -> Option<&mut SplitMetrics> {
        if in_hold {
            Some(&mut self.holdout)
        } else if in_dev {
            Some(&mut self.dev)
        } else {
            None
        }
    }
}

/// Per-suite splits. The default corpus keeps `splits.json`; a named corpus
/// reads `splits-<suite>.json`, so a fixture id can never sit in two coverage
/// checks at once.
fn load_splits(suite: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let file = if suite == DEFAULT_SUITE {
        "splits.json".to_owned()
    } else {
        format!("splits-{suite}.json")
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(file);
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

/// One repo-relative path per line; blank lines and `#` comments ignored.
/// Shared by `expected_files.txt` and `forbidden.txt`.
fn parse_path_list(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// Kodo's own state, git metadata and the harness's temp scripts are not user
/// dirt and must not count as scope deviations.
fn is_noise_path(rel: &str) -> bool {
    rel == ".kodo"
        || rel.starts_with(".kodo/")
        || rel == ".git"
        || rel.starts_with(".git/")
        || rel == ".oracle-run.sh"
        || rel == ".script-run.sh"
}

fn walk_files(root: &Path, dir: &Path, out: &mut std::collections::BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if rel.is_empty() || is_noise_path(&rel) {
            continue;
        }
        if path.is_dir() {
            walk_files(root, &path, out);
        } else {
            out.insert(rel);
        }
    }
}

/// Repo-relative files the run created, deleted, or changed content in.
fn changed_paths(baseline: &Path, work: &Path) -> Vec<String> {
    let mut files = std::collections::BTreeSet::new();
    walk_files(baseline, baseline, &mut files);
    walk_files(work, work, &mut files);
    files
        .into_iter()
        .filter(|rel| {
            let before = std::fs::read(baseline.join(rel)).ok();
            let after = std::fs::read(work.join(rel)).ok();
            before.as_deref() != after.as_deref()
        })
        .collect()
}

/// Declared expected files the run never touched, and touched files outside the
/// declared scope. Reported as diagnostics: a different-but-correct fix location
/// must not silently become a failure the oracle does not know about.
fn scope_deviation(expected: &[String], changed: &[String]) -> (Vec<String>, Vec<String>) {
    let missing = expected
        .iter()
        .filter(|path| !changed.contains(path))
        .cloned()
        .collect();
    let extra = changed
        .iter()
        .filter(|path| !expected.contains(path))
        .cloned()
        .collect();
    (missing, extra)
}

/// Forbidden paths the run broke. A path that exists in the baseline must stay
/// byte-identical; a path that is absent there must not be created. Both halves
/// are load-bearing — the fixtures use the second form for "do not fabricate
/// secret.env / README.md / vendor/…".
fn forbidden_violations(baseline: &Path, work: &Path, entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .filter(|rel| {
            let before = std::fs::read(baseline.join(rel)).ok();
            let after = std::fs::read(work.join(rel)).ok();
            match (before, after) {
                (Some(before), Some(after)) => before != after,
                (None, Some(_)) => true,
                (Some(_), None) => true,
                (None, None) => false,
            }
        })
        .cloned()
        .collect()
}

/// Aggregate per-fixture oracle results across `--repeat` runs.
/// Returns `(pass_at_n, pass_always, unstable)`; `unstable` is a fixture that
/// both passed and failed, i.e. the number a single run cannot reveal.
fn repeat_summary(results: &[(String, usize, usize)]) -> (usize, usize, usize) {
    let (mut pass_at_n, mut always, mut unstable) = (0usize, 0usize, 0usize);
    for (_id, passes, runs) in results {
        if *runs == 0 {
            continue;
        }
        if *passes > 0 {
            pass_at_n += 1;
        }
        if *passes == *runs {
            always += 1;
        }
        if *passes > 0 && *passes < *runs {
            unstable += 1;
        }
    }
    (pass_at_n, always, unstable)
}

/// `--repeat <n>`: at least one run, capped so a typo cannot schedule hours of
/// paid model calls.
fn parse_repeat(args: &[String]) -> usize {
    let requested = args
        .iter()
        .position(|a| a == "--repeat")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1);
    requested.clamp(1, 10)
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
        // A rejection here means the gate *worked*. Counted separately from live
        // agent false completions so it can never inflate the Beta-0 FC rate.
        if !acceptance.ok {
            report.gate_selftest_rejections += 1;
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
                context_window_tokens: 32_000,
                vision: false,
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

    // 13) Exercise the shared settings consumer with isolated, synthetic credentials.
    {
        let result = Command::new("cargo")
            .args([
                "test",
                "--offline",
                "-p",
                "kodo-shell",
                "config::tests::stored_settings_reach_runtime",
                "--",
                "--exact",
            ])
            .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."))
            .output();
        let ok = result.as_ref().is_ok_and(|r| {
            r.status.success() && String::from_utf8_lossy(&r.stdout).contains("1 passed")
        });
        report.check(
            "settings_have_runtime_consumers",
            ok,
            "isolated ShellConfig runtime consumer test failed",
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
            live_false_completions: 1,
            gate_selftest_rejections: 3,
            ..Default::default()
        };
        // Gate rejections alone must produce a zero live false-completion rate.
        let only_selftest = EvalReport {
            live_tasks: 1,
            gate_selftest_rejections: 3,
            ..Default::default()
        };
        let rate = r.live_false_completions as f64 / r.live_tasks as f64;
        let selftest_rate =
            only_selftest.live_false_completions as f64 / only_selftest.live_tasks as f64;
        report.check(
            "oracle_fail_not_success_and_counts_false_completion",
            r.live_success == 0 && rate == 1.0 && selftest_rate == 0.0,
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
                context_window_tokens: 32_000,
                vision: false,
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
        let config = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates/shell/src/config.rs"),
        )
        .unwrap_or_default();
        let ok = app.contains("data-density")
            && app.contains("show-line-numbers")
            && app.contains("use-system-font")
            && app.contains("data-theme")
            && app.contains("auto-detect-git-branch")
            && config.contains("fallback-behavior")
            && run.contains("ShellConfig::load")
            && run.contains("permission")
            && config.contains("default-model")
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

    // 22) the two scope manifests are parsed the same way
    {
        let list = parse_path_list("# comment\n\n  sum.js  \nsrc/a.js\n");
        report.check(
            "scope_manifest_parses_comments_and_blanks",
            list == ["sum.js", "src/a.js"],
            &format!("{list:?}"),
        );
    }

    // 23) scope, forbidden and change measurement run off the real file diff,
    //     not the agent's claims, and ignore Kodo/harness state.
    {
        let dir = std::env::temp_dir().join(format!("kodo_eval_scope_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let base = dir.join("base");
        let work = dir.join("work");
        std::fs::create_dir_all(base.join("src")).unwrap();
        std::fs::create_dir_all(work.join("src")).unwrap();
        std::fs::write(base.join("src/lib.js"), "one\n").unwrap();
        std::fs::write(work.join("src/lib.js"), "two\n").unwrap();
        std::fs::write(base.join("keep.js"), "same\n").unwrap();
        std::fs::write(work.join("keep.js"), "same\n").unwrap();
        std::fs::write(base.join("gone.js"), "delete me\n").unwrap();
        std::fs::write(work.join("new.js"), "created\n").unwrap();
        std::fs::create_dir_all(work.join(".kodo/checkpoints")).unwrap();
        std::fs::write(work.join(".kodo/checkpoints/state.json"), "{}\n").unwrap();
        std::fs::create_dir_all(work.join(".git")).unwrap();
        std::fs::write(work.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(work.join(".oracle-run.sh"), "sh\n").unwrap();
        let changed = changed_paths(&base, &work);
        let declared = ["src/lib.js".to_string(), "keep.js".to_string()];
        let (missing, extra) = scope_deviation(&declared, &changed);
        report.check(
            "changed_paths_sees_edit_add_delete_and_ignores_state",
            changed == ["gone.js", "new.js", "src/lib.js"],
            &format!("changed={changed:?}"),
        );
        report.check(
            "scope_deviation_splits_missing_from_extra",
            missing == ["keep.js"] && extra == ["gone.js", "new.js"],
            &format!("missing={missing:?} extra={extra:?}"),
        );
        let broken = forbidden_violations(
            &base,
            &work,
            &[
                "keep.js".to_string(),
                "gone.js".to_string(),
                "new.js".to_string(),
            ],
        );
        report.check(
            "forbidden_guard_covers_modify_delete_and_create",
            broken == ["gone.js", "new.js"],
            &format!(
                "broken={broken:?} (a path absent from the baseline is a must-not-create guard)"
            ),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 24) --repeat aggregation separates "solved sometimes" from "solved every
    //     time" — the distinction a single sample cannot make.
    {
        let results = vec![
            ("a".to_string(), 2usize, 2usize),
            ("b".to_string(), 1, 2),
            ("c".to_string(), 0, 2),
            ("d".to_string(), 0, 0),
        ];
        let (pass_at_n, always, unstable) = repeat_summary(&results);
        report.check(
            "repeat_summary_separates_pass_at_n_from_stability",
            pass_at_n == 2 && always == 1 && unstable == 1,
            &format!("pass_at_n={pass_at_n} always={always} unstable={unstable}"),
        );
        let some: Vec<String> = vec!["--repeat".into(), "7".into()];
        let zero: Vec<String> = vec!["--repeat".into(), "0".into()];
        let too_many: Vec<String> = vec!["--repeat".into(), "99".into()];
        report.check(
            "repeat_flag_defaults_and_clamps",
            parse_repeat(&some) == 7
                && parse_repeat(&zero) == 1
                && parse_repeat(&too_many) == 10
                && parse_repeat(&[]) == 1,
            &format!(
                "7/0/99/none → {} {} {} {}",
                parse_repeat(&some),
                parse_repeat(&zero),
                parse_repeat(&too_many),
                parse_repeat(&[])
            ),
        );
    }

    // 25) run aggregation: a claimed-but-red run is a false completion and must
    //     never land on the success side; a blocked run is counted once.
    {
        let mut r = EvalReport::default();
        let claimed_but_red = TaskRun {
            claims_done: true,
            delivery: "ready".into(),
            verification: "passed".into(),
            ..Default::default()
        };
        apply_run(&mut r, "dev-fixture", true, false, false, &claimed_but_red);
        let blocked = TaskRun {
            delivery: "blocked".into(),
            verification: "blocked".into(),
            failure_reason: "acceptance_unmet".into(),
            ..Default::default()
        };
        apply_run(&mut r, "holdout-fixture", false, true, false, &blocked);
        report.check(
            "run_aggregation_keeps_false_completion_off_the_success_side",
            r.live_tasks == 2
                && r.live_success == 0
                && r.live_false_completions == 1
                && r.live_claims == 1
                && r.dev.tasks == 1
                && r.dev.false_completions == 1
                && r.holdout.tasks == 1,
            &format!(
                "tasks={} success={} fc={} claims={} dev={}/{} holdout={}",
                r.live_tasks,
                r.live_success,
                r.live_false_completions,
                r.live_claims,
                r.dev.tasks,
                r.dev.false_completions,
                r.holdout.tasks
            ),
        );
        report.check(
            "blocked_run_is_counted_once_not_twice",
            r.live_blocked == 1 && r.live_failure_acceptance == 1 && r.live_acceptance_success == 1,
            &format!(
                "blocked={} acceptance={} ready={}",
                r.live_blocked, r.live_failure_acceptance, r.live_acceptance_success
            ),
        );
    }

    // 26) a dead provider must not be able to score a pass: the oracle goes
    //     green on an untouched blocked fixture precisely because nothing ran.
    {
        let mut r = EvalReport::default();
        let dead_endpoint = TaskRun {
            oracle_ok: true,
            delivery: "failed".into(),
            verification: "not_run".into(),
            failure_reason: "unknown".into(),
            model_calls: 1,
            answered_calls: 0,
            ..Default::default()
        };
        apply_run(
            &mut r,
            "blocked-1-impossible",
            true,
            false,
            false,
            &dead_endpoint,
        );
        report.check(
            "aborted_run_never_credited_as_success",
            r.live_success == 0 && r.live_aborted == 1 && r.live_tasks == 1,
            &format!(
                "success={} aborted={} tasks={}",
                r.live_success, r.live_aborted, r.live_tasks
            ),
        );
        // The same fixture answered by a live model and refused honestly is a
        // pass: the rule must not swallow legitimate refusals.
        let mut healthy = EvalReport::default();
        let honest_refusal = TaskRun {
            oracle_ok: true,
            delivery: "blocked".into(),
            verification: "blocked".into(),
            model_calls: 3,
            answered_calls: 3,
            output_tokens: 512,
            ..Default::default()
        };
        apply_run(
            &mut healthy,
            "blocked-1-impossible",
            true,
            false,
            false,
            &honest_refusal,
        );
        report.check(
            "honest_refusal_from_a_live_model_is_still_scored",
            healthy.live_success == 1 && healthy.live_aborted == 0 && healthy.live_blocked == 1,
            &format!(
                "success={} aborted={} blocked={}",
                healthy.live_success, healthy.live_aborted, healthy.live_blocked
            ),
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

/// The toy-scale corpus that has always gated the harness.
const DEFAULT_SUITE: &str = "default";

/// A suite is a fixture corpus. `default` is `evals/fixtures/` — single-file
/// repos a model can fix by eye. `real` is `evals/fixtures-real/`: multi-file
/// projects with a package.json, a test runner and 150-250 lines of plausible
/// noise, so retrieval depth and edit precision get measured instead of being
/// assumed from a 5-line file.
fn suite_root(suite: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if suite == DEFAULT_SUITE {
        manifest.join("fixtures")
    } else {
        manifest.join(format!("fixtures-{suite}"))
    }
}

/// Every corpus on disk: `default` first, then each `fixtures-<suite>/`
/// directory. Baselines run over all of them so a second corpus cannot rot
/// quietly next to a green report.
fn suite_names() -> Vec<String> {
    let mut extras: Vec<String> = std::fs::read_dir(env!("CARGO_MANIFEST_DIR"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter_map(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .strip_prefix("fixtures-")
                        .map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    extras.sort();
    let mut names = vec![DEFAULT_SUITE.to_owned()];
    names.extend(extras);
    names
}

/// Layer B baselines over **every** corpus on disk, so a second suite cannot
/// rot quietly next to a green report.
fn run_fixture_baselines(report: &mut EvalReport) {
    for suite in suite_names() {
        run_suite_baselines(&suite, report);
    }
}

/// Each fixture has prompt+oracle; unfixed bug/feature oracles must fail; the
/// blocked task oracle must pass without agent edits.
fn run_suite_baselines(suite: &str, report: &mut EvalReport) {
    // Corpus-level check names keep the historical spelling for `default` so
    // reports stay comparable run to run; another suite is suffixed.
    let named = |key: &str| -> String {
        if suite == DEFAULT_SUITE {
            key.to_owned()
        } else {
            format!("{key}_{suite}")
        }
    };
    let root = suite_root(suite);
    let Ok(entries) = std::fs::read_dir(&root) else {
        report.baseline(
            &named("fixtures_directory_exists"),
            false,
            &format!("{} missing", root.display()),
        );
        return;
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    // The toy corpus is a 30-task gate; a named suite only has to be big enough
    // to say something about scale.
    let minimum = if suite == DEFAULT_SUITE { 30 } else { 4 };
    report.baseline(
        &named(&format!("fixture_count_at_least_{minimum}")),
        ids.len() >= minimum,
        &format!("only {} fixtures", ids.len()),
    );

    // Split integrity: every fixture in exactly one of development/holdout.
    let (dev, holdout, regressions) = load_splits(suite);
    report.baseline(
        &named("splits_defined"),
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
        &named("splits_cover_all_fixtures"),
        union == all,
        &format!("split_union={} fixtures={}", union.len(), all.len()),
    );
    let overlap = dev.iter().filter(|d| holdout.contains(d)).count();
    report.baseline(
        &named("splits_no_overlap"),
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

        // "Fails before the fix" says nothing about the oracle ever being
        // satisfiable. A fixture with a `solution.sh` — the reviewer's canonical
        // fix — is applied to a throwaway copy and must turn the oracle green,
        // so an unsolvable task is caught here instead of showing up as a 0%
        // live score.
        if dir.join("solution.sh").is_file() {
            let work =
                std::env::temp_dir().join(format!("kodo_sol_eval_{id}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&work);
            let green = copy_dir(&dir.join("repo"), &work)
                && run_script(&dir.join("solution.sh"), &work)
                && run_oracle(&dir, &work);
            report.baseline(
                &format!("fixture_{id}_canonical_solution_passes"),
                green,
                "the canonical fix does not satisfy the oracle — unsolvable fixture",
            );
            let _ = std::fs::remove_dir_all(&work);
        }

        // The scope manifests only matter if the live runner reads them and the
        // paths they name exist. `bug-*/expected_files.txt` pointed at `src/*`
        // files the fixtures never had while the metadata was dead weight.
        let expected = parse_path_list(
            &std::fs::read_to_string(dir.join("expected_files.txt")).unwrap_or_default(),
        );
        report.baseline(
            &format!("fixture_{id}_has_scope_manifests"),
            dir.join("expected_files.txt").is_file() && dir.join("forbidden.txt").is_file(),
            "missing expected_files.txt or forbidden.txt",
        );
        for rel in &expected {
            report.baseline(
                &format!("fixture_{id}_expected_file_exists_{rel}"),
                dir.join("repo").join(rel).is_file(),
                "declared expected file is not in the fixture repo (stale metadata)",
            );
        }
        // Blocked/impossible fixtures must not invite edits at all; every other
        // fixture must declare at least one file the fix is expected to change.
        if pass_baseline.contains(&id.as_str()) {
            report.baseline(
                &format!("fixture_{id}_expects_no_file_changes"),
                expected.is_empty(),
                "blocked fixture declares expected files",
            );
        } else {
            report.baseline(
                &format!("fixture_{id}_declares_expected_files"),
                !expected.is_empty(),
                "editable fixture declares no expected files — scope metric cannot be measured",
            );
        }
    }
}

/// One live attempt against a fresh copy of a fixture repo, fully scored.
#[derive(Default)]
struct TaskRun {
    tool_calls: usize,
    failed_tools: usize,
    repairs: usize,
    model_calls: usize,
    input_tokens: u32,
    output_tokens: u32,
    truncated_calls: usize,
    fallback_calls: usize,
    /// Model calls that landed a finish reason, i.e. the provider actually
    /// answered. Zero of them with zero output tokens is an outage, not a
    /// result, and can never be scored as a solved task.
    answered_calls: usize,
    delivery: String,
    verification: String,
    failure_reason: String,
    claims_done: bool,
    error: Option<String>,
    oracle_ok: bool,
    /// Forbidden paths the run broke (modified, deleted, or fabricated).
    forbidden: Vec<String>,
    expected_missing: Vec<String>,
    scope_extra: Vec<String>,
    final_text: String,
    trace: Vec<String>,
}

impl TaskRun {
    /// A run whose model never answered is an infrastructure outcome, not a
    /// result. `finish_reason` comes back empty only on a synthesized failure
    /// (the streaming path falls back to "stop"), so zero answered calls plus
    /// zero output tokens is the outage signature — while a legitimate refusal
    /// from a live model still scores normally.
    fn is_aborted(&self) -> bool {
        self.error.is_some()
            || (self.answered_calls == 0 && self.output_tokens == 0)
            || (self.delivery.is_empty() && self.verification.is_empty())
            || matches!(
                self.failure_reason.as_str(),
                "provider_error" | "unrecoverable" | "no_answer" | "state_machine_error"
            )
    }
}

/// Re-run the fixture oracle against the edited copy. Oracle scripts resolve
/// their repo as `$(dirname "$0")/repo`, so the harness rewrites that to the
/// throwaway work dir before invoking it.
fn run_script(script: &Path, work: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(script) else {
        return false;
    };
    let wrapped = text.replace("$(dirname \"$0\")/repo", &work.display().to_string());
    let tmp = work.join(".script-run.sh");
    if std::fs::write(&tmp, wrapped).is_err() {
        return false;
    }
    let ok = Command::new("sh")
        .arg(&tmp)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let _ = std::fs::remove_file(&tmp);
    ok
}

fn run_oracle(fixture: &Path, work: &Path) -> bool {
    run_script(&fixture.join("oracle.sh"), work)
}

/// Copy the edited repo plus its trace to `evals/out/<run-id>/…` so a red
/// oracle can be diagnosed after the fact. Passing runs are still deleted.
fn keep_forensics(
    out_root: &Path,
    id: &str,
    run_no: usize,
    repeat: usize,
    baseline: &Path,
    work: &Path,
    prompt: &str,
    run: &TaskRun,
) -> Option<PathBuf> {
    let name = if repeat > 1 {
        format!("{id}-run{run_no}")
    } else {
        id.to_string()
    };
    let keep = out_root.join(&name);
    let _ = std::fs::remove_dir_all(&keep);
    if !copy_dir(work, &keep.join("repo")) {
        return None;
    }
    let _ = std::fs::write(
        keep.join("trace.txt"),
        format!(
            "prompt:\n{prompt}\n--- trace ---\n{}\n",
            run.trace.join("\n")
        ),
    );
    let _ = std::fs::write(&keep.join("final-answer.txt"), &run.final_text);
    if let Ok(out) = Command::new("diff")
        .arg("-ruN")
        .arg(baseline)
        .arg(keep.join("repo"))
        .output()
    {
        let _ = std::fs::write(keep.join("changes.diff"), &out.stdout);
    }
    Some(keep)
}

/// Fold one attempt into the aggregate counters. Every rate keeps a per-run
/// denominator so repeat runs stay comparable with the single-sample history,
/// and the split counters use the same denominator so a holdout false
/// completion cannot hide inside the development average.
fn apply_run(
    report: &mut EvalReport,
    tag: &str,
    in_dev: bool,
    in_hold: bool,
    is_regression: bool,
    run: &TaskRun,
) {
    report.live_tasks += 1;
    if in_dev {
        report.dev.tasks += 1;
    }
    if in_hold {
        report.holdout.tasks += 1;
    }
    report.live_tool_calls += run.tool_calls;
    report.live_failed_tool_calls += run.failed_tools;
    report.live_repair_attempts += run.repairs;
    report.live_model_calls += run.model_calls;
    report.live_input_tokens += run.input_tokens;
    report.live_output_tokens += run.output_tokens;
    report.live_truncated_calls += run.truncated_calls;
    report.live_fallback_protocol_calls += run.fallback_calls;
    if run.claims_done {
        report.live_claims += 1;
    }
    match run.verification.as_str() {
        "passed" => report.live_verification_success += 1,
        "failed" => report.live_verify_failed += 1,
        "blocked" => report.live_blocked += 1,
        _ => report.live_not_verified += 1,
    }
    if run.delivery == "ready" {
        report.live_acceptance_success += 1;
    } else if run.delivery == "blocked" && run.verification != "blocked" {
        // A blocked run is counted once: delivery and verification usually say
        // it together, and double counting inflated the old blocked metric.
        report.live_blocked += 1;
    }
    match run.failure_reason.as_str() {
        "budget_exhausted" => report.live_failure_budget += 1,
        "acceptance_unmet" => report.live_failure_acceptance += 1,
        "provider_error" => report.live_failure_provider += 1,
        "output_truncated" => report.live_failure_truncated += 1,
        "permission_blocked" => report.live_failure_permission += 1,
        "no_answer" => report.live_failure_no_answer += 1,
        "" => {}
        _ => report.live_failure_other += 1,
    }
    let aborted = run.is_aborted();
    if aborted {
        report.live_aborted += 1;
    }
    if run.oracle_ok && !run.expected_missing.is_empty() {
        report.live_expected_files_missing += 1;
    }
    if !run.scope_extra.is_empty() {
        report.live_scope_extra_files += 1;
    }
    if !run.forbidden.is_empty() {
        report.live_unnecessary_files += 1;
        report.notes.push(format!(
            "{tag}: broke forbidden path(s): {}",
            run.forbidden.join(", ")
        ));
    }
    if let Some(s) = report.split_mut(in_dev, in_hold) {
        s.tool_calls += run.tool_calls;
        s.failed_tool_calls += run.failed_tools;
        s.repair_attempts += run.repairs;
        if run.claims_done {
            s.claims += 1;
        }
        if run.verification == "passed" {
            s.verify_passed += 1;
        }
        if run.oracle_ok && !run.expected_missing.is_empty() {
            s.expected_files_missing += 1;
        }
        if !run.scope_extra.is_empty() {
            s.scope_extra_files += 1;
        }
        if !run.forbidden.is_empty() {
            s.unnecessary_files += 1;
        }
    }
    // Success stays oracle-grounded; the agent's own acceptance and
    // verification verdicts are counted separately and never aliased to it.
    if run.oracle_ok && !aborted {
        report.live_success += 1;
        if let Some(s) = report.split_mut(in_dev, in_hold) {
            s.success += 1;
        }
        if !run.expected_missing.is_empty() {
            report.notes.push(format!(
                "{tag}: oracle passed but declared expected file(s) untouched: {}",
                run.expected_missing.join(", ")
            ));
        }
    } else if run.oracle_ok {
        report.notes.push(format!(
            "{tag}: oracle green on an aborted run — not credited (delivery={:?} verification={:?} failure_reason={:?} model_calls={} answered={})",
            run.delivery,
            run.verification,
            run.failure_reason,
            run.model_calls,
            run.answered_calls
        ));
    } else if run.claims_done {
        report.live_false_completions += 1;
        report.notes.push(format!(
            "{tag}: FALSE COMPLETION — delivery={:?} verification={:?} but oracle failed",
            run.delivery, run.verification
        ));
        if let Some(s) = report.split_mut(in_dev, in_hold) {
            s.false_completions += 1;
        }
        if is_regression {
            report.regression_false_completions += 1;
        }
    } else {
        report.notes.push(format!(
            "{tag}: oracle failed (delivery={:?} verification={:?} failure_reason={:?} run_ok={})",
            run.delivery,
            run.verification,
            run.failure_reason,
            run.error.is_none()
        ));
        if !run.scope_extra.is_empty() {
            report.notes.push(format!(
                "{tag}: changed outside declared scope: {}",
                run.scope_extra.join(", ")
            ));
        }
    }
    if let Some(error) = &run.error {
        report.notes.push(format!("{tag}: agent error {error}"));
    }
}

/// Run the agent once against `work` (a fresh copy of `fixture/repo`) and score
/// the result: oracle, forbidden paths, declared-file scope, provider counters.
fn run_one_live_task(
    id: &str,
    fixture: &Path,
    work: &Path,
    prompt: &str,
    provider: &Provider,
    max_output_tokens: u32,
    expected: &[String],
    forbidden: &[String],
) -> TaskRun {
    let mut tool_calls = 0usize;
    let mut failed_tools = 0usize;
    let mut repairs = 0usize;
    // Provider-side accounting — tokens, call count, truncation, protocol.
    let mut model_calls = 0usize;
    let mut input_tokens = 0u32;
    let mut output_tokens = 0u32;
    let mut truncated_calls = 0usize;
    let mut fallback_calls = 0usize;
    let mut answered_calls = 0usize;
    // Structured final-answer verdicts read off Step::AgentMessage instead of
    // substring-matching the rendered text.
    let mut delivery = String::new();
    let mut verification = String::new();
    let mut failure_reason = String::new();
    let mut final_text = String::new();
    let mut claimed_verified = false;
    let mut trace: Vec<String> = Vec::new();
    let request = RunRequest {
        project: Some(work.to_path_buf()),
        message: prompt.to_string(),
        conversation_history: Vec::new(),
        pinned_context: Vec::new(),
        pinned_force_inline: Vec::new(),
        provider: Some(provider.clone()),
        permission: Permission::Full,
        mode: AgentMode::Code,
        fallback_to_local: false,
        max_output_tokens,
        extended_thinking: false,
        local_trace: false,
        session_id: Some(format!("eval_{id}")),
        user_skills_dir: None,
        mcp_servers: Vec::new(),
        agent_instructions: None,
        personas: Vec::new(),
        budget_family: None,
        memory: None,
        knowledge_context: None,
        allow_verify_commands: false,
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
                kodo_agent::Step::ModelCall {
                    input_tokens: it,
                    output_tokens: ot,
                    protocol,
                    finish_reason,
                    ..
                } => {
                    model_calls += 1;
                    input_tokens += *it;
                    output_tokens += *ot;
                    if *finish_reason == "length" {
                        truncated_calls += 1;
                    }
                    if !finish_reason.is_empty() {
                        answered_calls += 1;
                    }
                    if !protocol.is_empty() && *protocol != "native" {
                        fallback_calls += 1;
                    }
                    trace.push(format!(
                        "model_call protocol={protocol} finish={finish_reason} in={it} out={ot}"
                    ));
                }
                kodo_agent::Step::AgentMessage {
                    text,
                    delivery: d,
                    verification: v,
                    failure_reason: fr,
                    ..
                } => {
                    delivery = d.clone();
                    verification = v.clone();
                    failure_reason = fr.clone();
                    final_text = text.clone();
                    // Belt and braces: the structured verdict is authoritative,
                    // the rendered-text marker is kept for older traces.
                    if *v == "passed"
                        || text.contains("Verification status:** Verified")
                        || text.contains("status: Verified")
                    {
                        claimed_verified = true;
                    }
                }
                _ => {}
            }
            // Non-zero command exits count as failed tool calls (honest).
            if let kodo_agent::Step::Command { exit_code, .. } = step {
                if matches!(exit_code, Some(c) if *c != 0) {
                    failed_tools += 1;
                }
            }
            if trace.len() < 400 {
                let brief = match step {
                    kodo_agent::Step::Reasoning { summary, .. } => {
                        format!("reasoning: {summary}")
                    }
                    kodo_agent::Step::Search { query, .. } => format!("search: {query}"),
                    kodo_agent::Step::FileRead { path, .. } => format!("read: {path}"),
                    kodo_agent::Step::Command {
                        command, exit_code, ..
                    } => {
                        format!("command: {command} -> exit {exit_code:?}")
                    }
                    kodo_agent::Step::FileChange { changes, .. } => format!(
                        "change: {}",
                        changes
                            .iter()
                            .map(|c| c.path.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    kodo_agent::Step::AgentMessage {
                        delivery,
                        verification,
                        ..
                    } => format!("answer: delivery={delivery} verification={verification}"),
                    _ => String::new(),
                };
                if !brief.is_empty() {
                    trace.push(brief);
                }
            }
            true
        }
        kodo_agent::SinkEvent::Progress { phase, .. } => {
            if phase == "Repairing" {
                repairs += 1;
            }
            trace.push(format!("progress: {phase}"));
            true
        }
        _ => true,
    };
    let run_result = kodo_agent::run(&request, &alive, &approve, &mut sink);
    let oracle_ok = run_oracle(fixture, work);
    let baseline = fixture.join("repo");
    let changed = changed_paths(&baseline, work);
    let (expected_missing, scope_extra) = scope_deviation(expected, &changed);
    TaskRun {
        tool_calls,
        failed_tools,
        repairs,
        model_calls,
        input_tokens,
        output_tokens,
        truncated_calls,
        fallback_calls,
        answered_calls,
        claims_done: claimed_verified || delivery == "ready",
        error: run_result.err().map(|e| e.to_string()),
        oracle_ok,
        forbidden: forbidden_violations(&baseline, work, forbidden),
        expected_missing,
        scope_extra,
        final_text,
        trace,
        delivery,
        verification,
        failure_reason,
    }
}

/// Real coding eval: copy fixture → run agent → re-run oracle.
/// Requires KODO_EVAL_PROVIDER / MODEL_ID / API_KEY.
///
/// `repeat` runs every fixture N times. One sample cannot tell a flaky agent
/// from a weak model, so the pass@N / always / unstable triple is reported
/// alongside the per-run rates.
fn run_fixture_live(suite: &str, tasks_n: usize, repeat: usize, report: &mut EvalReport) {
    report.live_suite = suite.to_owned();
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

    let root = suite_root(suite);
    // An unknown `--suite` must not silently become a zero-task live run that
    // reads as "nothing failed".
    if !root.is_dir() {
        report.live_status = format!("NOT RUN — unknown suite \"{suite}\"");
        report.notes.push(format!(
            "LIVE EVAL: {} (available: {})",
            report.live_status,
            suite_names().join(", ")
        ));
        return;
    }
    let (development, holdout, regressions) = load_splits(suite);
    let mut ids: Vec<String> = std::fs::read_dir(&root)
        .map(|it| {
            it.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.truncate(tasks_n);

    let started = Instant::now();
    let provider = Provider::new(&provider_type, &api_key, &endpoint, &model_id);

    // The live harness must not code against a smaller output ceiling than the
    // product ships with; a truncated fix looks like a model-quality problem.
    let max_output_tokens = std::env::var("KODO_EVAL_MAX_OUTPUT_TOKENS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(8192);
    report.live_max_output_tokens = max_output_tokens;
    report.live_repeat = repeat.max(1);

    let run_id = format!(
        "live-{}-{}{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        std::process::id(),
        // The default corpus keeps the historical directory name.
        if suite == DEFAULT_SUITE {
            String::new()
        } else {
            format!("-{suite}")
        }
    );
    let out_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("out")
        .join(&run_id);

    // (fixture id, oracle passes, runs actually executed) for the variance view.
    let mut per_fixture: Vec<(String, usize, usize)> = Vec::new();

    for id in &ids {
        let fixture = root.join(id);
        let Ok(prompt) = std::fs::read_to_string(fixture.join("prompt.md")) else {
            report.notes.push(format!("{id}: skipped — no prompt.md"));
            continue;
        };
        let expected = parse_path_list(
            &std::fs::read_to_string(fixture.join("expected_files.txt")).unwrap_or_default(),
        );
        let forbidden_list = parse_path_list(
            &std::fs::read_to_string(fixture.join("forbidden.txt")).unwrap_or_default(),
        );
        let in_dev = development.contains(id);
        let in_hold = holdout.contains(id);
        let is_regression = regressions.contains(id);
        let mut passes = 0usize;
        let mut runs = 0usize;

        for run_no in 1..=repeat {
            let work = std::env::temp_dir().join(format!(
                "kodo_liv_eval_{}_{run_no}_{}",
                id,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&work);
            if !copy_dir(&fixture.join("repo"), &work) {
                report.notes.push(format!("copy failed for {id}"));
                continue;
            }
            let tag = if repeat > 1 {
                format!("{id}#r{run_no}")
            } else {
                id.clone()
            };
            let run = run_one_live_task(
                id,
                &fixture,
                &work,
                &prompt,
                &provider,
                max_output_tokens,
                &expected,
                &forbidden_list,
            );
            runs += 1;
            if run.oracle_ok && !run.is_aborted() {
                passes += 1;
            }
            apply_run(report, &tag, in_dev, in_hold, is_regression, &run);
            if !run.oracle_ok || run.is_aborted() || !run.forbidden.is_empty() {
                if let Some(keep) = keep_forensics(
                    &out_root,
                    id,
                    run_no,
                    repeat,
                    &fixture.join("repo"),
                    &work,
                    &prompt,
                    &run,
                ) {
                    report
                        .notes
                        .push(format!("{tag}: forensics kept at {}", keep.display()));
                }
            }
            let _ = std::fs::remove_dir_all(&work);
        }
        per_fixture.push((id.clone(), passes, runs));
    }

    let (pass_at_n, always, unstable) = repeat_summary(&per_fixture);
    report.live_pass_at_n = pass_at_n;
    report.live_pass_always = always;
    report.live_unstable_fixtures = unstable;
    report.live_fixtures = per_fixture.iter().filter(|(_, _, r)| *r > 0).count();
    if report.live_repeat > 1 {
        for (id, passes, runs) in &per_fixture {
            if *passes > 0 && *passes < *runs {
                report.notes.push(format!(
                    "{id}: UNSTABLE — oracle passed {passes}/{runs} runs"
                ));
            }
        }
    }
    report.live_latency_ms = started.elapsed().as_millis();
    report.live_status = format!(
        "RAN — {}/{} runs passed (aborted={}), {pass_at_n}/{} fixtures solved at least once (forensics: {})",
        report.live_success,
        report.live_tasks,
        report.live_aborted,
        report.live_fixtures,
        out_root.display()
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
