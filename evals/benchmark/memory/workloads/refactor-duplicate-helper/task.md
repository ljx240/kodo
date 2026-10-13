# Workload 2 — Refactor a duplicated helper

> Two near-identical helpers exist in the seed repo. The agent must
> pick one canonical form, route the other call-sites through it,
> and confirm the test suite still passes.

## Setup

A Python module `src/util.py` defines `format_currency(v, currency)`
and a near-duplicate `money_str(v, currency)` with the same behavior
but a different name and a different default-currency argument. The
seed has a `MEMORY.md` with a prior `kind: procedural` entry from
last week's refactor — "when two helpers are >80% line-identical,
promote one to canonical, retire the other."

## Task

Identify the duplicate, decide which to keep, replace the loser
with a thin re-export (or remove if no external callers), and run
the test suite. The agent must cite the procedural memory entry
by `id` in its plan.

## Pass criteria

- Exactly one of `format_currency` / `money_str` remains as the
  implementation; the other is either gone or a one-line re-export
- Test suite passes (`pytest -q`)
- A new `kind: procedural` memory entry is written for this
  refactor (different `id` from the seed's)
- The seed's procedural entry is not duplicated (idempotent)

## Memory Engine expectations

| what | outcome |
|---|---|
| Recall the seed's procedural rule | yes (count ≥ 1) |
| Persist a new procedural entry for the new pattern | yes |
| Re-call-site changes mark affected entries stale | yes (file changed → mark_changed_paths fires) |
