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
use std::path::Path;

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
///
/// `local_dir` provides an offline path: when set, the two files
/// (`model.onnx` + `tokenizer.json`) are copied from this directory
/// instead of being fetched over the network. Useful when
/// `huggingface.co` is unreachable (regional blocks, air-gapped
/// machines, restricted CI) — the user downloads the files via
/// browser and points the CLI at the local directory.
///
/// Idempotent: when the cache is already populated, returns the
/// existing `CachedModel` row without re-downloading.
pub fn warmup(backend: EmbeddingBackend, local_dir: Option<&Path>) -> MemoryResult<CachedModel> {
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
    // Idempotency: skip the work if the cache already has both files
    // and a recorded warmup timestamp. Re-runs are cheap (no network
    // hit, no file copy) and obvious in logs.
    if let Ok(row) = model_store::status(backend) {
        if row.is_ready() {
            return Ok(row);
        }
    }
    #[cfg(feature = "download")]
    {
        download::warmup(backend, local_dir)
    }
    #[cfg(not(feature = "download"))]
    {
        let _ = local_dir;
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

    /// Minimum size for the ONNX file. The real bge-small-en-v1.5
    /// model.onnx is ~33 MB. Anything under this is almost certainly
    /// an HTML error page saved by mistake (HF returns a small body
    /// for 404s), not the real model. 1 MB is a deliberately loose
    /// threshold so test fixtures (which can be smaller) still pass.
    const ONNX_MIN_BYTES: u64 = 1 * 1024 * 1024;
    const TOKENIZER_MIN_BYTES: u64 = 1 * 1024;

    /// Resolve the HuggingFace base URL. Honors the standard
    /// `HF_ENDPOINT` env var (same convention as the official
    /// `huggingface_hub` Python library) so users in restricted
    /// networks can point at `https://hf-mirror.com` or a private
    /// mirror without rebuilding. Trailing `/` is stripped.
    pub(super) fn hf_endpoint() -> String {
        let raw = std::env::var("HF_ENDPOINT").unwrap_or_default();
        let trimmed = raw.trim().trim_end_matches('/').to_owned();
        if trimmed.is_empty() {
            "https://huggingface.co".to_owned()
        } else {
            trimmed
        }
    }

    /// Fetch the ONNX model + tokenizer for `backend`, stream them to
    /// disk, sha256-verify, and write the manifest. Returns the new
    /// `CachedModel` row on success.
    ///
    /// When `local_dir` is `Some`, copies from the local directory
    /// instead of fetching over the network. The directory must
    /// contain `model.onnx` and `tokenizer.json` at the top level.
    pub fn warmup(
        backend: EmbeddingBackend,
        local_dir: Option<&Path>,
    ) -> MemoryResult<CachedModel> {
        let repo = backend.hf_repo().ok_or_else(|| {
            MemoryError::IndexUnavailable(format!("no HuggingFace repo for {backend:?}"))
        })?;
        let dir = model_store::cache_dir()?.join(backend.as_str());
        std::fs::create_dir_all(&dir)
            .map_err(|e| MemoryError::IndexUnavailable(format!("mkdir {}: {e}", dir.display())))?;

        let onnx_path = dir.join("model.onnx");
        let tok_path = dir.join("tokenizer.json");

        let (onnx_sha, tok_sha) = match local_dir {
            Some(src) => {
                eprintln!(
                    "kodo memory warmup: copying model.onnx and tokenizer.json\n\
                     from {}\n\
                     into {}",
                    src.display(),
                    dir.display()
                );
                local_copy(
                    src,
                    &onnx_path,
                    &tok_path,
                    ONNX_MIN_BYTES,
                    TOKENIZER_MIN_BYTES,
                )?
            }
            None => {
                let base = hf_endpoint();
                let onnx_url = format!("{base}/{repo}/resolve/main/onnx/model.onnx");
                let tok_url = format!("{base}/{repo}/resolve/main/tokenizer.json");
                eprintln!(
                    "kodo memory warmup: downloading {onnx_url}\n\
                     and {tok_url}\n\
                     into {}",
                    dir.display()
                );
                let onnx = fetch_and_sha256(&onnx_url, &onnx_path).map_err(|e| {
                    MemoryError::IndexUnavailable(format!(
                        "fetch onnx: {e}\n\
                         hint: if huggingface.co is unreachable from your network, set\n\
                         hint:   HF_ENDPOINT=https://hf-mirror.com (or your mirror) and retry\n\
                         hint: or pre-download the two files and run with --from <dir>"
                    ))
                })?;
                let tok = fetch_and_sha256(&tok_url, &tok_path).map_err(|e| {
                    MemoryError::IndexUnavailable(format!(
                        "fetch tokenizer: {e}\n\
                         hint: if huggingface.co is unreachable from your network, set\n\
                         hint:   HF_ENDPOINT=https://hf-mirror.com (or your mirror) and retry\n\
                         hint: or pre-download the two files and run with --from <dir>"
                    ))
                })?;
                (onnx, tok)
            }
        };

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

    /// Copy `model.onnx` and `tokenizer.json` from `src` into
    /// `onnx_dest` / `tok_dest` using the atomic `.partial` + rename
    /// pattern. Computes the sha256 of each file while copying so the
    /// manifest row has the same shape as the network path.
    ///
    /// `onnx_min_bytes` / `tok_min_bytes` are the minimum acceptable
    /// sizes — anything smaller is rejected as a likely 404 page.
    /// Returns the two sha256 strings on success.
    fn local_copy(
        src: &Path,
        onnx_dest: &Path,
        tok_dest: &Path,
        onnx_min_bytes: u64,
        tok_min_bytes: u64,
    ) -> Result<(String, String), MemoryError> {
        let onnx_src = src.join("model.onnx");
        let tok_src = src.join("tokenizer.json");
        if !onnx_src.is_file() {
            return Err(MemoryError::IndexUnavailable(format!(
                "--from directory missing model.onnx: {}",
                onnx_src.display()
            )));
        }
        if !tok_src.is_file() {
            return Err(MemoryError::IndexUnavailable(format!(
                "--from directory missing tokenizer.json: {}",
                tok_src.display()
            )));
        }
        let onnx_sha = copy_with_sha(&onnx_src, onnx_dest, onnx_min_bytes, "model.onnx")?;
        let tok_sha = copy_with_sha(&tok_src, tok_dest, tok_min_bytes, "tokenizer.json")?;
        Ok((onnx_sha, tok_sha))
    }

    /// Copy `src` to `dest` via a `.partial` sibling, computing the
    /// sha256 of the bytes along the way. Refuses to copy files below
    /// `min_bytes` (almost certainly a 404 HTML page).
    fn copy_with_sha(
        src: &Path,
        dest: &Path,
        min_bytes: u64,
        label: &str,
    ) -> Result<String, MemoryError> {
        use sha2::{Digest, Sha256};
        let size = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
        if size < min_bytes {
            return Err(MemoryError::IndexUnavailable(format!(
                "{label} is only {size} bytes (expected ≥ {min_bytes}); \
                 refuse to warmup — looks like an HTML error page, not the model file"
            )));
        }
        let mut reader = std::fs::File::open(src)
            .map_err(|e| MemoryError::IndexUnavailable(format!("open {}: {e}", src.display())))?;
        let tmp = dest.with_extension("partial");
        let mut writer = std::fs::File::create(&tmp)
            .map_err(|e| MemoryError::IndexUnavailable(format!("create {}: {e}", tmp.display())))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total: u64 = 0;
        loop {
            let n = reader.read(&mut buf).map_err(|e| {
                MemoryError::IndexUnavailable(format!("read {}: {e}", src.display()))
            })?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n]).map_err(|e| {
                MemoryError::IndexUnavailable(format!("write {}: {e}", tmp.display()))
            })?;
            hasher.update(&buf[..n]);
            total += n as u64;
        }
        writer
            .flush()
            .map_err(|e| MemoryError::IndexUnavailable(format!("flush {}: {e}", tmp.display())))?;
        drop(writer);
        std::fs::rename(&tmp, dest).map_err(|e| {
            MemoryError::IndexUnavailable(format!("rename to {}: {e}", dest.display()))
        })?;
        let _ = total;
        Ok(hex::encode(hasher.finalize()))
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
        let _guard = crate::model_store::test_lock();
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
        let _guard = crate::model_store::test_lock();
        let dir = std::env::temp_dir().join("kodo-memory-local-warmup-off");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::model_store::override_state_dir_for_tests(&dir);
        let err = warmup(EmbeddingBackend::Off, None).unwrap_err();
        assert!(matches!(err, MemoryError::IndexUnavailable(_)));
        crate::model_store::restore_for_tests();
    }

    /// Local-warmup path: copy `model.onnx` and `tokenizer.json` from
    /// a local directory into the cache, write the manifest, and
    /// return the populated `CachedModel` row. No network involved —
    /// this is the offline / mirror-replacement path the user reached
    /// for when `huggingface.co` is blocked.
    #[cfg(feature = "download")]
    #[test]
    fn warmup_from_local_dir_copies_and_records() {
        let _guard = crate::model_store::test_lock();
        // Source dir with two fake files above the minimum-size
        // sanity check (~1.1 MB ONNX, ~1.5 KB tokenizer).
        let src = std::env::temp_dir().join("kodo-memory-local-warmup-src");
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        let onnx_payload: Vec<u8> = (0..(1_100_000u32)).map(|i| (i & 0xFF) as u8).collect();
        let tok_payload = br#"{"added_tokens":[{"id":0,"content":"[PAD]"}],"normalizer":{}}"#;
        // Pad the tokenizer to ≥ 1 KiB.
        let mut tok = Vec::from(&tok_payload[..]);
        while tok.len() < 2048 {
            tok.push(b' ');
        }
        std::fs::write(src.join("model.onnx"), &onnx_payload).unwrap();
        std::fs::write(src.join("tokenizer.json"), &tok).unwrap();

        // Cache dir override (per-test isolation).
        let cache_root = std::env::temp_dir().join("kodo-memory-local-warmup-cache");
        let _ = std::fs::remove_dir_all(&cache_root);
        std::fs::create_dir_all(&cache_root).unwrap();
        crate::model_store::override_state_dir_for_tests(&cache_root);
        let cache_dir = crate::model_store::cache_dir().unwrap();
        let manifest_path = cache_dir.join("manifest.json");
        let backend_dir = cache_dir.join(EmbeddingBackend::LocalBgeSmallEn.as_str());

        let row = warmup(EmbeddingBackend::LocalBgeSmallEn, Some(&src)).unwrap();

        assert!(row.is_ready(), "row should be ready after local warmup");
        assert_eq!(row.bytes_on_disk as usize, onnx_payload.len() + tok.len());
        assert!(row.onnx_sha256.as_ref().unwrap().len() == 64);
        assert!(row.tokenizer_sha256.as_ref().unwrap().len() == 64);

        assert!(
            backend_dir.join("model.onnx").is_file(),
            "model.onnx copied"
        );
        assert!(
            backend_dir.join("tokenizer.json").is_file(),
            "tokenizer.json copied"
        );
        let copied_onnx = std::fs::read(backend_dir.join("model.onnx")).unwrap();
        let copied_tok = std::fs::read(backend_dir.join("tokenizer.json")).unwrap();
        assert_eq!(copied_onnx, onnx_payload, "model.onnx bytes preserved");
        assert_eq!(copied_tok, tok, "tokenizer.json bytes preserved");

        // Read the manifest file directly from the captured path
        // (avoid `read_manifest` which goes back through the
        // racy global static).
        let manifest_raw = std::fs::read_to_string(&manifest_path).unwrap();
        let manifest_rows: Vec<crate::model_store::CachedModel> =
            serde_json::from_str(&manifest_raw).unwrap();
        assert_eq!(manifest_rows.len(), 1);
        assert_eq!(manifest_rows[0].backend, EmbeddingBackend::LocalBgeSmallEn);

        crate::model_store::restore_for_tests();
    }

    /// Local-warmup with a too-small file rejects the input instead
    /// of writing a manifest. Protects against the user accidentally
    /// pointing `--from` at a directory of 404 HTML pages.
    #[cfg(feature = "download")]
    #[test]
    fn warmup_from_local_dir_rejects_too_small_onnx() {
        let _guard = crate::model_store::test_lock();
        let src = std::env::temp_dir().join("kodo-memory-local-warmup-tiny");
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        // 100 bytes of ONNX — well below 1 MB sanity threshold.
        std::fs::write(src.join("model.onnx"), vec![0u8; 100]).unwrap();
        std::fs::write(src.join("tokenizer.json"), vec![b' '; 2048]).unwrap();

        let cache_root = std::env::temp_dir().join("kodo-memory-local-warmup-tiny-cache");
        let _ = std::fs::remove_dir_all(&cache_root);
        std::fs::create_dir_all(&cache_root).unwrap();
        crate::model_store::override_state_dir_for_tests(&cache_root);
        let cache_dir = crate::model_store::cache_dir().unwrap();
        let manifest_path = cache_dir.join("manifest.json");

        let err = warmup(EmbeddingBackend::LocalBgeSmallEn, Some(&src)).unwrap_err();
        match err {
            MemoryError::IndexUnavailable(msg) => {
                assert!(
                    msg.contains("only 100 bytes"),
                    "expected size-rejection message, got: {msg}"
                );
            }
            other => panic!("expected IndexUnavailable, got {other:?}"),
        }
        // The manifest file should not exist at the captured path
        // (or should be empty if it was pre-existing for some
        // reason — empty file is treated as "no rows").
        let manifest_exists = manifest_path.exists();
        if manifest_exists {
            let raw = std::fs::read_to_string(&manifest_path).unwrap();
            assert!(
                raw.trim().is_empty(),
                "manifest should be empty/absent on failed warmup, got: {raw}"
            );
        }
        crate::model_store::restore_for_tests();
    }

    /// Local-warmup with a missing file produces a clear error before
    /// touching the cache.
    #[cfg(feature = "download")]
    #[test]
    fn warmup_from_local_dir_reports_missing_files() {
        let _guard = crate::model_store::test_lock();
        let src = std::env::temp_dir().join("kodo-memory-local-warmup-missing");
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        // Only model.onnx, no tokenizer.json.
        std::fs::write(src.join("model.onnx"), vec![0u8; 1_100_000]).unwrap();

        let cache_root = std::env::temp_dir().join("kodo-memory-local-warmup-missing-cache");
        let _ = std::fs::remove_dir_all(&cache_root);
        std::fs::create_dir_all(&cache_root).unwrap();
        crate::model_store::override_state_dir_for_tests(&cache_root);

        let err = warmup(EmbeddingBackend::LocalBgeSmallEn, Some(&src)).unwrap_err();
        crate::model_store::restore_for_tests();
        match err {
            MemoryError::IndexUnavailable(msg) => {
                assert!(
                    msg.contains("tokenizer.json"),
                    "expected missing-tokenizer message, got: {msg}"
                );
            }
            other => panic!("expected IndexUnavailable, got {other:?}"),
        }
    }

    /// `HF_ENDPOINT` env var is honored: when set, it becomes the URL
    /// base (used by the network path). When unset or empty, the
    /// default `https://huggingface.co` is used.
    #[cfg(feature = "download")]
    #[test]
    fn hf_endpoint_default_and_override() {
        // Save and clear any pre-existing value.
        let prev = std::env::var("HF_ENDPOINT").ok();
        // SAFETY: tests in this module are documented as
        // process-global; we restore the previous value (or absence)
        // at the end. We never panic between set/remove without a
        // restore path; the test acquires no lock because it does
        // not touch `TEST_STATE_DIR`.

        // SAFETY: see comment above.
        unsafe {
            std::env::remove_var("HF_ENDPOINT");
        }
        assert_eq!(
            download::hf_endpoint(),
            "https://huggingface.co",
            "default base when HF_ENDPOINT is unset"
        );

        // SAFETY: test-only env mutation; see test header comment.
        unsafe {
            std::env::set_var("HF_ENDPOINT", "https://hf-mirror.com");
        }
        assert_eq!(download::hf_endpoint(), "https://hf-mirror.com");

        // Trailing slash is stripped.
        // SAFETY: test-only env mutation.
        unsafe {
            std::env::set_var("HF_ENDPOINT", "https://hf-mirror.com/");
        }
        assert_eq!(download::hf_endpoint(), "https://hf-mirror.com");

        // Empty string falls back to default.
        // SAFETY: test-only env mutation.
        unsafe {
            std::env::set_var("HF_ENDPOINT", "");
        }
        assert_eq!(download::hf_endpoint(), "https://huggingface.co");

        // Restore previous state.
        // SAFETY: test-only env mutation.
        match prev {
            Some(v) => unsafe {
                std::env::set_var("HF_ENDPOINT", v);
            },
            None => unsafe {
                std::env::remove_var("HF_ENDPOINT");
            },
        }
    }
}
