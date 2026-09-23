//! The run driver: threads, session log writes, and the approval rendezvous.
//!
//! Work itself lives in `kodo-agent`. Steps are written **started** before the
//! work runs and **completed** when it finishes, so a killed run leaves a real
//! `running` envelope in the log.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use kodo_agent::{self as agent, FileDelta, SinkEvent, Step, StepKind};
use kodo_core::session::{self, Item, ItemKind, Phase, Status};

use crate::view::{ItemView, RunEvent};

pub use kodo_agent::Permission;

#[derive(Default, Clone)]
pub struct Runs(Arc<Mutex<HashSet<String>>>);

impl Runs {
    fn lock(&self) -> MutexGuard<'_, HashSet<String>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn begin(&self, id: &str) -> bool {
        self.lock().insert(id.to_owned())
    }

    pub fn is_live(&self, id: &str) -> bool {
        self.lock().contains(id)
    }

    pub fn cancel(&self, id: &str) {
        self.lock().remove(id);
    }
}

/// One approval decision from the user. `AllowSession` remembers the command
/// fingerprint for the rest of the conversation (codex-style graduated allow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalChoice {
    Deny,
    AllowOnce,
    AllowSession,
}

impl ApprovalChoice {
    pub fn from_parts(approved: bool, session_wide: bool) -> Self {
        match (approved, session_wide) {
            (false, _) => Self::Deny,
            (true, true) => Self::AllowSession,
            (true, false) => Self::AllowOnce,
        }
    }

    pub fn allows(self) -> bool {
        matches!(self, Self::AllowOnce | Self::AllowSession)
    }
}

type WaitMap = HashMap<(String, u32), Sender<ApprovalChoice>>;
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
        step: u32,
    ) -> (ApprovalTicket, Receiver<ApprovalChoice>) {
        let (tx, rx) = mpsc::channel();
        self.waits().insert((session.to_owned(), step), tx);
        (
            ApprovalTicket {
                session: session.to_owned(),
                step,
                map: self.clone(),
            },
            rx,
        )
    }

    pub fn resolve(&self, session: &str, step: u32, choice: ApprovalChoice) -> bool {
        match self.waits().remove(&(session.to_owned(), step)) {
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
        self.waits().retain(|(id, _), _| id != session);
    }
}

/// Stable fingerprint for "this exact step" when the user picks Allow for session.
fn approval_fingerprint(kind: StepKind, command: &str) -> String {
    format!("{}::{command}", step_kind_label(kind))
}

pub struct ApprovalTicket {
    session: String,
    step: u32,
    map: Approvals,
}

impl Drop for ApprovalTicket {
    fn drop(&mut self) {
        self.map.waits().remove(&(self.session.clone(), self.step));
    }
}

fn step_kind_label(kind: StepKind) -> &'static str {
    match kind {
        StepKind::Reasoning => "thinking",
        StepKind::Search => "search",
        StepKind::FileRead => "read file",
        StepKind::Command => "run command",
        StepKind::ModelCall => "call model",
        StepKind::FileChange => "edit files",
        StepKind::AgentMessage => "draft answer",
    }
}

fn step_failed(step: &Step, denied: bool) -> bool {
    denied || matches!(step, Step::Command { exit_code: Some(code), .. } if *code != 0)
}

fn to_item_kind(step: &Step, denied: bool) -> ItemKind {
    match step {
        Step::Reasoning {
            summary,
            phase,
            diagnostics,
        } => ItemKind::Reasoning {
            summary: summary.clone(),
            phase: (*phase).to_owned(),
            diagnostics: diagnostics.clone(),
        },
        Step::Search { query, detail } => ItemKind::Search {
            query: query.clone(),
            detail: detail.clone(),
        },
        Step::FileRead { path, detail } => ItemKind::FileRead {
            path: path.clone(),
            detail: detail.clone(),
        },
        Step::Command {
            command,
            cwd,
            output,
            exit_code,
        } => ItemKind::CommandExecution {
            command: command.clone(),
            cwd: cwd.clone(),
            output: output.clone(),
            exit_code: *exit_code,
            denied,
        },
        Step::ModelCall {
            model,
            input_tokens,
            output_tokens,
        } => ItemKind::ModelCall {
            model: model.clone(),
            input_tokens: *input_tokens,
            output_tokens: *output_tokens,
        },
        Step::FileChange { changes } => ItemKind::FileChange {
            changes: changes
                .iter()
                .map(
                    |FileDelta {
                         path,
                         added,
                         removed,
                     }| session::Change {
                        path: path.clone(),
                        added: *added,
                        removed: *removed,
                    },
                )
                .collect(),
        },
        Step::AgentMessage {
            text,
            checks,
            delivery,
            verification,
        } => ItemKind::AgentMessage {
            text: text.clone(),
            checks: checks.clone(),
            delivery: delivery.clone(),
            verification: verification.clone(),
        },
    }
}

