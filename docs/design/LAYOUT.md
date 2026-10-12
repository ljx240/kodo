# Kodo Layout Specification

## 1. App shell

Primary structure:

```text
┌──────────────┬──────────────────────────────────┬──────────────┐
│              │                                  │              │
│   Sidebar    │          Conversation            │  Inspector   │
│              │                                  │  optional    │
│              │                                  │              │
└──────────────┴──────────────────────────────────┴──────────────┘
```

At the reference viewport:

```text
Sidebar              260px
Main                  minmax(400px, 1fr)
Inspector expanded    330px
Inspector rail         40px
Top bar                44px
```

Inspector:
- default desktop width: 330px;
- resizable from 320px to the available workspace width (pointer drag, or
  `←`/`→` ±16 / Shift ±64 on keyboard focus) — windows ≥1200px only;
- while the `文件` or `浏览` panel is open the column widens to its own 640px
  width (windows ≥1200px) so the tree and the file sit side by side (or the
  embedded page breathes); closing it restores the previous width over the
  same 120ms transition — the base and wide widths are two independent
  persisted keys (defaults 330 / 640);
- **focus mode**: once the conversation reaches its 400px minimum, further
  leftward dragging hides it while keeping it mounted; the Inspector fills
  the remaining workspace (see §5 Inspector → Focus mode);
- collapse must always be supported.

When collapsed:
- keep a narrow 40px tool rail;
- main conversation immediately consumes freed width.

---

## 2. Sidebar

The sidebar is one navigation surface only.

Structure (labels as rendered, zh-CN from `apps/desktop/src/i18n.ts`):

```text
Kodo Code ˅                   ← brand dropdown = Work/Code switcher; the label
                                 names the current space (Kodo Work in Work)

新建任务          ← action, never active
插件

项目                          +
├─ project A
│  ├─ conversation A1
│  ├─ conversation A2
│  └─ conversation A3
├─ project B
└─ project C
未选择项目                    ← pseudo group, only when such sessions exist
├─ conversation N1
└─ conversation N2

设置
```

The tree shows only the current agent space's sessions (Work vs Code never
mix); project rows themselves are shared across both spaces and stay even with
zero current-space sessions. The 未选择项目 pseudo group gathers the current
space's project-less sessions only.

> Structure update: the old `Conversations`/`Archive` nav rows are gone. One
> tree carries projects and their conversations (a second conversation list was
> banned by the core layout rule anyway); `新建任务` is an action, not a
> destination; `归档` is a storage-management screen reached via 设置 → 归档与存储
> → 查看归档.
>
> Structure update 2026-09-30: 技能 and MCP no longer take their own rows —
> they are tabs on the single `插件` destination alongside the new 智能体 tab
> (COMPONENTS.md §13d/§14). The sidebar above the tree is therefore exactly
> `新建任务` + `插件`.

There is **no account row**. Kodo has no login, so the sidebar does not name a
user and does not offer a sign-out.

### Project behavior

Click project:
- expands nested conversations.

Click expanded project:
- collapses it.

Project row:
- 30px target height;
- trailing `⋯` opens the row menu: rename, reveal in Finder, remove from list
  (two-step: first click arms the item and flips its label to `确认移除`, the
  second click commits; clicking away resets — no modal).

Projects are **registered by hand**, never discovered by scanning a directory.
The `+` beside the `Projects` heading opens a two-item menu:

```text
添加已有文件夹…    registers an existing directory
新建项目…          creates a directory on disk, then registers it
```

Clicking a project row itself always means "expand/collapse". Registration and
creation live behind `+`; rename, reveal and remove live behind `⋯`.

Removing a project **only removes the entry**. It never touches the directory.
If the active conversation belonged to the removed project, the app returns to
the new-task/welcome state instead of dangling. Renaming changes the **display
name only**; the directory keeps its own name.

`未选择项目` is a pseudo group at the foot of the tree for sessions created
without a project (Work-mode pure conversation): a plain label row plus the
same conversation rows — never a project row (no chevron, no `+`, no `⋯`). It
appears only when such sessions exist.

`新建任务` preselects the remembered project (`kodo:last-project`) when it is
still registered in the workspace; otherwise the task starts without one.

Reordering uses native HTML5 drag-and-drop. The drop writes the whole new order,
not "moved item N to index M" — see `docs/设计文档.md`.

Conversation row:
- 28px target height;
- indented below project: thread line ≈18px from the tree's left edge, row
  padding-left 8px — tight enough that the rows hug the left like project rows;
- selected conversation uses subtle blue background;
- show compact timestamp/date on right where space allows.

Do not introduce:
- project detail screen as an intermediate step;
- second conversation column;
- permanent project-switcher panel beside sidebar.

### Long lists

Project list is independently scrollable.

