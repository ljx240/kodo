//! The run driver: threads, session log writes, and the approval rendezvous.
//!
//! Work itself lives in `kodo-agent`. Steps are written **started** before the
//! work runs and **completed** when it finishes, so a killed run leaves a real
//! `running` envelope in the log.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter};

use kodo_agent::{self as agent, SinkEvent, StepKind};
use kodo_memory::SqliteMemory;
use kodo_shell::{approval_fingerprint, step_kind_label, Logged, TurnLogger, TurnOutcome};

use crate::view::{ItemView, RunEvent};
use kodo_memory::MemoryHandle;

pub use kodo_agent::Permission;
pub use kodo_shell::ApprovalChoice;

#[derive(Clone)]
pub struct RunToken {
    generation: u64,
    alive: Arc<AtomicBool>,
    started_at: u64,
    progress: Arc<Mutex<Option<RunProgressView>>>,
}

impl RunToken {
    pub fn is_live(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// The run's generation. A consent is bound to it, so a grant from an
    /// earlier run can never be used by a later one.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The liveness flag, so a host that waits on a human can notice a
    /// stopped run.
    pub fn alive_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.alive)
    }

    fn set_progress(&self, phase: &str, detail: &str) {
        let mut current = self
            .progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current = Some(RunProgressView {
            phase: phase.to_owned(),
            detail: detail.to_owned(),
        });
    }
}

/// The phase line a reopened conversation shows for a still-running turn.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunProgressView {
    pub phase: String,
    pub detail: String,
}

/// A step blocked on the user, mirrored here so switching away and back does
/// not lose the approval card (the original `approvalRequest` event is not
/// replayed by the runner).
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApprovalView {
    pub run_id: u64,
    pub step: u32,
    pub kind: String,
    pub detail: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub risk_category: String,
    #[serde(default)]
    pub reason: String,
}

/// Authoritative live facts for one session. `approval` is only populated for
/// the generation that currently owns the session.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStateView {
    pub running: bool,
    pub started_at: Option<u64>,
    pub progress: Option<RunProgressView>,
    pub approval: Option<PendingApprovalView>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Default, Clone)]
pub struct Runs(Arc<Mutex<HashMap<String, RunToken>>>);

