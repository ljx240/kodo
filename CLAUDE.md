# Kodo Development Rules

Kodo is a minimal coding agent.

The product goal is not to build a generic admin dashboard or a full IDE.  
The GUI exists to make project-scoped coding conversations, agent execution, code changes, and verification easy to understand.

## Mandatory UI references

Before changing any GUI code, read:

- `docs/design/DESIGN.md`
- `docs/design/LAYOUT.md`
- `docs/design/COMPONENTS.md`
- `docs/design/UI_ACCEPTANCE.md`
- `docs/design/DEMO_DATA.md`

Visual references are in:

- `docs/design/references/`

Reference screenshots are the **visual source of truth**.

When text documentation and a reference image disagree:
- geometry, interaction, responsive behavior and hidden states: follow the docs;
- visual composition, hierarchy, density and appearance: follow the image.

## Do not redesign

The task is faithful implementation, not creative interpretation.

Do not:
- invent additional navigation levels;
- add dashboard widgets;
- add decorative cards;
- add a second project/conversation column;
- add features that are not required by the current task;
- convert compact IDE-like content into large consumer-style cards;
- create abstractions unless they remove real duplication.

## Core layout rule

The left sidebar is a single tree:

```text
Projects
├─ realtime-lakehouse
│  ├─ 修复 k2k-rust 未知表路由
│  ├─ 优化实时数仓搭建流程
│  └─ 排查 Flink 任务延迟问题
├─ chixiao
├─ realtime
├─ zhike
└─ insursight
```

Correct:

```text
Sidebar(Project + nested conversations) | Conversation | Optional Inspector
```

Incorrect:

```text
Sidebar | Project conversations | Conversation | Inspector
```

There must never be a permanent second conversation sidebar.

## UI implementation workflow

For every screen:

1. inspect the corresponding reference image;
2. read the relevant design rules;
3. inspect existing reusable components;
4. implement the smallest correct change;
5. run Kodo;
6. open the deterministic demo route/state;
7. capture a screenshot at the reference viewport;
8. compare against the reference;
9. fix differences in this order:
   - overall layout;
   - panel widths/heights;
   - spacing;
   - typography;
   - borders/backgrounds;
   - icons;
   - micro details;
10. repeat until `UI_ACCEPTANCE.md` passes.

A page is not complete because it compiles.

## Code quality

Prefer:
- simple React components;
- CSS Grid/Flexbox;
- CSS variables/tokens;
- composition;
- local state where sufficient;
- one source of truth for panel state.

Avoid:
- UI framework abstractions created for one screen;
- unnecessary context providers;
- event buses;
- duplicate component variants;
- premature design-system complexity;
- deeply nested wrappers with no visual or behavioral purpose.

If one implementation is simpler and equally faithful, choose the simpler implementation.
