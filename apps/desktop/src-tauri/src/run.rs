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
use std::time::Duration;

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
}

impl RunToken {
    pub fn is_live(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
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
        };
        runs.insert(id.to_owned(), token.clone());
        Some(token)
    }
    pub fn has_run(&self, id: &str) -> bool {
        self.lock().contains_key(id)
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

#[derive(Default, Clone)]
pub struct Approvals {
    waits: Arc<Mutex<WaitMap>>,
    /// session id → command fingerprints the user allowed for the whole session.
    session_allows: Arc<Mutex<AllowMap>>,
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
            Some(tx) => tx.send(choice).is_ok(),
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
    pub provider: Option<agent::Provider>,
    pub permission: Permission,
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
                emit_run(
                    &approve_app,
                    &approve_token,
                    RunEvent::ApprovalRequest {
                        session: approve_id.clone(),
                        step,
                        kind: step_kind_label(kind).to_owned(),
                        detail: format!("{safe_command}  ·  {reason}"),
                        command: safe_command.clone(),
                        cwd,
                        risk_category: risk_category.to_owned(),
                        reason: reason.to_owned(),
                    },
                );
                // Approval wait must respond to run cancellation — poll in
                // short slices instead of one 300s blocking recv.
                let deadline = std::time::Instant::now() + Duration::from_secs(300);
                let mut choice = ApprovalChoice::Deny;
                while std::time::Instant::now() < deadline {
                    if !approve_token.is_live() {
                        // Cancelled during approval → deny, do not run the step.
                        break;
                    }
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(value) => {
                            choice = value;
                            break;
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
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
            pinned_context: args.context.clone(),
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
        };

        let result = agent::run(&request, &alive, &approve, &mut sink);
        approvals.clear_session(&args.id);

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
    });

    Ok(())
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
