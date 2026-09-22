//! Serializable mirrors of the core's types.
//!
//! The core carries no serde, so the shell owns the transport shape. Tauri
//! converts the *arguments* of a command to camelCase but passes its *return
//! value* straight through, so nothing here is renamed behind our back — but the
//! names still have to be what the TypeScript side expects, which is why the
//! multi-word ones say so explicitly.

use kodo_core::session;
use kodo_core::workspace;

/// A project on the sidebar's list.
#[derive(serde::Serialize)]
pub struct ProjectView {
    pub path: String,
    pub name: String,
}

impl From<workspace::Project> for ProjectView {
    fn from(project: workspace::Project) -> Self {
        ProjectView {
            path: project.path.to_string_lossy().into_owned(),
            name: project.name,
        }
    }
}

/// A session as the sidebar needs it, without its events.
#[derive(serde::Serialize)]
pub struct SessionRefView {
    pub id: String,
    pub project: String,
    pub title: String,
    pub at: u64,
    pub archived: bool,
}

impl From<session::SessionRef> for SessionRefView {
    fn from(reference: session::SessionRef) -> Self {
        SessionRefView {
            id: reference.id,
            project: reference.project.to_string_lossy().into_owned(),
            title: reference.title,
            at: reference.at,
            archived: reference.archived,
        }
    }
}

/// Everything the GUI needs to draw the sidebar: the project list and the
/// session list in one round trip, so the two can never be shown out of step.
#[derive(serde::Serialize)]
pub struct Workspace {
    pub projects: Vec<ProjectView>,
    pub sessions: Vec<SessionRefView>,
}

/// A whole conversation.
#[derive(serde::Serialize)]
pub struct SessionView {
    pub id: String,
    pub project: String,
    pub title: String,
    pub at: u64,
    pub archived: bool,
    pub turns: Vec<TurnView>,
}

#[derive(serde::Serialize)]
pub struct TurnView {
    pub ask: String,
    /// Project-relative paths attached as context for this turn.
    pub context: Vec<String>,
    pub items: Vec<ItemView>,
    pub done: bool,
    pub stopped: bool,
    /// Set when recovery reclassified a killed run's open items as interrupted.
    pub interrupted: bool,
    pub error: Option<String>,
    /// Three orthogonal status axes, computed here so the GUI never guesses.
    pub status: TurnStatusView,
}

/// The run state model: lifecycle × delivery × verification, plus one
/// high-level outcome token for the headline copy.
#[derive(serde::Serialize, Clone)]
pub struct TurnStatusView {
    /// queued | working | awaiting_approval | completed | stopped |
    /// interrupted | failed (persisted turns never report the live values).
    pub lifecycle: &'static str,
    /// ready | partial | blocked | failed — did the answer land?
    pub delivery: &'static str,
    /// not_run | running | passed | failed | blocked — did acceptance land?
    pub verification: &'static str,
    /// working | partially_completed | blocked_by_environment | completed |
    /// failed | stopped | interrupted
    pub outcome: &'static str,
}

/// Map a final verification status code pair to the headline outcome.
fn outcome_for(
    lifecycle: &'static str,
    delivery: &'static str,
    verification: &'static str,
    wrote_files: bool,
) -> &'static str {
    match lifecycle {
        "stopped" => "stopped",
        "interrupted" => "interrupted",
        "failed" => "failed",
        _ => match verification {
            "blocked" => "blocked_by_environment",
            _ => match delivery {
                "failed" => "failed",
                "ready" if verification == "passed" => "completed",
                // Pure Q&A that never needed acceptance checks reads complete.
                "ready" if verification == "not_run" && !wrote_files => "completed",
                _ => "partially_completed",
            },
        },
    }
}

