//! Serializable mirrors of the core's types.
//!
//! The core carries no serde, so the shell owns the transport shape. Tauri
//! converts the *arguments* of a command to camelCase but passes its *return
//! value* straight through, so nothing here is renamed behind our back — but the
//! names still have to be what the TypeScript side expects, which is why the
//! multi-word ones say so explicitly.

use kodo_core::session;
use kodo_core::traces;
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
    pub mode: String,
}

impl From<session::SessionRef> for SessionRefView {
    fn from(reference: session::SessionRef) -> Self {
        SessionRefView {
            id: reference.id,
            project: reference.project.to_string_lossy().into_owned(),
            title: reference.title,
            at: reference.at,
            archived: reference.archived,
            mode: reference.mode.as_str().to_owned(),
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
    pub mode: String,
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
    explicit_ready: bool,
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
                // An explicit ready delivery already passed its task's gates.
                // A document may complete without a command verification run.
                "ready" if verification == "not_run" && (!wrote_files || explicit_ready) => {
                    "completed"
                }
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

    let explicit_ready = delivery == "ready";
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
        outcome: outcome_for(
            lifecycle,
            delivery,
            verification,
            wrote_files,
            explicit_ready,
        ),
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
        /// `native` / `json_fallback` / `tag_fallback` / `fence_fallback` /
        /// `text_only` / "" (unknown, legacy). Surfaced as a chip in the
        /// transparency UI; empty string means "unknown".
        protocol: String,
        /// `stop` / `length` / `tool_calls` / `content_filter` / `cancelled` /
        /// `error` / "" (unknown). Empty for legacy rows.
        #[serde(rename = "finishReason")]
        finish_reason: String,
        /// Whether the response carried a private reasoning block.
        #[serde(rename = "sawReasoning")]
        saw_reasoning: bool,
        /// Streaming wall time in ms.
        #[serde(rename = "durationMs")]
        duration_ms: u64,
        /// Provider-reported end-to-end latency (excluding failover retries).
        #[serde(rename = "providerLatencyMs")]
        provider_latency_ms: u64,
        /// `n` for the n-th retry of the same call (0 on first attempt).
        #[serde(rename = "retryIndex")]
        retry_index: u32,
        /// Relative path under `traces/<session>/<turn>/` pointing at the
        /// full request/response JSON. `None` when capture is disabled.
        #[serde(rename = "llmIoRef")]
        llm_io_ref: Option<String>,
    },
    FileChange {
        changes: Vec<ChangeView>,
        /// Relative path under `traces/<session>/<turn>/spans/` for the
        /// before/after capture of this write. `None` when snapshotting is
        /// disabled or the file didn't actually exist before the tool ran.
        #[serde(rename = "fileSpanRef")]
        file_span_ref: Option<String>,
    },
    AgentMessage {
        text: String,
        checks: Vec<String>,
        /// ready | partial | blocked | failed
        delivery: String,
        /// not_run | running | passed | failed | blocked
        verification: String,
        /// Conversational answer — the UI renders the text alone.
        plain: bool,
        /// Structured failure taxonomy code (`budget_exhausted` /
        /// `acceptance_unmet` / `unrecoverable` / `provider_error` /
        /// `unknown`) when this answer ended the run in a failed state.
        /// Empty for ready/partial answers. Camel-cased to `failureReason`
        /// on the JS side; the desktop reads it to surface the
        /// "继续本轮" affordance on a budget trip even when no file
        /// landed (delivery == "failed").
        #[serde(rename = "failureReason")]
        failure_reason: String,
    },
    /// Model's private reasoning block. Carries the full `<think>` content
    /// so the transparency UI can show what the model was thinking, separate
    /// from its public `Reasoning` summary.
    Thinking {
        content: String,
        /// Public phase that was active when this block was emitted.
        phase: String,
        /// `extraction_main` / `anthropic_reasoning` / `native`.
        source: String,
    },
    /// History-compaction pass. `source_summary` is the text that fed into the
    /// compaction — without it, the trace would only show the *event* of
    /// compacting, never the content that got compressed away.
    Compaction {
        #[serde(rename = "dropped")]
        dropped: usize,
        #[serde(rename = "summarizedInto")]
        summarized_into: usize,
        /// 0..=100
        #[serde(rename = "freedPct")]
        freed_pct: u8,
        #[serde(rename = "sourceSummary")]
        source_summary: String,
        #[serde(rename = "coveredMessages")]
        covered_messages: usize,
    },
    /// A failover event. `errorClass` is the bucketed cause.
    Failover {
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
        #[serde(rename = "retryIndex")]
        retry_index: u32,
    },
    /// A task-plan subtask status update.
    PlanStep {
        #[serde(rename = "stepId")]
        step_id: String,
        title: String,
        /// `search` / `read` / `edit` / `verify` / `explore` / `external`.
        /// Renamed away from `kind` because the variant-level `kind` tag
        /// would shadow it. Wire format uses the underscored form.
        #[serde(rename = "stepKind")]
        step_kind: String,
        status: String,
        #[serde(rename = "triggeredBy")]
        triggered_by: Vec<u32>,
        evidence: Vec<String>,
    },
    /// An approval decision. Distinct from a denied `CommandExecution`
    /// because that one records the *effect*; this records the *decision*.
    Permission {
        /// `command` / `write` / `external` / `websearch`. Same rename
        /// reasoning as `PlanStep.step_kind`.
        #[serde(rename = "permissionKind")]
        permission_kind: String,
        detail: String,
        decision: String,
        context: String,
    },
    /// Context window usage snapshot.
    ContextBudget {
        #[serde(rename = "used")]
        used: u32,
        #[serde(rename = "window")]
        window: u32,
        /// 0..=100
        #[serde(rename = "percent")]
        percent: u8,
        #[serde(rename = "cumulativeChars")]
        cumulative_chars: u64,
        #[serde(rename = "compactedCount")]
        compacted_count: u32,
        #[serde(rename = "lastModel")]
        last_model: String,
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
    #[serde(rename = "undoState")]
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
                protocol,
                finish_reason,
                saw_reasoning,
                duration_ms,
                provider_latency_ms,
                retry_index,
                llm_io_ref,
            } => ItemDetail::ModelCall {
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
            },
            session::ItemKind::FileChange { changes, .. } => ItemDetail::FileChange {
                changes: changes
                    .into_iter()
                    .map(|change| ChangeView {
                        path: change.path,
                        added: change.added,
                        removed: change.removed,
                    })
                    .collect(),
                // The capture layer in the agent attaches the file-span ref
                // to the chip once a snapshot has been persisted; for now
                // legacy chips carry none, so we surface `None`.
                file_span_ref: None,
            },
            session::ItemKind::AgentMessage {
                text,
                checks,
                delivery,
                verification,
                plain,
                failure_reason,
            } => ItemDetail::AgentMessage {
                text,
                checks,
                delivery,
                verification,
                plain,
                failure_reason,
            },
            session::ItemKind::Thinking {
                content,
                phase,
                source,
            } => ItemDetail::Thinking {
                content,
                phase,
                source,
            },
            session::ItemKind::Compaction {
                dropped,
                summarized_into,
                freed_pct,
                source_summary,
                covered_messages,
            } => ItemDetail::Compaction {
                dropped,
                summarized_into,
                freed_pct,
                source_summary,
                covered_messages,
            },
            session::ItemKind::Failover {
                from_provider,
                from_model,
                error_class,
                error,
                to_provider,
                to_model,
                retry_index,
            } => ItemDetail::Failover {
                from_provider,
                from_model,
                error_class,
                error,
                to_provider,
                to_model,
                retry_index,
            },
            session::ItemKind::PlanStep {
                step_id,
                title,
                kind,
                status,
                triggered_by,
                evidence,
            } => ItemDetail::PlanStep {
                step_id,
                title,
                step_kind: kind,
                status,
                triggered_by,
                evidence,
            },
            session::ItemKind::Permission {
                kind,
                detail,
                decision,
                context,
            } => ItemDetail::Permission {
                permission_kind: kind,
                detail,
                decision,
                context,
            },
            session::ItemKind::ContextBudget {
                used,
                window,
                percent,
                cumulative_chars,
                compacted_count,
                last_model,
            } => ItemDetail::ContextBudget {
                used,
                window,
                percent,
                cumulative_chars,
                compacted_count,
                last_model,
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
            mode: session.mode.as_str().to_owned(),
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
    /// Live context-window telemetry after every successful model call.
    /// UI uses this to render the Codex-style progress bar; consumers should
    /// treat it as best-effort (the provider may not report usage).
    ContextBudget {
        session: String,
        used: u32,
        window: u32,
        percent: u8,
        #[serde(rename = "cumulativeChars")]
        cumulative_chars: u64,
        #[serde(rename = "compactedCount")]
        compacted_count: u32,
        #[serde(rename = "lastModel")]
        last_model: String,
    },
    /// History-compaction pass finished — UI surfaces a short toast.
    Compacted {
        session: String,
        dropped: usize,
        #[serde(rename = "summarizedInto")]
        summarized_into: usize,
        #[serde(rename = "freedPct")]
        freed_pct: u8,
    },
    /// Model-initiated question (Phase 0 second-pass). The frontend surfaces a
    /// dialog; the answer is fed back as the next user message.
    AskUser {
        session: String,
        question: String,
        options: Vec<String>,
    },
    /// Memory Engine recall summary. Emitted at most once per turn when
    /// the engine is on. `count = 0` is a valid state (engine on, nothing
    /// matched); the absence of the event itself means the engine is off.
    MemoryRecalled {
        session: String,
        count: usize,
        ids: Vec<String>,
    },
    /// Terminal answer from the agent loop. Always emitted exactly once
    /// per turn unless the loop was killed before it could finish; in
    /// that case the run-side fallback emits a `delivery = "partial"`
    /// synthesized answer so the GUI never sees an empty TurnComplete.
    AgentMessage {
        session: String,
        text: String,
        delivery: String,
        verification: String,
        plain: bool,
        #[serde(rename = "failureReason")]
        failure_reason: String,
        checks: Vec<String>,
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

// ---------------------------------------------------------------------------
// Trace artifacts (Phase D — transparency UI)
//
// The session log stores chips with `llm_io_ref` pointers; the body lives
// under `~/Library/Application Support/Kodo/traces/<session>/<turn>/…`. These
// DTOs and the matching commands in `main.rs` let the webview open those
// captures without holding the whole log in memory.
// ---------------------------------------------------------------------------

/// One trace artifact (LLM I/O capture or file span), as the trace-page
/// list pane needs it.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRefView {
    /// `"llm_io"` or `"file_span"`.
    pub kind: &'static str,
    pub seq: u32,
    pub label: String,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: u64,
    /// Seconds since the Unix epoch.
    pub at: u64,
}

impl From<traces::ArtifactRef> for ArtifactRefView {
    fn from(value: traces::ArtifactRef) -> Self {
        Self {
            kind: value.kind,
            seq: value.seq,
            label: value.label,
            size_bytes: value.size_bytes,
            at: value.at,
        }
    }
}

/// One full LLM I/O capture. `request.messages` is kept opaque so the
/// renderer doesn't have to special-case each provider shape.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmIoView {
    pub seq: u32,
    pub model: String,
    pub protocol: String,
    pub retry_index: u32,
    #[serde(rename = "providerLatencyMs")]
    pub provider_latency_ms: u64,
    #[serde(rename = "finishReason")]
    pub finish_reason: String,
    #[serde(rename = "inputTokens")]
    pub input_tokens: u32,
    #[serde(rename = "outputTokens")]
    pub output_tokens: u32,
    #[serde(rename = "sawReasoning")]
    pub saw_reasoning: bool,
    pub at: u64,
    pub request: LlmIoRequestView,
    pub response: LlmIoResponseView,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmIoRequestView {
    pub system: String,
    pub messages: serde_json::Value,
    pub tools: serde_json::Value,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmIoResponseView {
    pub text: String,
    #[serde(rename = "reasoningContent")]
    pub reasoning_content: String,
    #[serde(rename = "nativeToolCalls")]
    pub native_tool_calls: serde_json::Value,
}

impl From<traces::LlmIoCapture> for LlmIoView {
    fn from(value: traces::LlmIoCapture) -> Self {
        Self {
            seq: value.seq,
            model: value.model,
            protocol: value.protocol,
            retry_index: value.retry_index,
            provider_latency_ms: value.provider_latency_ms,
            finish_reason: value.finish_reason,
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            saw_reasoning: value.saw_reasoning,
            at: value.at,
            request: LlmIoRequestView {
                system: value.request.system,
                messages: value.request.messages,
                tools: value.request.tools,
            },
            response: LlmIoResponseView {
                text: value.response.text,
                reasoning_content: value.response.reasoning_content,
                native_tool_calls: value.response.native_tool_calls,
            },
        }
    }
}

/// One file-span snapshot (before/after content of one tool's write).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpanView {
    pub path: String,
    #[serde(rename = "contentBefore")]
    pub content_before: String,
    /// `None` for create/delete of an empty/missing file.
    #[serde(rename = "contentAfter")]
    pub content_after: Option<String>,
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    pub seq: u32,
}

impl From<traces::FileSpanCapture> for FileSpanView {
    fn from(value: traces::FileSpanCapture) -> Self {
        Self {
            path: value.path,
            content_before: value.content_before,
            content_after: value.content_after,
            tool_call_id: value.tool_call_id,
            seq: value.seq,
        }
    }
}

/// One model inside a provider's configured model list.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelOptionView {
    pub model_id: String,
    pub display_name: String,
    /// Whether the model accepts image attachments as multimodal content
    /// blocks. False → Composer falls back to the path-only descriptor
    /// and (once) shows the user a notice that images will be skipped.
    /// Mirrors `ProviderCapabilities::vision` with a per-model
    /// `vision_override` applied (`crates/agent/src/provider.rs`).
    #[serde(default)]
    pub vision: bool,
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
    /// Configured model list. `None` on legacy records (frontend seeds
    /// template presets); always written back as an array on save so an
    /// intentionally emptied list stays distinguishable from legacy.
    #[serde(default)]
    pub models: Option<Vec<ModelOptionView>>,
    /// True when a secret exists in credentials.log.
    #[serde(default)]
    #[serde(rename = "hasKey")]
    pub has_key: bool,
}

/// One workflow step as the skill preview lists it.
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillStepView {
    pub id: String,
    /// `read` | `edit` | `command` | `verify`.
    pub kind: String,
    pub title: String,
}

/// One skill as the Skills page previews it: builtins (embedded, read-only),
/// project files (`.kodo/skills`) and user files share this shape so one
/// renderer draws all three.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkillView {
    pub name: String,
    /// `builtin` | `project` | `user`.
    pub source: String,
    /// One-line `## description` (falls back to the first criterion on load).
    pub description: String,
    /// Task type labels (`bug-fix`, `work`, …) — which space(s) it belongs to.
    pub task_types: Vec<String>,
    /// Context strategy label (`work-focused`, …).
    pub strategy: String,
    /// Verification policy label (`full` | `tests-only` | `none`).
    pub policy: String,
    pub workflow: Vec<SkillStepView>,
    pub allowed_tools: Vec<String>,
    pub completion_criteria: Vec<String>,
    pub guidance: String,
    /// Raw SKILL.md source — the preview pane's 原文 view and the editor base.
    pub markdown: String,
}

fn skill_step_view(step: &kodo_agent::WorkflowStep) -> SkillStepView {
    let kind = match step.kind {
        kodo_agent::SubtaskKind::Read => "read",
        kodo_agent::SubtaskKind::Edit => "edit",
        kodo_agent::SubtaskKind::Command => "command",
        kodo_agent::SubtaskKind::Verify => "verify",
    };
    SkillStepView {
        id: step.id.clone(),
        kind: kind.to_owned(),
        title: step.title.clone(),
    }
}

/// Map a parsed spec + its raw Markdown onto the wire shape.
pub fn skill_view(spec: &kodo_agent::AgentSkillSpec, source: &str, markdown: String) -> SkillView {
    SkillView {
        name: spec.name.clone(),
        source: source.to_owned(),
        description: spec.description.clone(),
        task_types: spec
            .applicable_task_types
            .iter()
            .map(|t| t.label().to_owned())
            .collect(),
        strategy: spec.context_strategy.label().to_owned(),
        policy: spec.verification_policy.label().to_owned(),
        workflow: spec.workflow.iter().map(skill_step_view).collect(),
        allowed_tools: spec
            .allowed_tools
            .iter()
            .map(|t| t.label().to_owned())
            .collect(),
        completion_criteria: spec.completion_criteria.clone(),
        guidance: spec.guidance.clone(),
        markdown,
    }
}

/// One persona as the 插件 · 智能体 tab lists it. `source` distinguishes
/// the 12 read-only built-ins (`"builtin"`) from user markdown (`"user"`)
/// and project markdown (`"project"`); the UI groups, badges, and gates
/// edit/delete on it.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub name: String,
    /// `"builtin"` | `"user"` | `"project"` — drives the row badge,
    /// edit/delete gating, and the Composer dropdown grouping
    /// (see COMPONENTS.md §13d).
    pub source: String,
    /// One-line `## description` (the selector hint and the row subtitle).
    pub description: String,
    /// `## instructions` body — what gets appended to the system prompt.
    pub instructions: String,
    /// Raw agent markdown — the detail modal's editor base.
    pub markdown: String,
    /// Per-persona session defaults (§0.1 / B.1-B.3). Empty fields mean
    /// "leave the composer's choice alone".
    pub defaults: PersonaDefaultsView,
    /// Declared template variables the `## instructions` may use
    /// (e.g. `project_path`, `branch`). Unknown `{{name}}` placeholders
    /// are preserved verbatim.
    pub variables: Vec<String>,
    /// Few-shot examples rendered after the persona block at run time.
    pub examples: Vec<ExampleView>,
}

