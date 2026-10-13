//! Pluggable embedding provider. P2: ships a trait and a default offline
//! backend. No real model; the trait surface is the contract.

use crate::error::{MemoryError, MemoryResult};
use crate::model::EmbeddingBackend;
use std::sync::Arc;

pub trait EmbeddingProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn dim(&self) -> usize;
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError>;
}

/// Default provider. Reports itself as unavailable so RRF simply skips
/// the embedding channel.
pub struct NoopEmbedding;

impl EmbeddingProvider for NoopEmbedding {
    fn name(&self) -> &'static str {
        "noop"
    }
    fn dim(&self) -> usize {
        0
    }
    fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Err(MemoryError::IndexUnavailable(
            "embedding disabled (noop)".to_owned(),
        ))
    }
}

/// Offline hash-based embedding. Trigram hash → fixed-dim vector. Cheap,
/// deterministic, no model. Used to exercise the wiring path even when
/// no real provider is configured.
pub struct LocalHashEmbedding {
    pub dim: usize,
}

impl LocalHashEmbedding {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(8) }
    }
}

impl EmbeddingProvider for LocalHashEmbedding {
    fn name(&self) -> &'static str {
        "local-hash"
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0_f32; self.dim];
                let chars: Vec<char> = t.chars().collect();
                if chars.len() < 3 {
                    return v;
                }
                for w in chars.windows(3) {
                    let n: u32 = (w[0] as u32)
                        .wrapping_mul(131)
                        .wrapping_add(w[1] as u32)
                        .wrapping_mul(131)
                        .wrapping_add(w[2] as u32);
                    let idx = (n as usize) % self.dim;
                    v[idx] += 1.0;
                }
                // L2 normalize.
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in &mut v {
                        *x /= norm;
                    }
                }
                v
            })
            .collect())
    }
}

/// Stub for the optional ONNX-backed provider. Only available behind
/// the `onnx` cargo feature. Construction goes through
/// `embedding_local::LocalOnnxEmbedding`; the variant here exists so
/// the rest of the crate can name the type uniformly.
#[cfg(any(feature = "download", feature = "onnx-inference"))]
pub use crate::embedding_local::LocalOnnxEmbedding;

/// Construct the provider for a given backend. The default
/// (`EmbeddingBackend::Off`) returns `NoopEmbedding`; everything else
/// returns a real provider. Used by `SqliteMemory::open_with_config`
/// and by the CLI / Tauri startup paths.
pub fn provider_for(backend: EmbeddingBackend) -> MemoryResult<Arc<dyn EmbeddingProvider>> {
    let provider: Arc<dyn EmbeddingProvider> = match backend {
        EmbeddingBackend::Off => Arc::new(NoopEmbedding),
        EmbeddingBackend::LocalHash => Arc::new(LocalHashEmbedding::new(backend.dim())),
        #[cfg(any(feature = "download", feature = "onnx-inference"))]
        EmbeddingBackend::LocalBgeSmallEn
        | EmbeddingBackend::LocalBgeSmallZh
        | EmbeddingBackend::LocalMiniLmL6 => Arc::new(
            crate::embedding_local::LocalOnnxEmbedding::load(backend)
                .map_err(|e| MemoryError::IndexUnavailable(format!("{e:?}")))?,
        ),
        #[cfg(not(any(feature = "download", feature = "onnx-inference")))]
        EmbeddingBackend::LocalBgeSmallEn
        | EmbeddingBackend::LocalBgeSmallZh
        | EmbeddingBackend::LocalMiniLmL6 => {
            return Err(MemoryError::IndexUnavailable(
                "local embedding backend requires `--features download` at build time".to_owned(),
            ))
        }
    };
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_reports_unavailable() {
        let p = NoopEmbedding;
        assert_eq!(p.name(), "noop");
        assert!(p.embed(&["a".to_owned()]).is_err());
    }

    #[test]
    fn local_hash_is_deterministic() {
        let p = LocalHashEmbedding::new(64);
        let a = p.embed(&["hello world".to_owned()]).unwrap();
        let b = p.embed(&["hello world".to_owned()]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].len(), 64);
    }

    #[test]
    fn provider_for_off_is_noop() {
        let p = provider_for(EmbeddingBackend::Off).unwrap();
        assert_eq!(p.name(), "noop");
        assert_eq!(p.dim(), 0);
    }

    #[test]
    fn provider_for_local_hash_is_deterministic() {
        let p = provider_for(EmbeddingBackend::LocalHash).unwrap();
        assert_eq!(p.name(), "local-hash");
        assert_eq!(p.dim(), 64);
        let a = p.embed(&["abc".to_owned(), "abc".to_owned()]).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0], a[1]);
    }
}
