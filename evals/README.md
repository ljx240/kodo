# Kodo Eval Harness

Two layers:

1. **Layer A — deterministic** — no LLM, no network. Evidence gates, cancel,
   timeout, process-tree kill, dirty worktree, undo, context staleness,
   serialization, settings wiring (`cargo run -p kodo-evals`).
2. **Layer B — fixture coding tasks** — 17 machine-checkable fixture repos
   under `evals/fixtures/`. Each has `prompt.md`, `oracle.sh`,
   `expected_files.txt`, `forbidden.txt`.

## False completion

If the model claims Verified/completed but the machine-checkable oracle fails,
the run is recorded as **false completion**. This metric is first-class.

## Fixture baselines (always run)

Bug/feature/refactor fixtures must **fail** their oracle before any fix.
The blocked fixture must **pass** on baseline (no fabricated deploy artifacts).

## Live runner

```bash
export KODO_EVAL_PROVIDER=custom        # or anthropic / openai / deepseek
export KODO_EVAL_MODEL_ID=mimo-v2.5
export KODO_EVAL_API_KEY=...            # never printed
export KODO_EVAL_ENDPOINT=https://...   # optional, required for custom
cargo run -p kodo-evals -- --live --tasks 17
```

Without credentials the live portion prints
`NOT RUN — credential unavailable` and exits 0 for CI.

## Fixture tasks (machine-checkable)

| id | type | oracle |
|----|------|--------|
| bug-1-off-by-one | bug fix | failing test becomes passing |
| bug-2-null-guard | bug fix | null guard test |
| bug-3-off-by-zero | bug fix | clamp upper bound |
| feat-1-greet | feature | greet() + test |
| feat-2-uppercase | feature | shout() + test |
| feat-3-default-budget | feature | default budget 100 |
| ref-1-dedupe | refactor | uniqueAgain calls unique |
| ref-2-extract-helper | refactor | vol reuses area |
| test-1-add-tests | tests | assert suite added |
| test-2-edge-cases | tests | edge cases added |
| cross-1-api-impl | cross-file | api wires db |
| cross-2-frontend-logic | cross-file | reducer fix |
| config-1-env-default | config | port default 8080 |
| frontend-1-form-validate | frontend | email validation |
| multi-1-two-files | multi-file | both files as needed |
| blocked-1-impossible | blocked | no fake deploy success |
| repair-1-loop | repair | repair until green |

## Metrics

- task_success / acceptance_success / verification_success
- false_completion / false_completion_rate
- files_changed / unnecessary_files_changed
- tool_calls / failed_tool_calls / repair_attempts
- input_tokens / output_tokens / latency