#[derive(serde::Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonaDefaultsView {
    /// Provider id (`openai`, `anthropic`, …). When set, the run-time
    /// check verifies it matches the active provider before applying
    /// `model` — cross-provider overrides are ignored.
    pub provider: Option<String>,
    /// Model id (`gpt-4o`, `claude-sonnet-4-5`, …) the persona wants.
    pub model: Option<String>,
    /// Permission level (`"ask"` | `"auto"` | `"full"`).
    pub permission: Option<String>,
    /// Skill ids the persona wants pre-selected. User selections in the
    /// composer are merged on top, never replaced.
    pub skills: Vec<String>,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExampleView {
    pub input: String,
    pub output: String,
}

/// Map a parsed spec + its raw Markdown onto the wire shape. `source` is
/// the caller's classification (`"builtin"` for the 12 embedded records,
/// `"user"` for files on disk, `"project"` for `<project>/.kodo/agents/`).
pub fn agent_view(
    spec: &kodo_agent::agents::AgentSpec,
    source: &str,
    markdown: String,
) -> AgentView {
    AgentView {
        name: spec.name.clone(),
        source: source.to_owned(),
        description: spec.description.clone(),
        instructions: spec.instructions.clone(),
        markdown,
        defaults: PersonaDefaultsView {
            provider: spec.defaults.provider.clone(),
            model: spec.defaults.model.clone(),
            permission: spec.defaults.permission.clone(),
            skills: spec.defaults.skills.clone(),
        },
        variables: spec.variables.clone(),
        examples: spec
            .examples
            .iter()
            .map(|ex| ExampleView {
                input: ex.input.clone(),
                output: ex.output.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kodo_core::session::{Item, ItemKind, Status};
    use serde_json::json;

    #[test]
    fn skill_view_round_trips_the_wire_shape() {
        let spec = kodo_agent::parse_skill(
            "# Skill: my-notes\n\
             ## applicable_task_types\n- work\n\
             ## context_strategy\nwork-focused\n\
             ## allowed_tools\n- read_file\n\
             ## verification_policy\nnone\n\
             ## workflow\n- s1 | read | Read the input\n\
             ## completion_criteria\n- The notes are complete\n\
             ## guidance\nKeep it short.\n",
        )
        .expect("parse");
        let view = skill_view(&spec, "user", "# Skill: my-notes\n".to_owned());
        let value = serde_json::to_value(&view).expect("serialize");
        assert_eq!(value["name"], "my-notes");
        assert_eq!(value["source"], "user");
        // No `## description` in the fixture: falls back to the first criterion.
        assert_eq!(value["description"], "The notes are complete");
        assert_eq!(value["taskTypes"], json!(["work"]));
        assert_eq!(value["strategy"], "work-focused");
        assert_eq!(value["policy"], "none");
        assert_eq!(value["workflow"][0]["kind"], "read");
        assert_eq!(value["workflow"][0]["id"], "s1");
        assert_eq!(value["allowedTools"], json!(["read_file"]));
        assert!(value["completionCriteria"].as_array().is_some());
        assert_eq!(value["guidance"], "Keep it short.");
        assert_eq!(value["markdown"], "# Skill: my-notes\n");
    }

    #[test]
    fn provider_view_tolerates_legacy_records_without_models() {
        let legacy = json!({
            "id": "p1", "name": "OpenAI", "template": "openai",
            "apiKey": "••••abcd", "endpoint": "", "model": "gpt-4o",
            "modelId": "gpt-4o", "hasKey": true
        });
        let view: ProviderView = serde_json::from_value(legacy).unwrap();
        assert!(view.models.is_none());
        // save always writes an array back, even an intentionally empty one
        let with_models = json!({
            "id": "p2", "name": "Custom", "template": "custom",
            "apiKey": "", "endpoint": "http://x", "model": "m",
            "models": [{"modelId": "m", "displayName": "M"}], "hasKey": false
        });
        let view: ProviderView = serde_json::from_value(with_models).unwrap();
        let models = view.models.as_ref().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "m");
        assert_eq!(models[0].display_name, "M");
        let round: serde_json::Value = serde_json::to_value(&view).unwrap();
        assert!(round.get("models").is_some());
    }

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
                    protocol: String::new(),
                    finish_reason: String::new(),
                    saw_reasoning: false,
                    duration_ms: 0,
                    provider_latency_ms: 0,
                    retry_index: 0,
                    llm_io_ref: None,
                },
                "modelCall",
            ),
            (
                ItemKind::FileChange {
                    file_span_ref: None,
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
            mode: kodo_core::session::Mode::Work,
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
        assert_eq!(value["mode"], json!("work"));
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
                    plain: false,
                    failure_reason: String::new(),
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
    fn file_delivery_respects_explicit_outcome_without_inventing_verification() {
        for file_span_ref in [None, Some("span".to_owned())] {
            for (delivery, verification, expected) in [
                ("ready", "not_run", "completed"),
                ("partial", "not_run", "partially_completed"),
                ("failed", "not_run", "failed"),
                ("ready", "failed", "partially_completed"),
                ("ready", "blocked", "blocked_by_environment"),
                ("", "", "partially_completed"),
            ] {
                let turn = turn_with(
                    vec![
                        item(ItemKind::FileChange {
                            changes: vec![session::Change {
                                path: "docs/architecture.md".to_owned(),
                                added: 415,
                                removed: 0,
                            }],
                            file_span_ref: file_span_ref.clone(),
                        }),
                        item(ItemKind::AgentMessage {
                            text: "文档已写入".to_owned(),
                            checks: vec!["已回读文件".to_owned()],
                            delivery: delivery.to_owned(),
                            verification: verification.to_owned(),
                            plain: false,
                            failure_reason: String::new(),
                        }),
                    ],
                    true,
                );
                let status = turn_status(&turn);
                assert_eq!(
                    status.outcome, expected,
                    "delivery={delivery}, span={file_span_ref:?}"
                );
                assert_eq!(
                    status.verification,
                    if verification.is_empty() {
                        "not_run"
                    } else {
                        verification
                    }
                );
            }
        }
    }

    #[test]
    fn explicit_not_run_is_preserved_after_exploration_and_legacy_answer() {
        let turn = turn_with(
            vec![
                item(ItemKind::CommandExecution {
                    command: "git status --short".to_owned(),
                    cwd: "/p".to_owned(),
                    output: "ok".to_owned(),
                    exit_code: Some(0),
                    denied: false,
                }),
                item(ItemKind::AgentMessage {
                    text: "文档已交付".to_owned(),
                    checks: Vec::new(),
                    delivery: "ready".to_owned(),
                    verification: "not_run".to_owned(),
                    plain: false,
                    failure_reason: String::new(),
                }),
                item(ItemKind::agent_message("后续说明", Vec::new())),
            ],
            true,
        );
        let status = turn_status(&turn);
        assert_eq!(status.outcome, "completed");
        assert_eq!(status.verification, "not_run");
    }

    #[test]
    fn an_answer_with_failed_verification_is_partially_completed() {
        let turn = turn_with(
            vec![item(ItemKind::AgentMessage {
                text: "done".to_owned(),
                checks: Vec::new(),
                delivery: "ready".to_owned(),
                verification: "failed".to_owned(),
                plain: false,
                failure_reason: String::new(),
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
                plain: false,
                failure_reason: String::new(),
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

/// Memory entry surfaced through Tauri. Mirrors `kodo_memory::MemoryEntry`
/// with a flat string encoding for the enums (the React side already
/// passes the same string back through `MemoryListArgs`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntryView {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub authority: String,
    pub source_trust: String,
    pub title: String,
    pub abstract_text: String,
    pub body: serde_json::Value,
    pub content_hash: String,
    pub confidence: f32,
    pub verified: bool,
    pub conditions: Vec<String>,
    pub source_uris: Vec<String>,
    pub scope_project_id: Option<String>,
    pub scope_repository_id: Option<String>,
    pub scope_branch: Option<String>,
    pub use_count: u32,
    pub version: u32,
    pub created_at: u64,
    pub updated_at: u64,
}

impl From<kodo_memory::MemoryEntry> for MemoryEntryView {
    fn from(value: kodo_memory::MemoryEntry) -> Self {
        Self {
            id: value.id,
            kind: value.kind.as_str().to_owned(),
            status: value.status.as_str().to_owned(),
            authority: value.authority.as_str().to_owned(),
            source_trust: value.source_trust.as_str().to_owned(),
            title: value.title,
            abstract_text: value.abstract_text,
            body: value.body,
            content_hash: value.content_hash,
            confidence: value.confidence,
            verified: value.verified,
            conditions: value.conditions,
            source_uris: value.source_uris,
            scope_project_id: value.scope.project_id,
            scope_repository_id: value.scope.repository_id,
            scope_branch: value.scope.branch,
            use_count: value.use_count,
            version: value.version,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}
