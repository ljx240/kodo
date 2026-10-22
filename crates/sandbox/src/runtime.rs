//! The sandbox runtime: one backend, one policy, one place that decides
//! whether a command may run.
//!
//! The runtime is the single source of truth shared by the CLI, the desktop
//! app and (later) a TUI. It owns the private scratch area, resolves the
//! platform backend, and hands out [`SandboxContext`] values that the process
//! runner applies before spawning.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::backend::{PreparedCommand, SandboxBackend, SandboxError};
use crate::capabilities::CapabilityReport;
use crate::grant::{GrantScope, GrantStore, OneShotGrant};
use crate::policy::{git_metadata_roots, SandboxMode, SandboxPolicy};

/// Root of the per-process private area: `kodo-sandbox-<pid>`.
static SESSION_ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
/// Counter for distinct private sub-directories.
static AREA_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The private area of one session: a root directory plus the writable
/// scratch directories the confined process may use.
///
/// The root itself is **not** writable by the confined process, so a hostile
/// process cannot rewrite the Seatbelt profile that confines it.
#[derive(Debug, Clone)]
pub struct Scratch {
    /// Writable scratch directory (temporary files, build output).
    pub work: PathBuf,
    /// Writable private home directory for tools that insist on one.
    pub home: PathBuf,
}

/// Create a fresh private directory under the session root.
pub fn private_dir(tag: &str) -> std::io::Result<PathBuf> {
    let root = session_root()?;
    let index = AREA_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = root.join(format!("{tag}-{index}"));
    std::fs::create_dir_all(&dir)?;
    set_dir_mode(&dir, 0o700);
    Ok(dir)
}

/// The per-process private root, created once.
fn session_root() -> std::io::Result<PathBuf> {
    let root = SESSION_ROOT
        .get_or_init(|| {
            let base = std::env::temp_dir().join(format!("kodo-sandbox-{}", std::process::id()));
            if std::fs::create_dir_all(&base).is_err() {
                return None;
            }
            set_dir_mode(&base, 0o700);
            Some(base)
        })
        .clone();
    root.ok_or_else(|| std::io::Error::other("无法创建沙箱私有目录"))
}

fn set_dir_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
}

/// Detected backend for this host.
fn detect_backend() -> Arc<dyn SandboxBackend> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(crate::macos::SeatbeltBackend::new())
    }
    #[cfg(target_os = "linux")]
    {
        Arc::new(crate::linux::BubblewrapBackend::new())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Arc::new(crate::linux::UnavailableBackend::new(
            "unsupported-platform",
        ))
    }
}

/// The sandbox runtime for one process.
pub struct SandboxRuntime {
    backend: Arc<dyn SandboxBackend>,
    capabilities: CapabilityReport,
    scratch: Mutex<Option<Scratch>>,
    /// One-shot escalation grants for this process. A grant is bound to one
    /// operation in one session and is burnt the moment it is used.
    grants: GrantStore,
}

impl std::fmt::Debug for SandboxRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxRuntime")
            .field("backend", &self.capabilities.backend)
            .field("capabilities", &self.capabilities)
            .finish()
    }
}

impl SandboxRuntime {
    /// Detect the platform backend and measure what it enforces.
    pub fn detect() -> Arc<Self> {
        let backend = detect_backend();
        let capabilities = backend.inspect();
        Arc::new(Self {
            backend,
            capabilities,
            scratch: Mutex::new(None),
            grants: GrantStore::new(),
        })
    }

    /// A runtime around an explicit backend — used by tests.
    pub fn with_backend(backend: Arc<dyn SandboxBackend>) -> Arc<Self> {
        let capabilities = backend.inspect();
        Arc::new(Self {
            backend,
            capabilities,
            scratch: Mutex::new(None),
            grants: GrantStore::new(),
        })
    }

    /// Issue a one-shot escalation grant.
    pub fn issue_grant(&self, scope: GrantScope) -> OneShotGrant {
        self.grants.issue(scope)
    }

    /// Consume a grant for exactly this session and operation.
    ///
    /// Returns `None` when no grant matches, when it expired, or when it was
    /// already used. A grant never crosses a session or an operation.
    pub fn consume_grant(&self, session_id: &str, operation: &str) -> Option<OneShotGrant> {
        self.grants.consume(session_id, operation)
    }

    /// What the active backend enforces on this host.
    pub fn capabilities(&self) -> &CapabilityReport {
        &self.capabilities
    }

    pub fn backend_id(&self) -> &str {
        &self.capabilities.backend
    }

    /// True when this host can confine commands at the level the default
    /// policy requires.
    ///
    /// A host whose network isolation could not be verified is **not**
    /// available: the default policy promises no outbound traffic, and a
    /// boundary that was not measured is not a boundary.
    pub fn is_available(&self) -> bool {
        let reference = SandboxPolicy::for_workspace(std::env::temp_dir());
        self.capabilities
            .capabilities()
            .meets(&reference.required_enforcement())
    }