/// Derive the three axes for a persisted turn. Delivery/verification come from
/// the answer's structured outcome when present; otherwise they are derived
/// from structured item facts (denial flags, exit codes, failure classes) —
/// never from display strings.
fn turn_status(turn: &session::Turn) -> TurnStatusView {
    let has_running = turn
        .items
        .iter()
        .any(|item| item.status == session::Status::Running);
    let lifecycle = if turn.error.is_some() {
        "failed"
    } else if turn.stopped {
        "stopped"
    } else if turn.interrupted || has_running || (!turn.done && !turn.stopped) {
        "interrupted"
    } else {
        "completed"
    };

    let wrote_files = turn
        .items
        .iter()
        .any(|item| matches!(item.kind, session::ItemKind::FileChange { .. }));

    let mut delivery = "";
    let mut verification = "";
    for item in &turn.items {
        if let session::ItemKind::AgentMessage {
            delivery: d,
            verification: v,
            ..
        } = &item.kind
        {
            if !d.is_empty() {
                delivery = d.as_str();
            }
            if !v.is_empty() {
                verification = v.as_str();
            }
        }
    }

    // Fallbacks for legacy turns whose answer predates the outcome fields.
    let mut failed_commands = 0usize;
    let mut blocked_commands = 0usize;
    let mut ok_commands = 0usize;
    for item in &turn.items {
        if let session::ItemKind::CommandExecution {
            output,
            exit_code,
            denied,
            ..
        } = &item.kind
        {
            let failed = *denied
                || matches!(exit_code, Some(code) if *code != 0)
                || (item.status == session::Status::Failed && exit_code.is_none());
            if !failed {
                if item.status == session::Status::Done {
                    ok_commands += 1;
                }
                continue;
            }
            match kodo_agent::TurnFailureKind::classify(*exit_code, output, *denied) {
                Some(kodo_agent::TurnFailureKind::CommandNotFound)
                | Some(kodo_agent::TurnFailureKind::PermissionDenied)
                | Some(kodo_agent::TurnFailureKind::Timeout) => blocked_commands += 1,
                _ => failed_commands += 1,
            }
        }
    }

    let delivery = if !delivery.is_empty() {
        match delivery {
            "partial" => "partial",
            "blocked" => "blocked",
            "failed" => "failed",
            _ => "ready",
        }
    } else if turn.error.is_some() {
        "failed"
    } else if turn
        .items
        .iter()
        .any(|item| matches!(item.kind, session::ItemKind::AgentMessage { .. }))
    {
        "ready"
    } else if wrote_files {
        "partial"
    } else {
        "failed"
    };
    let verification = if !verification.is_empty() {
        match verification {
            "running" => "running",
            "passed" => "passed",
            "failed" => "failed",
            "blocked" => "blocked",
            _ => "not_run",
        }
    } else if blocked_commands > 0 {
        "blocked"
    } else if failed_commands > 0 {
        "failed"
    } else if ok_commands > 0 {
        "passed"
    } else {
        "not_run"
    };

    TurnStatusView {
        lifecycle,
        delivery,
        verification,
        outcome: outcome_for(lifecycle, delivery, verification, wrote_files),
    }
}

/// One step, flattened so the GUI can switch on `kind` and read the rest
/// directly. This is a discriminated union rather than a generic
/// `tool { name, state }` because each kind renders differently.
#[derive(serde::Serialize, Clone)]
pub struct ItemView {
    pub id: u32,
    pub at: u64,
    pub status: &'static str,
    pub duration: Option<u64>,
    #[serde(flatten)]
    pub detail: ItemDetail,
}

#[derive(serde::Serialize, Clone)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ItemDetail {
    Reasoning {
        summary: String,
        /// Public phase code. Empty for legacy rows.
        phase: String,
        /// Internal scheduling diagnostics (Debug surfaces only).
        diagnostics: Option<String>,
    },
    Search {
        query: String,
        detail: String,
    },
    FileRead {
        path: String,
        detail: String,
    },
    CommandExecution {
        command: String,
        cwd: String,
        output: String,
        #[serde(rename = "exitCode")]
        exit_code: Option<i32>,
        denied: bool,
        /// Structured failure taxonomy code — classified here, not in the UI.
        #[serde(rename = "failureClass")]
        failure_class: Option<&'static str>,
        /// Missing binary for `command_not_found`, when one was named.
        #[serde(rename = "failureTool")]
        failure_tool: Option<String>,
    },
    ModelCall {
        model: String,
        #[serde(rename = "inputTokens")]
        input_tokens: u32,
        #[serde(rename = "outputTokens")]
        output_tokens: u32,
    },
    FileChange {
        changes: Vec<ChangeView>,
    },
    AgentMessage {
        text: String,
        checks: Vec<String>,
        /// ready | partial | blocked | failed
        delivery: String,
        /// not_run | running | passed | failed | blocked
        verification: String,
    },
}

