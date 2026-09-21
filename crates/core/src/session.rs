//! Conversations: one append-only log per session.
//!
//! A session is a file, `sessions/<id>.log`, and the file name *is* the session
//! id — storing the id again on every line would only create a way for the two
//! to disagree. Appending touches one file, so a damaged session cannot take the
//! others with it, and reading one costs that session rather than all history.
//!
//! ```text
//! open           <project>  <title>  <at>
//! ask            <at>  <text>
//! item.started   <seq>  <at>  <durationMs|->  <kind>  <fields…>
//! item.completed <seq>  <at>  <durationMs|->  <kind>  <fields…>
//! item.failed    <seq>  <at>
//! turn.complete  <at>
//! stopped        <at>
//! interrupted    <at>
//! error          <at>  <message>
//! retitle        <title>
//! archive
//! restore
//! ```
//!
//! **Status is not a stored field.** It comes from the envelope: an item with a
//! `started` line and nothing after it is running, and that is how a run killed
//! mid-flight is reported as interrupted rather than as finished. Nothing has to
//! be trusted to have been written correctly at the end of a process that died.
//! On reload without a live run, [`recover_interrupted`] stamps `interrupted` so
//! a persisted Running turn becomes Interrupted — never Completed — while
//! keeping items, changes, and verification intact.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::line;
use crate::settings;

const SESSIONS_DIR: &str = "sessions";

/// A session as the sidebar needs it, without reading its events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef {
    pub id: String,
    pub project: PathBuf,
    pub title: String,
    pub at: u64,
    pub archived: bool,
}

/// A whole conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub project: PathBuf,
    pub title: String,
    pub at: u64,
    pub turns: Vec<Turn>,
    pub archived: bool,
}

/// One request and everything the agent did about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub ask: String,
    /// Relative project paths pinned as context for this turn (not file bodies).
    pub context: Vec<String>,
    pub items: Vec<Item>,
    /// Set by `turn.complete`. A turn that never got one was interrupted.
    pub done: bool,
    /// Set by `stopped`. Without it, a turn the user stopped and a turn a crash
    /// killed would be indistinguishable after a restart — both simply lack
    /// `turn.complete`.
    pub stopped: bool,
    /// Set by recovery after a restart when the turn still had open items.
    /// Running never becomes Completed; it becomes Interrupted.
    pub interrupted: bool,
    pub error: Option<String>,
}

/// One step of work. `status` is derived from the log envelope, never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Identifies the item within its session, so a later line can complete it.
    pub id: u32,
    pub at: u64,
    pub status: Status,
    pub duration_ms: Option<u64>,
    pub kind: ItemKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Running,
    Done,
    Failed,
}

/// Which envelope line an item is being recorded under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Started,
    Completed,
    Failed,
}

