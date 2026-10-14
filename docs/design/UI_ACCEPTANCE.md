# Kodo UI Acceptance Criteria

A screen is complete only after visual verification.

Compilation, passing type checks, and functional correctness are necessary but not sufficient.

---

## 1. Reference viewport

Primary screenshot size:

```text
1586 × 992
```

All primary reference screenshots use this viewport.

Also smoke-test:

```text
1440 × 900
1920 × 1080
```

---

## 2. Geometry tolerance

At the reference viewport:

```text
Sidebar width difference           <= 6px
Inspector width difference         <= 8px
Top bar height difference          <= 4px
Major content start position       <= 8px
Repeated row height difference     <= 3px
Major spacing difference           <= 6px
Font size difference               <= 1px
```

Do not chase pixel-level antialiasing differences.

---

## 3. Mandatory layout checks

Conversation screen must satisfy all:

- [ ] one left sidebar only;
- [ ] projects appear in left sidebar;
- [ ] conversations expand directly below each project;
- [ ] no second project/conversation column;
- [ ] the turn rail (≥2 turns) is an overlay in the conversation's right
      gutter — it consumes no layout width and never reads as a second
      column; it is hidden below 2 turns and on containers <900px;
- [ ] the visible conversation stays at least 400px wide on wide viewports;
      dragging beyond that boundary hides it while retaining its run and draft;
- [ ] Inspector can be opened and collapsed;
- [ ] collapsed Inspector becomes a narrow rail;
- [ ] file viewing happens in the right sidebar's `文件` panel — never a
      modal or dialog over the conversation;
- [ ] main conversation expands when Inspector collapses;
- [ ] composer remains usable at bottom;
- [ ] while a run is active, at least 6–8 compact trace rows fit at the
      reference viewport (asserted in `behaviour.spec.ts`); a completed reply
      shows no inline trace rows.

---

## 4. Trace checks

- [ ] trace belongs to a specific assistant response;
- [ ] running state is visible;
- [ ] each step has action + detail + duration/status;
- [ ] command/tool output can expand;
- [ ] completed long output is not permanently expanded;
- [ ] failed output is prominent enough to diagnose;
- [ ] final answer is visually distinct from execution trace;
- [ ] once the final reply lands the inline trace is gone — the conversation
      shows only the final reply body (process detail lives on the Response
      Trace page); the trace stays visible while the run is live or when a
      turn ends with no final reply;
- [ ] user can open detailed Response Trace;
- [ ] opening the Response Trace mid-run never interrupts the Q&A — the run
      keeps streaming underneath and the back control returns to that same
      live state (asserted in `behaviour.spec.ts`);

> 2026-09-29 markdown rendering pass (FinalResponse and the file viewer 预览
> share one renderer):
> - [ ] headings render in three tiers, visibly above the 14px body;
> - [ ] a GFM table renders header + rows with column alignment, and a wide
>       table scrolls horizontally inside its wrapper;
> - [ ] nested lists indent one level at a time; `- [ ]` / `- [x]` render as
>       disabled checkboxes;
> - [ ] a multi-paragraph quote renders behind a left rule, one line per
>       paragraph;
> - [ ] `$…$` / `$$…$$` render as KaTeX while currency like `$5` stays
>       literal text;
> - [ ] ```` ```mermaid ```` shows `正在绘制图表…` then an SVG; a bad
>       diagram keeps the original code block visible;
> - [ ] ```` ```diff ```` colors `+`/`−` rows;
> - [ ] ≥4 headings grow the auto-TOC and a click scrolls to the heading;
> - [ ] `---` draws a rule, and non-http(s)/mailto link text stays plain.

> 2026-09-23 quiet-trace pass:

- [ ] no status icons on trace rows (state is words: `进行中` / `已中断` /
      `已停止` / `失败`, never color alone);
- [ ] a failed row has exactly one red emphasis (`原因：…`); exit chips are neutral;
- [ ] unmeasured durations render empty, never a bare `—`;
- [ ] command output is display-clean (ANSI escapes stripped);
- [ ] no streaming draft block above the trace — the answer appears only when final;
- [ ] interim stream prose renders as quiet note rows in arrival order inside
      the trace, and the draft of the final answer never doubles the reply;
- [ ] two or more consecutive same-type steps fold under one group row
      (label + `N 次` + total duration); expanding shows each step at the
      second level, each of which opens its own detail;
