# Memory Engine Evaluation

> How the Memory Engine is benchmarked, what the gates are, and which
> numbers the P2 release must produce before any of the advanced
> strategies ship.

## Methodology

Every benchmark in this document is a **paired replay**: the same
workload runs once with `memory: None` (the baseline) and once with
`memory: Some(handle)` (the candidate). The diff is the only signal
that counts; absolute token counts, latency, and accuracy are
secondary.

The paired-replay harness itself is **not implemented yet**. What exists
today is the input data (5 workloads under
`evals/benchmark/memory/workloads/`, see §Workload design) and an honest
stub: `kodo memory benchmark` (`crates/cli/src/memory_cmd.rs`) prints
"Benchmark harness not yet implemented" plus the commands that do work,
and produces no numbers. The legacy `agent_benchmark` /
`online_benchmark` suites under `crates/agent/tests/` are coding-agent
suites; they are not paired and no memory runner reads them. Nothing
under `evals/` consumes the workloads.

## Gates (from the implementation plan)

| gate | scope | what passes |
|---|---|---|
| **A** | P0 pre-merge | `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo test -p kodo-memory` all green |
| **B** | P0 merge | All of A, plus the `kodo-agent` / `kodo-shell` / `kodo-cli` test suites, plus the `memory-off` parity test (`initial_history` byte-identical when `memory: None`) |
| **C** | P1 merge | All of B, plus the P1 tests: `git_context_falls_back_without_git`, `worktree_isolation_denies_other_worktree`, `file_change_marks_stale`, `rrf_merges_fts_and_exact`, `compaction_anchors_intact_under_budget_pressure`, `tombstone_blocks_replay_revival` |
| **D** | P2 merge | All of C, plus the embedding trait round-trip, plus a real benchmark number on the P2 strategy (no fake numbers — see §4) |

## What P0 / P1 already measure

The P0 + P1 test suites already cover the high-signal properties:

- **Outbox at-least-once.** `enqueue_twice_yields_one_row` proves
  that the same `event_id` is deduplicated, regardless of how many
  times the source fires.
- **Scope isolation.** `project_a_entries_invisible_from_project_b`
  proves that a memory written in one project cannot leak into the
  recall of another, even when their `repository_id` matches.
- **Secret redaction.** `secret_body_is_redacted_before_enqueue`
  proves that the body has no surviving secret after the sanitizer
  runs. `high_privacy_refuses_secret_body` proves that the high
  privacy policy refuses to write at all in that case.
- **Hostile memory.** `hostile_body_is_quarantined_not_promoted`
  proves that the canonical prompt-injection string lands in
  `status=quarantined` and never reaches recall.
- **Tombstone.** `tombstone_suppresses_replay` and
  `tombstone_blocks_replay_revival` prove that a tombstoned id can
  not be re-introduced by a later outbox replay.
- **FTS sync.** `fts_keeps_synced` (P0; the test name reflects the
  contract) proves that an upsert + a delete both reach the FTS
  index in the same transaction as the `memory_entries` write.

## P2 benchmark requirements

P2 cannot ship any strategy (embedding model, MMR-lite, procedural
promotion heuristics) without a measured benefit. The pre-conditions
are explicit in the implementation plan §18:

1. The candidate strategy is implemented behind a feature flag.
2. A benchmark workload exists in `evals/benchmark/memory/` that
   exercises the strategy on at least 50 real-ish tasks. **Met on
   data, not on count**: 5 workloads are on disk (§Workload design),
   so the 50-task bar still needs either more workloads or recorded
   per-strategy repeats — the runner decides which, and the report
   says so in `samples`.
3. The candidate strategy improves at least one of:
   - **Recall@10** (fraction of relevant memories in the top 10 of
     the pack) by ≥ 5 percentage points over the BM25 baseline.
   - **First-turn token cost** (sum of `input_tokens` for the first
     turn of a session) by ≥ 10% with no quality regression.
   - **End-task success rate** (the same success metric
     `agent_benchmark` uses) by ≥ 3 percentage points.
4. The candidate does not regress any of the above by more than
   half the gain it produced.

Until all four are met, the P2 strategy stays behind the feature
flag and is not enabled by default. **No fabricated numbers.** The
CLI verb prints `Benchmark harness not yet implemented — see
docs/memory/EVALUATION.md` today; it must keep printing an honest
stub until a runner can produce the JSON below.

## Workload design

