//! Optional online model benchmark.
//!
//! Ignored by default so PR CI never needs a personal API key. Enable with:
//! ```bash
//! KODO_ONLINE_API_KEY=... cargo test -p kodo-agent --test online_benchmark -- --ignored --nocapture
//! ```
//! Or via the `agent-benchmark-online` workflow_dispatch job.

use std::fs;
use std::path::PathBuf;

use kodo_agent::{Permission, Provider, RunRequest, SinkEvent, Step, StepKind};

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
        project: project.clone(),
        message: "What is this repository? One sentence.".into(),
        pinned_context: vec![],
        provider: Some(provider),
        permission: Permission::Auto,
        fallback_to_local: true,
        max_output_tokens: 256,
        extended_thinking: false,
        session_id: None,
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
