# Skill: code-review

## description
Review changes read-only and report findings backed by tool evidence.

## applicable_task_types
- code-review

## context_strategy
review-only

## allowed_tools
- search
- read_file
- list_files
- find_symbol
- find_references
- read_range
- run_command

## verification_policy
none

## workflow
- s1 | read | Gather the changed or targeted files
- s2 | read | Inspect for correctness, safety, and design issues
- s3 | command | Optionally run read-only checks when helpful

## completion_criteria
- Findings cite concrete file and line evidence
- No project files were modified
- The review is grounded in tool observations, not assumptions

## guidance
Default mode is read-only: never write, patch, create, or delete files. Report findings with file:line references, ordered by severity. Running lightweight read-only commands is allowed; editing is not.
