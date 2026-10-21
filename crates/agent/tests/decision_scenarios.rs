//! Scenario-level acceptance for the optional decision component.
//!
//! These tests drive the real agent loop with a local fake provider and a
//! fake decision host. Nothing leaves the machine: the decision host is a
//! Rust object that records what it was asked and answers from a script.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use kodo_agent::decision::{
    DecisionAccess, DecisionInput, DecisionMode, DecisionOutcome, DecisionReceipt, DecisionResult,
    ValidatedDecision,
};
use kodo_agent::{AgentMode, Permission, Provider, RunRequest, SinkEvent, Step};

/// A decision host that records its questions and answers from a script.
struct RecordingDecision {
    calls: Mutex<Vec<DecisionInput>>,
    outcome: Mutex<DecisionOutcome>,
    current: AtomicBool,
}

impl RecordingDecision {
    fn new(outcome: DecisionOutcome) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            outcome: Mutex::new(outcome),
            current: AtomicBool::new(true),
        })
    }

    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl DecisionAccess for RecordingDecision {
    fn is_current(&self) -> bool {
        self.current.load(Ordering::SeqCst)
    }

    fn evaluate(&self, input: &DecisionInput, _alive: &dyn Fn() -> bool) -> DecisionOutcome {
        self.calls.lock().unwrap().push(input.clone());
        self.outcome.lock().unwrap().clone()
    }
}

fn receipt(mode: DecisionMode) -> DecisionReceipt {
    DecisionReceipt {
        request_id: "jev-test".to_owned(),
        mode,
        provider: kodo_agent::decision::DecisionProvider::TypeSafe,
        model: "jev-1.13.0".to_owned(),
        policy_version: "kodo-jev-v1".to_owned(),
        duration_ms: 12,
        consent_wait_ms: 7,
        input_tokens: Some(120),
    }
}

fn suggested(mode: DecisionMode, choice: &str, top: f64) -> DecisionOutcome {
    let mut probabilities = std::collections::BTreeMap::new();
    let rest = (1.0 - top) / 6.0;
    for key in [
        "bug-fix",
        "feature",
        "test",
        "code-review",
        "docs",
        "unknown",
    ] {
        probabilities.insert(key.to_owned(), rest);
    }
    probabilities.insert(choice.to_owned(), top);
    DecisionOutcome {
        receipt: receipt(mode),
        result: DecisionResult::Suggested(ValidatedDecision {
            scenario: kodo_agent::decision::DecisionScenario::TaskRoute,
            model: "jev-1.13.0".to_owned(),
            question_id: "task_type".to_owned(),
            choice: choice.to_owned(),
            probabilities,
            confidence: Some(0.91),
            input_tokens: Some(120),
        }),
    }
}

fn skipped(mode: DecisionMode) -> DecisionOutcome {
    DecisionOutcome {
        receipt: receipt(mode),
        result: DecisionResult::Skipped(kodo_agent::decision::DecisionSkip::new("disabled", "off")),
    }
}

/// An answer the service gave without a probability distribution.
fn uncalibrated() -> DecisionOutcome {
    DecisionOutcome {
        receipt: receipt(DecisionMode::Assist),
        result: DecisionResult::Uncalibrated {
            choice: "refactor".to_owned(),
        },
    }
}

fn cancelled() -> DecisionOutcome {
    DecisionOutcome {
        receipt: receipt(DecisionMode::Observe),
        result: DecisionResult::Cancelled,
    }
}

/// A project with one file, so the run is a task and not a conversation.
///
/// Every test gets its own directory: the tests run in parallel and must
/// not delete each other's fixture.
fn fixture() -> (std::path::PathBuf, std::path::PathBuf) {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "kodo-jev-scenario-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let project = dir.join("repo");
    std::fs::create_dir_all(&project).expect("project dir");
    std::fs::write(project.join("README.md"), "# Demo\n").expect("readme");
    let script = dir.join("script.jsonl");
    std::fs::write(
        &script,
        format!(
            "{}\n{}\n",
            serde_json::json!({ "text": "已完成。" }),
            serde_json::json!({ "text": "已完成。" }),
        ),
    )
    .expect("script");
    (project, script)
}

