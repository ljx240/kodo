# Memory Engine Architecture

> Kodo Memory Engine (KME) — design overview of the `crates/memory` crate
> and its placement in the rest of the Kodo workspace.

## Position in the stack

```
+------------------------------------------------------+
|  apps/desktop (Tauri + React)  |  crates/cli         |
+------------------------------------------------------+
|  crates/agent (RunRequest now carries an optional    |
|  MemoryHandle; recall happens in the user-message    |
|  observation block, never in the system block)       |
+------------------------------------------------------+
|  crates/memory  ← one crate, one DB, one trait       |
|    MemoryAccess ← Arc<dyn>, exposed as MemoryHandle  |
|    SqliteMemory ← default offline backend            |
+------------------------------------------------------+
|  crates/core (kodo_core::redact shared with the      |
|  memory sanitizer; settings::state_dir for the path) |
+------------------------------------------------------+
```

The shell layer (Tauri `run.rs`, CLI `turn.rs`) is responsible for opening
the handle once at startup and threading it into every `RunRequest`. The
agent loop does not open DBs, does not run schema migrations, and does not
know the on-disk path — it only calls `MemoryAccess` methods through the
`MemoryHandle` it was given.

## Trait surface

```rust
pub trait MemoryAccess: Send + Sync {
    fn recall(&self, query: &MemoryQuery) -> Result<MemoryPack, MemoryError>;
    fn observe(&self, event: &SafeMemoryEvent) -> Result<bool, MemoryError>;
    fn mark_changed_paths(&self, scope: &MemoryScope, paths: &[String])
        -> Result<(), MemoryError>;
    fn feedback(&self, usage: &MemoryUsage) -> Result<(), MemoryError>;
    fn list(&self, filter: &MemoryFilter) -> Result<Vec<MemoryEntry>, MemoryError>;
    fn inspect(&self, id: &str) -> Result<Option<MemoryEntry>, MemoryError>;
    fn disable(&self, id: &str) -> Result<(), MemoryError>;
    fn tombstone(&self, id: &str, reason: &str, by: &str) -> Result<(), MemoryError>;
    fn is_tombstoned(&self, id: &str) -> Result<bool, MemoryError>;
    fn metrics(&self) -> MetricsSnapshot;
}
```

Every call is best-effort from the agent's perspective: the trait returns
`Result`, but the agent loop only calls `handle.recall(...)` once at the
top of `run()` and falls back to an empty `MemoryPack` on
`StorageUnavailable | IndexUnavailable | ScopeDenied`. Tool flows call
`mark_changed_paths` and never wait for a return value.

## Schema (high-level)

- `memory_entries` — six memory kinds, two authorities, four trust levels,
  one scope, one status. 22 columns, indexed on `project_id`,
  `repository_id`, `branch`, `task_id`, `last_used_at`, `content_hash`,
  `status`.
- `memory_evidence` — paths, URIs, file digests, kind tags.  Joins
  one-to-many with `memory_entries`.
- `memory_links` — `kind` and `related_id` for graph edges (used by P2
  procedural promotion).
- `memory_outbox` — append-only event log; `event_id` is `UNIQUE` for
  at-least-once idempotency.
- `memory_tombstones` — separate "never revive this id" table. P0
  writes via the outbox compiler; the doctor surfaces the count.
- `memory_usage` — feedback log for `feedback(MemoryUsage)`. Indexed on
  `memory_id, used_at`.
- `memory_entries_fts` — FTS5 index on `title + abstract_text`. The DAO
  keeps the FTS row in sync inside the same transaction as the
  `memory_entries` write.

## Recall pipeline

```
                query.text + related_paths
                          │
                          ▼
        scope filter (project/repository/branch)
                          │
        ┌─────────────────┼─────────────────┐
        ▼                 ▼                 ▼
   FTS5 (BM25)    exact (LIKE on body)   recent
        │                 │                 │
        └──────── RRF(k=60) ────────────────┘
                          │
                          ▼
            budget → MemoryPack (empty fallback)
                          │
                          ▼
       injected into user-message observation block
              (NEVER the system block — invariant §9.5)
```

P0 ships FTS + exact + recent with RRF fusion and a 2000-token default
budget. The embedding channel is a no-op stub in P0; the trait exists so
P1+ can plug a local provider without touching the call site.

## Failure posture

- All trait methods are `Result`; the agent loop never `unwrap`s.
- `MemoryError::is_recoverable` decides whether the agent continues with
  empty data (`true`) or hard-fails (`false`). The classification lives
  in `crates/memory/src/error.rs` and is unit-tested.
- Every `fail-open` event increments `metrics.fail_open`; the doctor
  surfaces a snapshot of the counters.
- The on-disk DB is read-write by the shell process and
  `SQLITE_OPEN_READ_ONLY` for any side-channel read (none today, but the
  store accepts the flag in `Store::open` for future telemetry tools).
