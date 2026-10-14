-- Workbench v2 — the 待办 data model.
--
-- Adds a status catalog, tags, people, attachments, reminders and an activity
-- log, plus the four scalar columns the redesign's "自定义字段" card shows.
--
-- Compatibility note (the reason this is an additive migration and not a
-- table rebuild): `todos.status` keeps its name and its stored values. It is
-- *reinterpreted* as a `todo_statuses.id`, and the three built-in statuses are
-- seeded with the exact ids the old three-value enum used (`todo` / `doing` /
-- `done`). Every existing row therefore resolves without being rewritten, and
-- no table is dropped and recreated. SQLite cannot add a foreign key to an
-- existing table without the 12-step rebuild, so `todos.status` carries no FK
-- and the store validates it against the catalog on every write.
--
-- Also note: `todos.parent_id` is `ON DELETE SET NULL` (0001). Deleting a
-- parent therefore *promotes* its children rather than removing them, so
-- `delete_todo` removes the subtree explicitly instead of relying on a cascade.

CREATE TABLE IF NOT EXISTS todo_statuses (
    id         TEXT PRIMARY KEY,
    name       TEXT    NOT NULL,
    -- A palette key (`gray` / `blue` / `amber` / …), never a hex value: the
    -- frontend owns the actual colours so a theme swap stays CSS-only.
    color      TEXT    NOT NULL DEFAULT 'gray',
    -- The single source of truth for "this todo is done". There is no second
    -- completion flag on `todos`.
    is_done    INTEGER NOT NULL DEFAULT 0,
    -- The three seeded statuses cannot be deleted; the UI keeps its defaults.
    builtin    INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0,
    version    INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Ids match the pre-v2 enum values on purpose — see the compatibility note.
INSERT OR IGNORE INTO todo_statuses
    (id, name, color, is_done, builtin, sort_order, version, created_at, updated_at) VALUES
    ('todo',  '待处理', 'gray',  0, 1, 0, 1, 0, 0),
    ('doing', '进行中', 'blue',  0, 1, 1, 1, 0, 0),
    ('done',  '已完成', 'green', 1, 1, 2, 1, 0, 0);

-- People are local: Kodo has no accounts, so this is a hand-managed list the
-- assignee picker reads. One row is seeded so the picker is never empty on a
-- fresh install.
CREATE TABLE IF NOT EXISTS todo_people (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    color      TEXT NOT NULL DEFAULT 'blue',
    created_at INTEGER NOT NULL
);

INSERT OR IGNORE INTO todo_people (id, name, color, created_at)
    VALUES ('me', '我', 'blue', 0);

CREATE TABLE IF NOT EXISTS todo_tags (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    color      TEXT NOT NULL DEFAULT 'blue',
    sort_order INTEGER NOT NULL DEFAULT 0,
    version    INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Scalar columns for the editor's 自定义字段 card. Each carries a constant
-- default so every existing row is valid the moment the column appears.
ALTER TABLE todos ADD COLUMN due_time     TEXT;
ALTER TABLE todos ADD COLUMN assignee_id  TEXT REFERENCES todo_people(id) ON DELETE SET NULL;
ALTER TABLE todos ADD COLUMN item_type    TEXT NOT NULL DEFAULT 'task';
ALTER TABLE todos ADD COLUMN module       TEXT NOT NULL DEFAULT '';
ALTER TABLE todos ADD COLUMN estimate_hours REAL;
ALTER TABLE todos ADD COLUMN actual_hours   REAL;

CREATE INDEX IF NOT EXISTS todos_assignee_idx ON todos(assignee_id);

CREATE TABLE IF NOT EXISTS todo_tag_links (
    todo_id TEXT NOT NULL REFERENCES todos(id)      ON DELETE CASCADE,
    tag_id  TEXT NOT NULL REFERENCES todo_tags(id)  ON DELETE CASCADE,
    PRIMARY KEY (todo_id, tag_id)
);

CREATE INDEX IF NOT EXISTS todo_tag_links_tag_idx ON todo_tag_links(tag_id);

-- Related items, stored one way round; the store writes both directions when
-- the caller links two todos.
CREATE TABLE IF NOT EXISTS todo_links (
    todo_id    TEXT NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
    related_id TEXT NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
    PRIMARY KEY (todo_id, related_id)
);

CREATE INDEX IF NOT EXISTS todo_links_related_idx ON todo_links(related_id);

CREATE TABLE IF NOT EXISTS todo_attachments (
    id         TEXT PRIMARY KEY,
    todo_id    TEXT    NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    -- A path on disk. The record is a reference: deleting it never deletes
    -- the file.
    path       TEXT    NOT NULL,
    size       INTEGER,
    mime       TEXT    NOT NULL DEFAULT '',
    version    INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    request_id TEXT
);

CREATE INDEX IF NOT EXISTS todo_attachments_todo_idx ON todo_attachments(todo_id);
CREATE UNIQUE INDEX IF NOT EXISTS todo_attachments_request_idx
    ON todo_attachments(request_id) WHERE request_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS todo_reminders (
    id             TEXT PRIMARY KEY,
    todo_id        TEXT    NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
    -- 'due_before' (offset_minutes before the due moment) or 'status_change'.
    kind           TEXT    NOT NULL,
    offset_minutes INTEGER,
    message        TEXT    NOT NULL DEFAULT '',
    enabled        INTEGER NOT NULL DEFAULT 1,
    version        INTEGER NOT NULL DEFAULT 1,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS todo_reminders_todo_idx ON todo_reminders(todo_id);

CREATE TABLE IF NOT EXISTS todo_activity (
    id         TEXT PRIMARY KEY,
    todo_id    TEXT    NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
    -- 'comment' | 'created' | 'status' | 'updated' | … — free-form so a new
    -- event kind never needs a migration.
    kind       TEXT    NOT NULL,
    author_id  TEXT    REFERENCES todo_people(id) ON DELETE SET NULL,
    body       TEXT    NOT NULL DEFAULT '',
    -- Opaque JSON the store never reads; the app renders what it understands.
    meta       TEXT    NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS todo_activity_todo_idx ON todo_activity(todo_id, created_at);
