//! OS-level execution confinement for Kodo.
//!
//! Kodo runs project code — build scripts, test harnesses, package
//! installers — that the model chose. Approving a command answers "may this
//! start?", not "what may it touch?". This crate answers the second question
//! by confining the process at the operating-system level, so the limits hold
//! for every descendant the command spawns.
//!
//! Three ideas hold the design together:
//!
//! 1. **Orthogonal axes.** Approval (`Ask` / `Auto` / `Full`) decides whether
//!    a human is asked. The sandbox decides what the process may reach. A
//!    `Full` approval never widens the sandbox.
//! 2. **Fail closed.** If the backend is missing, broken or too weak, the
//!    run is refused. There is no path from "sandbox failed" to "run it
//!    anyway".
//! 3. **Report, do not assume.** File read, file write and network egress
//!    are measured independently and reported separately, so the UI never
//!    claims an isolation it does not have.
//!
//! ```no_run
//! use kodo_sandbox::{SandboxMode, SandboxRuntime};
//! use std::ffi::OsString;
//!
//! let runtime = SandboxRuntime::detect();
//! let policy = runtime.policy_for(std::path::Path::new("/work/project"), SandboxMode::WorkspaceWrite);
//! let prepared = runtime.prepare(
//!     std::path::Path::new("/bin/sh"),
//!     &[OsString::from("-c"), OsString::from("cargo test")],
//!     &policy,
//!     std::path::Path::new("/work/project"),
//! )?;
//! // `prepared.program` / `prepared.args` are what the process runner spawns.
//! # Ok::<(), kodo_sandbox::SandboxError>(())
//! ```

pub mod audit;
pub mod backend;
pub mod capabilities;
pub mod grant;
pub mod linux;
pub mod macos;
pub mod policy;
pub mod probe;
pub mod runtime;

pub use audit::{SandboxEvent, SandboxEventKind};
pub use backend::{PreparedCommand, SandboxBackend, SandboxError};
pub use capabilities::{CapabilityReport, EnforcementCapabilities, EnforcementLevel};
pub use grant::{operation_digest, GrantScope, GrantStore, GrantWidening, OneShotGrant};
pub use policy::{NetworkPolicy, SandboxMode, SandboxPolicy};
pub use runtime::{SandboxContext, SandboxRuntime};

/// Version of the secure-execution contract implemented by this crate.
pub const SANDBOX_VERSION: &str = "V1.1";