/// The per-kind payload. Each kind renders differently, so each carries only
/// what its own renderer needs — there is no generic `tool { name, state }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    /// A summary only. The model's actual reasoning is deliberately not a field
    /// here, so no renderer can show what must stay hidden.
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
        exit_code: Option<i32>,
    },
    ModelCall {
        model: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    FileChange {
        changes: Vec<Change>,
    },
    AgentMessage {
        text: String,
        checks: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// The default sessions directory, or `None` when `HOME` is unset.
pub fn dir() -> Option<PathBuf> {
    Some(settings::state_dir()?.join(SESSIONS_DIR))
}

/// Seconds since the Unix epoch, or 0 if the clock reads before it.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// Starts a session and returns its id.
///
/// The id is built from the clock and a counter, but uniqueness does **not** rest
/// on those being distinct: the file is opened with `create_new`, and an
/// `AlreadyExists` simply means trying the next id. That is atomic, and it needs
/// no assumption about the clock's resolution or about a restart's timing.
pub fn open(dir: &Path, project: &Path, title: &str, at: u64) -> io::Result<String> {
    fs::create_dir_all(dir)?;
    loop {
        let id = new_id();
        let path = dir.join(format!("{id}.log"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                writeln!(
                    file,
                    "{}",
                    line::join(&["open", &project.to_string_lossy(), title, &at.to_string()])
                )?;
                return Ok(id);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

/// Every session, newest first.
///
/// This reads each file rather than an index: an index can drift from the logs it
/// describes, and a listing that is wrong is worse than one that is slow. The
/// scan parses only the `open`, `retitle`, `archive` and `restore` lines, so it costs bytes
/// and not allocations. If listing ever becomes slow enough to matter, that is
/// the moment to add an index — not before.
pub fn list(dir: &Path) -> io::Result<Vec<SessionRef>> {
    let mut found = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
        Err(error) => return Err(error),
    };

    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("log") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if let Ok(lines) = line::read_lines(&path) {
            if let Some(summary) = summarize(id, &lines) {
                found.push(summary);
            }
        }
    }

    found.sort_by_key(|item| std::cmp::Reverse(item.at));
    Ok(found)
}

/// Reads one session, replaying its log in order.
pub fn load(dir: &Path, id: &str) -> io::Result<Session> {
    let lines = line::read_lines(&path_of(dir, id)?)?;

    let mut session = Session {
        id: id.to_owned(),
        project: PathBuf::new(),
        title: String::new(),
        at: 0,
        turns: Vec::new(),
        archived: false,
    };

    for raw in &lines {
        if raw.trim().is_empty() || raw.starts_with('#') {
            continue;
        }
        let fields = line::split(raw);
        match (fields[0].as_str(), &fields[1..]) {
            ("open", [project, title, at]) => {
                session.project = PathBuf::from(project);
                session.title = title.clone();
                session.at = at.parse().unwrap_or(0);
            }
            ("ask", [_at, text]) => session.turns.push(Turn {
                ask: text.clone(),
                context: Vec::new(),
                items: Vec::new(),
                done: false,
                stopped: false,
                interrupted: false,
                error: None,
            }),
            // `ask <at> <text> <context-json>` — optional trailing field.
            ("ask", [_at, text, context]) => session.turns.push(Turn {
                ask: text.clone(),
                context: parse_context_field(context),
                items: Vec::new(),
                done: false,
                stopped: false,
                interrupted: false,
                error: None,
            }),
            ("item.started", rest) => {
                if let Some(item) = decode_item(rest, Status::Running) {
                    upsert_item(&mut session, item);
                }
            }
            ("item.completed", rest) => {
                if let Some(item) = decode_item(rest, Status::Done) {
                    upsert_item(&mut session, item);
                }
            }
            ("item.failed", [id, _at]) => mark_failed(&mut session, id),
            ("turn.complete", [_at]) => {
                if let Some(turn) = session.turns.last_mut() {
                    turn.done = true;
                }
            }
            ("stopped", [_at]) => {
                if let Some(turn) = session.turns.last_mut() {
                    turn.stopped = true;
                }
            }
            ("interrupted", [_at]) => {
                if let Some(turn) = session.turns.last_mut() {
                    turn.interrupted = true;
                }
            }
            ("error", [_at, message]) => {
                if let Some(turn) = session.turns.last_mut() {
                    turn.error = Some(message.clone());
                }
            }
            ("retitle", [title]) => session.title = title.clone(),
            ("archive", _) => session.archived = true,
            ("restore", _) => session.archived = false,
            _ => {}
        }
    }

    Ok(session)
}

/// Records the user's message. A turn without an `ask` is not representable, so
/// this is what opens one.
pub fn record_ask(dir: &Path, id: &str, at: u64, text: &str) -> io::Result<()> {
    append(dir, id, &["ask", &at.to_string(), text])
}

/// Records the user's message plus pinned context paths (project-relative).
/// Paths are stored newline-separated in a trailing field — never file bodies.
pub fn record_ask_with_context(
    dir: &Path,
    id: &str,
    at: u64,
    text: &str,
    context: &[String],
) -> io::Result<()> {
    if context.is_empty() {
        return record_ask(dir, id, at, text);
    }
    let joined = context.join("\n");
    append(dir, id, &["ask", &at.to_string(), text, &joined])
}

/// Parses the optional trailing `ask` context field (newline-separated paths).
fn parse_context_field(raw: &str) -> Vec<String> {
    raw.split('\n')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Records a step of work under the given envelope.
pub fn record_item(dir: &Path, id: &str, item: &Item, phase: Phase) -> io::Result<()> {
    let verb = match phase {
        Phase::Started => "item.started",
        Phase::Completed => "item.completed",
        Phase::Failed => "item.failed",
    };

    // A failed item carries no payload: the step's own exit code and output
    // already say what went wrong, and repeating it here could only disagree.
    if phase == Phase::Failed {
        return append(dir, id, &[verb, &item.id.to_string(), &item.at.to_string()]);
    }

    let fields = encode_item(item);
    let mut refs: Vec<&str> = Vec::with_capacity(fields.len() + 1);
    refs.push(verb);
    refs.extend(fields.iter().map(String::as_str));
    append(dir, id, &refs)
}

/// Marks the current turn finished.
pub fn record_turn_complete(dir: &Path, id: &str, at: u64) -> io::Result<()> {
    append(dir, id, &["turn.complete", &at.to_string()])
}

/// Marks the current turn as stopped by the user rather than finished.
///
/// There is no `turn.complete` after this, so the turn still reads as unfinished
/// — which it is. This line only records *why*, so the GUI can say "stopped"
/// where a turn with no such line says "interrupted".
pub fn record_stopped(dir: &Path, id: &str, at: u64) -> io::Result<()> {
    append(dir, id, &["stopped", &at.to_string()])
}

/// Recovery for a killed run found on load with no live driver.
///
/// Appends `interrupted` only when the last turn still has open items and was
/// neither completed, stopped, nor already recovered. Never marks the turn
/// done, never rewrites item payloads, and never touches the changeset — so
/// last phase, known changes, verification, and undo/diff all survive.
///
/// Returns whether a marker was written.
pub fn recover_interrupted(dir: &Path, id: &str, at: u64) -> io::Result<bool> {
    let session = load(dir, id)?;
    let Some(turn) = session.turns.last() else {
        return Ok(false);
    };
    if turn.done || turn.stopped || turn.interrupted || turn.error.is_some() {
        return Ok(false);
    }
    if !turn.items.iter().any(|item| item.status == Status::Running) {
        return Ok(false);
    }
    append(dir, id, &["interrupted", &at.to_string()])?;
    Ok(true)
}

/// Records a failure that belongs to the turn rather than to one step.
pub fn record_error(dir: &Path, id: &str, at: u64, message: &str) -> io::Result<()> {
    append(dir, id, &["error", &at.to_string(), message])
}

/// Marks the current turn interrupted after recovery (app restart mid-run).
///
/// Never writes `turn.complete`: a running persisted session becomes Interrupted,
/// not Completed. Items, phases, and payload lines stay untouched so last phase,
/// known changes, and undo/diff remain available.
pub fn record_interrupted(dir: &Path, id: &str, at: u64) -> io::Result<()> {
    append(dir, id, &["interrupted", &at.to_string()])
}

/// On startup: convert unfinished non-live turns to Interrupted.
/// Returns session ids that received a recovery line.
///
/// Live ids (still running in this process) are skipped. Never emits
/// `turn.complete` — Running must not become Completed.
pub fn recover_interrupted(dir: &Path, live: &std::collections::HashSet<String>) -> io::Result<Vec<String>> {
    let mut recovered = Vec::new();
    for reference in list(dir)? {
        if live.contains(&reference.id) {
            continue;
        }
        let session = load(dir, &reference.id)?;
        let Some(turn) = session.turns.last() else {
            continue;
        };
        if turn.done || turn.stopped || turn.interrupted || turn.error.is_some() {
            continue;
        }
        // Unfinished turn with no driver → interrupted, not completed.
        record_interrupted(dir, &reference.id, now())?;
        recovered.push(reference.id);
    }
    Ok(recovered)
}

/// Changes the display title. The file name does not move.
pub fn retitle(dir: &Path, id: &str, title: &str) -> io::Result<()> {
    append(dir, id, &["retitle", title])
}

/// Hides the session from the active list. Nothing is deleted.
pub fn archive(dir: &Path, id: &str) -> io::Result<()> {
    append(dir, id, &["archive"])
}

/// Brings an archived session back to the active list.
pub fn restore(dir: &Path, id: &str) -> io::Result<()> {
    append(dir, id, &["restore"])
}

fn append(dir: &Path, id: &str, fields: &[&str]) -> io::Result<()> {
    line::append(&path_of(dir, id)?, &line::join(fields))
}

/// The file for `id`, refusing anything that is not a single ordinary name.
///
/// Ids reach here from the GUI, so `../../…` must not be able to name a file
/// outside the sessions directory. The `.log` suffix is pushed onto the validated
/// component rather than applied with `with_extension`, which would rewrite a
/// stem that happens to contain a dot.
fn path_of(dir: &Path, id: &str) -> io::Result<PathBuf> {
    let mut components = Path::new(id).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(part)), None) => {
            let mut name = part.to_os_string();
            name.push(".log");
            Ok(dir.join(name))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a usable session id: {id:?}"),
        )),
    }
}

/// `create_new` already guarantees this is free; the counter only makes the id
/// legible and keeps two calls in the same nanosecond from looking alike.
fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| u64::from(since.subsec_nanos()))
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{}", base36(now()), base36(nanos), base36(count))
}

