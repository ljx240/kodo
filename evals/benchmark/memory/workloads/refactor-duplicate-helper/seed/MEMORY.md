## Entry — refactor rule (procedural)

- **id:** `33333333-3333-3333-3333-333333333333`
- **kind:** procedural
- **status:** active
- **authority:** user_rule
- **source_trust:** tool_verified
- **title:** When two helpers are >80% line-identical, promote one
- **abstract:** Pick the older one as canonical, route the loser
  through a one-line re-export (or delete if no external callers),
  run the full test suite, persist a new procedural memory
  describing the canonical name. Idempotent: do not write a second
  procedural for the same pattern.
- **scope:** project = `/seed/repo`, branch = `main`
- **evidence:** `src/util.py`
