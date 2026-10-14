//! Tauri scheduler binding.
//!
//! Owns the persistent [`Scheduler`] (storage + dispatcher + resource
//! guard + job registry), runs a one-second tick thread, and converts each
//! `DispatchedRun` into either:
//!
//! - an **Agent** run — a fresh session per scheduled occurrence, driven
//!   by the existing [`crate::run::start`] so the approval rendezvous, the
//!   session-log writes, and the run-event stream all stay exactly the
//!   same as for a human-initiated message; or
//! - a **Job** run — a thread that calls the registered handler directly
//!   inside the scheduler process. Cancellation is honoured via
//!   [`JobCancellation`]; timeouts via the task's `timeout_seconds`.
//!
//! Scheduled triggers do **not** expand permissions: Agent runs are
//! pinned to `Permission::Ask` (the safe default). Operators who want
//! autonomous runs pre-authorise the per-session allow list in the GUI.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};

use kodo_agent::Permission;
use kodo_core::scheduler::{
    Clock, Outcome, Schedule, ScheduledTask, SystemClock, TaskKind, TaskRun, UnixSeconds,
};
use kodo_shell::{
    recover_inflight, DispatchedRun, Job, JobCancellation, JobContext, JobReport, ResourceHandle,
    Scheduler, SchedulerError,
};

use crate::run::{self, Approvals, Runs, StartArgs};

const TICK_INTERVAL: Duration = Duration::from_secs(1);
const AGENT_RUN_PERMISSION: Permission = Permission::Ask;
const AGENT_RUN_MODE: kodo_agent::AgentMode = kodo_agent::AgentMode::Code;

type JobCancelMap = Arc<Mutex<HashMap<String, JobCancellation>>>;

/// Public surface used by `main.rs` and the Tauri command handlers.
///
/// `Clone` is cheap (the only field is an `Arc`); commands receive
/// `State<SchedulerService>` and call its methods directly.
#[derive(Clone)]
pub struct SchedulerService {
    inner: Arc<SchedulerServiceInner>,
}

struct SchedulerServiceInner {
    /// The Scheduler is not `Clone` (it owns a `Connection`), so it
    /// lives behind an `Arc` and is shared with the tick thread, the
    /// executor threads, and the Tauri command threads.
    scheduler: Arc<Scheduler>,
    state_dir: PathBuf,
    runs: Runs,
    approvals: Approvals,
    /// Cancellation tokens for in-flight Job runs. Agent runs reuse the
    /// existing `Runs.cancel` path, so they do not appear here.
    job_cancels: JobCancelMap,
    /// Cancellation flag flipped by [`Self::shutdown`].
    stop: Arc<AtomicBool>,
    /// Typed `AppHandle` for event emit. Captured at construction time
    /// because the runtime is monomorphic (`tauri::Wry`).
    app: AppHandle<tauri::Wry>,
}

impl SchedulerService {
    /// Boot the service: open the store, recover in-flight runs, register
    /// the default job handlers, and start the tick thread.
    pub fn boot(
        app: AppHandle<tauri::Wry>,
        state_dir: PathBuf,
        runs: Runs,
        approvals: Approvals,
    ) -> Result<Self, String> {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let mut scheduler =
            Scheduler::open(&state_dir, clock).map_err(scheduler_error_to_string)?;
        for handler in kodo_shell::default_job_registry_handlers() {
            scheduler.jobs_mut().register(handler);
        }
        let now = unix_now();
        let recovered =
            recover_inflight(scheduler.store(), now).map_err(scheduler_error_to_string)?;
        if recovered > 0 {
            eprintln!("kodo scheduler: marked {recovered} interrupted runs on boot");
        }
        let service = Self {
            inner: Arc::new(SchedulerServiceInner {
                scheduler: Arc::new(scheduler),
                state_dir,
                runs,
                approvals,
                job_cancels: Arc::new(Mutex::new(HashMap::new())),
                stop: Arc::new(AtomicBool::new(false)),
                app,
            }),
        };
        service.spawn_tick_thread();
        Ok(service)
    }

