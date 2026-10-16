//! Retrieval. Pulls from the FTS channel + an exact-token channel, fuses
//! with RRF (k=60), enforces scope + validity + diversity, and assembles a
//! `MemoryPack` that fits the budget.

use std::collections::HashMap;

use crate::error::MemoryResult;
use crate::fts::{self, FtsHit};
use crate::metrics::MetricsSnapshot;
use crate::model::{
    MemoryEntry, MemoryPack, MemoryQuery, MemoryScope, MemorySnippet, ValidityLabel,
};
use crate::scope;
use crate::store::Store;
use crate::validity;

const RRF_K: f64 = 60.0;
const HARD_FETCH: usize = 200;

#[derive(Debug, Default)]
struct Rrf {
    /// id -> accumulated rrf score
    scores: HashMap<String, f64>,
    /// id -> rank (1-based) per channel
    fts_rank: HashMap<String, usize>,
    exact_rank: HashMap<String, usize>,
}

impl Rrf {
    fn add_fts(&mut self, hits: &[FtsHit]) {
        for (i, h) in hits.iter().enumerate() {
            let rank = i + 1;
            *self.scores.entry(h.id.clone()).or_insert(0.0) += 1.0 / (RRF_K + rank as f64);
            self.fts_rank.insert(h.id.clone(), rank);
        }
    }

    fn add_exact(&mut self, hits: &[String]) {
        for (i, id) in hits.iter().enumerate() {
            let rank = i + 1;
            *self.scores.entry(id.clone()).or_insert(0.0) += 1.0 / (RRF_K + rank as f64);
            self.exact_rank.insert(id.clone(), rank);
        }
    }

    fn into_sorted(self) -> Vec<(String, f64)> {
        let mut v: Vec<_> = self.scores.into_iter().collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        v
    }
}

pub struct RecallReport {
    pub pack: MemoryPack,
    pub metrics: MetricsSnapshot,
}

