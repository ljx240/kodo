# Skill: kodo-ui

## description
Implement or modify Kodo GUI with fidelity to the approved design — reproduce it, do not redesign it.

## applicable_task_types
- feature
- bug-fix
- refactor

## context_strategy
acceptance-first

## allowed_tools
- search
- read_file
- list_files
- find_symbol
- find_references
- read_range
- run_command
- write_file
- apply_patch
- replace_range
- create_file

## allowed_external_tools
- mcp__*

## verification_policy
full

## workflow
- s1 | read | Read CLAUDE.md and docs/design DESIGN, LAYOUT, COMPONENTS, UI_ACCEPTANCE, DEMO_DATA
- s2 | read | Analyse the screen: outer layout, panel dimensions, hierarchy, row density, typography, states, scroll boundaries
- s3 | read | Search existing components and confirm which ones can be reused
- s4 | edit | Implement in order: shell/columns, sidebar and project tree, conversation flow, trace rows, Inspector, composer, typography, colors and borders
- s5 | verify | Run the deterministic demo state at 1586 x 992, capture a screenshot and check it against UI_ACCEPTANCE.md

## completion_criteria
- The demo-state screenshot passes every `UI_ACCEPTANCE.md` check
- Existing layout primitives and components were reused; no abstraction was introduced for a single use case
- Every page sharing a changed component was re-screenshotted
- Completion was not reported on a successful build or typecheck alone

## guidance
Faithful reproduction, not redesign. Do not start by styling individual controls — settle the outer layout first.

Reuse before creating: layout primitives, buttons, inputs, icons, trace rows, Inspector sections. Never introduce an abstraction for one use case.

CSS: prefer Grid for the app shell, Flexbox for rows and toolbars, CSS variables and semantic class names, minimal DOM nesting. Avoid absolute positioning for normal layout, one-off pixel values where a token exists, excessive wrappers, and duplicated responsive logic.

If two solutions match the design equally well, choose fewer components, fewer state layers, fewer dependencies, fewer CSS rules and code that is easier to understand.
