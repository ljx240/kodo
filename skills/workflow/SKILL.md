# Skill: workflow

## description
Execute an explicit chained-command workflow step by step and record each outcome.

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
- s1 | read | Gather the inputs and understand the requested workflow
- s2 | command | Execute the chained commands step by step and record each outcome
- s3 | edit | Apply the batch file operations and cross-file updates
- s4 | read | Review the changed files and report the step-by-step outcome

## completion_criteria
- Every step's outcome is reported in order
- The workflow stops at the first unexpected failure
- Any files written stay inside the selected folder

## guidance
Workflow tasks are multi-step automations: batch file operations, chained
run_command calls, cross-file updates, and step-by-step outcome reports.
Report every step's outcome as it completes; stop at the first unexpected
failure instead of improvising onward; keep every file inside the selected
folder. File tools and commands remain subject to the Permission mode. Do not
run project test suites or builds as acceptance — an honest step-by-step
report is the deliverable.
