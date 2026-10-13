// P1: RRF must merge hits from the FTS channel and the exact-token
// channel. We seed two memories that exercise each channel
// asymmetrically and assert both appear in the pack when the query
// is broad enough to hit both.

mod _helpers;

use kodo_memory::MemoryQuery;

use _helpers::sample_entry;

#[test]
fn rrf_merges_fts_strong_and_exact_strong_into_one_pack() {
    let (_dir, store) = _helpers::fresh_store("rrf-fusion");

    // Memory A: strong FTS hit. The unique title word `florbleglorb`
    // and the abstract word `florbleglorb-2000` survive FTS5's
    // unicode61 tokenizer. The exact-token channel finds this too,
    // but the FTS contribution is the dominant signal.
    let entry_a = sample_entry(
        "rrf-a",
        "k2k routing florbleglorb",
        "florbleglorb-2000 k2k 路由在 catalog 缺失时",
        Some("/tmp/proj"),
    );
    store.upsert_entry(&entry_a).unwrap();

    // Memory B: exact-token-only. The body holds a path-shaped string
    // that the FTS escape function drops (slashes and `::` are not
    // alphanumerics). The exact-token channel still finds it because
    // it searches the body / title / abstract / source_uris columns
    // directly without going through the FTS tokenizer.
    let mut entry_b = sample_entry(
        "rrf-b",
        "tracing layer",
        "telemetry hooks for distributed",
        Some("/tmp/proj"),
    );
    entry_b.body = serde_json::json!({
        "path": "src/handlers/odd-symbol-route.rs",
        "hint": "k2k routes via the catalog"
    });
    entry_b.source_uris = vec!["kodo://sessions/abc".to_owned()];
    store.upsert_entry(&entry_b).unwrap();

    // The query carries `florbleglorb` (FTS-strong for A) and the
    // path-shaped symbol (exact-only for B). Both channels fire;
    // RRF should put both entries in the pack.
    let query = MemoryQuery {
        scope: kodo_memory::MemoryScope {
            project_id: Some("/tmp/proj".to_owned()),
            ..Default::default()
        },
        text: "florbleglorb src/handlers/odd-symbol-route.rs".to_owned(),
        related_paths: vec!["src/handlers/odd-symbol-route.rs".to_owned()],
        related_symbols: vec!["florbleglorb".to_owned()],
        budget_tokens: 2000,
        require_current_evidence: false,
    };
    let report = kodo_memory::retrieval::recall(&store, &query).unwrap();
    let ids: Vec<&str> = report.pack.snippets.iter().map(|s| s.id.as_str()).collect();
    assert!(
        ids.contains(&"rrf-a"),
        "FTS-strong memory must be in the pack; got {ids:?}"
    );
    assert!(
        ids.contains(&"rrf-b"),
        "exact-token memory must be in the pack; got {ids:?}"
    );
    // The diagnostic counters should reflect both channels firing.
    assert!(
        report.pack.diagnostic.exact_hits >= 1,
        "exact channel must contribute"
    );
    assert!(
        report.pack.diagnostic.fts_hits >= 1,
        "FTS channel must contribute"
    );
}
