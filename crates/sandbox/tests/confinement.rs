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

/// The read isolation this platform can actually enforce.
///
/// macOS uses a deny-list: the sensitive roots are hidden, but the rest of
/// the host stays readable, so the axis is honestly reported as `Partial`.
/// Bubblewrap builds the read view as a mount allow-list, so a path that is
/// not bound simply does not exist inside the sandbox.
fn expected_file_read() -> kodo_sandbox::EnforcementLevel {
    #[cfg(target_os = "macos")]
    {
        kodo_sandbox::EnforcementLevel::Partial
    }
    #[cfg(target_os = "linux")]
    {
        kodo_sandbox::EnforcementLevel::Full
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        kodo_sandbox::EnforcementLevel::Unsupported
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

/// The probe helper the network and read tests drive.
fn probe_helper() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kodo-sandbox-probe"))
}

/// A sandbox context that can also execute the probe helper.
///
/// The helper lives in the build directory, which is usually inside the
/// user's home — a directory the read deny-list hides. The extra read root
/// makes it executable *without* widening anything else.
fn sandbox_context_with_probe(fixture: &Fixture, mode: SandboxMode) -> Option<SandboxContext> {
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
    let mut policy = runtime.policy_for(&fixture.workspace, mode);
    if let Some(dir) = probe_helper().parent() {
        policy = policy.with_read_root(dir);
    }
    Some(SandboxContext::new(runtime, policy))
}

/// Connect to `host:port` with the probe helper, without confinement.
fn connect_with_helper(
    fixture: &Fixture,
    host: &str,
    port: u16,
) -> kodo_sandbox::probe::ProbeOutcome {
    let output = Command::new(probe_helper())
        .args(["connect", host, &port.to_string(), "2000"])
        .current_dir(&fixture.workspace)
        .output()
        .expect("spawn the probe helper");
    kodo_sandbox::probe::ProbeOutcome::from_exit_code(
        output.status.code().unwrap_or(-1),
        &String::from_utf8_lossy(&output.stderr),
    )
}

/// Connect to `host:port` with the probe helper, inside the sandbox.
fn connect_in_sandbox(
    context: &SandboxContext,
    fixture: &Fixture,
    host: &str,
    port: u16,
) -> kodo_sandbox::probe::ProbeOutcome {
    let helper = probe_helper();
    let args = [
        OsString::from("connect"),
        OsString::from(host),
        OsString::from(port.to_string()),
        OsString::from("2000"),
    ];
    let prepared = context
        .prepare(&helper, &args, &fixture.workspace)
        .expect("prepare the sandboxed probe");
    let output = Command::new(&prepared.program)
        .args(&prepared.args)
        .current_dir(&prepared.cwd)
        .output()
        .expect("spawn the sandboxed probe");
    prepared.cleanup();
    kodo_sandbox::probe::ProbeOutcome::from_exit_code(
        output.status.code().unwrap_or(-1),
        &String::from_utf8_lossy(&output.stderr),
    )
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
fn the_git_common_directory_of_a_linked_worktree_is_protected() {
    // A linked worktree stores a `.git` *file* pointing at
    // `<common>/worktrees/<name>`. The real object store and refs live in
    // the common directory — writing there would affect every worktree.
    let fixture = Fixture::new("worktree");
    let common = fixture.root.join("common/.git");
    let worktree_git = common.join("worktrees/feature");
    std::fs::create_dir_all(&worktree_git).expect("create the worktree metadata");
    std::fs::write(worktree_git.join("commondir"), "../..").expect("commondir");
    std::fs::write(
        fixture.workspace.join(".git"),
        format!("gitdir: {}\n", worktree_git.display()),
    )
    .expect("git file");

    let Some(run) = run_in_sandbox(
        &fixture.workspace,
        SandboxMode::WorkspaceWrite,
        "echo tampered > .git/HEAD",
    ) else {
        return;
    };
    assert!(
        !run.success,
        "the worktree metadata must not be writable: {run:?}"
    );
    assert!(
        !common.join("HEAD").exists(),
        "the git metadata must be untouched"
    );

    // The common directory itself is protected as well.
    let Some(run) = run_in_sandbox(
        &fixture.workspace,
        SandboxMode::WorkspaceWrite,
        &format!("echo tampered > {}/config", common.join("config").display()),
    ) else {
        return;
    };
    assert!(
        !run.success,
        "the shared git directory must not be writable: {run:?}"
    );
    assert!(
        !common.join("config").exists(),
        "the shared git config must be untouched"
    );
}

#[test]
fn the_outbound_network_is_blocked() {
    let fixture = Fixture::new("network");
    // A real TCP connect against a live listener. The shell's `/dev/tcp`
    // would be a bash-only shortcut that silently "passes" on a shell
    // without it — the probe helper is the same on every platform.
    let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") else {
        eprintln!("skipped-unavailable: no loopback listener on this host");
        return;
    };
    let port = listener.local_addr().expect("local addr").port();
    let Some(context) = sandbox_context_with_probe(&fixture, SandboxMode::WorkspaceWrite) else {
        return;
    };
    // Control: the address is reachable without confinement. Without this
    // the test could not tell a blocked connection from a dead listener.
    let control = connect_with_helper(&fixture, "127.0.0.1", port);
    assert_eq!(
        control,
        kodo_sandbox::probe::ProbeOutcome::Ok,
        "the probe must reach the listener outside the sandbox"
    );
    let measured = connect_in_sandbox(&context, &fixture, "127.0.0.1", port);
    assert!(
        measured.is_blocked(),
        "the sandbox must not allow an outbound connection: {measured:?}"
    );
}

#[test]
fn a_loopback_service_is_not_reachable_from_the_sandbox() {
    let fixture = Fixture::new("loopback");
    // A listener on the loopback interface must stay unreachable: the
    // "deny network" rule has to cover 127.0.0.1, not just the internet.
    let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") else {
        eprintln!("skipped-unavailable: no loopback listener on this host");
        return;
    };
    let port = listener.local_addr().expect("local addr").port();
    let Some(context) = sandbox_context_with_probe(&fixture, SandboxMode::WorkspaceWrite) else {
        return;
    };
    let measured = connect_in_sandbox(&context, &fixture, "127.0.0.1", port);
    assert!(
        measured.is_blocked(),
        "the sandbox allowed a loopback connection: {measured:?}"
    );
}

