# Skill: refactor

## applicable_task_types
- refactor

## context_strategy
minimal-change

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
- s1 | read | Understand the current structure and its callers
- s2 | edit | Apply the refactor with the smallest coherent diff
- s3 | verify | Run project verification to prove behavior holds

## completion_criteria
- The restructuring was applied to project files
- External behavior stays intact
- Verification commands pass

## guidance
Refactor in small, reviewable steps. Prefer moving/renaming over rewriting. No feature work and no drive-by fixes in the same change. Verification must pass because a refactor that breaks tests is not done.
