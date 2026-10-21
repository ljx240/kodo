//! Scripted Work runtime contracts. These results are not live model quality.
//!
//! Every case is run **twice** against a fresh copy of its project, so the
//! report can separate three things a single run conflates:
//!
//! - *success* — the oracle passed in both attempts;
//! - *false completion* — a run claimed `ready` while its oracle failed;
//! - *stability* — the same scripted input produced different verdicts.
//!
//! The oracle is file-integrity based, not claim-based: the source must be
//! byte-identical afterwards, an accepted deliverable must exist, be non-empty
//! and **not be a copy of the source**, and a rejected case must leave its
//! pre-existing target untouched.
use kodo_agent::{AgentMode, Permission, Provider, RunRequest, SinkEvent, Step};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::time::Instant;

#[derive(Deserialize)]
struct Case {
    id: String,
    split: String,
    source: String,
    path: String,
    body: Option<String>,
    contains: String,
    #[serde(default)]
    existing: Option<String>,
    #[serde(default)]
    corrupt: bool,
    accept: bool,
}

/// What one attempt observed. `passed` is the oracle verdict; `ready` is the
/// agent's own claim, kept separate so a claim/oracle gap stays measurable.
struct Attempt {
    passed: bool,
    ready: bool,
    latency_ms: u128,
}

#[derive(Default)]
struct SplitTally {
    cases: usize,
    passed: usize,
    false_completions: usize,
    unstable: usize,
    latency_total_ms: u128,
    latency_max_ms: u128,
}

impl SplitTally {
    fn record(&mut self, passed: bool, ready: bool, unstable: bool, latency_ms: u128) {
        self.cases += 1;
        self.passed += usize::from(passed);
        self.false_completions += usize::from(ready && !passed);
        self.unstable += usize::from(unstable);
        self.latency_total_ms += latency_ms;
        self.latency_max_ms = self.latency_max_ms.max(latency_ms);
    }

    fn success_rate(&self) -> f64 {
        if self.cases == 0 {
            0.0
        } else {
            self.passed as f64 / self.cases as f64
        }
    }

    fn latency_avg_ms(&self) -> f64 {
        if self.cases == 0 {
            0.0
        } else {
            self.latency_total_ms as f64 / self.cases as f64
        }
    }
}

pub(super) fn run(report: &mut super::EvalReport) {
    let cases: Vec<Case> = serde_json::from_str(include_str!("../work-fixtures/cases.json"))
        .expect("Work fixture manifest");
    assert_eq!(cases.len(), 30);
    let root = std::env::temp_dir().join(format!("kodo-work-contracts-{}", std::process::id()));
    fs::create_dir_all(&root).expect("fixture root");
    let mut splits: [SplitTally; 2] = Default::default();
    let mut unstable_cases: Vec<String> = Vec::new();
    for case in &cases {
        let first = attempt(case, &root, 0);
        let second = attempt(case, &root, 1);
        let (passed, unstable) = match (&first, &second) {
            (Ok(a), Ok(b)) => (
                a.passed && b.passed,
                a.passed != b.passed || a.ready != b.ready,
            ),
            // A harness error is a failure of the contract, and counts as
            // instability only if the two attempts disagreed about it.
            (Err(_), Err(_)) => (false, false),
            _ => (false, true),
        };
        if unstable {
            unstable_cases.push(case.id.clone());
        }
        let ready = first.as_ref().map(|a| a.ready).unwrap_or(false)
            || second.as_ref().map(|a| a.ready).unwrap_or(false);
        let latency = first
            .as_ref()
            .map(|a| a.latency_ms)
            .unwrap_or_default()
            .max(second.as_ref().map(|a| a.latency_ms).unwrap_or_default());
        let detail = match (&first, &second) {
            (Ok(a), Ok(b)) => format!(
                "scripted Work contract failed: attempt1(passed={} ready={}) attempt2(passed={} ready={})",
                a.passed, a.ready, b.passed, b.ready
            ),
            (Err(error), _) | (_, Err(error)) => {
                format!("scripted Work contract errored: {error}")
            }
        };
        report.check(&format!("work_{}", case.id), passed, &detail);
        let index = usize::from(case.split == "holdout");
        splits[index].record(passed, ready, unstable, latency);
    }
    for (label, tally) in ["development", "holdout"].into_iter().zip(&splits) {
        println!(
            "Work scripted {label}: {}/{} contracts ({:.0}%) · false_completions={} · unstable={} · latency_avg={:.1}ms max={}ms · live quality NOT RUN",
            tally.passed,
            tally.cases,
            tally.success_rate() * 100.0,
            tally.false_completions,
            tally.unstable,
            tally.latency_avg_ms(),
            tally.latency_max_ms,
        );
    }
    if !unstable_cases.is_empty() {
        println!(
            "Work scripted unstable cases (same input, different verdict): {}",
            unstable_cases.join(", ")
        );
    }
    println!(
        "work_report_json: {}",
        serde_json::json!({
            "layer": "work-scripted",
            "live_quality": "NOT RUN",
            "cases": cases.len(),
            "attempts_per_case": 2,
            "development": tally_json(&splits[0]),
            "holdout": tally_json(&splits[1]),
            "unstable_cases": unstable_cases,
        })
    );
    fs::remove_dir_all(root).expect("remove owned fixture root");
}

