# Kodo UI Skill

Use this skill whenever implementing or modifying Kodo GUI.

## Objective

Faithfully reproduce the approved Kodo UI.

The goal is implementation fidelity, not redesign.

## Before coding

Read:

- `CLAUDE.md`
- `docs/design/DESIGN.md`
- `docs/design/LAYOUT.md`
- `docs/design/COMPONENTS.md`
- `docs/design/UI_ACCEPTANCE.md`
- `docs/design/DEMO_DATA.md`

Inspect the target image in:

- `docs/design/references/`

State which reference image you are implementing.

## Analyze the image

Before changing code, identify:

- outer layout;
- panel dimensions;
- content hierarchy;
- row density;
- typography;
- repeated components;
- visible interaction states;
- Inspector state;
- scroll boundaries.

Do not start by styling individual controls.

## Reuse first

Search existing UI components before creating new ones.

Reuse:
- layout primitives;
- buttons;
- inputs;
- icons;
- trace rows;
- Inspector sections.

Do not introduce an abstraction for one use case.

## Implementation order

Implement visual fidelity in this order:

1. shell / major columns;
2. sidebar and project tree;
3. conversation flow;
4. trace rows;
5. Inspector;
6. composer;
7. typography;
8. colors/borders/icons.

## CSS rules

Prefer:
- CSS Grid for app shell;
- Flexbox for rows/toolbars;
- CSS variables;
- semantic class names;
- minimal DOM nesting.

Avoid:
- absolute positioning for normal layout;
- arbitrary one-off pixel values when a token exists;
- excessive wrappers;
- duplicated responsive logic.

## Verification

Run the deterministic demo state.

Use viewport:

```text
1586 × 992
```

Capture screenshot.

Compare it with the reference.

Repeat until `UI_ACCEPTANCE.md` passes.

Never report UI completion based only on successful build/typecheck.

## Regression safety

After changing a shared component:
- re-run visual screenshots for all pages that use it;
- do not fix one screen by breaking another.

## Minimalism

If two solutions match the design equally well, choose:
- fewer components;
- fewer state layers;
- fewer dependencies;
- fewer CSS rules;
- easier-to-understand code.
