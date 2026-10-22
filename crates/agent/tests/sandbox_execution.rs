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

/// The backend identifier this platform must report.
fn expected_backend() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "seatbelt"
    }
    #[cfg(target_os = "linux")]
    {
        "bubblewrap"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "none"
    }
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
    let report = outcome
        .sandbox
        .as_ref()
        .expect("a sandbox report must be present");
    assert!(report.is_confined(), "{report:?}");
    assert_eq!(report.mode, SandboxMode::WorkspaceWrite);
    // The backend is the one this platform ships — never a hard-coded name.
    assert_eq!(report.backend, expected_backend(), "{report:?}");
    assert!(
        outcome.failure.is_none(),
        "a successful run must not report a failure: {outcome:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_ordinary_non_zero_exit_is_not_a_sandbox_failure() {
    // The escalation trigger used to be the exit code. A program that
    // happens to exit with 126 is an ordinary failure — it must not be
    // classified as a permission problem.
    let runtime = SandboxRuntime::detect();
    require_backend(&runtime);
    if !runtime.is_available() {
        return;
    }
    let dir = workspace("exit126");
    let ws = dir.join("workspace");
    let spec = ProcessSpec::shell(&ws, "exit 126")
        .timeout(Duration::from_secs(10))
        .sandbox(context(runtime, &dir, SandboxMode::WorkspaceWrite));
    let outcome = ProcessRunner::run(&spec, &|| true);
    assert_eq!(outcome.status, ProcessStatus::ExitFailure, "{outcome:?}");
    assert_eq!(outcome.exit_code, Some(126), "{outcome:?}");
    assert!(
        outcome.failure.is_none(),
        "an ordinary exit code must not become a sandbox failure: {outcome:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_refused_sandbox_is_reported_as_a_structured_failure() {
    // The refusal is structural, so it survives without the exit code.
    let runtime = SandboxRuntime::detect();
    require_backend(&runtime);
    if !runtime.is_available() {
        return;
    }
    let dir = workspace("refusal");
    let ws = dir.join("workspace");
    // A policy the backend cannot enforce is refused before the process
    // starts — the outcome must say so.
    let mut policy = runtime.policy_for(&ws, SandboxMode::WorkspaceWrite);
    policy.network = kodo_sandbox::NetworkPolicy::AllowDomains(vec!["example.com".to_owned()]);
    let context = SandboxContext::new(runtime, policy);
    let spec = ProcessSpec::shell(&ws, "echo hi")
        .timeout(Duration::from_secs(10))
        .sandbox(context);
    let outcome = ProcessRunner::run(&spec, &|| true);
    assert_eq!(outcome.status, ProcessStatus::SpawnFailure, "{outcome:?}");
    let failure = outcome
        .failure
        .as_ref()
        .expect("a refusal must be reported structurally");
    assert!(failure.is_sandbox_refusal(), "{failure:?}");
    assert_eq!(failure.code(), "sandbox_unsupported", "{failure:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_real_toolchain_still_works_inside_the_sandbox() {
    // Scenario: the sandbox must not break normal development. The
    // toolchain has to be *visible* inside the confinement, and the home
    // directory has to be usable — a sandbox that makes `cargo` fail is
    // not a working sandbox.
    let runtime = SandboxRuntime::detect();
    require_backend(&runtime);
    if !runtime.is_available() {
        return;
    }
    let dir = workspace("toolchain");
    let ws = dir.join("workspace");
    let outcome = command_run_confined(
        &ws,
        "cargo --version",
        &|| true,
        30,
        &context(runtime, &dir, SandboxMode::WorkspaceWrite),
    );
    if outcome.output.contains("not found") || outcome.output.contains("No such file") {
        eprintln!("skipped-unavailable: cargo is not installed on this host");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert_eq!(
        outcome.kind,
        CommandOutcomeKind::Success,
        "the toolchain must work inside the sandbox: {outcome:?}"
    );
    assert!(
        outcome.output.contains("cargo"),
        "the toolchain version must be readable: {outcome:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_home_directory_is_usable_inside_the_sandbox() {
    // The confined process gets a private, writable home. A tool that
    // insists on one must find it — otherwise every build breaks.
    let runtime = SandboxRuntime::detect();
    require_backend(&runtime);
    if !runtime.is_available() {
        return;
    }
    let dir = workspace("home");
    let ws = dir.join("workspace");
    let outcome = command_run_confined(
        &ws,
        "printf 'home=%s\\n' \"$HOME\" && touch \"$HOME/.probe\" && test -w \"$HOME\"",
        &|| true,
        30,
        &context(runtime, &dir, SandboxMode::WorkspaceWrite),
    );
    assert_eq!(
        outcome.kind,
        CommandOutcomeKind::Success,
        "the private home must exist and be writable: {outcome:?}"
    );
    assert!(outcome.output.contains("home="), "{outcome:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_real_cargo_build_works_inside_the_sandbox() {
    // The end-to-end usability check: a project build must succeed inside
    // the confinement — the workspace is writable, the toolchain is
    // readable, and the private home does not break the build.
    let runtime = SandboxRuntime::detect();
    require_backend(&runtime);
    if !runtime.is_available() {
        return;
    }
    let dir = workspace("build");
    let ws = dir.join("workspace");
    std::fs::write(
        ws.join("Cargo.toml"),
        "[package]\nname = \"sbxbuild\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::write(
        ws.join("src/main.rs"),
        "fn main() { println!(\"built\"); }\n",
    )
    .unwrap();

    let outcome = command_run_confined(
        &ws,
        "cargo build --offline 2>&1 | tail -3",
        &|| true,
        180,
        &context(runtime, &dir, SandboxMode::WorkspaceWrite),
    );
    if outcome.output.contains("not found") || outcome.output.contains("No such file") {
        eprintln!("skipped-unavailable: cargo is not installed on this host");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert_eq!(
        outcome.kind,
        CommandOutcomeKind::Success,
        "a normal build must work inside the sandbox: {outcome:?}"
    );
    assert!(
        ws.join("target").exists(),
        "the build must have produced its output: {outcome:?}"
    );
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
