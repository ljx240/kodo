//! Policy trait. P0 ships an empty trait + default; the real adaptive
//! strategies (counterfactual replay, retention pruning) land in P2.

use crate::error::MemoryResult;
use crate::model::{MemoryEntry, MemoryQuery};

/// Pluggable strategy. Implementations may rewrite a `MemoryQuery` before
/// it hits retrieval (e.g. expand abbreviations, add terms) and may
/// rewrite a recall result (e.g. suppress recently-quarantined entries).
pub trait MemoryPolicy: Send + Sync {
    fn name(&self) -> &'static str;
    fn rewrite_query(&self, q: &mut MemoryQuery);
    fn post_filter(&self, entries: Vec<MemoryEntry>) -> MemoryResult<Vec<MemoryEntry>>;
}

/// Default no-op. The wiring path is exercised; nothing is changed.
pub struct NoopPolicy;

impl MemoryPolicy for NoopPolicy {
    fn name(&self) -> &'static str {
        "noop"
    }
    fn rewrite_query(&self, _q: &mut MemoryQuery) {}
    fn post_filter(&self, entries: Vec<MemoryEntry>) -> MemoryResult<Vec<MemoryEntry>> {
        Ok(entries)
    }
}
