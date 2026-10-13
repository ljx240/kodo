## Entry — three-places rule (procedural)

- **id:** `55555555-5555-5555-5555-555555555555`
- **kind:** procedural
- **status:** active
- **authority:** user_rule
- **source_trust:** tool_verified
- **title:** When porting a flag to config, update three places
- **abstract:** CLI flag → config key requires touching (1) the
  config file, (2) the help text, (3) the README. Also keep the
  env-var override as a power-user escape hatch. One PR, three
  diffs minimum.
- **scope:** project = `/seed/repo`, branch = `main`
- **evidence:** `main.go`, `config.toml`, `README.md`
