# Kodo Beta-0 Eval Harness

Two layers:

1. **deterministic** — no LLM, no network. Lives in `crates/agent` unit/integration
   tests and `evals/deterministic/` scripts that re-run the same oracles.
2. **live-agent** — real provider + fixture repos under `evals/fixtures/`.

## False completion

If the model claims done but the machine-checkable oracle fails, the run is
recorded as **false completion**. This metric is first-class.

## Live runner

```bash
# From repo root, after configuring a provider key in Kodo settings:
export KODO_EVAL_PROVIDER=anthropic   # or openai / deepseek / custom
export KODO_EVAL_MODEL_ID=claude-sonnet-4-5
export KODO_EVAL_API_KEY=...          # or rely on Kodo credentials.log
cargo run -p kodo-evals -- --live --tasks 5
```

Without credentials the runner prints `NOT RUN — credentials unavailable`
and exits 0 for CI.

## Fixture tasks (machine-checkable)

| id | type | oracle |
|----|------|--------|
| bug-1 | bug fix | failing test becomes passing |
| bug-2 | bug fix | off-by-one unit test |
| bug-3 | bug fix | panic path covered |
| feat-1 | feature | new function + test |
| feat-2 | feature | CLI flag works |
| feat-3 | feature | config default applied |
| ref-1 | refactor | tests still pass, API preserved |
| ref-2 | refactor | duplicate helper removed |
| test-1 | tests | coverage file exists + pass |
| test-2 | tests | edge-case tests added |
| rev-1 | code review | findings written |
| rev-2 | code review | risk list non-empty |
| multi-1 | multi-file | 2+ files changed + tests |
| amb-1 | ambiguous | clarifying plan, no destructive edit |
| repair-1 | verify fail | repair loop, tests end green |

## Metrics

- task_success
- acceptance_success
- tests_pass
- false_completion
- files_changed / unnecessary_files_changed
- tool_calls / failed_tool_calls / repair_attempts
- input_tokens / output_tokens / latency
- cancellation_success