fn request(
    project: std::path::PathBuf,
    script: std::path::PathBuf,
    mode: AgentMode,
    message: &str,
    decision: Option<kodo_agent::DecisionHandle>,
) -> RunRequest {
    RunRequest {
        project: Some(project),
        message: message.to_owned(),
        conversation_history: Vec::new(),
        pinned_context: Vec::new(),
        pinned_force_inline: Vec::new(),
        provider: Some(Provider::new(
            "fake",
            "fake-key",
            script.to_string_lossy().into_owned(),
            "fake-model",
        )),
        permission: Permission::Ask,
        mode,
        fallback_to_local: true,
        max_output_tokens: 512,
        extended_thinking: false,
        local_trace: false,
        session_id: None,
        user_skills_dir: None,
        mcp_servers: Vec::new(),
        agent_instructions: None,
        personas: vec![],
        budget_family: None,
        memory: None,
        knowledge_context: None,
        allow_verify_commands: false,
        decision,
    }
}

/// Run the turn and count how many decision-record steps started and how
/// many finished. A step that only starts would stay "running" in the
/// session log forever.
fn run_and_pair(request: RunRequest) -> (Result<(), String>, usize, usize) {
    let mut started = 0usize;
    let mut finished = 0usize;
    let mut emit = |event: SinkEvent| -> bool {
        let step = match &event {
            SinkEvent::Started { step } | SinkEvent::Finished { step, .. } => step,
            _ => return true,
        };
        let is_decision = match step {
            Step::Reasoning { diagnostics, .. } => diagnostics
                .as_deref()
                .is_some_and(|text| text.contains("decision.v1")),
            _ => false,
        };
        if !is_decision {
            return true;
        }
        match event {
            SinkEvent::Started { .. } => started += 1,
            _ => finished += 1,
        }
        true
    };
    let alive = || true;
    let approve = |_kind: kodo_agent::StepKind, _command: &str| -> bool { true };
    let result = kodo_agent::run(&request, &alive, &approve, &mut emit);
    (result, started, finished)
}

/// Every reasoning diagnostic the run emitted.
fn diagnostics(steps: &[Step]) -> Vec<String> {
    steps
        .iter()
        .filter_map(|step| match step {
            Step::Reasoning { diagnostics, .. } => diagnostics.clone(),
            _ => None,
        })
        .collect()
}

fn run_and_collect(request: RunRequest) -> (Result<(), String>, Vec<Step>) {
    let mut steps: Vec<Step> = Vec::new();
    let mut emit = |event: SinkEvent| -> bool {
        if let SinkEvent::Started { step } = event {
            steps.push(step);
        }
        true
    };
    let alive = || true;
    let approve = |_kind: kodo_agent::StepKind, _command: &str| -> bool { true };
    let result = kodo_agent::run(&request, &alive, &approve, &mut emit);
    (result, steps)
}

#[test]
fn observe_records_the_suggestion_without_changing_the_task() {
    let (project, script) = fixture();
    let handle = RecordingDecision::new(suggested(DecisionMode::Observe, "refactor", 0.93));
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "把重复的校验逻辑合并一下",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_result, steps) = run_and_collect(request);
    assert_eq!(handle.calls(), 1, "observe must ask exactly once");
    let records = diagnostics(&steps);
    let decision_record = records
        .iter()
        .find(|text| text.contains("decision.v1"))
        .expect("a decision record must be in the trace");
    assert!(
        decision_record.contains("\"status\":\"observed\""),
        "{decision_record}"
    );
    assert!(
        decision_record.contains("\"applied\":false"),
        "{decision_record}"
    );
    assert!(
        decision_record.contains("\"suggestion\":\"refactor\""),
        "{decision_record}"
    );
    // The local rule kept the category.
    assert!(
        !records
            .iter()
            .any(|text| text.contains("task_type=refactor")),
        "observe must not change the task category"
    );
    // The record carries no request content.
    assert!(
        !decision_record.contains("把重复的校验逻辑合并一下"),
        "{decision_record}"
    );
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