- [ ] a group that is running or holds a failure is open on its own — problems
      are never hidden behind the fold.

---

## 5. Inspector checks

- [ ] default expanded width approximately 330px;
- [ ] panel chips on the header row are exactly `Inspector` | `终端` | `文件` |
      `浏览` (route-dependent; the trace route has no 文件/浏览), with the
      active chip `aria-selected`;
- [ ] the rail shows `概览` / `改动` / `工具` / `模型` and is hidden while
      `终端`, `文件` or `浏览` is open — a panel and a rail section never share
      a name (`rail-files` is labelled 改动);
- [ ] supports Summary;
- [ ] supports Changed files (rail section 修改文件);
- [ ] supports Tools used;
- [ ] supports LLM calls;
- [ ] supports Terminal/output when relevant;
- [ ] `文件` panel: the file viewer sits inside `.ins-panel` (no modal, no
      backdrop, no dialog role); on a ≥1200px window the panel widens past
      330px toward 640px while open and returns to 330px when demoted;
- [ ] `文件` demotion: `Escape` and the header X return to `概览` with the
      panel still open; X from `概览`/`终端` closes the panel;
- [ ] `浏览` panel: URL bar + `前往` (Enter submits); only http(s) URLs load —
      `javascript:` shows the inline `browse-error` and never creates an
      iframe; a loaded site renders inside the sandboxed iframe with the foot's
      在系统浏览器打开 visible; a hanging embed (no load within 6s) shows the
      refusal banner with 在系统浏览器打开 + 重试; `Escape`/X demote to `概览`
      with the panel open; the column widens from the 640 wide key; no
      history, no tabs, no context menu;
- [ ] at ≤1199px the `文件` panel overlays (clamped) instead of widening, and
      the tree hides behind the FolderTree toggle (`tree-toggle`,
      `aria-expanded`), dropping in as a left drawer that closes on select;
- [ ] width handle (`inspector-resizer`, `role="separator"`,
      `aria-orientation="vertical"`, `aria-valuenow/min/max`): present only
      while the panel is open on ≥1200px windows, straddling the conversation
      ↔ panel boundary; drag resizes from 320px to the available workspace width
      (`aria-valuemax` tracks), `←`/`→` step 16 (Shift 64), `aria-valuenow`
      tracks the width, and while dragging the shell carries `app--resizing`
      (col-resize cursor, width tween off);
- [ ] base and `文件` widths are two independent keys (defaults 330 / 640):
      resizing the `文件` panel never moves the default width and vice versa;
- [ ] **focus mode**: once the conversation reaches 400px, further leftward
      dragging hides it before pointer release; the root carries `app--focus-mode`
      and the Inspector fills the workspace without a fixed 900/1100px cap;
- [ ] **focus mode triggers**: pointer and keyboard resizing use the same
      boundary; double-click toggles the mode;
- [ ] **focus mode exits**: the first rightward move restores the conversation
      at its minimum, then continues resizing; this works within one gesture
      and on a new gesture. Double-click restores the saved base/wide width;
- [ ] window/sidebar resizing updates the panel limit and `aria-valuemax`;
      normal mode clamps the panel to preserve the conversation minimum;
- [ ] `inspector-focus-mode` persists the hidden state without changing the
      base/wide widths; legacy `inspector-focus-width` values are ignored;
- [ ] cancelling a drag or losing the handle clears transient drag state and
      retains the saved layout; narrow viewports keep the conversation visible;
- [ ] below 1200px there is no handle and no inline width — the overlay clamp
      stays token-driven;
- [ ] panel switching animates in place — enter-only, 120ms ease (open slide
      6px, chip/section re-enter 8px), flattened by `prefers-reduced-motion`;
- [ ] each section uses compact rows;
- [ ] panel does not duplicate all timeline detail;
- [ ] collapsed rail remains available.

---

## 5a. Sidebar checks

