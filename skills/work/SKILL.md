# Skill: work

## description
Produce a work deliverable (documents, summaries, file organization) with no code verification.

## applicable_task_types
- work

## context_strategy
work-focused

## allowed_tools
- subagent
- search
- read_file
- office_read
- office_write
- write_file
- apply_patch
- replace_range
- create_file
- delete_file
- list_files
- run_command
- websearch

## allowed_external_tools
- mcp__*

## verification_policy
none

## workflow
- s1 | read | Understand the request and gather the relevant material
- s2 | edit | Draft or update the deliverable (documents, summaries, files)
- s3 | read | Review the deliverable for accuracy and completeness

## completion_criteria
- The deliverable answers the user's request completely
- File deliverables use a location selected or approved by the user
- The answer summarizes what was produced and what remains open

## guidance
Work tasks are general-purpose: documents, summaries, planning, Q&A, and everyday writing — not code-only. File tools stay inside the selected folder; every write and command still requires whatever approval the permission mode demands. Do not run project test suites or builds as acceptance — completeness is the deliverable itself. Without a selected folder, use the request and attached files. Office tools can read selected Office attachments with user approval and prepare desktop drafts for review.

### Office in Work

- Take Excel analysis, Word authoring or revision, and PowerPoint design requests from the conversation. The right File sidebar previews attachments, drafts, and saved results.
- For XLSX, use the local analysis option for numeric summaries and grouping. Cite the sheet and A1 range, state units, and identify cached or unavailable formulas. Use editable cells, formulas, and charts when an analysis workbook is requested.
- For DOCX, draft a document with a clear heading hierarchy and suitable tables. For revisions, read the original and use targeted operations with its source version.
- For PPTX, organize the story around the audience's decision. Use a concise conclusion, editable metrics or charts where they add evidence, and actions with owners or time frames when provided. Preserve the source template when revising it.
- On desktop, present prepared drafts for sidebar review and saving. Describe the actual delivery state.
