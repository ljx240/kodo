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
            session::ItemKind::Reasoning { summary } => ItemDetail::Reasoning { summary },
            session::ItemKind::Search { query, detail } => ItemDetail::Search { query, detail },
            session::ItemKind::FileRead { path, detail } => ItemDetail::FileRead { path, detail },
            session::ItemKind::CommandExecution {
                command,
                cwd,
                output,
                exit_code,
            } => ItemDetail::CommandExecution {
                command,
                cwd,
                output,
                exit_code,
            },
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
            session::ItemKind::AgentMessage { text, checks } => {
                ItemDetail::AgentMessage { text, checks }
            }
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
                .map(|turn| TurnView {
                    ask: turn.ask,
                    context: turn.context,
                    items: turn.items.into_iter().map(Into::into).collect(),
                    done: turn.done,
                    stopped: turn.stopped,
                    interrupted: turn.interrupted,
                    error: turn.error,
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
        /// Working directory the command would run in.
        #[serde(default)]
        cwd: String,
        /// Structured risk: Safe | FilesystemWrite | Network | PackageInstall |
        /// DestructiveGit | ProcessControl | SensitiveData | Dangerous.
        #[serde(default)]
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
        from_provider: String,
        from_model: String,
        error_class: String,
        error: String,
        to_provider: String,
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
        let mut running = item(ItemKind::Reasoning {
            summary: "先看目录".to_owned(),
        });
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
            (
                ItemKind::Reasoning {
                    summary: String::new(),
                },
                "reasoning",
            ),
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
                ItemKind::AgentMessage {
                    text: String::new(),
                    checks: Vec::new(),
                },
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
            item: ItemView::from(item(ItemKind::Reasoning {
                summary: "s".to_owned(),
            })),
        };
        let value = serde_json::to_value(event).expect("an event should serialize");

        assert_eq!(value["type"], json!("itemStarted"));
        assert_eq!(value["session"], json!("abc"));
        assert_eq!(value["item"]["kind"], json!("reasoning"));
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
                items: vec![item(ItemKind::Reasoning {
                    summary: "s".to_owned(),
                })],
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
}
