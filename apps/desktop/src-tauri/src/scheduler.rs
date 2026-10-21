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
/// Scheduled runs have no UI to answer an approval card; they still need a
/// bound so a background job cannot wait forever on an unanswerable prompt.
const SCHEDULED_APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

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
    runs: Runs,
    approvals: Approvals,
    /// Cancellation tokens for in-flight Job runs.
    job_cancels: JobCancelMap,
    /// scheduler run id → the real session id that executes it. Cancel and
    /// the result link both resolve through this map instead of assuming the
    /// two ids are the same string.
    run_sessions: Arc<Mutex<HashMap<String, String>>>,
    /// Cancellation flag flipped by [`SchedulerService::shutdown`].
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
                runs,
                approvals,
                job_cancels: Arc::new(Mutex::new(HashMap::new())),
                run_sessions: Arc::new(Mutex::new(HashMap::new())),
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
        if let TaskKind::Job { job_type, .. } = &task.task {
            if self.scheduler().jobs().get(job_type).is_none() {
                return Err("unknown_job".into());
            }
        }
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
        if let TaskKind::Job { job_type, .. } = &task.task {
            if self.scheduler().jobs().get(job_type).is_none() {
                return Err("unknown_job".into());
            }
        }
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
        // Cancel the *execution* behind this scheduler run: the live run is
        // registered under its session id, which the executor published in
        // `run_sessions` before starting. Cancelling the run id itself only
        // ever worked by accident (I-04/I-05).
        let session_id = self.inner.run_sessions.lock().get(run_id).cloned();
        if let Some(session_id) = session_id {
            self.inner.runs.cancel(&session_id);
            self.inner.approvals.clear_session(&session_id);
        }
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
        dispatch_via_inner(&self.inner, dispatched);
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
            // A refused execution has no live executor. Persist a terminal
            // result so the history never leaves it looking queued forever.
            let reason = error.to_string();
            let _ = inner.scheduler.store().mark_finished(
                &dispatched.run.id,
                unix_now(),
                Outcome::Skipped,
                None,
                None,
                None,
                None,
                Some(&reason),
                None,
            );
            emit_run_changed(&inner.app, &inner.scheduler, &dispatched.run.id);
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
    let (project, message, budget_family, mode_label, persona_names) = match &dispatched.task.task {
        TaskKind::Agent {
            project,
            message,
            budget_family,
            personas,
            mode,
            ..
        } => (
            project.clone(),
            message.clone(),
            budget_family.clone(),
            mode.clone(),
            personas.clone(),
        ),
        _ => unreachable!("spawn_agent_run called with a Job task"),
    };
    let run_id = dispatched.run.id.clone();
    let timeout_seconds = dispatched.task.timeout_seconds;
    thread::Builder::new()
        .name(format!("kodo-sched-agent-{run_id}"))
        .spawn(move || {
            execute_agent_run(
                inner,
                run_id,
                project,
                message,
                budget_family,
                mode_label,
                persona_names,
                timeout_seconds,
                handle,
            );
        })
        .expect("failed to spawn agent run thread");
}

/// A session created for one scheduled occurrence.
///
/// `session::open` mints the id; the host must use that id everywhere
/// (log dir, run record, approval, cancel) instead of inventing a parallel
/// one — two ids for one execution was the whole of I-05.
struct PreparedSession {
    id: String,
    dir: PathBuf,
}

fn prepare_session(
    project: &std::path::Path,
    message: &str,
    mode: kodo_core::session::Mode,
) -> Result<PreparedSession, String> {
    let dir = kodo_core::session::dir().ok_or_else(|| "state directory unavailable".to_string())?;
    prepare_session_in(&dir, project, message, mode)
}

/// The real work of `prepare_session`, with the state directory injected so
/// the same code path can be driven from a temporary directory in tests.
fn prepare_session_in(
    dir: &std::path::Path,
    project: &std::path::Path,
    message: &str,
    mode: kodo_core::session::Mode,
) -> Result<PreparedSession, String> {
    use kodo_core::session;
    let title = truncate(message, 80);
    let id = session::open(dir, project, &title, session::now(), mode)
        .map_err(|error| format!("session::open failed: {error}"))?;
    session::record_ask(dir, &id, session::now(), message)
        .map_err(|error| format!("record_ask failed: {error}"))?;
    Ok(PreparedSession {
        id,
        dir: dir.to_path_buf(),
    })
}

