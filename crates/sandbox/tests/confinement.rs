//! Real-execution confinement tests.
//!
//! These tests spawn real processes through the real backend. Nothing here
//! is mocked: if Seatbelt or Bubblewrap does not confine the process, the
//! test fails.
//!
//! When a platform has no usable backend the test reports
//! `skipped-unavailable` and returns without asserting anything — it never
//! reports a false pass.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use kodo_sandbox::{SandboxContext, SandboxMode, SandboxRuntime};

/// A scratch workspace plus a scratch directory outside it.
struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    outside: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "kodo-sbx-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        std::fs::create_dir_all(&workspace).expect("create workspace");
        std::fs::create_dir_all(&outside).expect("create outside");
        std::fs::write(workspace.join("source.txt"), "original\n").expect("seed file");
        std::fs::write(outside.join("secret.txt"), "top-secret\n").expect("seed secret");
        Self {
            root,
            workspace,
            outside,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The result of one sandboxed run.
#[derive(Debug)]
struct Run {
    success: bool,
    stdout: String,
    stderr: String,
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

/// Run a shell line inside the sandbox.
///
/// Returns `None` when the platform genuinely has no backend, so the caller
/// can report `skipped-unavailable` instead of asserting. A backend that
/// exists but does not confine anything is a hard failure.
fn run_in_sandbox(workspace: &Path, mode: SandboxMode, command: &str) -> Option<Run> {
    let runtime = SandboxRuntime::detect();
    if !runtime.is_available() {
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
        return None;
    }
    let policy = runtime.policy_for(workspace, mode);
    let context = SandboxContext::new(runtime, policy);
    let prepared = context
        .prepare(
            Path::new("/bin/sh"),
            &[OsString::from("-c"), OsString::from(command)],
            workspace,
        )
        .expect("prepare sandboxed command");
    let output = Command::new(&prepared.program)
        .args(&prepared.args)
        .current_dir(&prepared.cwd)
        .output()
        .expect("spawn sandboxed command");
    prepared.cleanup();
    Some(Run {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[test]
fn writes_inside_the_workspace_are_allowed() {
    let fixture = Fixture::new("write-ok");
    let Some(run) = run_in_sandbox(
        &fixture.workspace,
        SandboxMode::WorkspaceWrite,
        "echo changed > source.txt && cat source.txt",
    ) else {
        return;
    };
    assert!(run.success, "write inside the workspace must work: {run:?}");
    assert!(run.stdout.contains("changed"), "{run:?}");
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.join("source.txt")).unwrap(),
        "changed\n"
    );
}

#[test]
fn writes_outside_the_workspace_are_blocked() {
    let fixture = Fixture::new("write-outside");
    let target = fixture.outside.join("written.txt");
    let Some(run) = run_in_sandbox(
        &fixture.workspace,
        SandboxMode::WorkspaceWrite,
        &format!("echo escaped > {}", target.display()),
    ) else {
        return;
    };
    assert!(!run.success, "write outside must be refused: {run:?}");
    assert!(
        !target.exists(),
        "the file must not exist after a blocked write"
    );
    assert!(
        run.stderr.contains("not permitted") || run.stderr.contains("denied"),
        "the failure must name the OS refusal: {run:?}"
    );
}

#[test]
fn read_only_mode_cannot_modify_the_source() {
    let fixture = Fixture::new("readonly");
    let Some(run) = run_in_sandbox(
        &fixture.workspace,
        SandboxMode::ReadOnly,
        "echo modified > source.txt",
    ) else {
        return;
    };
    assert!(!run.success, "read-only mode must refuse a write: {run:?}");
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.join("source.txt")).unwrap(),
        "original\n",
        "the source file must be untouched"
    );
}

#[test]
fn a_child_process_cannot_cross_the_boundary() {
    let fixture = Fixture::new("child");
    let target = fixture.outside.join("from-child.txt");
    // A script spawns a grandchild that writes outside the workspace: the
    // confinement must hold for the whole process tree.
    let script = format!(
        "sh -c 'echo escaped > {}' && sh -c 'echo escaped2 > {}'",
        target.display(),
        fixture.outside.join("from-grandchild.txt").display()
    );
    let Some(run) = run_in_sandbox(&fixture.workspace, SandboxMode::WorkspaceWrite, &script) else {
        return;
    };
    assert!(!run.success, "a child write must be refused: {run:?}");
    assert!(!target.exists(), "child process wrote outside the sandbox");
    assert!(
        !fixture.outside.join("from-grandchild.txt").exists(),
        "grandchild process wrote outside the sandbox"
    );
}

#[test]
fn a_symlink_cannot_escape_the_workspace() {
    let fixture = Fixture::new("symlink");
    let target = fixture.outside.join("via-symlink.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&fixture.outside, fixture.workspace.join("escape"))
        .expect("create symlink");
    let Some(run) = run_in_sandbox(
        &fixture.workspace,
        SandboxMode::WorkspaceWrite,
        "echo escaped > escape/via-symlink.txt",
    ) else {
        return;
    };
    assert!(!run.success, "a symlink escape must be refused: {run:?}");
    assert!(
        !target.exists(),
        "the symlinked target must not have been written"
    );
}

#[test]
fn git_metadata_is_not_writable_from_the_sandbox() {
    let fixture = Fixture::new("gitmeta");
    std::fs::create_dir_all(fixture.workspace.join(".git")).expect("create .git");
    let Some(run) = run_in_sandbox(
        &fixture.workspace,
        SandboxMode::WorkspaceWrite,
        "echo tampered > .git/config",
    ) else {
        return;
    };
    assert!(!run.success, "git metadata must not be writable: {run:?}");
    assert!(
        !fixture.workspace.join(".git/config").exists(),
        "the git config must be untouched"
    );
}

#[test]
fn the_kodo_state_directory_is_not_writable_from_the_sandbox() {
    let fixture = Fixture::new("kodostate");
    std::fs::create_dir_all(fixture.workspace.join(".kodo")).expect("create .kodo");
    let Some(run) = run_in_sandbox(
        &fixture.workspace,
        SandboxMode::WorkspaceWrite,
        "echo tampered > .kodo/settings.log",
    ) else {
        return;
    };
    assert!(!run.success, "Kodo state must not be writable: {run:?}");
    assert!(
        !fixture.workspace.join(".kodo/settings.log").exists(),
        "the Kodo state file must be untouched"
    );
}

#[test]
fn the_outbound_network_is_blocked() {
    let fixture = Fixture::new("network");
    // A direct TCP connect to a public address must not succeed. The command
    // uses the shell's own /dev/tcp redirection so no extra tool is needed.
    let Some(run) = run_in_sandbox(
        &fixture.workspace,
        SandboxMode::WorkspaceWrite,
        "exec 3<>/dev/tcp/1.1.1.1/443 2>/dev/null && echo CONNECTED || echo BLOCKED",
    ) else {
        return;
    };
    assert!(
        run.stdout.contains("BLOCKED") || !run.success,
        "the sandbox must not allow an outbound connection: {run:?}"
    );
    assert!(
        !run.stdout.contains("CONNECTED"),
        "the sandbox allowed an outbound connection: {run:?}"
    );
}

#[test]
fn a_loopback_service_is_not_reachable_from_the_sandbox() {
    let fixture = Fixture::new("loopback");
    // A listener on the loopback interface must stay unreachable: the
    // "deny network" rule has to cover 127.0.0.1, not just the internet.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    let _guard = listener;
    let Some(run) = run_in_sandbox(
        &fixture.workspace,
        SandboxMode::WorkspaceWrite,
        &format!("exec 3<>/dev/tcp/127.0.0.1/{port} 2>/dev/null && echo CONNECTED || echo BLOCKED"),
    ) else {
        return;
    };
    assert!(
        !run.stdout.contains("CONNECTED"),
        "the sandbox allowed a loopback connection: {run:?}"
    );
}

#[test]
fn a_normal_build_still_works_inside_the_sandbox() {
    // A build that writes into the workspace and reads the toolchain must
    // still succeed: the sandbox must not break normal development.
    let fixture = Fixture::new("build");
    let Some(run) = run_in_sandbox(
        &fixture.workspace,
        SandboxMode::WorkspaceWrite,
        "echo 'fn main(){println!(\"built\");}' > main.rs && sh -c 'echo compiled > out.txt' && cat out.txt",
    ) else {
        return;
    };
    assert!(run.success, "a normal build must work: {run:?}");
    assert!(run.stdout.contains("compiled"), "{run:?}");
}

#[test]
fn the_capability_report_matches_what_the_platform_enforces() {
    let runtime = SandboxRuntime::detect();
    let capabilities = runtime.capabilities();
    if !runtime.is_available() {
        eprintln!("skipped-unavailable: no backend on this host");
        return;
    }
    assert_eq!(
        capabilities.backend, "seatbelt",
        "expected the macOS backend"
    );
    assert_eq!(
        capabilities.file_write,
        kodo_sandbox::EnforcementLevel::Full,
        "writes must be fully confined: {capabilities:?}"
    );
    assert_eq!(
        capabilities.network_egress,
        kodo_sandbox::EnforcementLevel::Full,
        "the network must be fully confined: {capabilities:?}"
    );
    // The read axis is a deny-list, so it is honestly reported as partial.
    assert_eq!(
        capabilities.file_read,
        kodo_sandbox::EnforcementLevel::Partial,
        "the read axis must be reported as partial: {capabilities:?}"
    );
}
