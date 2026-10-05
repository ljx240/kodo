//! Session-log envelope writes for one agent turn.
//!
//! Steps are written **started** before the work runs and **completed** when
//! it finishes, so a killed run leaves a real `running` envelope in the log.
//! A failed step is written twice: payload first (so exit code and output
//! survive reload), then a status-only `Failed` stamp.

use std::path::PathBuf;

use kodo_agent::{tools, FileDelta, SinkEvent, Step};
use kodo_core::session::{self, Change, Item, ItemKind, Phase, Status};

/// What `record` did for one `SinkEvent`, so the client knows what to emit.
pub enum Logged {
    /// Not a step event (text/progress/failover) — nothing written.
    NotApplicable,
    /// `item.started` written; carries the running item for the client.
    Started(Item),
    /// `item.completed` (plus optional failed stamp) written; carries the item.
    Finished(Item),
    /// Envelope write failed — the turn must abort.
    WriteFailed,
}

/// How the turn ended, for the final envelope.
pub enum TurnOutcome {
    Complete,
    /// User cancelled (GUI stop or CLI interrupt).
    Stopped,
    /// Turn-level failure; the message is redacted before it is recorded.
    Error(String),
}

pub struct TurnLogger {
    dir: PathBuf,
    session_id: String,
    seq: u32,
    /// id of the in-flight started step, if any
    open_id: Option<u32>,
}

impl TurnLogger {
    pub fn new(dir: PathBuf, session_id: String) -> Self {
        Self {
            dir,
            session_id,
            seq: 0,
            open_id: None,
        }
    }

    /// Writes the envelope for a step event. Call **before** emitting to the
    /// client, so the log is the source of truth.
    pub fn record(&mut self, event: &SinkEvent) -> Logged {
        match event {
            SinkEvent::Started { step } => {
                self.seq += 1;
                self.open_id = Some(self.seq);
                let item = Item {
                    id: self.seq,
                    at: session::now(),
                    status: Status::Running,
                    duration_ms: None,
                    kind: to_item_kind(&step.running(), false),
                };
                match session::record_item(&self.dir, &self.session_id, &item, Phase::Started) {
                    Ok(()) => Logged::Started(item),
                    Err(_) => Logged::WriteFailed,
                }
            }
            SinkEvent::Finished {
                step,
                duration_ms,
                denied,
            } => {
                let id = self.open_id.take().unwrap_or_else(|| {
                    self.seq += 1;
                    self.seq
                });
                let at = session::now();
                if step_failed(step, *denied) {
                    let failed = Item {
                        id,
                        at,
                        status: Status::Failed,
                        duration_ms: Some(*duration_ms),
                        kind: to_item_kind(step, *denied),
                    };
                    // The failed envelope only stamps status. Write the
                    // payload first so exit code and output survive reload.
                    if session::record_item(&self.dir, &self.session_id, &failed, Phase::Completed)
                        .is_err()
                    {
                        return Logged::WriteFailed;
                    }
                    if session::record_item(&self.dir, &self.session_id, &failed, Phase::Failed)
                        .is_err()
                    {
                        return Logged::WriteFailed;
                    }
                    return Logged::Finished(failed);
                }
                let item = Item {
                    id,
                    at,
                    status: Status::Done,
                    duration_ms: Some(*duration_ms),
                    kind: to_item_kind(step, false),
                };
                match session::record_item(&self.dir, &self.session_id, &item, Phase::Completed) {
                    Ok(()) => Logged::Finished(item),
                    Err(_) => Logged::WriteFailed,
                }
            }
            _ => Logged::NotApplicable,
        }
    }

