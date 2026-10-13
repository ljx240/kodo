# Memory Engine Operations

> How the Memory Engine is operated day-to-day. Targeted at the user
> who runs `kodo memory …` from the terminal and at the maintainer who
> triages a corruption report.

## Where the data lives

```
<state_dir>/memory/v1/memory.sqlite
<state_dir>/memory/v1/memory.sqlite-wal
<state_dir>/memory/v1/memory.sqlite-shm
```

`<state_dir>` is the same Application Support directory the rest of
Kodo uses (`kodo_core::settings::state_dir()`). The file is the same
on macOS, Linux, and Windows; the engine never reads from anywhere
else.

The DB is plain SQLite 3 (with the FTS5 extension) bundled into
`rusqlite`. The bundled SQLite is recent enough for the WAL/SHM
files but old enough that the file is forward-portable to any system
SQLite ≥ 3.40 for forensic inspection.

## CLI

```bash
# 摘要：行数、outbox、DB 大小
kodo memory status

# 列出当前项目下的所有记忆（含历史）
kodo memory list
kodo memory list --kind rule --status stale --limit 50
kodo memory list --project /path/to/repo

# 关键词检索
kodo memory search "kafka consumer lag" --limit 10

# 查看完整元数据 + 证据
kodo memory inspect <id>

# 停用
kodo memory disable <id>

# 诊断（默认 dry-run）
kodo memory doctor
kodo memory doctor --apply      # 同时跑 FTS 重建 + outbox 重放
kodo memory reindex --apply     # 同 doctor --apply
```

`kodo memory promote <id>` and `kodo memory benchmark` are reserved
for P2 and currently print `not yet implemented — see
docs/memory/EVALUATION.md`.

The CLI opens the same store the desktop uses, so a write from the
GUI shows up in the CLI and vice versa.

## GUI

- **Settings → Memory** shows the storage path, row counts, outbox
  pending, DB size, and a "Run doctor" / "Reindex" button pair.
- **Inspector → Memory** lists entries for the active project,
  supports search-by-keyword and search-by-snippet, and per-row
  "Disable". The list reflects `include_history = true` so disabled
  entries are still visible (with a `rejected` chip) so the user can
  re-enable them later if needed.
- **Topbar** has a `Memory` chip in the conversation route; clicking
  it opens the Inspector's Memory tab.

## Doctor report fields

`kodo memory doctor` prints, in order:

| field | meaning |
|---|---|
| `ok` | `true` if `PRAGMA integrity_check` returned `ok` |
| `integrity` | raw list — multiple entries = corruption |
| `tombstones` | number of rows in `memory_tombstones` |
| `db_size_bytes` | file size on disk |
| `outbox_pending` | rows in `memory_outbox` with `compiled_at IS NULL` |
| `outbox_replayed` | number of rows the doctor actually replayed (only with `--apply`) |
| `row_counts` | per-table count snapshot |

`doctor --apply` performs two additional actions:

1. **`fts5 rebuild`** — drops the FTS shadow table and re-inserts
   every entry from `memory_entries`. Use this when search misses a
   result that `inspect` shows exists.
2. **outbox replay** — calls `outbox::replay(store, privacy, 1024)`
   which compiles every pending outbox row, respecting tombstones.
   Rows that fail compilation stay pending with `last_error` set;
   rows that succeed transition to `compiled_at = now`.

## Backup and restore

The DB is a single file plus its WAL/SHM siblings. A consistent
backup is:

```bash
sqlite3 memory.sqlite ".backup memory.sqlite.bak"
```

The `.backup` command is the only way to copy a live SQLite DB
without risking a torn WAL. **Do not `cp` the file while Kodo is
running** — the WAL can hold the most recent writes.

To restore: `cp memory.sqlite.bak memory.sqlite` (and remove the
WAL/SHM files so SQLite re-checks them on next open).

## Corruption triage

If `doctor` reports `integrity` other than `ok`:

1. Stop Kodo (no process should hold the DB).
2. `sqlite3 memory.sqlite "PRAGMA integrity_check"` — read the full
   list of issues.
3. If only FTS is corrupt, `kodo memory doctor --apply` rebuilds it
   in place. The base `memory_entries` table is untouched.
4. If `memory_entries` itself is corrupt, restore from the most
   recent `.backup`. The session log is the source of truth — any
   missing memories can be replayed from there by future tooling.

## Forensics

For offline inspection, the system `sqlite3` CLI works:

```bash
sqlite3 ~/.local/share/kodo/memory/v1/memory.sqlite
sqlite> .tables
sqlite> .schema memory_entries
sqlite> SELECT id, kind, status, authority, title FROM memory_entries
        WHERE status = 'active' ORDER BY last_used_at DESC LIMIT 20;
```

The FTS5 search is also accessible directly:

```bash
sqlite> SELECT rowid FROM memory_entries_fts
        WHERE memory_entries_fts MATCH 'kafka consumer' LIMIT 5;
```

## Capacity and retention

P0 has no automatic retention. Entries stay until the user disables
them, the `tombstone` path is taken, or the file is rotated manually.
P2 adds a `retention_days` config key that prunes
`stale`/`superseded`/`rejected` entries older than the threshold on
`doctor --apply`. The default retention is `None` (no prune).
