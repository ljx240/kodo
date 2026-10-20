# Kodo Core UI Components

Only recurring, behaviorally meaningful components belong here.

---

## 1. `ProjectTree`

Purpose:
- display local projects;
- expand/collapse project conversations;
- switch conversation.

Props conceptually:

```ts
type ProjectTreeProps = {
  projects: ProjectNavItem[]
  activeProjectId?: string
  activeConversationId?: string
}
```

Rules:
- only one left navigation tree;
- nested conversations are part of the project row hierarchy;
- project and conversation rows must remain compact;
- avoid project cards;
- sessions created without a project group under a plain `未选择项目` label row
  at the foot of the tree (pseudo group — never a project row);
- the tree lists only the **current agent space's** sessions (Work vs Code —
  `DESIGN.md` §11): each conversation's immutable `mode` tag filters it in or
  out, and the 未选择项目 pseudo group gathers the current space's project-less
  sessions only (no such sessions → no group). Project rows themselves are
  shared across spaces and stay visible with zero current-space sessions;
- a group with more than five conversations shows the first five plus a
  `加载全部会话` row (`tree-load-all`, `T.action.loadAll`) inside the same
  thread-line indent; one click reveals the rest and the control retires.
  Five or fewer never fold, and the active conversation is never hidden past
  the fold — if it sits in the hidden tail, the list shows in full.

---

## 2. `ProjectRow`

Visual:

```text
⌄ 📁 realtime-lakehouse
```

States:
- collapsed;
- expanded;
- hover;
- selected project context;
- being dragged.

Height:
- about 30px.

