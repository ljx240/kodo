# Kodo Design System

## 1. Product character

Kodo is a work/coding-agent workstation with two separate Work / Code agent
spaces (see §11).

It should feel closer to a restrained IDE than to:
- a SaaS dashboard;
- a chat consumer app;
- a project-management tool.

Priority order:

1. conversation readability;
2. agent execution visibility;
3. information density;
4. fast project/session switching;
5. minimal chrome;
6. visual polish.

The UI must stay visually quiet.  
The content — conversation, commands, files, results — is the product.

---

## 2. Primary visual language

### Theme

V1 primary theme: **Light**

Use neutral whites and cool grays with one restrained blue accent.

```text
Canvas                #FFFFFF
Sidebar                #FAFBFC
Secondary surface      #F7F8FA
Hover                  #F3F6FA
Selected               #EEF5FF
Border                 #E5E9F0
Border strong          #D7DEE8

Text primary           #172033
Text secondary         #667085
Text muted             #98A2B3

Accent                 #2563EB
Accent hover           #1D4ED8
Success                #16A34A
Warning                #D97706
Error                  #DC2626
```

Do not use gradients in normal application chrome.

### Typography

Use system UI font first.

```text
UI font:
-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif

Code:
"SFMono-Regular", Consolas, "Liberation Mono", monospace
```

Base sizes:

```text
Primary UI text        13px
Secondary text         12px
Micro metadata         11px
Conversation body      14px
Code / commands        12px
Page title             18px
Section title          13px / 600
```

Typical line height:

```text
UI                     1.35
Conversation            1.55
Code                    1.45
```

Do not enlarge text to make the UI feel “premium”.  
Kodo uses compact IDE density.

---

## 3. Density

Primary reference viewport:

```text
1586 × 992
```

Secondary validation:

```text
1440 × 900
1920 × 1080
2560 × 1440
```

At 1586×992, the user should simultaneously see:
- project tree;
- multiple nested conversations;
- several agent trace items;
- final response;
- composer;
- optional Inspector content.

No oversized empty hero areas.

---

## 4. Shape language

Radius:

```text
Input / small control       5px
Panel                       6px
Popover                     8px
```

Avoid frequent 12–20px rounded cards.

Borders:
- 1px subtle separators;
- use boundaries more often than shadows.

Shadows:
- only floating menus, dialogs and transient overlays;
- no shadow around every content group.

---

## 5. Spacing

Base scale:

```text
2 / 4 / 6 / 8 / 12 / 16 / 20 / 24
```

Default compact control height:

```text
Small control        28px
Default control      32px
Toolbar              44px
Composer min         54px
```

Conversation content should use whitespace for hierarchy, but avoid card-per-message layouts.

---

## 6. Icons

Use one consistent line-icon family.

Rules:
- 14–16px in dense rows;
- 16–18px in primary navigation;
- filled icons only for strong status emphasis;
- tool types may have semantic icons;
- do not mix multiple icon styles.

---

## 7. Agent trace visual language

Agent work is shown inline inside an assistant reply.

A trace item has:

```text
status | action | short detail | metadata / duration | disclosure
```

Examples:

```text
✓ Thinking          分析需求，制定检查计划                    12s
✓ Search codebase   搜索 k2k / router / heartbeat             8s
✓ Read file         source/db/source.table                    14s
✓ Run command       cargo check                               18s
✓ Call model        claude-3-5-sonnet    12.4k → 2.1k        24s
```

Rules:
- trace is readable at a glance;
- long outputs stay collapsed by default;
- active item may expand;
- failed item remains expanded;
- completed output should not dominate the conversation.

---

## 8. Visual hierarchy

The hierarchy is:

```text
Project / conversation context
        ↓
User request
        ↓
Agent execution trace
        ↓
Final answer
        ↓
Changed files
        ↓
Composer
```

Inspector is secondary and must never visually overpower the conversation.

---

## 9. Minimalism rules

Do not add visual elements just because there is empty space.

Prefer:
- separators over containers;
- inline metadata over separate cards;
- compact rows over tile grids;
- nested tree navigation over duplicate sidebars;
- progressive disclosure over permanently visible detail.

