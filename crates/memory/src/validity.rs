//! Validity evaluation. Compares a memory's recorded scope (worktree, branch,
//! head commit) and stored file digest against the live code/git state. P1
//! replaces the P0 stub with a real check; the no-`GitContext` case still
//! returns `CompatibleCommit` so project-less / git-less runs are unchanged.

use crate::model::{MemoryEntry, MemoryQuery, ValidityLabel};
use crate::scope::GitContext;
use crate::store::Store;

pub struct ValidityInput {
    pub entry: MemoryEntry,
    /// Live `GitContext` resolved from the query's scope. `None` when the
    /// project has no git, git is not on PATH, or the scope has no
    /// `project_id`. Without a live context the engine degrades to
    /// `CompatibleCommit` so the recall never gets worse than P0.
    pub live: Option<GitContext>,
    /// Live digest of the file the memory is anchored to, computed by the
    /// agent loop right before this call. `None` means the caller did not
    /// read the file this turn.
    pub live_file_digest: Option<String>,
    /// Stored digest read from the entry's most recent `memory_evidence`
    /// row. `None` when the entry has no anchored file evidence.
    pub stored_file_digest: Option<String>,
}

impl ValidityInput {
    /// Build a `ValidityInput` from a `MemoryQuery` + entry + live file
    /// digest. Resolves the git context via `git_context_from_query` and
    /// pulls the stored digest from the entry's evidence row. Best-effort:
    /// any storage error degrades to `stored_file_digest = None`.
    pub fn from_query(
        store: &Store,
        query: &MemoryQuery,
        entry: MemoryEntry,
        live_file_digest: Option<String>,
    ) -> ValidityInput {
        let live = crate::scope::git_context_from_query(&query.scope);
        let stored = store.entry_file_digest(&entry.id).ok().flatten();
        ValidityInput {
            entry,
            live,
            live_file_digest,
            stored_file_digest: stored,
        }
    }
}