A group's conversation list folds past five rows (project groups and the
未选择项目 group alike): the first five stay, and a `加载全部会话` row under
them reveals the rest — one click, then the control retires. Lists of five or
fewer never grow the control, and the open conversation is never folded away:
if the active row sits past the fold, the list shows in full.

Settings stays pinned at bottom, below the scrollable tree, as a single
`nav-item` row.

---

## 3. Conversation page

Structure:

```text
ConversationPage
├─ TopBar
├─ ScrollStage            position context; wraps the scroller + rail
│  ├─ ScrollArea
│  │  ├─ UserMessage
│  │  ├─ AssistantReply
│  │  │  ├─ RunningState
│  │  │  ├─ AgentTrace    only while no final reply exists
│  │  │  ├─ FinalResponse
│  │  │  └─ ReplyActions  bottom row: copy / regenerate / trace icons,
│  │  │                   always visible (2026-09-30)
│  │  └─ ChangedFilesSummary
│  └─ TurnRail            overlay in the scroll area's right gutter
└─ Composer
```

> 2026-09-30: `TurnRail` (COMPONENTS.md §16) is an **overlay**, never a
> column. Its 28px frame floats over the transcript's right inset with a
> fixed 10px pitch — one tick per user turn — and consumes no layout width,
> so the conversation remains the largest visual region. It renders only
> with ≥2 turns and a scrollport ≥900px; hover/focus raises a prompt (1
> line) + response (3 lines) preview; a click scrolls that turn into view
> and releases stick-to-bottom; the active tick tracks the reading position;
> an over-long ladder scrolls inside the frame with fades at each end.

> 2026-09-30: the file viewer is no longer a modal. It is a panel in the
> right sidebar's Inspector panel layer (COMPONENTS.md §10) — reviewing a
> change never covers the conversation, and the conversation column never
> shifts. It is still not a route.

Top bar:
- project / conversation breadcrumb or conversation title — the single title
  surface; the current title edits in place behind the pencil;
- branch when relevant;
- conversation actions — pencil (inline rename) and `⋯` (重命名 / 归档);
- project-files icon — opens the `文件` panel in the right sidebar in browse
  mode (the tree, nothing preselected); in demo / without a project it falls
  back to the Inspector `改动` section;
- Inspector toggle.

> 2026-09-23: the model is no longer repeated in the top bar. It lives on the
> composer only (where it is chosen), so the top bar keeps just location
> context — project/title and branch.

> 2026-09-30: the 64px conversation title row inside the scroll area is gone —
> it duplicated the breadcrumb and made a page header the docs never asked
> for. The title lives only in the top bar (inline rename via the pencil);
> 归档 moved into the top-bar `⋯`; 查看响应轨迹 moved from that menu into
> each reply's bottom action row.

Avoid a large page header.

### Conversation width

The main message column should not span the entire center pane when very wide.

Preferred readable max width:

```text
920–1050px
```

Center it inside the available main pane.

When Inspector is open, use nearly all available width.

### Message layout

User message:
- simple subtle neutral background;
- no large chat bubble;
- max width roughly 85%;
- no speaker header (2026-09-30) — right alignment plus the neutral
  background already say "you"; the turn is content only.

Assistant:
- no enclosing giant card;
- no avatar/name/time header (2026-09-30) — left alignment says "Kodo";
- a quiet action row sits at the very bottom of the reply — 复制回复,
  重新生成, 查看响应轨迹 — always visible, never hover-revealed
  (2026-09-30); the buttons are icon-only, labels on tooltips/aria
  (2026-09-30);
- trace and answer flow vertically.

### Composer

Pinned to bottom of main content.

Height:
- collapsed/minimum: 84–92px (draft line + bottom control line);
- the draft line auto-grows to about 160px before internal scrolling.

Width:
- the composer box spans the same centered column as the reply text — it
  mirrors the page container geometry (900px box with 32px side padding → an
  836px content column at the reference viewport; 16px side padding below
  1199px), so input and conversation share one column edge (2026-09-30).

Contains:
- attachment cards above the draft (when any context is pinned);
- prompt input on top, full width;
- a bottom control line: attachment/context button, the permission trigger and
  the agent selector on the left; selected model and send/stop action on the
  right (2026-09-30: the 智能体 selector sits directly after the permission
  trigger — COMPONENTS.md §13).

Send gating (see `DESIGN.md` §11): the Work/Code switcher lives in the sidebar
brand block, not here. Sending never requires a project — without one the run
is constrained Q&A (no tools, no verify). 2026-09-29: the code-mode project
gate and its inline hint `选择项目后即可发送（代码模式）` are gone.

> 2026-09-23: controls sink to the bottom of the input box (one row of four
> controls beside the draft was too tight for the permission label and made
> the draft feel narrow). This raises the collapsed height from 54–60px.