The one exception: the MCP connector catalog is a tile grid (user-approved
2026-09-30, `COMPONENTS.md` §14) — compact tiles with the panel radius and 1px
borders, no shadows. No other surface converts rows into tiles.

Kodo should look calm even when it contains a lot of information.

## 10. Copy policy

All user-facing chrome copy is zh-CN and lives in one place:
`apps/desktop/src/i18n.ts`. Components never hardcode a user-facing string;
they read `T.*`. Reason: wording drifts per page when labels live next to the
markup, and the answer-experience work made status/trace vocabulary shared
across four surfaces (reply, trace, inspector, buttons).

Recorded English exceptions — keep as-is:

- brand: `Kodo`, `Get code done.`, and the space wordmarks `Kodo Work` /
  `Kodo Code` (welcome-stage identity, §11);
- product nouns that are names, not sentences: `Inspector`, `AI Provider`;
- model names, commands, file paths, file names, code chips;
- fixture data recorded in English (see `DEMO_DATA.md`).

Fixture and legacy tool names arrive in English (`Search codebase`,
`Model call #2`, `Finalize answer`). The UI maps them to zh labels through
`toolAlias()` in `i18n.ts` — tool names only. Step indexes fixtures record are
kept (`Model call #2` → `调用模型 #2`), and commands, models and paths inside a
row stay exactly as recorded.

---

## 11. Work / Code modes

Kodo runs two **separate agent spaces** — Work and Code — switched from the
Codex-style brand dropdown in the sidebar (the brand row is the trigger and
its label names the current space: `Kodo Code ˅` / `Kodo Work ˅`, so the
top-left always says which space is showing; the menu check-marks the current
space). Switching swaps the whole
space below it: the session list, the 插件页 · 技能 tab (two skill lists), and
new-task behavior. The two never mix in one list.

Each session is stamped `work` or `code` **at creation and the tag is
immutable** (like history). The session's own tag is the source of truth for
`RunRequest.mode` and for the project-required gate — recorded history never
changes. The shared settings key `agent-mode` (default `code`) only stamps
**new** sessions; desktop and CLI read the same key.

Two spaces, one project registry: the project tree structure is shared across
modes; only the conversations listed under it filter by the current space. A
project with zero current-space sessions still shows its row. Sessions with no
project gather under the 未选择项目 pseudo group in their own space only.

Switching while a conversation of the other space is open drops back to the
welcome stage — the conversation stays in its own space and switching back
finds it there.

The welcome stage names its space: the mode logo (blue tile for Code, violet
mark for Work) with the `Kodo Code` / `Kodo Work` wordmark sits above the
title (`welcome-identity`, driven by the current space), so a fresh 新建任务
is unambiguous before the first message.

Mode × project is the core semantics:

| | project selected | no project |
|---|---|---|
| **code** | full coding agent: symbol tools, file tools, websearch, verify phase | websearch only — the model can answer fresh-fact questions without a workspace |
| **work** | work skill: file tools + search + run_command + websearch, no symbol tools, no Verify phase | websearch only — same project-independent tool, no project files exposed |

The `websearch` tool is the lone project-independent tool: it ships in every
(mode, project) cell so the model can answer fresh-fact questions even in a
projectless conversation. Ask + Auto permissions gate the network egress
exactly like `ExternalTool`; Full passes through. Backend is DuckDuckGo HTML
(no API key, zero new dependencies — `crates/agent/src/websearch.rs`).

Rules:
- Work mode is for general work tasks — docs, summaries, Q&A, writing. File
  writes stay inside the selected project and remain subject to the Permission
  mode (approval bar unchanged).
- Work mode never runs the verify pipeline; deliverable quality is the bar.
  Work plans carry `deliverable_bar`: the model's claim is the delivery, so
  free-form completion criteria become the answer's checklist instead of a
  machine gate (code skills keep the fail-closed evidence bar — a claim alone
  can never open Finish there). Tools still enforce path containment and the
  Permission mode at execution time.
- A project is optional in both modes (2026-09-29: the Code-mode project
  gate is gone — projectless Code is Q&A). Without one the run is
  **constrained Q&A**: `ToolRegistry::for_mode` hands back an empty registry,
  verify is clamped off, and only absolute attachment paths are accepted —
  the response cannot touch the system. Such sessions group under
  未选择项目 like any other conversation. A fresh
  `新建任务` preselects the remembered project (`kodo:last-project`) when it is
  still registered; `清除项目选择` deselects only — it never removes a project.
  The remembered project is one shared key across both spaces.
