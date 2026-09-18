# Skill: bug-fix

## applicable_task_types
- bug-fix

## context_strategy
repro-first

## allowed_tools
- search
- read_file
- run_command
- write_file
- apply_patch
- replace_range
- create_file

## verification_policy
full

## workflow
- s1 | command | Reproduce the failure with a failing test or command
- s2 | read | Locate the root cause in the source
- s3 | edit | Apply the minimal fix
- s4 | verify | Run regression tests and project verification

## completion_criteria
- The failure was reproduced before the fix
- A changed file carries the fix
- Verification commands pass

## guidance
Reproduce first: never edit before a failing case exists. Fix the root cause with the smallest change. Prefer apply_patch over full-file rewrites. A passing reproduction alone is not enough — project verification must pass before claiming done.
