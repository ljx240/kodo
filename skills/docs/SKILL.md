# Skill: docs

## applicable_task_types
- docs

## context_strategy
docs-focused

## allowed_tools
- search
- read_file
- write_file
- apply_patch
- replace_range
- create_file

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
