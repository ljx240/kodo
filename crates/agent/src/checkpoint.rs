//! Turn change tracking + safe undo that only rolls back Kodo edits.
//!
//! Three states are tracked per file:
//! * **A** — user dirt that existed before the turn (`baseline_dirty`)
//! * **B** — Kodo's edits this turn (`kodo_touched` + baseline/after snapshots)
//! * **C** — user (or another process) edits **after** the turn
//!
//! Undo only restores **B**. It never runs `git reset --hard` / `git checkout .`
//! and never snapshot-overwrites when **C** is detected (hash mismatch vs the
//! recorded after-hash) unless a safe inverse/three-way apply succeeds.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub path: String,
    /// Content hash at capture time (baseline **or** after, depending on use).
    pub content_hash: String,
    /// `None` bytes means the file did not exist at baseline (Kodo created it).
    pub existed: bool,
    pub bytes: Option<Vec<u8>>,
    /// `git status --porcelain` XY token for this path at baseline
    /// (`""` clean/unknown, `" M"` modified, `"??"` untracked, …).
    #[serde(default)]
    pub pre_existing_git: String,
}

/// Why a file could not be safely undone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UndoConflictReason {
    /// Current content hash ≠ recorded after-hash (user/other edit post-turn).
    UserModifiedAfterTurn,
    /// Inverse/three-way apply hit overlapping hunks.
    MergeConflict,
    /// Missing baseline or after snapshot in the changeset.
    MissingSnapshot,
    /// Filesystem read/write error during undo.
    IoError,
}

impl UndoConflictReason {
    pub fn label(&self) -> &'static str {
        match self {
            Self::UserModifiedAfterTurn => "user_modified_after_turn",
            Self::MergeConflict => "merge_conflict",
            Self::MissingSnapshot => "missing_snapshot",
            Self::IoError => "io_error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoConflict {
    pub path: String,
    pub reason: UndoConflictReason,
    pub message: String,
}

/// Result of a safe undo pass: files restored vs files left alone with conflicts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UndoReport {
    pub restored: Vec<String>,
    pub conflicts: Vec<UndoConflict>,
}

impl UndoReport {
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// Per-file undo safety state observed against the live working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UndoFileState {
    /// `current == after` — Kodo's result still on disk; safe snapshot restore.
    Clean,
    /// `current == baseline` — already at pre-Kodo content.
    AlreadyBaseline,
    /// `current` matches neither — try inverse apply or report conflict.
    Diverged,
    /// Expected file missing (or unexpected create).
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TurnChangeSet {
    /// Paths dirty *before* Kodo touched anything (user-owned) — state A.
    pub baseline_dirty: BTreeSet<String>,
    /// Files Kodo wrote/patched this turn — state B.
    pub kodo_touched: BTreeSet<String>,
    /// Baseline snapshots for files Kodo touched (before content/hash).
    pub baselines: BTreeMap<String, FileSnapshot>,
    /// After-Kodo snapshots (content/hash at last `record_kodo_change`).
    #[serde(default)]
    pub afters: BTreeMap<String, FileSnapshot>,
    /// Unified diff of **Kodo delta only** (baseline → after), not user dirt.
    pub diffs: BTreeMap<String, String>,
    pub verified: Option<bool>,
}

impl TurnChangeSet {
    /// Capture git status --porcelain baseline before any Kodo mutation.
    pub fn capture_baseline(project: &Path) -> Self {
        let dirty = git_dirty_paths(project);
        Self {
            baseline_dirty: dirty,
            ..Default::default()
        }
    }

    /// Was this path already dirty before Kodo started?
    pub fn was_pre_existing(&self, path: &str) -> bool {
        self.baseline_dirty.contains(path)
            || self
                .baseline_dirty
                .iter()
                .any(|p| p.ends_with(path) || path.ends_with(p.as_str()))
    }

