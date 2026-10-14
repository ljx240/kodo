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

/// Replay only public conversation content; never replay private reasoning or
/// file/tool traces as assistant answers. The current open turn is excluded.
pub fn conversation_history(session: &session::Session) -> Vec<(String, String)> {
    let turns: Vec<_> = session
        .turns
        .iter()
        .filter(|turn| {
            !turn.knowledge_used
                && (turn.done || turn.stopped || turn.interrupted || turn.error.is_some())
        })
        .rev()
        .take(8)
        .collect();
    let mut history = Vec::new();
    for turn in turns.into_iter().rev() {
        history.push(("user".to_owned(), turn.ask.clone()));
        if let Some(text) = turn.items.iter().rev().find_map(|item| match &item.kind {
            ItemKind::AgentMessage { text, delivery, .. }
                if item.status == Status::Done
                    && delivery != "failed"
                    && !text.trim().is_empty() =>
            {
                Some(text)
            }
            _ => None,
        }) {
            history.push(("assistant".to_owned(), text.clone()));
        }
    }
    history
}

pub struct TurnLogger {
    dir: PathBuf,
    session_id: String,
    seq: u32,
    /// id of the in-flight started step, if any
    open_id: Option<u32>,
    capture_turn: Option<u32>,
    capture_seq: u32,
    pending_trace: Option<(&'static str, serde_json::Value)>,
}

impl TurnLogger {
    pub fn new(dir: PathBuf, session_id: String) -> Self {
        Self {
            dir,
            session_id,
            seq: 0,
            open_id: None,
            capture_turn: None,
            capture_seq: 0,
            pending_trace: None,
        }
    }

    pub fn with_capture(mut self, enabled: bool) -> Self {
        if enabled {
            self.capture_turn = session::load(&self.dir, &self.session_id)
                .ok()
                .and_then(|s| s.turns.len().checked_sub(1))
                .map(|seq| seq as u32);
        }
        self
    }

    fn attach_capture(&mut self, kind: &mut ItemKind) {
        let Some(turn) = self.capture_turn else {
            return;
        };
        let Some((label, mut value)) = self.pending_trace.take() else {
            return;
        };
        self.capture_seq += 1;
        value["seq"] = self.capture_seq.into();
        redact_capture(&mut value);
        let Some(root) = self.dir.parent().map(|p| p.join("traces")) else {
            return;
        };
        let reference = match (label, kind) {
            ("llm_io", ItemKind::ModelCall { llm_io_ref, .. }) => {
                serde_json::from_value::<kodo_core::traces::LlmIoCapture>(value)
                    .ok()
                    .and_then(|c| {
                        kodo_core::traces::write_llm_io_in(&root, &self.session_id, turn, &c).ok()
                    })
                    .map(|r| {
                        *llm_io_ref = Some(r);
                    })
            }
            ("spans", ItemKind::FileChange { file_span_ref, .. }) => serde_json::from_value::<
                kodo_core::traces::FileSpanCapture,
            >(value)
            .ok()
            .and_then(|c| {
                kodo_core::traces::write_file_span_in(&root, &self.session_id, turn, &c).ok()
            })
            .map(|r| {
                *file_span_ref = Some(r);
            }),
            _ => None,
        };
        if reference.is_none() {
            eprintln!("Kodo: local detailed trace unavailable; the session index remains valid");
        }
    }

