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

Primary conversation (recorded as `conversation.turns` — three turns, 2026-09-30):

```text
修复 k2k-rust 未知表路由
```

Turn 1 user prompt:

```text
请检查 k2k-rust 项目的未知表路由实现，确认核心功能是否已完成。
现在进行全量本地测试、离线检查和差异审阅，确认没有破坏旧路由与 heartbeat 处理。
```

Turn 1 trace must contain representative items:
- Thinking
- Search codebase
- Read file
- Run `cargo check`
- Run `docker build`
- Call model
- Draft/finalize

Turn 2 (analysis, no file changes): heartbeat 重连路径与积压消息处理说明
(prompt `heartbeat 处理这块只给了结论，没看到细节。…`).

Turn 3 (writing, no file changes): a release-notes summary that marks the two
items needing human confirmation (prompt `好的，把这次检查的结论整理成发布
说明，标注需要人工确认的两项…`).

Turn 1's recording is the one the rest of the demo keeps describing: the
Response Trace page (`/ui-demo/trace`) and the demo Inspector sections read
`conversation.turns[0]`, and the shared `changed_files` / `summary` blocks
belong to turn 1 — turns 2 and 3 are file-free so those totals stay truthful.
The turn rail (COMPONENTS §16) needs ≥2 turns to render, which is why the
fixture holds three.

Changed files count (turn 1):
- 7

Representative aggregate:
- `+737 -46`

The content may be shortened for narrow widths, but screenshot fixtures must not change between runs.

Every fixture conversation carries a `"mode": "work" | "code"` tag like a live
session (`DESIGN.md` §11). The default code space shows the four code sessions
(`c1`–`c4`); the work space shows `w1` (整理架构说明, under realtime-lakehouse)
and `work-plain` (写项目周报, in the 未选择项目 pseudo group). The sidebar tree
filters the fixture by the current mode — screenshots in the default code space
change only in the brand row.

## Demo routes / states

Recommended internal-only demo routes:

```text
/ui-demo/conversation
/ui-demo/conversation?inspector=closed
/ui-demo/trace
/ui-demo/archive
/ui-demo/plugins
/ui-demo/plugins?type=mcp
/ui-demo/plugins?type=agents
/ui-demo/settings
```

The legacy `/ui-demo/skills` and `/ui-demo/mcp` paths remain permanent
aliases of the plugins route (they land on the matching tab).

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
- **Skills** on `/ui-demo/plugins` (技能 tab): the builtin list is the static
  `src/data/skills.ts` (display only — selection happens in the core), the
  项目技能 section renders `DEMO_PROJECT_SKILLS` (one release-check sample)
  and the 自建技能 section renders `DEMO_USER_SKILLS` (one changelog sample),
  both through `api.ts`'s non-desktop path. The demo route keeps a project
  selected (the fixture project), so the project card is always visible in
  screenshots. A real workspace loads its own `<project>/.kodo/skills/*.md`
  and `state_dir()/skills/*.md` via `list_skills` instead.
- **Agents** on `/ui-demo/plugins?type=agents`: `DEMO_AGENTS` in
  `src/data/agents.ts` — two static personas (`reviewer` 严格代码评审,
  `explainer` 把概念讲清楚) with hand-written three-section markdown, served
  through `api.ts`'s non-desktop CRUD path (mutations hit an in-memory copy,
  never disk). Personas are **not** agent-space scoped — both spaces show the
  same list. A real workspace reads `state_dir()/agents/*.md` via
  `list_agents`.
- **MCP servers** on `/ui-demo/plugins?type=mcp`: `DEMO_MCP_SERVERS` in
  `src/data/demo.ts` (one enabled stdio server with a masked env value, one
  disabled http server with a masked header) reaches the page through
  `api.ts`'s non-desktop `loadMcpServers()` path, the same way skills demo
  their sections. The masks (`••••9f3a`) are display-only stand-ins — a real
  workspace loads its own `mcp-servers` entry from `settings.log` with
  per-value masks from `credentials.log`, and saves through `save_mcp_servers`.
  The expanding tool list reads `DEMO_MCP_TOOLS` (names and descriptions only)
  through the non-desktop `listMcpTools()` path instead of `mcp_list_tools`;
  the `plugins-mcp-tools` snapshot expands the filesystem server,
  `plugins-mcp-editor` opens the inline editor. Tool names are wire-adjacent
  code and descriptions stay
  English like the rest of the recorded fixture (DESIGN §10).
  The 连接器目录 above the rows is **not** fixture data: it is static
  `src/data/mcp.ts` (`MCP_TEMPLATES`, 20 tiles) plus `T.page.mcp.catalogDesc`,
  identical on demo and real routes. All three snapshots (`plugins-mcp`,
  `plugins-mcp-tools`, `plugins-mcp-editor`) show the catalog above the two
  demo server rows. Because the
  demo `filesystem` server carries the template's name and package, the
  filesystem tile renders the disabled `已添加` state in those snapshots —
  live coverage of the one-instance-per-connector rule (COMPONENTS §14), not
  a fixture override.
- **Timestamps**: only real sessions call `SystemTime::now()`. `src/data/demo.ts`
  carries literal strings such as `10:24:32` precisely so that a screenshot taken
  next year is identical to one taken today.
- **A live turn has no timestamp.** `kodo-core`'s `Turn` stores the question, the
  items and the outcome, but not the time it was asked, so a reply in a live
  conversation renders no `.msg-time`. This is a known gap, not an oversight: it
  is visible in the UI as a missing time on live messages only.

Fixture rows name tools in English (`Search codebase`, `Read file`,
`Model call #2`, `Finalize answer`) and that text is frozen data — the fixture
must never be edited to translate it. The UI maps tool names to zh labels at
render time via `toolAlias()` in `apps/desktop/src/i18n.ts`, keeping the step
index (`Model call #2` → `调用模型 #2`) and leaving commands, model names and
paths exactly as recorded. Reason: the fixture is a recording; the display
language is a presentation decision.

The Playwright suite runs in a plain browser with no core, so it stubs `invoke`
(`apps/desktop/tests/visual/shell.ts`) and drives the live routes with
hand-written payloads. That covers the GUI's half of the contract; Tauri's own
serialisation and the core's file handling are covered by `cargo test`.
