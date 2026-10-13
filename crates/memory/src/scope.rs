//! Scope resolution. Project / repository / worktree identities are derived
//! from on-disk facts; they never come from the user-supplied display name.

use std::path::{Path, PathBuf};

use crate::error::{MemoryError, MemoryResult};
use crate::model::MemoryScope;

/// Stable id for a project root. The current behaviour is the
/// canonicalized absolute path; a future revision can hash it for
/// shorter ids, but path equality is sufficient for the
/// local-first invariant.
pub fn project_id_for(root: &Path) -> MemoryResult<String> {
    let canonical = root
        .canonicalize()
        .map_err(|e| MemoryError::InvalidEvent(format!("project root invalid: {e}")))?;
    Ok(canonical.to_string_lossy().into_owned())
}

/// Resolved Git context. `None` fields when not in a git repo or when git
/// is not on PATH; the rest of the engine must not assume any of these are
/// populated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitContext {
    pub project_root: PathBuf,
    pub repository_id: Option<String>,
    pub worktree_id: Option<String>,
    pub branch: Option<String>,
    pub head_commit: Option<String>,
    pub dirty_paths: Vec<String>,
    pub observed_at: u64,
}

/// True if git is on PATH. Cheap: just a `which`-style check. We do not
/// shell out unless this returns true.
pub fn git_available() -> bool {
    crate::evidence::which("git").is_some()
}

/// Resolve a `MemoryScope` for the given project root. Uses `git` when
/// available; falls back to project-only scope. Never returns an error —
/// all degradations are silent.
pub fn scope_for(root: &Path, session_id: Option<&str>, task_id: Option<&str>) -> MemoryScope {
    let now = crate::store::now_secs();
    let project_id = project_id_for(root).ok();
    let mut scope = MemoryScope {
        project_id,
        session_id: session_id.map(str::to_owned),
        task_id: task_id.map(str::to_owned),
        ..MemoryScope::default()
    };
    if !git_available() {
        return scope;
    }
    if let Ok(ctx) = git_context(root, now) {
        scope.repository_id = ctx.repository_id;
        scope.branch = ctx.branch;
        scope.worktree_id = ctx.worktree_id;
    }
    scope
}

/// Run `git -C <root> rev-parse --show-toplevel` etc. and return a
/// `GitContext`. Failure (no git, no toplevel, git not installed) is
/// represented as a context with all-`None` fields.
pub fn git_context(root: &Path, observed_at: u64) -> MemoryResult<GitContext> {
    let toplevel = git_one_line(root, &["rev-parse", "--show-toplevel"])?;
    let common_dir = git_one_line(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .unwrap_or_else(|_| toplevel.clone());
    let branch = git_one_line(root, &["rev-parse", "--abbrev-ref", "HEAD"]).ok();
    let head_commit = git_one_line(root, &["rev-parse", "HEAD"]).ok();
    let worktree_id =
        git_one_line(root, &["rev-parse", "--path-format=absolute", "--git-dir"]).ok();
    let dirty_paths = git_dirty_paths(root).unwrap_or_default();
    Ok(GitContext {
        project_root: PathBuf::from(toplevel),
        repository_id: Some(common_dir),
        worktree_id,
        branch,
        head_commit,
        dirty_paths,
        observed_at,
    })
}

fn git_dirty_paths(root: &Path) -> MemoryResult<Vec<String>> {
    let output = crate::evidence::run_capture(root, "git", &["status", "--porcelain"])?;
    let mut paths = Vec::new();
    for line in output.lines() {
        // ` XY path` or `XY path` — third field onwards is the path.
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let mut chars = trimmed.chars();
        let _x = chars.next();
        let _y = chars.next();
        let rest = trimmed[2..].trim();
        // Rename/copy lines have `old -> new`; keep both sides.
        if let Some((a, b)) = rest.split_once(" -> ") {
            paths.push(a.trim().to_owned());
            paths.push(b.trim().to_owned());
        } else {
            paths.push(rest.to_owned());
        }
    }
    Ok(paths)
}

fn git_one_line(root: &Path, args: &[&str]) -> MemoryResult<String> {
    let output = crate::evidence::run_capture(root, "git", args)?;
    let line = output.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return Err(MemoryError::InvalidEvent(format!(
            "git {} returned empty",
            args.join(" ")
        )));
    }
    Ok(line.to_owned())
}

/// Resolve a `GitContext` from a `MemoryScope`. Returns `None` if the
/// scope has no `project_id` or the path is not on disk / not a git
/// repository. Failures are silent — a missing Git context is the
/// expected state for test runs and project-less memory.
pub fn git_context_from_query(scope: &crate::model::MemoryScope) -> Option<GitContext> {
    let pid = scope.project_id.as_deref()?;
    let root = std::path::Path::new(pid);
    if !root.is_dir() {
        return None;
    }
    if !git_available() {
        return None;
    }
    git_context(root, crate::store::now_secs()).ok()
}
