//! The backend contract: a backend *prepares* a restricted command, it never
//! runs one.
//!
//! `prepare()` returns a [`PreparedCommand`] — a program, an argv vector and
//! the environment additions the backend needs. The caller (Kodo's
//! `ProcessRunner`) owns spawning, cancellation, timeouts and output limits,
//! so there is exactly one process manager in the product.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::capabilities::{CapabilityReport, EnforcementCapabilities};
use crate::policy::SandboxPolicy;

/// Why a sandboxed run could not be prepared.
///
/// Every variant is a **refusal**. A backend that cannot honour the policy
/// must surface one of these instead of returning an unrestricted command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    /// No usable confinement backend on this host.
    Unavailable { reason: String },
    /// A backend exists but cannot meet the policy's enforcement floor.
    PartialEnforcement {
        reason: String,
        capabilities: EnforcementCapabilities,
    },
    /// The policy cannot be expressed by the available backend (for example a
    /// domain allow-list, which no P0 backend can enforce).
    Unsupported { reason: String },
    /// The backend could not build its confinement artefacts.
    Io { reason: String },
}

impl SandboxError {
    /// Stable, machine-readable reason for the audit log and the UI.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "sandbox_unavailable",
            Self::PartialEnforcement { .. } => "sandbox_partial_enforcement",
            Self::Unsupported { .. } => "sandbox_unsupported",
            Self::Io { .. } => "sandbox_io_error",
        }
    }
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable { reason } => {
                write!(f, "沙箱不可用，已拒绝执行：{reason}")
            }
            Self::PartialEnforcement { reason, .. } => {
                write!(f, "沙箱强制能力不足，已拒绝执行：{reason}")
            }
            Self::Unsupported { reason } => {
                write!(f, "当前平台不支持该沙箱策略，已拒绝执行：{reason}")
            }
            Self::Io { reason } => {
                write!(f, "沙箱准备失败，已拒绝执行：{reason}")
            }
        }
    }
}

impl std::error::Error for SandboxError {}

/// A confined command, ready to be spawned by the process runner.
///
/// The wrapper is expressed as `program` + `argv`, never as a shell string, so
/// a hostile command line cannot inject arguments into the sandbox launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommand {
    /// The executable to spawn — the sandbox launcher, not the target.
    pub program: PathBuf,
    /// Full argument vector, ending with the target program and its args.
    pub args: Vec<OsString>,
    /// Environment additions the backend needs (a private `HOME`, `TMPDIR`…).
    pub env: Vec<(String, String)>,
    /// Working directory for the confined process.
    pub cwd: PathBuf,
    /// A private artefact (Seatbelt profile) that must be removed after the
    /// run. It lives outside every writable root, so the confined process
    /// cannot rewrite its own rules.
    pub profile_path: Option<PathBuf>,
}

impl PreparedCommand {
    pub fn new(program: impl Into<PathBuf>, args: Vec<OsString>, cwd: PathBuf) -> Self {
        Self {
            program: program.into(),
            args,
            env: Vec::new(),
            cwd,
            profile_path: None,
        }
    }

    /// Remove the backend's private artefacts. Safe to call more than once.
    pub fn cleanup(&self) {
        if let Some(path) = &self.profile_path {
            let _ = std::fs::remove_file(path);
            if let Some(parent) = path.parent() {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
}

/// A platform backend that can confine a command.
pub trait SandboxBackend: Send + Sync {
    /// Stable identifier, e.g. `seatbelt`.
    fn id(&self) -> &'static str;

    /// What this backend enforces on this host, measured by a functional probe
    /// rather than by the presence of a binary.
    fn inspect(&self) -> CapabilityReport;

    /// Build the confined command. Returning `Err` means the command must not
    /// run.
    fn prepare(
        &self,
        program: &Path,
        args: &[OsString],
        policy: &SandboxPolicy,
        cwd: &Path,
    ) -> Result<PreparedCommand, SandboxError>;
}
