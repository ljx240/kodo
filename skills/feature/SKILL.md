# Skill: feature

## applicable_task_types
- feature

## context_strategy
acceptance-first

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
- s1 | read | Extract acceptance criteria and constraints from the task
- s2 | read | Locate the integration points in the existing code
- s3 | edit | Implement the feature
- s4 | verify | Run project verification

## completion_criteria
- Acceptance criteria were identified from the task before editing
- The feature code was written inside the project
- Verification commands pass

## guidance
Start by writing down concrete, checkable acceptance criteria derived from the user's request. Reuse existing components before adding new ones. Do not claim done until every acceptance criterion is backed by tool evidence and verification passes.