> Updated when the sidebar IA changed: `Conversations`/`Archive` nav rows were
> replaced by an explicit `新建任务` action plus destination rows above the
> tree, and `Archive` became a Settings destination. Reason: conversations are
> reached through the project tree (one tree, no second conversation list), a
> new task is an action rather than a destination, and archiving is a
> management concern that belongs with storage. `MCP` joined `技能` as a
> destination row on 2026-09-29 (it left Settings → 工具与权限). Labels are
> zh-CN via `apps/desktop/src/i18n.ts` (see `DESIGN.md` §10).
>
> Updated 2026-09-30: the sidebar destination rows are now exactly `新建任务`
> + `插件`. 技能, MCP and the new 智能体 are tabs on the one 插件 page
> (`/plugins`, `?type=skills|mcp|agents`; the old `/skills` and `/mcp` paths
> are permanent aliases of that route). See §5i and `COMPONENTS.md` §13d/§14.
>
> Updated 2026-10-13: `工作台` and `定时任务` joined the rows, so the order above
> the tree is `工作台` → `新建任务` → `插件` → `定时任务` (`LAYOUT.md` §2). The
> workbench's apps (待办 / 日历 / 成果中心) open inside `/workbench` and are not
> nav rows; the sidebar keeps one `工作台` row while an app is on screen.

- [ ] no account row, avatar or sign-out anywhere in the sidebar;
- [ ] `工作台`, `新建任务`, `插件` and `定时任务` are the only nav rows above
      the tree, in that order; the 插件 page itself shows the three tabs
      技能 / MCP / 智能体 (技能 selected by default, no `?type=` on the default
      URL);
- [ ] `新建任务` is an action and never renders as the active destination;
- [ ] `Settings` (`设置`) sits at the foot, below the scrollable tree;
- [ ] `Archive` (`归档`) is reached via Settings → 归档与存储 → 查看归档, never as a permanent nav row;
- [ ] `+` beside `Projects` offers "add existing folder" and "new project";
- [ ] a project row's `+` (left of `⋯`) starts a new task under that project
      in one click;
- [ ] conversation rows sit tight under their project (thread-line indent
      ≈18px — never parked deep under the project name);
- [ ] a project row's `⋯` offers rename, reveal in Finder, remove from list;
- [ ] renaming a project does not rename the directory;
- [ ] removing a project does not delete the directory;
- [ ] a group with more than 5 conversations shows the first 5 plus a
      `加载全部会话` row; clicking it reveals the rest and the control
      retires; 5 or fewer never grows the control, and the open conversation
      is never folded away;
- [ ] a filter row (`筛选会话…`) sits above the tree while the space has
      conversations: title hits are case-insensitive, project rows and the
      open conversation always stay visible, matches beyond the 5-row fold
      appear directly (no `加载全部会话` detour), and an empty result
      reports `没有匹配会话`;
- [ ] a conversation row's `⋯` offers rename, archive and `删除会话`; delete
      needs the second click on `确认删除`, and a live run refuses with
      `会话运行中，无法删除` while the row stays put;
- [ ] dragging a project row reorders the list.

---

## 5b. Settings checks

> Updated when `AI Provider` became its own category (it was folded into
> Models before): model/provider configuration is a first-class concern, so the
> list is seven categories, not six. The model-chip check now covers the
> composer chip only — the top bar stopped repeating the model on 2026-09-23
> (it is chosen on the composer; the top bar keeps location context).

- [ ] left column lists all seven categories (通用, 模型, AI Provider, 工具与权限, 项目, 归档与存储, 外观);
- [ ] selecting a category swaps the right-hand pane;
- [ ] no category opens onto an empty pane;
- [ ] the selected category is visibly marked;
- [ ] the page head (title, subtitle, search) is unchanged;
- [ ] the `Default model` control and the composer's model chip show the same value;
- [ ] a provider row's `测试连接` probes without opening the editor and reports
      inline — `连接成功 · N 个模型` or `连接失败（reason）`; the editor's own
      guards (no usable key, custom endpoint without a base URL) report their
      text without fetching;
- [ ] no row controls a setting that no longer exists.

---

## 5c. Archive checks

- [ ] dense table, not cards;
- [ ] columns: Conversation, Summary, Archived at, Model, Files changed, Status;
- [ ] filters sit in one compact toolbar above the table;
- [ ] selecting a row opens details in the Inspector;
- [ ] Restore is an Inspector action, not a row action;
- [ ] pagination footer reports the range and total.

---

## 5d. Behaviour checks

These are asserted in `apps/desktop/tests/visual/behaviour.spec.ts` and
`interaction-ux.spec.ts`. A screenshot cannot check any of them.

