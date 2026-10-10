# Skill: docs

## description
Update documentation accurately; no compile verification is required.

## applicable_task_types
- docs

## context_strategy
docs-focused

## allowed_tools
- search
- read_file
- list_files
- find_symbol
- find_references
- read_range
- write_file
- apply_patch
- replace_range
- create_file
- ask_user

## allowed_external_tools
- mcp__*

## verification_policy
none

## workflow
- s1 | read | Locate the documentation to update
- s2 | edit | Update the docs to match current behavior
- s3 | read | Re-read the edited docs for consistency

## completion_criteria
- Documentation files were written inside the project
- Docs match the behavior observed in the project
- No source code was modified beyond documentation files

## guidance
Docs tasks must not trigger full builds or test suites — verification is documentation consistency, not compilation. Match the existing documentation tone and structure. Keep edits scoped to docs, README, comments, and changelog files.
When a follow-up asks to save the preceding answer, reuse the supplied conversation instead of researching the repository again. Inspect the target, write the requested document and re-read it before claiming delivery. Keep tool calls compact and complete; use small patches for long documents. Follow repository instructions: if they block writing, use ask_user to state the exact rule and needed authorization, and keep a reviewable draft available. Changing the filename alone does not override those instructions.
