//! End-to-end sandbox tests through the agent's own process runner.
//!
//! These exercise the real path a model-driven command takes:
//! `command_run_confined` → `ProcessSpec` → `ProcessRunner` → OS sandbox.

use std::path::PathBuf;
use std::time::Duration;

use kodo_agent::process::{ProcessRunner, ProcessSpec, ProcessStatus};
use kodo_agent::tools::{command_run, command_run_confined, CommandOutcomeKind};
use kodo_sandbox::{SandboxContext, SandboxMode, SandboxRuntime};

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kodo-agent-sbx-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("workspace")).unwrap();
    std::fs::create_dir_all(dir.join("outside")).unwrap();
    dir
}

/// True when this platform is expected to provide a working backend.
///
/// A platform that ships the backend but cannot confine anything is a
/// failure, not a skip: silently skipping would turn a broken sandbox into
/// a green test run.
fn backend_expected() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::path::Path::new("/usr/bin/sandbox-exec").exists()
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).any(|dir| dir.join("bwrap").is_file()))
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

/// Fail loudly when the platform ships a backend that does not confine.
fn require_backend(runtime: &SandboxRuntime) {
    if runtime.is_available() {
        return;
    }
    let notes = runtime
        .capabilities()
        .notes
        .first()
        .cloned()
        .unwrap_or_else(|| "no sandbox backend".to_owned());
    assert!(
        !backend_expected(),
        "this platform ships a sandbox backend but it does not confine: {notes}"
    );
    eprintln!("skipped-unavailable: {notes}");
}

fn context(
    runtime: std::sync::Arc<SandboxRuntime>,
    dir: &std::path::Path,
    mode: SandboxMode,
) -> SandboxContext {
    let policy = runtime.policy_for(&dir.join("workspace"), mode);
    SandboxContext::new(runtime, policy)
}

#[test]
fn a_confined_command_cannot_write_outside_the_workspace() {
    let runtime = SandboxRuntime::detect();
    require_backend(&runtime);
    if !runtime.is_available() {
        return;
    }
    let dir = workspace("outside");
    let ws = dir.join("workspace");
    let outside = dir.join("outside");
    let target = outside.join("escaped.txt");

    let outcome = command_run_confined(
        &ws,
        &format!("echo escaped > {}", target.display()),
        &|| true,
        10,
        &context(runtime, &dir, SandboxMode::WorkspaceWrite),
    );
    assert_ne!(
        outcome.kind,
        CommandOutcomeKind::Success,
        "the write outside the workspace must fail"
    );
    assert!(!target.exists(), "the escaped file must not exist");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_confined_command_can_write_inside_the_workspace() {
    let runtime = SandboxRuntime::detect();
    require_backend(&runtime);
    if !runtime.is_available() {
        return;
    }
    let dir = workspace("inside");
    let ws = dir.join("workspace");

    let outcome = command_run_confined(
        &ws,
        "echo written > result.txt && cat result.txt",
        &|| true,
        10,
        &context(runtime, &dir, SandboxMode::WorkspaceWrite),
    );
    assert_eq!(outcome.kind, CommandOutcomeKind::Success, "{outcome:?}");
    assert!(outcome.output.contains("written"), "{outcome:?}");
    assert_eq!(
        std::fs::read_to_string(ws.join("result.txt")).unwrap(),
        "written\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_read_only_confined_command_cannot_modify_the_workspace() {
    let runtime = SandboxRuntime::detect();
    require_backend(&runtime);
    if !runtime.is_available() {
        return;
    }
    let dir = workspace("readonly");
    let ws = dir.join("workspace");
    std::fs::write(ws.join("source.txt"), "original\n").unwrap();

    let outcome = command_run_confined(
        &ws,
        "echo modified > source.txt",
        &|| true,
        10,
        &context(runtime, &dir, SandboxMode::ReadOnly),
    );
    assert_ne!(outcome.kind, CommandOutcomeKind::Success, "{outcome:?}");
    assert_eq!(
        std::fs::read_to_string(ws.join("source.txt")).unwrap(),
        "original\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_process_runner_reports_the_sandbox_decision() {
    let runtime = SandboxRuntime::detect();
    require_backend(&runtime);
    if !runtime.is_available() {
        return;
    }
    let dir = workspace("report");
    let ws = dir.join("workspace");
    let spec = ProcessSpec::shell(&ws, "echo hi")
        .timeout(Duration::from_secs(10))
        .sandbox(context(runtime, &dir, SandboxMode::WorkspaceWrite));
    let outcome = ProcessRunner::run(&spec, &|| true);
    assert_eq!(outcome.status, ProcessStatus::ExitSuccess, "{outcome:?}");
    let report = outcome.sandbox.expect("a sandbox report must be present");
    assert!(report.is_confined(), "{report:?}");
    assert_eq!(report.mode, SandboxMode::WorkspaceWrite);
    assert_eq!(report.backend, "seatbelt");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_command_without_a_sandbox_still_runs_for_the_control_plane() {
    // The trusted control plane (Kodo's own git queries) keeps working
    // unchanged — the sandbox is additive, not a replacement.
    let dir = workspace("control");
    let ws = dir.join("workspace");
    let outcome = command_run(&ws, "echo control-plane", &|| true, 10);
    assert_eq!(outcome.kind, CommandOutcomeKind::Success, "{outcome:?}");
    assert!(outcome.output.contains("control-plane"), "{outcome:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_timeout_still_kills_a_confined_process_tree() {
    let runtime = SandboxRuntime::detect();
    require_backend(&runtime);
    if !runtime.is_available() {
        return;
    }
    let dir = workspace("timeout");
    let ws = dir.join("workspace");
    // The marker lives *inside* the workspace: a surviving grandchild would
    // really create it, so the test proves the tree kill rather than the
    // sandbox's write restriction.
    let marker = ws.join("grandchild.txt");
    let script = format!("(sleep 2; echo x > '{}') & sleep 20", marker.display());
    let spec = ProcessSpec::shell(&ws, script)
        .timeout(Duration::from_millis(400))
        .sandbox(context(runtime, &dir, SandboxMode::WorkspaceWrite));
    let outcome = ProcessRunner::run(&spec, &|| true);
    assert!(
        outcome.timed_out || outcome.cancelled,
        "the confined run must still time out: {outcome:?}"
    );
    std::thread::sleep(Duration::from_millis(2500));
    assert!(
        !marker.exists(),
        "the confined grandchild must not survive the timeout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancellation_still_kills_a_confined_process() {
    let runtime = SandboxRuntime::detect();
    require_backend(&runtime);
    if !runtime.is_available() {
        return;
    }
    let dir = workspace("cancel");
    let ws = dir.join("workspace");
    let spec = ProcessSpec::shell(&ws, "sleep 30")
        .timeout(Duration::from_secs(30))
        .sandbox(context(runtime, &dir, SandboxMode::WorkspaceWrite));
    let outcome = ProcessRunner::run(&spec, &|| false);
    assert_eq!(outcome.status, ProcessStatus::Cancelled, "{outcome:?}");
    assert!(outcome.cancelled);
    let _ = std::fs::remove_dir_all(&dir);
}
