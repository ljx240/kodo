//! Turn change tracking + minimal checkpoint/undo that only rolls back Kodo edits.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub path: String,
    /// `None` means the file did not exist at baseline (Kodo created it).
    pub content_hash: String,
    pub existed: bool,
    pub bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TurnChangeSet {
    /// Paths dirty *before* Kodo touched anything (user-owned).
    pub baseline_dirty: BTreeSet<String>,
    /// Files Kodo wrote/patched this turn.
    pub kodo_touched: BTreeSet<String>,
    /// Baseline snapshots for files Kodo touched (for undo).
    pub baselines: BTreeMap<String, FileSnapshot>,
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

    /// Snapshot file contents before Kodo mutates it.
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
        self.baselines.insert(
            relative.to_owned(),
            FileSnapshot {
                path: relative.to_owned(),
                content_hash,
                existed,
                bytes,
            },
        );
    }

    /// Record that Kodo changed `relative`. Never records user pre-existing dirt.
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
        let after = fs::read_to_string(project.join(relative)).unwrap_or_default();
        let diff = unified_diff(relative, &before, &after);
        self.diffs.insert(relative.to_owned(), diff);
    }

    /// Files Kodo changed this turn (excludes pre-existing user dirt).
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

    /// Undo only Kodo's changes. User pre-existing edits are restored from
    /// the pre-Kodo snapshot of that file (which already included user dirt).
    pub fn undo_kodo_changes(&self, project: &Path) -> Result<Vec<String>, String> {
        let mut restored = Vec::new();
        for path in &self.kodo_touched {
            let snapshot = self
                .baselines
                .get(path)
                .ok_or_else(|| format!("missing baseline snapshot for {path}"))?;
            let full = project.join(path);
            if snapshot.existed {
                let bytes = snapshot
                    .bytes
                    .as_ref()
                    .ok_or_else(|| format!("missing baseline bytes for {path}"))?;
                fs::write(&full, bytes).map_err(|e| e.to_string())?;
            } else if full.exists() {
                fs::remove_file(&full).map_err(|e| e.to_string())?;
            }
            restored.push(path.clone());
        }
        Ok(restored)
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

fn git_dirty_paths(project: &Path) -> BTreeSet<String> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(project)
        .output();
    let mut set = BTreeSet::new();
    if let Ok(out) = output {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if line.len() < 4 {
                continue;
            }
            // "XY path" or "XY origin -> path"
            let path = line[3..].trim();
            let path = path.split(" -> ").last().unwrap_or(path).trim().to_owned();
            if !path.is_empty() {
                set.insert(path);
            }
        }
    }
    set
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
    fn clean_repo_undo_only_kodo_changes() {
        let dir = temp_project("clean");
        fs::write(dir.join("src/a.rs"), "fn a() {}\n").unwrap();
        init_git(&dir);

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        assert!(cs.baseline_dirty.is_empty());

        cs.snapshot_before(&dir, "src/a.rs");
        fs::write(dir.join("src/a.rs"), "fn a() { /* kodo */ }\n").unwrap();
        cs.record_kodo_change(&dir, "src/a.rs");

        assert_eq!(cs.kodo_changes().len(), 1);
        assert_eq!(cs.kodo_changes()[0].as_str(), "src/a.rs");
        assert!(cs.user_changes_only().is_empty());

        let restored = cs.undo_kodo_changes(&dir).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].as_str(), "src/a.rs");
        assert_eq!(
            fs::read_to_string(dir.join("src/a.rs")).unwrap(),
            "fn a() {}\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dirty_repo_user_changes_are_not_kodo_changes() {
        let dir = temp_project("dirty");
        fs::write(dir.join("src/a.rs"), "fn a() {}\n").unwrap();
        fs::write(dir.join("src/user.rs"), "fn user() {}\n").unwrap();
        init_git(&dir);
        // User dirt before Kodo
        fs::write(dir.join("src/user.rs"), "fn user() { /* user edit */ }\n").unwrap();

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        assert!(cs.was_pre_existing("src/user.rs"));
        assert_eq!(cs.user_changes_only().len(), 1);
        assert_eq!(cs.user_changes_only()[0].as_str(), "src/user.rs");

        // Kodo edits a clean file
        cs.snapshot_before(&dir, "src/a.rs");
        fs::write(dir.join("src/a.rs"), "fn a() { /* kodo */ }\n").unwrap();
        cs.record_kodo_change(&dir, "src/a.rs");

        assert_eq!(cs.kodo_changes().len(), 1);
        assert_eq!(cs.kodo_changes()[0].as_str(), "src/a.rs");
        assert_eq!(cs.user_changes_only().len(), 1);
        assert!(cs.overlapping().is_empty());

        // Undo Kodo — user file untouched
        cs.undo_kodo_changes(&dir).unwrap();
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
    fn same_file_already_dirty_undo_restores_user_version() {
        let dir = temp_project("overlap");
        fs::write(dir.join("src/shared.rs"), "original\n").unwrap();
        init_git(&dir);
        fs::write(dir.join("src/shared.rs"), "user dirty\n").unwrap();

        let mut cs = TurnChangeSet::capture_baseline(&dir);
        assert!(cs.was_pre_existing("src/shared.rs"));
        cs.snapshot_before(&dir, "src/shared.rs");
        // Kodo overwrites — baseline already includes user's "user dirty"
        fs::write(dir.join("src/shared.rs"), "kodo overwrite\n").unwrap();
        cs.record_kodo_change(&dir, "src/shared.rs");
        assert_eq!(cs.overlapping().len(), 1);
        assert_eq!(cs.overlapping()[0].as_str(), "src/shared.rs");

        cs.undo_kodo_changes(&dir).unwrap();
        // Undo restores the pre-Kodo snapshot (user's edit), not git HEAD.
        assert_eq!(
            fs::read_to_string(dir.join("src/shared.rs")).unwrap(),
            "user dirty\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn multiple_edits_and_created_file_undo() {
        let dir = temp_project("multi");
        fs::write(dir.join("src/a.rs"), "a\n").unwrap();
        init_git(&dir);
        let mut cs = TurnChangeSet::capture_baseline(&dir);

        cs.snapshot_before(&dir, "src/a.rs");
        fs::write(dir.join("src/a.rs"), "a2\n").unwrap();
        cs.record_kodo_change(&dir, "src/a.rs");

        cs.snapshot_before(&dir, "src/new.rs");
        fs::write(dir.join("src/new.rs"), "new\n").unwrap();
        cs.record_kodo_change(&dir, "src/new.rs");

        assert_eq!(cs.kodo_changes().len(), 2);
        cs.undo_kodo_changes(&dir).unwrap();
        assert_eq!(fs::read_to_string(dir.join("src/a.rs")).unwrap(), "a\n");
        assert!(!dir.join("src/new.rs").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_after_verification_failure() {
        let dir = temp_project("verify_fail");
        fs::write(dir.join("src/lib.rs"), "ok\n").unwrap();
        init_git(&dir);
        let mut cs = TurnChangeSet::capture_baseline(&dir);
        cs.snapshot_before(&dir, "src/lib.rs");
        fs::write(dir.join("src/lib.rs"), "broken\n").unwrap();
        cs.record_kodo_change(&dir, "src/lib.rs");
        cs.verified = Some(false);
        assert!(!cs.kodo_touched.is_empty());
        cs.undo_kodo_changes(&dir).unwrap();
        assert_eq!(fs::read_to_string(dir.join("src/lib.rs")).unwrap(), "ok\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_failure_still_undoes_written_files() {
        let dir = temp_project("partial");
        fs::write(dir.join("src/a.rs"), "a\n").unwrap();
        init_git(&dir);
        let mut cs = TurnChangeSet::capture_baseline(&dir);
        cs.snapshot_before(&dir, "src/a.rs");
        fs::write(dir.join("src/a.rs"), "a-mod\n").unwrap();
        cs.record_kodo_change(&dir, "src/a.rs");
        // Second write "failed" — never recorded, undo only what Kodo wrote.
        cs.undo_kodo_changes(&dir).unwrap();
        assert_eq!(fs::read_to_string(dir.join("src/a.rs")).unwrap(), "a\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_marks_added_and_deleted_lines() {
        let diff = unified_diff("src/a.rs", "one\ntwo\nthree\n", "one\nTWO\nthree\nfour\n");
        assert!(diff.contains("--- a/src/a.rs"));
        assert!(diff.contains("-two"));
        assert!(diff.contains("+TWO"));
        assert!(diff.contains("+four"));
    }
}