fn base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_owned();
    }
    let mut digits = Vec::new();
    while value > 0 {
        digits.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    digits.reverse();
    String::from_utf8(digits).expect("base36 digits are ASCII")
}

/// The sidebar's view of a session, taken from its header lines only.
fn summarize(id: &str, lines: &[String]) -> Option<SessionRef> {
    let mut project = None;
    let mut title = String::new();
    let mut at = 0;
    let mut archived = false;

    for raw in lines {
        if raw.starts_with("open\t") {
            let fields = line::split(raw);
            if let [_, project_field, title_field, at_field] = fields.as_slice() {
                project = Some(PathBuf::from(project_field));
                title = title_field.clone();
                at = at_field.parse().unwrap_or(0);
            }
        } else if raw.starts_with("retitle\t") {
            let fields = line::split(raw);
            if let [_, title_field] = fields.as_slice() {
                title = title_field.clone();
            }
        } else if raw == "archive" {
            archived = true;
        } else if raw == "restore" {
            archived = false;
        }
    }

    // No `open` line means the file was never a session.
    project.map(|project| SessionRef {
        id: id.to_owned(),
        project,
        title,
        at,
        archived,
    })
}

/// Replaces an item already present under the same id, else appends it. This is
/// what lets `item.completed` carry only the fields it has.
fn upsert_item(session: &mut Session, item: Item) {
    let Some(turn) = session.turns.last_mut() else {
        // Items arrive inside a turn. One that does not is as unusable as a
        // damaged line, and is skipped the same way.
        return;
    };
    match turn
        .items
        .iter_mut()
        .find(|existing| existing.id == item.id)
    {
        Some(existing) => *existing = item,
        None => turn.items.push(item),
    }
}