- [ ] clicking anywhere along a trace row opens it, not only the chevron;
- [ ] every trace row reports `aria-expanded`;
- [ ] the keyboard reaches a trace row (Enter and Space);
- [ ] a step that starts with no output opens when it completes, and stays open;
- [ ] composer sends on Enter, clears, and leaves Shift+Enter for newlines;
- [ ] a running composer shows stop, not send;
- [ ] a finished reply does not claim to be working;
- [ ] a killed run reports as interrupted, never as finished or working;
- [ ] a recovered interrupted turn still reports as interrupted, never as working;
- [ ] a failover event shows failed provider/model, error class, and next model;
- [ ] the Response Trace tabs each swap the pane;
- [ ] an Inspector card collapses and restores from its header;
- [ ] a new conversation's first answer lands live (run finishing before its id
      never strands the turn at 执行中);
- [ ] a conversational answer renders as the sentence alone (no report chrome);
- [ ] a projectless run or a knowledge question (no skill chip) answers through
      the conversational channel: plain sentence, axes 已生成 + 未验证
      (outcome 已完成), no `**Verification status:**` footer, no conclusion
      card and no verification card;
- [ ] the auto onboarding advances itself and never returns once dismissed;
- [ ] no streaming draft is shown while an answer streams (no answer surface
      until the final reply lands);
- [ ] interim stream prose appears as trace notes in normal order, and the
      trailing draft of the final answer is discarded, never duplicated;
- [ ] run output renders without ANSI escape garbage;
- [ ] consecutive same-type steps fold under one group row (second level per
      call, third level per detail); a group holding a failure opens itself;
- [ ] a project row's `+` starts a new task under that project in one click;
- [ ] after the final reply the execution trace and step/token footer leave
      the conversation and only the reply remains (the trace stays while the
      run is live or the turn has no final reply);
- [ ] the Response Trace page returns to the conversation — the back control
      in the page bar, or clicking a conversation in the sidebar;
- [ ] the reply's bottom actions are icon-only — 复制回复 / 重新生成 /
      查看响应轨迹 as 22px icon buttons whose labels live on `title` +
      `aria-label` (2026-09-30);
- [ ] provider tool-call protocol (JSON `tool_calls` and the tag protocol
      `tool_call` / `function=NAME` / `parameter=KEY`) never leaks into the
      final reply — only the prose answer renders;
- [ ] the changed-files card lists this turn's Kodo edits only (merged per
      path, with edit counts) — never the pre-existing dirty working tree;
- [ ] `查看修改` opens the `文件` panel in the right sidebar (tree +
      完整文件 / 变更) at the first changed path; a changed-file row opens it
      at that file; demo mode falls back to the Inspector `改动` section —
      the conversation is never covered by a modal;
- [ ] the top-bar 文件 icon opens the same `文件` panel as a project file
      browser (tree visible, nothing preselected); demo mode falls back to
      the `改动` section;
- [ ] click-through opens the `文件` panel in place: a path-shaped inline
      code in the answer (`md-code-path` button) opens the full tab after a
      probe; a project-internal context card in a sent message opens the
      panel; an edit-step file chip in the expanded trace opens the 变更 tab.
      Non-paths (version numbers like `v1.2`, URLs, `..`, spaces) and
      external absolute context paths never render as clickable;
- [ ] the file viewer offers a 预览 tab for markdown and HTML (HTML runs its
      scripts in an isolated `allow-scripts` sandbox — visible effect, no
      parent-window access), a path filter and a 仅变更 toggle on the
      tree, and a 统一/对照 diff with long unchanged runs folded behind
      展开 N 行上下文;
- [ ] syntax highlighting is visible in 完整文件 and markdown code blocks
      (`.hljs-*` spans); unknown languages and oversized files degrade to
      plain text without errors;
- [ ] image / PDF / Office / binary files never surface 读取失败 — images
      inline (or a kind card when oversized), CSV as a table, and the kind
      card's 用系统应用打开 hands the path to the OS;
- [ ] image attachments ≤ 4 MiB reach the model as multimodal content when
      the selected model is vision-capable (`ProviderCapabilities::vision` +
      `ModelSpec::vision_override`); oversize or vision-incapable models
      fall back to the path-only descriptor, and the first vision-incapable
      + image send shows a one-time banner before dispatching (subsequent
      sends in the same session are silent);
- [ ] trace and Inspector diffs render +/− colored rows with file headers
      stripped (no raw `--- a/` / `+++ b/` lines);
