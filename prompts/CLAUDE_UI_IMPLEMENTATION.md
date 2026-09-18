# Claude Code Goal — Implement Kodo GUI Faithfully

Implement Kodo GUI V1 from the approved design references.

This is **not** a redesign task.

First read:

- `CLAUDE.md`
- all files under `docs/design/`
- `skills/kodo-ui/SKILL.md`

Then inspect:

- `docs/design/references/01-conversation-main.png`
- `docs/design/references/02-response-trace.png`
- `docs/design/references/03-archive.png`
- `docs/design/references/04-settings.png`

## Product model

Kodo is intentionally simple:

```text
local project
  → conversations
    → user message
    → agent execution trace
    → final answer
    → changed files
```

The left sidebar must show project folders, and clicking a project directly expands its conversations below it.

Do **not** create a second project-conversation sidebar.

## Implement V1 only

Implement:

1. single left ProjectTree;
2. project expand/collapse;
3. conversation switch/create/archive;
4. conversation page;
5. inline assistant execution trace;
6. final response;
7. changed-files summary;
8. collapsible right Inspector;
9. detailed Response Trace;
10. Archive;
11. Settings.

Do not add unrelated functionality.

## Visual requirement

The reference image is the visual source of truth.

Primary viewport:

```text
1586 × 992
```

Kodo must look like a compact modern IDE:
- small typography;
- high information density;
- restrained borders;
- very few decorative cards;
- no oversized empty areas.

## Development method

Do not implement all pages blindly in one pass.

Work page by page in this order:

1. Conversation page with Inspector open
2. Inspector collapsed rail
3. Response Trace
4. Archive
5. Settings

For each page:

1. inspect reference;
2. explain the major component structure;
3. implement;
4. run the app;
5. load deterministic fixture;
6. capture screenshot at 1586×992;
7. compare to reference;
8. correct visual differences;
9. repeat;
10. only then continue.

Before writing code, first output:
- the files/components you will reuse;
- the smallest component structure you need;
- the order you will implement the pages.

Keep the code minimal and maintainable. Do not create architecture for hypothetical future requirements.