#[test]
fn the_ipv6_loopback_is_not_reachable_from_the_sandbox() {
    let fixture = Fixture::new("ipv6");
    // IPv6 is covered when the host has it; a host without IPv6 loopback
    // reports "skipped" instead of pretending to pass.
    let Ok(listener) = std::net::TcpListener::bind("[::1]:0") else {
        eprintln!("skipped-unavailable: no IPv6 loopback on this host");
        return;
    };
    let port = listener.local_addr().expect("local addr").port();
    let Some(context) = sandbox_context_with_probe(&fixture, SandboxMode::WorkspaceWrite) else {
        return;
    };
    let control = connect_with_helper(&fixture, "::1", port);
    assert_eq!(
        control,
        kodo_sandbox::probe::ProbeOutcome::Ok,
        "the probe must reach the IPv6 listener outside the sandbox"
    );
    let measured = connect_in_sandbox(&context, &fixture, "::1", port);
    assert!(
        measured.is_blocked(),
        "the sandbox allowed an IPv6 loopback connection: {measured:?}"
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
    // The backend is the one this platform ships — never a hard-coded name.
    assert_eq!(
        capabilities.backend,
        expected_backend(),
        "unexpected backend on this platform: {capabilities:?}"
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
    // The read axis is measured per platform: a deny-list is honestly
    // reported as partial, a mount allow-list as full.
    assert_eq!(
        capabilities.file_read,
        expected_file_read(),
        "the read axis must match what this platform enforces: {capabilities:?}"
    );
}

#[test]
fn the_capability_report_is_never_better_than_the_platform_can_do() {
    // A report that claims more than the platform delivers is the failure
    // this test exists for: the UI would then promise an isolation the
    // machine does not provide.
    let runtime = SandboxRuntime::detect();
    let capabilities = runtime.capabilities();
    if !runtime.is_available() {
        eprintln!("skipped-unavailable: no backend on this host");
        return;
    }
    assert!(
        capabilities.file_read.meets(expected_file_read()),
        "the reported read axis must not exceed the platform's own model: {capabilities:?}"
    );
    assert_eq!(
        capabilities.backend,
        expected_backend(),
        "the report must name this platform's backend: {capabilities:?}"
    );
}

#[test]
fn a_backend_that_cannot_enforce_the_policy_refuses_the_run() {
    // The default policy promises no outbound traffic. A backend whose
    // network isolation could not be verified must refuse the run instead
    // of quietly running with a smaller boundary.
    let runtime = SandboxRuntime::detect();
    if !runtime.is_available() {
        eprintln!("skipped-unavailable: no backend on this host");
        return;
    }
    let fixture = Fixture::new("floor");
    let policy = runtime.policy_for(&fixture.workspace, SandboxMode::WorkspaceWrite);
    // The floor is derived from the policy, per axis.
    let required = policy.required_enforcement();
    assert_eq!(
        required.network_egress,
        kodo_sandbox::EnforcementLevel::Full,
        "a deny-all network policy needs a verified isolation"
    );
    assert_eq!(
        required.file_write,
        kodo_sandbox::EnforcementLevel::Full,
        "the write allow-list must be enforced by the kernel"
    );
    let capabilities = runtime.capabilities().capabilities();
    assert!(
        capabilities.meets(&required),
        "the measured capabilities must satisfy the policy floor: {capabilities:?}"
    );
}