    /// Spawn the tick thread. Idempotent: a second call is a no-op
    /// because the inner `stop` flag would have been cleared first.
    fn spawn_tick_thread(&self) {
        let inner = self.inner.clone();
        thread::Builder::new()
            .name("kodo-scheduler-tick".into())
            .spawn(move || tick_loop(inner))
            .expect("failed to spawn scheduler tick thread");
    }

    /// Schedule a clean shutdown. The tick thread exits at the next
    /// `TICK_INTERVAL` boundary. The thread is not joined here because
    /// the OS reclaims it on process exit; in-flight runs are left to
    /// `recover_inflight` at the next boot.
    pub fn shutdown(&self) {
        self.inner.stop.store(true, Ordering::SeqCst);
    }

    fn scheduler(&self) -> &Scheduler {
        &self.inner.scheduler
    }

    // -- public API used by the Tauri commands ------------------------------

    pub fn list_tasks(&self, include_archived: bool) -> Result<Vec<ScheduledTask>, String> {
        self.scheduler()
            .store()
            .list_tasks(include_archived)
            .map_err(scheduler_error_to_string)
    }

    pub fn get_task(&self, id: &str) -> Result<Option<ScheduledTask>, String> {
        self.scheduler()
            .store()
            .get_task(id)
            .map_err(scheduler_error_to_string)
    }

    pub fn preview_schedule(
        &self,
        schedule: &Schedule,
        after: UnixSeconds,
    ) -> Result<Vec<UnixSeconds>, String> {
        self.scheduler()
            .preview_schedule(schedule, after)
            .map_err(scheduler_error_to_string)
    }

    pub fn create_task(&self, mut task: ScheduledTask) -> Result<ScheduledTask, String> {
        ensure_valid_task(&task)?;
        let now = unix_now();
        task.created_at = now;
        task.updated_at = now;
        let created = self
            .scheduler()
            .create_task(task)
            .map_err(scheduler_error_to_string)?;
        self.emit_task_changed(&created);
        Ok(created)
    }

