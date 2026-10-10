//! Per-turn trace archive for the transparency UI.
//!
//! Sessions live in `sessions/<id>.log` as an append-only event log; that is
//! what the GUI replays on load. The trace archive is the **second** store for
//! the same turn, and it exists for one reason: the session log is sized in
//! kilobytes, but a single LLM call can carry tens of kilobytes of full
//! request/response — large enough to balloon the log past anything a UI can
//! load lazily. So we keep the session log as a chip-level index
//! (`llm_io_ref = Some("llm_io/001.json")`) and put the heavy bodies in
//! `traces/<session_id>/<turn_seq>/llm_io/<seq>.json`.
//!
//! ```text
//! traces/
//!     <session_id>/
//!         <turn_seq>/
//!             llm_io/
//!                 001.json       ← raw request/response (one per call)
//!                 002.json
//!             spans/
//!                 001-file-<hash>.json   ← file content snapshot for one tool
//!             turn_meta.json
//! ```
//!
//! Capture is opt-in. Tests and evals leave it off so they stay hermetic; the
//! GUI and CLI default to capture off. Callers invoke writers only after the
//! user enables local detailed traces. Bodies stay local, redacted, bounded,
//! and private; no private reasoning body is captured.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::settings;

/// One full LLM I/O capture (system prompt + messages + response). Persisted
/// alongside the chip-level `ItemKind::ModelCall` so the transparency UI can
/// show what the model actually received and replied with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmIoCapture {
    /// Sequential index within the turn (1-based).
    pub seq: u32,
    pub model: String,
    /// `native` / `json_fallback` / `tag_fallback` / `fence_fallback` /
    /// `text_only`. Mirrors `ModelCall::protocol`.
    pub protocol: String,
    /// 0 on first attempt, `n` for the n-th retry of the same call.
    pub retry_index: u32,
    pub provider_latency_ms: u64,
    /// `stop` / `length` / `tool_calls` / `content_filter` / `cancelled` /
    /// `error`.
    pub finish_reason: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Whether the response carried a private reasoning block.
    pub saw_reasoning: bool,
    /// Seconds since the Unix epoch.
    pub at: u64,
    pub request: RequestPayload,
    pub response: ResponsePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestPayload {
    pub system: String,
    /// Plain JSON array of provider message objects — kept opaque so we don't
    /// lock ourselves to a specific provider shape.
    pub messages: serde_json::Value,
    #[serde(default)]
    pub tools: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsePayload {
    pub text: String,
    pub reasoning_content: String,
    /// Provider-native tool calls (JSON array).
    pub native_tool_calls: serde_json::Value,
}

/// A file content snapshot taken before a write tool touched the file.
/// `content_after == None` on a delete; `None` on a create.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSpanCapture {
    pub path: String,
    pub content_before: String,
    pub content_after: Option<String>,
    pub tool_call_id: String,
    /// Sequence within the turn (matches `llm_io` numbering).
    pub seq: u32,
}

/// Turn-level summary, written once at turn-complete so a list view doesn't
/// have to walk every llm_io JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnMeta {
    pub session_id: String,
    pub turn_seq: u32,
    pub model: String,
    pub total_input_tokens: u32,
    pub total_output_tokens: u32,
    pub at: u64,
    pub llm_call_count: u32,
}

const CAPTURE_LIMIT: u64 = 8 * 1024 * 1024;

fn checked_path(root: &Path, session: &str, turn: u32, reference: &str) -> io::Result<PathBuf> {
    let parts: Vec<_> = reference.split('/').collect();
    if session.is_empty()
        || session.len() > 128
        || !session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || parts.len() != 2
        || !matches!(parts[0], "llm_io" | "spans")
        || !parts[1].ends_with(".json")
        || parts[1].starts_with('.')
        || !parts[1]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid trace reference",
        ));
    }
    let mut path = root.to_owned();
    for part in [session, &turn.to_string(), parts[0], parts[1]] {
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trace symlink forbidden",
            ));
        }
        path.push(part);
    }
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "trace symlink forbidden",
        ));
    }
    Ok(path)
}