impl Runs {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, RunToken>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    pub fn begin(&self, id: &str) -> Option<RunToken> {
        static GENERATION: AtomicU64 = AtomicU64::new(1);
        let mut runs = self.lock();
        if runs.contains_key(id) {
            return None;
        }
        let token = RunToken {
            generation: GENERATION.fetch_add(1, Ordering::SeqCst),
            alive: Arc::new(AtomicBool::new(true)),
            started_at: now_ms(),
            progress: Arc::new(Mutex::new(None)),
        };
        runs.insert(id.to_owned(), token.clone());
        Some(token)
    }
    pub fn has_run(&self, id: &str) -> bool {
        self.lock().contains_key(id)
    }
    /// Read-only snapshot for a conversation that is being reopened. A stale
    /// approval from an earlier generation is filtered out here rather than
    /// trusted to cleanup timing.
    pub fn state(&self, id: &str, approval: Option<PendingApprovalView>) -> RunStateView {
        let runs = self.lock();
        let Some(token) = runs.get(id) else {
            return RunStateView {
                running: false,
                started_at: None,
                progress: None,
                approval: None,
            };
        };
        let progress = token
            .progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        RunStateView {
            running: true,
            started_at: Some(token.started_at),
            progress,
            approval: approval.filter(|pending| pending.run_id == token.generation),
        }
    }
    pub fn cancel(&self, id: &str) {
        if let Some(token) = self.lock().get(id) {
            token.alive.store(false, Ordering::SeqCst);
        }
    }
    pub fn release(&self, id: &str, token: &RunToken) {
        let mut runs = self.lock();
        if runs
            .get(id)
            .is_some_and(|current| current.generation == token.generation)
        {
            runs.remove(id);
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RunEventEnvelope {
    run_id: u64,
    #[serde(flatten)]
    event: RunEvent,
}

fn emit_run(app: &AppHandle, token: &RunToken, event: RunEvent) {
    // Mirror the run's lifecycle onto the status item. The window can be hidden
    // for the whole run (closing hides rather than quits), so this is the only
    // ambient signal that the agent is still working — hence it hangs off the
    // one funnel every run event already passes through.
    match &event {
        RunEvent::TurnStarted { .. } => crate::platform::set_run_status(app, true),
        RunEvent::TurnComplete { .. } | RunEvent::Stopped { .. } | RunEvent::Error { .. } => {
            crate::platform::set_run_status(app, false)
        }
        _ => {}
    }
    let _ = app.emit(
        "run:event",
        RunEventEnvelope {
            run_id: token.generation,
            event,
        },
    );
}

type WaitMap = HashMap<(String, u64, u32), Sender<ApprovalChoice>>;
type AllowMap = HashMap<String, HashSet<String>>;

type PendingMap = HashMap<String, PendingApprovalView>;

#[derive(Default, Clone)]
pub struct Approvals {
    waits: Arc<Mutex<WaitMap>>,
    /// session id → command fingerprints the user allowed for the whole session.
    session_allows: Arc<Mutex<AllowMap>>,
    /// session id → the approval card currently blocking that run.
    pending: Arc<Mutex<PendingMap>>,
}

impl Approvals {
    fn waits(&self) -> MutexGuard<'_, WaitMap> {
        self.waits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn allows(&self) -> MutexGuard<'_, AllowMap> {
        self.session_allows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn pendings(&self) -> MutexGuard<'_, PendingMap> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn set_pending(&self, session: &str, approval: PendingApprovalView) {
        self.pendings().insert(session.to_owned(), approval);
    }

    pub fn pending(&self, session: &str) -> Option<PendingApprovalView> {
        self.pendings().get(session).cloned()
    }

    /// Drop the card only when it still belongs to this generation + step —
    /// a late ticket from a cancelled run must not clear its successor's card.
    fn clear_pending(&self, session: &str, generation: u64, step: u32) {
        let mut pending = self.pendings();
        let owned = pending
            .get(session)
            .is_some_and(|view| view.run_id == generation && view.step == step);
        if owned {
            pending.remove(session);
        }
    }

    pub fn wait_point(
        &self,
        session: &str,
        generation: u64,
        step: u32,
    ) -> (ApprovalTicket, Receiver<ApprovalChoice>) {
        let (tx, rx) = mpsc::channel();
        self.waits()
            .insert((session.to_owned(), generation, step), tx);
        (
            ApprovalTicket {
                session: session.to_owned(),
                step,
                generation,
                map: self.clone(),
            },
            rx,
        )
    }

    pub fn resolve(
        &self,
        session: &str,
        generation: u64,
        step: u32,
        choice: ApprovalChoice,
    ) -> bool {
        match self.waits().remove(&(session.to_owned(), generation, step)) {
            Some(tx) => {
                self.clear_pending(session, generation, step);
                tx.send(choice).is_ok()
            }
            None => false,
        }
    }

    pub fn remember_session_allow(&self, session: &str, fingerprint: &str) {
        self.allows()
            .entry(session.to_owned())
            .or_default()
            .insert(fingerprint.to_owned());
    }

    pub fn is_session_allowed(&self, session: &str, fingerprint: &str) -> bool {
        self.allows()
            .get(session)
            .map(|set| set.contains(fingerprint))
            .unwrap_or(false)
    }

    pub fn clear_session(&self, session: &str) {
        self.waits().retain(|(id, _, _), _| id != session);
        self.pendings().remove(session);
    }
}

pub struct ApprovalTicket {
    session: String,
    step: u32,
    generation: u64,
    map: Approvals,
}

impl Drop for ApprovalTicket {
    fn drop(&mut self) {
        self.map
            .waits()
            .remove(&(self.session.clone(), self.generation, self.step));
        self.map
            .clear_pending(&self.session, self.generation, self.step);
    }
}

pub struct StartArgs {
    pub dir: PathBuf,
    pub id: String,
    /// `None` = no project selected (Work-mode pure conversation).
    pub project: Option<PathBuf>,
    pub message: String,
    pub conversation_history: Vec<(String, String)>,
    /// Approved evidence for this turn; never persisted as a user instruction.
    pub knowledge_context: Option<agent::knowledge::KnowledgeContext>,
    /// Project-relative context paths pinned for this turn.
    pub context: Vec<String>,
    /// Paths the user attached explicitly (paste / drop with no shell-known
    /// filesystem path). The agent's image inlining branch honors these
    /// even when the provider capability flag has vision=false (the user's
    /// intent is the strong signal; vision_ok only gates auto-attached
    /// or history-inherited contexts).
    pub force_inline: Vec<String>,
    pub provider: Option<agent::Provider>,
    pub permission: Permission,
    /// How long to wait for a human approval decision. `None` = wait until
    /// the user answers or the run is cancelled. Only headless callers, which
    /// have no UI to answer the card, pass a bound.
    pub approval_timeout: Option<Duration>,
    pub mode: agent::AgentMode,
    pub fallback_to_local: bool,
    pub max_output_tokens: u32,
    pub extended_thinking: bool,
    pub local_trace: bool,
    /// MCP servers from the shared shell config (empty = no external tools).
    pub mcp_servers: Vec<agent::mcp::McpServerConfig>,
    /// Persona instructions resolved by the entry point (插件 · 智能体);
    /// `None` = no agent selected for this send.
    pub agent_instructions: Option<String>,
    /// Persona chain (preferred when non-empty). Each block carries its
    /// defaults / examples; `lib.rs::run` substitutes the declared
    /// template variables and emits the chain header.
    pub personas: Vec<agent::PersonaBlock>,
    /// Optional budget-family override. `None` = per-intent default;
    /// `Some("summary")` = lift to read-only-summary caps (12 / 128 / 0).
    /// Currently only the desktop "继续本轮" affordance sets this to
    /// `"summary"` when a prior turn died with `BudgetExhausted`.
    pub budget_family: Option<String>,
    /// Optional decision-assistance handle (Jev).
    ///
    /// `None` — the background scheduler's path, and every default — means
    /// this run makes no decision request at all. A scheduled run must not
    /// inherit a desktop approval, and it must not open a prompt that
    /// nobody is there to answer.
    pub decision: Option<agent::DecisionHandle>,
    /// Trusted terminal summary channel.
    ///
    /// `start()` returns as soon as the thread is spawned, which says nothing
    /// about the turn. A caller that must record a *real* terminal state (the
    /// scheduler) passes a sender here and waits for the summary the executor
    /// builds from the actual `SinkEvent` stream. Interactive callers pass
    /// `None` and keep relying on the UI event stream.
    pub terminal: Option<Sender<TurnTerminal>>,
}

/// How a run actually ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnTerminalKind {
    /// The agent loop returned and a terminal answer was recorded.
    Completed,
    /// The user (or a scheduler deadline) stopped it; the loop confirmed exit.
    Stopped,
    /// The loop returned an error.
    Failed,
}

/// Trusted terminal summary for one run, built from real events only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnTerminal {
    pub kind: TurnTerminalKind,
    /// Last terminal answer's delivery axis (`ready` / `partial` / `failed` /
    /// `blocked`), empty when the run never produced an answer.
    pub delivery: String,
    pub verification: String,
    pub failure_reason: String,
}