- [ ] the project file tree still shows the project's real content on
      junk-heavy projects: git-tracked files survive the list cap (an
      untracked venv-backup pile never crowds them out), `.venv.*` backups
      and `.pytest_cache` stay out, and image / PDF / Office files are
      browsable — opening one previews or shows the kind card;
- [ ] every changed file can show its change content: the 变更 tab exists for
      any changed path even when the changeset holds no diff body (a create
      synthesizes its additions; a bodyless edit falls back to an honest note) —
      a +n −m badge alone is never the whole story;
- [ ] undo feedback matches the report — restored / nothing to undo / conflict
      detail, never a false 已撤销;
- [ ] ⌘F opens the in-conversation find bar: `n/total` counting (or
      `无匹配`), Enter / Shift+Enter cycle turns with the active hit outlined,
      Escape closes;
- [ ] the turn rail (≥2 turns, container ≥900px) click on a tick scrolls that
      turn into view and disengages stick-to-bottom (scrolling up never yanks
      the view back down);
- [ ] hover/focus on a tick raises the preview — prompt clamped to 1 line,
      response clamped to 3 lines, `pointer-events: none` (text behind it stays
      selectable);
- [ ] the active tick tracks the reading position while scrolling (topmost
      turn past the 70% band), and an over-long tick ladder scrolls inside the
      rail with fades at each scrollable end;
- [ ] ticks are keyboard-reachable buttons — focus opens the preview, the
      active tick carries `aria-current`; `prefers-reduced-motion` drops all
      rail motion;
- [ ] ⌘K opens the quick switcher (命令 then 会话, case-insensitive filter,
      ↑↓/Enter/Escape); ⌘N starts a new task only while no composer field has
      focus; demo mode lists commands only;
- [ ] user turns offer copy and `编辑并发送` — edit prefills the composer and
      resends append-only (the session log is never truncated);
- [ ] composer drafts persist per conversation across reload and clear on send;
- [ ] Enter during an IME composition never sends — only a plain Enter does;
- [ ] the empty welcome offers `继续上次对话` into the space's newest thread
      and hides it when the space has no history (or in demo);
- [ ] the file viewer header offers `复制路径` (absolute path, `已复制` flip)
      and `在 Finder 中显示` (project-guarded reveal);
- [ ] `turnComplete` and `error` ping the native window for attention — a
      user stop never does;
- [ ] right-click is suppressed app-wide — the WebView default menu
      (Back / Reload / Inspect Element) never appears.

---

## 5e. Onboarding checks

> 2026-09-23: the first-run Auto Onboarding (Dia-style startup sequence,
> `COMPONENTS.md` §15). Hero type is allowed on this transient surface only.

- [ ] plays automatically on first launch as a full-viewport dialog;
- [ ] stages auto-advance on a fixed cadence with a per-stage timing bar;
- [ ] dots jump between stages; the final stage stops on `开始使用`;
- [ ] `跳过`, `Escape` and `开始使用` all dismiss and stamp `onboarding-done`;
- [ ] a done onboarding never returns on later launches;
- [ ] demo routes and plain-browser fixtures never show it;
- [ ] `prefers-reduced-motion` drops the movement;
- [ ] behind the overlay the shell is unchanged (single sidebar tree, welcome
      stage, composer).

---

## 5f. Work / Code mode & project-selection checks

> Added with the Work/Code dual-mode upgrade (`DESIGN.md` §11). The switcher is
> the Codex-style brand dropdown (2026-09-28: it replaced the
> `K [工作 | 代码]` segmented control; 2026-09-29: the trigger label names the
> current space — `Kodo Code ˅` / `Kodo Work ˅`); the two modes are separate
> agent spaces (session lists never mix). A project is optional in both
> modes (2026-09-29: the Code-mode project gate is gone) — without one the
> run is constrained Q&A: no tools, no verify. Also 2026-09-29: both
> projectless runs and knowledge questions take the conversational channel —
> no skill criteria, no acceptance bounce; the first prose answer ends the
> run as plain `ready` + `not_run`.

- [ ] the brand dropdown sits in the sidebar brand block (not in the
      composer): the brand row is the trigger and its label reads
      `Kodo Code` / `Kodo Work` — the top-left always names the current
      space, and the label flips the moment the mode switches; the menu
      lists 工作 / 代码 with the current space check-marked, and the choice
      persists across reloads (shared settings key `agent-mode`, same one
      the CLI reads);