    pub fn update_task(
        &self,
        expected_revision: u64,
        mut task: ScheduledTask,
    ) -> Result<ScheduledTask, String> {
        ensure_valid_task(&task)?;
        let now = unix_now();
        task.updated_at = now;
        let updated = self
            .scheduler()
            .update_task(expected_revision, task)
            .map_err(scheduler_error_to_string)?;
        self.emit_task_changed(&updated);
        Ok(updated)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<ScheduledTask, String> {
        let task = self
            .scheduler()
            .set_enabled(id, enabled)
            .map_err(scheduler_error_to_string)?;
        self.emit_task_changed(&task);
        Ok(task)
    }

    pub fn archive_task(&self, id: &str) -> Result<(), String> {
        self.scheduler()
            .archive_task(id)
            .map_err(scheduler_error_to_string)?;
        if let Some(task) = self.scheduler().store().get_task(id).ok().flatten() {
            self.emit_task_changed(&task);
        }
        Ok(())
    }

    pub fn run_now(&self, id: &str) -> Result<TaskRun, String> {
        let request_id = format!("manual-{}-{}", id, unix_now());
        let dispatched = self
            .scheduler()
            .enqueue_manual(id, &request_id)
            .map_err(scheduler_error_to_string)?;
        let run_id = dispatched.run.id.clone();
        self.dispatch_run(dispatched);
        self.scheduler()
            .store()
            .get_run(&run_id)
            .map_err(scheduler_error_to_string)?
            .ok_or_else(|| "run vanished immediately after enqueue".to_string())
    }

    pub fn list_runs(&self, task_id: &str, limit: usize) -> Result<Vec<TaskRun>, String> {
        self.scheduler()
            .store()
            .list_runs(task_id, limit)
            .map_err(scheduler_error_to_string)
    }

    pub fn cancel_run(&self, run_id: &str) -> Result<bool, String> {
        let accepted = self
            .scheduler()
            .request_cancel(run_id)
            .map_err(scheduler_error_to_string)?;
        // The session id equals the run id for Agent runs (see
        // `dispatch_run`); flip the live flag in case it is still held.
        self.inner.runs.cancel(run_id);
        if let Some(token) = self.inner.job_cancels.lock().remove(run_id) {
            token.cancel();
        }
        if let Some(run) = self
            .scheduler()
            .store()
            .get_run(run_id)
            .map_err(scheduler_error_to_string)?
        {
            self.emit_run_changed(&run);
        }
        Ok(accepted)
    }

    pub fn list_job_types(&self) -> Vec<String> {
        self.scheduler().jobs().types()
    }

    // -- dispatch ----------------------------------------------------------

    /// Hand a freshly-queued `DispatchedRun` to the right executor. This
    /// is the seam where the Tauri-specific machinery (sessions, Runs,
    /// Approvals, event emit) meets the storage-agnostic dispatcher.
    fn dispatch_run(&self, dispatched: DispatchedRun) {
        let resource_keys = dispatched.task.task.resource_keys();
        let handle = match self
            .inner
            .scheduler
            .try_admit(&dispatched.run.id, &resource_keys)
        {
            Ok(handle) => handle,
            Err(error) => {
                eprintln!(
                    "kodo scheduler: refused to dispatch run {}: {error}",
                    dispatched.run.id
                );
                return;
            }
        };
        match &dispatched.task.task {
            TaskKind::Agent { .. } => {
                spawn_agent_run(self.inner.clone(), dispatched, handle);
            }
            TaskKind::Job { .. } => {
                spawn_job_run(self.inner.clone(), dispatched, handle);
            }
        }
    }

    fn emit_task_changed(&self, task: &ScheduledTask) {
        let _ = self.inner.app.emit("scheduler:task_changed", task);
    }

    fn emit_run_changed(&self, run: &TaskRun) {
        let _ = self.inner.app.emit("scheduler:run_changed", run);
    }
}

// ---------------------------------------------------------------------------
// Tick loop
// ---------------------------------------------------------------------------

fn tick_loop(inner: Arc<SchedulerServiceInner>) {
    while !inner.stop.load(Ordering::SeqCst) {
        match inner.scheduler.tick() {
            Ok(plan) => {
                for dispatched in plan {
                    dispatch_via_inner(&inner, dispatched);
                }
            }
            Err(error) => eprintln!("kodo scheduler: tick failed: {error}"),
        }
        thread::sleep(TICK_INTERVAL);
    }
}

fn dispatch_via_inner(inner: &Arc<SchedulerServiceInner>, dispatched: DispatchedRun) {
    let resource_keys = dispatched.task.task.resource_keys();
    let handle = match inner
        .scheduler
        .try_admit(&dispatched.run.id, &resource_keys)
    {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!(
                "kodo scheduler: refused to dispatch run {}: {error}",
                dispatched.run.id
            );
            return;
        }
    };
    match &dispatched.task.task {
        TaskKind::Agent { .. } => spawn_agent_run(inner.clone(), dispatched, handle),
        TaskKind::Job { .. } => spawn_job_run(inner.clone(), dispatched, handle),
    }
}

// ---------------------------------------------------------------------------
// Executors
// ---------------------------------------------------------------------------

fn spawn_agent_run(
    inner: Arc<SchedulerServiceInner>,
    dispatched: DispatchedRun,
    handle: ResourceHandle,
) {
    let (project, message, budget_family) = match dispatched.task.task {
        TaskKind::Agent {
            project,
            message,
            budget_family,
            ..
        } => (project, message, budget_family),
        _ => unreachable!("spawn_agent_run called with a Job task"),
    };
    let run_id = dispatched.run.id.clone();
    let task_id = dispatched.task.id.clone();
    let scheduled_for = dispatched.run.scheduled_for;
    let session_id = format!("sched-{task_id}-{scheduled_for}");
    thread::Builder::new()
        .name(format!("kodo-sched-agent-{}", run_id))
        .spawn(move || {
            execute_agent_run(
                inner,
                run_id,
                task_id,
                session_id,
                project,
                message,
                budget_family,
                handle,
            );
        })
        .expect("failed to spawn agent run thread");
}

