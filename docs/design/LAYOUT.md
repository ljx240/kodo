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
Main                  minmax(620px, 1fr)
Inspector expanded    330px
Inspector rail         40px
Top bar                44px
```

Inspector:
- default desktop width: 330px;
- minimum: 300px;
- maximum: 440px;
- resizable is optional for V1;
- collapse must always be supported.

When collapsed:
- keep a narrow 40px tool rail;
- main conversation immediately consumes freed width.

---

## 2. Sidebar

The sidebar is one navigation surface only.

Structure:

```text
Kodo
Search

Conversations
Archive

Projects                          +
├─ project A
│  ├─ conversation A1
│  ├─ conversation A2
│  └─ conversation A3
├─ project B
└─ project C

Settings
```

There is **no account row**. Kodo has no login, so the sidebar does not name a
user and does not offer a sign-out.

### Project behavior

Click project:
- expands nested conversations.

Click expanded project:
- collapses it.

Project row:
- 30px target height;
- trailing `⋯` opens the row menu: rename, reveal in Finder, remove from list.

Projects are **registered by hand**, never discovered by scanning a directory.
The `+` beside the `Projects` heading opens a two-item menu:

```text
添加已有文件夹…    registers an existing directory
新建项目…          creates a directory on disk, then registers it
```

Clicking a project row itself always means "expand/collapse". Registration and
creation live behind `+`; rename, reveal and remove live behind `⋯`.

Removing a project **only removes the entry**. It never touches the directory.
Renaming changes the **display name only**; the directory keeps its own name.

Reordering uses native HTML5 drag-and-drop. The drop writes the whole new order,
not "moved item N to index M" — see `docs/设计文档.md`.

Conversation row:
- 28px target height;
- indented below project;
- selected conversation uses subtle blue background;
- show compact timestamp/date on right where space allows.

Do not introduce:
- project detail screen as an intermediate step;
- second conversation column;
- permanent project-switcher panel beside sidebar.

### Long lists

Project list is independently scrollable.

Settings stays pinned at bottom, below the scrollable tree, as a single
`nav-item` row.

---

## 3. Conversation page

Structure:

```text
ConversationPage
├─ TopBar
├─ ScrollArea
│  ├─ UserMessage
│  ├─ AssistantReply
│  │  ├─ RunningState
│  │  ├─ AgentTrace
│  │  └─ FinalResponse
│  └─ ChangedFilesSummary
└─ Composer
```

Top bar:
- project / conversation breadcrumb or conversation title;
- branch when relevant;
- model selector;
- compact actions;
- Inspector toggle.

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
- max width roughly 85%.

Assistant:
- no enclosing giant card;
- avatar + name + time;
- trace and answer flow vertically.

### Composer

Pinned to bottom of main content.

Height:
- collapsed/minimum: 54–60px;
- auto-grow to about 160px before internal scrolling.

Contains:
- attachment/context button;
- prompt input;
- selected model;
- send/stop action.

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
│  ├─ Search
│  ├─ Read
│  ├─ Run command
│  ├─ Tool result
│  ├─ Model call
│  └─ Draft/finalize
└─ Final answer
```

Every assistant reply can open its own full Response Trace page/view.

Inline trace is compact:
- one line per step by default;
- output appears only for active/error/explicitly expanded items.

---

## 5. Inspector

Inspector is contextual to the current response or conversation.

Expanded layout:

```text
Inspector
├─ Header / close
├─ tabs or vertical tool rail
├─ Summary
├─ Changed files
├─ Tools used
├─ LLM calls
└─ Current project
```

Preferred sections for V1:

```text
Summary
Files
Tools
LLM
Terminal
```

Do not show all detailed data at once.

Each section card carries a chevron and **collapses**: clicking its header hides
the body, clicking again restores it. Cards with no body stay inert. The chevron
glyph is drawn the same in both states — the appearance follows the reference
image, and only `aria-expanded` reports which state the card is in.

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

The page keeps its route (`/settings`, `/ui-demo/settings`); only the inside of
the page is split. Settings is not a dialog and not a nested route.

Inspector may show Help / Information.

---

## 9. Responsive behavior

### >= 1500px
- Sidebar 260
- Main flex
- Inspector 330

### 1200–1499px
- Sidebar 240
- Inspector 300
- slightly tighter content spacing

### 900–1199px
- Sidebar 220
- Inspector defaults collapsed
- conversation owns remaining width

### < 900px
Not a primary V1 target.
Desktop application quality takes priority.
