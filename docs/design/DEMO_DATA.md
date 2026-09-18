# Deterministic UI Demo Data

Visual regression requires stable content.

Do not use live current time, random IDs, actual local repositories, or model API responses in screenshot tests.

Use the fixture:

`demo/conversation.fixture.json`

## Required fixed state

Project:

```text
realtime-lakehouse
~/projects/realtime-lakehouse
branch: main
```

Primary conversation:

```text
修复 k2k-rust 未知表路由
```

User prompt:

```text
请检查 k2k-rust 项目的未知表路由实现，确认核心功能是否已完成。
现在进行全量本地测试、离线检查和差异审阅，确认没有破坏旧路由与 heartbeat 处理。
```

Trace must contain representative items:
- Thinking
- Search codebase
- Read file
- Run `cargo check`
- Run `docker build`
- Call model
- Draft/finalize

Changed files count:
- 7

Representative aggregate:
- `+737 -46`

The content may be shortened for narrow widths, but screenshot fixtures must not change between runs.

## Demo routes / states

Recommended internal-only demo routes:

```text
/ui-demo/conversation
/ui-demo/conversation?inspector=closed
/ui-demo/trace
/ui-demo/archive
/ui-demo/settings
```

If desktop routing makes URLs inconvenient, expose the same states through a development-only fixture switch.

## Where the fixture ends and real data begins

Kodo now keeps a real workspace on disk for real use. The boundary is the route:

| Route | Source | Clock |
|---|---|---|
| `/ui-demo/*` | `demo/conversation.fixture.json` + `src/data/demo.ts` | frozen strings, never read |
| everything else, in the desktop shell | `~/Library/Application Support/Kodo/` | real |

Concretely:

- **Projects** live in `projects.log`, append-only, folded into the current list.
  The fixture's five projects (and their conversations) are demo-only; a real
  workspace starts empty until a folder is registered.
- **Sessions** live one file per conversation under `sessions/<id>.log`. The
  fixture conversation is never written anywhere.
- **Timestamps**: only real sessions call `SystemTime::now()`. `src/data/demo.ts`
  carries literal strings such as `10:24:32` precisely so that a screenshot taken
  next year is identical to one taken today.
- **A live turn has no timestamp.** `kodo-core`'s `Turn` stores the question, the
  items and the outcome, but not the time it was asked, so a reply in a live
  conversation renders no `.msg-time`. This is a known gap, not an oversight: it
  is visible in the UI as a missing time on live messages only.

The Playwright suite runs in a plain browser with no core, so it stubs `invoke`
(`apps/desktop/tests/visual/shell.ts`) and drives the live routes with
hand-written payloads. That covers the GUI's half of the contract; Tauri's own
serialisation and the core's file handling are covered by `cargo test`.