fn execute_agent_run(
    inner: Arc<SchedulerServiceInner>,
    run_id: String,
    _task_id: String,
    session_id: String,
    project: String,
    message: String,
    budget_family: Option<String>,
    handle: ResourceHandle,
) {
    let scheduler = inner.scheduler.clone();
    let runs = inner.runs.clone();
    let approvals = inner.approvals.clone();
    let app = inner.app.clone();
    let project_path = PathBuf::from(&project);

    if let Err(error) = ensure_session(&session_id, &project_path, &message) {
        let _ = scheduler.store().mark_finished(
            &run_id,
            unix_now(),
            Outcome::Failed,
            None,
            None,
            None,
            None,
            Some("session_open_failed"),
            Some(&error),
        );
        emit_run_changed(&app, &scheduler, &run_id);
        return;
    }
    let token = match runs.begin(&session_id) {
        Some(token) => token,
        None => {
            let _ = scheduler.store().mark_finished(
                &run_id,
                unix_now(),
                Outcome::Skipped,
                None,
                None,
                None,
                None,
                Some("run_already_active"),
                Some("session already has a run in flight"),
            );
            return;
        }
    };
    if let Err(error) = scheduler.mark_started(&run_id) {
        runs.release(&session_id, &token);
        let _ = scheduler.store().mark_finished(
            &run_id,
            unix_now(),
            Outcome::Failed,
            None,
            None,
            None,
            None,
            Some("mark_started_failed"),
            Some(&format!("{error}")),
        );
        return;
    }
    emit_run_changed(&app, &scheduler, &run_id);

    let config = kodo_shell::ShellConfig::load(kodo_shell::Overrides::default());
    let start_args = StartArgs {
        dir: PathBuf::new(),
        id: session_id.clone(),
        project: Some(project_path.clone()),
        message,
        conversation_history: Vec::new(),
        knowledge_context: None,
        context: Vec::new(),
        provider: config.provider,
        permission: AGENT_RUN_PERMISSION,
        mode: AGENT_RUN_MODE,
        fallback_to_local: config.fallback_to_local,
        max_output_tokens: config.max_output_tokens,
        extended_thinking: config.extended_thinking,
        local_trace: config.local_trace,
        mcp_servers: config.mcp_servers,
        agent_instructions: None,
        // Scheduled runs use the persona names declared on the task; the
        // agent driver resolves them through the same registry the
        // session "plugins" UI uses. Empty list = no persona for this
        // run.
        personas: Vec::new(),
        budget_family,
    };
    let result = run::start(&app, &runs, &approvals, start_args, token.clone());
    let outcome = match &result {
        Ok(()) => Outcome::Succeeded,
        Err(_) => Outcome::Failed,
    };
    let reason_code = result
        .as_ref()
        .err()
        .map(|e| format!("agent_run_error: {e}"));
    let error_summary = result.as_ref().err().map(|e| truncate(e, 240));
    let _ = scheduler.mark_finished(
        &run_id,
        outcome,
        Some(if result.is_ok() { "delivered" } else { "none" }),
        Some("approved_run"),
        Some(&session_id),
        Some(&session_id),
        reason_code.as_deref(),
        error_summary.as_deref(),
        handle,
    );
    runs.release(&session_id, &token);
    emit_run_changed(&app, &scheduler, &run_id);
}

