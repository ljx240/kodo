//! Offline agent capability benchmark — 9 required scenarios.
//!
//! Every scenario drives the real `kodo_agent::run` loop with a **fake
//! provider** (`template = "fake"`): responses come from a local JSONL script,
//! never the network. Reports land in `target/agent-benchmark/report.json`.
//!
//! Run:
//! ```bash
//! cargo test -p kodo-agent --test agent_benchmark -- --nocapture
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};

use kodo_agent::{Permission, Provider, RunRequest, SinkEvent, Step, StepKind};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Completed,
    Cancelled,
    Failed(String),
}

struct ScenarioResult {
    id: &'static str,
    name: &'static str,
    passed: bool,
    detail: String,
    outcome: Outcome,
}

struct Harness {
    root: PathBuf,
    results: Vec<ScenarioResult>,
}

static REPORT_LOCK: Mutex<()> = Mutex::new(());
static REPORT_RESET: Once = Once::new();

impl Harness {
    fn new(tag: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("agent-benchmark")
            .join(tag);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("benchmark tmpdir");
        Self {
            root,
            results: Vec::new(),
        }
    }

    fn project(&self, name: &str) -> PathBuf {
        let dir = self.root.join(name);
        fs::create_dir_all(&dir).expect("project dir");
        // Isolate from any parent repo so `git status` / context scan stay local.
        let _ = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status();
        dir
    }

    /// Commit fixture files so a later `git status` only shows agent edits.
    fn commit_fixture(&self, project: &Path) {
        let run = |args: &[&str]| {
            let _ = std::process::Command::new("git")
                .args(args)
                .current_dir(project)
                .env("GIT_AUTHOR_NAME", "bench")
                .env("GIT_AUTHOR_EMAIL", "bench@local")
                .env("GIT_COMMITTER_NAME", "bench")
                .env("GIT_COMMITTER_EMAIL", "bench@local")
                .status();
        };
        run(&["add", "-A"]);
        run(&["commit", "-q", "-m", "fixture", "--allow-empty"]);
    }

    #[allow(dead_code)] // reserved for future scripted fixtures
    fn write_script(&self, scenario: &str, lines: &[Value]) -> String {
        let path = self.root.join(format!("{scenario}.script.jsonl"));
        let body: String = lines
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{body}\n")).expect("write script");
        path.to_string_lossy().into_owned()
    }

    fn record(
        &mut self,
        id: &'static str,
        name: &'static str,
        passed: bool,
        detail: String,
        outcome: Outcome,
    ) {
        self.results.push(ScenarioResult {
            id,
            name,
            passed,
            detail,
            outcome,
        });
    }

    fn write_report(&self) {
        let _guard = REPORT_LOCK.lock().expect("benchmark report lock");
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("agent-benchmark");
        // Also mirror under target/agent-benchmark for CI artifact upload.
        let out_dirs = [
            dir.clone(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/agent-benchmark"),
        ];
        REPORT_RESET.call_once(|| {
            for output_dir in &out_dirs {
                let _ = fs::remove_file(output_dir.join("report.json"));
            }
        });

        let mut scenarios = out_dirs
            .iter()
            .find_map(|output_dir| {
                let text = fs::read_to_string(output_dir.join("report.json")).ok()?;
                serde_json::from_str::<Value>(&text)
                    .ok()?
                    .get("scenarios")
                    .and_then(Value::as_array)
                    .cloned()
            })
            .unwrap_or_default();
        for result in &self.results {
            let scenario = json!({
                "id": result.id,
                "name": result.name,
                "passed": result.passed,
                "detail": result.detail,
                "outcome": match &result.outcome {
                    Outcome::Completed => "completed",
                    Outcome::Cancelled => "cancelled",
                    Outcome::Failed(_) => "failed",
                },
            });
            if let Some(existing) = scenarios
                .iter_mut()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(result.id))
            {
                *existing = scenario;
            } else {
                scenarios.push(scenario);
            }
        }
        scenarios.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
        let arr = scenarios;
        let passed = arr.iter().filter(|v| v["passed"] == true).count();
        let report = json!({
            "suite": "agent-benchmark",
            "provider": "fake",
            "total": arr.len(),
            "passed": passed,
            "failed": arr.len() - passed,
            "scenarios": arr,
        });
        for d in &out_dirs {
            let _ = fs::create_dir_all(d);
            let _ = fs::write(
                d.join("report.json"),
                serde_json::to_string_pretty(&report).unwrap(),
            );
        }
    }
}

