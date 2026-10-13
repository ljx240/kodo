## Entry 1 — cache semantic (semantic)

- **id:** `66666666-6666-6666-6666-666666666666`
- **kind:** semantic
- **status:** active
- **authority:** user_rule
- **source_trust:** tool_verified
- **title:** Caches that hold request bodies must use WeakValueDictionary
- **abstract:** A cache keyed by request id but holding the body as
  the value leaks unless the value side is weak. Use
  `WeakValueDictionary` for the body map; key side stays strong.
  Combine with a per-key LRU cap to bound growth when the value
  side is still alive.
- **scope:** project = `/seed/repo`, branch = `main`
- **evidence:** `cache.py`

## Entry 2 — cap rule (rule)

- **id:** `77777777-7777-7777-7777-777777777777`
- **kind:** rule
- **status:** active
- **authority:** user_rule
- **source_trust:** test_verified
- **title:** Always cap cache size, never trust upstream cardinality
- **abstract:** A `dict` with no cap will grow with traffic. Pair
  any cache with a hard upper bound and a real eviction policy
  (LRU or LFU), not just a `maxsize` parameter that no caller
  passes.
- **scope:** project = `/seed/repo`, branch = `main`
- **evidence:** `cache.py`