- [ ] switching swaps the whole space: session list, 插件页 · 技能 tab
      (builtin and 自建 lists) and plus-panel skills, and new-task welcome
      copy all follow the mode — work and code sessions never mix in one
      list, and a user skill only appears in the space its task types put it
      in;
- [ ] the welcome stage names the space above the title: the mode logo plus
      the `Kodo Code` / `Kodo Work` wordmark (`welcome-identity`) follows the
      switcher immediately, so the two spaces are told apart at a glance;
- [ ] project rows stay shared across both spaces even with zero sessions of
      the current space; the 未选择项目 pseudo group shows only the current
      space's project-less sessions;
- [ ] switching away while a conversation of the other space is open falls back
      to the welcome stage (nothing dangling); switching back finds the
      conversation in its own list and it opens again;
- [ ] code mode without a project: send stays enabled — the run is
      constrained Q&A (the send payload carries `project=""`, and the reply
      trace shows no Command / FileChange steps; no
      选择项目后即可发送（代码模式） hint exists anywhere);
- [ ] work mode without a project: send works, the welcome copy is the work
      voice (写文档、做总结、提问都可以…), and the run carries no tools;
- [ ] a projectless conversation (either mode) groups under 未选择项目 at
      the foot of the tree and renders as an ordinary conversation row;
- [ ] work mode with a project: file tools are available and the trace has no
      Verify phase;
- [ ] `新建任务` preselects the remembered project only when it is still
      registered in the workspace — otherwise the task starts 未选择项目;
- [ ] `清除项目选择` deselects the project only — it never removes it and
      never forgets the remembered project;
- [ ] `从列表移除` needs the second click on `确认移除`; clicking away resets;
- [ ] removing the project that owns the active conversation returns to the
      new-task/welcome state (no dangling conversation);
- [ ] the archive page stays cross-mode (storage management) and a restored
      conversation returns to its own space;
- [ ] the approval bar still intercepts writes/commands per permission mode in
      both modes.

---

## 5g. Skills checks

> Added with skill preview + user-skill CRUD (`COMPONENTS.md` §13b/§13c,
> `DESIGN.md` §11 “Skill selection”). Covered by `tests/visual/skills.spec.ts`.
> Since 2026-09-30 these checks apply to the 插件页 · 技能 tab (the page head
> and tabs belong to PluginsPage — `COMPONENTS.md` §13d); every check below is
> unchanged.

- [ ] every builtin row is a real button: click (and keyboard focus + Enter)
      opens the read-only `skill-detail` modal with the parsed sections
      (工作流 rows included) and the 内置 source chip; the modal closes via
      the X, the backdrop or Esc; builtins never show 编辑/删除;
- [ ] `新建技能` opens the editor with a valid per-space template prefilled;
      an invalid name or empty content shows the zh-CN `role="alert"` error
      and **no** `save_skill` call reaches the core; a valid save closes the
      modal and the row lands in 自建技能 (`save_skill` observed once);
- [ ] a backend save rejection (parse failure / reserved name) surfaces
      inline in the editor without closing it;
- [ ] editing a user skill round-trips: reopen shows the saved content in
      技能内容; the row's 删除 needs the second click on `确认删除？` and then
      the row disappears (`delete_skill` observed);
- [ ] a space with no user skills shows exactly 当前空间还没有自建技能。;
- [ ] with a selected project the 项目技能 card sits between 可用技能 and
      自建技能 (its ＋ opens the editor scoped to the project store — a save
      lands in 项目技能 via `data-project-skill-id`, never in 自建技能);
      project rows edit/delete the project store only; with no project
      selected the card is absent entirely;
- [ ] a space with no project skills shows exactly 当前项目还没有项目技能。;
- [ ] a user skill appears in the composer's + 技能 panel and the `/`
      suggestions for **its own space only**, serializes unchanged as
      `【技能：<name>】` before the draft, and the payload format matches the
      builtin-chip contract byte for byte;
- [ ] code space still shows exactly 7 `.skill-row` (store rows carry
      `.user-skill-row` — the class split is the invariant).

---

## 5h. MCP checks

> Added with the MCP sidebar destination (`COMPONENTS.md` §14); since
> 2026-09-30 MCP is the 插件页 · MCP tab (`/plugins?type=mcp`, alias
> `/mcp`) — `COMPONENTS.md` §14/§13d. Covered by the MCP cases in
> `tests/visual/behaviour.spec.ts` and the `plugins-mcp` /
> `plugins-mcp-tools` / `plugins-mcp-editor` snapshots in
> `tests/visual/pages.spec.ts`.