struct Captured {
    answer: String,
    checks: Vec<String>,
    cancelled: bool,
    steps_seen: Vec<String>,
    file_changes: Vec<String>,
}

fn tool_json(calls: Value) -> Value {
    json!({ "text": json!(calls).to_string() })
}

fn run_scenario(
    project: &Path,
    message: &str,
    script_lines: &[Value],
    permission: Permission,
    opts: ScenarioOpts,
) -> Result<Captured, String> {
    // Keep the script outside the project so git status stays clean for
    // read-only scenarios (script is not part of the fixture tree).
    let script_path = project.parent().unwrap_or(project).join(format!(
        ".bench-script-{}.jsonl",
        project.file_name().unwrap_or_default().to_string_lossy()
    ));
    let body: String = script_lines
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&script_path, format!("{body}\n")).map_err(|e| e.to_string())?;

    let provider = Provider::new(
        "fake",
        "fake-key",
        script_path.to_string_lossy(),
        "fake-model",
    );

    let request = RunRequest {
        project: project.to_path_buf(),
        message: message.to_owned(),
        pinned_context: Vec::new(),
        provider: Some(provider),
        permission,
        fallback_to_local: true,
        max_output_tokens: 1024,
        extended_thinking: false,
        session_id: None,
    };

    let cancel_after = Arc::new(AtomicUsize::new(
        opts.cancel_after_steps.unwrap_or(usize::MAX),
    ));
    let step_count = Arc::new(AtomicUsize::new(0));
    let cancelled_flag = Arc::new(AtomicBool::new(false));

    let alive_steps = Arc::clone(&step_count);
    let alive_cancel = Arc::clone(&cancelled_flag);
    let alive_cap = Arc::clone(&cancel_after);
    let alive = move || {
        let n = alive_steps.load(Ordering::SeqCst);
        if n >= alive_cap.load(Ordering::SeqCst) {
            alive_cancel.store(true, Ordering::SeqCst);
            false
        } else {
            true
        }
    };

    let deny_writes = opts.deny_writes;
    let deny_commands = opts.deny_commands;
    let approve = move |kind: StepKind, _label: &str| match kind {
        StepKind::FileChange => !deny_writes,
        StepKind::Command => !deny_commands,
        _ => true,
    };

    let mut answer = String::new();
    let mut checks = Vec::new();
    let mut steps_seen = Vec::new();
    let mut file_changes = Vec::new();

    {
        let step_count = Arc::clone(&step_count);
        let mut emit = |event: SinkEvent| match event {
            SinkEvent::Started { step } => {
                steps_seen.push(preview(&step));
                step_count.fetch_add(1, Ordering::SeqCst);
                true
            }
            SinkEvent::Finished { step, .. } => {
                if let Step::AgentMessage {
                    text, checks: c, ..
                } = &step
                {
                    answer = text.clone();
                    checks = c.clone();
                }
                if let Step::FileChange { changes } = &step {
                    for ch in changes {
                        file_changes.push(ch.path.clone());
                    }
                }
                true
            }
            _ => true,
        };

        kodo_agent::run(&request, &alive, &approve, &mut emit).map_err(|e| e.to_string())?;
    }

    let cancelled = cancelled_flag.load(Ordering::SeqCst)
        || answer.starts_with("已取消")
        || checks.iter().any(|c| c.contains("用户停止"));

    Ok(Captured {
        answer,
        checks,
        cancelled,
        steps_seen,
        file_changes,
    })
}

#[derive(Default)]
struct ScenarioOpts {
    deny_writes: bool,
    deny_commands: bool,
    cancel_after_steps: Option<usize>,
}

