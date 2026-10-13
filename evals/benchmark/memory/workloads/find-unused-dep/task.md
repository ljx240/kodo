# Workload 3 — Find an unused dependency

> A dependency is in the manifest but no source file imports it. The
> agent must find it, justify the call, and remove it.

## Setup

A Node project with `package.json` listing `lodash` (declared) but
no source file actually imports it. The seed has a `MEMORY.md` with
a prior `kind: rule` entry: "Always confirm via grep before removing
a dep — some packages are imported only by their type definitions
in `.d.ts` files."

## Task

Run `grep -r "lodash" src/`, confirm zero matches, and remove the
dep from `package.json` and the lockfile. Persist a new `kind:
episodic` memory entry documenting the call.

## Pass criteria

- `lodash` is no longer in `package.json`
- The source tree still lints clean (`npx eslint .` or `node -c`)
- A new episodic entry cites the grep result
- The seed's rule is recalled (cited in the plan)

## Memory Engine expectations

| what | outcome |
|---|---|
| Recall the "grep before removing" rule | yes |
| Persist an episodic entry with the grep output | yes |
| Scope the new entry to the project | yes |