pub struct StartArgs {
    pub dir: PathBuf,
    pub id: String,
    pub project: PathBuf,
    pub message: String,
    /// Project-relative context paths pinned for this turn.
    pub context: Vec<String>,
    pub provider: Option<agent::Provider>,
    pub permission: Permission,
    pub fallback_to_local: bool,
    pub max_output_tokens: u32,
    pub extended_thinking: bool,
}

pub fn start(
    app: &AppHandle,
    runs: &Runs,
    approvals: &Approvals,
    args: StartArgs,
) -> Result<(), String> {
    if !runs.begin(&args.id) {
        return Err("this session already has a run in flight".to_owned());
    }

    let app = app.clone();
    let runs = runs.clone();
    let approvals = approvals.clone();

    thread::spawn(move || {
        let notify = |event: RunEvent| {
            let _ = app.emit("run:event", event);
        };

        notify(RunEvent::TurnStarted {
            session: args.id.clone(),
        });

        let alive_id = args.id.clone();
        let alive_runs = runs.clone();
        let alive = move || alive_runs.is_live(&alive_id);

        let approve_id = args.id.clone();
        let approve_approvals = approvals.clone();
        let approve_app = app.clone();
        let approve_project = args.project.clone();
        let approve_runs = runs.clone();
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
                if !approve_runs.is_live(&approve_id) {
                    return false;
                }
                let mut seq = approve_seq.lock().unwrap_or_else(|p| p.into_inner());
                *seq += 1;
                let step = *seq;
                drop(seq);
                let (ticket, rx) = approve_approvals.wait_point(&approve_id, step);
                let risk = kodo_agent::tools::classify_command_risk(command);
                let reason = agent::tools::redact_secrets(
                    agent::dangerous_reason(command).unwrap_or_else(|| risk.reason()),
                );
                let risk_category = match risk {
                    kodo_agent::tools::CommandRisk::Catastrophic => "Catastrophic",
                    kodo_agent::tools::CommandRisk::DestructiveGit => "DestructiveGit",
                    kodo_agent::tools::CommandRisk::SensitiveData => "SensitiveData",
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
                let _ = approve_app.emit(
                    "run:event",
                    RunEvent::ApprovalRequest {
                        session: approve_id.clone(),
                        step,
                        kind: step_kind_label(kind).to_owned(),
                        detail: format!("{safe_command}  ·  {reason}"),
                        command: command.to_owned(),
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
                    if !approve_runs.is_live(&approve_id) {
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
        let record_dir = args.dir.clone();
        let record_runs = runs.clone();
        let record_app = app.clone();
        let mut seq = 0u32;
        // id of the in-flight started step, if any
        let mut open_id: Option<u32> = None;

        let mut sink = move |event: SinkEvent| -> bool {
            if !record_runs.is_live(&record_id) {
                return false;
            }
            match event {
                SinkEvent::TextDelta { text } => {
                    let _ = record_app.emit(
                        "run:event",
                        RunEvent::TextDelta {
                            session: record_id.clone(),
                            text,
                        },
                    );
                    record_runs.is_live(&record_id)
                }
                SinkEvent::Progress { phase, detail } => {
                    let _ = record_app.emit(
                        "run:event",
                        RunEvent::Progress {
                            session: record_id.clone(),
                            phase,
                            detail,
                        },
                    );
                    record_runs.is_live(&record_id)
                }
                SinkEvent::Failover {
                    from_provider,
                    from_model,
                    error_class,
                    error,
                    to_provider,
                    to_model,
                } => {
                    let _ = record_app.emit(
                        "run:event",
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
                    record_runs.is_live(&record_id)
                }
                SinkEvent::Started { step } => {
                    seq += 1;
                    open_id = Some(seq);
                    let at = session::now();
                    let running_item = Item {
                        id: seq,
                        at,
                        status: Status::Running,
                        duration_ms: None,
                        kind: to_item_kind(&step.running(), false),
                    };
                    if session::record_item(&record_dir, &record_id, &running_item, Phase::Started)
                        .is_err()
                    {
                        return false;
                    }
                    let _ = record_app.emit(
                        "run:event",
                        RunEvent::ItemStarted {
                            session: record_id.clone(),
                            item: ItemView::from(running_item),
                        },
                    );
                    record_runs.is_live(&record_id)
                }
                SinkEvent::Finished {
                    step,
                    duration_ms,
                    denied,
                } => {
                    let id = open_id.take().unwrap_or_else(|| {
                        seq += 1;
                        seq
                    });
                    let at = session::now();
                    if step_failed(&step, denied) {
                        let failed = Item {
                            id,
                            at,
                            status: Status::Failed,
                            duration_ms: Some(duration_ms),
                            kind: to_item_kind(&step, denied),
                        };
                        // The failed envelope only stamps status. Write the
                        // payload first so exit code and output survive reload.
                        if session::record_item(&record_dir, &record_id, &failed, Phase::Completed)
                            .is_err()
                        {
                            return false;
                        }
                        if session::record_item(&record_dir, &record_id, &failed, Phase::Failed)
                            .is_err()
                        {
                            return false;
                        }
                        let _ = record_app.emit(
                            "run:event",
                            RunEvent::ItemCompleted {
                                session: record_id.clone(),
                                item: ItemView::from(failed),
                            },
                        );
                        return record_runs.is_live(&record_id);
                    }

                    let finished_item = Item {
                        id,
                        at,
                        status: Status::Done,
                        duration_ms: Some(duration_ms),
                        kind: to_item_kind(&step, false),
                    };
                    if session::record_item(
                        &record_dir,
                        &record_id,
                        &finished_item,
                        Phase::Completed,
                    )
                    .is_err()
                    {
                        return false;
                    }
                    let _ = record_app.emit(
                        "run:event",
                        RunEvent::ItemCompleted {
                            session: record_id.clone(),
                            item: ItemView::from(finished_item),
                        },
                    );
                    record_runs.is_live(&record_id)
                }
            }
        };

        let request = agent::RunRequest {
            project: args.project.clone(),
            message: args.message.clone(),
            pinned_context: args.context.clone(),
            provider: args.provider.clone(),
            permission: args.permission,
            fallback_to_local: args.fallback_to_local,
            max_output_tokens: args.max_output_tokens,
            extended_thinking: args.extended_thinking,
            session_id: Some(args.id.clone()),
        };

        let result = agent::run(&request, &alive, &approve, &mut sink);
        approvals.clear_session(&args.id);

        match result {
            Ok(()) => {
                if !runs.is_live(&args.id) {
                    let _ = session::record_stopped(&args.dir, &args.id, session::now());
                    notify(RunEvent::Stopped {
                        session: args.id.clone(),
                    });
                } else if session::record_turn_complete(&args.dir, &args.id, session::now())
                    .is_err()
                {
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
                let _ = session::record_error(&args.dir, &args.id, session::now(), &safe_error);
                notify(RunEvent::Error {
                    session: args.id.clone(),
                    message: safe_error,
                });
            }
        }
        runs.cancel(&args.id);
    });

    Ok(())
}

/// Secure-by-default: missing or unknown permission means Ask.
pub fn permission_from_settings(value: Option<String>) -> Permission {
    value
        .map(|raw| Permission::parse(&raw))
        .unwrap_or(Permission::Ask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_zero_command_exit_is_a_failed_step() {
        let command = Step::Command {
            command: "check".into(),
            cwd: "/tmp".into(),
            output: "FAIL".into(),
            exit_code: Some(1),
        };
        assert!(step_failed(&command, false));
        assert!(step_failed(&command, true));
        assert!(!step_failed(
            &Step::Reasoning {
                summary: "ok".into(),
                phase: "execute",
                diagnostics: None,
            },
            false
        ));
    }
}
