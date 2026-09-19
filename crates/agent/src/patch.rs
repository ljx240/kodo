//! Precise patch edits with preconditions and real line diffs.
//!
//! Primary edit protocol: `apply_patch` / `replace_range` / `create_file` /
//! `delete_file`. Full-file `write_file` remains for large rewrites only.

use std::fs;
use std::path::{Path, PathBuf};

use crate::protocol::{ToolError, ToolErrorCode};
use crate::tools::resolve_in_project;

/// Result of a successful edit (before protocol ToolResult wrapping).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchOutcome {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    /// Human summary for notes / ToolResult.output.
    pub summary: String,
    pub existed_before: bool,
}

/// Exact context found at most once for a safe patch.
#[derive(Debug, Clone)]
pub struct ApplyPatchArgs {
    pub path: String,
    /// Exact bytes currently in the file (precondition).
    pub old: String,
    /// Replacement text.
    pub new: String,
    /// Optional 1-based line where `old` is expected (disambiguates repeats).
    pub start_line: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ReplaceRangeArgs {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub new_text: String,
}

/// Real LCS line diff: returns (added, removed) counts.
/// Accurate for repeated lines (unlike HashSet set-difference).
pub fn line_diff_counts(old: &str, new: &str) -> (u32, u32) {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let n = a.len();
    let m = b.len();
    if n == 0 && m == 0 {
        return (0, 0);
    }
    if n == 0 {
        return (m as u32, 0);
    }
    if m == 0 {
        return (0, n as u32);
    }

    // LCS length table — O(n*m); files here are project sources, fine for phase 1.
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            if a[i] == b[j] {
                dp[i + 1][j + 1] = dp[i][j] + 1;
            } else {
                dp[i + 1][j + 1] = dp[i][j + 1].max(dp[i + 1][j]);
            }
        }
    }
    let lcs = dp[n][m] as u32;
    let added = (m as u32).saturating_sub(lcs);
    let removed = (n as u32).saturating_sub(lcs);
    (added, removed)
}

/// Find all non-overlapping occurrences of `needle` in `haystack`.
fn find_all(haystack: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(pos) = haystack[from..].find(needle) {
        let abs = from + pos;
        out.push(abs);
        from = abs + needle.len().max(1);
        if from >= haystack.len() {
            break;
        }
    }
    out
}

fn conflict(message: impl Into<String>) -> ToolError {
    ToolError::new(ToolErrorCode::PatchConflict, message)
}

/// Apply a context precondition patch. Never overwrites blindly:
/// if `old` is missing or ambiguous → structured `PatchConflict`.
pub fn apply_patch(project: &Path, args: &ApplyPatchArgs) -> Result<PatchOutcome, ToolError> {
    let full = resolve_in_project(project, &args.path).map_err(|e| {
        if e.contains("escape") || e.contains("absolute") {
            ToolError::path_escape(e)
        } else {
            ToolError::invalid_args(e)
        }
    })?;

    if args.old.is_empty() {
        return Err(ToolError::invalid_args(
            "apply_patch requires non-empty `old` context",
        ));
    }

    let old_content = if full.is_file() {
        fs::read_to_string(&full)
            .map_err(|e| conflict(format!("re-read required: cannot open {}: {e}", args.path)))?
    } else {
        return Err(conflict(format!(
            "patch conflict: file does not exist: {} (re-read the tree; use create_file)",
            args.path
        )));
    };

    // Reject binary-ish content.
    if old_content.bytes().take(4096).any(|b| b == 0) {
        return Err(ToolError::execution(format!(
            "binary file refused: {}",
            args.path
        )));
    }

    let mut hits = find_all(&old_content, &args.old);
    if let Some(expected) = args.start_line {
        // Keep only the occurrence that starts on the expected line (exact).
        let mut filtered = Vec::new();
        for pos in hits {
            let line = old_content[..pos].matches('\n').count() + 1;
            if line == expected {
                filtered.push(pos);
            }
        }
        hits = filtered;
    }

    match hits.len() {
        0 => Err(conflict(format!(
            "patch conflict: context not found in {} (file may have changed — re-read before retrying)",
            args.path
        ))),
        1 => {
            let pos = hits[0];
            let mut new_content = String::with_capacity(
                old_content.len() - args.old.len() + args.new.len(),
            );
            new_content.push_str(&old_content[..pos]);
            new_content.push_str(&args.new);
            new_content.push_str(&old_content[pos + args.old.len()..]);

            let (added, removed) = line_diff_counts(&old_content, &new_content);
            write_checked(&full, &new_content)?;
            Ok(PatchOutcome {
                path: args.path.clone(),
                added,
                removed,
                summary: format!("patched {} (+{added} -{removed})", args.path),
                existed_before: true,
            })
        }
        n => Err(conflict(format!(
            "patch conflict: context matched {n} times in {} — disambiguate with start_line or more context (re-read)",
            args.path
        ))),
    }
}

