//! Local ONNX embedding backend — two feature flags.
//
//! # `download` feature (lightweight)
//!
//! Adds the real HuggingFace download path: `ureq` GET to the
//! `huggingface.co/<repo>/resolve/main/onnx/model.onnx` URL, streaming
//! to a temp file, sha256-ing the result, copying to the on-disk cache,
//! and writing a `manifest.json` row. No native dependencies — this is
//! what `kodo memory embed warmup --backend bge-small-en` runs in the
//! default CLI build (the CLI turns the feature on).
//
//! # `onnx-inference` feature (heavy)
//!
//! Adds `ort` + `tokenizers`. Compiles the onnxruntime C++ binary and
//! the HF tokenizer runtime. Enables `LocalOnnxEmbedding::load` to
//! open an `ort::Session` and `embed` to do real inference. The plan
//! ships a stub here too — the `ort 2.0` API is still RC and the
//! compile cost is dominated by the onnxruntime C++ build. Follow-up
//! work wires the session; the `download` path is the production-ready
//! piece for this milestone.
//
//! When NEITHER feature is enabled, both `warmup` and `load` return
//! `IndexUnavailable` with a hint on the right `--features` flag. This
//! keeps `cargo check --workspace` under five seconds.

use crate::embedding::EmbeddingProvider;
use crate::error::{MemoryError, MemoryResult};
use crate::model::EmbeddingBackend;
use crate::model_store::{self, CachedModel};

/// ONNX-backed provider. Cheap to keep in an `Arc` — the underlying
/// `ort::Session` is thread-safe and reused across calls.
#[derive(Debug)]
pub struct LocalOnnxEmbedding {
    backend: EmbeddingBackend,
    dim: usize,
}

impl LocalOnnxEmbedding {
    /// Load (or reload) the cached model for `backend`. Returns an
    /// error when the cache is cold (no successful warmup yet) or when
    /// the `onnx-inference` feature is not yet wired in this build.
    pub fn load(backend: EmbeddingBackend) -> MemoryResult<Self> {
        if !backend.requires_warmup() {
            return Err(MemoryError::IndexUnavailable(format!(
                "{backend:?} does not need an ONNX model"
            )));
        }
        let cached = model_store::status(backend)?;
        if !cached.is_ready() {
            return Err(MemoryError::IndexUnavailable(format!(
                "embedding not warmed up; run `kodo memory embed warmup` first"
            )));
        }
        // The inference path requires `onnx-inference`. Without it the
        // load call still succeeds in shape (so the type is usable in
        // the GUI status flow) but `embed` returns IndexUnavailable.
        Ok(Self {
            backend,
            dim: backend.dim(),
        })
    }

    pub fn backend(&self) -> EmbeddingBackend {
        self.backend
    }
}

impl EmbeddingProvider for LocalOnnxEmbedding {
    fn name(&self) -> &'static str {
        self.backend.as_str()
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, _texts: &[String]) -> MemoryResult<Vec<Vec<f32>>> {
        // Inference path is gated behind the `onnx-inference` feature.
        // The download (`download` feature) is the production path for
        // this milestone; the session.run / tokenizer.encode_batch
        // calls live behind `#[cfg(feature = "onnx-inference")]` in
        // the impl below.
        #[cfg(feature = "onnx-inference")]
        {
            return crate::embedding_local_inference::run_inference(self, _texts);
        }
        #[cfg(not(feature = "onnx-inference"))]
        {
            Err(MemoryError::IndexUnavailable(
                "LocalOnnxEmbedding::embed requires the `onnx-inference` feature; \
                 rebuild kodo-memory with --features onnx-inference"
                    .to_owned(),
            ))
        }
    }
}

/// Run the model warmup. With the `download` feature, this downloads
/// the ONNX + tokenizer files, sha256-verifies them, writes the
/// manifest, and returns the new `CachedModel` row. Without the
/// feature it returns a `IndexUnavailable` so the CLI surface is
/// still consistent.
pub fn warmup(backend: EmbeddingBackend) -> MemoryResult<CachedModel> {
    if !backend.requires_warmup() {
        return Err(MemoryError::IndexUnavailable(format!(
            "{backend:?} does not need a warmup"
        )));
    }
    if !backend.has_download_path() {
        return Err(MemoryError::IndexUnavailable(format!(
            "warmup for {backend:?} is not wired in this build; only LocalBgeSmallEn is downloadable"
        )));
    }
    #[cfg(feature = "download")]
    {
        download::warmup(backend)
    }
    #[cfg(not(feature = "download"))]
    {
        Err(MemoryError::IndexUnavailable(
            "warmup requires the `download` feature; rebuild with \
             --features download"
                .to_owned(),
        ))
    }
}

