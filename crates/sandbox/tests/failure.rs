//! Fail-closed tests: when the sandbox cannot be established, the command
//! must not run at all.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kodo_sandbox::{
    CapabilityReport, EnforcementCapabilities, EnforcementLevel, PreparedCommand, SandboxBackend,
    SandboxContext, SandboxError, SandboxMode, SandboxPolicy, SandboxRuntime,
};

/// A backend that always refuses to prepare a command.
struct BrokenBackend;

impl SandboxBackend for BrokenBackend {
    fn id(&self) -> &'static str {
        "broken"
    }

    fn inspect(&self) -> CapabilityReport {
        CapabilityReport {
            backend: "broken".to_owned(),
            file_read: EnforcementLevel::Unsupported,
            file_write: EnforcementLevel::Unsupported,
            network_egress: EnforcementLevel::Unsupported,
            notes: vec!["测试用后端：始终拒绝".to_owned()],
        }
    }

    fn prepare(
        &self,
        _program: &Path,
        _args: &[OsString],
        _policy: &SandboxPolicy,
        _cwd: &Path,
    ) -> Result<PreparedCommand, SandboxError> {
        Err(SandboxError::Unavailable {
            reason: "测试注入的沙箱故障".to_owned(),
        })
    }
}

#[test]
fn a_broken_backend_refuses_instead_of_running_the_command() {
    let runtime = SandboxRuntime::with_backend(Arc::new(BrokenBackend));
    let workspace = std::env::temp_dir().join(format!("kodo-sbx-fail-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&workspace);
    let marker = workspace.join("must-not-exist.txt");
    let _ = std::fs::remove_file(&marker);

    let policy = runtime.policy_for(&workspace, SandboxMode::WorkspaceWrite);
    let context = SandboxContext::new(runtime, policy);
    let prepared = context.prepare(
        Path::new("/bin/sh"),
        &[
            OsString::from("-c"),
            OsString::from(format!("echo ran > {}", marker.display())),
        ],
        &workspace,
    );
    assert!(
        prepared.is_err(),
        "a broken backend must not produce a runnable command"
    );
    let error = prepared.expect_err("must refuse");
    assert_eq!(error.code(), "sandbox_unavailable");
    assert!(!marker.exists(), "the command must never have started");
    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn a_broken_backend_is_reported_as_unavailable() {
    let runtime = SandboxRuntime::with_backend(Arc::new(BrokenBackend));
    assert!(!runtime.is_available());
    let capabilities = runtime.capabilities();
    assert_eq!(capabilities.file_write, EnforcementLevel::Unsupported);
    assert_eq!(capabilities.network_egress, EnforcementLevel::Unsupported);
    assert!(!capabilities.capabilities().is_full());
}

#[test]
fn a_policy_requiring_full_enforcement_is_refused_by_a_partial_backend() {
    // The Seatbelt backend reports the read axis as partial. A policy that
    // demands full enforcement must therefore be refused rather than
    // downgraded.
    let runtime = SandboxRuntime::detect();
    if !runtime.is_available() {
        eprintln!("skipped-unavailable: no backend on this host");
        return;
    }
    let workspace = std::env::temp_dir().join(format!("kodo-sbx-floor-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&workspace);
    let mut policy = runtime.policy_for(&workspace, SandboxMode::WorkspaceWrite);
    policy.require_full_enforcement = true;
    let context = SandboxContext::new(runtime, policy);
    let prepared = context.prepare(
        Path::new("/bin/sh"),
        &[OsString::from("-c"), OsString::from("echo hi")],
        &workspace,
    );
    match prepared {
        Ok(_) => {
            // The backend met the floor — acceptable, and the report must say
            // so.
            let capabilities = SandboxRuntime::detect().capabilities().clone();
            assert!(capabilities.capabilities().is_full());
        }
        Err(error) => {
            assert_eq!(error.code(), "sandbox_partial_enforcement");
        }
    }
    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn a_domain_allow_list_is_refused_rather_than_faked() {
    // The P0 backends cannot enforce a domain allow-list. They must refuse
    // it instead of pretending a proxy variable is a boundary.
    let runtime = SandboxRuntime::detect();
    if !runtime.is_available() {
        eprintln!("skipped-unavailable: no backend on this host");
        return;
    }
    let workspace = std::env::temp_dir().join(format!("kodo-sbx-dom-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&workspace);
    let mut policy = runtime.policy_for(&workspace, SandboxMode::WorkspaceWrite);
    policy.network = kodo_sandbox::NetworkPolicy::AllowDomains(vec!["example.com".to_owned()]);
    let context = SandboxContext::new(runtime, policy);
    let prepared = context.prepare(
        Path::new("/bin/sh"),
        &[OsString::from("-c"), OsString::from("echo hi")],
        &workspace,
    );
    let error = prepared.expect_err("a domain allow-list must be refused");
    assert_eq!(error.code(), "sandbox_unsupported");
    let _ = std::fs::remove_dir_all(&workspace);
}

/// A backend whose network isolation could not be verified.
struct WeakNetworkBackend;

impl SandboxBackend for WeakNetworkBackend {
    fn id(&self) -> &'static str {
        "weak-network"
    }

    fn inspect(&self) -> CapabilityReport {
        CapabilityReport {
            backend: "weak-network".to_owned(),
            // Writes are confined, but the network axis was never verified.
            file_read: EnforcementLevel::Partial,
            file_write: EnforcementLevel::Full,
            network_egress: EnforcementLevel::Unsupported,
            notes: vec!["测试用后端：网络隔离未通过探测".to_owned()],
        }
    }

    fn prepare(
        &self,
        program: &Path,
        args: &[OsString],
        policy: &SandboxPolicy,
        cwd: &Path,
    ) -> Result<PreparedCommand, SandboxError> {
        // The real backends check the policy floor before they build the
        // confinement. This one does the same, so the refusal is produced by
        // the shared rule rather than by the test.
        let required = policy.required_enforcement();
        let capabilities = self.inspect().capabilities();
        if !capabilities.meets(&required) {
            let missing = capabilities.missing(&required);
            return Err(SandboxError::PartialEnforcement {
                reason: format!("本机强制能力不足，缺少：{}", missing.join("、")),
                capabilities,
            });
        }
        Ok(PreparedCommand::new(
            program.to_path_buf(),
            args.to_vec(),
            cwd.to_path_buf(),
        ))
    }
}

#[test]
fn an_unverified_network_isolation_refuses_the_run() {
    // The policy promises no outbound traffic. A backend that could not
    // verify that promise must refuse the run — it must not fall back to an
    // unconfined execution.
    let runtime = SandboxRuntime::with_backend(Arc::new(WeakNetworkBackend));
    let workspace = std::env::temp_dir().join(format!("kodo-sbx-net-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&workspace);
    let marker = workspace.join("must-not-exist.txt");
    let _ = std::fs::remove_file(&marker);

    let policy = runtime.policy_for(&workspace, SandboxMode::WorkspaceWrite);
    let context = SandboxContext::new(runtime, policy);
    let prepared = context.prepare(
        Path::new("/bin/sh"),
        &[
            OsString::from("-c"),
            OsString::from(format!("echo ran > {}", marker.display())),
        ],
        &workspace,
    );
    let error = prepared.expect_err("an unverified network isolation must refuse");
    assert_eq!(error.code(), "sandbox_partial_enforcement");
    assert!(
        error.to_string().contains("network_egress"),
        "the refusal must name the missing axis: {error}"
    );
    assert!(!marker.exists(), "the command must never have started");
    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn a_missing_sandbox_is_not_a_license_to_run_unconfined() {
    // The runtime never downgrades a refusal into an unconfined run. The
    // only way out is an explicit, user-approved one-shot escalation.
    let runtime = SandboxRuntime::with_backend(Arc::new(BrokenBackend));
    let workspace = std::env::temp_dir().join(format!("kodo-sbx-downgrade-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&workspace);
    let policy = runtime.policy_for(&workspace, SandboxMode::WorkspaceWrite);
    let context = SandboxContext::new(runtime, policy);
    let prepared = context.prepare(
        Path::new("/bin/sh"),
        &[OsString::from("-c"), OsString::from("echo hi")],
        &workspace,
    );
    assert!(
        prepared.is_err(),
        "a broken backend must not produce a command"
    );
    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn the_unrestricted_mode_runs_without_a_backend() {
    // `unrestricted` is the explicit escape hatch: it runs the command
    // without confinement, and the policy reports that it does not confine.
    let workspace = PathBuf::from("/tmp");
    let policy = SandboxPolicy::for_workspace(&workspace).with_mode(SandboxMode::Unrestricted);
    assert!(!policy.confines());
    assert!(policy.allows_write(&workspace.join("anything.txt")));
    assert!(policy.allows_read(&PathBuf::from("/etc/passwd")));
}

#[test]
fn the_capabilities_of_a_broken_backend_are_not_full() {
    let capabilities = EnforcementCapabilities {
        file_read: EnforcementLevel::Full,
        file_write: EnforcementLevel::Unsupported,
        network_egress: EnforcementLevel::Full,
    };
    assert!(!capabilities.is_full());
    assert_eq!(capabilities.weakest(), EnforcementLevel::Unsupported);
}
