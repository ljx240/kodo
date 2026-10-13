// P1: `scope::scope_for` must fall back to project-only when the path is
// not a git repo. The fields that the recall path filters on (branch,
// worktree_id, repository_id) stay `None`; project_id is still set so
// `MemoryScope::matches` can keep cross-session recall working.

mod _helpers;

use kodo_memory::scope;

#[test]
fn scope_for_outside_a_git_repo_has_no_branch_or_worktree() {
    let dir = tempfile::Builder::new()
        .prefix("kodo-memory-no-git-")
        .tempdir()
        .unwrap();
    let scope = scope::scope_for(dir.path(), Some("sess-1"), Some("task-1"));
    assert!(scope.project_id.is_some(), "project_id must be set");
    assert!(scope.repository_id.is_none(), "no repo id without git");
    assert!(scope.branch.is_none(), "no branch without git");
    assert!(scope.worktree_id.is_none(), "no worktree without git");
    assert_eq!(scope.session_id.as_deref(), Some("sess-1"));
    assert_eq!(scope.task_id.as_deref(), Some("task-1"));
}

#[test]
fn git_context_from_query_on_a_non_git_path_is_none() {
    let dir = tempfile::Builder::new()
        .prefix("kodo-memory-no-git-ctx-")
        .tempdir()
        .unwrap();
    let query_scope = scope::scope_for(dir.path(), None, None);
    let ctx = scope::git_context_from_query(&query_scope);
    assert!(ctx.is_none(), "no live git context for a non-git path");
}