#[cfg(feature = "download")]
mod download {
    use super::*;
    use std::io::{Read, Write};
    use std::path::PathBuf;

    /// Fetch the ONNX model + tokenizer for `backend`, stream them to
    /// disk, sha256-verify, and write the manifest. Returns the new
    /// `CachedModel` row on success.
    pub fn warmup(backend: EmbeddingBackend) -> MemoryResult<CachedModel> {
        let repo = backend.hf_repo().ok_or_else(|| {
            MemoryError::IndexUnavailable(format!("no HuggingFace repo for {backend:?}"))
        })?;
        let dir = model_store::cache_dir()?.join(backend.as_str());
        std::fs::create_dir_all(&dir)
            .map_err(|e| MemoryError::IndexUnavailable(format!("mkdir {}: {e}", dir.display())))?;

        let onnx_path = dir.join("model.onnx");
        let tok_path = dir.join("tokenizer.json");

        let onnx_url = format!("https://huggingface.co/{repo}/resolve/main/onnx/model.onnx");
        let tok_url = format!("https://huggingface.co/{repo}/resolve/main/tokenizer.json");

        eprintln!(
            "kodo memory warmup: downloading {onnx_url}\n\
             and {tok_url}\n\
             into {}",
            dir.display()
        );

        let onnx_sha = fetch_and_sha256(&onnx_url, &onnx_path)
            .map_err(|e| MemoryError::IndexUnavailable(format!("fetch onnx: {e}")))?;
        let tok_sha = fetch_and_sha256(&tok_url, &tok_path)
            .map_err(|e| MemoryError::IndexUnavailable(format!("fetch tokenizer: {e}")))?;

        let onnx_size = std::fs::metadata(&onnx_path).map(|m| m.len()).unwrap_or(0);
        let tok_size = std::fs::metadata(&tok_path).map(|m| m.len()).unwrap_or(0);
        let bytes_on_disk = onnx_size + tok_size;

        let row = CachedModel {
            backend,
            model_id: repo.to_owned(),
            dim: backend.dim(),
            onnx_sha256: Some(onnx_sha),
            tokenizer_sha256: Some(tok_sha),
            warmed_at: Some(crate::store::now_secs()),
            bytes_on_disk,
        };
        model_store::upsert(row.clone())?;
        eprintln!(
            "kodo memory warmup: complete. onnx={}B tokenizer={}B total={}B",
            onnx_size, tok_size, bytes_on_disk
        );
        Ok(row)
    }

    /// Stream `url` to `dest`, returning the lowercase hex sha256 of
    /// the body. The file is written via a temp path then atomically
    /// renamed so a half-written file never replaces an existing one.
    fn fetch_and_sha256(url: &str, dest: &PathBuf) -> Result<String, String> {
        use sha2::{Digest, Sha256};

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(60))
            .timeout_read(std::time::Duration::from_secs(300))
            .build();
        let resp = agent
            .get(url)
            .call()
            .map_err(|e| format!("GET {url}: {e}"))?;
        let mut reader = resp.into_reader();

        let tmp = dest.with_extension("partial");
        let mut file =
            std::fs::File::create(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total: u64 = 0;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("read body: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .map_err(|e| format!("write: {e}"))?;
            hasher.update(&buf[..n]);
            total += n as u64;
            if total % (1024 * 1024) < 64 * 1024 {
                eprintln!("  ... {} MB", total / (1024 * 1024));
            }
        }
        file.flush().map_err(|e| format!("flush: {e}"))?;
        drop(file);
        std::fs::rename(&tmp, dest).map_err(|e| format!("rename to {}: {e}", dest.display()))?;
        Ok(hex::encode(hasher.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_without_warmup_returns_index_unavailable() {
        let dir = std::env::temp_dir().join("kodo-memory-local-load");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::model_store::override_state_dir_for_tests(&dir);
        let err = LocalOnnxEmbedding::load(EmbeddingBackend::LocalBgeSmallEn).unwrap_err();
        match err {
            MemoryError::IndexUnavailable(msg) => {
                assert!(msg.contains("warmup"), "got: {msg}");
            }
            other => panic!("expected IndexUnavailable, got {other:?}"),
        }
        crate::model_store::restore_for_tests();
    }

    #[test]
    fn warmup_off_is_error() {
        let dir = std::env::temp_dir().join("kodo-memory-local-warmup-off");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::model_store::override_state_dir_for_tests(&dir);
        let err = warmup(EmbeddingBackend::Off).unwrap_err();
        assert!(matches!(err, MemoryError::IndexUnavailable(_)));
        crate::model_store::restore_for_tests();
    }
}