    /// Writes the envelope for a step event. Call **before** emitting to the
    /// client, so the log is the source of truth.
    pub fn record(&mut self, event: &SinkEvent) -> Logged {
        match event {
            SinkEvent::LocalTrace { kind, capture } => {
                if self.capture_turn.is_some() {
                    self.pending_trace = Some((kind, capture.clone()));
                }
                Logged::NotApplicable
            }
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
                    let mut failed = Item {
                        id,
                        at,
                        status: Status::Failed,
                        duration_ms: Some(*duration_ms),
                        kind: to_item_kind(step, *denied),
                    };
                    self.attach_capture(&mut failed.kind);
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
                let mut item = Item {
                    id,
                    at,
                    status: Status::Done,
                    duration_ms: Some(*duration_ms),
                    kind: to_item_kind(step, false),
                };
                self.attach_capture(&mut item.kind);
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

fn redact_capture(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => *text = tools::redact_secrets(text),
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_capture),
        serde_json::Value::Object(map) => {
            for (key, item) in map {
                if item.is_string()
                    && [
                        "api_key",
                        "apikey",
                        "password",
                        "authorization",
                        "token",
                        "secret",
                        "access_token",
                    ]
                    .contains(&key.to_ascii_lowercase().as_str())
                {
                    *item = "[REDACTED]".into();
                } else {
                    redact_capture(item);
                }
            }
        }
        _ => {}
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
        Step::FileChange { changes, file_span_ref, .. } => ItemKind::FileChange {
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
            file_span_ref: file_span_ref.clone(),
        },
        Step::AgentMessage {
            text,
            checks,
            delivery,
            verification,
            plain,
            failure_reason,
        } => ItemKind::AgentMessage {
            text: text.clone(),
            checks: checks.clone(),
            delivery: delivery.clone(),
            verification: verification.clone(),
            plain: *plain,
            failure_reason: failure_reason.clone(),
        },
        // Transparency-side steps never carry user-facing content; render them
        // as `Reasoning` chips with a stable phase so the trace UI can still
        // surface them and the session log stays closed.
        Step::Thinking {
            content,
            phase,
            source,
        } => ItemKind::Reasoning {
            summary: format!("thinking ({source})"),
            phase: phase.clone(),
            diagnostics: Some(content.clone()),
        },
        Step::Compaction {
            dropped,
            summarized_into,
            freed_pct,
            source_summary,
            covered_messages,
        } => ItemKind::Reasoning {
            summary: format!(
                "compacted {covered_messages} messages into {summarized_into} ({dropped} dropped, {freed_pct}% freed)"
            ),
            phase: "compact".into(),
            diagnostics: Some(source_summary.clone()),
        },
        Step::Failover {
            from_provider,
            from_model,
            error_class,
            error,
            to_provider,
            to_model,
            retry_index,
        } => ItemKind::Reasoning {
            summary: format!(
                "failover retry #{retry_index}: {from_provider}/{from_model} -> {to_provider}/{to_model} ({error_class})"
            ),
            phase: "failover".into(),
            diagnostics: Some(error.clone()),
        },
        Step::PlanStep {
            step_id,
            title,
            kind,
            status,
            triggered_by,
            evidence,
        } => ItemKind::Reasoning {
            summary: format!("plan step {step_id} ({kind}) {status}: {title}"),
            phase: "plan".into(),
            diagnostics: Some(format!(
                "triggered_by={:?} evidence={:?}",
                triggered_by, evidence
            )),
        },
        Step::Permission {
            kind,
            detail,
            decision,
            context,
        } => ItemKind::Reasoning {
            summary: format!("permission {kind}: {decision}"),
            phase: "permission".into(),
            diagnostics: Some(format!("{detail} | {context}")),
        },
        Step::ContextBudget {
            used,
            window,
            percent,
            cumulative_chars,
            compacted_count,
            last_model,
        } => ItemKind::Reasoning {
            summary: format!(
                "context {percent}% ({used}/{window}) after {cumulative_chars} chars, {compacted_count} compactions, model={last_model}"
            ),
            phase: "budget".into(),
            diagnostics: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_answer_turn(
        dir: &std::path::Path,
        id: &str,
        at: u64,
        ask: &str,
        answer: &str,
        knowledge_used: bool,
    ) {
        let turn_seq = session::load(dir, id).unwrap().turns.len() as u64 + 1;
        session::record_ask(dir, id, at, ask).unwrap();
        if knowledge_used {
            session::record_knowledge_turn(dir, id, turn_seq).unwrap();
        }
        let step = Step::AgentMessage {
            text: answer.to_owned(),
            checks: Vec::new(),
            delivery: "ready".to_owned(),
            verification: "not_run".to_owned(),
            plain: true,
            failure_reason: String::new(),
        };
        let mut logger = TurnLogger::new(dir.to_path_buf(), id.to_owned());
        assert!(matches!(
            logger.record(&SinkEvent::Started { step: step.clone() }),
            Logged::Started(_)
        ));
        assert!(matches!(
            logger.record(&SinkEvent::Finished {
                step,
                duration_ms: 1,
                denied: false,
            }),
            Logged::Finished(_)
        ));
        logger.finish(TurnOutcome::Complete).unwrap();
    }

    #[test]
    fn knowledge_turns_are_not_replayed_but_ordinary_turns_are() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        let id = session::open(
            dir,
            std::path::Path::new("/tmp/fixture"),
            "history",
            1,
            session::Mode::Code,
        )
        .unwrap();

        record_answer_turn(dir, &id, 2, "ordinary question", "ordinary answer", false);
        // Match the desktop's ask → provenance marker → model dispatch order.
        record_answer_turn(dir, &id, 3, "knowledge question", "knowledge answer", true);
        record_answer_turn(
            dir,
            &id,
            4,
            "later ordinary question",
            "later ordinary answer",
            false,
        );

        let saved = session::load(dir, &id).unwrap();
        assert_eq!(
            saved
                .turns
                .iter()
                .map(|turn| turn.knowledge_used)
                .collect::<Vec<_>>(),
            vec![false, true, false]
        );
        assert_eq!(
            conversation_history(&saved),
            vec![
                ("user".to_owned(), "ordinary question".to_owned()),
                ("assistant".to_owned(), "ordinary answer".to_owned()),
                ("user".to_owned(), "later ordinary question".to_owned()),
                ("assistant".to_owned(), "later ordinary answer".to_owned()),
            ]
        );
    }

    #[test]
    fn history_replays_public_answers_without_private_traces_or_open_turns() {
        let dir = std::env::temp_dir().join(format!("kodo-public-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = session::open(
            &dir,
            std::path::Path::new("/tmp/fixture"),
            "history",
            1,
            session::Mode::Code,
        )
        .unwrap();
        session::record_ask(&dir, &id, 2, "分析项目架构").unwrap();
        let mut logger = TurnLogger::new(dir.clone(), id.clone());
        for step in [
            Step::Reasoning {
                summary: "PUBLIC_PROGRESS".to_owned(),
                phase: "commentary",
                diagnostics: None,
            },
            Step::Thinking {
                content: "PRIVATE_REASONING".to_owned(),
                phase: "analyze".to_owned(),
                source: "native".to_owned(),
            },
            Step::Command {
                command: "fixture".to_owned(),
                cwd: "/tmp/fixture".to_owned(),
                output: "RAW_TOOL_OUTPUT".to_owned(),
                exit_code: Some(0),
            },
            Step::AgentMessage {
                text: "# 架构\n公开回答。".to_owned(),
                checks: vec![],
                delivery: "ready".to_owned(),
                verification: "not_run".to_owned(),
                plain: true,
                failure_reason: String::new(),
            },
        ] {
            logger.record(&SinkEvent::Started { step: step.clone() });
            logger.record(&SinkEvent::Finished {
                step,
                duration_ms: 1,
                denied: false,
            });
        }
        logger.finish(TurnOutcome::Complete).unwrap();
        session::record_ask(&dir, &id, 3, "保存文档").unwrap();
        let mut logger = TurnLogger::new(dir.clone(), id.clone());
        let failure = Step::AgentMessage {
            text: "FAILURE_LOG".to_owned(),
            checks: vec![],
            delivery: "failed".to_owned(),
            verification: "not_run".to_owned(),
            plain: true,
            failure_reason: String::new(),
        };
        logger.record(&SinkEvent::Started {
            step: failure.clone(),
        });
        logger.record(&SinkEvent::Finished {
            step: failure,
            duration_ms: 1,
            denied: false,
        });
        logger.finish(TurnOutcome::Complete).unwrap();
        session::record_ask(&dir, &id, 4, "CURRENT_OPEN_REQUEST").unwrap();
        let saved = session::load(&dir, &id).unwrap();
        assert!(saved.turns[0].items.iter().any(|item| matches!(
            &item.kind, ItemKind::Reasoning { summary, phase, diagnostics: None }
                if phase == "commentary" && summary == "PUBLIC_PROGRESS"
        )));
        let history = conversation_history(&saved);
        assert_eq!(
            history,
            vec![
                ("user".to_owned(), "分析项目架构".to_owned()),
                ("assistant".to_owned(), "# 架构\n公开回答。".to_owned()),
                ("user".to_owned(), "保存文档".to_owned()),
            ]
        );
    }

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
            file_span_ref: None,
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
                    }],
                    file_span_ref: None,
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
                    protocol: String::new(),
                    finish_reason: String::new(),
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
    #[test]
    fn detailed_capture_is_opt_in_redacted_and_refs_point_to_readable_private_files() {
        let root = std::env::temp_dir().join(format!("kodo-capture-sink-{}", std::process::id()));
        let dir = root.join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let id = session::open(
            &dir,
            std::path::Path::new("/tmp/fixture"),
            "trace",
            1,
            session::Mode::Code,
        )
        .unwrap();
        session::record_ask(&dir, &id, 2, "task").unwrap();
        let body = serde_json::json!({"seq":0,"model":"fixture","protocol":"native","retry_index":0,"provider_latency_ms":1,"finish_reason":"stop","input_tokens":1,"output_tokens":1,"saw_reasoning":false,"at":2,"request":{"system":"synthetic","messages":[{"password":"private-fixture"}]},"response":{"text":"done","reasoning_content":"","native_tool_calls":[]}});
        let event = SinkEvent::LocalTrace {
            kind: "llm_io",
            capture: body.clone(),
        };
        let mut off = TurnLogger::new(dir.clone(), id.clone());
        off.record(&event);
        assert!(!root.join("traces").exists());
        let mut on = TurnLogger::new(dir.clone(), id.clone()).with_capture(true);
        on.record(&event);
        let step = Step::ModelCall {
            model: "fixture".into(),
            input_tokens: 1,
            output_tokens: 1,
            protocol: "native".into(),
            finish_reason: "stop".into(),
            saw_reasoning: false,
            duration_ms: 1,
            provider_latency_ms: 1,
            retry_index: 0,
            llm_io_ref: None,
        };
        on.record(&SinkEvent::Started { step: step.clone() });
        let Logged::Finished(item) = on.record(&SinkEvent::Finished {
            step,
            duration_ms: 1,
            denied: false,
        }) else {
            panic!("finished");
        };
        let ItemKind::ModelCall {
            llm_io_ref: Some(reference),
            ..
        } = item.kind
        else {
            panic!("reference");
        };
        let capture =
            kodo_core::traces::read_llm_io_in(&root.join("traces"), &id, 0, &reference).unwrap();
        assert_eq!(capture.request.messages[0]["password"], "[REDACTED]");
        assert_eq!(capture.response.text, "done");
        let mut value = serde_json::json!({"authorization":"token-value","input_tokens":5,"nested":{"secret":"another"}});
        redact_capture(&mut value);
        assert_eq!(value["authorization"], "[REDACTED]");
        assert_eq!(value["input_tokens"], 5);
        assert_eq!(value["nested"]["secret"], "[REDACTED]");
        std::fs::remove_dir_all(root).unwrap();
    }
}