fn mark_failed(session: &mut Session, id: &str) {
    let Ok(id) = id.parse::<u32>() else {
        return;
    };
    if let Some(turn) = session.turns.last_mut() {
        if let Some(item) = turn.items.iter_mut().find(|item| item.id == id) {
            item.status = Status::Failed;
        }
    }
}

/// `seq`, `at`, duration, kind, then the kind's own fields. Duration sits in a
/// fixed position so a variable-length kind (file changes) can still be decoded:
/// whatever follows the kind is its payload, and nothing follows that.
fn encode_item(item: &Item) -> Vec<String> {
    let mut fields = vec![
        item.id.to_string(),
        item.at.to_string(),
        item.duration_ms
            .map(|ms| ms.to_string())
            .unwrap_or_default(),
        kind_name(&item.kind).to_owned(),
    ];

    match &item.kind {
        ItemKind::Reasoning { summary } => fields.push(summary.clone()),
        ItemKind::Search { query, detail } => {
            fields.push(query.clone());
            fields.push(detail.clone());
        }
        ItemKind::FileRead { path, detail } => {
            fields.push(path.clone());
            fields.push(detail.clone());
        }
        ItemKind::CommandExecution {
            command,
            cwd,
            output,
            exit_code,
        } => {
            fields.push(command.clone());
            fields.push(cwd.clone());
            fields.push(output.clone());
            fields.push(exit_code.map(|code| code.to_string()).unwrap_or_default());
        }
        ItemKind::ModelCall {
            model,
            input_tokens,
            output_tokens,
        } => {
            fields.push(model.clone());
            fields.push(input_tokens.to_string());
            fields.push(output_tokens.to_string());
        }
        ItemKind::FileChange { changes } => {
            for change in changes {
                fields.push(change.path.clone());
                fields.push(change.added.to_string());
                fields.push(change.removed.to_string());
            }
        }
        ItemKind::AgentMessage { text, checks } => {
            fields.push(text.clone());
            // Newlines are escaped by the codec, so one field can hold the list.
            fields.push(checks.join("\n"));
        }
    }

    fields
}

fn decode_item(rest: &[String], status: Status) -> Option<Item> {
    let [id, at, duration, kind, payload @ ..] = rest else {
        return None;
    };
    let id = id.parse().ok()?;
    let at = at.parse().ok()?;
    let duration_ms = duration.parse().ok();

    let kind = match (kind.as_str(), payload) {
        ("reasoning", [summary]) => ItemKind::Reasoning {
            summary: summary.clone(),
        },
        ("search", [query, detail]) => ItemKind::Search {
            query: query.clone(),
            detail: detail.clone(),
        },
        ("fileRead", [path, detail]) => ItemKind::FileRead {
            path: path.clone(),
            detail: detail.clone(),
        },
        ("commandExecution", [command, cwd, output, exit_code]) => ItemKind::CommandExecution {
            command: command.clone(),
            cwd: cwd.clone(),
            output: output.clone(),
            exit_code: exit_code.parse().ok(),
        },
        ("modelCall", [model, input, output]) => ItemKind::ModelCall {
            model: model.clone(),
            input_tokens: input.parse().ok()?,
            output_tokens: output.parse().ok()?,
        },
        ("fileChange", changes) if changes.len() % 3 == 0 => ItemKind::FileChange {
            changes: changes
                .as_chunks::<3>()
                .0
                .iter()
                .map(|change| Change {
                    path: change[0].clone(),
                    added: change[1].parse().unwrap_or(0),
                    removed: change[2].parse().unwrap_or(0),
                })
                .collect(),
        },
        ("agentMessage", [text, checks]) => ItemKind::AgentMessage {
            text: text.clone(),
            checks: if checks.is_empty() {
                Vec::new()
            } else {
                checks.split('\n').map(str::to_owned).collect()
            },
        },
        _ => return None,
    };

    Some(Item {
        id,
        at,
        status,
        duration_ms,
        kind,
    })
}

