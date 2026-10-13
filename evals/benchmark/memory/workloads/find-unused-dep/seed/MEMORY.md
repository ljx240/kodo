## Entry — grep-before-remove rule (rule)

- **id:** `44444444-4444-4444-4444-444444444444`
- **kind:** rule
- **status:** active
- **authority:** user_rule
- **source_trust:** test_verified
- **title:** Grep before removing a dep
- **abstract:** Before deleting a declared dep, run
  `grep -r "<name>" src/ test/ types/`. Some packages are imported
  only by their `.d.ts` shims and the JS bundle will silently drop
  the symbol; always confirm zero hits in source AND types.
- **scope:** project = `/seed/repo`, branch = `main`
- **evidence:** `package.json`, `types/`