impl TurnTerminal {
    fn new(kind: TurnTerminalKind, answer: Option<(String, String, String)>) -> Self {
        let (delivery, verification, failure_reason) = answer.unwrap_or_default();
        Self {
            kind,
            delivery,
            verification,
            failure_reason,
        }
    }
}

pub fn start(
    app: &AppHandle,
    runs: &Runs,
    approvals: &Approvals,
    args: StartArgs,
    token: RunToken,
) -> Result<(), String> {
    let app = app.clone();
    let runs = runs.clone();
    let approvals = approvals.clone();

    thread::spawn(move || {
        let notify = |event: RunEvent| {
            emit_run(&app, &token, event);
        };

        notify(RunEvent::TurnStarted {
            session: args.id.clone(),
        });

        let alive_token = token.clone();
        let alive = move || alive_token.is_live();

        let approve_id = args.id.clone();
        let approve_approvals = approvals.clone();
        let approve_app = app.clone();
        let approve_project = args.project.clone().unwrap_or_default();
        let approve_token = token.clone();
        let permission = args.permission;
        let approval_timeout = args.approval_timeout;
        let approve_seq = Arc::new(Mutex::new(0u32));
        let approve = {
            let approve_seq = approve_seq.clone();
            move |kind: StepKind, command: &str| -> bool {
                if !permission.needs_approval(kind, Some(command)) {
                    return true;
                }
                // Session-wide allow from an earlier "本会话允许".
                let fingerprint = approval_fingerprint(kind, command);
                if approve_approvals.is_session_allowed(&approve_id, &fingerprint) {
                    return true;
                }
                // Run already cancelled — never wait for an approval ticket.
                if !approve_token.is_live() {
                    return false;
                }
                let mut seq = approve_seq.lock().unwrap_or_else(|p| p.into_inner());
                *seq += 1;
                let step = *seq;
                drop(seq);
                let (ticket, rx) =
                    approve_approvals.wait_point(&approve_id, approve_token.generation, step);
                let risk = kodo_agent::tools::classify_command_risk(command);
                let reason = agent::tools::redact_secrets(
                    agent::dangerous_reason(command).unwrap_or_else(|| risk.reason()),
                );
                let risk_category = match risk {
                    kodo_agent::tools::CommandRisk::Catastrophic => "Catastrophic",
                    kodo_agent::tools::CommandRisk::DestructiveGit => "DestructiveGit",
                    kodo_agent::tools::CommandRisk::SensitiveData => "SensitiveData",
                    kodo_agent::tools::CommandRisk::Unknown => "Unknown",
                    kodo_agent::tools::CommandRisk::ProcessControl => "ProcessControl",
                    kodo_agent::tools::CommandRisk::PackageInstall => "PackageInstall",
                    kodo_agent::tools::CommandRisk::Network => "Network",
                    kodo_agent::tools::CommandRisk::FilesystemWrite => "FilesystemWrite",
                    kodo_agent::tools::CommandRisk::ReadOnly => {
                        if matches!(kind, StepKind::FileChange) {
                            "FilesystemWrite"
                        } else {
                            "Safe"
                        }
                    }
                };
                let cwd = approve_project.to_string_lossy().into_owned();
                let safe_command = kodo_agent::tools::redact_secrets(command);
                let kind_label = step_kind_label(kind).to_owned();
                let detail = format!("{safe_command}  ·  {reason}");
                // Mirror the request before emitting: a conversation that is
                // not open right now still needs this card when it comes back.
                approve_approvals.set_pending(
                    &approve_id,
                    PendingApprovalView {
                        run_id: approve_token.generation,
                        step,
                        kind: kind_label.clone(),
                        detail: detail.clone(),
                        command: safe_command.clone(),
                        cwd: cwd.clone(),
                        risk_category: risk_category.to_owned(),
                        reason: reason.to_owned(),
                    },
                );
                emit_run(
                    &approve_app,
                    &approve_token,
                    RunEvent::ApprovalRequest {
                        session: approve_id.clone(),
                        step,
                        kind: kind_label,
                        detail,
                        command: safe_command,
                        cwd,
                        risk_category: risk_category.to_owned(),
                        reason: reason.to_owned(),
                    },
                );
                // Approval waits on the human, not on a clock: a card left
                // unanswered for five minutes is not a denial. Cancellation
                // (or a headless deadline) is the only way out without an
                // answer.
                let deadline = approval_timeout.map(|timeout| std::time::Instant::now() + timeout);
                let choice = wait_for_approval(&rx, deadline, &approve_token);
                drop(ticket);
                if choice == ApprovalChoice::AllowSession {
                    approve_approvals.remember_session_allow(&approve_id, &fingerprint);
                }
                choice.allows()
            }
        };

        let record_id = args.id.clone();
        let record_token = token.clone();
        let record_app = app.clone();
        // Memory handle for the outbox writer. Opened independently of
        // the `RunRequest` so the sink can forward started/completed
        // items without having to thread the request through the
        // closure capture. Same on-disk file as the agent loop will
        // see — fail-open if HOME is unset.
        // Evidence and knowledge-derived answers must not enter the ordinary
        // memory outbox and later bypass per-turn approval as recalled memory.
        let memory_handle = if args.knowledge_context.is_some() {
            None
        } else {
            open_memory_handle()
        };
        // Envelope writes live in kodo-shell; this sink records first, then
        // emits, so the log stays the source of truth for the GUI.
        let mut logger =
            TurnLogger::new(args.dir.clone(), args.id.clone()).with_capture(args.local_trace);
        let logger_ref = &mut logger;
        // Track whether the agent loop emitted a terminal `AgentMessage`
        // with `delivery == "ready"`. If it didn't, the run loop exited
        // without a final answer — the most common cause in review-style
        // tasks is context truncation, and the GUI should still see a
        // synthesized "partial" answer instead of `TurnComplete` with no
        // body. See `synthesize_fallback_answer` below.
        let answered: std::sync::Arc<std::sync::atomic::AtomicBool> =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let answered_for_sink = answered.clone();
        // Last terminal answer's axes, kept for the trusted terminal summary.
        let last_answer: Arc<Mutex<Option<(String, String, String)>>> = Arc::new(Mutex::new(None));
        let last_answer_for_sink = last_answer.clone();
        let terminal = args.terminal.clone();

        let mut sink = move |event: SinkEvent| -> bool {
            if !record_token.is_live() {
                return false;
            }
            let logged = logger_ref.record(&event);
            if matches!(logged, Logged::WriteFailed) {
                return false;
            }
            match event {
                SinkEvent::LocalTrace { .. } => {}
                SinkEvent::TextDelta { text } => {
                    emit_run(
                        &record_app,
                        &record_token,
                        RunEvent::TextDelta {
                            session: record_id.clone(),
                            text,
                        },
                    );
                }
                SinkEvent::Progress { phase, detail } => {
                    record_token.set_progress(&phase, &detail);
                    emit_run(
                        &record_app,
                        &record_token,
                        RunEvent::Progress {
                            session: record_id.clone(),
                            phase,
                            detail,
                        },
                    );
                }
                SinkEvent::Failover {
                    from_provider,
                    from_model,
                    error_class,
                    error,
                    to_provider,
                    to_model,
                } => {
                    emit_run(
                        &record_app,
                        &record_token,
                        RunEvent::Failover {
                            session: record_id.clone(),
                            from_provider,
                            from_model,
                            error_class,
                            error,
                            to_provider,
                            to_model,
                        },
                    );
                }
                SinkEvent::Started { .. } => {
                    if let Logged::Started(item) = logged {
                        if let (Some(handle), Some(event)) = (
                            memory_handle.as_ref(),
                            kodo_shell::event_for_item(&item, kodo_core::session::Phase::Started),
                        ) {
                            kodo_shell::write_event(handle, &event);
                        }
                        emit_run(
                            &record_app,
                            &record_token,
                            RunEvent::ItemStarted {
                                session: record_id.clone(),
                                item: ItemView::from(item),
                            },
                        );
                    }
                }
                SinkEvent::Finished { .. } => {
                    if let Logged::Finished(item) = logged {
                        let phase = match item.status {
                            kodo_core::session::Status::Failed => kodo_core::session::Phase::Failed,
                            _ => kodo_core::session::Phase::Completed,
                        };
                        if let (Some(handle), Some(event)) = (
                            memory_handle.as_ref(),
                            kodo_shell::event_for_item(&item, phase),
                        ) {
                            kodo_shell::write_event(handle, &event);
                        }
                        emit_run(
                            &record_app,
                            &record_token,
                            RunEvent::ItemCompleted {
                                session: record_id.clone(),
                                item: ItemView::from(item),
                            },
                        );
                    }
                }
                SinkEvent::ContextBudget {
                    used,
                    window,
                    percent,
                    cumulative_chars,
                    compacted_count,
                    last_model,
                } => {
                    emit_run(
                        &record_app,
                        &record_token,
                        RunEvent::ContextBudget {
                            session: record_id.clone(),
                            used,
                            window,
                            percent,
                            cumulative_chars,
                            compacted_count,
                            last_model,
                        },
                    );
                }
                SinkEvent::Compacted {
                    dropped,
                    summarized_into,
                    freed_pct,
                } => {
                    emit_run(
                        &record_app,
                        &record_token,
                        RunEvent::Compacted {
                            session: record_id.clone(),
                            dropped,
                            summarized_into,
                            freed_pct,
                        },
                    );
                }
                SinkEvent::MemoryRecalled { count, ids } => {
                    emit_run(
                        &record_app,
                        &record_token,
                        RunEvent::MemoryRecalled {
                            session: record_id.clone(),
                            count,
                            ids,
                        },
                    );
                }
                SinkEvent::AskUser { question, options } => {
                    emit_run(
                        &record_app,
                        &record_token,
                        RunEvent::AskUser {
                            session: record_id.clone(),
                            question,
                            options,
                        },
                    );
                }
                SinkEvent::AgentMessage {
                    text,
                    delivery,
                    verification,
                    plain,
                    failure_reason,
                    checks,
                } => {
                    // Any terminal `AgentMessage` counts as the agent
                    // loop's final answer — the post-run fallback only
                    // fires when zero `AgentMessage` events were emitted
                    // (the "执行成功但未返回正文" symptom). Delivery
                    // "failed" still counts: the loop guard (F3-3) uses
                    // it to surface a forced-terminal body, and the user
                    // must not see a second synthesised message on top.
                    answered_for_sink.store(true, std::sync::atomic::Ordering::SeqCst);
                    *last_answer_for_sink
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((
                        delivery.clone(),
                        verification.clone(),
                        failure_reason.clone(),
                    ));
                    emit_run(
                        &record_app,
                        &record_token,
                        RunEvent::AgentMessage {
                            session: record_id.clone(),
                            text,
                            delivery,
                            verification,
                            plain,
                            failure_reason,
                            checks,
                        },
                    );
                }
            }
            record_token.is_live()
        };

        let request = agent::RunRequest {
            project: args.project.clone(),
            message: args.message.clone(),
            conversation_history: args.conversation_history.clone(),
            knowledge_context: args.knowledge_context.clone(),
            allow_verify_commands: false,
            pinned_context: args.context.clone(),
            pinned_force_inline: args.force_inline.clone(),
            provider: args.provider.clone(),
            permission: args.permission,
            mode: args.mode,
            fallback_to_local: args.fallback_to_local,
            max_output_tokens: args.max_output_tokens,
            extended_thinking: args.extended_thinking,
            local_trace: args.local_trace,
            session_id: Some(args.id.clone()),
            // Same directory the skill CRUD commands read/write.
            user_skills_dir: kodo_core::settings::state_dir().map(|d| d.join("skills")),
            mcp_servers: args.mcp_servers.clone(),
            agent_instructions: args.agent_instructions.clone(),
            personas: args.personas.clone(),
            budget_family: args.budget_family.clone(),
            memory: if args.knowledge_context.is_some() {
                None
            } else {
                open_memory_handle()
            },
            decision: args.decision.clone(),
        };

        let result = agent::run(&request, &alive, &approve, &mut sink);
        approvals.clear_session(&args.id);
        let terminal_kind = if result.is_err() {
            TurnTerminalKind::Failed
        } else if !token.is_live() {
            TurnTerminalKind::Stopped
        } else {
            TurnTerminalKind::Completed
        };

        match result {
            Ok(()) => {
                if !token.is_live() {
                    let _ = logger.finish(TurnOutcome::Stopped);
                    notify(RunEvent::Stopped {
                        session: args.id.clone(),
                    });
                } else if !answered.load(std::sync::atomic::Ordering::SeqCst) {
                    // The agent loop returned cleanly but never emitted a
                    // terminal `AgentMessage` (the "执行成功但未返回正文"
                    // symptom). Synthesize a partial answer so the GUI
                    // surfaces a body instead of an empty `TurnComplete`.
                    let fallback = synthesize_fallback_answer(&request.message);
                    notify(RunEvent::AgentMessage {
                        session: args.id.clone(),
                        text: fallback,
                        delivery: "partial".to_owned(),
                        verification: "not_run".to_owned(),
                        plain: false,
                        failure_reason: "loop_terminated_without_answer".to_owned(),
                        checks: Vec::new(),
                    });
                    if logger.finish(TurnOutcome::Complete).is_err() {
                        notify(RunEvent::Error {
                            session: args.id.clone(),
                            message: "failed to mark the turn complete".to_owned(),
                        });
                    } else {
                        notify(RunEvent::TurnComplete {
                            session: args.id.clone(),
                        });
                    }
                } else if logger.finish(TurnOutcome::Complete).is_err() {
                    notify(RunEvent::Error {
                        session: args.id.clone(),
                        message: "failed to mark the turn complete".to_owned(),
                    });
                } else {
                    notify(RunEvent::TurnComplete {
                        session: args.id.clone(),
                    });
                }
            }
            Err(error) => {
                let safe_error = kodo_agent::tools::redact_secrets(&error);
                // finish() redacts again before recording; cheap and idempotent.
                let _ = logger.finish(TurnOutcome::Error(error));
                notify(RunEvent::Error {
                    session: args.id.clone(),
                    message: safe_error,
                });
            }
        }
        runs.release(&args.id, &token);
        // Publish the trusted terminal summary *after* the run mapping is
        // released, so a waiter that wakes up cannot observe a half-finished
        // cleanup. A dropped receiver (interactive runs) is not an error.
        if let Some(tx) = terminal {
            let answer = last_answer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let _ = tx.send(TurnTerminal::new(terminal_kind, answer));
        }
    });

    Ok(())
}

