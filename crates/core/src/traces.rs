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
//! GUI ships `CaptureConfig::default()` (everything on). When off, no
//! directories are created and every public function here returns the
//! appropriate `None`/`Err` shape so callers can fall back to the chip.

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

/// Computes `traces/` directory under the kodo state root. Returns `None`
/// when `HOME` is unset (mirrors `session::dir`).
pub fn traces_root() -> Option<PathBuf> {
    Some(settings::state_dir()?.join("traces"))
}

/// The directory that owns one turn's trace artifacts.
pub fn turn_dir(session_id: &str, turn_seq: u32) -> Option<PathBuf> {
    let root = traces_root()?;
    Some(root.join(session_id).join(turn_seq.to_string()))
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
pub fn write_llm_io(
    session_id: &str,
    turn_seq: u32,
    capture: &LlmIoCapture,
) -> io::Result<String> {
    let Some(dir) = turn_dir(session_id, turn_seq) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no state dir (HOME unset)",
        ));
    };
    let llm_io_dir = dir.join("llm_io");
    fs::create_dir_all(&llm_io_dir)?;
    let filename = format!("{:03}.json", capture.seq);
    let raw_path = llm_io_dir.join(&filename);

    let json = serde_json::to_vec_pretty(capture).map_err(io::Error::other)?;
    fs::write(&raw_path, &json)?;

    Ok(format!("llm_io/{filename}"))
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
    let dir = root.join(session_id).join(turn_seq.to_string());
    let llm_io_dir = dir.join("llm_io");
    fs::create_dir_all(&llm_io_dir)?;
    let filename = format!("{:03}.json", capture.seq);
    let raw_path = llm_io_dir.join(&filename);
    let json = serde_json::to_vec_pretty(capture).map_err(io::Error::other)?;
    fs::write(&raw_path, &json)?;
    Ok(format!("llm_io/{filename}"))
}

/// Reads a captured LLM I/O by chip reference (e.g. `"llm_io/001.json"`).
pub fn read_llm_io(session_id: &str, turn_seq: u32, ref_path: &str) -> io::Result<LlmIoCapture> {
    let dir = turn_dir(session_id, turn_seq)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no state dir"))?;
    let bytes = fs::read(dir.join(ref_path))?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

/// Reads a capture from an explicit root directory. Test-only escape hatch.
#[doc(hidden)]
pub fn read_llm_io_in(
    root: &Path,
    session_id: &str,
    turn_seq: u32,
    ref_path: &str,
) -> io::Result<LlmIoCapture> {
    let bytes = fs::read(root.join(session_id).join(turn_seq.to_string()).join(ref_path))?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

/// Captures a file content snapshot before a write tool touches it. Same
/// shape as the GUI's `FileSpanModal` payload.
pub fn write_file_span(
    session_id: &str,
    turn_seq: u32,
    span: &FileSpanCapture,
) -> io::Result<String> {
    let dir = turn_dir(session_id, turn_seq)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no state dir"))?;
    let spans_dir = dir.join("spans");
    fs::create_dir_all(&spans_dir)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    span.path.hash(&mut hasher);
    span.tool_call_id.hash(&mut hasher);
    let hash = hasher.finish();
    let filename = format!("{:03}-file-{:016x}.json", span.seq, hash);
    let path = spans_dir.join(&filename);
    let json = serde_json::to_vec_pretty(span).map_err(io::Error::other)?;
    fs::write(&path, &json)?;
    Ok(format!("spans/{filename}"))
}

/// Same shape as `write_file_span` but takes an explicit root. Test-only.
#[doc(hidden)]
pub fn write_file_span_in(
    root: &Path,
    session_id: &str,
    turn_seq: u32,
    span: &FileSpanCapture,
) -> io::Result<String> {
    let dir = root.join(session_id).join(turn_seq.to_string());
    let spans_dir = dir.join("spans");
    fs::create_dir_all(&spans_dir)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    span.path.hash(&mut hasher);
    span.tool_call_id.hash(&mut hasher);
    let hash = hasher.finish();
    let filename = format!("{:03}-file-{:016x}.json", span.seq, hash);
    let path = spans_dir.join(&filename);
    let json = serde_json::to_vec_pretty(span).map_err(io::Error::other)?;
    fs::write(&path, &json)?;
    Ok(format!("spans/{filename}"))
}

pub fn read_file_span(
    session_id: &str,
    turn_seq: u32,
    ref_path: &str,
) -> io::Result<FileSpanCapture> {
    let dir = turn_dir(session_id, turn_seq)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no state dir"))?;
    let bytes = fs::read(dir.join(ref_path))?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

/// Reads a span from an explicit root directory. Test-only escape hatch.
#[doc(hidden)]
pub fn read_file_span_in(
    root: &Path,
    session_id: &str,
    turn_seq: u32,
    ref_path: &str,
) -> io::Result<FileSpanCapture> {
    let bytes = fs::read(root.join(session_id).join(turn_seq.to_string()).join(ref_path))?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
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
    let mut out = Vec::new();
    let Some(dir) = turn_dir(session_id, turn_seq) else {
        return Ok(out);
    };
    let Ok(entries) = fs::read_dir(dir.join("llm_io")) else {
        return Ok(out);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let meta = entry.metadata().ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let at = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let seq = stem.parse::<u32>().unwrap_or(0);
        out.push(ArtifactRef {
            kind: "llm_io",
            seq,
            label: format!("llm_io/{stem}.json"),
            size_bytes: size,
            at,
        });
    }
    out.sort_by_key(|a| (a.kind, a.seq));
    Ok(out)
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
    let dir = root.join(session_id).join(turn_seq.to_string());
    let Ok(entries) = fs::read_dir(dir.join("llm_io")) else {
        return Ok(out);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let meta = entry.metadata().ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let at = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let seq = stem.parse::<u32>().unwrap_or(0);
        out.push(ArtifactRef {
            kind: "llm_io",
            seq,
            label: format!("llm_io/{stem}.json"),
            size_bytes: size,
            at,
        });
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
}