- [ ] the page head is the 插件 pattern (Puzzle mark, `插件`, per-tab
      subtitle) with the MCP tab selected; with the MCP tab active the
      subtitle is the one line naming the `mcp__服务器__工具名` wire
      convention and `credentials.log`;
- [ ] a server row shows the expand chevron, name, transport chip, command or
      URL, enable toggle and edit/delete — no dead controls anywhere;
- [ ] the chevron expands that server's tool list (tool name chips plus
      descriptions read through `mcp_list_tools`) and collapses again; a
      loading or failure state is one zh-CN hint line, never fatal;
- [ ] the editor shows 显示名称, 传输方式 and only the active transport's
      fields; switching transport swaps the fields and clears the other half
      on save; there is no preset select;
- [ ] the 连接器目录 section shows one card per `MCP_TEMPLATES` entry (20)
      with icon, zh one-line description, transport chip and ＋添加; a card
      click opens the prefilled add editor with secret rows blank and the
      `来自连接器目录：<name>` hint; `添加 MCP 服务器` opens a blank custom
      editor; editing an existing server never prefills from a template;
- [ ] a catalog card is one instance per connector — a template that is
      already configured (by name or launch identity: the stdio package or
      the http URL) renders `已添加` with a Check, is disabled and refuses a
      second instance; `添加 MCP 服务器` stays unlimited for deliberate
      variants (a second filesystem root), and deleting the instance frees
      the card again;
- [ ] env / header values round-trip masked — blank or masked keeps the stored
      secret, a typed value replaces it, deleting the row removes it;
- [ ] an empty list shows exactly the empty hint under the `已配置服务器` head
      (which carries `添加 MCP 服务器`), and a created server starts enabled;
- [ ] Settings → 工具与权限 shows the `权限模式` row only — no MCP block.

---

## 5i. Plugins page & agent checks

> Added 2026-09-30 with the 插件 page (`COMPONENTS.md` §13d): 技能 / MCP /
> 智能体 are one page's tabs behind a single sidebar `插件` row, and the
> 智能体 persona reaches the model through the Composer selector. Covered by
> `tests/visual/skills.spec.ts`, the plugins cases in
> `tests/visual/behaviour.spec.ts` and `tests/visual/agents.spec.ts`.

- [ ] the 插件 page head carries the Puzzle mark, `插件` and a per-tab
      subtitle; the head action is context-sensitive — `新建技能` on the 技能
      tab, `新建智能体` on the 智能体 tab, no head action on the MCP tab;
- [ ] the tabs row (技能 / MCP / 智能体) marks the active tab and rewrites
      `?type=` in the URL; reloading with `?type=agents` (or `?type=mcp`)
      restores that tab, and the plain `/plugins` (and alias `/skills`)
      lands on 技能 with **no** query in the URL;
- [ ] the 智能体 tab shows one compact-row card: each row is name +
      description with edit and two-step delete (`确认删除？`); rows use the
      `.agent-row` family and never `.skill-row` / `.user-skill-row`; an
      empty tab shows exactly the empty hint (`agents-empty`);
- [ ] the agent editor (view / edit / new) validates in order 名称必填 →
      名称字符（字母、数字、-、_，最多 64 字符）→ 描述必填 → 说明词必填 with
      zh-CN `role="alert"` errors and no save call until valid; a save
      round-trips (reopen shows the saved 描述/说明词), backend rejections
      surface inline without closing the modal, and the editor has **no**
      enable toggle and no model/permission/tool fields;
- [ ] the Composer left cluster is `[+] [权限▼] [智能体▼] … [模型▼] [发送]`:
      the agent trigger sits directly after the permission trigger, defaults
      to `无智能体`, and its menu lists `无智能体` plus one entry per agent
      (description as the hint) with the current choice check-marked;
- [ ] the choice is session-scoped — it survives switching to another
      conversation and back, resets for a brand-new task after the previous
      draft's choice was carried into its created session, and applies to the
      very next send (the send payload carries the agent name; a `null`
      agent reaches the backend as no persona block).

---

## 5j. Workbench & app checks