#[test]
fn assist_changes_only_the_task_category() {
    let (project, script) = fixture();
    let handle = RecordingDecision::new(suggested(DecisionMode::Assist, "refactor", 0.93));
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "把重复的校验逻辑合并一下",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_result, steps) = run_and_collect(request);
    assert_eq!(handle.calls(), 1);
    let records = diagnostics(&steps);
    let decision_record = records
        .iter()
        .find(|text| text.contains("decision.v1"))
        .expect("a decision record must be in the trace");
    assert!(
        decision_record.contains("\"status\":\"applied\""),
        "{decision_record}"
    );
    assert!(
        decision_record.contains("\"applied\":true"),
        "{decision_record}"
    );
    // The category changed, and the change is visible in the trace.
    assert!(
        records
            .iter()
            .any(|text| text.contains("task_type=refactor")),
        "assist must change the task category: {records:?}"
    );
    // The baseline is still reported next to the suggestion.
    assert!(
        decision_record.contains("\"baseline\":\"feature\""),
        "{decision_record}"
    );
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

#[test]
fn a_low_confidence_suggestion_is_recorded_but_not_applied() {
    let (project, script) = fixture();
    let handle = RecordingDecision::new(suggested(DecisionMode::Assist, "refactor", 0.55));
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "把重复的校验逻辑合并一下",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_result, steps) = run_and_collect(request);
    assert_eq!(handle.calls(), 1);
    let records = diagnostics(&steps);
    let decision_record = records
        .iter()
        .find(|text| text.contains("decision.v1"))
        .expect("a decision record must be in the trace");
    assert!(
        decision_record.contains("\"applied\":false"),
        "{decision_record}"
    );
    assert!(
        decision_record.contains("\"rejectedReason\":\"below_threshold\""),
        "{decision_record}"
    );
    assert!(
        !records
            .iter()
            .any(|text| text.contains("task_type=refactor")),
        "a low-confidence suggestion must not change the category"
    );
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

#[test]
fn the_agent_rechecks_host_validity_before_adopting_a_result() {
    let (project, script) = fixture();
    let handle = RecordingDecision::new(suggested(DecisionMode::Assist, "refactor", 0.99));
    handle.current.store(false, Ordering::SeqCst);
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "把重复的校验逻辑合并一下",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_, steps) = run_and_collect(request);
    assert_eq!(handle.calls(), 1);
    let records = diagnostics(&steps);
    assert!(records
        .iter()
        .any(|record| record.contains("\"fallbackCode\":\"config_changed\"")));
    assert!(!records
        .iter()
        .any(|record| record.contains("task_type=refactor")));
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

