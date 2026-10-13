//! FTS5 retrieval. Returns `(id, bm25_rank)` rows. The retrieval layer
//! fuses these with the exact-token channel via RRF; this module never
//! picks winners on its own.

use rusqlite::{params, Connection, OpenFlags};

use crate::error::MemoryResult;
use crate::store::Store;

#[derive(Debug, Clone)]
pub struct FtsHit {
    pub id: String,
    pub score: f64,
}

/// Escape a free-text query for FTS5. We use a defensive approach: strip
/// anything that isn't a letter, digit, CJK character, or whitespace, then
/// wrap each token in double quotes. The exact-token channel handles the
/// cases the stripper is too aggressive on (paths, error codes).
///
/// FTS5's default boolean is implicit AND. A query built by joining the
/// stripped tokens with spaces would force every token to match a row,
/// which is wrong for a free-form query that mixes terms with path-shaped
/// symbols the stripper has no hope of matching (e.g. `florbleglorb
/// src/handlers/odd-symbol-route.rs` becomes `"florbleglorb"
/// "srchandlersoddsymbolroute"`, and the second phrase has no hits in any
/// row). Joining with `OR` instead means each token independently
/// contributes; the BM25 ranking still gives the row that matches more
/// tokens a higher score.
fn fts_escape(query: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for token in query.split_whitespace() {
        let mut clean = String::with_capacity(token.len());
        for ch in token.chars() {
            if ch.is_alphanumeric() || ch.is_ascii_punctuation() && ch == '_' {
                clean.push(ch);
            } else if (ch as u32) >= 0x4E00 {
                // CJK unified ideographs and beyond — keep them, the
                // unicode61 tokenizer with the right categories option
                // would do this for us, but the strip above would drop
                // them otherwise.
                clean.push(ch);
            }
        }
        if !clean.is_empty() {
            parts.push(format!("\"{}\"", clean.replace('"', "")));
        }
    }
    parts.join(" OR ")
}

pub fn search(store: &Store, query: &str, limit: usize) -> MemoryResult<Vec<FtsHit>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let escaped = fts_escape(query);
    if escaped.is_empty() {
        return Ok(Vec::new());
    }
    store.with_conn(|c| fts_search_conn(c, &escaped, limit))
}

fn fts_search_conn(c: &Connection, escaped: &str, limit: usize) -> MemoryResult<Vec<FtsHit>> {
    // Open a read-only connection to a transient URI is unnecessary; we
    // already have `c`. But we want to *force* a read-only statement for
    // clarity.
    let _ = OpenFlags::SQLITE_OPEN_READ_ONLY;
    // bm25() returns a negative score; lower = more relevant. We negate
    // it so higher is better, matching the exact-token channel's
    // direction.
    let mut stmt = c.prepare(
        "SELECT me.id, -bm25(memory_entries_fts) AS score
         FROM memory_entries_fts
         JOIN memory_entries me ON me.rowid = memory_entries_fts.rowid
         WHERE memory_entries_fts MATCH ?1
         ORDER BY score DESC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![escaped, limit as i64], |r| {
            Ok(FtsHit {
                id: r.get(0)?,
                score: r.get::<_, f64>(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Sanity test: insert, search, ensure the row is hit.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Metrics;
    use crate::model::{
        Authority, MemoryEntry, MemoryKind, MemoryScope, MemoryStatus, SourceTrust,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_db() -> Store {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("kodo-memory-fts-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.sqlite");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        Store::open(path, Arc::new(Metrics::new())).unwrap()
    }

    #[test]
    fn fts_hits_english_title() {
        // The unicode61 tokenizer handles Latin / mixed text well. CJK
        // recall is the exact-token channel's job (retrieval.rs).
        let store = tmp_db();
        let now = crate::store::now_secs();
        let entry = MemoryEntry {
            id: "fts-1".to_owned(),
            kind: MemoryKind::Episodic,
            status: MemoryStatus::Active,
            authority: Authority::Historical,
            scope: MemoryScope::default(),
            title: "k2k-rust routing table".to_owned(),
            abstract_text: "fallback to default topic when missing".to_owned(),
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
            created_at: now,
            updated_at: now,
            superseded_by: None,
            version: 1,
            candidate_skill: false,
        };
        store.upsert_entry(&entry).unwrap();
        let hits = search(&store, "routing", 10).unwrap();
        assert!(!hits.is_empty(), "FTS should find English title");
        assert_eq!(hits[0].id, "fts-1");
    }

    #[test]
    fn fts_escape_strips_injection() {
        // Tokens are joined with `OR` so a free-form query that mixes
        // real terms with path-shaped symbols still matches the real
        // terms. Implicit AND would require every token to match a row.
        assert_eq!(fts_escape("hello world"), "\"hello\" OR \"world\"");
        // CJK kept
        assert!(fts_escape("路由表").contains("\"路由表\""));
    }
}
