//! Shared shell layer: the pieces every client of `kodo-agent` needs but
//! `kodo-agent` itself must not own — session-log envelope writes, approval
//! fingerprints, and loading provider/permission settings. The desktop shell
//! and the CLI both build on this; GUI-only wiring (event emit, threads,
//! approval rendezvous) stays in each client.

pub mod approve;
pub mod config;
pub mod turn;

pub use approve::{approval_fingerprint, step_kind_label, ApprovalChoice};
pub use config::{permission_from_settings, Overrides, ShellConfig};
pub use turn::{Logged, TurnLogger, TurnOutcome};
