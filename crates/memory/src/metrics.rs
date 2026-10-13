//! Atomic counters. No external sink — only the in-process counter is
//! read; the GUI surfaces them through a separate "memory_status"
//! command that calls `Metrics::snapshot()`.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct Metrics {
    pub recall_calls: AtomicU64,
    pub recall_hits: AtomicU64,
    pub recall_empty: AtomicU64,
    pub outbox_enqueued: AtomicU64,
    pub outbox_compiled: AtomicU64,
    pub outbox_failed: AtomicU64,
    pub outbox_deduped: AtomicU64,
    pub marks_stale: AtomicU64,
    pub fail_open: AtomicU64,
    pub tombstones: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            recall_calls: self.recall_calls.load(Ordering::Relaxed),
            recall_hits: self.recall_hits.load(Ordering::Relaxed),
            recall_empty: self.recall_empty.load(Ordering::Relaxed),
            outbox_enqueued: self.outbox_enqueued.load(Ordering::Relaxed),
            outbox_compiled: self.outbox_compiled.load(Ordering::Relaxed),
            outbox_failed: self.outbox_failed.load(Ordering::Relaxed),
            outbox_deduped: self.outbox_deduped.load(Ordering::Relaxed),
            marks_stale: self.marks_stale.load(Ordering::Relaxed),
            fail_open: self.fail_open.load(Ordering::Relaxed),
            tombstones: self.tombstones.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct MetricsSnapshot {
    pub recall_calls: u64,
    pub recall_hits: u64,
    pub recall_empty: u64,
    pub outbox_enqueued: u64,
    pub outbox_compiled: u64,
    pub outbox_failed: u64,
    pub outbox_deduped: u64,
    pub marks_stale: u64,
    pub fail_open: u64,
    pub tombstones: u64,
}