/// Replace an inclusive 1-based line range with `new_text`.
pub fn replace_range(project: &Path, args: &ReplaceRangeArgs) -> Result<PatchOutcome, ToolError> {
    let full = resolve_in_project(project, &args.path).map_err(|e| {
        if e.contains("escape") || e.contains("absolute") {
            ToolError::path_escape(e)
        } else {
            ToolError::invalid_args(e)
        }
    })?;
    if !full.is_file() {
        return Err(conflict(format!(
            "patch conflict: file does not exist: {} (re-read; use create_file)",
            args.path
        )));
    }
    let old_content =
        fs::read_to_string(&full).map_err(|e| conflict(format!("re-read required: {e}")))?;
    if old_content.bytes().take(4096).any(|b| b == 0) {
        return Err(ToolError::execution(format!(
            "binary file refused: {}",
            args.path
        )));
    }

    let lines: Vec<&str> = old_content.lines().collect();
    let total = lines.len();
    if total == 0 {
        return Err(conflict(format!(
            "patch conflict: {} is empty — cannot replace_range",
            args.path
        )));
    }
    if args.start_line == 0 || args.end_line == 0 {
        return Err(ToolError::invalid_args("start_line/end_line are 1-based"));
    }
    if args.start_line > args.end_line {
        return Err(ToolError::invalid_args("start_line must be <= end_line"));
    }
    if args.start_line > total {
        return Err(conflict(format!(
            "patch conflict: start_line {} beyond EOF ({total}) in {} — re-read",
            args.start_line, args.path
        )));
    }
    let end = args.end_line.min(total);
    if args.end_line > total {
        // Clamp but note: still a conflict-style guard if wildly off
        if args.end_line > total && args.start_line > total {
            return Err(conflict("range outside file"));
        }
    }

    let mut new_lines: Vec<String> = lines[..args.start_line - 1]
        .iter()
        .map(|s| s.to_string())
        .collect();
    // new_text may be multi-line; empty means delete the range
    if !args.new_text.is_empty() {
        for l in args.new_text.lines() {
            new_lines.push(l.to_owned());
        }
    }
    for l in &lines[end..] {
        new_lines.push((*l).to_owned());
    }

    let mut new_content = new_lines.join("\n");
    if (old_content.ends_with('\n') || !new_content.is_empty())
        && !new_content.ends_with('\n') {
            new_content.push('\n');
        }

    let (added, removed) = line_diff_counts(&old_content, &new_content);
    write_checked(&full, &new_content)?;
    Ok(PatchOutcome {
        path: args.path.clone(),
        added,
        removed,
        summary: format!(
            "replaced {}:{}-{} (+{added} -{removed})",
            args.path, args.start_line, end
        ),
        existed_before: true,
    })
}

/// Create a new file (fails if it already exists — use apply_patch to edit).
pub fn create_file(project: &Path, path: &str, content: &str) -> Result<PatchOutcome, ToolError> {
    let full = resolve_in_project(project, path).map_err(|e| {
        if e.contains("escape") || e.contains("absolute") {
            ToolError::path_escape(e)
        } else {
            ToolError::invalid_args(e)
        }
    })?;
    if full.exists() {
        return Err(conflict(format!(
            "patch conflict: {path} already exists — use apply_patch/replace_range (re-read)"
        )));
    }
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|e| ToolError::execution(e.to_string()))?;
    }
    fs::write(&full, content).map_err(|e| ToolError::execution(e.to_string()))?;
    let added = content.lines().count() as u32;
    Ok(PatchOutcome {
        path: path.to_owned(),
        added,
        removed: 0,
        summary: format!("created {path} (+{added})"),
        existed_before: false,
    })
}

/// Delete a project file. Caller must enforce permission approval first.
pub fn delete_file(project: &Path, path: &str) -> Result<PatchOutcome, ToolError> {
    let full: PathBuf = resolve_in_project(project, path).map_err(|e| {
        if e.contains("escape") || e.contains("absolute") {
            ToolError::path_escape(e)
        } else {
            ToolError::invalid_args(e)
        }
    })?;
    if !full.is_file() {
        return Err(conflict(format!("patch conflict: {path} does not exist")));
    }
    let old = fs::read_to_string(&full).unwrap_or_default();
    let removed = old.lines().count() as u32;
    fs::remove_file(&full).map_err(|e| ToolError::execution(e.to_string()))?;
    Ok(PatchOutcome {
        path: path.to_owned(),
        added: 0,
        removed,
        summary: format!("deleted {path} (-{removed})"),
        existed_before: true,
    })
}

