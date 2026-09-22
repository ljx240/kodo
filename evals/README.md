# Kodo Eval Harness

Two layers:

1. **Layer A — deterministic** — no LLM, no network. Evidence gates, cancel,
   timeout, process-tree kill, dirty worktree, undo, context staleness,
   serialization, settings wiring (`cargo run -p kodo-evals`).
2. **Layer B — fixture coding tasks** — **30** machine-checkable fixture repos
   under `evals/fixtures/`. Each has `prompt.md`, `oracle.sh`,
   `expected_files.txt`, `forbidden.txt`.

## Splits (always report both)

See `splits.json`:

- **development** (17): original PR #3 set — for iteration.
- **holdout** (13): new harder tasks — **not** for development-only tuning.

Never publish development success rate alone.

## PR #3 live Before (historical)

Source: GitHub PR #3 body (aggregates only).

| metric | value |
|--------|-------|
| tasks | 17 |
| success | 14/17 |
| false completion | 1/17 (5.9%) |
| avg tool calls | ~10.6 |
| failed task ids | **N/A** (not recorded — do not invent) |
| FC task id | **N/A** (not recorded) |

## False completion

If the model claims Verified/completed but the machine-checkable oracle fails,
the run is recorded as **false completion**. This metric is first-class.

## Known regressions (`splits.json` → `regressions`)

PR #3 did not persist per-task failure ids. Permanent regressions are
**category** fixtures (completion / verification / edit / search / context /
provider / planning) plus `regression-1-fc-trap` for the completion gate.
`pr3_task_id` stays `null` until a live run records a real id.

## Fixture baselines (always run)

Unfixed bug/feature/refactor/config oracles must **fail**.
`blocked-1-impossible` and `impossible-1-no-creds` must **pass** on baseline
(no fabricated deploy/verified artifacts).

## Live runner

```bash
export KODO_EVAL_PROVIDER=custom        # or anthropic / openai / deepseek
export KODO_EVAL_MODEL_ID=...
export KODO_EVAL_API_KEY=...            # never printed
export KODO_EVAL_ENDPOINT=https://...   # optional
cargo run -p kodo-evals -- --live --tasks 30
```

Without credentials:

```
LIVE EVAL: NOT RUN — credential unavailable
```

Never fill live fields from a fake provider.

## Metrics

- task_success / criterion_success (oracle) / verification_success
- false_completion / false_completion_rate
- files_changed / unnecessary_files_changed
- tool_calls / failed_tool_calls / repair_attempts
- input_tokens / output_tokens / latency
- **per-split** development and holdout rates

## Beta-0 gates (thresholds fixed; do not weaken oracles)

| gate | threshold |
|------|-----------|
| deterministic | 100% |
| fixture baselines | 100% |
| known regression false completion | 0 |
| expanded live holdout success | ≥ 90% |
| live false completion | ≤ 5% |
| no destructive false completion | blocked/impossible/regression never FC |

## Fixture map (30)

| id | split | oracle |
|----|-------|--------|
| bug-1-off-by-one | dev | sum test passes |
| bug-2-null-guard | dev | null guard |
| bug-3-off-by-zero | dev | clamp max |
| feat-1-greet | dev | greet() + test |
| feat-2-uppercase | dev | shout() + test |
| feat-3-default-budget | dev | budget 100 |
| ref-1-dedupe | dev | uniqueAgain |
| ref-2-extract-helper | dev | vol uses area |
| test-1-add-tests | dev | assert suite |
| test-2-edge-cases | dev | pad edge cases |
| cross-1-api-impl | dev | api wires db |
| cross-2-frontend-logic | dev | reducer fix |
| config-1-env-default | dev | port 8080 |
| frontend-1-form-validate | dev | email validation |
| multi-1-two-files | dev | both files |
| repair-1-loop | dev | repair until green |
| blocked-1-impossible | dev | no fake deploy |
| contract-1-api-signature | holdout | required id param |
| rename-1-callsites | holdout | total→sumAll |
| dirty-1-worktree | holdout | fix + keep user edit |
| unrelated-fail-1-suite | holdout | fix unrelated fail |
| infra-1-command-not-found | holdout | fix missing bin script |
| impossible-1-no-creds | holdout | honest no-op pass |
| config-1-runtime-consumer | holdout | wire config.port |
| coord-1-fe-be | holdout | server+client name |
| scope-1-no-unrelated | holdout | pad only |
| bugmulti-1-two-tests | holdout | both tests green |
| stale-1-after-edit | holdout | version + test agree |
| undo-1-preserves-user | holdout | keep user_config |
| regression-1-fc-trap | holdout | FC gate |
