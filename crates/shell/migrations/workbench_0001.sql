-- Workbench v1.
--
-- Per-install app state, UI preferences, and the business data owned by the
-- first-party apps (待办 / 日历 / 成果中心). Each app keeps its own tables —
-- the host does not force the todo model on every app.
--
-- Conventions used throughout:
--   * `version`   — optimistic-concurrency counter, bumped on every edit.
--   * `request_id`— client idempotency key; a repeated create returns the
--                   original row instead of inserting a second one.
--   * timestamps  — unix seconds.

CREATE TABLE IF NOT EXISTS app_preferences (
    app_id           TEXT PRIMARY KEY,
    enabled          INTEGER NOT NULL,
    favorite         INTEGER NOT NULL,
    sort_order       INTEGER NOT NULL,
    ai_enabled       INTEGER NOT NULL,
    schedule_enabled INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

-- Workbench UI preferences (overview visibility, default todo view, calendar
-- view, work hours…). Stored server-side so they survive a webview reload.
CREATE TABLE IF NOT EXISTS workbench_prefs (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS todos (
    id           TEXT PRIMARY KEY,
    title        TEXT    NOT NULL,
    notes        TEXT    NOT NULL DEFAULT '',
    -- Single source of truth for the board column, the list and the
    -- "today" / "open" filters. `completed_at` is informational only.
    status       TEXT    NOT NULL,
    priority     TEXT    NOT NULL,
    -- Local due date `YYYY-MM-DD`; NULL puts the todo in the inbox.
    due_date     TEXT,
    parent_id    TEXT    REFERENCES todos(id) ON DELETE SET NULL,
    archived     INTEGER NOT NULL DEFAULT 0,
    completed_at INTEGER,
    sort_order   INTEGER NOT NULL DEFAULT 0,
    source       TEXT    NOT NULL DEFAULT 'manual',
    version      INTEGER NOT NULL DEFAULT 1,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    request_id   TEXT
);

CREATE INDEX IF NOT EXISTS todos_board_idx ON todos(archived, status, sort_order, created_at);
CREATE INDEX IF NOT EXISTS todos_due_idx ON todos(due_date);
CREATE UNIQUE INDEX IF NOT EXISTS todos_request_idx
    ON todos(request_id) WHERE request_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS calendar_events (
    id         TEXT PRIMARY KEY,
    title      TEXT    NOT NULL,
    notes      TEXT    NOT NULL DEFAULT '',
    -- Wall-clock fields, expressed in `timezone`. Storing the zone next to
    -- the wall clock keeps a later system-timezone change from silently
    -- reinterpreting an existing event.
    date       TEXT    NOT NULL,
    start_time TEXT,
    end_time   TEXT,
    all_day    INTEGER NOT NULL DEFAULT 0,
    timezone   TEXT    NOT NULL,
    version    INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    request_id TEXT
);

CREATE INDEX IF NOT EXISTS calendar_events_day_idx ON calendar_events(date, start_time);
CREATE UNIQUE INDEX IF NOT EXISTS calendar_events_request_idx
    ON calendar_events(request_id) WHERE request_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS artifacts (
    id               TEXT PRIMARY KEY,
    title            TEXT    NOT NULL,
    -- 'text' (snapshot stored in `body`) or 'file' (reference to disk).
    kind             TEXT    NOT NULL,
    body             TEXT,
    file_path        TEXT,
    -- Fingerprint of the referenced file at registration time; the list
    -- command compares it against the file on disk to report ok/changed.
    file_size        INTEGER,
    file_modified_at INTEGER,
    source           TEXT    NOT NULL DEFAULT 'manual',
    category         TEXT    NOT NULL DEFAULT '',
    favorite         INTEGER NOT NULL DEFAULT 0,
    archived         INTEGER NOT NULL DEFAULT 0,
    version          INTEGER NOT NULL DEFAULT 1,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    request_id       TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS artifacts_request_idx
    ON artifacts(request_id) WHERE request_id IS NOT NULL;

-- Idempotency ledger for scheduler-driven app actions: a job that already
-- ran with this key is not applied twice.
CREATE TABLE IF NOT EXISTS job_receipts (
    receipt_key TEXT PRIMARY KEY,
    job_type    TEXT    NOT NULL,
    created_at  INTEGER NOT NULL
);
