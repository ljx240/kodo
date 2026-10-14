//! Local, scope-aware knowledge storage for notes and project material.
//!
//! This crate stores immutable text revisions in a standalone SQLite catalog.
//! It does not read source files, call a model, or make network requests.

mod error;
mod model;
mod store;
mod text;

pub use error::{KnowledgeError, KnowledgeResult};
pub use model::{
    BaseDraft, DocumentDetail, DocumentStatus, ImportDraft, KnowledgeBase, KnowledgeChunk,
    KnowledgeDocument, KnowledgeHit, KnowledgeSelection, SearchQuery, SourceKind, TurnKnowledge,
};
pub use store::KnowledgeStore;
pub use text::MAX_TEXT_BYTES;

/// Validate text and return its Unicode character count and real P0 chunk
/// count without opening the catalog or persisting anything.
pub fn preview_text(body: &str) -> KnowledgeResult<(usize, usize)> {
    text::validate_text_body(body)?;
    let chunks = text::make_chunks("", body)?;
    Ok((body.chars().count(), chunks.len()))
}

#[cfg(test)]
mod tests;
