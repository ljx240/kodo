# Memory Engine Schema

> The current SQLite schema for `crates/memory`. Migrations live in
> `crates/memory/migrations/`; the on-disk database is
> `<state_dir>/memory/v1/memory.sqlite`.

## Tables

### `memory_entries`

| column | type | notes |
|---|---|---|
| `id` | TEXT PK | `uuid` v4 string |
| `kind` | TEXT NOT NULL | `working` / `episodic` / `semantic` / `procedural` / `rule` / `evidence_ref` |
| `status` | TEXT NOT NULL | `candidate` / `active` / `stale` / `superseded` / `quarantined` / `rejected` |
| `authority` | TEXT NOT NULL | `historical` / `user_rule` / `system` |
| `source_trust` | TEXT NOT NULL | `untrusted` / `tool_verified` / `test_verified` / `user_confirmed` |
| `scope_project_id` | TEXT | canonicalized project root |
| `scope_repository_id` | TEXT | git common-dir absolute path |
| `scope_branch` | TEXT | branch name |
| `scope_worktree_id` | TEXT | git worktree id |
| `scope_session_id` | TEXT | session id (latest turn scope) |
| `scope_task_id` | TEXT | optional sub-task grouping |
| `title` | TEXT NOT NULL | one-line summary |
| `abstract_text` | TEXT NOT NULL | up to 1 KiB preview |
| `body` | TEXT NOT NULL | JSON document (capped at 64 KiB) |
| `content_hash` | TEXT NOT NULL | SHA-256 of `kind + title + body` |
| `confidence` | REAL NOT NULL | 0.0 – 1.0 |
| `verified` | INTEGER NOT NULL | 0/1 — set when source_trust ≥ tool_verified |
| `conditions` | TEXT NOT NULL | JSON array of free-form preconditions |
| `source_uris` | TEXT NOT NULL | JSON array of `kodo://` / `git://` URIs |
| `valid_from` / `valid_until` | INTEGER | epoch seconds; NULL = open-ended |
| `last_verified_at` | INTEGER | epoch seconds |
| `last_used_at` | INTEGER | epoch seconds; updated by feedback |
| `use_count` | INTEGER NOT NULL | incremented by feedback |
| `created_at` | INTEGER NOT NULL | epoch seconds |
| `updated_at` | INTEGER NOT NULL | epoch seconds; bumped on every write |
| `superseded_by` | TEXT | id of the entry that replaced this one |
| `version` | INTEGER NOT NULL | starts at 1; bumps on every upsert |
| `candidate_skill` | INTEGER NOT NULL | P2 procedural promotion flag |

Indexes: `(scope_project_id, status)`,
`(scope_repository_id, scope_branch, status)`,
`(last_used_at DESC)`,
`(content_hash)`,
`(task_id)`.

### `memory_evidence`

| column | type | notes |
|---|---|---|
| `id` | INTEGER PK | autoincrement |
| `memory_id` | TEXT NOT NULL | FK → memory_entries.id ON DELETE CASCADE |
| `kind` | TEXT NOT NULL | `path` / `uri` / `command` / `diff` / `log` / `note` |
| `path` | TEXT | project-relative path; NULL when N/A |
| `uri` | TEXT | `kodo://` or `git://` URI; NULL when N/A |
| `digest` | TEXT | SHA-256 of file content; NULL when N/A |
| `snippet` | TEXT | up to 2 KiB excerpt |
| `created_at` | INTEGER NOT NULL | epoch seconds |

Index: `(memory_id)`, `(path)` — the second one is what
`mark_changed_paths` hits when a file mutates.

### `memory_links`

`kind` + `related_id` + UNIQUE(`memory_id`, `kind`, `related_id`). P2 uses
this for procedural → skill promotion; today the table exists for the
schema to be forward-compatible.

### `memory_outbox`

| column | type | notes |
|---|---|---|
| `id` | INTEGER PK | autoincrement |
| `event_id` | TEXT UNIQUE NOT NULL | derived from `session + turn_seq + item_seq + phase + event_type + source_digest` |
| `kind` | TEXT NOT NULL | mirrors `MemoryEventKind` |
| `phase` | TEXT NOT NULL | `started` / `finished` / `turn_complete` |
| `session_id` | TEXT | session id, when known |
| `turn_seq` | INTEGER | turn sequence within the session |
| `payload` | TEXT NOT NULL | JSON event body |
| `created_at` | INTEGER NOT NULL | epoch seconds |
| `compiled_at` | INTEGER | epoch seconds; NULL = pending |
| `attempts` | INTEGER NOT NULL DEFAULT 0 | bumped on every replay |
| `last_error` | TEXT | last compile error; NULL on success |

Replay uses `WHERE compiled_at IS NULL AND attempts < N` ordering by id.

### `memory_tombstones`

| column | type | notes |
|---|---|---|
| `id` | TEXT PK | the entry id that was deleted |
| `reason` | TEXT NOT NULL | human-readable; e.g. "user-disabled", "superseded" |
| `by` | TEXT NOT NULL | actor: `user`, `system`, `doctor` |
| `tombstoned_at` | INTEGER NOT NULL | epoch seconds |

`disable(id)` is *not* a tombstone (it just flips status to `rejected`);
`tombstone(id)` is reserved for "never revive" — and any replay that
would re-insert a tombstoned id is dropped at compile time.

### `memory_usage`

| column | type | notes |
|---|---|---|
| `id` | INTEGER PK | autoincrement |
| `memory_id` | TEXT NOT NULL | FK → memory_entries.id |
| `session_id` | TEXT | session that used this memory |
| `turn_seq` | INTEGER | turn within session |
| `used_at` | INTEGER NOT NULL | epoch seconds |
| `outcome` | TEXT | `helpful` / `harmful` / `unused` — P2 only |

### `memory_entries_fts`

FTS5 virtual table on `(title, abstract_text)` with the `unicode61`
tokenizer. Rowids match `memory_entries.rowid` for direct joins. The
DAO keeps it in sync inside the same transaction as the
`memory_entries` write — no triggers, so replay is deterministic.

## Migrations

| file | purpose |
|---|---|
| `0001_initial.sql` | all tables above + the FTS5 virtual table + indexes |
| `0002_tombstones.sql` | `memory_tombstones` |

`Store::open` runs every migration in order, idempotently, inside a
single transaction. Re-opening the same file twice does not duplicate
rows or re-apply DDL. The `schema_migrations` table records the
applied version and the SHA-256 of the migration body so a tampered
migration file is detected on the next open.

## PRAGMAs applied at open

```sql
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 3000;
PRAGMA foreign_keys = ON;
PRAGMA synchronous = NORMAL;
```

WAL lets the Tauri command handler and the doctor run concurrently; the
busy timeout absorbs the worst-case contention when the desktop writes a
"started" envelope and the CLI runs `memory doctor` in the same
millisecond.
