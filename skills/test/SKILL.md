# Skill: test

## description
Add or repair tests for existing behavior under a tests-only verification policy.

## applicable_task_types
- test

## context_strategy
test-first

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
tests-only

## workflow
- s1 | read | Locate the code under test and existing test conventions
- s2 | edit | Add or extend the tests
- s3 | verify | Run the relevant test suite

## completion_criteria
- Tests were written for the targeted behavior
- A test command ran successfully against the new tests
- Verification reports the test suite passing

## guidance
Follow the repository's existing test layout and naming. Tests must exercise real behavior, not trivial assertions. Full builds are unnecessary — running the relevant tests is the verification that matters.
