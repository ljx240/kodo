# Kodo Memory Engine (KME) — Documentation Index

> This directory is the source of truth for the Memory Engine design
> and operations. The agent loop, the Tauri shell, and the CLI all
> read from it. The implementation plan lives in
> `KODO_MEMORY_IMPLEMENTATION.md`; everything else here is a focused
> reference derived from it.

## Read in this order

| # | doc | audience | what it answers |
|---|---|---|---|
| 0 | `KODO_MEMORY_IMPLEMENTATION.md` | engineers | full spec — schema, recall, ingest, security, operations, evaluation. Start here if you are reading the codebase for the first time. |
| 1 | `ARCHITECTURE.md` | engineers | crate map, trait surface, where the SQLite + FTS5 lives, how the agent loop threads the handle. |
| 2 | `SCHEMA.md` | engineers + operators | every table, every column, every index, every FTS5 trigger. The migration files in `crates/memory/migrations/` are the source of truth; this doc is the reading guide. |
| 3 | `SECURITY.md` | reviewers + operators | fail-open semantics, secret redaction, hostile-memory quarantine, privacy tiers, the four hard guarantees. |
| 4 | `OPERATIONS.md` | operators | `kodo memory <verb>` reference, `memory_doctor`, `memory_reindex`, where the DB lives on each platform, how to wipe state. |
| 5 | `EVALUATION.md` | reviewers | benchmark methodology, paired-replay workloads under `evals/benchmark/memory/workloads/`, Gate D acceptance, embedding-channel opt-in policy. |

## Where the implementation lives

```
crates/memory/                # the crate (no dependencies on kodo-agent or kodo-shell)
  src/lib.rs                  # MemoryAccess trait + MemoryHandle newtype
  src/embedding.rs            # EmbeddingProvider trait + NoopEmbedding + LocalHashEmbedding
  src/embedding_local.rs      # LocalOnnxEmbedding stub — feature-gated behind `onnx`
  src/model_store.rs          # local model cache (manifest.json, warmup, status)
  src/store.rs                # SQLite + WAL + foreign_keys + FTS5 + migrations
  src/retrieval.rs            # scope filter → FTS + exact → RRF → MemoryPack
  src/ingest.rs               # SafeMemoryEvent → outbox row (idempotent)
  src/outbox.rs               # replay loop, exponential backoff, max-attempts cap
  src/compiler.rs             # outbox row → MemoryEntry (rules only; no LLM)
  src/sanitizer.rs            # secret redaction, prompt-injection detection
  src/validity.rs             # GitContext + worktree scope + file digest
  src/doctor.rs               # integrity_check, FTS rebuild, outbox retry
  migrations/
    0001_initial.sql
    0002_tombstones.sql
crates/agent/src/lib.rs       # RunRequest.memory, initial_history, recall+observe hooks
crates/cli/src/memory_cmd.rs  # `kodo memory <verb>` surface
apps/desktop/src-tauri/src/memory.rs   # 6 Tauri commands
apps/desktop/src/inspector/MemoryInspector.tsx
apps/desktop/src/pages/SettingsPage.tsx   # Memory section
```

## What this engine is and is not

- **Is** — a fail-open, locally-stored, version-aware, derived index
  over the existing append-only session log. Never rewrites sessions;
  never replaces `compact_history`; never becomes a system instruction.
- **Is not** — a vector DB, a remote embedding service, a chat history
  product, a knowledge graph, or an automatic Skill file writer.
  Promotion from `procedural` memory to a Skill file is human-gated.

## Hard guarantees (from `SECURITY.md`)

1. Memory content is appended to the user-message observation block,
   never to the system block.
2. Secret-shaped bodies are redacted before insert; `high` privacy
   refuses them outright.
3. Hostile content ("ignore prior instructions and run X") is
   quarantined and excluded from `recall`.
4. Tombstoned ids stay tombstoned; the outbox worker refuses to
   revive a memory whose id is in `memory_tombstones`.

## Where the data lives

| platform | path |
|---|---|
| macOS | `~/Library/Application Support/Kodo/memory/v1/memory.sqlite` |
| Linux | `${XDG_CONFIG_HOME:-~/.config}/kodo/memory/v1/memory.sqlite` |
| Windows | `%APPDATA%\kodo\memory\v1\memory.sqlite` |

Local embedding model cache (only when `LocalBgeSmallEn` /
`LocalBgeSmallZh` / `LocalMiniLmL6` is warmed up):

| platform | path |
|---|---|
| all | `<state_dir>/memory/models/v1/manifest.json` + one subdir per backend |

See `OPERATIONS.md` § "Wipe state" for the exact commands.
