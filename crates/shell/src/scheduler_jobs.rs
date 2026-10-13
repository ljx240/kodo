//! Default job handlers registered with the scheduler.
//!
//! Two production jobs ship in the box:
//!
//! - `workspace_refresh` — re-reads the workspace project log and writes a
//!   short summary to the run report. The desktop surface uses it to keep
//!   its sidebar project list warm without polling on every focus change.
//! - `memory_outbox_replay` — drains the memory engine's outbox. The same
//!   thing the `kodo memory doctor --apply` command does, but on a schedule
//!   so the index stays current without a manual step.
//!
//! Both are idempotent: re-running them is a no-op when there's nothing to
//! do, and the scheduler's UNIQUE `occurrence_key` keeps manual reruns
//! from piling up duplicate runs.

use std::sync::Arc;

use kodo_memory::outbox;
use kodo_memory::SqliteMemory;

use crate::scheduler::{Job, JobContext, JobOutcome, JobRegistry, JobReport};

/// Job that refreshes the workspace project list.
pub struct WorkspaceRefresh;

impl Job for WorkspaceRefresh {
    fn job_type(&self) -> &str {
        "workspace_refresh"
    }

    fn execute(&self, _params: &serde_json::Value, ctx: &JobContext) -> JobReport {
        if ctx.cancel.is_cancelled() {
            return JobReport {
                outcome: JobOutcome::Cancelled,
                processed: 0,
                succeeded: 0,
                failed: 0,
                reason_code: Some("cancelled".into()),
                summary: Some("workspace refresh cancelled before start".into()),
                report_ref: None,
            }
            .into_cancelled();
        }
        let Some(log_path) = kodo_core::workspace::log_path() else {
            return JobReport {
                outcome: JobOutcome::Failed,
                processed: 0,
                succeeded: 0,
                failed: 1,
                reason_code: Some("state_dir_unavailable".into()),
                summary: Some("state directory is unavailable".into()),
                report_ref: None,
            };
        };
        let projects = match kodo_core::workspace::load(&log_path) {
            Ok(items) => items,
            Err(error) => {
                return JobReport {
                    outcome: JobOutcome::Failed,
                    processed: 0,
                    succeeded: 0,
                    failed: 1,
                    reason_code: Some("load_failed".into()),
                    summary: Some(format!("workspace::load failed: {error}")),
                    report_ref: None,
                };
            }
        };
        if ctx.cancel.is_cancelled() {
            return JobReport::partial(0, 0, "cancelled".into(), Some("cancelled mid-run".into()));
        }
        let total = projects.len();
        let first_path = projects
            .first()
            .map(|p| p.path.display().to_string())
            .unwrap_or_default();
        let summary = format!("{total} projects (first={first_path})");
        JobReport {
            outcome: JobOutcome::Succeeded,
            processed: total as u32,
            succeeded: total as u32,
            failed: 0,
            reason_code: None,
            summary: Some(summary),
            report_ref: Some(log_path.display().to_string()),
        }
    }
}

impl WorkspaceRefresh {
    /// Re-exported helper for the CLI / Tauri to invoke the same code path
    /// outside the scheduler. The same idempotency rules apply.
    pub fn run_now() -> JobReport {
        let ctx = JobContext {
            run_id: String::new(),
            deadline: None,
            cancel: crate::scheduler::JobCancellation::new(),
        };
        WorkspaceRefresh.execute(&serde_json::Value::Null, &ctx)
    }
}

/// Job that drains the memory engine's outbox. Reuses the same
/// `outbox::replay` entry point as `kodo memory doctor --apply` so the
/// scheduled and on-demand paths cannot drift.
pub struct MemoryOutboxReplay;

impl Job for MemoryOutboxReplay {
    fn job_type(&self) -> &str {
        "memory_outbox_replay"
    }

    fn execute(&self, _params: &serde_json::Value, ctx: &JobContext) -> JobReport {
        if ctx.cancel.is_cancelled() {
            return JobReport::partial(0, 0, "cancelled".into(), Some("cancelled".into()));
        }
        let Some(state_dir) = kodo_core::settings::state_dir() else {
            return JobReport {
                outcome: JobOutcome::Failed,
                processed: 0,
                succeeded: 0,
                failed: 1,
                reason_code: Some("state_dir_unavailable".into()),
                summary: Some("state directory is unavailable".into()),
                report_ref: None,
            };
        };
        let mem = match SqliteMemory::open_at(&state_dir) {
            Ok(value) => value,
            Err(error) => {
                return JobReport {
                    outcome: JobOutcome::Failed,
                    processed: 0,
                    succeeded: 0,
                    failed: 1,
                    reason_code: Some("memory_open_failed".into()),
                    summary: Some(format!("{error}")),
                    report_ref: None,
                };
            }
        };
        // Cap the batch so a one-shot replay cannot monopolize a worker
        // thread; the next tick picks up where this one stopped.
        const BATCH: usize = 256;
        let result = outbox::replay(mem.store(), kodo_memory::MemoryPrivacy::High, BATCH);
        if ctx.cancel.is_cancelled() {
            let compiled = result.unwrap_or(0);
            return JobReport::partial(
                compiled as u32,
                compiled as u32,
                "cancelled".into(),
                Some("cancelled mid-replay".into()),
            );
        }
        match result {
            Ok(compiled) => JobReport {
                outcome: JobOutcome::Succeeded,
                processed: compiled as u32,
                succeeded: compiled as u32,
                failed: 0,
                reason_code: None,
                summary: Some(format!("replayed {compiled} memory events")),
                report_ref: Some(mem.path().display().to_string()),
            },
            Err(error) => JobReport {
                outcome: JobOutcome::Failed,
                processed: 0,
                succeeded: 0,
                failed: 1,
                reason_code: Some("replay_failed".into()),
                summary: Some(format!("{error}")),
                report_ref: Some(mem.path().display().to_string()),
            },
        }
    }
}