#[test]
fn work_mode_never_asks() {
    let (project, script) = fixture();
    let handle = RecordingDecision::new(suggested(DecisionMode::Observe, "refactor", 0.93));
    let request = request(
        project.clone(),
        script,
        AgentMode::Work,
        "把重复的校验逻辑合并一下",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_result, _steps) = run_and_collect(request);
    assert_eq!(handle.calls(), 0, "work mode must not ask");
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

#[test]
fn a_conversation_never_asks() {
    let (project, script) = fixture();
    let handle = RecordingDecision::new(suggested(DecisionMode::Observe, "refactor", 0.93));
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "介绍下什么是数据仓库",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_result, _steps) = run_and_collect(request);
    assert_eq!(handle.calls(), 0, "a conversation must not ask");
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

#[test]
fn an_explicit_skill_never_asks() {
    let (project, script) = fixture();
    let handle = RecordingDecision::new(suggested(DecisionMode::Observe, "refactor", 0.93));
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "【技能：bug-fix】\n把重复的校验逻辑合并一下",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_result, _steps) = run_and_collect(request);
    assert_eq!(handle.calls(), 0, "an explicit skill must not ask");
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

#[test]
fn without_a_handle_nothing_is_asked_and_nothing_changes() {
    let (project, script) = fixture();
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "把重复的校验逻辑合并一下",
        None,
    );
    let (_result, steps) = run_and_collect(request);
    let records = diagnostics(&steps);
    assert!(
        !records.iter().any(|text| text.contains("decision.v1")),
        "without a handle there must be no decision record"
    );
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

#[test]
fn a_cancelled_decision_stops_the_round() {
    let (project, script) = fixture();
    let handle = RecordingDecision::new(cancelled());
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "把重复的校验逻辑合并一下",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_result, steps) = run_and_collect(request);
    assert_eq!(handle.calls(), 1);
    let records = diagnostics(&steps);
    let decision_record = records
        .iter()
        .find(|text| text.contains("decision.v1"))
        .expect("a decision record must be in the trace");
    assert!(
        decision_record.contains("\"status\":\"cancelled\""),
        "{decision_record}"
    );
    // A cancelled decision must not be followed by more work.
    let command_steps = steps
        .iter()
        .filter(|step| matches!(step, Step::Command { .. }))
        .count();
    assert_eq!(command_steps, 0, "a cancelled decision must stop the tools");
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

#[test]
fn a_skipped_decision_keeps_the_local_path() {
    let (project, script) = fixture();
    let handle = RecordingDecision::new(skipped(DecisionMode::Observe));
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "把重复的校验逻辑合并一下",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_result, steps) = run_and_collect(request);
    assert_eq!(handle.calls(), 1);
    let records = diagnostics(&steps);
    let decision_record = records
        .iter()
        .find(|text| text.contains("decision.v1"))
        .expect("a decision record must be in the trace");
    assert!(
        decision_record.contains("\"status\":\"skipped\""),
        "{decision_record}"
    );
    assert!(
        !records
            .iter()
            .any(|text| text.contains("task_type=refactor")),
        "a skipped decision must not change the category"
    );
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

#[test]
fn an_uncalibrated_answer_is_recorded_but_never_applied() {
    // The thresholds assume real probabilities. An answer without them is
    // shown to the user and refused as a decision.
    let (project, script) = fixture();
    let handle = RecordingDecision::new(uncalibrated());
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "把重复的校验逻辑合并一下",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_result, steps) = run_and_collect(request);
    assert_eq!(handle.calls(), 1);
    let records = diagnostics(&steps);
    let decision_record = records
        .iter()
        .find(|text| text.contains("decision.v1"))
        .expect("a decision record must be in the trace");
    assert!(
        decision_record.contains("\"status\":\"uncalibrated\""),
        "{decision_record}"
    );
    assert!(
        decision_record.contains("\"applied\":false"),
        "{decision_record}"
    );
    assert!(
        !records
            .iter()
            .any(|text| text.contains("task_type=refactor")),
        "an uncalibrated answer must not change the category"
    );
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

#[test]
fn a_decision_record_leaves_no_dangling_item() {
    // A step that only starts would stay "running" in the session log
    // forever. The decision record must close its own item.
    let (project, script) = fixture();
    let handle = RecordingDecision::new(suggested(DecisionMode::Observe, "refactor", 0.93));
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "把重复的校验逻辑合并一下",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_result, started, finished) = run_and_pair(request);
    assert_eq!(
        started, 1,
        "the decision record must produce exactly one step"
    );
    assert_eq!(
        started, finished,
        "every started decision step must have a matching finished step"
    );
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

/// A counter used to prove the run is not asked twice in one turn.
static ASKED: AtomicUsize = AtomicUsize::new(0);

#[test]
fn one_turn_asks_at_most_once() {
    ASKED.store(0, Ordering::SeqCst);
    let (project, script) = fixture();
    let handle = RecordingDecision::new(suggested(DecisionMode::Observe, "refactor", 0.93));
    let request = request(
        project.clone(),
        script,
        AgentMode::Code,
        "把重复的校验逻辑合并一下",
        Some(handle.clone() as kodo_agent::DecisionHandle),
    );
    let (_result, _steps) = run_and_collect(request);
    assert!(handle.calls() <= 1, "one turn must ask at most once");
    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}