#[derive(serde::Serialize, Clone)]
pub struct ChangeView {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// One Kodo-touched file with its unified diff for the inspector.
#[derive(serde::Serialize, Clone)]
pub struct TurnChangeView {
    pub path: String,
    pub diff: String,
    #[serde(rename = "userPreexisting")]
    pub user_preexisting: bool,
    /// Live undo state: "clean" | "already_baseline" | "diverged" | "missing".
    pub undo_state: String,
    /// True when the recorded after-hash no longer matches the working tree.
    pub conflict: bool,
}

/// Safe undo outcome: restored paths + per-file conflicts (state C).
#[derive(serde::Serialize, Clone)]
pub struct UndoReportView {
    pub restored: Vec<String>,
    pub conflicts: Vec<UndoConflictView>,
}

#[derive(serde::Serialize, Clone)]
pub struct UndoConflictView {
    pub path: String,
    pub reason: String,
    pub message: String,
}

impl From<session::Item> for ItemView {
    fn from(item: session::Item) -> Self {
        let detail = match item.kind {
            session::ItemKind::Reasoning {
                summary,
                phase,
                diagnostics,
            } => ItemDetail::Reasoning {
                summary,
                phase,
                diagnostics,
            },
            session::ItemKind::Search { query, detail } => ItemDetail::Search { query, detail },
            session::ItemKind::FileRead { path, detail } => ItemDetail::FileRead { path, detail },
            session::ItemKind::CommandExecution {
                command,
                cwd,
                output,
                exit_code,
                denied,
            } => {
                let failed = denied || exit_code.map(|code| code != 0).unwrap_or(false);
                let kind = if failed {
                    kodo_agent::TurnFailureKind::classify(exit_code, &output, denied)
                } else {
                    None
                };
                let failure_tool = kind
                    .filter(|k| *k == kodo_agent::TurnFailureKind::CommandNotFound)
                    .and_then(|_| kodo_agent::TurnFailureKind::missing_tool(&output));
                ItemDetail::CommandExecution {
                    command,
                    cwd,
                    output,
                    exit_code,
                    denied,
                    failure_class: kind.map(|k| k.code()),
                    failure_tool,
                }
            }
            session::ItemKind::ModelCall {
                model,
                input_tokens,
                output_tokens,
            } => ItemDetail::ModelCall {
                model,
                input_tokens,
                output_tokens,
            },
            session::ItemKind::FileChange { changes } => ItemDetail::FileChange {
                changes: changes
                    .into_iter()
                    .map(|change| ChangeView {
                        path: change.path,
                        added: change.added,
                        removed: change.removed,
                    })
                    .collect(),
            },
            session::ItemKind::AgentMessage {
                text,
                checks,
                delivery,
                verification,
            } => ItemDetail::AgentMessage {
                text,
                checks,
                delivery,
                verification,
            },
        };

        ItemView {
            id: item.id,
            at: item.at,
            status: match item.status {
                session::Status::Running => "running",
                session::Status::Done => "done",
                session::Status::Failed => "failed",
            },
            duration: item.duration_ms,
            detail,
        }
    }
}

impl From<session::Session> for SessionView {
    fn from(session: session::Session) -> Self {
        SessionView {
            id: session.id,
            project: session.project.to_string_lossy().into_owned(),
            title: session.title,
            at: session.at,
            archived: session.archived,
            turns: session
                .turns
                .into_iter()
                .map(|turn| {
                    let status = turn_status(&turn);
                    TurnView {
                        ask: turn.ask,
                        context: turn.context,
                        items: turn.items.into_iter().map(Into::into).collect(),
                        done: turn.done,
                        stopped: turn.stopped,
                        interrupted: turn.interrupted,
                        error: turn.error,
                        status,
                    }
                })
                .collect(),
        }
    }
}

/// What the runner pushes at the GUI while it works.
///
/// The lifecycle is the envelope, not a field on the item: a run that is killed
/// leaves an `itemStarted` with no `itemCompleted`, and that gap is what "still
/// running" means. Nothing has to be written correctly at the end of a process
/// that died for the GUI to be right.
///
/// Every variant names its session. The GUI has one channel for all of them, so
/// an event that did not say where it came from would be applied to whichever
/// conversation happened to be open.
#[derive(serde::Serialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RunEvent {
    TurnStarted {
        session: String,
    },
    ItemStarted {
        session: String,
        item: ItemView,
    },
    ItemCompleted {
        session: String,
        item: ItemView,
    },
    TurnComplete {
        session: String,
    },
    Stopped {
        session: String,
    },
    Error {
        session: String,
        message: String,
    },
    /// The runner is waiting for the user to approve or deny a step.
    ApprovalRequest {
        session: String,
        step: u32,
        kind: String,
        /// Command or short detail when the step has one.
        detail: String,
        /// Exact command/label used for the session allowlist fingerprint.
        #[serde(default)]
        command: String,
        /// Working directory the command would run in.
        #[serde(default)]
        cwd: String,
        /// Structured risk: Safe | FilesystemWrite | Network | PackageInstall |
        /// DestructiveGit | ProcessControl | SensitiveData | Dangerous.
        #[serde(default)]
        #[serde(rename = "riskCategory")]
        risk_category: String,
        /// Human-readable reason for the risk.
        #[serde(default)]
        reason: String,
    },
    /// Incremental assistant text while the model streams (not persisted).
    TextDelta {
        session: String,
        text: String,
    },
    /// Structured agent progress phase (never chain-of-thought).
    Progress {
        session: String,
        phase: String,
        detail: String,
    },
    /// Provider switch on the streaming path: failed side, error class, next side.
    Failover {
        session: String,
        #[serde(rename = "fromProvider")]
        from_provider: String,
        #[serde(rename = "fromModel")]
        from_model: String,
        #[serde(rename = "errorClass")]
        error_class: String,
        error: String,
        #[serde(rename = "toProvider")]
        to_provider: String,
        #[serde(rename = "toModel")]
        to_model: String,
    },
}