The row is a container, not a button: the main area expands/collapses, and the
trailing controls never expand anything. The trailing cluster is `+` then `⋯`,
both hover-revealed: `+` starts a new task under this project in one click
(same as the menu's `新建任务`), and `⋯` opens the row menu.

Row menu (`ProjectMenu`):

```text
新建任务
Rename…
Reveal in Finder
Remove from list        ← first click flips to 确认移除, second commits
```

"Remove from list" is destructive-looking but is not destructive: the entry goes,
the directory stays. The label says "from list" for that reason. Removal is
two-step inside the menu (arm → `确认移除`), no modal; clicking away resets the
armed state.

Drag: the browser's own drag-and-drop, no library. Dropping writes the entire new
order to the core rather than a relative move.

---

## 2b. `BrandSwitcher`

The sidebar brand row is the Work/Code switcher (`DESIGN.md` §11) — a
Codex-style dropdown (2026-09-28: the segmented control is gone; the brand
name with a caret is the trigger, the way Codex switches its space):

```text
[K] Kodo Code ˅          ← the label names the current space
```

- the whole brand row is the menu trigger (`data-testid="mode-toggle"`:
  brand mark + current-space wordmark (`Kodo Code` / `Kodo Work`) + caret;
  the caret rotates while the menu is open);
- the menu holds the two spaces (`mode-work` / `mode-code`); the current one
  is check-marked and tinted (`--kodo-selected` + `--kodo-accent`), the other
  keeps a spacer glyph so the labels stay on one column;
- switching swaps the whole agent space below it — session list, the 插件页
  skill lists, new-task behavior — never mixing the two lists (the tree
  filters per the session's immutable mode tag);
- an open conversation of the other space falls back to the welcome stage and
  stays in its own space;
- the choice persists in the shared settings key `agent-mode` (the CLI stamps
  new sessions with the same key).

---

## 3. `ConversationRow`

Visual:

```text
   │ ◉ 修复 k2k-rust 未知表路由                 10:24
```

Rules:
- one line title when possible;
- compact metadata;
- the thread-line indent stays tight — conversation rows hug the left like the
  project rows above them (LAYOUT.md §2), never parked deep under the project
  name;
- selected: subtle blue tint + accent indicator;
- archived conversations do not appear here unless explicitly restored.

---

## 3a. `SidebarFooter`

A single `nav-item` pinned below the scrollable tree:

```text
⚙ 设置
```

There is **no `AccountRow`**. Kodo does not log in, so the sidebar names no user,
shows no avatar and offers no sign-out. `Settings` is never mixed into the
project tree, and the tree is the only conversation list.

> Copy update: the old line said `Conversations`/`Archive` stay at the top.
> They no longer exist as nav rows — above the tree sit the `新建任务` action and
> the `插件` destination (see `LAYOUT.md` §2), and `归档` opens from Settings →
> 归档与存储. Reason recorded there.
> Copy update 2026-09-30: 技能 and MCP are tabs on 插件 (§13d), so the rows
> above the tree are exactly `新建任务` + `插件`.

---

## 4. `AssistantReply`

Structure:

```text
Working/completed state
AgentTrace            only while no final reply exists
FinalResponse
ChangedFilesSummary
Reply actions         quiet bottom row: copy / regenerate / trace, always visible
```

Rules:
- no outer card around the entire reply;
- no avatar, name or timestamp header (2026-09-30) — the left alignment and
  the user's neutral bubble carry the speaker; the per-answer model chip is
  gone with the header (the composer chip and the trace/Inspector carry the
  model);
- 复制回复 / 重新生成 / 查看响应轨迹 sit in one quiet row at the reply's very
  bottom, always visible — never hover-revealed (2026-09-30). The buttons are
  icon-only (`icon-btn icon-btn--sm`; the label lives on `aria-label` +
  `title`), and whichever of the three the turn allows renders, so an
  incomplete turn shows only 查看响应轨迹;
- trace belongs visually to the reply;
- final response must be visually stronger than trace metadata.

> 2026-09-23: there is no streaming draft above the reply. Provider text is
> never hoisted to the top of the turn — the answer surface appears only once
> the final reply lands. Interim prose between tool steps rides inside the
> trace as quiet note rows in arrival order (`TraceItem` note rows); the
> trailing stream that is the draft of the final answer is discarded when the
> answer arrives.

> 2026-09-24: the inline `AgentTrace` and the step/token footer render only
> while there is no final reply (a live run, or a turn that ended without
> one). Once the final reply lands the conversation shows the reply body
> alone — the plain sentence or the conclusion-first card. The execution
> process is read on the Response Trace page; a finished answer is never
> parked under its own history.

---

## 5. `AgentTrace`

Container for response execution steps.

States:
- running;
- completed;
- failed.

Behavior:
- visible only while there is no final reply — running turns, and turns that
  ended without one; once the answer lands the trace leaves the conversation
  and its detail is read on the Response Trace page;
- newest running step stays visible;
- completed steps collapse long output;
- two or more consecutive steps of the same type fold under one group row
  (`调用模型 · 3 次 · 28.1s`); expanding shows each step on the second level,
  and each of those opens its own detail. A group that is running or holds a
  failure opens itself — a problem is never hidden behind the fold. Interim
  note rows and lone steps stay loose;
- user can expand individual steps;
- user can open full trace.

---

## 6. `TraceItem`

Collapsed form (2026-09-23: no status icons — the colored check circles
shouted louder than the content):

```text
› Run command   cargo check   检查项目编译状态       18s
```

Recommended row height:
- 32–36px.

Fields:
- action;
- primary detail;
- optional metadata;
- duration/status (words: `进行中` / `已中断` / `已停止` / `失败`);
- disclosure.

Status is carried by words and the row's `trace-row--<status>` class, never by
an icon and never by color alone. Unmeasured durations render empty — never a
bare `—`.

Interim stream prose renders as a quiet note row in the same flow: no chip, no
icon, no disclosure, just the text in secondary color, in arrival order.

Expanded content examples:
- command output (ANSI-stripped for display; the log keeps the raw bytes);
- tool result;
- edited file paths (edit steps) — each chip is a button that opens the
  `文件` panel at that path when a project is loaded;
- searched files;
- model token info.

Error:
- one primary emphasis only: the `原因：…` line in restrained error color
  (exit-code chips stay neutral);
- keep result expanded.

---

## 7. `FinalResponse`

Purpose:
- display user-facing result after trace.

Rules:
- readable 14px body;
- markdown supported — the local parser renders (2026-09-29 pass, one
  renderer shared with the file viewer 预览):
  - headings clamp to three tiers (h3 16px/700, h4 15px/600, h5 13px/600),
    always visibly above the 14px body;
  - GFM pipe tables sit in one bordered surface with 13px cells, a 600
    header row, no zebra, and horizontal scroll inside the wrapper when
    wide; column alignment follows the `:---:` separators;
  - lists nest one indent level at a time (deeper jumps collapse); `- [ ]`
    / `- [x]` render as disabled checkboxes;
  - quotes are a surface block behind a left rule, one `<p>` per paragraph;
  - inline `$…$` and block `$$…$$` render through KaTeX (lazy-loaded; while
    loading or on invalid TeX the literal source stays on screen);
  - ```` ```mermaid ```` fences render as SVG (loading note
    `正在绘制图表…`; any failure falls back to the plain code block);
  - ```` ```diff ```` fences color `+`/`−` rows via hljs
    addition/deletion tokens;
  - `---` draws a rule (no setext headings);
  - links and bare http(s) URLs open in the system browser — only
    http/https/mailto become links, any other scheme stays text;
  - ≥4 headings still get the auto-TOC; a click scrolls the heading into
    view (anchors cannot hash-jump inside the inner scroll container);
- code blocks use mono font;
- avoid wrapping the entire response in a decorative card;
- optional small result/verification block may use a bordered surface;
- when the final reply is on screen, it is the reply — the inline trace and
  its step/token footer are gone (2026-09-24). Process detail belongs to the
  Response Trace page.

> 2026-09-23: conversational answers (你是谁 / 你好 — `plain` on the answer item)
> render as the sentence alone: no 结论 header, no status/axes line, no 本次完成
> wrapper, no 下一步 actions. The report card stays for task answers — including
> task answers with zero steps. The marker is explicit; the UI never guesses
> "plain" from step counts.

> 2026-09-29: `plain` also covers the conversational channel — projectless
> runs and knowledge questions (no skill chip) — not only the greeting
> whitelist. Same sentence-alone rendering; the marker stays explicit.

> 2026-09-30: markdown completeness pass (same local renderer, no new deps):
> inline `~~strikethrough~~` / `==highlight==` (selected token) / `__bold__`
> and backslash escapes, plus `www.` / email autolinks; ordered lists honor a
> non-1 start; `>>` quotes nest one level per marker, and a
> `> [!NOTE|TIP|WARNING|IMPORTANT]` marker sharing its line with the first
> sentence still opens a callout (the remainder becomes the body); `[^id]`
> footnotes hoist to a labeled tail with `↩` back links; code blocks gain a
> head row (language label + copy, same 1.2s `已复制` flip); hljs ships the
> full language set; the TOC follows scroll (`md-toc-active`) and caps at
> 40vh; mermaid blocks gain 查看源码 / 复制代码 / 全屏 (zoom 0.5×–3×,
> Escape/backdrop close); display formulas copy their LaTeX.

---

## 8. `ChangedFilesSummary`

Inline collapsed summary:

```text
▣ 7 files changed       +737  -46               View files →
```

This is the bridge from conversation to code inspection.

> 2026-09-24: the list is this turn's Kodo edits only — one row per path with
> its edit count and cumulative per-patch +/−. It is never the dirty working
> tree. Paths the user had already modified and paths where undo would conflict
> are separate groups.

Click:
- `查看修改` opens the `文件` panel in the right sidebar (§10) at the first
  changed path, on its 变更 tab; demo mode (no file bodies) falls back to the
  Inspector `改动` section;
- each changed-file row opens the `文件` panel at that path (变更 when the turn
  holds a diff, 完整文件 otherwise);
- `撤销修改` undoes only Kodo's edits and reports the truth underneath the
  card — `已撤销本轮 Kodo 修改。` / `本轮没有可撤销的 Kodo 修改。` / conflict
  detail. It never claims success when nothing was restored.

---

## 9. `Inspector`

Modes:

```text
rail
expanded
```

Expanded width:
- 330px default, resizable from 320px to the available workspace width
  (`inspector-resizer`, `role="separator"`, drag or keyboard ←/→ ±16 /
  Shift ±64, windows ≥1200px only);
- two independent persisted widths: base (default 330) and wide (default
  640 for 文件/浏览);
- the visible conversation has a 400px minimum. Further leftward dragging
  hides it without unmounting its run or draft, and the Inspector fills the
  workspace (`app--focus-mode`). Its limit adapts to window/sidebar/rail width;
- rightward dragging restores the conversation immediately at its minimum,
  then continues resizing. Double-click restores the saved base/wide width;
- `inspector-focus-mode` persists the hidden state; the old focus-width setting
  is ignored. Below 1200px the conversation remains visible under the overlay.

Panel layer (chips on the header row):

```text
Inspector | 终端 | 文件 | 浏览
```

Rail sections (inside the `Inspector` panel only; hidden while `终端`,
`文件` or `浏览` is open):

```text
概览 | 改动 | 工具 | 模型
```

The section navigation may be tabs or vertical icons.

An earlier revision of this document forbade both at once. **That rule is
withdrawn**: the panel chips and the icon rail appear together. The two are
not redundant — the chips select the panel, the rail selects the section
within it. A panel and a rail section never share a name: once `文件` became a
panel chip, the rail section was renamed **改动** (testid `rail-files`, icon
and body title 修改文件 unchanged).

Inspector state must be independent from conversation scroll.

The header X closes the panel — except in the `文件` and `浏览` panels, where
it (and `Escape`) demotes to `概览` and the panel stays open.

Each section card collapses from its header. The chevron is drawn identically in
both states; `aria-expanded` carries the state.

---

## 9b. `BrowsePanel` (浏览 panel)

The right-sidebar embedded browser, selected by the `浏览` panel chip
(conversation route; not demo-gated, unlike `文件`, which needs a project).

Structure:

```text
URL bar        input + 前往 (Enter submits)
[refusal row]  only after a hanging embed
native webview Tauri child window parented to main (NSWindow.addChildWindow:
              on macOS, owned + skip_taskbar on Win/Linux); tracks the panel
              content rect via ResizeObserver, debounced 50ms
foot           在系统浏览器打开 — whenever a URL is on screen
```

Rules:
- whitelist `^https?://` — enforced in **both** layers: JS first line in
  `BrowsePanel.tsx` and Rust `parse_safe_external_url` in `main.rs` (`browse_open`,
  `browse_set_url`). `javascript:` / `data:` / `file:` never reach the native
  webview, the system browser, or the iframe slot that used to live here;
- the native webview replaces the previous `<iframe sandbox=…>` — the X-Frame
  restrictions that refused GitHub login / Google / 知乎 cross-origin embed
  don't apply to a child NSWindow;
- position + size: Rust commands `browse_open` / `browse_set_position` /
  `browse_close` track `.ins-content--flush` via `ResizeObserver` (debounced
  50ms, pointerup commits, no per-frame IPC);
- the last URL persists (`useSetting("browse-url")`) and auto-loads on the
  next open;
- no successful load within 10s → the refusal banner offers 在系统浏览器打开
  and 重试 (the native webview never fires a cross-platform "load failed"
  event, so the timeout is the only signal we have; cross-origin
  X-Frame-Options no longer apply, but network errors, TLS errors, and
  CAPTCHAs still happen);
- `Escape` and the header X demote to `概览` — same contract as `文件`;
- widens from the same 640 wide key as `文件`; no history, no tabs, no
  context menu (app-wide right-click suppression stays).

---

## 10. `ChangedFiles`

Compact list:

```text
source/db/source.table            +124  -18
src/router/mod.rs                 +210  -12
src/heartbeat.rs                   +96   -6
```

Click file:
- show diff/detail.
- realized as the `FileViewer` inside the right sidebar's **`文件` panel**
  (Codex-style, not a modal and not a route): project file tree on the left,
  and on the right up to three tabs — 完整文件 (line-numbered full text
  with syntax highlighting via highlight.js — a curated language set from
  `src/highlight.ts`, unknown languages degrade to plain text), 预览, and 变更
  (this turn's diff, only when a diff exists for the selected file; changed
  files open on 变更). The panel body fills the column (no outer padding, no
  backdrop). `Escape` or the header X **demotes** to the `概览` section — the
  panel stays open; from `概览`/`终端`, X closes the panel. On windows
  <1200px the tree hides behind a FolderTree toggle and drops in as a 260px
  left drawer; selecting a file closes the drawer. The demo fixture carries no
  file bodies, so every file entry falls back to the Inspector `改动` section
  there. Besides the card entries (§8), the top-bar 文件 icon opens the same
  panel in browse mode: full tree, nothing preselected.
- **click-through from the conversation** — three more entries open the same
  panel directly:
  - answer markdown inline code that looks like a path (`src/api.ts`,
    `crates/core/src/lib.rs`) — rendered as a button (`md-code-path`), probed
    via `validateContextPath` before the panel opens (the full tab);
  - context cards in a sent message (project-internal paths only; external
    absolute paths stay static cards);
  - edit-step file chips inside the expanded trace — known-good paths, open
    on the 变更 tab directly.
  Shape-gated at render (`looksLikePath`), probe-gated at click; URL schemes,
  `..`, and spaces never render as clickable.
- selection is probe-first: `probe_context_file` classifies the file before
  any read (non-text never hits the backend's binary refusal). 预览 routes by
  kind — rendered markdown (its code blocks also highlighted), HTML with its
  scripts enabled inside a sandboxed iframe (`sandbox="allow-scripts"` —
  scripts, CSS and animation run, but the frame stays isolated from the
  app), images inlined as data URLs (≤4MB), CSV
  as a real table (quoted cells honored), and PDF / Office / oversized /
  binary files as a kind card (icon, badge, size) with 用系统应用打开
  handing off to the system default app. Non-text files hide the 完整文件 tab
  (there is no readable body — the card says so instead of erroring).
- file rows in the tree carry a monochrome kind icon; directory rows keep
  their caret.

Tree aids: a 筛选文件… path filter (substring, auto-opens matching
directories) and a 仅变更 toggle that narrows the tree to this turn's changed
files.

The listing behind the tree keeps git-tracked content ahead of untracked
files under the backend cap, so an untracked junk pile (`.venv.*` backups,
`.pytest_cache`) can never crowd real files out of a large project; those
junk dirs are ignored outright (matching the agent's ContextManager), while
image / PDF / Office files stay in the list precisely because the viewer can
preview them.

差异 view: per-file +n −m stats and a 统一/对照 toggle — unified with long
unchanged runs folded behind 展开 N 行上下文 (two lines kept each side), or
split side-by-side pairing deletions (left) with additions (right).

The diff parse and colored row rendering are shared (`conversation/diff.tsx`):
trace edit steps and the Inspector's changed-file bodies render the same
+/- colored rows with headers stripped. Folding and 统一/对照 stay
viewer-only — review wants collapsed context; a glance wants the body.

变更 is reachable for **every** changed file — the tab never gates on a stored
diff body. A create whose body was never recorded renders as its added lines
(synthesized from the file content); an edit without a recorded body shows the
stats and an honest 未留存 note that points at 完整文件. A `@@ no changes @@`
stub is treated as no body, never as empty change content.

---

## 11. `ToolUsage`

Compact aggregate:

```text
Search codebase        1×
Read file              3×
Run command            2×
Docker                 1×
Edit file              2×
```

This is a summary only.  
Detailed calls live in Response Trace.

---

## 12. `LLMCalls`

Compact aggregate:

```text
Claude 3.5 Sonnet
Input   12.4k
Output   2.1k
Duration 24s
```

Detailed request/response metadata belongs in Response Trace.

Never expose hidden/private reasoning.  
Only show user-visible thinking summaries and model-call metadata that the runtime intentionally records.

---

## 13. `Composer`

Minimum structure (2026-09-23: controls sunk to the bottom of the input box):

```text
[ thumbnail | gray card + type badge ]     ← attachment cards, when present
Message Kodo...
[ attach ] [ permission ▼ ] [ agent ▼ ]   [ model ▼ ] [ send ]
```

The Work/Code switcher is **not** here — it lives in the sidebar brand block
(`BrandSwitcher`, `DESIGN.md` §11). The composer's skill picker (plus panel and
`/` filter) follows the current space: code skills vs work skills
(`data/skills.ts`).

**Agent selector** (`.composer-agent-trigger`, 2026-09-30, the one approved
composer addition): a `Bot` icon + current choice + caret directly after the
permission trigger, styled with the permission trigger's chrome. The menu
lists `无智能体` (`composer-agent-none`) plus one `composer-agent-<name>` item
per persona with its description as the hint; the active choice is
check-marked (spacer glyph otherwise, same column discipline as the Sidebar
mode menu). The choice is **session-scoped** (App-owned map keyed by
conversation, in-memory — a reload resets to `无智能体`), switchable
mid-session, and resolved **at send time**: the backend loads the persona's
instructions before opening/running the session and appends them to the
system prompt after the skill block (插件页 智能体 tab, §13d). Default is
无智能体 — a send with no selection carries no persona block.

> 2026-10-04 — **persona list density (12+ builtins)**. The menu now
> top-stacks a `<input class="composer-agent-filter">` (`search 智能体…` —
> matches `name` and `description` case-sensitively, no regex, no fuzzy rank)
> over two `composer-agent-section` group headings — `内置`
> (`data-testid="composer-section-builtin"`) first, then `自定义`
> (`data-testid="composer-section-user"`) — followed by `无智能体` last so it
> sits next to the trigger regardless of filtering. The builtin items carry
> an `内置` badge in the hint column; user items show the description as
> before. Filtered-out items drop out of the DOM (no disabled-but-listed);
> when the filter narrows a section to zero, the heading also hides. The
> list refetches on a per-App `agentsVersion` bump so a save/delete in
> AgentsTab shows up in the next dropdown open without a page reload.

> 2026-09-30: the model menu is two panels, not an accordion — providers in a
> left column (clicking one makes it the active provider), the active
> provider's model list in a right panel next to it
> (`data-provider-index` rows on the left, `data-model-id` rows on the right,
> selected model on `menu-item--selected`). The panel right-aligns on the
> trigger so it never spills past the viewport edge. The footer keeps
> 配置自定义模型; an empty provider list still offers 添加提供商 first.

It is a controlled `<textarea rows={1}>` that grows with its content up to about
160px, then scrolls internally. Enter sends; Shift+Enter inserts a newline.

**Context ring** (`.composer-context-ring`, 2026-10-04): a Codex-style
circular progress ring sits **as a sibling of the model picker** in
`.composer-right`, with `gap: 8px` between them so the ring never visually
merges with the picker chrome. While a turn is in flight, the ring fills
proportionally to `SinkEvent::ContextBudget::percent`; on turn end it
clears. Three color bands, matching the old in-reply bar:
- `<60%` → muted (`--kodo-text-muted`),
- `60–80%` → warn (`--kodo-warning`),
- `≥80%` → danger (`--kodo-danger`).
Hover or keyboard focus raises a popover (role=tooltip) with three lines,
following the `.turn-rail-preview` recipe (§16): `背景信息窗口：` /
`N% 已用（剩余 M%）` / `已用 X 标记, 共 Y`. Tokens only — no per-tone
classes leak. The reply-side horizontal context bar was retired; the
ring is now the single source of truth.

Attachment cards (`.ctx-card`, 2026-09-23) sit above the draft:
- `image` → thumbnail card (a lone image is one large thumbnail);
- `pdf` / `office` / `text` / `other` → gray card with a centered glyph, a
  colored type badge (PDF #1F2328, DOC #2B579A, XLS #217346, PPT #D24726,
  TXT/CODE #667085, IMG #2563EB, unknown #667085) and the file name;
- the remove control keeps the `移除上下文 <path>` label; `data-context-path`
  stays the test hook.
The backend `probe_context_file` command classifies the file and returns a
small preview (image data URL or text snippet); binary attachments attach
fine and are noted as 「已附带附件（内容未内联）」 at send time instead of
failing the turn.

Image attachments (2026-10-13) ride the multimodal path when the active
model supports it: ≤ 4 MiB images inline as a `ContentBlock::Image` on the
first user message (read through `tools::resolve_pinned_file` so sensitive
paths and project-base validation still apply); > 4 MiB or vision-incapable
model fall back to the path-only descriptor above. Selection is driven by
`ProviderCapabilities::vision` (template default) plus
`ModelSpec::vision_override` (per-model).

Sent messages render the same card in the bubble (2026-09-29): one shared
`ContextCard` component for both sites — the bubble variant is static (no
remove control), keeps `data-context-path` and the full path in `title`, and
shows the file name instead of a truncated path. Without a probe the card
degrades by extension (`kindFromPath` still yields the right badge); image
paths in a reloaded turn are lazily re-probed once so thumbnails come back
after a reload. Probe state lives on the conversation page, is cleared on
session switch, and survives sends so the bubble keeps its thumbnail.

States:

| State | Trigger | Composer |
|---|---|---|
| idle | draft empty | send disabled |
| ready | draft non-empty, nothing running | send enabled |
| generating | a run is in flight | send replaced by **stop** |
| disabled | no conversation open | input and send both inert |
| approval pending | a write/command step awaits the user | composer keeps working; the **approval bar** above the reply intercepts the step (implemented, see run driver) |

> 2026-09-29: the `no project (code mode)` row is gone — sending never
> requires a project; a projectless run is constrained Q&A with no tools
> (`DESIGN.md` §11).

A run that is over must not leave the composer claiming to be generating, and a
run that was killed must not leave it claiming to be finished.

Do not add many tool toggles to the composer in V1 — the agent selector
(above) is the only approved composer addition of 2026-09-30.

---

## 13b. `SkillsTab` (插件 · 技能 tab under PluginsPage)

> 2026-09-30: the page head and the `新建技能` action moved up to PluginsPage
> (§13d) — this component renders only the tab body: the three cards. Class
> split, test hooks and content rules below are unchanged.

Three cards (the head's right side on the PluginsPage carries the
`新建技能` action button): the builtin list, then — only while a project is
selected — the project's own skills, then the user's. The middle card is a
scoping distinction, not a new navigation level.

```text
[Sparkles] 技能                    [+ 新建技能]   ← PluginsPage head (per-tab action)
可从任务输入框调用的可复用工作流

┌ (aria-label 可用技能, no visible head) ┐
│ 🧩 缺陷修复                      /bug-fix│
│ …builtin rows for the current space…     │
└──────────────────────────────────────────┘
┌ 项目技能 (head + ＋) ────────────────────┐
│ 🧩 release-check       /release-check ✎ 🗑│   ← only with a selected project
│ (or) 当前项目还没有项目技能。                │
└──────────────────────────────────────────┘
┌ 自建技能 (visible head) ───────────────┐
│ 🧩 changelog        /changelog ✎ 🗑│
│ (or) 当前空间还没有自建技能。        │
└───────────────────────────────────┘
```

- **Builtin card**: one list of builtin skills for the **current agent space**
  (`data/skills.ts`): code shows the seven coding skills (缺陷修复 … Kodo UI),
  work shows the work skills (写文档、做总结、问答、工作流自动化). Rows are
  label + `/id` in the same compact style; switching the brand switcher swaps
  the list. Every row is a `<button className="skill-row" data-skill-id>` that
  opens the detail modal (keyboard-reachable; a pixel-neutral button reset
  keeps the row geometry identical to the old div).
- **Project card**: `list_skills` filtered to `source=project` **and** the
  row's space, shown only when the shell has a selected project. Its head
  carries an icon-only `＋` (`在项目中新建技能`, `data-testid="skill-new-project"`)
  that opens the editor pre-scoped to the project store. Rows are the same
  `.user-skill-row` geometry plus `.project-skill-row` and a
  `data-project-skill-id` hook (user rows use `data-user-skill-id`); edit and
  delete hit the project store. Empty state: 当前项目还没有项目技能。
- **User card**: `list_skills` filtered to `source=user` **and** the row's
  space (`skillSpace(taskTypes) === agentMode`), refreshed on mount and after
  every save/delete. Rows are `.user-skill-row` containers: an inner
  `.user-skill-open` button opens the detail, with separate Pencil (编辑) and
  Trash2 (删除) icon buttons at the tail. Delete is two-step like
  `从列表移除`: the first click flips the label to `确认删除？`, the second
  executes. With no user skills in the space, one `.skills-empty` row shows
  当前空间还没有自建技能。
- **Class split is an invariant**: `.skill-row` belongs to builtin rows only;
  store rows (project and user) must never carry it (acceptance asserts
  exactly 7 `.skill-row` in code space, while the demo fixtures' store skills
  are present).

---

## 13c. `SkillDetailModal`

Preview / editor for one skill, in the `settings-modal` house style (backdrop +
panel, Esc closes). Panel geometry:
`width: min(640px, 100%); height: min(720px, 100%)`, flex column of
`fv-head` / body / optional foot; test hook `data-testid="skill-detail"`.

- **View mode** shows sections only when parsed data exists: 描述 (the skill's
  one-line `## description`, falling back to the static row detail), 适用任务,
  上下文策略, 验证策略, 工作流 (`id · kind · title` rows),
  允许工具, 完成标准, 使用提示, 技能内容 (raw SKILL.md in a `<pre>`). The
  header shows the display label and a 内置/项目/自建 source chip. Opening a
  builtin row upgrades the static detail with `read_skill` when the core
  answers; raw-browser fallbacks keep rendering the static detail.
- **Builtins are read-only**: no foot actions, no edit/delete. **Project and
  user skills** get a foot with 编辑 and 删除 (two-step confirm, no dialog);
  every read/write names its store (`scope: "project" | "user"`) so the row's
  card and the modal always agree — a project row never touches the user
  store and vice versa.
- **Edit / new** swap the body for the raw editor: a name input plus a
  monospace SKILL.md textarea (`.skill-editor` — the parser is the validator,
  so there is no second structured form). 新建预填 a per-space template
  (work → `- work` / work-focused / run_command set; code → docs-style).
- **Validation** runs before any invoke (charset `[A-Za-z0-9_-]{1,64}`, empty
  name/content, display-layer reserved ids) with zh-CN inline errors
  (`role="alert"`); backend `save_skill` rejections surface the same way
  without closing the editor. On save the `# Skill:` title line is synced to
  the filename (`withTitle`). Renames store the new file first, then
  best-effort delete the old one — worst case a recoverable duplicate, never
  data loss. Save/delete success refreshes the list; delete closes the modal.

---

## 13d. `PluginsPage` (插件页)

> Added 2026-09-30: the sidebar's 技能 and MCP destinations merged into one
> `插件` row; the page adds a third tab, 智能体. The 4th plugin type (插件) is
> explicitly deferred — **no placeholder, no empty tab**.

Structure: `main.main > header.page-head` (Puzzle mark, `插件` title, a
per-tab subtitle, spacer, per-tab head action) → `div.scroll > page-inner
page-inner--wide` containing `nav.page-tabs[role=tablist]` (markup follows
TracePage's tabs; generic `page-tabs` CSS) and the active tab body.

```text
[Puzzle] 插件        <per-tab subtitle>        [per-tab action]
[ 技能 ] [ MCP ] [ 智能体 ]
┌ tab body (SkillsTab | McpTab | AgentsTab) ─────────────────┐
```

- **Route**: `/plugins` with `?type=skills|mcp|agents`; the default tab
  (skills) carries **no query** so wide screenshots stay query-free. Old
  `/skills` / `/mcp` (with or without the `/ui-demo` prefix) are permanent
  `parse()` aliases landing on the matching tab. Tab clicks rewrite `?type=`
  with `URLSearchParams`, preserving `?inspector=`; test hooks
  `plugins-tab-<type>`.
- **Head actions**: 技能 → `新建技能`, 智能体 → `新建智能体`, MCP → none.
  Each action is a nonce counter handed to the tab as `openNew: number`; a
  first-run ref guard inside the tab prevents a stale nonce from reopening
  the modal after a tab switch. The nonce re-fires only on repeat clicks.
- **Subtitles**: 技能 → the existing skills subtitle, MCP → the wire-convention
  line (`mcp__服务器__工具名`, `credentials.log`), 智能体 → 名称、描述与说明词
  —— 发送时注入系统提示.
- **Per-tab heads**: the cards inside each tab keep their own card heads
  (自建技能, 连接器目录, 智能体 …); only the page head and tabs belong to
  PluginsPage.

### 智能体 (AgentsTab + AgentDetailModal)

Persona records — **not subagents**: name + description + instructions that
append to the system prompt when selected in the Composer (§13).

- **Row card**: two `section.skills-card` blocks stacked under the head —
  `内置` first with the 12 read-only builtins (`.agent-row--builtin`,
  `data-agent-source="builtin"`, `agent-builtin-<name>`, `agent-badge-builtin`
  pill), then `自定义` with user-defined personas. The `.agent-row` class
  stays the unifying shell so the class-split invariant (`.agent-row .skill-row`
  never appears) holds for both blocks. The builtin card hides Pencil /
  Trash2 — editing / deleting a builtin goes through the error path the core
  rejects (zh-CN `智能体名与内置冲突` / `内置智能体 … 不可删除`). User rows
  keep the same two-step delete armed via `skill.deleteConfirm`.
- **Editor**: `settings-modal` house style (same family as §13c) with three
  structured fields — 名称 (`agent-name`), 描述 (`agent-description`),
  说明词 (`agent-instructions` textarea) — and nothing else: no enable
  toggle, no model/permission/tool fields, no raw-markdown mode. Validation
  order before any save: 名称必填 → charset (`[A-Za-z0-9_-]{1,64}`) → 与内置
  不冲突 → 描述必填 → 说明词必填, zh-CN `role="alert"` inline. The frontend
  composes `# Agent: <name>\n\n## description\n…\n\n## instructions\n…`; the
  Rust parser is the authoritative validator and its rejections surface
  inline without closing. Renames save the new file first, then best-effort
  delete the old (recoverable duplicate, never data loss).

#### Built-in personas (2026-10-04)

The 12 read-only builtins ship as static markdown at `agents/<name>.md` in
the workspace root, embedded by `crates/agent/src/agents.rs::BUILTIN_AGENTS`
with `include_str!("../../../agents/<name>.md")` — mirroring the skills
`SOURCES` pattern (`crates/agent/src/skill.rs`):

| id | description |
|---|---|
| `reviewer` | 严格代码评审 |
| `explainer` | 把概念讲清楚 |
| `debugger` | 根因定位 |
| `refactorer` | 重构与清理 |
| `tester` | 测试编写 |
| `security-reviewer` | 安全审查 |
| `perf-reviewer` | 性能审查 |
| `documenter` | 文档撰写 |
| `planner` | 任务拆解 |
| `api-designer` | API 设计 |
| `translator` | 翻译与本地化 |
| `ui-designer` | UI 设计（kodo-aware） |

The `AgentView.source: "builtin" | "user"` field drives row badges, the
Composer dropdown grouping, edit/delete gating, and the `agentsVersion`
re-fetch on the Composer side after AgentsTab CRUD.

### 智能体数据 (storage & injection)

- **Format**: markdown at `state_dir()/agents/<name>.md` with three required
  parts: `# Agent: <name>` (title must match the filename stem), `## description`
  (single line), `## instructions` (body, multi-line). Name charset mirrors
  skills: `[A-Za-z0-9_-]{1,64}`. User scope only — no project/agent-space
  scoping (personas are global).
- **Builtins win**: the 12 read-only builtins in
  `crates/agent/src/agents.rs::BUILTIN_AGENTS` (loaded via `include_str!`
  from `crates/agent/agents/<name>.md` — see the Built-in personas
  subsection above) take priority over user files of the same name on
  `resolve_instructions`, and `save_user_agent` / `delete_user_agent`
  reject builtin names with a zh-CN error (`… 是内置智能体名，请换一个名称`
  / `内置智能体 … 不可删除`). `load_user_agents` silently skips files that
  shadow a builtin name so a stale user copy can never reach the UI.
- **Commands**: `list_agents` / `read_agent` / `save_agent` / `delete_agent`
  (`view.rs::AgentView`, camelCase wire: name, description, instructions,
  markdown, **source** = `"builtin" | "user"`). `list_agents` prepends the
  builtins to the user list; `read_agent` checks builtins first. Missing
  persona errors are zh-CN (`智能体不存在…`).
- **Injection**: the Composer selection travels with `send_message(…, agent)`;
  the backend resolves the persona **before** `session::open` (a bad name has
  zero side effects), threads it as `RunRequest.agent_instructions`, and the
  run appends `agent_instructions_block` to the system prompt **after** the
  skill block and before the extended-thinking line. No selection → no block.
- **v1 boundary**: `direct_conversation_answer` and the canned shortcut stay
  mode-based — the persona rides the tool-run channel only for now.

---

## 14. `SettingsNav`

The left column of the Settings page: one row per category, icon plus title, the
selected row using the same active tokens as `nav-item`.

```text
⚙ General
▣ Models
🔧 Tools & Permissions
📁 Projects
🗄 Archive & Storage
🎨 Appearance
```

Behavior:
- exactly one category selected at a time;
- selecting a category swaps the right-hand pane;
- every category must render a non-empty pane.

The right pane is `SettingsSection`. It is a bordered group — section title, short
explanation, form controls. Do not turn every individual setting into a card.

Settings rows never include a control that no longer changes anything: when the
scan-based project model was removed, the `Default workspace root` and
`Scan for project config files` rows were deleted rather than left in place.
Tools & Permissions holds the `权限模式` row for the shared permission key —
the duplicate `文件写入` / `Git 访问` selects that bound the same key were
deleted the same way (one setting, one row; the composer trigger is the live
entry point). MCP servers are not a settings row at all — see the MCP page
below.

### MCP tab (插件 · MCP, was: MCP sidebar destination)

> 2026-09-29: the MCP surface moved out of Settings → 工具与权限 onto its own
> sidebar destination under 技能 (discoverability). Tools & Permissions keeps
> only the `权限模式` row.
>
> 2026-09-30: the sidebar destination became a tab on the 插件 page
> (`/plugins?type=mcp`, screenshot `/ui-demo/plugins?type=mcp`; the old
> `/mcp` paths are permanent aliases) — §13d. All content rules below are
> unchanged.
>
> 2026-09-30: the preset select became the connector catalog — a card grid
> above the server rows (user-requested; the one approved tile grid, DESIGN
> §9). The editor keeps no preset field.
>
> 2026-09-30 (second pass): the catalog is one instance per connector. A
> template that is already configured renders a disabled 已添加 tile instead
> of adding a duplicate; `添加 MCP 服务器` stays unlimited for deliberate
> variants.

The tab is `McpTab`, `/plugins?type=mcp` (`/ui-demo/plugins?type=mcp` for
screenshots; alias `/mcp`). Layout: the PluginsPage head (Puzzle mark, `插件`
title, the MCP subtitle line naming the `mcp__服务器__工具名` wire convention
and `credentials.log`), the tabs row with MCP selected, then a
`page-inner--wide` stack of two sections — the connector catalog and the
configured-server list with its inline editor.

```text
MCP   外部工具服务器；启用后其工具以 mcp__服务器__工具名 提供…
连接器目录  点击卡片按预设添加服务器；每个连接器只添加一次，密钥留空…
┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐
│ ⬡ github│ │ ⬡ slack │ │ ⬡ notion│ │ …×20    │
│ 操作 …  │ │ 收发 …  │ │ 读写 …  │ │         │
│[stdio]＋│ │[stdio]＋│ │ [http] ＋│ │         │
└─────────┘ └─────────┘ └─────────┘ └─────────┘
          configured template → disabled tile, foot reads ✓已添加
已配置服务器                    ＋ 添加 MCP 服务器
┌────────────────────────────────────────────────────────────┐
│ › filesystem   [stdio]  npx                     [on]  ✎ ␡  │
│ › remote-tools [http]   https://mcp.example.com [off] ✎ ␡  │
└────────────────────────────────────────────────────────────┘
```

Connector catalog (`.mcp-catalog` / `.mcp-card`): one compact tile per
`MCP_TEMPLATES` entry — `src/data/mcp.ts`, the `PROVIDER_TEMPLATES` pattern,
20 well-known community servers (the original 10 plus sequential-thinking,
context7, playwright, google-drive, redis, everything, figma, linear, fetch,
time). A tile shows a geometric lucide icon (icon key on the template, map on
the page — no brand logos), the product name, a one-line zh description from
`T.page.mcp.catalogDesc`, the transport chip and a ＋添加 affordance. The tile
is one `<button>` (the affordance is a span); clicking it opens the add editor
prefilled from the template. **One instance per connector** — a template is
"already configured" when any server carries its name or its launch identity
(the stdio package — first non-flag argv — or the http URL); that tile becomes
the disabled 已添加 state (Check + `已添加`, no hover lift) and refuses a second
instance. A deliberate variant (a second filesystem root) goes through
`添加 MCP 服务器`, which stays unlimited and never dedupes; deleting the
configured instance frees the tile again. This tile grid is the **approved
exception** to the compact-rows rule (user-approved 2026-09-30; DESIGN §9):
6px radius, 1px borders, no shadows, hover touches border/background only.
Every other surface keeps its rows and tables.

Row (configured servers): a leading chevron that expands the server's tool
list below it, display name, a transport chip (`stdio` / `http` — technical
values, not translated), command (stdio) or URL (http) as the secondary line,
an enable toggle (the shared switch markup), then edit and delete icon buttons.
The section head carries `已配置服务器` and the `添加 MCP 服务器` button (blank
custom editor).

Tool list (expanding the chevron): one compact line per tool — the bare tool
name in a `code-chip` plus its description. It is read through `mcp_list_tools`,
which connects the server once with its stored secrets **server-side** and
returns names and descriptions only; schemas and secrets never reach the
webview. Loading and failure are single zh-CN hint lines (a broken server is
never fatal). There is no test-connection control — the expanding list is the
one live contact the page makes.

Editor (`.provider-editor`): 显示名称, 传输方式 select, then the active
transport's fields only — stdio shows 命令 / 参数 (one input, whitespace-
separated) / 环境变量 key-value rows; http shows 服务器地址 / 请求头 key-value
rows. Switching transport swaps the fields and clears the other half on save.
Actions are 取消 / 保存. There is no preset field: opened from a catalog tile
the editor shows the `来自连接器目录：<name>` hint and is prefilled (known
secret keys come up as blank rows); opened from `添加 MCP 服务器` it is a blank
custom seed; editing an existing server never prefills from a template — a
template can clobber configuration only at creation time.

Key/value rows (`.kv-row`): key input, value input (masked values render as
dots), remove button; `添加一行` appends. A masked or blank value sent back
keeps the stored secret — mask writeback, the provider API key contract — and
deleting a row removes the secret. Env and header values live only in
`credentials.log`; the settings copy stores empty strings.

Empty list shows the empty hint (which points at the catalog) under the
`已配置服务器` head. Creating a server starts enabled (the row toggle can pause
it later).

---

## 15. `AutoOnboarding`

> 2026-09-23: first-run auto onboarding, in the spirit of Dia's startup
> sequence. A full-viewport dialog (`z-index` above settings modal) that plays
> itself — no "start tour" click.

Structure:

```text
AutoOnboarding (fixed, inset 0, canvas white)
├─ 跳过                    top-right
├─ Stage (centered column)
│  ├─ kicker               optional overline (accent, tracked)
│  ├─ title                hero line (40px)
│  ├─ body                 one sentence (14px)
│  └─ 开始使用              final stage only
└─ Dots                    bottom center, one slim bar per stage
```

Behavior:
- stages auto-advance every 3.2s; the active dot's bar fills over the same
  duration (`--onb-stage-ms`, driven from `STAGE_MS` in the component);
- dots are clickable (jump to a stage, restarts its timing bar);
- the final stage stops on `开始使用`; `跳过` and `Escape` dismiss from any stage;
- both `开始使用` and `跳过` stamp `onboarding-done` — the overlay is one-shot
  and never returns on later launches;
- `waiting` → `open`/`closed` after the setting resolves, so a returning user
  never sees a flash of the overlay;
- demo routes (`/ui-demo/*`) and the plain browser never show it;
- entrance motion is a staggered fade/rise (children only); under
  `prefers-reduced-motion` movement is dropped and the active bar holds static.

The 40px hero title is a deliberate exception to the density rules (DESIGN.md
§3/§2): this is a transient startup surface, not application chrome. Everything
behind it keeps compact IDE density.

Copy lives in `T.onboarding` (zh-CN except the brand line `Get code done.`,
per DESIGN.md §10).

---

## 16. `TurnRail`

> 2026-09-30: conversation turn navigator, modeled on deepseek-harness's
> TurnNavigator. An **overlay** in the scroll area's right gutter — it
> consumes no layout width and is never a column (LAYOUT §3, UI_ACCEPTANCE
> §3). It is the only new conversation chrome of the day.

Structure:

```text
.conv-stage                    position context + container query target
├─ .scroll                     the transcript scroller (unchanged)
└─ TurnRail (.turn-rail)       absolute inset 0, pointer-events none, z 10
   └─ .turn-rail-frame         right 12px, vertically centered, width 28px
      ├─ .turn-rail-scroller   hidden scrollbar, overscroll contain, end fades
      │  └─ .turn-rail-marks   fixed 10px pitch ladder (6px inset per end)
      │     └─ .turn-rail-mark-position > button.turn-rail-mark  per turn
      └─ .turn-rail-preview    role=tooltip, right of the frame, pointer-events none
```

Behavior:
- renders only with **≥2 turns**; hidden when the stage container is
  **<900px** wide (`@container`, first in the repo);
- one tick per turn, right-aligned 12×2px at rest (`--kodo-border-strong`),
  18px `--kodo-text-secondary` on hover/focus, 20px `--kodo-accent` when
  active, focus ring `--kodo-focus-ring`;
- pointer anywhere on the frame jumps to the nearest tick; each tick is also
  a `<button>` — keyboard reachable, `aria-label` `跳转到第 N 轮`,
  `aria-current="true"` on the active one, `aria-describedby` links the open
  preview;
- hover or focus raises `.turn-rail-preview`: prompt clamped to 1 line (600
  weight) + response clamped to 3 lines (`--kodo-text-secondary`),
  `pointer-events: none`, `--kodo-canvas` on `--kodo-border`, popover radius
  and the find-bar shadow (transient overlay, DESIGN §4);
- click scrolls the turn into view (`scrollIntoView({ block: "center" })`,
  smooth unless `prefers-reduced-motion`); the first scroll event releases
  stick-to-bottom, so an upward jump never fights the follow effect;
- the active tick tracks the reading band (IntersectionObserver,
  `rootMargin "0px 0px -70% 0px"` — the Markdown TOC precedent): topmost turn
  past the 70% line wins; queue/approval rows carry no `data-turn-index` and
  never win;
- an over-long ladder scrolls **inside** the frame (height
  `min(natural, stage−64px, 420px)`) with 24px mask fades at each scrollable
  end; while the pointer works the rail, active-follow does not scroll it;
- ticks carry `data-turn-index` on both `.msg-user` and `.reply` roots — the
  rail's addressing hook (⌘F keeps its positional addressing, unchanged);
- `prefers-reduced-motion` drops every transition/animation in the rail.

Slots: `data-testid` `turn-rail` / `turn-rail-mark` / `turn-rail-preview`;
`z-index: 10` — above transcript content, below find-bar (30), composer menus
(40–42), lightbox (60) and palette (65). Copy lives in `T.turnRail`
(label / jump / turn). Tokens only: `--kodo-border-strong`,
`--kodo-text-secondary`, `--kodo-accent`, `--kodo-focus-*`, `--kodo-canvas`,
`--kodo-border`, `--kodo-radius-popover`, `--kodo-font-*`.

---

## 17. Platform shell — macOS menu bar & status item

> 2026-10-14. The only surface in this document with **no DOM**. Both menus
> are drawn by the OS, live outside the window, and are built in Rust
> (`apps/desktop/src-tauri/src/platform.rs`). The renderer contributes
> nothing: no component, no CSS, no `i18n.ts` key, no visual-regression
> baseline change (LAYOUT §1).

### Menu bar (macOS only)

```text
Kodo │ 文件        编辑      显示            窗口      帮助
     │ 新建任务⌘N  撤销      侧边栏 ✓        最小化⌘M  键盘快捷键
     │ 添加项目⌘O  重做      检查器 ✓        缩放      打开状态目录
     │ ───────      ───────   外观 ▸ 浅色/深色/跟随系统
     │ 关闭窗口⌘W  剪切       ───────
     │             复制      进入全屏
     │             粘贴
     │             全选
```

- **The Edit roles stay predefined.** `undo` / `redo` / `cut` / `copy` /
  `paste` / `select_all` are `PredefinedMenuItem`s, not text items. Dropping
  them is the standard way a custom Tauri menu silently takes ⌘C, ⌘V and ⌘A
  away from the webview.
- Predefined roles keep **no custom label** — macOS localizes them itself.
  Custom items (新建任务, 侧边栏, …) carry their zh-CN label in Rust, which is
  the DESIGN §10 carve-out recorded there.
- **App menu** is the first submenu: 关于 Kodo (native About panel),
  服务, 隐藏/隐藏其他/全部显示, 设置… ⌘,, 退出 Kodo.
- **Scope.** macOS only. `Menu::default` is the only thing that ever installed
  a menu bar, and it is `#[cfg(target_os = "macos")]`, so Linux has never had
  one — an unconditional custom menu would hand Linux a menu bar nobody asked
  for.
- 开发者工具 appears under 显示 in debug builds only.

### Status item (tray)

Declared by `tauri.conf.json` → `app.trayIcon` (icon embedded at compile time
from `icons/tray.png`), so it exists on every desktop platform. The menu and
tooltip are attached in `setup` by `platform::build_tray`.

```text
显示 Kodo
新建任务
──────────
运行状态：空闲        (disabled — reports, never commands)
──────────
退出 Kodo
```

- `iconAsTemplate: true` — the icon is a **black silhouette**, so macOS
  inverts it for a dark menu bar. `icons/tray.png` is derived from the
  transparent icon pack; the command is recorded in `docs/logos/README.md`.
- `showMenuOnLeftClick: false` — left click **toggles the window**, right click
  opens the menu.
- Tooltip and the 运行状态 row follow the run lifecycle (`Kodo` ⇄
  `Kodo — 运行中`), driven from `emit_run` — the single funnel every run event
  already passes through. This matters because the window is often hidden
  during a run.

### Window behaviors

- **Close hides, does not quit** (`CloseRequested` → `prevent_close()` +
  `hide()`, main window only, macOS). A run in flight outlives the window.
- **Dock click returns**: `RunEvent::Reopen { has_visible_windows: false }` →
  `show_main`. Without it, hide-to-tray would be a one-way door.

### The bridge

The renderer never renders a label and never sees a string — it receives an
**action id** on `menu:action` and dispatches it onto handles the app already
had (`App.tsx`, one `switch`). Ids: `new-task` · `add-project` ·
`open-settings` · `toggle-sidebar` · `toggle-inspector` · `theme-light` ·
`theme-dark` · `theme-system` · `shortcuts`. Two menu entries are handled
entirely in Rust and never reach the renderer: 退出 Kodo and 打开状态目录.

Check marks flow the other way: the window is the source of truth, and
`set_menu_state` (`{ sidebar, inspector, theme }`) is called whenever one of
them changes, so the menu can never show a stale ✓.

Menu ids are dotted (`kodo.view.sidebar`) and looked up one level at a time —
`Menu::get` / `Submenu::get` never recurse.
