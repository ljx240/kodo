//! Optional online model benchmark.
//!
//! Ignored by default so PR CI never needs a personal API key. Enable with:
//! ```bash
//! KODO_ONLINE_API_KEY=... cargo test -p kodo-agent --test online_benchmark -- --ignored --nocapture
//! ```
//! Or via the `agent-benchmark-online` workflow_dispatch job.

use std::fs;
use std::path::PathBuf;

use kodo_agent::{AgentMode, Permission, Provider, RunRequest, SinkEvent, Step, StepKind};

/// Minimal online smoke: one read-only question against a real endpoint.
/// Skips cleanly when the key env var is missing.
#[test]
#[ignore = "requires KODO_ONLINE_API_KEY; never run on PR CI"]
fn online_read_only_smoke() {
    let key = std::env::var("KODO_ONLINE_API_KEY").unwrap_or_default();
    if key.trim().is_empty() {
        eprintln!("skip: KODO_ONLINE_API_KEY not set");
        return;
    }
    let endpoint = std::env::var("KODO_ONLINE_ENDPOINT").unwrap_or_default();
    let model = std::env::var("KODO_ONLINE_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());

    let project = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("online-bench");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("README.md"), "# Online bench\n").unwrap();

    let provider = Provider::new("openai", key, endpoint, model);
    let request = RunRequest {
        project: Some(project.clone()),
        message: "What is this repository? One sentence.".into(),
        conversation_history: Vec::new(),
        pinned_context: vec![],
        pinned_force_inline: Vec::new(),
        provider: Some(provider),
        permission: Permission::Auto,
        mode: AgentMode::Code,
        fallback_to_local: true,
        max_output_tokens: 256,
        extended_thinking: false,
        local_trace: false,
        session_id: None,
        user_skills_dir: None,
        mcp_servers: Vec::new(),
        agent_instructions: None,
        personas: Vec::new(),
        budget_family: None,
        memory: None,
        knowledge_context: None,
        allow_verify_commands: false,
        decision: None,
    };

    let mut answer = String::new();
    let alive = || true;
    let approve = |_: StepKind, _: &str| true;
    let mut emit = |event: SinkEvent| {
        if let SinkEvent::Finished {
            step: Step::AgentMessage { text, .. },
            ..
        } = event
        {
            answer = text;
        }
        true
    };

    kodo_agent::run(&request, &alive, &approve, &mut emit).expect("online run");
    assert!(!answer.is_empty(), "online smoke must produce an answer");
    eprintln!(
        "online answer head: {}",
        answer.chars().take(200).collect::<String>()
    );
}

/// Live websearch smoke — confirms DDG is reachable and `parse_results` is
/// still aligned with the real HTML. PR CI skips this; run on demand:
/// ```bash
/// cargo test -p kodo-agent --test online_benchmark -- --ignored websearch_live_returns_hits --nocapture
/// ```
#[test]
#[ignore = "requires network; never run on PR CI"]
fn websearch_live_returns_hits() {
    let hits = kodo_agent::websearch::search("tauri webview window", 5)
        .expect("live websearch must succeed when DDG is reachable");
    assert!(!hits.is_empty(), "live query must produce at least one hit");
    let first = &hits[0];
    assert!(!first.title.is_empty(), "first hit title must be non-empty");
    assert!(
        !first.url.is_empty(),
        "first hit url must be non-empty (after the uddg unwrap)"
    );
    assert!(
        !first.snippet.is_empty(),
        "first hit snippet must be non-empty"
    );
    eprintln!(
        "live websearch returned {} hit(s); first title = {:?}",
        hits.len(),
        first.title,
    );
}