fn preview(step: &Step) -> String {
    match step {
        Step::Reasoning { summary, .. } => format!("reasoning:{summary}"),
        Step::Search { query, .. } => format!("search:{query}"),
        Step::FileRead { path, .. } => format!("read:{path}"),
        Step::Command { command, .. } => format!("command:{command}"),
        Step::ModelCall { model, .. } => format!("model:{model}"),
        Step::FileChange { changes } => {
            let paths: Vec<_> = changes.iter().map(|c| c.path.clone()).collect();
            format!("write:{}", paths.join(","))
        }
        Step::AgentMessage { .. } => "message".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

#[test]
fn benchmark_01_read_only_question() {
    let mut h = Harness::new("01");
    let project = h.project("01-read-only");
    fs::write(
        project.join("README.md"),
        "# Demo\n\nLayout: crates/ + apps/.\n",
    )
    .unwrap();
    h.commit_fixture(&project);
    let script =
        vec![json!({ "text": "The repo has crates/core and crates/agent.\n- read README" })];

    let cap = run_scenario(
        &project,
        "What is the repository layout?",
        &script,
        Permission::Auto,
        ScenarioOpts::default(),
    );

    let (passed, detail, outcome) = match cap {
        Ok(c) => {
            let ok = !c.cancelled
                && c.answer.contains("Verification status")
                && c.file_changes.is_empty();
            (
                ok,
                format!(
                    "cancelled={} answer_head={:?} changes={:?} checks={:?}",
                    c.cancelled,
                    c.answer.chars().take(120).collect::<String>(),
                    c.file_changes,
                    c.checks
                ),
                Outcome::Completed,
            )
        }
        Err(e) => (false, e.clone(), Outcome::Failed(e)),
    };
    h.record("01", "read-only question", passed, detail.clone(), outcome);
    h.write_report();
    assert!(
        passed,
        "read-only scenario must complete without file writes: {detail}"
    );
}

#[test]
fn benchmark_02_single_file_bug() {
    let mut h = Harness::new("02");
    let project = h.project("02-single-bug");
    fs::write(
        project.join("app.js"),
        "function add(a, b) {\n  return a - b;\n}\n",
    )
    .unwrap();
    fs::write(
        project.join("package.json"),
        r#"{"name":"t","scripts":{"test":"node -e \"process.exit(0)\"","typecheck":"node -e \"process.exit(0)\"","build":"node -e \"process.exit(0)\"","lint":"node -e \"process.exit(0)\""}}"#,
    )
    .unwrap();

    let script = vec![
        tool_json(json!([{
            "id": "w1",
            "name": "apply_patch",
            "arguments": {
                "path": "app.js",
                "old": "return a - b;",
                "new": "return a + b;"
            }
        }])),
        json!({ "text": "Fixed add().\n- patched app.js\n- tests pass" }),
    ];

    let cap = run_scenario(
        &project,
        "Fix the single-file bug in add()",
        &script,
        Permission::Full,
        ScenarioOpts::default(),
    );

    let content = fs::read_to_string(project.join("app.js")).unwrap_or_default();
    let (passed, detail, outcome) = match cap {
        Ok(c) => {
            let ok = content.contains("a + b") && c.file_changes.iter().any(|p| p == "app.js");
            (
                ok,
                format!(
                    "changes={:?} has_plus={}",
                    c.file_changes,
                    content.contains("a + b")
                ),
                Outcome::Completed,
            )
        }
        Err(e) => (false, e.clone(), Outcome::Failed(e)),
    };
    h.record("02", "single-file bug", passed, detail, outcome);
    h.write_report();
    assert!(passed, "single-file bug must be patched");
}

#[test]
fn benchmark_03_multi_file_bug() {
    let mut h = Harness::new("03");
    let project = h.project("03-multi-bug");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/a.js"), "module.exports = () => 1 - 1;\n").unwrap();
    fs::write(project.join("src/b.js"), "module.exports = () => 2 - 2;\n").unwrap();
    fs::write(
        project.join("package.json"),
        r#"{"name":"t","scripts":{"test":"node -e \"process.exit(0)\"","typecheck":"node -e \"process.exit(0)\"","build":"node -e \"process.exit(0)\"","lint":"node -e \"process.exit(0)\""}}"#,
    )
    .unwrap();

    let script = vec![
        tool_json(json!([
            {
                "id": "w1",
                "name": "apply_patch",
                "arguments": {
                    "path": "src/a.js",
                    "old": "1 - 1",
                    "new": "1 + 1"
                }
            },
            {
                "id": "w2",
                "name": "apply_patch",
                "arguments": {
                    "path": "src/b.js",
                    "old": "2 - 2",
                    "new": "2 + 2"
                }
            }
        ])),
        json!({ "text": "Fixed both files.\n- src/a.js\n- src/b.js" }),
    ];

    let cap = run_scenario(
        &project,
        "Fix bugs across multiple files",
        &script,
        Permission::Full,
        ScenarioOpts::default(),
    );

    let a = fs::read_to_string(project.join("src/a.js")).unwrap_or_default();
    let b = fs::read_to_string(project.join("src/b.js")).unwrap_or_default();
    let (passed, detail, outcome) = match cap {
        Ok(c) => {
            let ok = a.contains("1 + 1")
                && b.contains("2 + 2")
                && c.file_changes
                    .iter()
                    .filter(|p| p.starts_with("src/"))
                    .count()
                    >= 2;
            (
                ok,
                format!("changes={:?}", c.file_changes),
                Outcome::Completed,
            )
        }
        Err(e) => (false, e.clone(), Outcome::Failed(e)),
    };
    h.record("03", "multi-file bug", passed, detail, outcome);
    h.write_report();
    assert!(passed, "both files must be patched");
}

#[test]
fn benchmark_04_test_generation() {
    let mut h = Harness::new("04");
    let project = h.project("04-test-gen");
    fs::write(project.join("math.js"), "exports.add = (a, b) => a + b;\n").unwrap();
    fs::write(
        project.join("package.json"),
        r#"{"name":"t","scripts":{"test":"node -e \"require('./math.test.js'); process.exit(0)\"","typecheck":"node -e \"process.exit(0)\"","build":"node -e \"process.exit(0)\"","lint":"node -e \"process.exit(0)\""}}"#,
    )
    .unwrap();

    let test_body = "const assert = require('assert');\nconst { add } = require('./math');\nassert.strictEqual(add(1, 2), 3);\nconsole.log('ok');\n";
    let script = vec![
        tool_json(json!([{
            "id": "c1",
            "name": "create_file",
            "arguments": {
                "path": "math.test.js",
                "content": test_body
            }
        }])),
        json!({ "text": "Added math.test.js.\n- create_file math.test.js\n- tests pass" }),
    ];

    let cap = run_scenario(
        &project,
        "Generate tests for math.add",
        &script,
        Permission::Full,
        ScenarioOpts::default(),
    );

    let test_exists = project.join("math.test.js").is_file();
    let (passed, detail, outcome) = match cap {
        Ok(c) => (
            test_exists && c.file_changes.iter().any(|p| p == "math.test.js"),
            format!("test_file={test_exists} changes={:?}", c.file_changes),
            Outcome::Completed,
        ),
        Err(e) => (false, e.clone(), Outcome::Failed(e)),
    };
    h.record("04", "test generation", passed, detail, outcome);
    h.write_report();
    assert!(passed, "test file must be created");
}

#[test]
fn benchmark_05_malformed_tool_call() {
    let mut h = Harness::new("05");
    let project = h.project("05-malformed");
    fs::write(project.join("README.md"), "# x\n").unwrap();

    // Broken JSON that looks like the tool protocol → structured rejection.
    let script = vec![
        json!({ "text": r#"{"tool_calls":[{"id":"1","name":"read_file","arguments":{"path":""# }),
        json!({ "text": "Recovered after malformed tool JSON.\n- parsed rejection" }),
    ];

    let cap = run_scenario(
        &project,
        "Read README (model will emit malformed tool JSON first)",
        &script,
        Permission::Auto,
        ScenarioOpts::default(),
    );

    let (passed, detail, outcome) = match cap {
        Ok(c) => {
            // Must not panic; must produce a final answer after recovery.
            let ok = !c.answer.is_empty() && c.answer.contains("Verification status");
            (
                ok,
                format!("steps={} answer_len={}", c.steps_seen.len(), c.answer.len()),
                Outcome::Completed,
            )
        }
        Err(e) => (false, e.clone(), Outcome::Failed(e)),
    };
    h.record("05", "malformed tool call", passed, detail, outcome);
    h.write_report();
    assert!(passed, "malformed tool call must not kill the session");
}

#[test]
fn benchmark_06_failed_verification() {
    let mut h = Harness::new("06");
    let project = h.project("06-verify-fail");
    fs::write(project.join("app.js"), "export const x = 1;\n").unwrap();
    // Verify command always fails.
    fs::write(
        project.join("package.json"),
        r#"{"name":"t","scripts":{"test":"node -e \"process.exit(1)\"","typecheck":"node -e \"process.exit(1)\"","build":"node -e \"process.exit(1)\"","lint":"node -e \"process.exit(1)\""}}"#,
    )
    .unwrap();

    let script = vec![
        tool_json(json!([{
            "id": "w1",
            "name": "write_file",
            "arguments": {
                "path": "app.js",
                "content": "export const x = 2;\n"
            }
        }])),
        // After failed verify, model may be nudged; keep answering until budget.
        json!({ "text": "Tried a fix.\n- write app.js" }),
        json!({ "text": "Still failing verification.\n- verify failed" }),
        json!({ "text": "Cannot claim success.\n- verify failed" }),
        json!({ "text": "Giving up under budget.\n- verify failed" }),
        json!({ "text": "Budget exhausted.\n- verify failed" }),
    ];

    let cap = run_scenario(
        &project,
        "Fix the failing tests",
        &script,
        Permission::Full,
        ScenarioOpts::default(),
    );

    let (passed, detail, outcome) = match cap {
        Ok(c) => {
            // Must NOT claim Verified when verification failed.
            let claims_verified = c.answer.contains("Verification status:** Verified");
            let mentions_status = c.answer.contains("Verification status");
            let ok = mentions_status && !claims_verified;
            (
                ok,
                format!(
                    "claims_verified={claims_verified} checks={:?}",
                    c.checks.iter().take(3).collect::<Vec<_>>()
                ),
                Outcome::Completed,
            )
        }
        Err(e) => (false, e.clone(), Outcome::Failed(e)),
    };
    h.record("06", "failed verification", passed, detail, outcome);
    h.write_report();
    assert!(
        passed,
        "failed verification must not be reported as Verified"
    );
}

#[test]
fn benchmark_07_permission_denial() {
    let mut h = Harness::new("07");
    let project = h.project("07-permission");
    fs::write(project.join("README.md"), "# keep\n").unwrap();
    fs::write(
        project.join("package.json"),
        r#"{"name":"t","scripts":{"test":"node -e \"process.exit(0)\"","typecheck":"node -e \"process.exit(0)\"","build":"node -e \"process.exit(0)\"","lint":"node -e \"process.exit(0)\""}}"#,
    )
    .unwrap();

    let script = vec![
        tool_json(json!([{
            "id": "w1",
            "name": "write_file",
            "arguments": {
                "path": "README.md",
                "content": "# overwritten\n"
            }
        }])),
        json!({ "text": "Write was denied by the user.\n- permission denied" }),
        json!({ "text": "Still denied; will not claim success.\n- permission denied" }),
        json!({ "text": "Stopped after denials.\n- permission denied" }),
        json!({ "text": "Stopped.\n- permission denied" }),
        json!({ "text": "Stopped.\n- permission denied" }),
    ];

    let cap = run_scenario(
        &project,
        "Overwrite README",
        &script,
        Permission::Ask,
        ScenarioOpts {
            deny_writes: true,
            ..Default::default()
        },
    );

    let readme = fs::read_to_string(project.join("README.md")).unwrap_or_default();
    let (passed, detail, outcome) = match cap {
        Ok(c) => {
            let ok = readme.contains("# keep") && !readme.contains("# overwritten");
            (
                ok,
                format!("readme_untouched={} checks={:?}", ok, c.checks),
                Outcome::Completed,
            )
        }
        Err(e) => (false, e.clone(), Outcome::Failed(e)),
    };
    h.record("07", "permission denial", passed, detail, outcome);
    h.write_report();
    assert!(passed, "denied write must not touch the file");
}

#[test]
fn benchmark_08_cancellation() {
    let mut h = Harness::new("08");
    let project = h.project("08-cancel");
    fs::write(project.join("README.md"), "# c\n").unwrap();

    let script = vec![
        tool_json(json!([{
            "id": "r1",
            "name": "read_file",
            "arguments": { "path": "README.md" }
        }])),
        json!({ "text": "should not fully complete\n" }),
        json!({ "text": "should not fully complete\n" }),
    ];

    // Cancel after a handful of started steps (context gathering + model).
    let cap = run_scenario(
        &project,
        "Long task that the user cancels",
        &script,
        Permission::Auto,
        ScenarioOpts {
            cancel_after_steps: Some(4),
            ..Default::default()
        },
    );

    let (passed, detail, outcome) = match cap {
        Ok(c) => {
            let ok = c.cancelled
                || c.answer.starts_with("已取消")
                || c.checks.iter().any(|x| x.contains("取消"));
            (
                ok,
                format!("cancelled={} checks={:?}", c.cancelled, c.checks),
                if c.cancelled {
                    Outcome::Cancelled
                } else {
                    Outcome::Completed
                },
            )
        }
        Err(e) => (false, e.clone(), Outcome::Failed(e)),
    };
    h.record("08", "cancellation", passed, detail, outcome);
    h.write_report();
    assert!(passed, "cancellation must be reported, not silent success");
}

#[test]
fn benchmark_09_symlink_escape_attempt() {
    let mut h = Harness::new("09");
    let project = h.project("09-symlink");
    let outside = h.root.join("09-outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "SECRET\n").unwrap();
    fs::write(project.join("README.md"), "# p\n").unwrap();
    // Symlink inside project pointing outside the root.
    #[cfg(unix)]
    {
        let _ = fs::remove_file(project.join("escape"));
        std::os::unix::fs::symlink(&outside, project.join("escape")).unwrap();
    }

    let script = vec![
        tool_json(json!([
            {
                "id": "e1",
                "name": "read_file",
                "arguments": { "path": "escape/secret.txt" }
            },
            {
                "id": "e2",
                "name": "write_file",
                "arguments": {
                    "path": "escape/pwned.txt",
                    "content": "pwned\n"
                }
            },
            {
                "id": "e3",
                "name": "read_file",
                "arguments": { "path": "../outside/secret.txt" }
            }
        ])),
        json!({ "text": "All escape attempts were blocked.\n- path escape refused" }),
        json!({ "text": "Still blocked.\n- path escape refused" }),
        json!({ "text": "Blocked.\n- path escape refused" }),
        json!({ "text": "Blocked.\n- path escape refused" }),
        json!({ "text": "Blocked.\n- path escape refused" }),
    ];

    let cap = run_scenario(
        &project,
        "Try to read and write outside the project via symlink",
        &script,
        Permission::Full,
        ScenarioOpts::default(),
    );

    let pwned = outside.join("pwned.txt").exists();
    let (passed, detail, outcome) = match cap {
        Ok(c) => {
            let ok = !pwned && !c.answer.contains("SECRET");
            (
                ok,
                format!(
                    "pwned={pwned} answer_head={:?} checks={:?}",
                    c.answer.chars().take(160).collect::<String>(),
                    c.checks
                ),
                Outcome::Completed,
            )
        }
        Err(e) => (false, e.clone(), Outcome::Failed(e)),
    };
    h.record(
        "09",
        "symlink escape attempt",
        passed,
        detail.clone(),
        outcome,
    );
    h.write_report();
    assert!(
        passed,
        "symlink/relative escape must not reach outside the project: {detail}"
    );
}