    /// Snapshot file contents before Kodo mutates it (records pre-existing git state).
    pub fn snapshot_before(&mut self, project: &Path, relative: &str) {
        if self.baselines.contains_key(relative) {
            return;
        }
        let full = project.join(relative);
        let existed = full.is_file();
        let bytes = if existed { fs::read(&full).ok() } else { None };
        let content_hash = bytes
            .as_ref()
            .map(|b| hash_bytes(b))
            .unwrap_or_else(|| "missing".to_owned());
        let pre_existing_git = git_status_line(project, relative);
        self.baselines.insert(
            relative.to_owned(),
            FileSnapshot {
                path: relative.to_owned(),
                content_hash,
                existed,
                bytes,
                pre_existing_git,
            },
        );
    }

    /// Record that Kodo changed `relative`. Refreshes after-hash and the
    /// Kodo-only diff (baseline → current). Multiple calls keep first baseline
    /// and update after to the latest Kodo result.
    pub fn record_kodo_change(&mut self, project: &Path, relative: &str) {
        if !self.baselines.contains_key(relative) {
            self.snapshot_before(project, relative);
        }
        self.kodo_touched.insert(relative.to_owned());
        let before = self
            .baselines
            .get(relative)
            .and_then(|s| s.bytes.clone())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let after_bytes = fs::read(project.join(relative)).ok();
        let after = after_bytes
            .as_ref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        let after_hash = after_bytes
            .as_ref()
            .map(|b| hash_bytes(b))
            .unwrap_or_else(|| "missing".to_owned());
        self.afters.insert(
            relative.to_owned(),
            FileSnapshot {
                path: relative.to_owned(),
                content_hash: after_hash,
                existed: after_bytes.is_some(),
                bytes: after_bytes,
                pre_existing_git: String::new(),
            },
        );
        let diff = unified_diff(relative, &before, &after);
        self.diffs.insert(relative.to_owned(), diff);
    }

    /// Files Kodo changed this turn.
    pub fn kodo_changes(&self) -> Vec<&String> {
        self.kodo_touched.iter().collect()
    }

    /// Files that were dirty before Kodo and Kodo did not touch.
    pub fn user_changes_only(&self) -> Vec<&String> {
        self.baseline_dirty
            .iter()
            .filter(|p| !self.kodo_touched.contains(*p))
            .collect()
    }

    /// Files dirty in both worlds: user already had edits AND Kodo touched them.
    pub fn overlapping(&self) -> Vec<&String> {
        self.kodo_touched
            .iter()
            .filter(|p| self.was_pre_existing(p))
            .collect()
    }

    /// Observe live working-tree state vs recorded after/baseline hashes.
    pub fn undo_file_state(&self, project: &Path, path: &str) -> UndoFileState {
        let Some(after) = self.afters.get(path) else {
            return UndoFileState::Missing;
        };
        let full = project.join(path);
        let Ok(current) = fs::read(&full) else {
            if !after.existed {
                return UndoFileState::AlreadyBaseline;
            }
            return UndoFileState::Missing;
        };
        let current_hash = hash_bytes(&current);
        if current_hash == after.content_hash {
            return UndoFileState::Clean;
        }
        if let Some(base) = self.baselines.get(path) {
            if current_hash == base.content_hash {
                return UndoFileState::AlreadyBaseline;
            }
        }
        UndoFileState::Diverged
    }

    /// Undo only Kodo's changes (state B). Conflicts are collected, not fatal —
    /// successful files are still restored (partial failure is reported).
    pub fn undo_kodo_changes(&self, project: &Path) -> Result<UndoReport, String> {
        let mut report = UndoReport::default();
        for path in &self.kodo_touched {
            match self.undo_one(project, path) {
                Ok(()) => report.restored.push(path.clone()),
                Err(conflict) => report.conflicts.push(conflict),
            }
        }
        Ok(report)
    }