Attachment cards (2026-09-23, Codex-style):
- image files render a thumbnail card; a single image renders one large
  thumbnail above the input;
- PDF / Office / text / other files render a gray card with a centered type
  glyph, a colored type badge (PDF, DOC, XLS, PPT, TXT, CODE…) and the file
  name;
- each card carries a remove action and shows the full path on hover;
- cards hold paths only — file bodies never enter the page;
- sent messages reuse the same card inside the user bubble (2026-09-29) —
  static, no remove action, file name instead of a truncated path
  (COMPONENTS.md §13).

Keep toolbars minimal.

---

## 4. Agent trace inside reply

The trace belongs to each individual assistant reply.

Structure:

```text
Assistant reply
├─ Status: Working / Completed
├─ Trace
│  ├─ Thinking
│  ├─ Interim text (quiet note rows, in order)
│  ├─ Search
│  ├─ Read
│  ├─ Run command
│  ├─ Tool result
│  └─ Model call
└─ Final answer
```

Every assistant reply can open its own full Response Trace page/view.

Inline trace is compact:
- one line per step by default;
- consecutive same-type steps fold under one group row (`调用模型 · 3 次`);
  expanding shows each step at the second level, then each step's detail;
- output appears only for active/error/explicitly expanded items;
- no status icons on rows — state is words (`进行中`, `原因：…`);
- there is no streaming draft block: interim prose sits in the trace where it
  arrived, and the final answer appears only once it is final.

> 2026-09-24: the inline trace (and its step/token footer) exists only while
> there is no final reply. Once the answer lands the conversation shows the
> reply alone — only the final reply body. The process is read on the Response
> Trace page (§6); a finished answer is never parked under its own history.

> 2026-09-23: a conversational answer (`plain`) drops the report chrome and
> keeps only the final sentence — no 结论/本次完成 sections and no 下一步 row.
> Task answers keep the full conclusion-first card.

> 2026-09-29: `plain` now also comes from the conversational channel — a
> projectless run or a knowledge question (no skill chip) answers as plain
> text with `ready` + `not_run` and no verification footer. They never grow
> 结论 chrome or a verification card; task answers are unchanged.

> 2026-09-23: first launch layers a full-viewport `AutoOnboarding` dialog over
> the shell (Dia-style auto-playing stages, COMPONENTS.md §15). It is one-shot
> (`onboarding-done`) and never part of the steady-state layout.

---

## 5. Inspector

Inspector is contextual to the current response or conversation.

Expanded layout — two layers:

```text
Inspector
├─ Header / close (X)
├─ Panel layer   Inspector | 终端 | 文件 | 浏览  (chips; route-dependent)
├─ Rail          概览 | 改动 | 工具 | 模型      (only inside Inspector panel)
├─ Summary
├─ Changed files
├─ Tools used
├─ LLM calls
└─ Current project
```

The panel layer picks which whole panel occupies the column; the rail picks
which section shows inside the `Inspector` panel. A panel and a rail section
never share a name — when `文件` became a panel chip, the rail section
formerly called 文件 became **改动** (testid `rail-*`, icon and body title
修改文件 are unchanged). The rail is hidden while `终端`, `文件` or `浏览` is
open — they have no sections beside themselves.

The `文件` panel is the FileViewer, de-modalized (COMPONENTS.md §10): the
body fills the panel with no outer padding, the column widens to 640px on
windows ≥1200px (width transition 120ms ease), and at ≤1199px it keeps the
overlay clamp like every other panel. X and Escape **demote** it to the
`概览` section (the panel stays open); from `概览` or `终端`, X closes the
panel entirely. In demo / without a project, file entries fall back to the
`改动` section (no file bodies exist there).

The `浏览` panel (COMPONENTS.md §9b) is the embedded browser: same widen,
same full-bleed body and same demotion contract; unlike `文件` it is not
demo-gated (it needs no project or file data).

Do not show all detailed data at once.

Each section card carries a chevron and **collapses**: clicking its header hides
the body, clicking again restores it. Cards with no body stay inert. The chevron
glyph is drawn the same in both states — only `aria-expanded` reports which
state the card is in.

### Collapsed state

Collapsed width: 40px.

Show vertical icons:
- Info / Summary
- Files
- Tools
- Terminal
- LLM

Click icon:
- opens Inspector directly to selected section.

Close:
- returns to rail, not total disappearance.

The user should always be able to open the panel with one click.

### Focus mode (adaptive workspace width)

The conversation has a 400px minimum while visible. Further leftward dragging
hides it immediately (`.app--focus-mode`), without unmounting its run or draft.
The Inspector fills all remaining workspace width, including in 文件/浏览;
its limit follows the window, sidebar, rail and border rather than a fixed cap.

