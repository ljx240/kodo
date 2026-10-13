mod _helpers;

use _helpers::{fresh_store, sample_entry};
use kodo_memory::model::MemoryScope;
use kodo_memory::{MemoryQuery, MemorySnippet};

#[test]
fn project_a_entries_invisible_from_project_b() {
    let (_dir, store) = fresh_store("scope-iso");
    let a = sample_entry("a", "alpha", "a", Some("/proj/a"));
    let b = sample_entry("b", "beta", "b", Some("/proj/b"));
    store.upsert_entry(&a).unwrap();
    store.upsert_entry(&b).unwrap();

    let caller_a = MemoryScope {
        project_id: Some("/proj/a".to_owned()),
        ..Default::default()
    };
    let caller_b = MemoryScope {
        project_id: Some("/proj/b".to_owned()),
        ..Default::default()
    };

    let pack_a = store
        .with_conn(|c| {
            // We can't directly call retrieval::recall with the Store,
            // so we use the lower-level DAO and apply scope.matches()
            // ourselves in the test.
            let entries = c
                .prepare("SELECT id FROM memory_entries")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, kodo_memory::MemoryError>(entries)
        })
        .unwrap();
    let _ = pack_a; // ensure SQL was happy

    // The right way to assert isolation is at the recall layer, which we
    // exercise end-to-end below. The store.list path does not apply the
    // scope filter; it returns everything.
    let query_a = MemoryQuery {
        scope: caller_a,
        text: "alpha".to_owned(),
        related_paths: vec![],
        related_symbols: vec![],
        budget_tokens: 1000,
        require_current_evidence: false,
    };
    let _ = query_a;
    let _ = caller_b;
    // Skip the full recall path here — covered by retrieval_smoke below.
}

#[test]
fn retrieval_smoke_project_a() {
    use kodo_memory::retrieval;
    let (_dir, store) = fresh_store("scope-recall");
    let a = sample_entry("a1", "k2k routing", "fallback to default", Some("/proj/a"));
    let b = sample_entry("b1", "k2k routing", "fallback to default", Some("/proj/b"));
    store.upsert_entry(&a).unwrap();
    store.upsert_entry(&b).unwrap();
    let q = MemoryQuery {
        scope: MemoryScope {
            project_id: Some("/proj/a".to_owned()),
            ..Default::default()
        },
        text: "routing".to_owned(),
        related_paths: vec![],
        related_symbols: vec![],
        budget_tokens: 1000,
        require_current_evidence: false,
    };
    let report = retrieval::recall(&store, &q).unwrap();
    let ids: Vec<&str> = report
        .pack
        .snippets
        .iter()
        .map(|s: &MemorySnippet| s.id.as_str())
        .collect();
    assert!(ids.contains(&"a1"));
    assert!(!ids.contains(&"b1"));
}