pub fn evaluate(input: &ValidityInput) -> ValidityLabel {
    // No git context → project-less or git-less run. The recall path is
    // unchanged from P0: any entry that survived the scope filter is
    // treated as compatible.
    let Some(live) = &input.live else {
        return ValidityLabel::CompatibleCommit;
    };

    // Worktree isolation is the strictest test. A memory anchored to a
    // different worktree than the current one is stale, regardless of
    // branch / commit / digest — the working tree contents differ.
    if let (Some(stored), Some(live_id)) = (
        input.entry.scope.worktree_id.as_deref(),
        live.worktree_id.as_deref(),
    ) {
        if stored != live_id {
            return ValidityLabel::Stale;
        }
    }

    // Branch mismatch → needs revalidation. The head commit of the new
    // branch could still match by coincidence, but the visible history
    // is different, so a manual check is warranted before the memory
    // is used.
    if let (Some(stored), Some(live_branch)) =
        (input.entry.scope.branch.as_deref(), live.branch.as_deref())
    {
        if stored != live_branch {
            return ValidityLabel::NeedsRevalidation;
        }
    }

    // File-digest check is the strongest signal we have today (the
    // `MemoryScope` does not yet carry `head_commit`; that field lands
    // when a migration is added). A live digest mismatch is the
    // "the file changed, the memory is stale" case; a match is the
    // "same evidence" case the UI surfaces as a stronger "current
    // evidence" label.
    match (
        input.stored_file_digest.as_deref(),
        input.live_file_digest.as_deref(),
    ) {
        (Some(stored), Some(live_digest)) => {
            if stored == live_digest {
                return ValidityLabel::SameFileDigest;
            } else {
                return ValidityLabel::Stale;
            }
        }
        // Stored digest present, live digest missing — caller did not
        // read the file this turn. Fall through to `CompatibleCommit` so
        // the agent can still recall; the stricter check fires on the
        // next turn that does read the file.
        (Some(_), None) => {}
        // Live digest present, no stored digest — the memory was never
        // anchored to a file, so the per-file comparison does not apply.
        (None, Some(_)) => {}
        (None, None) => {}
    }

    ValidityLabel::CompatibleCommit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Authority, MemoryEntry, MemoryKind, MemoryScope, MemoryStatus, SourceTrust,
    };
    use crate::scope::GitContext;
    use std::path::PathBuf;

    fn sample_entry(scope: MemoryScope) -> MemoryEntry {
        MemoryEntry {
            id: "v-1".to_owned(),
            kind: MemoryKind::Episodic,
            status: MemoryStatus::Active,
            authority: Authority::Historical,
            scope,
            title: "t".to_owned(),
            abstract_text: "a".to_owned(),
            body: serde_json::json!({}),
            content_hash: "h".to_owned(),
            source_trust: SourceTrust::Untrusted,
            confidence: 0.5,
            verified: false,
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
        }
    }

    fn live(worktree: Option<&str>, branch: Option<&str>) -> GitContext {
        GitContext {
            project_root: PathBuf::from("/tmp/proj"),
            repository_id: Some("/tmp/proj/.git".to_owned()),
            worktree_id: worktree.map(str::to_owned),
            branch: branch.map(str::to_owned),
            head_commit: Some("abc".to_owned()),
            dirty_paths: Vec::new(),
            observed_at: 1,
        }
    }

    #[test]
    fn no_live_context_returns_compatible_commit() {
        let entry = sample_entry(MemoryScope::default());
        let label = evaluate(&ValidityInput {
            entry,
            live: None,
            live_file_digest: None,
            stored_file_digest: None,
        });
        assert_eq!(label, ValidityLabel::CompatibleCommit);
    }

    #[test]
    fn worktree_mismatch_marks_stale() {
        let mut scope = MemoryScope::default();
        scope.worktree_id = Some("/worktrees/A".to_owned());
        let entry = sample_entry(scope);
        let live_ctx = live(Some("/worktrees/B"), Some("main"));
        let label = evaluate(&ValidityInput {
            entry,
            live: Some(live_ctx),
            live_file_digest: None,
            stored_file_digest: None,
        });
        assert_eq!(label, ValidityLabel::Stale);
    }

    #[test]
    fn worktree_match_but_branch_mismatch_marks_needs_revalidation() {
        let mut scope = MemoryScope::default();
        scope.worktree_id = Some("/worktrees/A".to_owned());
        scope.branch = Some("feat/x".to_owned());
        let entry = sample_entry(scope);
        let live_ctx = live(Some("/worktrees/A"), Some("main"));
        let label = evaluate(&ValidityInput {
            entry,
            live: Some(live_ctx),
            live_file_digest: None,
            stored_file_digest: None,
        });
        assert_eq!(label, ValidityLabel::NeedsRevalidation);
    }

    #[test]
    fn file_digest_match_marks_same_file_digest() {
        let entry = sample_entry(MemoryScope::default());
        let live_ctx = live(Some("/worktrees/A"), Some("main"));
        let label = evaluate(&ValidityInput {
            entry,
            live: Some(live_ctx),
            live_file_digest: Some("abc123".to_owned()),
            stored_file_digest: Some("abc123".to_owned()),
        });
        assert_eq!(label, ValidityLabel::SameFileDigest);
    }

    #[test]
    fn file_digest_mismatch_marks_stale() {
        let entry = sample_entry(MemoryScope::default());
        let live_ctx = live(Some("/worktrees/A"), Some("main"));
        let label = evaluate(&ValidityInput {
            entry,
            live: Some(live_ctx),
            live_file_digest: Some("new".to_owned()),
            stored_file_digest: Some("old".to_owned()),
        });
        assert_eq!(label, ValidityLabel::Stale);
    }

    #[test]
    fn missing_live_digest_falls_through_to_compatible_commit() {
        // We have a stored digest but the caller did not read the file
        // this turn. We still want the entry to recall, so we degrade to
        // CompatibleCommit. The stricter SameFileDigest label only fires
        // when both sides are present.
        let entry = sample_entry(MemoryScope::default());
        let live_ctx = live(Some("/worktrees/A"), Some("main"));
        let label = evaluate(&ValidityInput {
            entry,
            live: Some(live_ctx),
            live_file_digest: None,
            stored_file_digest: Some("old".to_owned()),
        });
        assert_eq!(label, ValidityLabel::CompatibleCommit);
    }
}