/// One archived conversation, as the Archive table needs it.
#[derive(serde::Serialize)]
pub struct ArchivedItemView {
    pub id: String,
    pub project: String,
    #[serde(rename = "projectName")]
    pub project_name: String,
    pub title: String,
    pub at: u64,
    pub model: String,
    /// Last agent message or ask — never a project name stand-in.
    pub summary: String,
    #[serde(rename = "filesChanged")]
    pub files_changed: u32,
    pub added: u32,
    pub removed: u32,
}

/// A provider as the settings screen edits it. `api_key` is a mask when
/// loaded; a new plaintext key is only sent on save.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub template: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
    pub endpoint: String,
    pub model: String,
    #[serde(default, rename = "modelId", skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(
        default,
        rename = "displayName",
        skip_serializing_if = "Option::is_none"
    )]
    pub display_name: Option<String>,
    /// True when a secret exists in credentials.log.
    #[serde(default)]
    #[serde(rename = "hasKey")]
    pub has_key: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use kodo_core::session::{Item, ItemKind, Status};
    use serde_json::json;

    fn item(kind: ItemKind) -> Item {
        Item {
            id: 3,
            at: 1_700_000_000,
            status: Status::Done,
            duration_ms: Some(1_234),
            kind,
        }
    }

    fn to_json(item: Item) -> serde_json::Value {
        serde_json::to_value(ItemView::from(item)).expect("an item should serialize")
    }

    #[test]
    fn an_item_is_one_flat_tagged_object() {
        let value = to_json(item(ItemKind::CommandExecution {
            command: "cargo check".to_owned(),
            cwd: "/p".to_owned(),
            output: "ok".to_owned(),
            exit_code: Some(0),
            denied: false,
        }));

        // The envelope and the payload sit side by side, so the GUI switches on
        // `kind` and reads `command` directly instead of unwrapping a level.
        assert_eq!(value["id"], json!(3));
        assert_eq!(value["status"], json!("done"));
        assert_eq!(value["duration"], json!(1234));
        assert_eq!(value["kind"], json!("commandExecution"));
        assert_eq!(value["command"], json!("cargo check"));
        assert_eq!(value["exitCode"], json!(0));
        assert!(
            value.get("detail").is_none(),
            "the payload was nested: {value}"
        );
    }

    #[test]
    fn a_running_item_has_no_duration() {
        let mut running = item(ItemKind::reasoning("先看目录".to_owned()));
        running.status = Status::Running;
        running.duration_ms = None;
        let value = to_json(running);

        assert_eq!(value["kind"], json!("reasoning"));
        assert_eq!(value["status"], json!("running"));
        assert!(value["duration"].is_null(), "{value}");
        assert_eq!(value["summary"], json!("先看目录"));
    }

    #[test]
    fn every_kind_uses_the_tag_the_gui_switches_on() {
        let cases = vec![
            (ItemKind::reasoning(String::new()), "reasoning"),
            (
                ItemKind::Search {
                    query: String::new(),
                    detail: String::new(),
                },
                "search",
            ),
            (
                ItemKind::FileRead {
                    path: String::new(),
                    detail: String::new(),
                },
                "fileRead",
            ),
            (
                ItemKind::CommandExecution {
                    command: String::new(),
                    cwd: String::new(),
                    output: String::new(),
                    exit_code: None,
                    denied: false,
                },
                "commandExecution",
            ),
            (
                ItemKind::ModelCall {
                    model: String::new(),
                    input_tokens: 0,
                    output_tokens: 0,
                },
                "modelCall",
            ),
            (
                ItemKind::FileChange {
                    changes: Vec::new(),
                },
                "fileChange",
            ),
            (
                ItemKind::agent_message(String::new(), Vec::new()),
                "agentMessage",
            ),
        ];

        for (kind, expected) in cases {
            assert_eq!(to_json(item(kind))["kind"], json!(expected));
        }
    }

    #[test]
    fn a_run_event_carries_its_lifecycle_tag() {
        let event = RunEvent::ItemStarted {
            session: "abc".to_owned(),
            item: ItemView::from(item(ItemKind::reasoning("s".to_owned()))),
        };
        let value = serde_json::to_value(event).expect("an event should serialize");

        assert_eq!(value["type"], json!("itemStarted"));
        assert_eq!(value["session"], json!("abc"));
        assert_eq!(value["item"]["kind"], json!("reasoning"));
    }

    #[test]
    fn failover_event_carries_both_sides_and_class() {
        let event = super::RunEvent::Failover {
            session: "s1".into(),
            from_provider: "GPT-4o".into(),
            from_model: "gpt-4o".into(),
            error_class: "RateLimit".into(),
            error: "429".into(),
            to_provider: "Claude".into(),
            to_model: "claude-sonnet".into(),
        };
        let value = serde_json::to_value(event).expect("serialize");
        assert_eq!(value["type"], json!("failover"));
        assert_eq!(value["fromProvider"], json!("GPT-4o"));
        assert_eq!(value["fromModel"], json!("gpt-4o"));
        assert_eq!(value["errorClass"], json!("RateLimit"));
        assert_eq!(value["toProvider"], json!("Claude"));
        assert_eq!(value["toModel"], json!("claude-sonnet"));
    }

    #[test]
    fn a_session_view_carries_its_turns() {
        let session = kodo_core::session::Session {
            id: "abc".to_owned(),
            project: std::path::PathBuf::from("/p"),
            title: "t".to_owned(),
            at: 7,
            archived: false,
            turns: vec![kodo_core::session::Turn {
                ask: "帮我看一下".to_owned(),
                context: vec!["src/lib.rs".to_owned()],
                items: vec![item(ItemKind::reasoning("s".to_owned()))],
                done: true,
                stopped: false,
                interrupted: false,
                error: None,
            }],
        };
        let value = serde_json::to_value(SessionView::from(session)).expect("serialize");

        assert_eq!(value["id"], json!("abc"));
        assert_eq!(value["project"], json!("/p"));
        assert_eq!(value["turns"][0]["ask"], json!("帮我看一下"));
        assert_eq!(value["turns"][0]["context"], json!(["src/lib.rs"]));
        assert_eq!(value["turns"][0]["done"], json!(true));
        assert_eq!(value["turns"][0]["items"][0]["kind"], json!("reasoning"));
    }

    #[test]
    fn a_failed_command_carries_a_structured_failure_class() {
        let value = to_json(item(ItemKind::CommandExecution {
            command: "cargo build".to_owned(),
            cwd: "/p".to_owned(),
            output: "/bin/sh: cargo: command not found".to_owned(),
            exit_code: Some(127),
            denied: false,
        }));
        assert_eq!(value["failureClass"], json!("command_not_found"));
        assert_eq!(value["failureTool"], json!("cargo"));
        assert_eq!(value["denied"], json!(false));

        let denied = to_json(item(ItemKind::CommandExecution {
            command: "rm -rf /".to_owned(),
            cwd: "/p".to_owned(),
            output: String::new(),
            exit_code: None,
            denied: true,
        }));
        assert_eq!(denied["failureClass"], json!("denied"));
        assert_eq!(denied["denied"], json!(true));
    }

    #[test]
    fn reasoning_carries_phase_and_hidden_diagnostics() {
        let value = to_json(item(ItemKind::Reasoning {
            summary: "Planning the work".to_owned(),
            phase: "analyze".to_owned(),
            diagnostics: Some("rounds 1/6 · tools 2/32".to_owned()),
        }));
        assert_eq!(value["summary"], json!("Planning the work"));
        assert_eq!(value["phase"], json!("analyze"));
        assert_eq!(value["diagnostics"], json!("rounds 1/6 · tools 2/32"));
    }

    fn turn_with(items: Vec<Item>, done: bool) -> kodo_core::session::Turn {
        kodo_core::session::Turn {
            ask: "q".to_owned(),
            context: Vec::new(),
            items,
            done,
            stopped: false,
            interrupted: false,
            error: None,
        }
    }

    #[test]
    fn an_answer_with_passed_verification_is_completed() {
        let turn = turn_with(
            vec![
                item(ItemKind::agent_message("done".to_owned(), Vec::new())),
                item(ItemKind::AgentMessage {
                    text: "done".to_owned(),
                    checks: Vec::new(),
                    delivery: "ready".to_owned(),
                    verification: "passed".to_owned(),
                }),
            ],
            true,
        );
        // First message is the legacy-shaped one; last answer wins.
        let status = turn_status(&turn);
        assert_eq!(status.lifecycle, "completed");
        assert_eq!(status.delivery, "ready");
        assert_eq!(status.verification, "passed");
        assert_eq!(status.outcome, "completed");
    }

    #[test]
    fn an_answer_with_failed_verification_is_partially_completed() {
        let turn = turn_with(
            vec![item(ItemKind::AgentMessage {
                text: "done".to_owned(),
                checks: Vec::new(),
                delivery: "ready".to_owned(),
                verification: "failed".to_owned(),
            })],
            true,
        );
        let status = turn_status(&turn);
        assert_eq!(status.outcome, "partially_completed");
    }

    #[test]
    fn environment_blockade_is_never_plain_failure() {
        let turn = turn_with(
            vec![item(ItemKind::AgentMessage {
                text: "无法验证".to_owned(),
                checks: Vec::new(),
                delivery: "blocked".to_owned(),
                verification: "blocked".to_owned(),
            })],
            true,
        );
        let status = turn_status(&turn);
        assert_eq!(status.outcome, "blocked_by_environment");
    }

    #[test]
    fn a_stopped_turn_reports_stopped_not_failed() {
        let mut turn = turn_with(vec![item(ItemKind::reasoning("s"))], false);
        turn.stopped = true;
        let status = turn_status(&turn);
        assert_eq!(status.lifecycle, "stopped");
        assert_eq!(status.outcome, "stopped");
    }

    #[test]
    fn a_qanda_without_checks_still_reads_completed() {
        let turn = turn_with(
            vec![item(ItemKind::agent_message("答案", Vec::new()))],
            true,
        );
        let status = turn_status(&turn);
        assert_eq!(status.verification, "not_run");
        assert_eq!(status.outcome, "completed");
    }

    #[test]
    fn missing_tool_failures_become_blocked_verification() {
        let turn = turn_with(
            vec![
                item(ItemKind::CommandExecution {
                    command: "cargo build".to_owned(),
                    cwd: "/p".to_owned(),
                    output: "/bin/sh: cargo: command not found".to_owned(),
                    exit_code: Some(127),
                    denied: false,
                }),
                item(ItemKind::agent_message("done".to_owned(), Vec::new())),
            ],
            true,
        );
        let status = turn_status(&turn);
        assert_eq!(status.verification, "blocked");
        assert_eq!(status.outcome, "blocked_by_environment");
    }
}
