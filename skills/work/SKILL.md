# Skill: work

## description
Produce a work deliverable (documents, summaries, file organization) with no code verification.

## applicable_task_types
- work

## context_strategy
work-focused

## allowed_tools
- search
- read_file
- write_file
- apply_patch
- replace_range
- create_file
- delete_file
- list_files
- run_command
- websearch

## verification_policy
none

## workflow
- s1 | read | Understand the request and gather the relevant material
- s2 | edit | Draft or update the deliverable (documents, summaries, files)
- s3 | read | Review the deliverable for accuracy and completeness

## completion_criteria
- The deliverable answers the user's request completely
- Any files written live inside the selected folder
- The answer summarizes what was produced and what remains open

## guidance
Work tasks are general-purpose: documents, summaries, planning, Q&A, and everyday writing — not code-only. File tools stay inside the selected folder; every write and command still requires whatever approval the permission mode demands. Do not run project test suites or builds as acceptance — completeness is the deliverable itself. Without a selected folder the tool list is empty: answer directly from the request and attached files.