/// Resolve the persona names stored on the task through the same registry the
/// interactive composer uses. A missing name is a specific error, never a
/// silent fallback to another role.
fn resolve_personas(
    names: &[String],
    project: &std::path::Path,
) -> Result<Vec<kodo_agent::PersonaBlock>, String> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let user_dir = crate::agents_dir()?;
    let project_dir = crate::project_agents_dir(Some(&project.to_string_lossy()));
    resolve_personas_in(&user_dir, project_dir.as_deref(), names)
}

/// `resolve_personas` with both registry roots injected.
fn resolve_personas_in(
    user_dir: &std::path::Path,
    project_dir: Option<&std::path::Path>,
    names: &[String],
) -> Result<Vec<kodo_agent::PersonaBlock>, String> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    kodo_agent::agents::resolve_chain(user_dir, project_dir, names)
        .map(|chain| {
            chain
                .into_iter()
                .map(kodo_agent::PersonaBlock::from)
                .collect()
        })
        .map_err(|error| format!("persona_unresolved: {error}"))
}

/// Mode labels come from the persisted task. `work` is the only label that
/// switches modes; everything else (including tasks written before the field
/// existed) stays on the legacy Code path.
fn map_mode(label: Option<&str>) -> (kodo_agent::AgentMode, kodo_core::session::Mode) {
    match label {
        Some("work") => (kodo_agent::AgentMode::Work, kodo_core::session::Mode::Work),
        _ => (kodo_agent::AgentMode::Code, kodo_core::session::Mode::Code),
    }
}

/// How long a cancelled run gets to confirm it stopped before the scheduler
/// records `cancelled` anyway (the run handle is then reported as unconfirmed).
const CANCEL_CONFIRM_TIMEOUT: Duration = Duration::from_secs(30);

/// Wait for the executor's trusted terminal summary.
///
/// Returns `(terminal, timed_out)`. Without a deadline this is a plain
/// blocking wait — the executor always sends one summary (or drops the
/// sender, which disconnects the channel). With a deadline the timer lives in
/// the *wait*, not in a sleeping thread that must be joined, so a fast run
/// returns immediately (I-06).
fn wait_for_terminal(
    rx: &std::sync::mpsc::Receiver<run::TurnTerminal>,
    deadline: Option<std::time::Instant>,
    grace: Duration,
    cancel: impl FnOnce(),
) -> (Option<run::TurnTerminal>, bool) {
    use std::sync::mpsc::RecvTimeoutError;
    let Some(deadline) = deadline else {
        return (rx.recv().ok(), false);
    };
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    match rx.recv_timeout(remaining) {
        Ok(terminal) => (Some(terminal), false),
        Err(RecvTimeoutError::Disconnected) => (None, false),
        Err(RecvTimeoutError::Timeout) => {
            cancel();
            match rx.recv_timeout(grace) {
                Ok(terminal) => (Some(terminal), true),
                // The executor did not confirm within the grace window. Record
                // the deadline honestly; the handle is released so the next
                // occurrence is not blocked forever.
                Err(_) => (None, true),
            }
        }
    }
}

