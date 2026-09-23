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
- avoid project cards.

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
trailing `⋯` opens the row menu without expanding anything.

Row menu (`ProjectMenu`):

```text
Rename…
Reveal in Finder
Remove from list
```

"Remove from list" is destructive-looking but is not destructive: the entry goes,
the directory stays. The label says "from list" for that reason.

Drag: the browser's own drag-and-drop, no library. Dropping writes the entire new
order to the core rather than a relative move.

---

## 3. `ConversationRow`

Visual:

```text
   │ ◉ 修复 k2k-rust 未知表路由                 10:24
```

Rules:
- one line title when possible;
- compact metadata;
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
> the `技能` destination (see `LAYOUT.md` §2), and `归档` opens from Settings →
> 归档与存储. Reason recorded there.

---

## 4. `AssistantReply`

Structure:

```text
Avatar + Kodo + time
Working/completed state
AgentTrace
FinalResponse
ChangedFilesSummary
```

Rules:
- no outer card around the entire reply;
- trace belongs visually to the reply;
- final response must be visually stronger than trace metadata.

---

## 5. `AgentTrace`

Container for response execution steps.

States:
- running;
- completed;
- failed.

Behavior:
- newest running step stays visible;
- completed steps collapse long output;
- user can expand individual steps;
- user can open full trace.

---

## 6. `TraceItem`

Collapsed form:

```text
✓ Run command   cargo check   检查项目编译状态       18s   ›
```

Recommended row height:
- 32–36px.

Fields:
- status;
- icon;
- action;
- primary detail;
- optional metadata;
- duration;
- disclosure.

Expanded content examples:
- command output;
- tool result;
- searched files;
- model token info.

Error:
- use restrained error color;
- keep result expanded.

---

## 7. `FinalResponse`

Purpose:
- display user-facing result after trace.

Rules:
- readable 14px body;
- markdown supported;
- code blocks use mono font;
- avoid wrapping the entire response in a decorative card;
- optional small result/verification block may use a bordered surface.

---

## 8. `ChangedFilesSummary`

Inline collapsed summary:

```text
▣ 7 files changed       +737  -46               View files →
```

This is the bridge from conversation to code inspection.

Click:
- open Inspector Files section.

---

## 9. `Inspector`

Modes:

```text
rail
expanded
```

Expanded width:
- 330px default.

Sections:
- Summary
- Files
- Tools
- Terminal
- LLM

The section navigation may be tabs or vertical icons.

An earlier revision of this document forbade both at once. **That rule is
withdrawn**: reference image `01-conversation-main.png` shows the tab row
(`Inspector` / `Terminal`) and the icon rail together, and the reference image is
the source of truth for appearance. The two are not redundant — the tabs select
the panel, the rail selects the section within it.

Inspector state must be independent from conversation scroll.

Each section card collapses from its header. The chevron is drawn identically in
both states; `aria-expanded` carries the state.

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
- V1 may use a modal or Inspector subview.

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

Minimum structure:

```text
[ attach ] Message Kodo...          [ model ▼ ] [ send ]
```

It is a controlled `<textarea rows={1}>` that grows with its content up to about
160px, then scrolls internally. Enter sends; Shift+Enter inserts a newline.

States:

| State | Trigger | Composer |
|---|---|---|
| idle | draft empty | send disabled |
| ready | draft non-empty, nothing running | send enabled |
| generating | a run is in flight | send replaced by **stop** |
| disabled | no conversation open | input and send both inert |
| approval pending | reserved for the Agent Loop | not implemented in this slice |

A run that is over must not leave the composer claiming to be generating, and a
run that was killed must not leave it claiming to be finished.

Do not add many tool toggles to the composer in V1.

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
