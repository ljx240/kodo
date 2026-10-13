-- KME schema v2. Doc: docs/memory/KODO_MEMORY_IMPLEMENTATION.md §7.4.
-- A tombstone is the only way to permanently remove a memory without
-- risking its reappearance during outbox replay (the outbox worker must
-- skip rows whose memory_id appears here).

CREATE TABLE IF NOT EXISTS memory_tombstones (
    memory_id   TEXT PRIMARY KEY,
    reason      TEXT NOT NULL,
    tombstoned_at INTEGER NOT NULL,
    tombstoned_by TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tombstones_at ON memory_tombstones(tombstoned_at DESC);