- Conversational questions take a dedicated channel (2026-09-29): a run is
  conversational when there is no project, or when a knowledge/explanation
  question arrives with no pinned skill chip (conservative detector: a
  knowledge marker such as 什么是/介绍/解释/区别, and an explicit task verb
  like 修复/实现/添加 always wins). Conversational runs carry an empty plan —
  no skill criteria, no plan gates, no acceptance bounce; the first prose
  answer ends the run. The answer lands `plain` with axes `ready` +
  `not_run` (the UI renders the sentence alone, outcome 已完成) and carries
  no `**Verification status:**` footer. A pinned skill chip is explicit
  task intent and keeps the full machinery.
  Attachments reach the model in every branch (2026-09-29): pinned paths are
  inlined into the first user message as observations — spreadsheets and
  text extract to (truncated) content, binary/Office formats degrade to a
  path-and-kind descriptor (「已附带附件（内容未内联）」) — so a
  conversational or projectless answer is grounded in its attachments even
  though no tools run. The greeting shortcut yields whenever an attachment
  is present.
- Legacy sessions with no `mode` line in their log read as `code` (no
  migration).
- The archive page stays cross-mode (storage management); a restored session
  returns to its own space with its tag intact.
- External tools (MCP servers) attach to a run as `mcp__<server>__<tool>` wire
  tools alongside the builtins (`ToolInvocation::External`); mail/calendar stay
  out of scope. Skill `allowed_tools` is the gating insertion point — see rule
  7 below.

### Skill selection

A skill is chosen in `run()` (`crates/agent/src/lib.rs`) before classification,
so clicking a chip is deterministic:

1. **Chip protocol.** The composer serializes picked skills as leading
   `【技能：<id>】` whole lines above the draft (`Composer.tsx`, contract frozen
   in `docs/superpowers/specs/2026-09-21-conversation-state-and-skill-composer.md`).
   `split_skill_chip` consumes only leading whole-line markers whose id matches
   `[A-Za-z0-9_-]{1,64}`; imitations mid-text are ignored. The provider-visible
   `request.message` keeps the prefix — only `classify()` sees the stripped
   text, so the chip can no longer pollute keyword scoring.
2. **Resolve + space filter.** `resolve_skill` looks each id up with
   `registry.get(id)` and honors it only if the skill is in the current space:
   Work mode requires `applicable_task_types` to contain `work`; Code mode
   requires it not to. Out-of-space or unknown ids fall through — Work stays
   pinned to `TaskType::Work`, Code falls back to `classify(stripped text)`,
   and the registry's `select(task_type)` picks the default builtin.
3. **Registry order.** `select()` is first-match, so the `workflow` skill is
   registered **after** `work` (guarded by the `workflow_registration_keeps_work_selectable`
   test) — general Work tasks keep the `work` skill unless a chip says
   otherwise.