fn spawn_job_run(
    inner: Arc<SchedulerServiceInner>,
    dispatched: DispatchedRun,
    handle: ResourceHandle,
) {
    let (job_type, params) = match &dispatched.task.task {
        TaskKind::Job {
            job_type, params, ..
        } => (job_type.clone(), params.clone()),
        _ => unreachable!("spawn_job_run called with an Agent task"),
    };
    let job = match inner.scheduler.jobs().get(&job_type) {
        Some(job) => job,
        None => {
            let _ = inner.scheduler.store().mark_finished(
                &dispatched.run.id,
                unix_now(),
                Outcome::Failed,
                None,
                None,
                None,
                None,
                Some("unknown_job_type"),
                Some(&format!("job_type `{job_type}` is not registered")),
            );
            emit_run_changed(&inner.app, &inner.scheduler, &dispatched.run.id);
            return;
        }
    };
    let run_id = dispatched.run.id.clone();
    let scheduler = inner.scheduler.clone();
    let cancel = JobCancellation::new();
    inner
        .job_cancels
        .lock()
        .insert(run_id.clone(), cancel.clone());
    let run_id_for_thread = run_id.clone();
    let job_cancels = inner.job_cancels.clone();
    let app = inner.app.clone();
    let timeout = dispatched.task.timeout_seconds;
    thread::Builder::new()
        .name(format!("kodo-sched-job-{run_id}"))
        .spawn(move || {
            if let Err(error) = scheduler.mark_started(&run_id_for_thread) {
                let _ = scheduler.store().mark_finished(
                    &run_id_for_thread,
                    unix_now(),
                    Outcome::Failed,
                    None,
                    None,
                    None,
                    None,
                    Some("mark_started_failed"),
                    Some(&format!("{error}")),
                );
                emit_run_changed(&app, &scheduler, &run_id_for_thread);
                return;
            }
            emit_run_changed(&app, &scheduler, &run_id_for_thread);
            let deadline = timeout.map(|s| unix_now() + s as i64);
            let ctx = JobContext {
                run_id: run_id_for_thread.clone(),
                deadline,
                cancel: cancel.clone(),
            };
            let report: JobReport = if let Some(secs) = timeout {
                run_with_timeout(job.as_ref(), &params, &ctx, Duration::from_secs(secs))
            } else {
                job.execute(&params, &ctx)
            };
            let outcome: Outcome = report.outcome.into();
            let _ = scheduler.mark_finished(
                &run_id_for_thread,
                outcome,
                None,
                None,
                None,
                report.report_ref.as_deref(),
                report.reason_code.as_deref(),
                report.summary.as_deref(),
                handle,
            );
            job_cancels.lock().remove(&run_id_for_thread);
            emit_run_changed(&app, &scheduler, &run_id_for_thread);
        })
        .expect("failed to spawn job run thread");
}

fn run_with_timeout(
    job: &dyn Job,
    params: &serde_json::Value,
    ctx: &JobContext,
    timeout: Duration,
) -> JobReport {
    // Cooperative timeout: the job checks `cancel` itself. We arm the
    // cancel flag once the deadline elapses; the job's next check turns
    // the report into a partial / cancelled one.
    let cancel = ctx.cancel.clone();
    let armed = Arc::new(AtomicBool::new(false));
    let armed_for_thread = armed.clone();
    let cancel_for_thread = cancel.clone();
    let timer = thread::spawn(move || {
        thread::sleep(timeout);
        armed_for_thread.store(true, Ordering::SeqCst);
        cancel_for_thread.cancel();
    });
    let report = job.execute(params, ctx);
    let _ = timer.join();
    if armed.load(Ordering::SeqCst) && matches!(report.outcome, kodo_shell::JobOutcome::Succeeded) {
        JobReport {
            outcome: kodo_shell::JobOutcome::Partial,
            reason_code: Some("timed_out".into()),
            summary: Some("job did not finish before its deadline".into()),
            ..report
        }
    } else {
        report
    }
}