The workloads **do** live in `evals/benchmark/memory/workloads/` —
5 directories, prepared and reviewed as data, and read by no runner
yet. Each workload is a self-contained directory:

- `task.md` — the human-readable task prompt.
- `seed/` — the project as it should look at the start of the run
  (small enough to fit in a git repo): `seed/repo/` plus a
  `seed/MEMORY.md` of prior memory entries, including one quarantined
  prompt-injection attempt so a candidate can be caught promoting it.
- `expected.json` — the entries the engine *should* produce after a
  paired replay, plus the `expected_recall_ids` and the status
  assertions; the harness diffs the candidate's output against this.
- `scoring.toml` — the weights (recall 0.4 / end-task success 0.4 /
  first-turn tokens 0.1 / no-regressions 0.1, pass at combined ≥ 0.6
  with no individual score below 0.3).
- `verify.sh` — the end-task oracle `scoring.toml` scores.

The 5 workloads are: "explain a flaky test", "refactor a duplicated
helper", "find an unused dep", "port a CLI flag to a config key",
and "diagnose a memory-leak symptom". Each is a 2-turn run with a
short, deterministic answer.

## Reporting template

A P2 benchmark run **will** produce a JSON file at
`evals/out/<date>-<strategy>.json`. No such file exists: `evals/out/`
is gitignored and today holds only the coding harness's live forensics
directories (`live-<ts>-<pid>/`, see `evals/README.md`). The shape a
future memory runner must write:

```json
{
  "strategy": "bm25_baseline" | "local_hash_embedding" | "noop" | ...,
  "workloads": 5,
  "recall_at_10": 0.62,
  "first_turn_tokens": 18432,
  "end_task_success": 0.84,
  "delta_vs_baseline": {
    "recall_at_10": "+0.04",
    "first_turn_tokens": "-8%",
    "end_task_success": "+0.02"
  },
  "regressions": [],
  "samples": 50
}
```

The report is the artifact the plan reviewer reads. A run with
`regressions: []` and at least one positive `delta_vs_baseline`
clears Gate D.

## Embedding channel policy (opt-in only)

The embedding channel is the third RRF lane. It is **opt-in** — the
default `MemoryConfig::default()` keeps it disabled and the
`NoopEmbedding` provider returns `IndexUnavailable`, so RRF degrades
gracefully to FTS + exact-token only. Three rules are hard-coded:

1. **No silent downloads.** Model weights are never pulled during
   `SqliteMemory::open`, the agent loop, or the desktop startup path.
   The only entry point is the explicit CLI command
   `kodo memory embed warmup --backend <name>` (or the equivalent
   SettingsPage button), gated on the `onnx` cargo feature.
2. **Feature flag.** The `ort` + `tokenizers` + `ureq` dependencies
   (≈ 120 MB onnxruntime binary) are off by default. Consumers who
   do not need local embedding pay zero build cost; building with
   `--features kodo-memory/onnx` is the only way to enable warmup.
3. **No remote embedding.** The four supported backends — `Off`,
   `LocalBgeSmallEn`, `LocalBgeSmallZh`, `LocalMiniLmL6`, `LocalHash`
   — all run weights downloaded to `~/.cache/kodo/memory/models/v1/`.
   No network call after warmup completes; no API key, no SaaS
   dependency, no data leaving the host.

A backend that requires warmup (`LocalBgeSmallEn` and friends) only
returns ready-to-embed vectors after `model_store::warmup(backend)`
has populated `manifest.json` with non-empty `onnx_sha256` +
`tokenizer_sha256` + `warmed_at`. `LocalOnnxEmbedding::load` refuses
to construct otherwise and surfaces `IndexUnavailable` so callers
can show "warmup required" in the UI.

### Default backend per Gate D outcome

| outcome | default | when to flip |
|---|---|---|
| local embedding ≥ +0.05 `recall_at_10` vs `Off` on ≥ 4/5 workloads, with `regressions: []` | may move `LocalBgeSmallEn` from off-by-default to on-by-default | after at least one full paired-replay run lands a result in `evals/out/` — none has landed, so this row has never fired |
| local embedding neutral or worse | **stays off by default**; remains opt-in | always |

The flip is itself a Gate D change — a new commit that (a) updates
`MemoryConfig::default()`, (b) removes the "opt-in" banner from
SettingsPage, and (c) cites the JSON in `evals/out/` in its message.
Without all three, the default stays `Off`.
