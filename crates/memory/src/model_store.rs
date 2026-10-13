//! Local model cache for the optional `onnx` embedding backends.
//!
//! The download itself lives in the `onnx` feature; this module is the
//! always-available bookkeeping layer that answers three questions:
//!
//! 1. Where do the weights live? — `cache_dir()` (under `state_dir()`).
//! 2. Which backend is ready right now? — `status(backend)`.
//! 3. What was the last-known state? — read/write `manifest.json`.
//!
//! No network code here. The `onnx` feature wires `ort` + `ureq` and
//! fills the cache; without that feature, `warmup()` is a no-op that
//! reports "feature disabled".

use crate::error::{MemoryError, MemoryResult};
use crate::model::EmbeddingBackend;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One cache entry. `None` for any field means the asset is missing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedModel {
    pub backend: EmbeddingBackend,
    pub model_id: String,
    pub dim: usize,
    /// sha256 of the ONNX file in hex. Lowercase, 64 chars.
    pub onnx_sha256: Option<String>,
    /// sha256 of `tokenizer.json` in hex. Lowercase, 64 chars.
    pub tokenizer_sha256: Option<String>,
    /// Wall-clock seconds when warmup last completed (kodo_core::now_secs()).
    pub warmed_at: Option<u64>,
    /// Bytes written for the model + tokenizer. Used by the GUI to render
    /// "cache size".
    pub bytes_on_disk: u64,
}

impl CachedModel {
    pub fn is_ready(&self) -> bool {
        self.onnx_sha256.is_some() && self.tokenizer_sha256.is_some() && self.warmed_at.is_some()
    }
}

use std::sync::Mutex;
static TEST_STATE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Test-only: install a fake `state_dir()` for the rest of the process.
/// Pair every call with `restore_for_tests()`. Process-global, not
/// per-thread — tests in this module must not run concurrently with
/// each other or with any other test that mutates the cache dir.
pub fn override_state_dir_for_tests(dir: &Path) {
    let mut guard = TEST_STATE_DIR.lock().unwrap();
    *guard = Some(dir.to_path_buf());
}

pub fn restore_for_tests() {
    let mut guard = TEST_STATE_DIR.lock().unwrap();
    *guard = None;
}

fn effective_cache_dir() -> MemoryResult<PathBuf> {
    if let Some(override_dir) = TEST_STATE_DIR.lock().unwrap().clone() {
        return Ok(override_dir.join("memory").join("models").join("v1"));
    }
    cache_dir()
}

/// Local on-disk directory that holds the optional embedding model
/// weights. Always returns a path under the platform `state_dir()`.
pub fn cache_dir() -> MemoryResult<PathBuf> {
    let base = kodo_core::settings::state_dir().ok_or(MemoryError::IndexUnavailable(
        "no state dir; HOME is unset".to_owned(),
    ))?;
    Ok(base.join("memory").join("models").join("v1"))
}

/// Where the manifest.json lives. Single file shared by all backends.
pub fn manifest_path() -> MemoryResult<PathBuf> {
    Ok(cache_dir()?.join("manifest.json"))
}

/// Read the whole manifest, or return an empty default when the file
/// is absent / unreadable. Never errors on "file missing" — that is
/// the cold-start case.
pub fn read_manifest() -> MemoryResult<Vec<CachedModel>> {
    let path = effective_cache_dir()?.join("manifest.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        MemoryError::IndexUnavailable(format!("read manifest {}: {e}", path.display()))
    })?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).map_err(|e| {
        MemoryError::IndexUnavailable(format!("parse manifest {}: {e}", path.display()))
    })
}

fn write_manifest(rows: &[CachedModel]) -> MemoryResult<()> {
    let dir = effective_cache_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| MemoryError::IndexUnavailable(format!("mkdir {}: {e}", dir.display())))?;
    let path = dir.join("manifest.json");
    let raw = serde_json::to_string_pretty(rows)
        .map_err(|e| MemoryError::IndexUnavailable(format!("serialize manifest: {e}")))?;
    std::fs::write(&path, raw).map_err(|e| {
        MemoryError::IndexUnavailable(format!("write manifest {}: {e}", path.display()))
    })?;
    Ok(())
}