4. **User and project skills.** User files live in `state_dir()/skills/<name>.md`;
   project files live in `<project>/.kodo/skills/<name>.md` (same flat layout,
   git-friendly, excluded from change tracking). Filename = chip id = `# Skill:`
   title, validated by `parse_skill` (description, task types, context strategy,
   allowed tools, verification policy, workflow steps, completion criteria).
   `build_skill_registry` registers builtins → project → user, so `select()` can
   never be hijacked — a project/user skill is reachable **only** via its chip —
   and on a name collision the project copy shadows the user copy (git
   local-over-global convention). Builtin names stay reserved in both dirs.
   Tests/evals pass `RunRequest.user_skills_dir = None` (project dir is derived
   from the run's project root and is usually absent in temp fixtures) and stay
   hermetic. The desktop shell reads, previews and edits them through
   `list_skills` / `read_skill` / `save_skill` / `delete_skill` with a `scope`
   of `user` or `project` (`COMPONENTS.md` §13b/§13c); builtin names and
   display-layer ids (`kodo-ui`, `doc`, `summary`, `qa`) are refused as
   filenames.
5. **Work guarantees are structural, not advisory.** The `workflow` skill
   stamps `deliverable_bar` (work-family plans) and declares
   `verification_policy: none`; regardless of what any skill file claims,
   `clamp_plan_verification` forces `requires_verify = false` when mode is Work
   or there is no project — Work never runs the verify pipeline (§11 rule).
6. **Guidance scope (progressive disclosure).** A skill's `description` and
   `guidance` reach the model **only when the skill is matched** — they ride in
   the `skill_prompt_block` appended to the system prompt (banner + `Skill
   description` + `Skill guidance`). Workflow steps and completion criteria
   travel in the plan block instead and are not duplicated. No skills catalog
   is injected: selection is deterministic (chip + classify), so a catalog
   would only burn tokens. Both fields stay visible in the 插件页 技能 tab's
   preview and the editor.
7. **Tool gating: allowlists narrow builtins only.** `allowed_tools` filters
   the builtin `ToolRegistry` (`filter_registry`); external MCP tools
   (`mcp__*` wire names) always pass the skill gate (`SkillSpec::allows_label`
   returns true for the prefix) and always survive registry filtering. A skill
   can never block a server tool the project configured — but it also never
   widens one: every external call still goes through
   `Permission::needs_approval` exactly like builtins. This keeps read-only
   skill guarantees intact (they constrain builtin tools) while leaving
   external risk to the Permission mode.

Builtins are eight embedded files (`skills/<id>/SKILL.md`, `include_str!` via
`BUILTIN_SOURCES`): six code skills (bug-fix, feature, test, refactor,
code-review, docs) and two work skills (work, workflow). The frontend's
display lists carry a few extra presentation ids that have no backend file —
`kodo-ui` in code space; `doc` / `summary` / `qa` in work space (all render the
`work` skill's behavior via mode defaults). Reserved filename sets in the
backend and the skill editor cover both groups.

---

## 12. MCP external tools

MCP servers extend a run with external tools without opening the tool protocol:
builtins stay a closed `ToolName` enum, and server tools enter as
`ToolInvocation::External` under wire names `mcp__<server>__<tool>` (double
underscore — provider tool-name validators reject colons).

- **Configuration.** `settings.log` key `mcp-servers`: a JSON array of
  `{id, name, enabled, transport: "stdio" | "http", command, args, env, url,
  headers}`. Env and header **values** live only in `credentials.log` (per
  server under `mcp.<id>`, 0600); the settings copy stores empty strings and
  the GUI round-trips masks like provider API keys. Secret values never pass
  through `settings.log` or the webview.
- **Transports.** stdio spawns a persistent local process (process-group kill
  on drop); http POSTs to a Streamable HTTP endpoint (JSON or SSE responses,
  optional `Mcp-Session-Id` capture/replay, best-effort DELETE on drop). Both
  are synchronous — the stack stays blocking `ureq` + `std::thread`, no async
  runtime. Handshake: `initialize` (pinned `protocolVersion: "2026-07-28"`,
  echoes accepted) → `notifications/initialized` → `tools/list` (with
  `nextCursor`) → `tools/call`.
- **Lifecycle.** `run()` connects the enabled servers after the
  direct-conversation early return and only when a project is present —
  projectless Q&A stays sealed (§11). One broken server is a zh-CN note, never
  fatal. Tool discovery is cached per run; `Drop` tears the transports down on
  every exit path.
- **Permission and trace.** An external call is `StepKind::ExternalTool`: Ask
  and Auto both approve it, Full passes. It traces as `Step::Command` whose
  detail carries the wire name and redacted args — no new `ItemKind`/`Step`
  variants, the Inspector is unchanged.
- **Skill gating** is rule 7 under Skill selection: allowlists narrow builtins
  only; `mcp__*` always passes the skill gate and always faces Permission.

The management UI is the 插件页 MCP tab (`COMPONENTS.md`
§14/§13d) — a connector catalog of compact tiles (the documented tile exception,
DESIGN §9) that prefills the add editor, one instance per connector (an
already-configured template renders a disabled 已添加 tile), plus the server
list with enable toggles, per-server tool lists (connected once via
`mcp_list_tools`, names and descriptions only) and inline editing. Settings →
工具与权限 keeps only the permission mode.
The demo fixture boundary is documented in `DEMO_DATA.md`.