fn ensure_session(
    session_id: &str,
    project: &std::path::Path,
    message: &str,
) -> Result<(), String> {
    use kodo_core::session::{self, Mode};
    let Some(dir) = session::dir() else {
        return Err("state directory unavailable".into());
    };
    let summary = truncate(message, 80);
    if let Err(error) = session::open(&dir, project, &summary, session::now(), Mode::Code) {
        let text = format!("{error}");
        if !text.contains("exists") && !text.contains("already") {
            return Err(format!("session::open failed: {text}"));
        }
    }
    session::record_ask(&dir, session_id, session::now(), message)
        .map_err(|e| format!("record_ask failed: {e}"))?;
    Ok(())
}

fn emit_run_changed(app: &AppHandle<tauri::Wry>, scheduler: &Scheduler, run_id: &str) {
    if let Ok(Some(run)) = scheduler.store().get_run(run_id) {
        let _ = app.emit("scheduler:run_changed", &run);
    }
}

fn unix_now() -> UnixSeconds {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn scheduler_error_to_string(error: SchedulerError) -> String {
    format!("{error}")
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let end = text.char_indices().nth(max).map(|(i, _)| i).unwrap_or(max);
    format!("{}…", &text[..end])
}

fn ensure_valid_task(task: &ScheduledTask) -> Result<(), String> {
    if task.id.is_empty() {
        return Err("task id is required".into());
    }
    if task.title.trim().is_empty() {
        return Err("task title is required".into());
    }
    Ok(())
}

trait TaskKindExt {
    fn resource_keys(&self) -> Vec<String>;
}
impl TaskKindExt for TaskKind {
    fn resource_keys(&self) -> Vec<String> {
        TaskKind::resource_keys(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kodo_core::scheduler::{MisfirePolicy, OverlapPolicy};

    #[test]
    fn resource_keys_for_agent_use_the_declared_key() {
        let kind = TaskKind::Agent {
            project: "/p".into(),
            message: "hi".into(),
            personas: Vec::new(),
            budget_family: None,
            resource_key: Some("agent:/p".into()),
        };
        assert_eq!(kind.resource_keys(), vec!["agent:agent:/p".to_string()]);
    }

    #[test]
    fn resource_keys_for_job_use_the_prefixed_keys() {
        let kind = TaskKind::Job {
            job_type: "demo".into(),
            params: serde_json::json!({}),
            resources: vec!["workspace:/p".into()],
        };
        assert_eq!(kind.resource_keys(), vec!["job:workspace:/p".to_string()]);
    }

    #[test]
    fn truncate_respects_byte_length() {
        assert_eq!(truncate("hello", 10), "hello".to_string());
        assert_eq!(truncate("hello world", 5), "hello…".to_string());
    }

    #[test]
    fn scheduler_error_renders_a_stable_code() {
        let error = SchedulerError::RevisionConflict;
        assert_eq!(scheduler_error_to_string(error), "revision_conflict");
    }

    #[test]
    fn ensure_valid_task_rejects_blank_inputs() {
        let mut task = make_valid_task();
        task.id = String::new();
        assert!(ensure_valid_task(&task).is_err());
        let mut task = make_valid_task();
        task.title = "   ".into();
        assert!(ensure_valid_task(&task).is_err());
        let task = make_valid_task();
        assert!(ensure_valid_task(&task).is_ok());
    }

    fn make_valid_task() -> ScheduledTask {
        ScheduledTask {
            id: "t1".into(),
            title: "demo".into(),
            state: kodo_core::scheduler::TaskState::Enabled,
            revision: 1,
            schedule_generation: 1,
            schedule: Schedule::Once {
                fire_at: 1_700_000_000,
            },
            task: TaskKind::Job {
                job_type: "demo".into(),
                params: serde_json::json!({}),
                resources: Vec::new(),
            },
            timezone: "UTC".into(),
            misfire_policy: MisfirePolicy::LatestOnce,
            grace_seconds: 3600,
            overlap_policy: OverlapPolicy::Skip,
            timeout_seconds: None,
            created_at: 0,
            updated_at: 0,
            next_fire_at: None,
            archived: false,
        }
    }
}
