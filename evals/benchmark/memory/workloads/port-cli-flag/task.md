# Workload 4 — Port a CLI flag to a config key

> Move a single CLI flag from `argv` parsing to a project-level
> config file. The change is mechanical but the failure modes are
> easy to miss.

## Setup

A small Go CLI parses `--strict` from argv. The seed has a
`MEMORY.md` with a prior `kind: procedural` entry: "When porting a
flag to config, also update the help text, the README, AND the
env-var override — three places, one PR."

## Task

Add a `strict: bool` field to `config.toml`. Remove `--strict`
from argv. Keep the env-var override (`APP_STRICT=1`). Update the
help text and the README. Persist a new procedural memory for the
port.

## Pass criteria

- `config.toml` has a `strict` field
- The CLI no longer accepts `--strict` on argv (or, if accepted,
  it is a deprecation shim that prints a warning)
- `APP_STRICT=1 ./app` still flips the behavior
- The help text mentions `strict`
- A new procedural entry is written

## Memory Engine expectations

| what | outcome |
|---|---|
| Recall the "three places" rule | yes |
| Persist a new procedural entry scoped to the project | yes |
| The seed's procedural entry flips to stale (config file changed) | yes |