/// Wait for an approval answer or a reason to stop waiting. `deadline` is
/// only set for headless runs; interactive runs stay pending until the user
/// answers or cancels the run.
fn wait_for_approval(
    rx: &Receiver<ApprovalChoice>,
    deadline: Option<std::time::Instant>,
    token: &RunToken,
) -> ApprovalChoice {
    loop {
        if !token.is_live() {
            return ApprovalChoice::Deny;
        }
        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            return ApprovalChoice::Deny;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(choice) => return choice,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return ApprovalChoice::Deny;
            }
        }
    }
}

/// Build a short, honest fallback answer when the agent loop exits
/// without producing a terminal `AgentMessage`. The text is generic
/// (not a fabricated review) — the user reads it as "I ran out of
/// budget before the model could write an answer", and the UI keeps
/// the live transcript / response-trace links working.
fn synthesize_fallback_answer(original_message: &str) -> String {
    let trimmed = original_message.trim();
    let head = if trimmed.is_empty() {
        "你".to_owned()
    } else {
        format!("「{trimmed}」")
    };
    format!(
        "{head} 这一轮我执行了若干读操作，但模型在生成最终答复前就被截断了 —— 常见原因是上下文窗口耗尽、复合命令被权限策略拒绝后没有 fallback 路径，或可读取文件过多挤掉了正文。\n\n可尝试：\n1. 在「查看响应轨迹」里看执行记录和已读取文件清单；\n2. 在 Settings → 模型中调大「最大输出 tokens」后重试；\n3. 如果是 review / 调研类任务，先把任务拆成更小的子任务再发。",
    )
}

