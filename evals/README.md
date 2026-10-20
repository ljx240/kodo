# Kodo Eval Harness

Two layers:

1. **Layer A — deterministic** — no LLM, no network. Evidence gates, cancel,
   timeout, process-tree kill, dirty worktree, undo, context staleness,
   serialization, settings wiring (`cargo run -p kodo-evals`).
2. **Layer B — fixture coding tasks** — machine-checkable fixture repos. Each
   fixture has `prompt.md`, `oracle.sh`, `expected_files.txt`,
   `forbidden.txt`, and there are **two corpora (suites)**:
   - `fixtures/` — the historical **30**-task set. Its repos are small on
     purpose (69 files / 291 lines total; the largest single repo is 20 lines),
     so a model can hold one in view without searching. It measures protocol
     correctness, not retrieval depth.
   - `fixtures-real/` — **4** real-scale projects (11–12 files, 171–216 lines
     each, a `package.json` and a `test/run.js` suite plus green companion
     suites). The noise is the point: the fix needs a multi-file read, and the
     Verify gate resolves a genuine `npm test`.

Baselines run over **every** corpus found on disk, so `fixtures-real/` cannot
rot quietly next to a green report: `cargo run -p kodo-evals` gates on the two
corpora together (currently 73 deterministic + 200 baseline checks, of which 32
belong to the real suite). Live runs pick one corpus with `--suite` (default:
`default`).

## Splits (always report both)

Each corpus has its own split file so a fixture id cannot sit in two coverage
checks at once: `splits.json` for `fixtures/`, `splits-<suite>.json` for
another suite (`splits-real.json`).

- **development** (17): original PR #3 set — for iteration.
- **holdout** (13): new harder tasks — **not** for development-only tuning.
- real suite: development `real-1`/`real-2`, holdout `real-3`/`real-4`.

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

"Oracle fails before the fix" alone does not prove the task is solvable, so a
fixture may ship an optional `solution.sh` — the reviewer's canonical fix, using
the same `cd "$(dirname "$0")/repo"` anchor the oracle does. When the file is
present the baselines copy `repo/` to a throwaway directory, apply
`solution.sh`, re-run the oracle and gate on `fixture_<id>_canonical_solution_passes`.
The live runner copies only `repo/`, never `solution.sh`, so a canonical fix
cannot reach the model. All four `fixtures-real/` tasks carry one; the toy
corpus predates the mechanism and is verified by hand.

The two scope manifests are load-bearing, so the baselines check them too:
every declared expected path must name a real file in `repo/`, every editable
fixture must declare at least one, and the two no-edit fixtures must declare
none. (`bug-1/2/3` declared `src/sum.js`, `src/name.js`, `src/clamp.js` for as
long as nothing read the file — the fixtures keep their sources at the root.)

### Scope manifests

- `expected_files.txt` — repo-relative files the fix is expected to **modify**.
  Files a task is supposed to *create* are asserted by `oracle.sh`, not here.
  The live runner diffs the work copy against `repo/` and reports
  `expected_files_missing` (oracle green, declared file untouched) and
  `scope_extra_files` (changes outside the declaration). Both are diagnostics:
  a different-but-correct fix location must not be turned into a failure the
  oracle does not know about.
- `forbidden.txt` — a path that exists must stay byte-identical; a path that is
  **absent** must not be created. The second form is how fixtures express
  "do not fabricate `secret.env` / `vendor/lib.js` / `CHANGELOG.md`", and the
  live runner now enforces both halves.

## Live runner

```bash
export KODO_EVAL_PROVIDER=custom        # or anthropic / openai / deepseek
export KODO_EVAL_MODEL_ID=...
export KODO_EVAL_API_KEY=...            # never printed
export KODO_EVAL_ENDPOINT=https://...   # optional
export KODO_EVAL_MAX_OUTPUT_TOKENS=8192 # optional, default 8192
cargo run -p kodo-evals -- --live --tasks 30 --repeat 2
cargo run -p kodo-evals -- --live --suite real   # the 4 real-scale projects
```

`--suite` selects the corpus for the live pass (`default` → `fixtures/`,
`real` → `fixtures-real/`) and its split file. `--tasks N` truncates the sorted
fixture list, so `--tasks 30` is the whole default corpus and the real suite
only needs 4. An unknown suite name reports
`live: NOT RUN — unknown suite "<x>"` with the available list instead of
quietly scoring zero tasks.

`--repeat N` (clamped to 1..=10) runs every selected fixture N times. One
sample cannot tell a flaky agent from a weak model, so the run also reports
`variance: pass_at_n / pass_always / unstable`. All rates keep a **per-run**
denominator (`live_tasks` counts runs, `live_fixtures` counts distinct
fixtures), so repeated runs stay comparable with the single-sample history.

