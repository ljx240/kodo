// P1: a memory written under worktree A's `repository_id` must not be
// recalled by a query that anchors to worktree B, even when both worktrees
// share a `project_id` and a `repository_id`. The hard filter is on
// `worktree_id`; without it the recall would leak unmerged changes.

mod _helpers;

use std::process::Command;

use kodo_memory::scope::GitContext;
use kodo_memory::validity::ValidityInput;
use kodo_memory::{MemoryQuery, MemoryScope, SqliteMemory, ValidityLabel};

fn run(cmd: &str, args: &[&str], cwd: &std::path::Path) {
    let status = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .status()
        .expect(cmd);
    assert!(
        status.success(),
        "{cmd} {args:?} failed in {}",
        cwd.display()
    );
}

fn init_repo(root: &std::path::Path) {
    run("git", &["init", "--initial-branch=main"], root);
    run("git", &["config", "user.email", "kodo@test"], root);
    run("git", &["config", "user.name", "kodo"], root);
    // Disable the committer date check that gets in the way of empty commits.
    run("git", &["commit", "--allow-empty", "-m", "init"], root);
}

fn worktree_path(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    root.join("..")
        .join(format!("{name}-{}", std::process::id()))
}

#[test]
fn worktree_isolation_denies_recall_from_other_worktree() {
    let dir = tempfile::Builder::new()
        .prefix("kodo-memory-wt-")
        .tempdir()
        .unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    // Make two worktrees under the same repo. The first one becomes A.
    let wt_a = worktree_path(&repo, "wt-A");
    let wt_b = worktree_path(&repo, "wt-B");
    run(
        "git",
        &["worktree", "add", "-b", "wt-a", wt_a.to_str().unwrap()],
        &repo,
    );
    run(
        "git",
        &["worktree", "add", "-b", "wt-b", wt_b.to_str().unwrap()],
        &repo,
    );

    // Resolve the GitContexts for both worktrees.
    let ctx_a = scope::git_context(&wt_a, 0).expect("ctx A");
    let ctx_b = scope::git_context(&wt_b, 0).expect("ctx B");
    assert_eq!(ctx_a.repository_id, ctx_b.repository_id, "same repo");
    assert_ne!(ctx_a.worktree_id, ctx_b.worktree_id, "different worktrees");
    assert_ne!(ctx_a.branch, ctx_b.branch, "different branches");

    // Build a memory under worktree A. We bypass `scope_for` because the
    // path-based resolver would be hermetic only when the test is run
    // from inside the worktree; explicit fields are clearer.
    let entry = kodo_memory::MemoryEntry {
        id: "wt-only-1".to_owned(),
        kind: kodo_memory::MemoryKind::Episodic,
        status: kodo_memory::MemoryStatus::Active,
        authority: kodo_memory::Authority::Historical,
        scope: MemoryScope {
            project_id: Some(repo.to_string_lossy().into_owned()),
            repository_id: ctx_a.repository_id.clone(),
            branch: ctx_a.branch.clone(),
            worktree_id: ctx_a.worktree_id.clone(),
            session_id: None,
            task_id: None,
            subtask_id: None,
        },
        title: "unmerged work in A".to_owned(),
        abstract_text: "branch wt-a only".to_owned(),
        body: serde_json::json!({}),
        content_hash: "h-wt".to_owned(),
        source_trust: kodo_memory::SourceTrust::ToolVerified,
        confidence: 0.9,
        verified: true,
        conditions: vec![],
        source_uris: vec![],
        valid_from: None,
        valid_until: None,
        last_verified_at: None,
        last_used_at: None,
        use_count: 0,
        created_at: 0,
        updated_at: 0,
        superseded_by: None,
        version: 1,
        candidate_skill: false,
    };
    let mem_path = dir.path().join("memory.sqlite");
    let mem = SqliteMemory::open(&mem_path).unwrap();
    mem.store().upsert_entry(&entry).unwrap();

    // A direct `evaluate` call from worktree B's `GitContext` must
    // mark the entry stale. Worktree isolation is the strictest test.
    let live_b = GitContext {
        project_root: wt_b.clone(),
        repository_id: ctx_b.repository_id.clone(),
        worktree_id: ctx_b.worktree_id.clone(),
        branch: ctx_b.branch.clone(),
        head_commit: ctx_b.head_commit.clone(),
        dirty_paths: Vec::new(),
        observed_at: 0,
    };
    let label = kodo_memory::validity::evaluate(&ValidityInput {
        entry: entry.clone(),
        live: Some(live_b),
        live_file_digest: None,
        stored_file_digest: None,
    });
    assert_eq!(
        label,
        ValidityLabel::Stale,
        "memory anchored to wt-a must read stale from wt-b"
    );

    // And the recall path (which uses `git_context_from_query` under the
    // hood) must drop the entry from the pack.
    let query = MemoryQuery {
        scope: MemoryScope {
            project_id: Some(repo.to_string_lossy().into_owned()),
            repository_id: ctx_b.repository_id.clone(),
            branch: ctx_b.branch.clone(),
            worktree_id: ctx_b.worktree_id.clone(),
            session_id: None,
            task_id: None,
            subtask_id: None,
        },
        text: "unmerged work".to_owned(),
        related_paths: Vec::new(),
        related_symbols: Vec::new(),
        budget_tokens: 2000,
        require_current_evidence: false,
    };
    let report = kodo_memory::retrieval::recall(mem.store(), &query).unwrap();
    assert!(
        report.pack.snippets.is_empty(),
        "wt-b query must not see wt-a memory; got {:?}",
        report.pack.snippets
    );
}

mod scope {
    pub use kodo_memory::scope::*;
}