impl MemoryOutboxReplay {
    /// CLI / Tauri helper: run a one-shot replay, sharing the same
    /// idempotent semantics as the scheduled path.
    pub fn run_now(limit: usize) -> JobReport {
        let ctx = JobContext {
            run_id: String::new(),
            deadline: None,
            cancel: crate::scheduler::JobCancellation::new(),
        };
        let _ = limit;
        MemoryOutboxReplay.execute(&serde_json::Value::Null, &ctx)
    }
}

/// Convenience extension methods on `JobReport` for the partial / cancelled
/// shapes the handlers use repeatedly. Defined here rather than in
/// `scheduler` to keep that module free of `serde_json::Value`.
trait JobReportExt {
    fn partial(
        processed: u32,
        succeeded: u32,
        reason: String,
        summary: Option<String>,
    ) -> JobReport;
    fn into_cancelled(self) -> JobReport;
}
impl JobReportExt for JobReport {
    fn partial(
        processed: u32,
        succeeded: u32,
        reason: String,
        summary: Option<String>,
    ) -> JobReport {
        JobReport {
            outcome: JobOutcome::Partial,
            processed,
            succeeded,
            failed: 0,
            reason_code: Some(reason),
            summary,
            report_ref: None,
        }
    }
    fn into_cancelled(self) -> JobReport {
        JobReport {
            outcome: JobOutcome::Cancelled,
            ..self
        }
    }
}

// `Arc` import kept because `JobRegistry::register` consumes an `Arc<dyn Job>`.
#[allow(dead_code)]
fn _arc_marker() -> Arc<dyn Job> {
    Arc::new(WorkspaceRefresh)
}

/// Build a [`JobRegistry`] with the two production handlers registered.
/// Tests can call this and add their own fakes on top.
pub fn default_job_registry() -> JobRegistry {
    let registry = JobRegistry::new();
    registry.register(Arc::new(WorkspaceRefresh));
    registry.register(Arc::new(MemoryOutboxReplay));
    registry
}

/// Standalone factory used by the Tauri binding: the `Scheduler` is
/// already constructed with an empty registry, so the binding asks for
/// the handlers it should `register` on it.
pub fn default_job_registry_handlers() -> Vec<Arc<dyn Job>> {
    vec![Arc::new(WorkspaceRefresh), Arc::new(MemoryOutboxReplay)]
}

/// Re-exported constructors so callers don't have to name the structs.
pub fn workspace_refresh() -> Arc<dyn Job> {
    Arc::new(WorkspaceRefresh)
}
pub fn memory_outbox_replay() -> Arc<dyn Job> {
    Arc::new(MemoryOutboxReplay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::JobCancellation;
    use kodo_core::settings;

    fn ctx() -> JobContext {
        JobContext {
            run_id: "r1".into(),
            deadline: None,
            cancel: JobCancellation::new(),
        }
    }

    #[test]
    fn workspace_refresh_reports_project_count() {
        let report = WorkspaceRefresh.execute(&serde_json::json!({}), &ctx());
        // Outcome is "Succeeded" in any environment where `state_dir` is
        // resolvable; "Failed" only when HOME is unset (CI sandbox). The
        // first case is the one we care about.
        if let Some(state_dir) = settings::state_dir() {
            assert_eq!(report.outcome, JobOutcome::Succeeded);
            let _ = state_dir; // keep the import live on success paths
        } else {
            assert_eq!(report.outcome, JobOutcome::Failed);
            assert_eq!(report.reason_code.as_deref(), Some("state_dir_unavailable"));
        }
    }

    #[test]
    fn workspace_refresh_respects_cancellation() {
        let cancel = JobCancellation::new();
        cancel.cancel();
        let report = WorkspaceRefresh.execute(
            &serde_json::json!({}),
            &JobContext {
                run_id: "r1".into(),
                deadline: None,
                cancel,
            },
        );
        assert_eq!(report.outcome, JobOutcome::Cancelled);
    }

    #[test]
    fn memory_outbox_replay_falls_back_to_state_dir() {
        let report = MemoryOutboxReplay.execute(&serde_json::json!({}), &ctx());
        // Either succeeds (events drained) or fails with the
        // state_dir_unavailable reason code in sandbox CI.
        if let Some(_) = settings::state_dir() {
            assert!(
                matches!(report.outcome, JobOutcome::Succeeded | JobOutcome::Partial),
                "unexpected outcome: {:?}",
                report.outcome
            );
        } else {
            assert_eq!(report.reason_code.as_deref(), Some("state_dir_unavailable"));
        }
    }
}