fn write_checked(full: &Path, content: &str) -> Result<(), ToolError> {
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|e| ToolError::execution(e.to_string()))?;
    }
    fs::write(full, content).map_err(|e| ToolError::execution(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kodo-patch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        dir
    }

    #[test]
    fn line_diff_counts_repeated_lines_accurately() {
        // File with many identical lines — HashSet approach would collapse them.
        let old = "x\nx\nx\nx\ny\n";
        let new = "x\nx\nx\nx\nz\n";
        let (added, removed) = line_diff_counts(old, new);
        assert_eq!((added, removed), (1, 1));

        let old = "a\nb\na\nb\na\n";
        let new = "a\nb\na\nb\na\nb\n";
        let (added, removed) = line_diff_counts(old, new);
        assert_eq!((added, removed), (1, 0));
    }

    #[test]
    fn small_patch_changes_only_target_lines() {
        let root = fixture();
        fs::write(root.join("src/lib.rs"), "fn a() {}\nfn b() {}\nfn c() {}\n").unwrap();
        let out = apply_patch(
            &root,
            &ApplyPatchArgs {
                path: "src/lib.rs".into(),
                old: "fn b() {}".into(),
                new: "fn b() { /* fixed */ }".into(),
                start_line: None,
            },
        )
        .expect("patch");
        assert_eq!(out.added, 1);
        assert_eq!(out.removed, 1);
        assert!(out.existed_before);
        let text = fs::read_to_string(root.join("src/lib.rs")).unwrap();
        assert_eq!(text, "fn a() {}\nfn b() { /* fixed */ }\nfn c() {}\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_context_conflicts_without_write() {
        let root = fixture();
        let original = "line1\nline2\nline3\n";
        fs::write(root.join("src/a.txt"), original).unwrap();
        // File changed after model took a stale snapshot.
        fs::write(root.join("src/a.txt"), "line1\nCHANGED\nline3\n").unwrap();
        let err = apply_patch(
            &root,
            &ApplyPatchArgs {
                path: "src/a.txt".into(),
                old: "line2".into(),
                new: "line2b".into(),
                start_line: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ToolErrorCode::PatchConflict);
        assert!(err.message.contains("re-read"), "{}", err.message);
        let after = fs::read_to_string(root.join("src/a.txt")).unwrap();
        assert_eq!(after, "line1\nCHANGED\nline3\n", "must not overwrite");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ambiguous_context_requires_disambiguation() {
        let root = fixture();
        fs::write(root.join("src/dup.txt"), "same\nsame\nsame\n").unwrap();
        let err = apply_patch(
            &root,
            &ApplyPatchArgs {
                path: "src/dup.txt".into(),
                old: "same".into(),
                new: "SAME".into(),
                start_line: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ToolErrorCode::PatchConflict);
        assert!(err.message.contains("3 times") || err.message.contains("matched"));

        // With start_line it succeeds on the first line only.
        let out = apply_patch(
            &root,
            &ApplyPatchArgs {
                path: "src/dup.txt".into(),
                old: "same".into(),
                new: "SAME".into(),
                start_line: Some(1),
            },
        )
        .expect("disambiguated");
        assert_eq!(out.added, 1);
        assert_eq!(out.removed, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn replace_range_edits_exact_lines() {
        let root = fixture();
        fs::write(root.join("src/b.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        let out = replace_range(
            &root,
            &ReplaceRangeArgs {
                path: "src/b.txt".into(),
                start_line: 2,
                end_line: 3,
                new_text: "TWO\nTHREE".into(),
            },
        )
        .expect("replace");
        assert_eq!((out.added, out.removed), (2, 2));
        let text = fs::read_to_string(root.join("src/b.txt")).unwrap();
        assert_eq!(text, "one\nTWO\nTHREE\nfour\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn replace_range_stale_line_conflicts() {
        let root = fixture();
        fs::write(root.join("src/c.txt"), "only\n").unwrap();
        let err = replace_range(
            &root,
            &ReplaceRangeArgs {
                path: "src/c.txt".into(),
                start_line: 10,
                end_line: 12,
                new_text: "x".into(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ToolErrorCode::PatchConflict);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn create_and_delete_file() {
        let root = fixture();
        let created = create_file(&root, "src/new.rs", "pub fn x() {}\n").expect("create");
        assert!(!created.existed_before);
        assert_eq!(created.added, 1);
        let err = create_file(&root, "src/new.rs", "x").unwrap_err();
        assert_eq!(err.code, ToolErrorCode::PatchConflict);
        let deleted = delete_file(&root, "src/new.rs").expect("delete");
        assert_eq!(deleted.removed, 1);
        assert!(!root.join("src/new.rs").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn path_escape_rejected() {
        let root = fixture();
        let err = apply_patch(
            &root,
            &ApplyPatchArgs {
                path: "../evil.txt".into(),
                old: "a".into(),
                new: "b".into(),
                start_line: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ToolErrorCode::PathEscape);
        let _ = fs::remove_dir_all(&root);
    }
}