fn read_capture<T: serde::de::DeserializeOwned>(
    root: &Path,
    session: &str,
    turn: u32,
    reference: &str,
    kind: &str,
) -> io::Result<T> {
    if !reference.starts_with(&format!("{kind}/")) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "wrong trace kind",
        ));
    }
    let path = checked_path(root, session, turn, reference)?;
    if fs::metadata(&path)?.len() > CAPTURE_LIMIT {
        return Err(io::Error::other("capture exceeds 8 MiB"));
    }
    serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)
}

fn write_capture<T: Serialize>(
    root: &Path,
    session: &str,
    turn: u32,
    reference: &str,
    capture: &T,
) -> io::Result<()> {
    let path = checked_path(root, session, turn, reference)?;
    let json = serde_json::to_vec(capture).map_err(io::Error::other)?;
    if json.len() as u64 > CAPTURE_LIMIT {
        return Err(io::Error::other("capture exceeds 8 MiB"));
    }
    let existing = list_artifacts_in(root, session, turn)?;
    if existing.len() >= 200
        || existing.iter().map(|a| a.size_bytes).sum::<u64>() + json.len() as u64 > 64 * 1024 * 1024
    {
        return Err(io::Error::other("turn trace retention limit reached"));
    }
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for dir in [
            root.to_owned(),
            root.join(session),
            root.join(session).join(turn.to_string()),
            parent.to_owned(),
        ] {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temp = parent.join(format!(
        ".capture-{}-{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = (|| {
        use std::io::Write;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&json)?;
        file.sync_all()?;
        fs::rename(&temp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// Computes `traces/` directory under the kodo state root. Returns `None`
/// when `HOME` is unset (mirrors `session::dir`).
pub fn traces_root() -> Option<PathBuf> {
    Some(settings::state_dir()?.join("traces"))
}

/// The directory that owns one turn's trace artifacts.
pub fn turn_dir(session_id: &str, turn_seq: u32) -> Option<PathBuf> {
    let root = traces_root()?;
    checked_path(&root, session_id, turn_seq, "llm_io/001.json")
        .ok()?
        .parent()?
        .parent()
        .map(Path::to_owned)
}

/// Returns `true` when a turn directory has at least one file under it —
/// cheap probe the GUI can use to decide whether to render the trace tab.
pub fn turn_has_artifacts(session_id: &str, turn_seq: u32) -> bool {
    let Some(dir) = turn_dir(session_id, turn_seq) else {
        return false;
    };
    dir.is_dir()
        && fs::read_dir(&dir)
            .map(|entries| entries.flatten().any(|entry| entry.path().is_dir()))
            .unwrap_or(false)
}

/// Writes the JSON body in one go. Creates the parent directory if it
/// doesn't exist. Returns the relative path the chip
/// (`ItemKind::ModelCall::llm_io_ref`) should store.
pub fn write_llm_io(session_id: &str, turn_seq: u32, capture: &LlmIoCapture) -> io::Result<String> {
    write_llm_io_in(
        &traces_root().ok_or_else(|| io::Error::other("no state dir"))?,
        session_id,
        turn_seq,
        capture,
    )
}

/// Writes a capture into an explicit root directory. Test-only escape hatch:
/// lets hermetic tests skip `HOME` manipulation.
#[doc(hidden)]
pub fn write_llm_io_in(
    root: &Path,
    session_id: &str,
    turn_seq: u32,
    capture: &LlmIoCapture,
) -> io::Result<String> {
    let reference = format!("llm_io/{:03}.json", capture.seq);
    write_capture(root, session_id, turn_seq, &reference, capture)?;
    Ok(reference)
}

/// Reads a captured LLM I/O by chip reference (e.g. `"llm_io/001.json"`).
pub fn read_llm_io(session_id: &str, turn_seq: u32, ref_path: &str) -> io::Result<LlmIoCapture> {
    read_llm_io_in(
        &traces_root().ok_or_else(|| io::Error::other("no state dir"))?,
        session_id,
        turn_seq,
        ref_path,
    )
}

/// Reads a capture from an explicit root directory. Test-only escape hatch.
#[doc(hidden)]
pub fn read_llm_io_in(
    root: &Path,
    session_id: &str,
    turn_seq: u32,
    ref_path: &str,
) -> io::Result<LlmIoCapture> {
    read_capture(root, session_id, turn_seq, ref_path, "llm_io")
}

/// Captures a file content snapshot before a write tool touches it. Same
/// shape as the GUI's `FileSpanModal` payload.
pub fn write_file_span(
    session_id: &str,
    turn_seq: u32,
    span: &FileSpanCapture,
) -> io::Result<String> {
    write_file_span_in(
        &traces_root().ok_or_else(|| io::Error::other("no state dir"))?,
        session_id,
        turn_seq,
        span,
    )
}

/// Same shape as `write_file_span` but takes an explicit root. Test-only.
#[doc(hidden)]
pub fn write_file_span_in(
    root: &Path,
    session_id: &str,
    turn_seq: u32,
    span: &FileSpanCapture,
) -> io::Result<String> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    span.path.hash(&mut hash);
    span.tool_call_id.hash(&mut hash);
    let reference = format!("spans/{:03}-file-{:016x}.json", span.seq, hash.finish());
    write_capture(root, session_id, turn_seq, &reference, span)?;
    Ok(reference)
}

pub fn read_file_span(
    session_id: &str,
    turn_seq: u32,
    ref_path: &str,
) -> io::Result<FileSpanCapture> {
    read_file_span_in(
        &traces_root().ok_or_else(|| io::Error::other("no state dir"))?,
        session_id,
        turn_seq,
        ref_path,
    )
}

/// Reads a span from an explicit root directory. Test-only escape hatch.
#[doc(hidden)]
pub fn read_file_span_in(
    root: &Path,
    session_id: &str,
    turn_seq: u32,
    ref_path: &str,
) -> io::Result<FileSpanCapture> {
    read_capture(root, session_id, turn_seq, ref_path, "spans")
}

/// Lists every artifact under a turn — both `llm_io/*.json` and
/// `spans/*.json`. The GUI uses this to render the trace-page list without
/// having to know the underlying layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub kind: &'static str,
    /// Sequential number within `kind`. `001`, `002`, …
    pub seq: u32,
    /// Wire label the user reads. Path relative to the turn dir.
    pub label: String,
    pub size_bytes: u64,
    /// Seconds since the Unix epoch.
    pub at: u64,
}

pub fn list_artifacts(session_id: &str, turn_seq: u32) -> io::Result<Vec<ArtifactRef>> {
    let Some(root) = traces_root() else {
        return Ok(Vec::new());
    };
    list_artifacts_in(&root, session_id, turn_seq)
}

/// Same listing logic as [`list_artifacts`] but walks an explicit root. Used by
/// hermetic tests so they don't have to flip `HOME`.
#[doc(hidden)]
pub fn list_artifacts_in(
    root: &Path,
    session_id: &str,
    turn_seq: u32,
) -> io::Result<Vec<ArtifactRef>> {
    let mut out = Vec::new();
    for kind in ["llm_io", "spans"] {
        let dir = checked_path(root, session_id, turn_seq, &format!("{kind}/001.json"))?
            .parent()
            .unwrap()
            .to_owned();
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten().take(200) {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() || path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let at = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            out.push(ArtifactRef {
                kind: if kind == "spans" { "file_span" } else { kind },
                seq: stem.split('-').next().unwrap_or("").parse().unwrap_or(0),
                label: format!("{kind}/{stem}.json"),
                size_bytes: meta.len(),
                at,
            });
        }
    }
    out.sort_by_key(|a| (a.kind, a.seq));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn capture(seq: u32, system: &str) -> LlmIoCapture {
        LlmIoCapture {
            seq,
            model: "claude-sonnet-5".into(),
            protocol: "native".into(),
            retry_index: 0,
            provider_latency_ms: 100,
            finish_reason: "stop".into(),
            input_tokens: 12,
            output_tokens: 4,
            saw_reasoning: false,
            at: 1_700_000_000,
            request: RequestPayload {
                system: system.into(),
                messages: serde_json::json!([{"role": "user", "content": "hi"}]),
                tools: serde_json::json!([]),
            },
            response: ResponsePayload {
                text: "你好".into(),
                reasoning_content: String::new(),
                native_tool_calls: serde_json::json!([]),
            },
        }
    }

    #[test]
    fn llm_io_round_trips_through_disk() {
        let tmp = TempDir::new("traces-llm");
        let cap = capture(1, "sys-prompt");
        let _ref_path = write_llm_io_in(tmp.path(), "session-a", 0, &cap).expect("write");
        let back = read_llm_io_in(tmp.path(), "session-a", 0, "llm_io/001.json").expect("read");
        assert_eq!(back.model, "claude-sonnet-5");
        assert_eq!(back.request.system, "sys-prompt");
    }

    #[test]
    fn list_artifacts_is_empty_when_turn_has_no_dir() {
        let tmp = TempDir::new("traces-list");
        let empty = list_artifacts_in(tmp.path(), "nope", 99).expect("list");
        assert!(empty.is_empty());
    }

    #[test]
    fn file_span_round_trips() {
        let tmp = TempDir::new("traces-span");
        let span = FileSpanCapture {
            path: "src/foo.rs".into(),
            content_before: "old".into(),
            content_after: Some("new".into()),
            tool_call_id: "t1".into(),
            seq: 1,
        };
        let r = write_file_span_in(tmp.path(), "s", 0, &span).expect("write");
        let back = read_file_span_in(tmp.path(), "s", 0, &r).expect("read");
        assert_eq!(back.content_before, "old");
        assert_eq!(back.content_after.as_deref(), Some("new"));
    }
    #[test]
    fn trace_references_cannot_escape_or_follow_symlinks_and_spans_are_listed() {
        let tmp = TempDir::new("trace-boundary");
        let cap = capture(1, "synthetic");
        write_llm_io_in(tmp.path(), "s", 0, &cap).unwrap();
        for reference in [
            "../outside.json",
            "/tmp/outside.json",
            "llm_io/../001.json",
            "spans/001.json",
        ] {
            assert!(read_llm_io_in(tmp.path(), "s", 0, reference).is_err());
        }
        assert!(write_llm_io_in(tmp.path(), "../../escape", 0, &cap).is_err());
        let span = FileSpanCapture {
            path: "report.md".into(),
            content_before: "before".into(),
            content_after: Some("after".into()),
            tool_call_id: "a".into(),
            seq: 2,
        };
        write_file_span_in(tmp.path(), "s", 0, &span).unwrap();
        let refs = list_artifacts_in(tmp.path(), "s", 0).unwrap();
        assert_eq!(refs.len(), 2);
        assert!(refs
            .iter()
            .any(|r| r.kind == "llm_io" && r.label == "llm_io/001.json"));
        assert!(refs
            .iter()
            .any(|r| r.kind == "file_span" && r.label.starts_with("spans/002-")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = tmp.path().join("s/0/llm_io/001.json");
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(tmp.path().join("missing"), &path).unwrap();
            assert!(read_llm_io_in(tmp.path(), "s", 0, "llm_io/001.json").is_err());
            assert!(write_llm_io_in(tmp.path(), "s", 0, &cap).is_err());
        }
    }
}
