mod _helpers;

use _helpers::{fresh_store, sample_entry};
use kodo_memory::model::MemoryScope;
use kodo_memory::{retrieval, MemoryFilter, MemoryQuery};

#[test]
fn listing_applies_project_worktree_and_session_scope_before_limit() {
    let (_dir, store) = fresh_store("scope-iso");
    let mut a = sample_entry("a", "alpha", "a", Some("/proj/a"));
    a.scope.worktree_id = Some("a".into());
    a.updated_at = 1;
    let b = sample_entry("b", "beta", "b", Some("/proj/b"));
    store.upsert_entry(&a).unwrap();
    store.upsert_entry(&b).unwrap();
    let mut other = b;
    other.scope = a.scope.clone();
    other.scope.session_id = Some("private session".into());
    other.id = "session-only".into();
    store.upsert_entry(&other).unwrap();
    other.id = "worktree-only".into();
    other.scope.session_id = None;
    other.scope.worktree_id = Some("different worktree".into());
    store.upsert_entry(&other).unwrap();

    let entries = store
        .list(&MemoryFilter {
            scope: Some(MemoryScope {
                session_id: Some("another session".into()),
                ..a.scope.clone()
            }),
            limit: Some(1),
            include_history: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["a"]
    );
}

#[test]
fn retrieval_smoke_project_a() {
    let (_dir, store) = fresh_store("scope-recall");
    let mut a = sample_entry("a1", "routing 修复路由表", "项目 A 的记忆", Some("/proj/a"));
    a.updated_at = 1;
    store.upsert_entry(&a).unwrap();
    for index in 0..201 {
        let b = sample_entry(
            &format!("b{index}"),
            "routing routing routing",
            "路由表",
            Some("/proj/b"),
        );
        store.upsert_entry(&b).unwrap();
        let distractor = sample_entry(
            &format!("a-recent-{index}"),
            "unrelated",
            "cache",
            Some("/proj/a"),
        );
        store.upsert_entry(&distractor).unwrap();
    }
    let mut q = MemoryQuery {
        scope: MemoryScope {
            project_id: Some("/proj/a".to_owned()),
            session_id: Some("new session".into()),
            ..Default::default()
        },
        text: "routing".to_owned(),
        related_paths: vec![],
        related_symbols: vec![],
        budget_tokens: 1000,
        require_current_evidence: false,
    };
    let report = retrieval::recall(&store, &q).unwrap();
    assert_eq!(report.pack.retrieved_ids, ["a1"]);
    assert_eq!(report.pack.diagnostic.fts_hits, 1);
    assert_eq!(report.pack.diagnostic.exact_hits, 1);
    q.text = "请检查路由表".to_owned();
    assert_eq!(
        retrieval::recall(&store, &q).unwrap().pack.retrieved_ids,
        ["a1"]
    );
}