> Added 2026-10-13 with the 工作台 (`LAYOUT.md` §2): it is the app collection and
> the only home for its apps. Covered by `tests/visual/workbench.spec.ts`
> (behaviour) and the `workbench*` entries in `tests/visual/pages.spec.ts`
> (snapshots at the reference viewport).

- [ ] `/workbench` is reached from the sidebar's `工作台` row; the head is
      `工作台` + its subtitle, with `应用管理` as the only head action;
- [ ] the collection is **rows** (`.wb-app-row`, 38px) — an icon, the app name,
      a one-line description and a compact `打开` button. It is not a tile grid:
      the MCP connector catalog stays the only documented tiling exception
      (§6 above);
- [ ] sections are `收藏` (only when non-empty) then `全部应用`; a disabled app
      keeps its data but leaves the collection, and its `?app=` URL lands back
      on the collection with an `已停用` notice instead of a dead screen;
- [ ] search filters by app name/description; `应用管理` carries enable,
      favorite, ↑/↓ order, the AI and 调度 integration switches and the 今日概览
      switch, and every one of them persists (a reload shows the stored value);
- [ ] opening an app keeps the sidebar at one `工作台` row (no app row appears),
      the app's own head carries `返回工作台`, and 今日概览's two headings open
      待办 / 日历 respectively;
- [ ] 今日概览 lists today's **open** 待办 (due today) and today's 日历 events,
      each with a `· n` count, and disappears entirely when its switch is off;
- [ ] 待办: three fixed columns 待处理 / 进行中 / 已完成 with a per-column count
      and `+`; board and list are two views of one dataset, so a filter, a
      status change or 完成/恢复 is visible in both, in the column counts, in
      今日概览 and in the 日历 待办截止 overlay;
- [ ] 待办 card rows show title, 截止/无截止日期, priority and status; a
      sub-item renders under its parent (never as a column card of its own);
      drag starts from the handle only, the target column shows the drop state,
      and Escape or a drop outside a column leaves the status unchanged;
- [ ] 日历: 周 / 月 views over real dates (no fixture dates outside the demo
      route), events carry their own IANA zone, and the editor validates
      标题必填 → 结束时间必须晚于开始时间; delete is two-step (`确认删除？`);
- [ ] a 周 block renders whole lines only — never a line squeezed to a sliver of
      its glyphs. It carries the title, the start time once it is an hour tall,
      and the event's zone once it is 1.5 hours tall (below that the zone stays
      in the block's `title` tooltip). Editing an event never re-zones it to the
      calendar's own zone;
- [ ] 成果中心: rows of title + kind + category + file status, a text artifact
      previews as text and a file reference shows its path with `打开文件` /
      `在 Finder 中显示`; a moved or replaced file reads `文件缺失` / `文件已更改`
      with the record kept, and deleting the record never deletes the file;
- [ ] light and dark themes both render every workbench surface (no hard-coded
      colour), and the board falls back to a single column below 820px.

---

## 6. Density checks

Reject the implementation if:
- content is enlarged merely to fill space;
- there are large empty hero sections;
- every block is a rounded card (the MCP connector catalog on the 插件页 ·
  MCP tab is the documented exception — `COMPONENTS.md` §14; any *other*
  surface turning rows into tile cards is still a rejection);
- typography is consumer-chat sized;
- fewer than expected project/conversation rows fit vertically;
- unnecessary labels consume entire rows.

---

## 7. Visual regression workflow

For each page:

1. load deterministic fixture;
2. set viewport to `1586x992`;
3. wait for fonts/data;
4. capture screenshot;
5. check it against the design docs and the checks in this file;
6. correct large geometry first;
7. repeat.

### Snapshots are a net, not a spec

The Playwright baselines are **re-generated** whenever a design change is
deliberate, because such a change alters the screens the suite covers.

The consequence, stated plainly: the snapshot suite can only catch
*unintended* change. It can never catch *design drift*, because whatever the
app renders today is what it was told to expect. Only the design docs
(`DESIGN.md` / `LAYOUT.md` / `COMPONENTS.md`) and the checks above hold the
design.

Recommended order of correction:

```text
layout
→ widths/heights
→ spacing
→ typography
→ borders/backgrounds
→ icons
→ micro details
```

---

## 8. Completion rule

Do not say “implemented” until:
- functional tests pass;
- visual screenshot has been inspected;
- no known high-impact difference remains;
- no unapproved UI concept has been introduced.