fn kind_name(kind: &ItemKind) -> &'static str {
    match kind {
        ItemKind::Reasoning { .. } => "reasoning",
        ItemKind::Search { .. } => "search",
        ItemKind::FileRead { .. } => "fileRead",
        ItemKind::CommandExecution { .. } => "commandExecution",
        ItemKind::ModelCall { .. } => "modelCall",
        ItemKind::FileChange { .. } => "fileChange",
        ItemKind::AgentMessage { .. } => "agentMessage",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn item(id: u32, kind: ItemKind) -> Item {
        Item {
            id,
            at: 1_700_000_000,
            status: Status::Done,
            duration_ms: Some(1_200),
            kind,
        }
    }

    #[test]
    fn opening_twice_gives_two_distinct_sessions() {
        let tmp = TempDir::new("sess-open");
        let dir = tmp.dir("sessions");

        let first = open(&dir, Path::new("/projects/one"), "新对话", 1).expect("first");
        let second = open(&dir, Path::new("/projects/one"), "新对话", 1).expect("second");

        assert_ne!(first, second);
        assert!(dir.join(format!("{first}.log")).is_file());
        assert!(dir.join(format!("{second}.log")).is_file());
    }

    #[test]
    fn an_opened_session_remembers_its_project_and_title() {
        let tmp = TempDir::new("sess-open-fields");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/projects/one"), "修复路由", 42).expect("open");

        let session = load(&dir, &id).expect("load");
        assert_eq!(session.project, PathBuf::from("/projects/one"));
        assert_eq!(session.title, "修复路由");
        assert_eq!(session.at, 42);
        assert!(session.turns.is_empty());
    }

    #[test]
    fn an_id_is_a_single_plain_file_name() {
        let tmp = TempDir::new("sess-id-shape");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");

        assert!(!id.contains('/'), "{id}");
        assert!(!id.contains('.'), "{id}");
        assert!(!id.contains(".."), "{id}");
    }

    #[test]
    fn a_traversing_id_is_refused() {
        let tmp = TempDir::new("sess-traversal");
        let dir = tmp.dir("sessions");

        for id in ["../escape", "..", ".", "", "a/b"] {
            assert!(load(&dir, id).is_err(), "{id:?} should have been refused");
            assert!(
                record_ask(&dir, id, 1, "hi").is_err(),
                "{id:?} should have been refused"
            );
        }
    }

    #[test]
    fn ask_opens_a_turn_that_turn_complete_closes() {
        let tmp = TempDir::new("sess-turn");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");

        record_ask(&dir, &id, 10, "帮我看一下这个报错").expect("ask");
        let session = load(&dir, &id).expect("load");
        assert_eq!(session.turns.len(), 1);
        assert_eq!(session.turns[0].ask, "帮我看一下这个报错");
        assert!(session.turns[0].context.is_empty());
        assert!(!session.turns[0].done, "a fresh turn is not done");

        record_turn_complete(&dir, &id, 20).expect("complete");
        assert!(load(&dir, &id).expect("load").turns[0].done);
    }

    #[test]
    fn ask_round_trips_pinned_context_paths() {
        let tmp = TempDir::new("sess-ctx");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");

        record_ask(&dir, &id, 10, "无上下文").expect("ask");
        record_ask_with_context(
            &dir,
            &id,
            20,
            "再看这个",
            &["src/lib.rs".to_owned(), "a b.md".to_owned()],
        )
        .expect("ask+ctx");

        let loaded = load(&dir, &id).expect("load");
        assert_eq!(loaded.turns.len(), 2);
        assert!(loaded.turns[0].context.is_empty());
        assert_eq!(loaded.turns[0].ask, "无上下文");
        assert_eq!(
            loaded.turns[1].context,
            vec!["src/lib.rs".to_owned(), "a b.md".to_owned()]
        );
        assert_eq!(loaded.turns[1].ask, "再看这个");
    }

    #[test]
    fn started_then_completed_is_one_item() {
        let tmp = TempDir::new("sess-item");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");
        record_ask(&dir, &id, 10, "go").expect("ask");

        let mut running = item(
            1,
            ItemKind::Reasoning {
                summary: "先看看目录结构".to_owned(),
            },
        );
        running.status = Status::Running;
        running.duration_ms = None;
        record_item(&dir, &id, &running, Phase::Started).expect("started");

        let after_start = load(&dir, &id).expect("load");
        assert_eq!(after_start.turns[0].items.len(), 1);
        assert_eq!(after_start.turns[0].items[0].status, Status::Running);
        assert_eq!(
            after_start.turns[0].items[0].duration_ms, None,
            "a running step has no duration yet"
        );

        // The completion carries the duration the start could not know.
        let finished = item(
            1,
            ItemKind::Reasoning {
                summary: "先看看目录结构".to_owned(),
            },
        );
        record_item(&dir, &id, &finished, Phase::Completed).expect("completed");
        let after_done = load(&dir, &id).expect("load");
        assert_eq!(
            after_done.turns[0].items.len(),
            1,
            "the item was duplicated"
        );
        assert_eq!(after_done.turns[0].items[0].status, Status::Done);
        assert_eq!(after_done.turns[0].items[0].duration_ms, Some(1_200));
    }

    #[test]
    fn a_killed_run_replays_as_still_running() {
        // The whole reason status is an envelope and not a field: nothing has to
        // be written on the way out for this to be right.
        let tmp = TempDir::new("sess-killed");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");
        record_ask(&dir, &id, 10, "go").expect("ask");
        record_item(
            &dir,
            &id,
            &item(
                1,
                ItemKind::Reasoning {
                    summary: "s".to_owned(),
                },
            ),
            Phase::Started,
        )
        .expect("started");

        let session = load(&dir, &id).expect("load");
        assert_eq!(session.turns[0].items[0].status, Status::Running);
        assert!(
            !session.turns[0].done,
            "an interrupted turn must not read as finished"
        );
    }

    #[test]
    fn recovery_turns_running_into_interrupted_without_completion() {
        let tmp = TempDir::new("sess-recover");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");
        record_ask(&dir, &id, 10, "go").expect("ask");
        record_item(
            &dir,
            &id,
            &item(
                1,
                ItemKind::Reasoning {
                    summary: "phase note".to_owned(),
                },
            ),
            Phase::Started,
        )
        .expect("started");

        let stamped = recover_interrupted(&dir, &id, 99).expect("recover");
        assert!(stamped, "open running turn must be recovered");
        let session = load(&dir, &id).expect("reload");
        assert!(session.turns[0].interrupted, "must read as interrupted");
        assert!(
            !session.turns[0].done,
            "recovery must never mark the turn Completed"
        );
        assert_eq!(session.turns[0].items[0].status, Status::Running);
        assert_eq!(
            session.turns[0].items[0].kind,
            ItemKind::Reasoning {
                summary: "phase note".to_owned()
            },
            "item payload (last phase) is preserved"
        );

        // Idempotent: a second recovery does not append another marker.
        assert!(!recover_interrupted(&dir, &id, 100).expect("second"));
    }

    #[test]
    fn recovery_skips_completed_and_stopped_turns() {
        let tmp = TempDir::new("sess-recover-done");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");
        record_ask(&dir, &id, 10, "go").expect("ask");
        record_turn_complete(&dir, &id, 20).expect("complete");
        assert!(!recover_interrupted(&dir, &id, 30).expect("done"));

        let id2 = open(&dir, Path::new("/p"), "t2", 1).expect("open");
        record_ask(&dir, &id2, 10, "go").expect("ask");
        record_item(
            &dir,
            &id2,
            &item(1, ItemKind::Reasoning { summary: "s".to_owned() }),
            Phase::Started,
        )
        .expect("started");
        record_stopped(&dir, &id2, 20).expect("stop");
        assert!(!recover_interrupted(&dir, &id2, 30).expect("stopped"));
        let session = load(&dir, &id2).expect("load");
        assert!(session.turns[0].stopped);
        assert!(!session.turns[0].interrupted);
    }

    #[test]
    fn a_failed_item_is_marked_without_a_payload() {
        let tmp = TempDir::new("sess-failed");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");
        record_ask(&dir, &id, 10, "go").expect("ask");

        let step = item(
            7,
            ItemKind::CommandExecution {
                command: "cargo test".to_owned(),
                cwd: "/p".to_owned(),
                output: "error[E0308]".to_owned(),
                exit_code: Some(101),
            },
        );
        record_item(&dir, &id, &step, Phase::Completed).expect("completed");
        record_item(&dir, &id, &step, Phase::Failed).expect("failed");

        let session = load(&dir, &id).expect("load");
        assert_eq!(session.turns[0].items.len(), 1);
        assert_eq!(session.turns[0].items[0].status, Status::Failed);
        // The payload survived: the exit code is how the UI explains it.
        assert!(matches!(
            session.turns[0].items[0].kind,
            ItemKind::CommandExecution {
                exit_code: Some(101),
                ..
            },
        ));
    }

    #[test]
    fn every_item_kind_round_trips() {
        let tmp = TempDir::new("sess-round-trip");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");
        record_ask(&dir, &id, 10, "go").expect("ask");

        let kinds = vec![
            ItemKind::Reasoning {
                summary: "看目录\n再决定".to_owned(),
            },
            ItemKind::Search {
                query: "unknown table".to_owned(),
                detail: "12 处匹配".to_owned(),
            },
            ItemKind::FileRead {
                path: "/p/src/route.rs".to_owned(),
                detail: "214 行".to_owned(),
            },
            ItemKind::CommandExecution {
                command: "cargo check --workspace".to_owned(),
                cwd: "/p".to_owned(),
                output: "warning: unused\n\tfinished".to_owned(),
                exit_code: Some(0),
            },
            ItemKind::ModelCall {
                model: "claude-sonnet-5".to_owned(),
                input_tokens: 12_480,
                output_tokens: 1_096,
            },
            ItemKind::FileChange {
                changes: vec![
                    Change {
                        path: "src/a.rs".to_owned(),
                        added: 12,
                        removed: 3,
                    },
                    Change {
                        path: "src/b\t.rs".to_owned(),
                        added: 0,
                        removed: 8,
                    },
                ],
            },
            ItemKind::AgentMessage {
                text: "已修复。".to_owned(),
                checks: vec!["cargo test 通过".to_owned(), "无新增告警".to_owned()],
            },
        ];

        let mut expected = Vec::new();
        for (index, kind) in kinds.into_iter().enumerate() {
            let step = item(index as u32 + 1, kind);
            record_item(&dir, &id, &step, Phase::Completed).expect("record");
            expected.push(step);
        }

        let session = load(&dir, &id).expect("load");
        assert_eq!(session.turns[0].items, expected);
    }

    #[test]
    fn an_empty_check_list_stays_empty() {
        let tmp = TempDir::new("sess-no-checks");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");
        record_ask(&dir, &id, 10, "go").expect("ask");

        let step = item(
            1,
            ItemKind::AgentMessage {
                text: "done".to_owned(),
                checks: Vec::new(),
            },
        );
        record_item(&dir, &id, &step, Phase::Completed).expect("record");

        assert_eq!(load(&dir, &id).expect("load").turns[0].items[0], step);
    }

    #[test]
    fn retitle_changes_the_title_and_keeps_the_file_name() {
        let tmp = TempDir::new("sess-retitle");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "新对话", 1).expect("open");

        retitle(&dir, &id, "修复 k2k-rust 未知表路由").expect("retitle");

        assert_eq!(
            load(&dir, &id).expect("load").title,
            "修复 k2k-rust 未知表路由"
        );
        assert!(
            dir.join(format!("{id}.log")).is_file(),
            "the file was renamed"
        );
    }

    #[test]
    fn archive_hides_without_deleting() {
        let tmp = TempDir::new("sess-archive");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");

        archive(&dir, &id).expect("archive");

        assert!(load(&dir, &id).expect("load").archived);
        let listed = list(&dir).expect("list");
        assert_eq!(listed.len(), 1, "archiving must not remove the file");
        assert!(listed[0].archived);
    }

    #[test]
    fn list_returns_newest_first() {
        let tmp = TempDir::new("sess-list-order");
        let dir = tmp.dir("sessions");
        let old = open(&dir, Path::new("/p"), "old", 100).expect("open");
        let new = open(&dir, Path::new("/p"), "new", 300).expect("open");
        let middle = open(&dir, Path::new("/p"), "middle", 200).expect("open");

        let ids: Vec<String> = list(&dir)
            .expect("list")
            .into_iter()
            .map(|entry| entry.id)
            .collect();
        assert_eq!(ids, vec![new, middle, old]);
    }

    #[test]
    fn list_uses_a_retitled_title() {
        let tmp = TempDir::new("sess-list-retitle");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "新对话", 1).expect("open");
        retitle(&dir, &id, "改过的标题").expect("retitle");

        let listed = list(&dir).expect("list");
        assert_eq!(listed[0].title, "改过的标题");
        assert_eq!(listed[0].project, PathBuf::from("/p"));
    }

    #[test]
    fn list_ignores_files_that_are_not_sessions() {
        let tmp = TempDir::new("sess-list-junk");
        let dir = tmp.dir("sessions");
        open(&dir, Path::new("/p"), "real", 1).expect("open");
        fs::write(dir.join("notes.txt"), "not a session").expect("junk");
        fs::write(dir.join("stray.log"), "no open line here").expect("junk");

        assert_eq!(list(&dir).expect("list").len(), 1);
    }

    #[test]
    fn list_of_a_missing_directory_is_empty() {
        let tmp = TempDir::new("sess-list-missing");
        assert!(list(&tmp.path().join("nope")).expect("list").is_empty());
    }

    #[test]
    fn a_damaged_line_costs_only_itself() {
        let tmp = TempDir::new("sess-damaged");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");
        record_ask(&dir, &id, 10, "go").expect("ask");

        let good = item(
            1,
            ItemKind::Reasoning {
                summary: "kept".to_owned(),
            },
        );
        record_item(&dir, &id, &good, Phase::Completed).expect("record");
        line::append(&dir.join(format!("{id}.log")), "item.completed\tnonsense").expect("junk");
        line::append(&dir.join(format!("{id}.log")), "a verb nobody knows\tx").expect("junk");
        record_item(
            &dir,
            &id,
            &item(
                2,
                ItemKind::Reasoning {
                    summary: "also kept".to_owned(),
                },
            ),
            Phase::Completed,
        )
        .expect("record");

        let session = load(&dir, &id).expect("load");
        assert_eq!(
            session.turns[0].items.len(),
            2,
            "a damaged line ate a good one"
        );
        assert_eq!(session.turns[0].items[0], good);
    }

    #[test]
    fn an_item_before_any_ask_is_skipped() {
        let tmp = TempDir::new("sess-item-no-turn");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");

        record_item(
            &dir,
            &id,
            &item(
                1,
                ItemKind::Reasoning {
                    summary: "s".to_owned(),
                },
            ),
            Phase::Completed,
        )
        .expect("record");

        let session = load(&dir, &id).expect("load");
        assert!(
            session.turns.is_empty(),
            "an item with no turn should not invent one"
        );
    }

    #[test]
    fn a_stopped_turn_is_told_apart_from_a_killed_one() {
        // Both lack `turn.complete`; only one has `stopped`.
        let tmp = TempDir::new("sess-stopped");
        let dir = tmp.dir("sessions");

        let stopped = open(&dir, Path::new("/p"), "stopped", 1).expect("open");
        record_ask(&dir, &stopped, 10, "go").expect("ask");
        record_stopped(&dir, &stopped, 11).expect("stopped");

        let killed = open(&dir, Path::new("/p"), "killed", 2).expect("open");
        record_ask(&dir, &killed, 10, "go").expect("ask");

        let stopped = load(&dir, &stopped).expect("load").turns.remove(0);
        let killed = load(&dir, &killed).expect("load").turns.remove(0);

        assert!(!stopped.done && !killed.done, "neither turn finished");
        assert!(stopped.stopped, "the user stopped this one");
        assert!(!killed.stopped, "this one was killed, not stopped");
    }

    #[test]
    fn a_turn_error_is_kept() {
        let tmp = TempDir::new("sess-error");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "t", 1).expect("open");
        record_ask(&dir, &id, 10, "go").expect("ask");
        record_error(&dir, &id, 11, "模型返回了无法解析的内容").expect("error");

        assert_eq!(
            load(&dir, &id).expect("load").turns[0].error.as_deref(),
            Some("模型返回了无法解析的内容"),
        );
    }

    #[test]
    fn recovery_marks_running_as_interrupted_never_completed() {
        let tmp = TempDir::new("sess-recover");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "killed", 1).expect("open");
        record_ask(&dir, &id, 10, "go").expect("ask");
        record_item(
            &dir,
            &id,
            &item(
                1,
                ItemKind::Reasoning {
                    summary: "s".to_owned(),
                },
            ),
            Phase::Started,
        )
        .expect("started");

        let live = std::collections::HashSet::new();
        let recovered = recover_interrupted(&dir, &live).expect("recover");
        assert_eq!(recovered, vec![id.clone()]);

        let session = load(&dir, &id).expect("load");
        let turn = &session.turns[0];
        assert!(turn.interrupted, "must convert Running → Interrupted");
        assert!(!turn.done, "recovery must never write turn.complete");
        assert_eq!(turn.items[0].status, Status::Running, "item envelope kept");
        assert_eq!(
            turn.items[0].kind,
            item(
                1,
                ItemKind::Reasoning {
                    summary: "s".to_owned(),
                }
            )
            .kind,
            "last phase payload preserved for undo/diff"
        );

        // Second recovery is a no-op (idempotent).
        let again = recover_interrupted(&dir, &live).expect("recover again");
        assert!(again.is_empty());
    }

    #[test]
    fn recovery_skips_live_and_finished_turns() {
        let tmp = TempDir::new("sess-recover-skip");
        let dir = tmp.dir("sessions");

        let done = open(&dir, Path::new("/p"), "done", 1).expect("open");
        record_ask(&dir, &done, 10, "go").expect("ask");
        record_turn_complete(&dir, &done, 20).expect("complete");

        let live = open(&dir, Path::new("/p"), "live", 2).expect("open");
        record_ask(&dir, &live, 10, "go").expect("ask");

        let mut live_set = std::collections::HashSet::new();
        live_set.insert(live.clone());
        let recovered = recover_interrupted(&dir, &live_set).expect("recover");
        assert!(recovered.is_empty(), "live and completed must be skipped");
        assert!(!load(&dir, &live).expect("load").turns[0].interrupted);
        assert!(load(&dir, &done).expect("load").turns[0].done);
    }

    #[test]
    fn restore_brings_an_archived_session_back() {
        let tmp = TempDir::new("sess-restore");
        let dir = tmp.dir("sessions");
        let id = open(&dir, Path::new("/p"), "归档后恢复", 1).expect("open");
        archive(&dir, &id).expect("archive");
        assert!(load(&dir, &id).expect("load").archived);
        assert!(list(&dir).expect("list")[0].archived);

        restore(&dir, &id).expect("restore");
        assert!(!load(&dir, &id).expect("load").archived);
        assert!(!list(&dir).expect("list")[0].archived);
    }

    #[test]
    fn base36_covers_its_range() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
    }
}