`KODO_EVAL_MAX_OUTPUT_TOKENS` exists because the harness previously hardcoded a
2048 ceiling while the product ships 102400 (desktop) / 4096 (CLI). A capped
completion is a harness artifact, not a model-quality signal, so the ceiling is
configurable and echoed back in the report as `truncated_calls ... at
max_output_tokens=N`.

Without credentials:

```
LIVE EVAL: NOT RUN — credential unavailable
```

Never fill live fields from a fake provider.

### Failure forensics

A fixture whose oracle fails, that touches a `forbidden.txt` path, or whose run
never landed a model answer is **not** deleted. Its workdir, full step trace,
final answer and a `changes.diff` against the baseline repo are copied to:

```
evals/out/live-<ts>-<pid>/<fixture-id>/            # --repeat 1
evals/out/live-<ts>-<pid>/<fixture-id>-run<k>/     # --repeat >1
evals/out/live-<ts>-<pid>-<suite>/...              # any --suite other than default
```

That directory is gitignored — it can contain provider output. Passing fixtures
are removed as before. `live_status` prints the run directory and `live_suite`
prints which corpus produced the numbers (it reads
`n/a (live not run)` when no live pass ran, so a baseline-only report cannot be
mistaken for a measured one).

## Metrics

- task_success (oracle) — counted only from `oracle.sh`, and **never** credited
  to an aborted run
- aborted_runs — provider outage, crash, empty final, state-machine error
  (`failure_reason` in `provider_error` / `unrecoverable` / `no_answer` /
  `state_machine_error`, or no model call ever landed a finish reason). An
  untouched `blocked-1-impossible` goes green exactly *because* nothing ran, so
  scoring it as solved would let a dead endpoint "pass" the eval
- acceptance_success — agent's final `delivery == "ready"`
- verification_success — agent's final `verification == "passed"`
- self_report — `claims` vs `oracle_confirmed` and the gap between them
- agent_verdicts — `not_verified` / `verify_failed` / `blocked` (a blocked run
  is counted once, not once for delivery and once for verification)
- failure_taxonomy — `budget` / `acceptance` / `provider` / `truncated` /
  `permission` / `no_answer` / `other`
- false_completion / false_completion_rate — oracle failed **and** the agent
  claimed a verified or ready answer. Deterministic acceptance-gate rejections
  are reported separately as `gate_selftest_rejections` and never enter this rate
- expected_files_missing / scope_extra_files / unnecessary_files_changed
- tool_calls / failed_tool_calls / repair_attempts
- model_calls / input_tokens / output_tokens / truncated_calls /
  fallback_protocol_calls / latency
- variance — `pass_at_n` / `pass_always` / `unstable` when `--repeat > 1`
- **per-split** development and holdout rates, including per-split `claims` and
  `verify` so a holdout self-report gap cannot hide in the development average

## Beta-0 gates (thresholds fixed; do not weaken oracles)

| gate | threshold |
|------|-----------|
| deterministic | 100% |
| fixture baselines | 100% |
| known regression false completion | 0 |
| expanded live holdout success | ≥ 90% |
| live false completion | ≤ 5% |
| no destructive false completion | blocked/impossible/regression never FC |

## Fixture map (default suite, 30)

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

## Fixture map (`--suite real`, 4)

Every one of these is a 11–12 file project whose oracle is the repo's own
`npm test` (`test/run.js`), with green companion suites that must stay green
and the test files listed in `forbidden.txt` so a pass cannot be bought by
editing the contract.

| id | split | scale | what it measures |
|----|-------|-------|------------------|
| real-1-pricing-discount | dev | 11 files / 216 lines | boundary bug (`>` vs `>=`) buried in a pricing module with a green neighbour suite |
| real-2-reservation-stock | dev | 11 files / 183 lines | **two independent bugs** in two files; either fix alone still fails |
| real-3-legacy-dedupe | holdout | 11 files / 171 lines | carried-out ADR: share `formatDate`/`padAmount` through `src/text.js`, delete the private copies (oracle greps that they are gone) |
| real-4-customer-report | holdout | 12 files / 213 lines | implement a spec'd API in `src/report.js` **and** register kind `by-customer` through `src/service.js` (oracle greps the wiring) |

The 291-line whole-corpus default set measures protocol compliance at toy
scale; these four measure whether retrieval depth and edit precision survive a
project that has to be read. Both numbers are reported — the real suite does
not replace the default gate.