/// Run a recall. `metrics` is passed in so callers can update and read
/// their own counters; the pack's `diagnostic` is filled in here.
pub fn recall(store: &Store, query: &MemoryQuery) -> MemoryResult<RecallReport> {
    let start = std::time::Instant::now();
    store
        .metrics
        .recall_calls
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    if query.text.trim().is_empty()
        && query.related_paths.is_empty()
        && query.related_symbols.is_empty()
    {
        let pack = MemoryPack::empty();
        store
            .metrics
            .recall_empty
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return Ok(RecallReport {
            pack,
            metrics: store.metrics.snapshot(),
        });
    }

    // 1. FTS channel.
    let fts_hits = fts::search(store, &query.text, HARD_FETCH).unwrap_or_default();
    // 2. Exact-token channel: scan memory_entries for body/title/abstract
    //    matches. This is what catches CJK / paths / error codes that
    //    FTS5's tokenizer stumbles on.
    let exact_hits = exact_token_search(store, query)?;
    // 3. Path/symbol channel: any entry whose evidence mentions one of
    //    the query's paths boosts it.
    let path_hits = path_evidence_search(store, &query.related_paths)?;

    let mut rrf = Rrf::default();
    rrf.add_fts(&fts_hits);
    rrf.add_exact(&exact_hits);
    rrf.add_exact(&path_hits);

    let sorted = rrf.into_sorted();
    let live_ctx = scope::git_context_from_query(&query.scope);

    let mut snippets: Vec<MemorySnippet> = Vec::new();
    let mut omitted = 0usize;
    let mut tokens_used = 0usize;
    let mut scope_filter_hits = 0usize;
    let mut after_validity = 0usize;
    let mut after_diversity = 0usize;

    for (id, score) in sorted.iter() {
        let Some(entry) = store.get_entry(id)? else {
            continue;
        };
        if !entry.visible_to_recall() {
            omitted += 1;
            continue;
        }
        if !entry.scope.matches(&query.scope) {
            omitted += 1;
            continue;
        }
        scope_filter_hits += 1;
        if query.require_current_evidence && !entry.verified {
            omitted += 1;
            continue;
        }
        // Validity. The caller does not pass a live file digest in the
        // recall path — that wire is only used by the agent loop right
        // before `mark_changed_paths` — so we only get the
        // worktree/branch check here. The stricter SameFileDigest label
        // surfaces from a direct `from_query` call.
        let stored_digest = store.entry_file_digest(&entry.id).ok().flatten();
        let label = validity::evaluate(&validity::ValidityInput {
            entry: entry.clone(),
            live: live_ctx.clone(),
            live_file_digest: None,
            stored_file_digest: stored_digest,
        });
        after_validity += 1;
        if label == ValidityLabel::Stale {
            omitted += 1;
            continue;
        }
        // Diversity by id (already-accepted) and by title Jaccard once
        // the first three slots are filled. The first three are kept
        // unconditionally so the highest-ranked hits always surface;
        // the MMR-lite filter only kicks in for the long tail where
        // many near-duplicates would otherwise crowd the budget.
        if snippets.iter().any(|s| s.id == entry.id) {
            omitted += 1;
            continue;
        }
        if snippets.len() >= 3 && title_jaccard_to_last(&entry, snippets.last().unwrap()) > 0.7 {
            omitted += 1;
            continue;
        }
        let snippet = build_snippet(&entry, label, score);
        let cost = estimate_tokens(&snippet);
        if tokens_used + cost > query.budget_tokens.max(1) {
            omitted += 1;
            continue;
        }
        tokens_used += cost;
        snippets.push(snippet);
        after_diversity += 1;
        let _ = store.touch_used(&entry.id);
    }

    let elapsed = start.elapsed().as_millis() as u64;
    let pack = MemoryPack {
        snippets,
        estimated_tokens: tokens_used,
        retrieved_ids: sorted.iter().take(64).map(|(id, _)| id.clone()).collect(),
        omitted_count: omitted,
        diagnostic: crate::model::MemoryDiagnostic {
            scope_filter_hits,
            fts_hits: fts_hits.len(),
            exact_hits: exact_hits.len(),
            after_validity_filter: after_validity,
            after_diversity_filter: after_diversity,
            tokens_used,
            elapsed_ms: elapsed,
            note: String::new(),
        },
    };
    if pack.snippets.is_empty() {
        store
            .metrics
            .recall_empty
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    } else {
        store
            .metrics
            .recall_hits
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(RecallReport {
        pack,
        metrics: store.metrics.snapshot(),
    })
}

fn build_snippet(entry: &MemoryEntry, label: ValidityLabel, _score: &f64) -> MemorySnippet {
    let scope_label = match label {
        ValidityLabel::SameFileDigest => "current evidence (same file digest)",
        ValidityLabel::CompatibleCommit => "current evidence (compatible commit)",
        ValidityLabel::NeedsRevalidation => "needs revalidation",
        ValidityLabel::Stale => "stale (filtered)",
        ValidityLabel::HistoricalOnly => "historical only",
    }
    .to_owned();
    MemorySnippet {
        id: entry.id.clone(),
        kind: entry.kind,
        title: entry.title.clone(),
        abstract_text: entry.abstract_text.clone(),
        source_uris: entry.source_uris.clone(),
        conditions: entry.conditions.clone(),
        scope_label,
        authority: entry.authority,
        verified: entry.verified,
        validity: label,
    }
}

fn estimate_tokens(s: &MemorySnippet) -> usize {
    // Rough rule of thumb: 1 token per 4 chars in CJK, 1 per 4 chars
    // otherwise. We use a single 4-chars-per-token heuristic; good
    // enough for budget enforcement.
    let chars: usize = s.title.chars().count() + s.abstract_text.chars().count() + 16;
    chars.div_ceil(4)
}

fn exact_token_search(store: &Store, query: &MemoryQuery) -> MemoryResult<Vec<String>> {
    let mut out = Vec::new();
    if query.text.trim().is_empty() && query.related_symbols.is_empty() {
        return Ok(out);
    }
    let candidates: Vec<String> = query
        .related_symbols
        .iter()
        .cloned()
        .chain(query.text.split_whitespace().map(|s| s.to_owned()))
        .filter(|s| s.chars().count() >= 2)
        .collect();
    if candidates.is_empty() {
        return Ok(out);
    }
    let filter = crate::model::MemoryFilter {
        limit: Some(HARD_FETCH),
        include_history: false,
        ..Default::default()
    };
    let entries = store.list(&filter)?;
    for e in &entries {
        for c in &candidates {
            if entry_contains(e, c) {
                if !out.contains(&e.id) {
                    out.push(e.id.clone());
                }
                break;
            }
        }
    }
    Ok(out)
}

fn entry_contains(e: &MemoryEntry, needle: &str) -> bool {
    if e.title.contains(needle) || e.abstract_text.contains(needle) {
        return true;
    }
    // The body is a JSON value (often an Object, never a plain string
    // except for very simple entries). `as_str()` returns `None` for
    // any non-string value, so we serialize back to a string for the
    // substring check. `to_string()` is cheap on Object values and
    // hits the same stringification that lands on disk.
    let body_str = e.body.to_string();
    if body_str.contains(needle) {
        return true;
    }
    for u in &e.source_uris {
        if u.contains(needle) {
            return true;
        }
    }
    false
}

fn path_evidence_search(store: &Store, paths: &[String]) -> MemoryResult<Vec<String>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    store.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT DISTINCT memory_id FROM memory_evidence WHERE path = ?1 \
             ORDER BY memory_id",
        )?;
        let mut out = Vec::new();
        for p in paths {
            let rows = stmt
                .query_map(rusqlite::params![p], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for r in rows {
                if !out.contains(&r) {
                    out.push(r);
                }
            }
        }
        Ok(out)
    })
}

pub fn filter_by_scope(store: &Store, scope: &MemoryScope) -> MemoryResult<Vec<MemoryEntry>> {
    let filter = crate::model::MemoryFilter {
        scope: Some(scope.clone()),
        ..Default::default()
    };
    store.list(&filter)
}

/// MMR-lite diversity: Jaccard similarity over the first 40 chars of
/// the title (the high-signal half of the snippet's title) compared to
/// the last accepted snippet. Returns 0.0 when either side is empty.
/// The `> 0.7` threshold in the caller is a heuristic — confirmed in
/// unit tests with both "identical" and "unrelated" inputs.
fn title_jaccard_to_last(entry: &MemoryEntry, last: &MemorySnippet) -> f64 {
    let a: Vec<char> = entry.title.chars().take(40).collect();
    let b: Vec<char> = last.title.chars().take(40).collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a_set: std::collections::HashSet<char> = a.iter().copied().collect();
    let b_set: std::collections::HashSet<char> = b.iter().copied().collect();
    let inter = a_set.intersection(&b_set).count() as f64;
    let union = a_set.union(&b_set).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}
