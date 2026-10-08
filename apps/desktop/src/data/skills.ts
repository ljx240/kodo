/**
 * Builtin skill lists per agent space, mirrored from `skills/<id>/SKILL.md`
 * (runtime SkillRegistry). Plain display lists for the picker and the Skills
 * page — selection itself happens in the backend (`run()` resolves the
 * `【技能：id】` chip); these only label the rows the user clicks.
 */

import type { SkillView } from "../api";

export type Skill = { id: string; label: string; detail?: string };

/** Code agent: the coding workflow skills. */
export const CODE_SKILLS: Skill[] = [
  { id: "bug-fix", label: "缺陷修复", detail: "先复现、再修复、后验证的缺陷处理流程" },
  { id: "feature", label: "功能开发", detail: "先明确验收标准，再实现并验证的功能开发流程" },
  { id: "test", label: "测试", detail: "先理解测试惯例，再补齐用例并跑通测试套件" },
  { id: "refactor", label: "重构", detail: "保持最小差异的结构重构，验证行为不变" },
  { id: "code-review", label: "代码评审", detail: "只读评审，给出问题与建议，不改动代码" },
  { id: "docs", label: "文档", detail: "轻量的文档更新流程，不运行完整编译" },
  { id: "kodo-ui", label: "Kodo UI", detail: "界面实现专用：忠实还原设计稿与参考截图" },
];

/** Work agent: document/report/answer flows (see skills/work/SKILL.md). */
export const WORK_SKILLS: Skill[] = [
  { id: "doc", label: "写文档", detail: "面向交付的文档撰写与整理流程" },
  { id: "summary", label: "做总结", detail: "提炼要点、结构化输出的总结流程" },
  { id: "qa", label: "问答", detail: "直接回答问题，给出准确、可核对的结论" },
  { id: "workflow", label: "工作流自动化", detail: "多步命令串联、批量文件操作与逐步汇报的自动化流程" },
];

/** The skill list belonging to an agent space; anything unknown is code. */
export function skillsFor(mode: string): Skill[] {
  return mode === "work" ? WORK_SKILLS : CODE_SKILLS;
}

/** Which space a skill belongs to, from its task types: a `work` type puts it
 *  in the work space; everything else is code. Shared by the Skills page and
 *  the composer so the two lists can never disagree. */
export function skillSpace(taskTypes: string[]): "work" | "code" {
  return taskTypes.includes("work") ? "work" : "code";
}

/**
 * Demo fixture for `/ui-demo/skills` and the browser (no Tauri): one code-space
 * user skill so the 自建技能 section renders with real content. The desktop
 * shell gets its user skills from `list_skills` instead.
 */
export const DEMO_USER_SKILLS: SkillView[] = [
  {
    name: "changelog",
    source: "user",
    description: "Group the changes by audience into dated changelog entries",
    taskTypes: ["docs"],
    strategy: "docs-focused",
    policy: "none",
    workflow: [
      { id: "s1", kind: "read", title: "Read the commits since the last tag" },
      { id: "s2", kind: "edit", title: "Group the changes by audience" },
      { id: "s3", kind: "read", title: "Review the wording and dates" },
    ],
    allowedTools: ["search", "read_file", "write_file", "list_files"],
    completionCriteria: [
      "Every user-facing change appears exactly once",
      "Entries are grouped by audience and dated",
    ],
    guidance:
      "Write the changelog for readers, not for git: group by audience, one line per change, dates in ISO form.",
    markdown:
      "# Skill: changelog\n\n## description\nGroup the changes by audience into dated changelog entries\n\n## applicable_task_types\n- docs\n\n## context_strategy\ndocs-focused\n\n## allowed_tools\n- search\n- read_file\n- write_file\n- list_files\n\n## verification_policy\nnone\n\n## workflow\n- s1 | read | Read the commits since the last tag\n- s2 | edit | Group the changes by audience\n- s3 | read | Review the wording and dates\n\n## completion_criteria\n- Every user-facing change appears exactly once\n- Entries are grouped by audience and dated\n\n## guidance\nWrite the changelog for readers, not for git: group by audience, one line per change, dates in ISO form.\n",
  },
];

/**
 * Demo fixture for the 项目技能 card under `/ui-demo/skills`: one project-scope
 * skill as `<project>/.kodo/skills` would carry it (team-shared, shadowing a
 * same-named user skill at run time).
 */
export const DEMO_PROJECT_SKILLS: SkillView[] = [
  {
    name: "release-check",
    source: "project",
    description: "Run the project's release checklist before tagging",
    taskTypes: ["docs"],
    strategy: "docs-focused",
    policy: "none",
    workflow: [
      { id: "s1", kind: "read", title: "Read the release checklist" },
      { id: "s2", kind: "command", title: "Run the checklist commands" },
      { id: "s3", kind: "read", title: "Record the outcomes" },
    ],
    allowedTools: ["search", "read_file", "list_files", "run_command"],
    completionCriteria: ["Every checklist item has a recorded outcome"],
    guidance: "Never skip an item; record skips with a reason.",
    markdown:
      "# Skill: release-check\n\n## description\nRun the project's release checklist before tagging\n\n## applicable_task_types\n- docs\n\n## context_strategy\ndocs-focused\n\n## allowed_tools\n- search\n- read_file\n- list_files\n- run_command\n\n## verification_policy\nnone\n\n## workflow\n- s1 | read | Read the release checklist\n- s2 | command | Run the checklist commands\n- s3 | read | Record the outcomes\n\n## completion_criteria\n- Every checklist item has a recorded outcome\n\n## guidance\nNever skip an item; record skips with a reason.\n",
  },
];