/// Look up one backend in the manifest. Returns a synthetic "not ready"
/// row when absent so callers can compare on shape.
pub fn status(backend: EmbeddingBackend) -> MemoryResult<CachedModel> {
    let rows = read_manifest()?;
    if let Some(row) = rows.into_iter().find(|r| r.backend == backend) {
        return Ok(row);
    }
    Ok(CachedModel {
        backend,
        model_id: backend.model_id().unwrap_or("").to_owned(),
        dim: backend.dim(),
        onnx_sha256: None,
        tokenizer_sha256: None,
        warmed_at: None,
        bytes_on_disk: 0,
    })
}

/// Update or insert one row in the manifest. Used by the `onnx` feature
/// after a successful warmup.
pub fn upsert(row: CachedModel) -> MemoryResult<()> {
    let mut rows = read_manifest()?;
    if let Some(slot) = rows.iter_mut().find(|r| r.backend == row.backend) {
        *slot = row;
    } else {
        rows.push(row);
    }
    write_manifest(&rows)
}

/// Remove one backend from the manifest + delete its on-disk assets.
/// Refuses to touch `Off` / `LocalHash` (they have no assets).
pub fn clear(backend: EmbeddingBackend) -> MemoryResult<()> {
    if !backend.requires_warmup() {
        return Ok(());
    }
    let dir = effective_cache_dir()?.join(backend.as_str());
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| MemoryError::IndexUnavailable(format!("rm {}: {e}", dir.display())))?;
    }
    let mut rows = read_manifest()?;
    rows.retain(|r| r.backend != backend);
    write_manifest(&rows)
}

/// Total bytes occupied by the cache directory tree. Cheap (one
/// `read_dir` per level); used by the GUI status surface.
pub fn cache_size_bytes() -> MemoryResult<u64> {
    let dir = effective_cache_dir()?;
    if !dir.exists() {
        return Ok(0);
    }
    Ok(scan_size(&dir))
}

fn scan_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            total += scan_size(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}

/// Warmup entry point used by CLI and Tauri. Without the `download`
/// feature this is a no-op that reports the feature as disabled —
/// callers decide what to do (SettingsPage shows "feature off" chip,
/// CLI prints an explanation).
#[cfg(not(feature = "download"))]
pub fn warmup(_backend: EmbeddingBackend) -> MemoryResult<CachedModel> {
    Err(MemoryError::IndexUnavailable(
        "embedding warmup unavailable: build with `--features download`".to_owned(),
    ))
}

/// Warmup entry point used by CLI and Tauri. With the `download`
/// feature the real implementation lives in `embedding_local::warmup`
/// and re-exports through this module so the call site is one line.
#[cfg(feature = "download")]
pub fn warmup(backend: EmbeddingBackend) -> MemoryResult<CachedModel> {
    crate::embedding_local::warmup(backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("kodo-memory-model-store-{tag}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn manifest_round_trip() {
        let dir = fresh_dir("rt");
        override_state_dir_for_tests(&dir);
        let row = CachedModel {
            backend: EmbeddingBackend::LocalBgeSmallEn,
            model_id: "Xenova/bge-small-en-v1.5".to_owned(),
            dim: 384,
            onnx_sha256: Some("a".repeat(64)),
            tokenizer_sha256: Some("b".repeat(64)),
            warmed_at: Some(1_700_000_000),
            bytes_on_disk: 33_000_000,
        };
        upsert(row.clone()).unwrap();
        let got = status(EmbeddingBackend::LocalBgeSmallEn).unwrap();
        assert_eq!(got, row);
        assert!(got.is_ready());
        restore_for_tests();
    }

    #[test]
    fn status_missing_backend_is_synthetic() {
        let dir = fresh_dir("synthetic");
        override_state_dir_for_tests(&dir);
        let got = status(EmbeddingBackend::LocalMiniLmL6).unwrap();
        assert!(!got.is_ready());
        assert_eq!(got.dim, 384);
        assert!(got.onnx_sha256.is_none());
        restore_for_tests();
    }

    #[test]
    fn clear_off_is_noop() {
        let dir = fresh_dir("noop");
        override_state_dir_for_tests(&dir);
        clear(EmbeddingBackend::Off).unwrap();
        // No panic, no file written under the override.
        assert!(!dir
            .join("memory")
            .join("models")
            .join("v1")
            .join("manifest.json")
            .exists());
        restore_for_tests();
    }
}
