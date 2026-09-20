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
- [ ] conversation is the largest visual region;
- [ ] Inspector can be opened and collapsed;
- [ ] collapsed Inspector becomes a narrow rail;
- [ ] main conversation expands when Inspector collapses;
- [ ] composer remains usable at bottom;
- [ ] at least 6–8 compact trace rows can be visible at reference viewport.

---

## 4. Trace checks

- [ ] trace belongs to a specific assistant response;
- [ ] running state is visible;
- [ ] each step has action + detail + duration/status;
- [ ] command/tool output can expand;
- [ ] completed long output is not permanently expanded;
- [ ] failed output is prominent enough to diagnose;
- [ ] final answer is visually distinct from execution trace;
- [ ] user can open detailed Response Trace.

---

## 5. Inspector checks

- [ ] default expanded width approximately 330px;
- [ ] supports Summary;
- [ ] supports Changed files;
- [ ] supports Tools used;
- [ ] supports LLM calls;
- [ ] supports Terminal/output when relevant;
- [ ] each section uses compact rows;
- [ ] panel does not duplicate all timeline detail;
- [ ] collapsed rail remains available.

---

## 5a. Sidebar checks

- [ ] no account row, avatar or sign-out anywhere in the sidebar;
- [ ] `Conversations` and `Archive` are the only top-level nav rows;
- [ ] `Settings` sits at the foot, below the scrollable tree;
- [ ] `+` beside `Projects` offers "add existing folder" and "new project";
- [ ] a project row's `⋯` offers rename, reveal in Finder, remove from list;
- [ ] renaming a project does not rename the directory;
- [ ] removing a project does not delete the directory;
- [ ] dragging a project row reorders the list.

---

## 5b. Settings checks

- [ ] left column lists all six categories;
- [ ] selecting a category swaps the right-hand pane;
- [ ] no category opens onto an empty pane;
- [ ] the selected category is visibly marked;
- [ ] the page head (title, subtitle, search) is unchanged;
- [ ] the `Default model` control and the top bar's model chip show the same value;
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

These are asserted in `apps/desktop/tests/visual/behaviour.spec.ts`. A screenshot
cannot check any of them.

- [ ] clicking anywhere along a trace row opens it, not only the chevron;
- [ ] every trace row reports `aria-expanded`;
- [ ] the keyboard reaches a trace row (Enter and Space);
- [ ] a step that starts with no output opens when it completes, and stays open;
- [ ] composer sends on Enter, clears, and leaves Shift+Enter for newlines;
- [ ] a running composer shows stop, not send;
- [ ] a finished reply does not claim to be working;
- [ ] a killed run reports as interrupted, never as finished or working;
- [ ] a failover event shows failed provider/model, error class, and next model;
- [ ] the Response Trace tabs each swap the pane;
- [ ] an Inspector card collapses and restores from its header.

---

## 6. Density checks

Reject the implementation if:
- content is enlarged merely to fill space;
- there are large empty hero sections;
- every block is a rounded card;
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
5. compare with reference;
6. correct large geometry first;
7. repeat.

### Snapshots are a net, not a spec

The five Playwright baselines were **re-generated** after the project-management
change, because that change deliberately altered every screen that contains a
sidebar (`docs/design/references/README.md` explains why the reference PNGs no
longer match).

The consequence, stated plainly: the snapshot suite can now only catch
*unintended* change. It can no longer catch *design drift*, because whatever the
app renders today is what it was told to expect. Only the reference images and
the checks above hold the design.

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