    /// The private scratch area, created on first use.
    fn scratch(&self) -> Result<Scratch, SandboxError> {
        let mut guard = self.scratch.lock().map_err(|_| SandboxError::Io {
            reason: "沙箱运行锁已损坏".to_owned(),
        })?;
        if let Some(scratch) = guard.as_ref() {
            return Ok(scratch.clone());
        }
        let root = private_dir("session").map_err(|error| SandboxError::Io {
            reason: error.to_string(),
        })?;
        let work = root.join("work");
        let home = root.join("home");
        std::fs::create_dir_all(&work).map_err(|error| SandboxError::Io {
            reason: error.to_string(),
        })?;
        std::fs::create_dir_all(&home).map_err(|error| SandboxError::Io {
            reason: error.to_string(),
        })?;
        set_dir_mode(&work, 0o700);
        set_dir_mode(&home, 0o700);
        let scratch = Scratch { work, home };
        *guard = Some(scratch.clone());
        Ok(scratch)
    }

    /// Build the policy for a workspace and mode.
    ///
    /// The Git metadata of the workspace (including the common directory a
    /// linked worktree points at) is added to the denied write roots, so
    /// opening the workspace for writes never opens the repository.
    pub fn policy_for(&self, workspace: &Path, mode: SandboxMode) -> SandboxPolicy {
        let mut policy = SandboxPolicy::for_workspace(workspace).with_mode(mode);
        if let Ok(scratch) = self.scratch() {
            policy = policy
                .with_write_root(&scratch.work)
                .with_write_root(&scratch.home);
        }
        for root in git_metadata_roots(workspace) {
            policy = policy.with_denied_write_root(root);
        }
        // Kodo's own state directory inside the workspace is never writable
        // from a confined process.
        policy = policy.with_denied_write_root(workspace.join(".kodo"));
        policy
    }

    /// Prepare a confined command.
    ///
    /// `Unrestricted` is an **explicit** policy decision, not an accident:
    /// the policy says the user turned the sandbox off, so the command runs
    /// unchanged. Every other mode goes through the backend, and a backend
    /// that refuses produces an error — never an unconfined run.
    pub fn prepare(
        &self,
        program: &Path,
        args: &[std::ffi::OsString],
        policy: &SandboxPolicy,
        cwd: &Path,
    ) -> Result<PreparedCommand, SandboxError> {
        if !policy.mode.confines() {
            // The user explicitly disabled the sandbox. The command runs
            // unchanged, and the audit trail records the bypass.
            return Ok(PreparedCommand::new(
                program.to_path_buf(),
                args.to_vec(),
                cwd.to_path_buf(),
            ));
        }
        let mut prepared = self.backend.prepare(program, args, policy, cwd)?;
        if let Ok(scratch) = self.scratch() {
            // A private, writable home: tools that insist on one get a
            // usable directory instead of the user's real home.
            prepared
                .env
                .push(("HOME".to_owned(), scratch.home.display().to_string()));
            prepared
                .env
                .push(("TMPDIR".to_owned(), scratch.work.display().to_string()));
            prepared
                .env
                .push(("TMP".to_owned(), scratch.work.display().to_string()));
            prepared
                .env
                .push(("TEMP".to_owned(), scratch.work.display().to_string()));
        }
        Ok(prepared)
    }
}

/// A resolved confinement decision for one run.
///
/// This is what the process runner carries: the runtime that will confine the
/// command and the policy that describes the limits.
#[derive(Clone)]
pub struct SandboxContext {
    runtime: Arc<SandboxRuntime>,
    policy: SandboxPolicy,
}

impl std::fmt::Debug for SandboxContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxContext")
            .field("backend", &self.runtime.backend_id())
            .field("mode", &self.policy.mode)
            .field("workspace", &self.policy.workspace_root)
            .finish()
    }
}

impl SandboxContext {
    pub fn new(runtime: Arc<SandboxRuntime>, policy: SandboxPolicy) -> Self {
        Self { runtime, policy }
    }

    pub fn runtime(&self) -> &Arc<SandboxRuntime> {
        &self.runtime
    }

    pub fn policy(&self) -> &SandboxPolicy {
        &self.policy
    }

    pub fn mode(&self) -> SandboxMode {
        self.policy.mode
    }

    /// Prepare the confined command. `Err` means the command must not run.
    pub fn prepare(
        &self,
        program: &Path,
        args: &[std::ffi::OsString],
        cwd: &Path,
    ) -> Result<PreparedCommand, SandboxError> {
        self.runtime.prepare(program, args, &self.policy, cwd)
    }

    /// Whether the confined execution may write to `path`.
    pub fn allows_write(&self, path: &Path) -> bool {
        self.policy.allows_write(path)
    }

    /// Whether the confined execution may read from `path`.
    pub fn allows_read(&self, path: &Path) -> bool {
        self.policy.allows_read(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::SandboxMode;

    #[test]
    fn private_areas_are_not_shared_between_calls() {
        let first = private_dir("unit").unwrap();
        let second = private_dir("unit").unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with(std::env::temp_dir()));
    }

    #[test]
    fn policy_protects_git_metadata_and_kodo_state() {
        let workspace = std::env::temp_dir().join(format!("kodo-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workspace);
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        let runtime = SandboxRuntime::detect();
        let policy = runtime.policy_for(&workspace, SandboxMode::WorkspaceWrite);
        assert!(
            policy
                .denied_write_roots
                .iter()
                .any(|root| root.ends_with(".git")),
            "{:?}",
            policy.denied_write_roots
        );
        assert!(
            policy
                .denied_write_roots
                .iter()
                .any(|root| root.ends_with(".kodo")),
            "{:?}",
            policy.denied_write_roots
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }
}