/// Open the on-disk memory store at `<state_dir>/memory/v1/memory.sqlite`
/// and wrap it in a `MemoryHandle`. Returns `None` if the state directory
/// is unavailable (HOME unset, sandbox restrictions) — fail-open: the
/// agent loop continues with the memory channel disabled.
fn open_memory_handle() -> Option<MemoryHandle> {
    let state_dir = kodo_core::settings::state_dir()?;
    let path = state_dir.join("memory").join("v1").join("memory.sqlite");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match SqliteMemory::open(&path) {
        Ok(mem) => Some(mem.into()),
        Err(e) => {
            eprintln!(
                "kodo memory: failed to open store at {}: {e}",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_run_cannot_revive_or_release_successor() {
        let runs = Runs::default();
        let first = runs.begin("session").unwrap();
        runs.cancel("session");
        assert!(!first.is_live());
        assert!(
            runs.begin("session").is_none(),
            "old worker must finish before new admission"
        );
        runs.release("session", &first);
        let second = runs.begin("session").unwrap();
        assert_ne!(first.generation, second.generation);
        runs.release("session", &first);
        assert!(runs.has_run("session") && second.is_live());
        assert!(!first.is_live());
    }

    #[test]
    fn late_approval_and_old_ticket_cannot_touch_new_run() {
        let approvals = Approvals::default();
        let (old, _) = approvals.wait_point("s", 1, 1);
        approvals.clear_session("s");
        let (_new, rx) = approvals.wait_point("s", 2, 1);
        drop(old);
        assert!(!approvals.resolve("s", 1, 1, ApprovalChoice::AllowOnce));
        assert!(approvals.resolve("s", 2, 1, ApprovalChoice::Deny));
        assert!(!rx.recv().unwrap().allows());
    }

    #[test]
    fn session_allow_survives_turn_cleanup_and_remains_scoped() {
        let approvals = Approvals::default();
        let command = r#"mcp__github__connect {"command":"server-a"}"#;
        let fingerprint = approval_fingerprint(StepKind::ExternalTool, command);
        approvals.remember_session_allow("s", &fingerprint);
        let (_ticket, _) = approvals.wait_point("s", 1, 1);
        approvals.clear_session("s");
        let next_turn = approvals.clone();
        assert!(next_turn.is_session_allowed("s", &fingerprint));
        assert!(!next_turn.is_session_allowed("other", &fingerprint));
        assert!(!next_turn.is_session_allowed(
            "s",
            &approval_fingerprint(
                StepKind::ExternalTool,
                r#"mcp__github__connect {"command":"server-b"}"#,
            )
        ));
        assert!(
            !next_turn.is_session_allowed("s", &approval_fingerprint(StepKind::Command, command))
        );
    }

    fn pending(run_id: u64, step: u32) -> PendingApprovalView {
        PendingApprovalView {
            run_id,
            step,
            kind: "Command".into(),
            detail: "cargo test  ·  safe".into(),
            command: "cargo test".into(),
            cwd: "/tmp/project".into(),
            risk_category: "Safe".into(),
            reason: "tests".into(),
        }
    }

    #[test]
    fn run_state_reports_started_at_progress_and_pending_approval() {
        let runs = Runs::default();
        let approvals = Approvals::default();
        let idle = runs.state("s", approvals.pending("s"));
        assert!(!idle.running);
        assert!(idle.started_at.is_none());

        let token = runs.begin("s").unwrap();
        token.set_progress("execute", "cargo test");
        approvals.set_pending("s", pending(token.generation, 1));
        let live = runs.state("s", approvals.pending("s"));
        assert!(live.running);
        assert!(live.started_at.is_some_and(|started| started > 0));
        assert_eq!(live.progress.unwrap().phase, "execute");
        let approval = live.approval.expect("pending approval is visible");
        assert_eq!(approval.run_id, token.generation);
        assert_eq!(approval.step, 1);

        // A card from an older generation is not surfaced for this run.
        approvals.set_pending("s", pending(token.generation + 1, 2));
        assert!(runs.state("s", approvals.pending("s")).approval.is_none());
        approvals.clear_session("s");
        assert!(runs.state("s", approvals.pending("s")).approval.is_none());

        runs.release("s", &token);
        assert!(!runs.state("s", approvals.pending("s")).running);
    }

    #[test]
    fn pending_approval_clears_without_touching_a_successor() {
        let approvals = Approvals::default();
        let (ticket, _rx) = approvals.wait_point("s", 1, 1);
        approvals.set_pending("s", pending(1, 1));
        assert!(approvals.pending("s").is_some());
        assert!(approvals.resolve("s", 1, 1, ApprovalChoice::AllowOnce));
        assert!(approvals.pending("s").is_none());
        drop(ticket);

        // A late ticket from generation 1 must not clear generation 2's card.
        let (stale, _) = approvals.wait_point("s", 1, 2);
        let (_current, _) = approvals.wait_point("s", 2, 1);
        approvals.set_pending("s", pending(2, 1));
        drop(stale);
        assert_eq!(approvals.pending("s").unwrap().run_id, 2);
        approvals.clear_session("s");
        assert!(approvals.pending("s").is_none());
    }

    fn test_token(live: bool) -> RunToken {
        RunToken {
            generation: 1,
            alive: Arc::new(AtomicBool::new(live)),
            started_at: 0,
            progress: Arc::new(Mutex::new(None)),
        }
    }

    #[test]
    fn interactive_approval_waits_for_the_user_without_a_deadline() {
        let (tx, rx) = mpsc::channel();
        let token = test_token(true);
        let answer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            tx.send(ApprovalChoice::AllowOnce)
                .expect("wait point still open");
        });
        assert_eq!(
            wait_for_approval(&rx, None, &token),
            ApprovalChoice::AllowOnce
        );
        answer.join().unwrap();
    }

    #[test]
    fn approval_wait_denies_when_the_run_is_cancelled() {
        let (_tx, rx) = mpsc::channel();
        assert_eq!(
            wait_for_approval(&rx, None, &test_token(false)),
            ApprovalChoice::Deny
        );
    }

    #[test]
    fn approval_wait_denies_when_the_channel_is_gone() {
        let (tx, rx) = mpsc::channel::<ApprovalChoice>();
        drop(tx);
        assert_eq!(
            wait_for_approval(&rx, None, &test_token(true)),
            ApprovalChoice::Deny
        );
    }

    #[test]
    fn headless_approval_still_has_a_deadline() {
        let (_tx, rx) = mpsc::channel();
        let deadline = std::time::Instant::now() + Duration::from_millis(30);
        assert_eq!(
            wait_for_approval(&rx, Some(deadline), &test_token(true)),
            ApprovalChoice::Deny
        );
    }

    #[test]
    fn event_envelope_serializes_generation_without_changing_event_shape() {
        let encoded = serde_json::to_value(RunEventEnvelope {
            run_id: 7,
            event: RunEvent::TurnStarted {
                session: "s".into(),
            },
        })
        .unwrap();
        assert_eq!(encoded["runId"], 7);
        assert_eq!(encoded["type"], "turnStarted");
    }
}
