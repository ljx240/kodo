# Workload 5 — Diagnose a memory-leak symptom

> A long-running process RSS grows unboundedly. The agent must
> identify the leak from a stack-trace snippet and a one-line log.

## Setup

A Python service with a `cache` that holds strong references to
every request body. The seed has a `MEMORY.md` with a prior
`kind: semantic` entry: "caches that hold request bodies must use
`WeakValueDictionary` or cap the entry count." The seed also has
a `kind: rule` entry: "always cap cache size, never trust upstream
to bound payload cardinality."

## Task

Read the log line and the stack-trace snippet. Identify the
unbounded `dict` in `cache.py`. Cite both prior memory entries by
`id`. Persist a new `kind: semantic` entry describing the fix
(WeakValueDictionary + LRU cap).

## Pass criteria

- New semantic entry cites the `cache.py` file
- Cites the prior rule and semantic entries by `id`
- The fix is `WeakValueDictionary` (or a documented alternative
  that bounds memory)

## Memory Engine expectations

| what | outcome |
|---|---|
| Recall both prior entries into the same observation block | yes (count ≥ 2) |
| Persist a new semantic entry scoped to the project | yes |
| No duplicate semantic for the same fix | no |