/// Map a real terminal summary to the scheduler's outcome vocabulary.
///
/// `Ok` alone is not success: only `delivery == "ready"` counts as delivered.
fn terminal_outcome(
    timed_out: bool,
    terminal: Option<&run::TurnTerminal>,
) -> (Outcome, Option<&'static str>, Option<&'static str>) {
    if timed_out {
        return (Outcome::TimedOut, None, Some("timed_out"));
    }
    let Some(terminal) = terminal else {
        return (
            Outcome::Interrupted,
            None,
            Some("executor_exited_without_terminal"),
        );
    };
    match terminal.kind {
        run::TurnTerminalKind::Failed => (Outcome::Failed, Some("none"), Some("agent_run_error")),
        run::TurnTerminalKind::Stopped => (Outcome::Cancelled, Some("partial"), Some("cancelled")),
        run::TurnTerminalKind::Completed => match terminal.delivery.as_str() {
            "ready" => (Outcome::Succeeded, Some("delivered"), None),
            "partial" => (Outcome::Partial, Some("partial"), Some("partial_delivery")),
            "blocked" => (Outcome::Blocked, Some("blocked"), Some("awaiting_user")),
            "failed" | "" => (Outcome::Failed, Some("none"), Some("no_delivery")),
            _ => (Outcome::Partial, Some("partial"), Some("unknown_delivery")),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_agent_run(
    inner: Arc<SchedulerServiceInner>,
    run_id: String,
    project: String,
    message: String,
    budget_family: Option<String>,
    mode_label: Option<String>,
    persona_names: Vec<String>,
    timeout_seconds: Option<u64>,
    handle: ResourceHandle,
) {
    let scheduler = inner.scheduler.clone();
    let runs = inner.runs.clone();
    let approvals = inner.approvals.clone();
    let app = inner.app.clone();
    let project_path = PathBuf::from(&project);
    // Scheduled work keeps the mode the user chose. A task written before the
    // field existed stays on the legacy Code path.
    let (agent_mode, session_mode) = map_mode(mode_label.as_deref());

    let prepared = match prepare_session(&project_path, &message, session_mode) {
        Ok(prepared) => prepared,
        Err(error) => {
            let _ = scheduler.store().mark_finished(
                &run_id,
                unix_now(),
                Outcome::Failed,
                None,
                None,
                None,
                None,
                Some("session_open_failed"),
                Some(&truncate(&error, 240)),
            );
            emit_run_changed(&app, &scheduler, &run_id);
            return;
        }
    };
    let session_id = prepared.id.clone();
    // run → session mapping: cancel and the result link both go through it.
    inner
        .run_sessions
        .lock()
        .insert(run_id.clone(), session_id.clone());

    let personas = match resolve_personas(&persona_names, &project_path) {
        Ok(personas) => personas,
        Err(error) => {
            let _ = scheduler.store().mark_finished(
                &run_id,
                unix_now(),
                Outcome::Failed,
                None,
                None,
                Some(&session_id),
                None,
                Some("persona_unresolved"),
                Some(&truncate(&error, 240)),
            );
            inner.run_sessions.lock().remove(&run_id);
            emit_run_changed(&app, &scheduler, &run_id);
            return;
        }
    };

    let token = match runs.begin(&session_id) {
        Some(token) => token,
        None => {
            let _ = scheduler.store().mark_finished(
                &run_id,
                unix_now(),
                Outcome::Skipped,
                None,
                None,
                Some(&session_id),
                None,
                Some("run_already_active"),
                Some("session already has a run in flight"),
            );
            inner.run_sessions.lock().remove(&run_id);
            emit_run_changed(&app, &scheduler, &run_id);
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
            Some(&session_id),
            None,
            Some("mark_started_failed"),
            Some(&format!("{error}")),
        );
        inner.run_sessions.lock().remove(&run_id);
        return;
    }
    emit_run_changed(&app, &scheduler, &run_id);

    let config = kodo_shell::ShellConfig::load(kodo_shell::Overrides::default());
    let (terminal_tx, terminal_rx) = std::sync::mpsc::channel();
    let start_args = StartArgs {
        dir: prepared.dir,
        id: session_id.clone(),
        project: Some(project_path.clone()),
        message,
        conversation_history: Vec::new(),
        knowledge_context: None,
        context: Vec::new(),
        force_inline: Vec::new(),
        provider: config.provider,
        // Scheduled triggers never expand permissions: Ask stays the default
        // regardless of mode, persona or trigger source.
        permission: AGENT_RUN_PERMISSION,
        approval_timeout: Some(SCHEDULED_APPROVAL_TIMEOUT),
        mode: agent_mode,
        fallback_to_local: config.fallback_to_local,
        max_output_tokens: config.max_output_tokens,
        extended_thinking: config.extended_thinking,
        local_trace: config.local_trace,
        mcp_servers: config.mcp_servers,
        agent_instructions: None,
        personas,
        budget_family,
        // Background runs do not consult the decision service.
        decision: None,
        terminal: Some(terminal_tx),
    };
    // A scheduled run never gets a decision handle: it cannot ask a human
    // for permission, and it must not inherit a desktop approval.
    let started = run::start(&app, &runs, &approvals, start_args, token.clone());
    let (terminal, timed_out) = match started {
        Ok(()) => {
            let deadline =
                timeout_seconds.map(|secs| std::time::Instant::now() + Duration::from_secs(secs));
            wait_for_terminal(&terminal_rx, deadline, CANCEL_CONFIRM_TIMEOUT, || {
                // Same execution the run registered: cancel by session id and
                // wake an approval wait (the loop polls `is_live`).
                runs.cancel(&session_id);
                approvals.clear_session(&session_id);
            })
        }
        Err(error) => {
            let _ = scheduler.store().mark_finished(
                &run_id,
                unix_now(),
                Outcome::Failed,
                None,
                None,
                Some(&session_id),
                None,
                Some("start_failed"),
                Some(&truncate(&error, 240)),
            );
            runs.release(&session_id, &token);
            inner.run_sessions.lock().remove(&run_id);
            emit_run_changed(&app, &scheduler, &run_id);
            return;
        }
    };

    let (outcome, delivery, reason) = terminal_outcome(timed_out, terminal.as_ref());
    let error_summary = terminal
        .as_ref()
        .map(|t| t.failure_reason.clone())
        .filter(|reason| !reason.is_empty())
        .or_else(|| reason.map(str::to_owned));
    if let Err(error) = scheduler.mark_finished(
        &run_id,
        outcome,
        delivery,
        terminal.as_ref().map(|t| t.verification.as_str()),
        Some(&session_id),
        Some(&session_id),
        reason,
        error_summary.as_deref(),
        handle,
    ) {
        // Terminal write failed: keep the run mapping and do not pretend the
        // run succeeded. The resource handle was consumed by the failed call,
        // so the next occurrence stays blocked until an operator intervenes.
        eprintln!("kodo scheduler: failed to persist terminal state for {run_id}: {error}");
    }
    runs.release(&session_id, &token);
    inner.run_sessions.lock().remove(&run_id);
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
                run_with_timeout(
                    job.clone(),
                    params.clone(),
                    ctx.clone(),
                    Duration::from_secs(secs),
                    JOB_CANCEL_GRACE,
                )
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

/// How long a cooperative job gets to return after the deadline fired.
const JOB_CANCEL_GRACE: Duration = Duration::from_secs(5);

/// Run one job with a wall-clock deadline.
///
/// The job runs on a worker thread and the *wait* is bounded — there is no
/// sleeping timer that has to be joined afterwards. A job that finishes in
/// 200 ms returns in 200 ms even with a 60 s deadline (I-06: the old shape
/// always waited the full timeout and then relabelled the success as
/// `timed_out`).
///
/// Timeout is cooperative: at the deadline we flip the job's cancel flag and
/// wait a short grace period for the job to observe it. A job that ignores it
/// is reported as `timed_out` with `confirmed=false` in the summary rather
/// than pretending it was stopped.
fn run_with_timeout(
    job: Arc<dyn Job>,
    params: serde_json::Value,
    ctx: JobContext,
    timeout: Duration,
    grace: Duration,
) -> JobReport {
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = {
        let ctx = ctx.clone();
        thread::Builder::new()
            .name(format!("kodo-job-{}", ctx.run_id))
            .spawn(move || {
                let report = job.execute(&params, &ctx);
                let _ = tx.send(report);
            })
    };
    if let Err(error) = worker {
        return JobReport {
            outcome: kodo_shell::JobOutcome::Failed,
            processed: 0,
            succeeded: 0,
            failed: 0,
            reason_code: Some("job_spawn_failed".into()),
            summary: Some(format!("could not start the job: {error}")),
            report_ref: None,
        };
    }
    match rx.recv_timeout(timeout) {
        Ok(report) => report,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            // Single cancel signal at the deadline.
            ctx.cancel.cancel();
            match rx.recv_timeout(grace) {
                Ok(report) => JobReport {
                    outcome: kodo_shell::JobOutcome::Partial,
                    reason_code: Some("timed_out".into()),
                    summary: Some(format!(
                        "job stopped after its deadline{}",
                        report
                            .summary
                            .as_deref()
                            .map(|s| format!(": {s}"))
                            .unwrap_or_default()
                    )),
                    ..report
                },
                Err(_) => JobReport {
                    outcome: kodo_shell::JobOutcome::Partial,
                    processed: 0,
                    succeeded: 0,
                    failed: 0,
                    reason_code: Some("timed_out".into()),
                    summary: Some(
                        "deadline passed and the job did not confirm it stopped (uncooperative job)"
                            .into(),
                    ),
                    report_ref: None,
                },
            }
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => JobReport {
            outcome: kodo_shell::JobOutcome::Failed,
            processed: 0,
            succeeded: 0,
            failed: 0,
            reason_code: Some("job_panicked".into()),
            summary: Some("job thread exited without a report".into()),
            report_ref: None,
        },
    }
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
    if task.id.trim().is_empty() || task.title.trim().is_empty() || task.timeout_seconds == Some(0)
    {
        return Err("invalid_input".into());
    }
    match &task.task {
        TaskKind::Agent {
            project, message, ..
        } if project.trim().is_empty() || message.trim().is_empty() => {
            return Err("invalid_input".into());
        }
        TaskKind::Job { job_type, .. } if job_type.trim().is_empty() => {
            return Err("invalid_input".into());
        }
        _ => {}
    }
    Ok(())
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
            mode: None,
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

    // -----------------------------------------------------------------------
    // M2 — host lifecycle seams (T009–T016)
    //
    // These drive the real production functions (`prepare_session_in`,
    // `resolve_personas_in`, `wait_for_terminal`, `terminal_outcome`,
    // `run_with_timeout`, `Scheduler::mark_finished`) rather than a mock of
    // the IPC layer. Only the Tauri `AppHandle` — which needs a webview —
    // stays out of the picture; everything that decides an outcome is here.
    // -----------------------------------------------------------------------

    use std::sync::atomic::AtomicU64;

    /// Fresh temp dir per test, name-scoped so parallel tests never collide.
    fn scratch(tag: &str) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kodo-sched-{tag}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn terminal(kind: run::TurnTerminalKind, delivery: &str) -> run::TurnTerminal {
        run::TurnTerminal {
            kind,
            delivery: delivery.into(),
            verification: String::new(),
            failure_reason: String::new(),
        }
    }

    fn scheduler_in(dir: &std::path::Path, unix: UnixSeconds) -> Scheduler {
        let clock: Arc<dyn Clock> = Arc::new(kodo_core::scheduler::FixedClock { unix, mono: 0 });
        Scheduler::open(dir, clock).expect("scheduler")
    }

    /// A job task the store will accept: `Once` must be in the future, so the
    /// fire instant is a full hour past the fixed clock.
    fn job_task(id: &str) -> ScheduledTask {
        let mut task = make_valid_task();
        task.id = id.into();
        task.schedule = Schedule::Once {
            fire_at: 1_700_003_600,
        };
        task.task = TaskKind::Job {
            job_type: "demo".into(),
            params: serde_json::json!({}),
            resources: vec!["workspace:/p".into()],
        };
        task
    }

    /// T012: `run::start` returning `Ok` is not success. Only the terminal
    /// summary's `ready` delivery is.
    #[test]
    fn a_ready_delivery_is_the_only_success() {
        use run::TurnTerminalKind::{Completed, Failed, Stopped};
        let cases = [
            (Completed, "ready", Outcome::Succeeded),
            (Completed, "partial", Outcome::Partial),
            (Completed, "blocked", Outcome::Blocked),
            (Completed, "failed", Outcome::Failed),
            (Completed, "", Outcome::Failed),
            (Completed, "something-new", Outcome::Partial),
            (Stopped, "partial", Outcome::Cancelled),
            (Stopped, "ready", Outcome::Cancelled),
            (Failed, "ready", Outcome::Failed),
        ];
        for (kind, delivery, expected) in cases {
            let summary = terminal(kind, delivery);
            let (outcome, _, _) = terminal_outcome(false, Some(&summary));
            assert_eq!(outcome, expected, "{kind:?}/{delivery}");
        }
        let summary = terminal(Completed, "ready");
        let (_, delivery_axis, reason) = terminal_outcome(false, Some(&summary));
        assert_eq!(delivery_axis, Some("delivered"));
        assert_eq!(reason, None);
    }

    /// T013: a deadline outranks a late answer — the run did not finish in
    /// time, so it is `timed_out` even though the executor eventually said
    /// `ready`.
    #[test]
    fn a_deadline_outranks_a_late_ready_answer() {
        let summary = terminal(run::TurnTerminalKind::Completed, "ready");
        let (outcome, delivery, reason) = terminal_outcome(true, Some(&summary));
        assert_eq!(outcome, Outcome::TimedOut);
        assert_eq!(delivery, None);
        assert_eq!(reason, Some("timed_out"));
    }

    /// T016: an executor that exited without a terminal summary must never be
    /// recorded as success.
    #[test]
    fn a_missing_terminal_is_interrupted_never_success() {
        let (outcome, delivery, reason) = terminal_outcome(false, None);
        assert_eq!(outcome, Outcome::Interrupted);
        assert_eq!(delivery, None);
        assert_eq!(reason, Some("executor_exited_without_terminal"));
    }

    /// I-06 / T013: a run that answers immediately returns immediately, even
    /// with a 30 s deadline, and is not cancelled on the way out.
    #[test]
    fn a_fast_run_does_not_wait_for_its_deadline() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(terminal(run::TurnTerminalKind::Completed, "ready"))
            .expect("send");
        let started = std::time::Instant::now();
        let deadline = started + Duration::from_secs(30);
        let (got, timed_out) =
            wait_for_terminal(&rx, Some(deadline), Duration::from_millis(50), || {
                panic!("a run that already finished must not be cancelled")
            });
        assert!(!timed_out);
        assert_eq!(got.map(|t| t.delivery), Some("ready".to_string()));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?} for a 30 s deadline",
            started.elapsed()
        );
    }

    /// T013: at the deadline the *same* execution is cancelled, and a job that
    /// never confirms is still reported as timed out rather than hanging.
    #[test]
    fn a_deadline_cancels_the_same_execution_and_reports_timed_out() {
        let (_tx, rx) = std::sync::mpsc::channel::<run::TurnTerminal>();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let deadline = std::time::Instant::now() + Duration::from_millis(60);
        let (got, timed_out) =
            wait_for_terminal(&rx, Some(deadline), Duration::from_millis(120), move || {
                flag.store(true, Ordering::SeqCst);
            });
        assert!(timed_out, "an unconfirmed stop is still a timeout");
        assert!(
            cancelled.load(Ordering::SeqCst),
            "the deadline must signal cancel on the running execution"
        );
        assert!(got.is_none());
    }

    /// T013: a cooperative stop that confirms within the grace window keeps
    /// its own summary, but the outcome is still `timed_out`.
    #[test]
    fn a_cancelled_run_that_confirms_keeps_its_summary() {
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            let _ = tx.send(terminal(run::TurnTerminalKind::Stopped, "partial"));
        });
        let deadline = std::time::Instant::now() + Duration::from_millis(40);
        let (got, timed_out) =
            wait_for_terminal(&rx, Some(deadline), Duration::from_secs(5), || {});
        assert!(timed_out);
        assert_eq!(got.map(|t| t.kind), Some(run::TurnTerminalKind::Stopped));
        worker.join().expect("worker");
    }

    /// T016: a dropped channel is "the executor vanished", not a timeout.
    #[test]
    fn a_dropped_executor_channel_is_not_a_timeout() {
        let (tx, rx) = std::sync::mpsc::channel::<run::TurnTerminal>();
        drop(tx);
        let (got, timed_out) = wait_for_terminal(&rx, None, Duration::from_millis(10), || {});
        assert!(!timed_out);
        assert!(got.is_none());
    }

    enum Behaviour {
        Instant,
        Cooperative,
        Deaf,
        Panic,
    }

    struct FakeJob(Behaviour);

    fn report(outcome: kodo_shell::JobOutcome) -> JobReport {
        JobReport {
            outcome,
            processed: 1,
            succeeded: 0,
            failed: 0,
            reason_code: None,
            summary: None,
            report_ref: None,
        }
    }

    impl Job for FakeJob {
        fn job_type(&self) -> &str {
            "fake"
        }
        fn execute(&self, _params: &serde_json::Value, ctx: &JobContext) -> JobReport {
            match self.0 {
                Behaviour::Instant => report(kodo_shell::JobOutcome::Succeeded),
                Behaviour::Cooperative => {
                    let until = std::time::Instant::now() + Duration::from_secs(3);
                    while std::time::Instant::now() < until {
                        if ctx.cancel.is_cancelled() {
                            let mut report = report(kodo_shell::JobOutcome::Partial);
                            report.reason_code = Some("cancelled".into());
                            return report;
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    report(kodo_shell::JobOutcome::Succeeded)
                }
                Behaviour::Deaf => {
                    thread::sleep(Duration::from_secs(2));
                    report(kodo_shell::JobOutcome::Succeeded)
                }
                Behaviour::Panic => panic!("job blew up"),
            }
        }
    }

    fn job_context(run_id: &str) -> JobContext {
        JobContext {
            run_id: run_id.into(),
            deadline: None,
            cancel: JobCancellation::new(),
        }
    }

    /// I-06 / T013: the old shape always waited the full timeout and then
    /// relabelled the finished job as `timed_out`.
    #[test]
    fn a_fast_job_is_not_relabelled_timed_out() {
        let job: Arc<dyn Job> = Arc::new(FakeJob(Behaviour::Instant));
        let started = std::time::Instant::now();
        let report = run_with_timeout(
            job,
            serde_json::json!({}),
            job_context("fast"),
            Duration::from_secs(60),
            Duration::from_secs(5),
        );
        assert_eq!(report.outcome, kodo_shell::JobOutcome::Succeeded);
        assert_eq!(report.reason_code, None);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "a 1 ms job waited {:?} against a 60 s deadline",
            started.elapsed()
        );
    }

    /// T013: a cooperative job observes the cancel flag and its partial work is
    /// recorded as `timed_out`, not as a silent success.
    #[test]
    fn a_cooperative_job_stops_at_the_deadline() {
        let job: Arc<dyn Job> = Arc::new(FakeJob(Behaviour::Cooperative));
        let report = run_with_timeout(
            job,
            serde_json::json!({}),
            job_context("coop"),
            Duration::from_millis(60),
            Duration::from_secs(5),
        );
        assert_eq!(report.outcome, kodo_shell::JobOutcome::Partial);
        assert_eq!(report.reason_code.as_deref(), Some("timed_out"));
    }

    /// T016: a job that ignores cancellation is reported honestly instead of
    /// being claimed as stopped.
    #[test]
    fn an_uncooperative_job_is_reported_as_unconfirmed() {
        let job: Arc<dyn Job> = Arc::new(FakeJob(Behaviour::Deaf));
        let report = run_with_timeout(
            job,
            serde_json::json!({}),
            job_context("deaf"),
            Duration::from_millis(60),
            Duration::from_millis(80),
        );
        assert_eq!(report.outcome, kodo_shell::JobOutcome::Partial);
        assert_eq!(report.reason_code.as_deref(), Some("timed_out"));
        assert!(
            report
                .summary
                .as_deref()
                .is_some_and(|s| s.contains("uncooperative")),
            "summary was {:?}",
            report.summary
        );
    }

    /// T016: a panicking job becomes a failed run, not a stuck one.
    #[test]
    fn a_panicking_job_is_reported_as_failed() {
        let job: Arc<dyn Job> = Arc::new(FakeJob(Behaviour::Panic));
        let report = run_with_timeout(
            job,
            serde_json::json!({}),
            job_context("panic"),
            Duration::from_secs(5),
            Duration::from_millis(80),
        );
        assert_eq!(report.outcome, kodo_shell::JobOutcome::Failed);
        assert_eq!(report.reason_code.as_deref(), Some("job_panicked"));
    }

    /// T010: the session id used by the run is the one `session::open`
    /// minted, and it names a real log with the ask already recorded.
    #[test]
    fn a_scheduled_run_uses_the_session_id_the_core_minted() {
        let dir = scratch("session");
        let project = scratch("project");
        let prepared = prepare_session_in(
            &dir,
            &project,
            "整理季度报告",
            kodo_core::session::Mode::Work,
        )
        .expect("session");
        assert!(!prepared.id.is_empty());
        assert_eq!(prepared.dir, dir);
        assert!(
            dir.join(format!("{}.log", prepared.id)).exists(),
            "the id must name a real session log"
        );
        let session = kodo_core::session::load(&dir, &prepared.id).expect("load");
        assert_eq!(session.mode, kodo_core::session::Mode::Work);
        assert_eq!(session.project, project);
        assert_eq!(
            session.turns.first().map(|turn| turn.ask.as_str()),
            Some("整理季度报告")
        );

        // Two occurrences are two sessions: the id is never recycled.
        let second = prepare_session_in(&dir, &project, "第二轮", kodo_core::session::Mode::Code)
            .expect("second session");
        assert_ne!(second.id, prepared.id);
    }

    /// T014: the stored mode label survives, and a task written before the
    /// field existed stays on the legacy Code path.
    #[test]
    fn scheduled_mode_survives_and_legacy_tasks_stay_on_code() {
        assert_eq!(
            map_mode(Some("work")),
            (kodo_agent::AgentMode::Work, kodo_core::session::Mode::Work)
        );
        assert_eq!(
            map_mode(Some("code")),
            (kodo_agent::AgentMode::Code, kodo_core::session::Mode::Code)
        );
        assert_eq!(
            map_mode(None),
            (kodo_agent::AgentMode::Code, kodo_core::session::Mode::Code)
        );
        // An unknown label never silently becomes Work.
        assert_eq!(
            map_mode(Some("something-new")),
            (kodo_agent::AgentMode::Code, kodo_core::session::Mode::Code)
        );
    }

    /// T014: personas are resolved through the same registry the composer
    /// uses, and an unknown name is a specific error rather than a fallback.
    #[test]
    fn scheduled_personas_resolve_through_the_same_registry() {
        let user = scratch("agents-user");
        let project = scratch("agents-project");
        std::fs::write(
            user.join("sched-auditor.md"),
            "# Agent: sched-auditor\n## description\nScheduled auditor.\n\
             ## instructions\nSCHEDULED-PERSONA.\n",
        )
        .expect("write persona");

        let chain = vec!["sched-auditor".to_string()];
        let resolved = resolve_personas_in(&user, Some(&project), &chain).expect("resolve");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "sched-auditor");
        assert!(
            resolved[0].instructions.contains("SCHEDULED-PERSONA"),
            "instructions were {:?}",
            resolved[0].instructions
        );

        let ghost = vec!["ghost".to_string()];
        let error = resolve_personas_in(&user, Some(&project), &ghost).expect_err("unknown name");
        assert!(error.starts_with("persona_unresolved:"), "{error}");

        // No personas is not an error and never injects a default role.
        assert!(resolve_personas_in(&user, Some(&project), &[])
            .expect("empty")
            .is_empty());
    }

    /// T015: scheduled triggers never expand permissions.
    #[test]
    fn scheduled_runs_keep_the_ask_permission() {
        assert_eq!(AGENT_RUN_PERMISSION, Permission::Ask);
    }

    /// T015/T016: the resource key stays taken until the terminal row is
    /// written, and the run record carries the same session id.
    #[test]
    fn the_resource_lock_is_held_until_the_terminal_row_is_written() {
        let dir = scratch("lock");
        let scheduler = scheduler_in(&dir, 1_700_000_000);
        let task = scheduler.create_task(job_task("t-lock")).expect("create");
        let dispatched = scheduler.enqueue_manual(&task.id, "req-1").expect("manual");
        let keys = dispatched.task.task.resource_keys();
        let handle = scheduler
            .try_admit(&dispatched.run.id, &keys)
            .expect("admit");
        scheduler
            .mark_started(&dispatched.run.id)
            .expect("mark started");

        // In flight: the same key is busy.
        assert!(matches!(
            scheduler.try_admit("other-run", &keys),
            Err(SchedulerError::ResourceBusy)
        ));

        scheduler
            .mark_finished(
                &dispatched.run.id,
                Outcome::Succeeded,
                Some("delivered"),
                None,
                Some("sess-1"),
                Some("sess-1"),
                None,
                None,
                handle,
            )
            .expect("finish");

        let run = scheduler
            .store()
            .get_run(&dispatched.run.id)
            .expect("get run")
            .expect("row");
        assert_eq!(run.outcome, Some(Outcome::Succeeded));
        assert_eq!(run.session_id.as_deref(), Some("sess-1"));
        assert!(
            scheduler.try_admit("other-run", &keys).is_ok(),
            "the key must be free only after the terminal row landed"
        );
    }

    /// T016: a run left `running` by a crash is recovered as interrupted on
    /// the next start, so the task is not blocked forever.
    #[test]
    fn a_run_left_running_by_a_crash_is_recovered_as_interrupted() {
        let dir = scratch("recover");
        let first = scheduler_in(&dir, 1_700_000_000);
        let task = first.create_task(job_task("t-recover")).expect("create");
        let dispatched = first.enqueue_manual(&task.id, "req-1").expect("manual");
        let keys = dispatched.task.task.resource_keys();
        let _handle = first.try_admit(&dispatched.run.id, &keys).expect("admit");
        first.mark_started(&dispatched.run.id).expect("started");
        // Process dies here: no mark_finished, no release.

        let second = scheduler_in(&dir, 1_700_000_500);
        let recovered = recover_inflight(second.store(), 1_700_000_500).expect("recover");
        assert_eq!(recovered, 1);
        let run = second
            .store()
            .get_run(&dispatched.run.id)
            .expect("get run")
            .expect("row");
        assert_eq!(run.outcome, Some(Outcome::Interrupted));
        assert!(run.finished_at.is_some());
    }

    /// T016: releasing with a stale token must not evict a newer run.
    #[test]
    fn releasing_a_stale_generation_leaves_the_newer_run() {
        let runs = Runs::default();
        let stale = runs.begin("s").expect("first");
        runs.cancel("s");
        runs.release("s", &stale);
        assert!(!runs.has_run("s"));

        let fresh = runs.begin("s").expect("second");
        // A late `release` from the earlier generation arrives now.
        runs.release("s", &stale);
        assert!(
            runs.has_run("s"),
            "a stale release must not evict the current generation"
        );
        assert!(fresh.is_live());
        runs.release("s", &fresh);
        assert!(!runs.has_run("s"));
    }
}
