//! Shared shell layer: the pieces every client of `kodo-agent` needs but
//! `kodo-agent` itself must not own — session-log envelope writes, approval
//! fingerprints, and loading provider/permission settings. The desktop shell
//! and the CLI both build on this; GUI-only wiring (event emit, threads,
//! approval rendezvous) stays in each client.

pub mod approve;
pub mod apps;
pub mod config;
pub mod memory_outbox;
pub mod scheduler;
pub mod scheduler_jobs;
pub mod turn;

pub use approve::{approval_fingerprint, step_kind_label, ApprovalChoice};
pub use apps::AppStore;
pub use config::{permission_from_settings, Overrides, ShellConfig};
pub use memory_outbox::{event_for_item, open_default_handle, write_event};
pub use scheduler::{
    recover_inflight, AdmissionLimits, DispatchedRun, DispatcherConfig, Job, JobCancellation,
    JobContext, JobOutcome, JobRegistry, JobReport, ResourceGuard, ResourceHandle, Scheduler,
    SchedulerError, SchedulerStore, TaskSummary,
};
pub use scheduler_jobs::{
    default_job_registry, default_job_registry_handlers, memory_outbox_replay, workspace_refresh,
};
pub use turn::{Logged, TurnLogger, TurnOutcome};