    fn undo_one(&self, project: &Path, path: &str) -> Result<(), UndoConflict> {
        let conflict = |reason: UndoConflictReason, message: String| UndoConflict {
            path: path.to_owned(),
            reason,
            message,
        };
        let Some(baseline) = self.baselines.get(path) else {
            return Err(conflict(
                UndoConflictReason::MissingSnapshot,
                format!("missing baseline snapshot for {path}"),
            ));
        };
        let Some(after) = self.afters.get(path) else {
            return Err(conflict(
                UndoConflictReason::MissingSnapshot,
                format!("missing after snapshot for {path}"),
            ));
        };
        let full = project.join(path);
        match self.undo_file_state(project, path) {
            UndoFileState::AlreadyBaseline => Ok(()),
            UndoFileState::Missing if !baseline.existed => Ok(()),
            UndoFileState::Missing => Err(conflict(
                UndoConflictReason::IoError,
                format!("file missing unexpectedly: {path}"),
            )),
            UndoFileState::Clean => {
                if baseline.existed {
                    let bytes = baseline.bytes.as_ref().ok_or_else(|| {
                        conflict(
                            UndoConflictReason::MissingSnapshot,
                            format!("missing baseline bytes for {path}"),
                        )
                    })?;
                    if let Some(parent) = full.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    fs::write(&full, bytes)
                        .map_err(|e| conflict(UndoConflictReason::IoError, e.to_string()))?;
                } else if full.exists() {
                    // Untracked Kodo-created file; current == after → no user edit.
                    fs::remove_file(&full)
                        .map_err(|e| conflict(UndoConflictReason::IoError, e.to_string()))?;
                }
                Ok(())
            }
            UndoFileState::Diverged => {
                if !baseline.existed {
                    return Err(conflict(
                        UndoConflictReason::UserModifiedAfterTurn,
                        format!(
                            "Kodo-created file `{path}` was edited after the turn; not deleting"
                        ),
                    ));
                }
                let base_txt = String::from_utf8(baseline.bytes.clone().unwrap_or_default())
                    .map_err(|_| {
                        conflict(
                            UndoConflictReason::IoError,
                            format!("baseline for {path} is not UTF-8"),
                        )
                    })?;
                let after_txt = String::from_utf8(after.bytes.clone().unwrap_or_default())
                    .map_err(|_| {
                        conflict(
                            UndoConflictReason::IoError,
                            format!("after for {path} is not UTF-8"),
                        )
                    })?;
                let current_raw = fs::read(&full)
                    .map_err(|e| conflict(UndoConflictReason::IoError, e.to_string()))?;
                let current_txt = String::from_utf8(current_raw).map_err(|_| {
                    conflict(
                        UndoConflictReason::IoError,
                        format!("current {path} is not UTF-8"),
                    )
                })?;
                match reverse_kodo_edit(&base_txt, &after_txt, &current_txt) {
                    Ok(merged) => fs::write(&full, merged)
                        .map_err(|e| conflict(UndoConflictReason::IoError, e.to_string()))
                        .map(|_| ()),
                    Err(msg) => Err(conflict(UndoConflictReason::MergeConflict, msg)),
                }
            }
        }
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    // FNV-1a 64 — no extra dependency.
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Porcelain XY token for one path (`""` if clean/unknown).
fn git_status_line(project: &Path, relative: &str) -> String {
    use crate::process::{ProcessRunner, ProcessSpec, ProcessStatus};
    let spec = ProcessSpec::shell(project, "git status --porcelain")
        .timeout(std::time::Duration::from_secs(5))
        .stdout_limit(32 * 1024)
        .stderr_limit(2 * 1024);
    let outcome = ProcessRunner::run(&spec, &|| true);
    if outcome.status != ProcessStatus::ExitSuccess {
        return String::new();
    }
    for line in outcome.stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        let path = line[3..].trim();
        let path = path.split(" -> ").last().unwrap_or(path).trim();
        if path == relative {
            return line[..2].to_owned();
        }
    }
    String::new()
}

fn git_dirty_paths(project: &Path) -> BTreeSet<String> {
    use crate::process::{ProcessRunner, ProcessSpec, ProcessStatus};
    let spec = ProcessSpec::shell(project, "git status --porcelain")
        .timeout(std::time::Duration::from_secs(10))
        .stdout_limit(64 * 1024)
        .stderr_limit(4 * 1024);
    let outcome = ProcessRunner::run(&spec, &|| true);
    let mut set = BTreeSet::new();
    if outcome.status == ProcessStatus::ExitSuccess {
        for line in outcome.stdout.lines() {
            if line.len() < 4 {
                continue;
            }
            let path = line[3..].trim();
            let path = path.split(" -> ").last().unwrap_or(path).trim().to_owned();
            if !path.is_empty() {
                set.insert(path);
            }
        }
    }
    set
}

/// Reverse Kodo's `base → after` edit onto `current` (may include user edits).
///
/// * `current == after` → return `base` (classic clean undo).
/// * `current == base`  → already undone.
/// * Otherwise apply hunk-level inverse: each Kodo change segment from
///   `base→after` must still appear contiguously in `current`; replace it with
///   the base segment. Context lines keep **current** content (user edits
///   outside Kodo hunks survive). Overlap / missing segment → `Err` conflict.
fn reverse_kodo_edit(base: &str, after: &str, current: &str) -> Result<String, String> {
    if current == after {
        return Ok(base.to_owned());
    }
    if current == base {
        return Ok(base.to_owned());
    }
    let base_lines: Vec<&str> = base.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let cur_lines: Vec<&str> = current.lines().collect();
    let segments = change_segments(&base_lines, &after_lines);

    if segments.is_empty() {
        return Ok(join_lines(&cur_lines));
    }

    // Locate each after-segment in current (ordered).
    let mut subs: Vec<(usize, usize, usize)> = Vec::new(); // cur_start, cur_end, seg_idx
    let mut cursor = 0usize;
    for (idx, seg) in segments.iter().enumerate() {
        let after_seg = &after_lines[seg.after_start..seg.after_end];
        match find_subslice(&cur_lines, after_seg, cursor) {
            Some(pos) => {
                let end = pos + after_seg.len();
                subs.push((pos, end, idx));
                cursor = end;
            }
            None => {
                // Segment gone: allow only if base segment is already present
                // (partial prior undo); otherwise user touched this hunk.
                let base_seg = &base_lines[seg.base_start..seg.base_end];
                if find_subslice(&cur_lines, base_seg, 0).is_none() {
                    return Err(format!(
                        "conflict: hunk {idx} no longer matches Kodo's after-text \
                         (user modified the same region)"
                    ));
                }
            }
        }
    }
    for w in subs.windows(2) {
        if w[0].1 > w[1].0 {
            return Err("overlapping Kodo hunks in current file".into());
        }
    }

    let mut out: Vec<String> = Vec::new();
    let mut ci = 0usize;
    let mut si = 0usize;
    while ci < cur_lines.len() {
        if si < subs.len() && ci == subs[si].0 {
            let seg = &segments[subs[si].2];
            for line in &base_lines[seg.base_start..seg.base_end] {
                out.push((*line).to_owned());
            }
            ci = subs[si].1;
            si += 1;
        } else {
            out.push(cur_lines[ci].to_owned());
            ci += 1;
        }
    }
    Ok(join_lines_owned(&out))
}

fn join_lines(lines: &[&str]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

fn join_lines_owned(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

struct ChangeSeg {
    base_start: usize,
    base_end: usize,
    after_start: usize,
    after_end: usize,
}

/// Maximal differing segments between base and after (aligned via LCS walk).
fn change_segments(base: &[&str], after: &[&str]) -> Vec<ChangeSeg> {
    let n = base.len();
    let m = after.len();
    if base == after {
        return Vec::new();
    }
    if n.saturating_mul(m) > 4_000_000 {
        return vec![ChangeSeg {
            base_start: 0,
            base_end: n,
            after_start: 0,
            after_end: m,
        }];
    }
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if base[i] == after[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut segs = Vec::new();
    let mut i = 0;
    let mut j = 0;
    let mut in_change = false;
    let mut bs = 0;
    let mut as_ = 0;
    while i < n && j < m {
        if base[i] == after[j] {
            if in_change {
                segs.push(ChangeSeg {
                    base_start: bs,
                    base_end: i,
                    after_start: as_,
                    after_end: j,
                });
                in_change = false;
            }
            i += 1;
            j += 1;
        } else {
            if !in_change {
                bs = i;
                as_ = j;
                in_change = true;
            }
            if dp[i + 1][j] >= dp[i][j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    if in_change {
        segs.push(ChangeSeg {
            base_start: bs,
            base_end: n,
            after_start: as_,
            after_end: m,
        });
    } else if i < n || j < m {
        segs.push(ChangeSeg {
            base_start: i,
            base_end: n,
            after_start: j,
            after_end: m,
        });
    }
    segs.retain(|s| s.after_start < s.after_end || s.base_start < s.base_end);
    segs
}

fn find_subslice(hay: &[&str], needle: &[&str], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from);
    }
    if needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// Minimal unified diff (enough for the UI viewer + tests).
pub fn unified_diff(path: &str, before: &str, after: &str) -> String {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    if before == after {
        out.push_str("@@ no changes @@\n");
        return out;
    }
    // LCS-free simple line walk for compact diffs.
    let max = before_lines.len().max(after_lines.len());
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut body = String::new();
    for i in 0..max {
        match (before_lines.get(i), after_lines.get(i)) {
            (Some(b), Some(a)) if b == a => body.push_str(&format!(" {b}\n")),
            (Some(b), Some(a)) => {
                removed += 1;
                added += 1;
                body.push_str(&format!("-{b}\n"));
                body.push_str(&format!("+{a}\n"));
            }
            (Some(b), None) => {
                removed += 1;
                body.push_str(&format!("-{b}\n"));
            }
            (None, Some(a)) => {
                added += 1;
                body.push_str(&format!("+{a}\n"));
            }
            (None, None) => {}
        }
    }
    out.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        before_lines.len(),
        removed,
        after_lines.len(),
        added
    ));
    out.push_str(&body);
    out
}

/// Snapshot directory for file-level backups outside git (optional helper).
pub fn checkpoint_dir(project: &Path) -> PathBuf {
    project.join(".kodo").join("checkpoints")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kodo_cs_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        dir
    }

    fn init_git(dir: &Path) {
        let _ = Command::new("git").args(["init"]).current_dir(dir).output();
        let _ = Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output();
        let _ = Command::new("git")
            .args([
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "init",
            ])
            .current_dir(dir)
            .output();
    }

    #[test]
    fn changeset_clean_file_edit_undo() {
        let dir = temp_project("clean");
        fs::write(dir.join("src/a.rs"), "fn a() {}\n").unwrap();
        init_git(&dir);

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        assert!(cs.baseline_dirty.is_empty());

        cs.snapshot_before(&dir, "src/a.rs");
        assert_eq!(cs.baselines["src/a.rs"].pre_existing_git, "");
        fs::write(dir.join("src/a.rs"), "fn a() { /* kodo */ }\n").unwrap();
        cs.record_kodo_change(&dir, "src/a.rs");

        assert_eq!(cs.kodo_changes().len(), 1);
        assert!(cs.afters.contains_key("src/a.rs"));
        assert_eq!(cs.undo_file_state(&dir, "src/a.rs"), UndoFileState::Clean);

        let report = cs.undo_kodo_changes(&dir).unwrap();
        assert!(report.is_clean(), "conflicts={:?}", report.conflicts);
        assert_eq!(report.restored, vec!["src/a.rs".to_owned()]);
        assert_eq!(
            fs::read_to_string(dir.join("src/a.rs")).unwrap(),
            "fn a() {}\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_dirty_file_edit_undo() {
        let dir = temp_project("dirty");
        fs::write(dir.join("src/a.rs"), "fn a() {}\n").unwrap();
        fs::write(dir.join("src/user.rs"), "fn user() {}\n").unwrap();
        init_git(&dir);
        fs::write(dir.join("src/user.rs"), "fn user() { /* user edit */ }\n").unwrap();

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        assert!(cs.was_pre_existing("src/user.rs"));
        assert_eq!(cs.user_changes_only().len(), 1);

        cs.snapshot_before(&dir, "src/a.rs");
        fs::write(dir.join("src/a.rs"), "fn a() { /* kodo */ }\n").unwrap();
        cs.record_kodo_change(&dir, "src/a.rs");

        assert_eq!(cs.kodo_changes().len(), 1);
        assert!(cs.overlapping().is_empty());

        let report = cs.undo_kodo_changes(&dir).unwrap();
        assert!(report.is_clean());
        assert_eq!(
            fs::read_to_string(dir.join("src/a.rs")).unwrap(),
            "fn a() {}\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("src/user.rs")).unwrap(),
            "fn user() { /* user edit */ }\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_user_and_kodo_modify_different_hunks() {
        let dir = temp_project("hunks");
        let base = "line1\nline2\nline3\nline4\nline5\n";
        fs::write(dir.join("src/m.rs"), base).unwrap();
        init_git(&dir);

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        cs.snapshot_before(&dir, "src/m.rs");
        // Kodo edits line1 only.
        fs::write(dir.join("src/m.rs"), "KODO\nline2\nline3\nline4\nline5\n").unwrap();
        cs.record_kodo_change(&dir, "src/m.rs");
        // User edits line5 after the turn (different hunk).
        fs::write(dir.join("src/m.rs"), "KODO\nline2\nline3\nline4\nUSER\n").unwrap();
        assert_eq!(
            cs.undo_file_state(&dir, "src/m.rs"),
            UndoFileState::Diverged
        );

        let report = cs.undo_kodo_changes(&dir).unwrap();
        assert!(
            report.is_clean(),
            "different hunks must inverse-apply: {:?}",
            report.conflicts
        );
        let after = fs::read_to_string(dir.join("src/m.rs")).unwrap();
        assert_eq!(after, "line1\nline2\nline3\nline4\nUSER\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_same_hunk_conflict() {
        let dir = temp_project("same_hunk");
        fs::write(dir.join("src/m.rs"), "shared\nkeep\n").unwrap();
        init_git(&dir);

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        cs.snapshot_before(&dir, "src/m.rs");
        fs::write(dir.join("src/m.rs"), "KODO_SHARED\nkeep\n").unwrap();
        cs.record_kodo_change(&dir, "src/m.rs");
        // User overwrites the same hunk after the turn.
        fs::write(dir.join("src/m.rs"), "USER_SHARED\nkeep\n").unwrap();

        let report = cs.undo_kodo_changes(&dir).unwrap();
        assert!(!report.is_clean(), "same hunk must conflict");
        assert_eq!(report.restored.len(), 0);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(
            report.conflicts[0].reason,
            UndoConflictReason::MergeConflict
        );
        // User content preserved — no snapshot overwrite.
        assert_eq!(
            fs::read_to_string(dir.join("src/m.rs")).unwrap(),
            "USER_SHARED\nkeep\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_user_edits_after_completion_snapshot_not_overwritten() {
        let dir = temp_project("after_turn");
        fs::write(dir.join("src/t.rs"), "before\n").unwrap();
        init_git(&dir);

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        cs.snapshot_before(&dir, "src/t.rs");
        fs::write(dir.join("src/t.rs"), "before\nkodo\n").unwrap();
        cs.record_kodo_change(&dir, "src/t.rs");

        // User appends after completion — diverged; if merge fails, conflict;
        // if merge succeeds (append is outside kodo hunk), user line stays.
        fs::write(dir.join("src/t.rs"), "before\nkodo\nuser-append\n").unwrap();
        let report = cs.undo_kodo_changes(&dir).unwrap();
        let text = fs::read_to_string(dir.join("src/t.rs")).unwrap();
        // User append must never be lost.
        assert!(text.contains("user-append"), "lost user edit: {text:?}");
        if report.is_clean() {
            // Kodo line removed via inverse; user append kept.
            assert!(
                !text.contains("kodo\n"),
                "kodo hunk should reverse: {text:?}"
            );
        } else {
            assert_eq!(
                report.conflicts[0].reason,
                UndoConflictReason::MergeConflict
            );
            assert_eq!(text, "before\nkodo\nuser-append\n");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_kodo_created_file_later_edited_by_user() {
        let dir = temp_project("created_edit");
        fs::write(dir.join("src/keep.rs"), "x\n").unwrap();
        init_git(&dir);

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        cs.snapshot_before(&dir, "src/new.rs");
        assert!(!cs.baselines["src/new.rs"].existed);
        fs::write(dir.join("src/new.rs"), "created by kodo\n").unwrap();
        cs.record_kodo_change(&dir, "src/new.rs");
        // User edits the new file after the turn.
        fs::write(dir.join("src/new.rs"), "created by kodo\nuser note\n").unwrap();

        let report = cs.undo_kodo_changes(&dir).unwrap();
        assert!(!report.is_clean());
        assert_eq!(
            report.conflicts[0].reason,
            UndoConflictReason::UserModifiedAfterTurn
        );
        // File must still exist with user's edit — never deleted.
        assert!(dir.join("src/new.rs").exists());
        assert!(fs::read_to_string(dir.join("src/new.rs"))
            .unwrap()
            .contains("user note"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_kodo_created_file_unchanged_is_deleted() {
        let dir = temp_project("created_clean");
        fs::write(dir.join("src/keep.rs"), "x\n").unwrap();
        init_git(&dir);

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        cs.snapshot_before(&dir, "src/new.rs");
        fs::write(dir.join("src/new.rs"), "created by kodo\n").unwrap();
        cs.record_kodo_change(&dir, "src/new.rs");

        let report = cs.undo_kodo_changes(&dir).unwrap();
        assert!(report.is_clean(), "{:?}", report.conflicts);
        assert!(
            !dir.join("src/new.rs").exists(),
            "unmodified create must delete"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_multiple_kodo_edits_same_file() {
        let dir = temp_project("multi_edit");
        fs::write(dir.join("src/a.rs"), "v1\n").unwrap();
        init_git(&dir);
        let mut cs = TurnChangeSet::capture_baseline(&dir);

        cs.snapshot_before(&dir, "src/a.rs");
        fs::write(dir.join("src/a.rs"), "v2\n").unwrap();
        cs.record_kodo_change(&dir, "src/a.rs");
        fs::write(dir.join("src/a.rs"), "v3\n").unwrap();
        cs.record_kodo_change(&dir, "src/a.rs");

        assert_eq!(cs.kodo_changes().len(), 1);
        assert_eq!(cs.undo_file_state(&dir, "src/a.rs"), UndoFileState::Clean);
        let report = cs.undo_kodo_changes(&dir).unwrap();
        assert!(report.is_clean());
        assert_eq!(fs::read_to_string(dir.join("src/a.rs")).unwrap(), "v1\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_partial_failure_still_undoes_written_files() {
        let dir = temp_project("partial");
        fs::write(dir.join("src/a.rs"), "a\n").unwrap();
        fs::write(dir.join("src/b.rs"), "b\n").unwrap();
        init_git(&dir);
        let mut cs = TurnChangeSet::capture_baseline(&dir);

        cs.snapshot_before(&dir, "src/a.rs");
        fs::write(dir.join("src/a.rs"), "a-mod\n").unwrap();
        cs.record_kodo_change(&dir, "src/a.rs");

        cs.snapshot_before(&dir, "src/b.rs");
        fs::write(dir.join("src/b.rs"), "b-mod\n").unwrap();
        cs.record_kodo_change(&dir, "src/b.rs");

        // User clobbers b after turn with same-hunk change → conflict on b only.
        fs::write(dir.join("src/b.rs"), "user-b\n").unwrap();

        let report = cs.undo_kodo_changes(&dir).unwrap();
        assert_eq!(report.restored, vec!["src/a.rs".to_owned()]);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].path, "src/b.rs");
        // a restored, b user content kept.
        assert_eq!(fs::read_to_string(dir.join("src/a.rs")).unwrap(), "a\n");
        assert_eq!(
            fs::read_to_string(dir.join("src/b.rs")).unwrap(),
            "user-b\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_diff_is_kodo_delta_not_user_baseline_dirt() {
        let dir = temp_project("diff_delta");
        fs::write(dir.join("src/s.rs"), "git clean\n").unwrap();
        init_git(&dir);
        fs::write(dir.join("src/s.rs"), "git clean\nuser dirt\n").unwrap();

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        cs.snapshot_before(&dir, "src/s.rs");
        // Kodo adds one line after user's dirt.
        fs::write(dir.join("src/s.rs"), "git clean\nuser dirt\nkodo\n").unwrap();
        cs.record_kodo_change(&dir, "src/s.rs");

        let diff = cs.diffs.get("src/s.rs").unwrap();
        assert!(diff.contains("+kodo"), "diff must show Kodo delta: {diff}");
        assert!(
            !diff.contains("-user dirt") && !diff.contains("+user dirt"),
            "user baseline dirt must not appear as Kodo delta: {diff}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_same_file_already_dirty_undo_restores_user_version() {
        let dir = temp_project("overlap");
        fs::write(dir.join("src/shared.rs"), "original\n").unwrap();
        init_git(&dir);
        fs::write(dir.join("src/shared.rs"), "user dirty\n").unwrap();

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        assert!(cs.was_pre_existing("src/shared.rs"));
        cs.snapshot_before(&dir, "src/shared.rs");
        fs::write(dir.join("src/shared.rs"), "kodo overwrite\n").unwrap();
        cs.record_kodo_change(&dir, "src/shared.rs");
        assert_eq!(cs.overlapping().len(), 1);

        let report = cs.undo_kodo_changes(&dir).unwrap();
        assert!(report.is_clean(), "{:?}", report.conflicts);
        assert_eq!(
            fs::read_to_string(dir.join("src/shared.rs")).unwrap(),
            "user dirty\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_undo_after_verification_failure() {
        let dir = temp_project("verify_fail");
        fs::write(dir.join("src/lib.rs"), "ok\n").unwrap();
        init_git(&dir);
        let mut cs = TurnChangeSet::capture_baseline(&dir);
        cs.snapshot_before(&dir, "src/lib.rs");
        fs::write(dir.join("src/lib.rs"), "broken\n").unwrap();
        cs.record_kodo_change(&dir, "src/lib.rs");
        cs.verified = Some(false);
        let report = cs.undo_kodo_changes(&dir).unwrap();
        assert!(report.is_clean());
        assert_eq!(fs::read_to_string(dir.join("src/lib.rs")).unwrap(), "ok\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changeset_diff_marks_added_and_deleted_lines() {
        let diff = unified_diff("src/a.rs", "one\ntwo\nthree\n", "one\nTWO\nthree\nfour\n");
        assert!(diff.contains("--- a/src/a.rs"));
        assert!(diff.contains("-two"));
        assert!(diff.contains("+TWO"));
        assert!(diff.contains("+four"));
    }
}
