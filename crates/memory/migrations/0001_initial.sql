-- KME schema v1. Doc: docs/memory/KODO_MEMORY_IMPLEMENTATION.md §7.
-- All PRAGMAs are applied from the connection layer, not this file.

CREATE TABLE IF NOT EXISTS schema_migrations (
    version     INTEGER PRIMARY KEY,
    applied_at  INTEGER NOT NULL,
    description TEXT NOT NULL
);

-- The headline table. Body is JSON. Status/Kind/Trust/Authority/Privacy/Mode are
-- stored as TEXT (the on-disk names in model.rs). Booleans are INTEGER 0/1.
-- Strings default to '' so we never have to deal with NULL in the DAO.
CREATE TABLE IF NOT EXISTS memory_entries (
    id                TEXT PRIMARY KEY,
    kind              TEXT NOT NULL,
    status            TEXT NOT NULL,
    authority         TEXT NOT NULL,
    title             TEXT NOT NULL,
    abstract_text     TEXT NOT NULL,
    body_json         TEXT NOT NULL,
    content_hash      TEXT NOT NULL,

    source_trust      TEXT NOT NULL,
    confidence        REAL NOT NULL DEFAULT 0.5,
    verified          INTEGER NOT NULL DEFAULT 0,
    candidate_skill   INTEGER NOT NULL DEFAULT 0,

    -- Scope columns. Each is nullable; the application layer turns them into
    -- a MemoryScope and enforces the `matches` check.
    project_id        TEXT,
    repository_id     TEXT,
    branch            TEXT,
    worktree_id       TEXT,
    session_id        TEXT,
    task_id           TEXT,
    subtask_id        TEXT,

    -- Free-form gating + provenance.
    conditions_json   TEXT NOT NULL DEFAULT '[]',
    source_uris_json  TEXT NOT NULL DEFAULT '[]',

    -- Time / usage.
    valid_from        INTEGER,
    valid_until       INTEGER,
    last_verified_at  INTEGER,
    last_used_at      INTEGER,
    use_count         INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,

    superseded_by     TEXT,
    version           INTEGER NOT NULL DEFAULT 1
);

-- Hot lookups.
CREATE INDEX IF NOT EXISTS idx_entries_scope_project
    ON memory_entries(project_id);
CREATE INDEX IF NOT EXISTS idx_entries_scope_repo
    ON memory_entries(repository_id);
CREATE INDEX IF NOT EXISTS idx_entries_scope_session
    ON memory_entries(session_id);
CREATE INDEX IF NOT EXISTS idx_entries_status
    ON memory_entries(status);
CREATE INDEX IF NOT EXISTS idx_entries_kind
    ON memory_entries(kind);
CREATE INDEX IF NOT EXISTS idx_entries_content_hash
    ON memory_entries(content_hash);
CREATE INDEX IF NOT EXISTS idx_entries_last_used_at
    ON memory_entries(last_used_at DESC);
CREATE INDEX IF NOT EXISTS idx_entries_updated_at
    ON memory_entries(updated_at DESC);

-- Memory-to-memory relations. Used for supersede chains and "see also" links.
-- 'kind' is the relation kind (supersede, derive, related, conflict).
CREATE TABLE IF NOT EXISTS memory_links (
    src_id     TEXT NOT NULL,
    dst_id     TEXT NOT NULL,
    kind       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (src_id, dst_id, kind),
    FOREIGN KEY (src_id) REFERENCES memory_entries(id) ON DELETE CASCADE,
    FOREIGN KEY (dst_id) REFERENCES memory_entries(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_links_dst ON memory_links(dst_id);

-- Per-path file digests captured when the memory is written. When
-- `mark_changed_paths` runs and the new digest doesn't match, the entry is
-- marked stale. `uri` is a kodo:// or git:// reference.
CREATE TABLE IF NOT EXISTS memory_evidence (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_id   TEXT NOT NULL,
    path        TEXT NOT NULL,
    digest      TEXT NOT NULL,
    uri         TEXT,
    captured_at INTEGER NOT NULL,
    FOREIGN KEY (memory_id) REFERENCES memory_entries(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_evidence_memory ON memory_evidence(memory_id);
CREATE INDEX IF NOT EXISTS idx_evidence_path   ON memory_evidence(path);

-- Outbox. The agent writes here; the compiler promotes rows to entries.
-- Replay is idempotent on `event_id` (UNIQUE).
CREATE TABLE IF NOT EXISTS memory_outbox (
    event_id       TEXT PRIMARY KEY,
    session_id     TEXT NOT NULL,
    turn_seq       INTEGER NOT NULL,
    item_seq       INTEGER NOT NULL,
    phase          TEXT NOT NULL,
    kind           TEXT NOT NULL,
    source_digest  TEXT NOT NULL,
    observed_at    INTEGER NOT NULL,
    project_path   TEXT,
    label          TEXT NOT NULL,
    body           TEXT,
    extra_json     TEXT NOT NULL DEFAULT '{}',
    attempts       INTEGER NOT NULL DEFAULT 0,
    last_error     TEXT,
    last_attempted INTEGER,
    enqueued_at    INTEGER NOT NULL,
    processed_at   INTEGER,
    status         TEXT NOT NULL DEFAULT 'pending'
    -- 'pending' | 'compiled' | 'failed' | 'superseded'
);

CREATE INDEX IF NOT EXISTS idx_outbox_status ON memory_outbox(status, enqueued_at);
CREATE INDEX IF NOT EXISTS idx_outbox_session ON memory_outbox(session_id, turn_seq, item_seq);

-- Per-recall usage feedback. P0 just stores it; P2 reads it for policy.
CREATE TABLE IF NOT EXISTS memory_usage (
    run_id      TEXT NOT NULL,
    memory_id   TEXT NOT NULL,
    operation   TEXT NOT NULL,
    score       REAL,
    token_cost  INTEGER,
    elapsed_ms  INTEGER,
    at          INTEGER NOT NULL,
    PRIMARY KEY (run_id, memory_id, operation)
);

CREATE INDEX IF NOT EXISTS idx_usage_memory ON memory_usage(memory_id);
CREATE INDEX IF NOT EXISTS idx_usage_at     ON memory_usage(at DESC);

-- FTS5 virtual table for retrieval. `title` and `abstract_text` are
-- mirrored; `body` is intentionally NOT included (too noisy for BM25;
-- the exact-token path in retrieval.rs hits body instead). Triggers keep
-- it in sync. We deliberately keep a copy of the indexed columns
-- (no `content=''`) so that `'rebuild'` works and the doctor command
-- can recover from drift.
CREATE VIRTUAL TABLE IF NOT EXISTS memory_entries_fts USING fts5(
    title,
    abstract_text,
    tokenize='unicode61 remove_diacritics 2'
);

-- FTS sync is done explicitly in the DAO (see store.rs::upsert_entry
-- and tombstone). Triggers fire inside the same statement that mutated
-- `memory_entries`, and we want the FTS update to live in the same
-- transaction as the rest of the upsert so it can roll back together.