    /// Writes the turn-final envelope (`turn.complete` / `stopped` / `error`).
    /// Errors are redacted before recording.
    pub fn finish(&self, outcome: TurnOutcome) -> Result<(), String> {
        let result = match outcome {
            TurnOutcome::Complete => {
                session::record_turn_complete(&self.dir, &self.session_id, session::now())
            }
            TurnOutcome::Stopped => {
                session::record_stopped(&self.dir, &self.session_id, session::now())
            }
            TurnOutcome::Error(message) => {
                let safe = tools::redact_secrets(&message);
                session::record_error(&self.dir, &self.session_id, session::now(), &safe)
            }
        };
        result.map_err(|error| error.to_string())
    }
}

pub fn step_failed(step: &Step, denied: bool) -> bool {
    denied || matches!(step, Step::Command { exit_code: Some(code), .. } if *code != 0)
}

pub fn to_item_kind(step: &Step, denied: bool) -> ItemKind {
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
            protocol,
            finish_reason,
            saw_reasoning,
            duration_ms,
            provider_latency_ms,
            retry_index,
            llm_io_ref,
        } => ItemKind::ModelCall {
            model: model.clone(),
            input_tokens: *input_tokens,
            output_tokens: *output_tokens,
            protocol: protocol.clone(),
            finish_reason: finish_reason.clone(),
            saw_reasoning: *saw_reasoning,
            duration_ms: *duration_ms,
            provider_latency_ms: *provider_latency_ms,
            retry_index: *retry_index,
            llm_io_ref: llm_io_ref.clone(),
        },
        Step::FileChange { changes } => ItemKind::FileChange {
            changes: changes
                .iter()
                .map(
                    |FileDelta {
                         path,
                         added,
                         removed,
                     }| Change {
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
            plain,
        } => ItemKind::AgentMessage {
            text: text.clone(),
            checks: checks.clone(),
            delivery: delivery.clone(),
            verification: verification.clone(),
            plain: *plain,
        },
        Step::Thinking {
            content,
            phase,
            source,
        } => ItemKind::Thinking {
            content: content.clone(),
            phase: phase.clone(),
            source: source.clone(),
        },
        Step::Compaction {
            dropped,
            summarized_into,
            freed_pct,
            source_summary,
            covered_messages,
        } => ItemKind::Compaction {
            dropped: *dropped,
            summarized_into: *summarized_into,
            freed_pct: *freed_pct,
            source_summary: source_summary.clone(),
            covered_messages: *covered_messages,
        },
        Step::Failover {
            from_provider,
            from_model,
            error_class,
            error,
            to_provider,
            to_model,
            retry_index,
        } => ItemKind::Failover {
            from_provider: from_provider.clone(),
            from_model: from_model.clone(),
            error_class: error_class.clone(),
            error: error.clone(),
            to_provider: to_provider.clone(),
            to_model: to_model.clone(),
            retry_index: *retry_index,
        },
        Step::PlanStep {
            step_id,
            title,
            kind,
            status,
            triggered_by,
            evidence,
        } => ItemKind::PlanStep {
            step_id: step_id.clone(),
            title: title.clone(),
            kind: kind.clone(),
            status: status.clone(),
            triggered_by: triggered_by.clone(),
            evidence: evidence.clone(),
        },
        Step::Permission {
            kind,
            detail,
            decision,
            context,
        } => ItemKind::Permission {
            kind: kind.clone(),
            detail: detail.clone(),
            decision: decision.clone(),
            context: context.clone(),
        },
        Step::ContextBudget {
            used,
            window,
            percent,
            cumulative_chars,
            compacted_count,
            last_model,
        } => ItemKind::ContextBudget {
            used: *used,
            window: *window,
            percent: *percent,
            cumulative_chars: *cumulative_chars,
            compacted_count: *compacted_count,
            last_model: last_model.clone(),
        },
    }
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

    /// Format-drift guard: what TurnLogger writes must load back through
    /// `session::load` with the kinds and statuses the GUI expects.
    #[test]
    fn envelopes_round_trip_through_session_load() {
        let tmp = std::env::temp_dir().join(format!("kodo-shell-turn-{}", std::process::id()));
        let dir = tmp.join("sessions");
        std::fs::create_dir_all(&dir).expect("tempdir");
        let project = tmp.join("proj");
        std::fs::create_dir_all(&project).expect("proj");
        let id =
            session::open(&dir, &project, "t", session::now(), session::Mode::Code).expect("open");
        // Items attach to the turn an `ask` line opens — same order the shell writes.
        session::record_ask(&dir, &id, session::now(), "跑 ls").expect("ask");

        let mut logger = TurnLogger::new(dir.clone(), id.clone());

        let running = Step::Command {
            command: "ls".into(),
            cwd: project.to_string_lossy().into_owned(),
            output: String::new(),
            exit_code: None,
        };
        match logger.record(&SinkEvent::Started {
            step: running.clone(),
        }) {
            Logged::Started(item) => {
                assert_eq!(item.id, 1);
                assert!(matches!(item.status, Status::Running));
            }
            _ => panic!("expected Started"),
        }

        let finished = Step::Command {
            command: "ls".into(),
            cwd: project.to_string_lossy().into_owned(),
            output: "ok".into(),
            exit_code: Some(0),
        };
        match logger.record(&SinkEvent::Finished {
            step: finished,
            duration_ms: 5,
            denied: false,
        }) {
            Logged::Finished(item) => {
                assert_eq!(item.id, 1);
                assert!(matches!(item.status, Status::Done));
                assert_eq!(item.duration_ms, Some(5));
            }
            _ => panic!("expected Finished"),
        }

        // Denied step: double write, status Failed.
        let denied = Step::FileChange {
            changes: vec![FileDelta {
                path: "a.rs".into(),
                added: 1,
                removed: 0,
            }],
        };
        logger.record(&SinkEvent::Started {
            step: denied.clone(),
        });
        match logger.record(&SinkEvent::Finished {
            step: denied,
            duration_ms: 1,
            denied: true,
        }) {
            Logged::Finished(item) => {
                assert_eq!(item.id, 2);
                assert!(matches!(item.status, Status::Failed));
            }
            _ => panic!("expected Finished"),
        }

        logger.finish(TurnOutcome::Complete).expect("finish");

        let loaded = session::load(&dir, &id).expect("load");
        let steps: Vec<_> = loaded
            .turns
            .iter()
            .flat_map(|turn| turn.items.iter())
            .collect();
        // Phases merge per id: started+completed → one Done item, and the
        // denied step's payload+failed stamp → one Failed item.
        assert_eq!(steps.len(), 2, "expected one merged item per step id");
        let done = steps.iter().find(|item| item.id == 1).expect("item 1");
        assert!(matches!(done.status, Status::Done));
        assert_eq!(done.duration_ms, Some(5));
        let failed = steps.iter().find(|item| item.id == 2).expect("item 2");
        assert!(matches!(failed.status, Status::Failed));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn to_item_kind_maps_every_step_variant() {
        assert!(matches!(
            to_item_kind(
                &Step::Search {
                    query: "q".into(),
                    detail: "d".into()
                },
                false
            ),
            ItemKind::Search { .. }
        ));
        assert!(matches!(
            to_item_kind(
                &Step::FileChange {
                    changes: vec![FileDelta {
                        path: "p".into(),
                        added: 2,
                        removed: 1,
                    }]
                },
                true
            ),
            ItemKind::FileChange { .. }
        ));
        assert_eq!(
            step_kind_of(&to_item_kind(
                &Step::ModelCall {
                    model: "m".into(),
                    input_tokens: 1,
                    output_tokens: 2,
                    protocol: "native".into(),
                    finish_reason: "stop".into(),
                    saw_reasoning: false,
                    duration_ms: 0,
                    provider_latency_ms: 0,
                    retry_index: 0,
                    llm_io_ref: None,
                },
                false
            )),
            "model"
        );
    }

    fn step_kind_of(kind: &ItemKind) -> &'static str {
        match kind {
            ItemKind::ModelCall { .. } => "model",
            _ => "other",
        }
    }
}