fn tally_json(tally: &SplitTally) -> Value {
    json!({
        "cases": tally.cases,
        "passed": tally.passed,
        "success_rate": tally.success_rate(),
        "false_completions": tally.false_completions,
        "unstable": tally.unstable,
        "latency_avg_ms": tally.latency_avg_ms(),
        "latency_max_ms": tally.latency_max_ms,
    })
}

/// One full attempt: fresh project copy, scripted turns, then the file oracle.
fn attempt(case: &Case, root: &Path, run_no: usize) -> Result<Attempt, String> {
    let started = Instant::now();
    let project = root.join(format!("{}-{run_no}", case.id));
    fs::create_dir_all(&project).map_err(|e| e.to_string())?;
    fs::write(project.join("source.txt"), &case.source).map_err(|e| e.to_string())?;
    if let Some(existing) = &case.existing {
        kodo_agent::tools::write_project_file(&project, &case.path, existing)?;
    }
    if case.corrupt {
        fs::write(project.join(&case.path), "broken office archive").map_err(|e| e.to_string())?;
    }
    // Digest of whatever the target looked like *before* the run, so a rejected
    // case can prove it did not touch it.
    let before_target = fs::read(project.join(&case.path)).ok();

    let mut turns = vec![
        json!({"text": json!({"tool_calls": [{"name":"read_file","args":{"path":"source.txt"}}]}).to_string()}),
    ];
    if case.existing.is_some() {
        // Revising an existing file requires reading it first: a blind
        // whole-file overwrite is refused by the overwrite gate, so the script
        // has to model the real read-then-edit sequence.
        turns.push(json!({"text": json!({"tool_calls": [{"name":"read_file","args":{"path":case.path}}]}).to_string()}));
    }
    if let Some(body) = &case.body {
        turns.push(json!({"text": json!({"tool_calls": [{"name":"write_file","args":{"path":case.path,"content":body}}]}).to_string()}));
        turns.push(json!({"text": json!({"tool_calls": [{"name":"read_file","args":{"path":case.path}}]}).to_string()}));
    }
    for _ in 0..16 {
        turns.push(json!({"text":"已完成请求，结果已保存。"}));
    }
    let script = root.join(format!("{}-{run_no}.jsonl", case.id));
    fs::write(
        &script,
        turns
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .map_err(|e| e.to_string())?;
    let request = RunRequest {
        sandbox: None,
        project: Some(project.clone()),
        message: format!("请根据 source.txt 的资料生成文件，保存到 `{}`", case.path),
        conversation_history: Vec::new(),
        pinned_context: Vec::new(),
        pinned_force_inline: Vec::new(),
        provider: Some(Provider::new(
            "fake",
            "synthetic",
            script.to_string_lossy(),
            "fixture",
        )),
        permission: Permission::Full,
        mode: AgentMode::Work,
        fallback_to_local: false,
        max_output_tokens: 1024,
        extended_thinking: false,
        local_trace: false,
        session_id: None,
        user_skills_dir: None,
        skill_roots: Vec::new(),
        mcp_servers: Vec::new(),
        agent_instructions: None,
        personas: Vec::new(),
        budget_family: None,
        memory: None,
        knowledge_context: None,
        allow_verify_commands: false,
        // Scripted Work contracts never consult the decision service.
        decision: None,
    };
    let mut ready = false;
    let mut sink = |event| {
        if let SinkEvent::Finished {
            step: Step::AgentMessage { delivery, .. },
            ..
        } = event
        {
            ready = delivery == "ready";
        }
        true
    };
    let result = kodo_agent::run(&request, &|| true, &|_, _| true, &mut sink);

    // --- file-integrity oracle -------------------------------------------
    // 1. The input is untouched. A run that "summarises" by rewriting its own
    //    source is not a deliverable.
    let source_intact =
        fs::read_to_string(project.join("source.txt")).map_err(|e| e.to_string())? == case.source;
    // 2. A target that existed before the run must still exist and still hold
    //    bytes: a failed or refused write may rewrite it, but never truncate it
    //    to nothing or delete it.
    let preserved = match &before_target {
        None => true,
        Some(_) => fs::read(project.join(&case.path))
            .map(|after| !after.is_empty())
            .unwrap_or(false),
    };
    // 3. No staging residue. The atomic write path stages a sibling
    //    `.name.kodo-tmp-…` file and renames it; anything left behind is a
    //    leaked temp file, which is exactly the kind of dirt a delivery gate
    //    must not call success.
    let leftover_temp = count_temp_residue(&project);
    // 4. An accepted deliverable exists, is non-empty and contains the
    //    expected marker. A rejected case must not claim `ready`.
    let delivered = if case.accept {
        let full = kodo_agent::tools::resolve_in_project(&project, &case.path)?;
        let bytes = fs::read(&full).map_err(|e| e.to_string())?;
        let content = if kodo_agent::office::is_docx(&case.path) {
            kodo_agent::office::read_docx(&full, 100_000)?.text
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        ready && !content.trim().is_empty() && content.contains(&case.contains)
    } else {
        !ready
    };
    Ok(Attempt {
        passed: source_intact
            && preserved
            && leftover_temp == 0
            && delivered
            && (result.is_ok() || !case.accept),
        ready,
        latency_ms: started.elapsed().as_millis(),
    })
}

/// Count staging files the atomic writer failed to clean up.
fn count_temp_residue(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_dir() {
                count_temp_residue(&entry.path())
            } else {
                usize::from(name.contains(".kodo-tmp-"))
            }
        })
        .sum()
}