Rightward dragging first restores the conversation at its minimum, then keeps
resizing with the pointer. This applies within one gesture and to a new gesture.
Keyboard arrows use the same rule. Double-click toggles focus mode and restores
the saved base/wide width on exit.

`inspector-focus-mode` persists the hidden state. Base and wide widths remain
independent; the old `inspector-focus-width` is ignored. Below 1200px, the
conversation is visible and the Inspector uses the existing overlay.

---

## 6. Response Trace

This is the detailed inspection page for one assistant response.

Structure:

```text
Header
├─ response metadata
└─ final response summary

Tabs
├─ Timeline    the step table (primary)
├─ Logs        the same ten steps as raw log lines
└─ Artifacts   the changed files for this response

The three tabs are three renderings of one run, and each is reachable. A tab
that changes nothing when clicked is a defect.

Timeline table
├─ Time
├─ Duration
├─ Type
└─ Details

Inspector
├─ Changed files
├─ Terminal output
├─ LLM calls
└─ Metadata
```

The Timeline is the primary content.

Each row is compact, approximately 38–42px.

Returning: the page bar's back control goes to the conversation, and clicking a
conversation in the sidebar does too — from this page (or any other) the
conversation is always one click away.

> 2026-09-24: added after “进入响应轨迹之后如何返回” — the page used to be a
> dead end (static crumbs, no top bar on the route).

> 2026-09-30: opening this page mid-run never interrupts the Q&A. While a run
> is live the conversation stays mounted underneath (hidden), so its event
> stream, queue and approvals keep flowing; the back control returns to the
> same live state — the scroll position restores to where the reader was.

Click a row:
- expand detail inline or reveal corresponding Inspector section.

---

## 7. Archive

Archive is a dense list/table, not cards.

Main columns:

```text
Conversation
Summary
Archived at
Model
Files changed
Status
```

Filters live in one compact toolbar.

Selecting an item opens details in Inspector.

Restore is an Inspector action.

---

## 8. Settings

Settings is a **left column of categories plus a right detail pane**, inside main
content:

```text
Settings
├─ page head (title, subtitle, Search settings…)
└─ .settings-layout
   ├─ .settings-nav      category list, one row per category
   └─ .scroll
      └─ .settings-detail   the selected category only
```

Categories, in the order the left column lists them:

- General
- Models
- AI Provider
- Tools & Permissions
- Projects
- Archive & Storage
- Appearance

Exactly one category is shown at a time. Every category must open onto a
non-empty pane — a category that renders nothing is a defect, not a placeholder.

Tools & Permissions holds the `权限模式` row only. MCP servers are not a
setting row — they live on the 插件页 MCP tab (COMPONENTS.md §14/§13d), the
same page as 技能 and 智能体.

The page keeps its route (`/settings`, `/ui-demo/settings`); only the inside of
the page is split. Settings is not a dialog and not a nested route.

Inspector may show Help / Information.

---

## 9. Responsive behavior

### >= 1500px
- Sidebar 260
- Main flex
- Inspector 330 default, resizable from 320px to the available width (640 by default for 文件)
- **Focus mode available**: drag past the conversation's 400px minimum → hide conversation, Inspector fills workspace

### 1200–1499px
- Sidebar 240
- Inspector 330 default, resizable from 320px to the available width (640 by default for 文件)
- slightly tighter content spacing
- **Focus mode available** — Inspector is clamped to whatever room is left so it never overflows the window

### 900–1199px
- Sidebar 220
- Inspector defaults collapsed
- conversation owns remaining width
- the 文件 panel overlays the conversation (same clamp as every panel);
  it never widens the column here, and there is no width handle — the
  overlay width stays token-driven (the narrow token opens at 300px)
- **Focus mode not available** (no resizer; column cannot collapse)

### < 900px
Not a primary V1 target.

### Width handle & motion

- the handle is a focusable `role="separator"` (`aria-orientation="vertical"`,
  `aria-valuenow` tracking the width, minimum 320px and an adaptive maximum)
  straddling the conversation ↔ inspector boundary; it renders only while the
  panel is open on ≥1200px windows;
- it edits two independent persisted widths — base `inspector-width` (default
  330) and wide `inspector-wide-width` (default 640 for 文件/浏览); focus mode
  retains these values and fills the available workspace without a width key;
- motion is enter-only at `120ms ease`: the panel slides in on open (6px),
  the content re-enters on every chip/section switch (8px), the column width
  tweens across the same 120ms; while dragging, the width tween is suppressed
  (`app--resizing` also sets the col-resize cursor and disables selection);
- double-click on the separator toggles focus mode (see §5 Inspector →
  Focus mode);
- the global `prefers-reduced-motion` block (app.css) flattens all of it.
Desktop application quality takes priority.
