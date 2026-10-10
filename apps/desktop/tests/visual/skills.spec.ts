import { expect, test } from "@playwright/test";
import { project, sessionRef, stubShell, switchMode } from "./shell";

/** One code-space user skill, mirroring the app's demo fixture shape. */
const changelog = {
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
  guidance: "Write the changelog for readers, not for git.",
  markdown:
    "# Skill: changelog\n\n## description\nGroup the changes by audience into dated changelog entries\n\n## applicable_task_types\n- docs\n\n## context_strategy\ndocs-focused\n\n## allowed_tools\n- search\n- read_file\n- write_file\n- list_files\n\n## verification_policy\nnone\n\n## workflow\n- s1 | read | Read the commits since the last tag\n- s2 | edit | Group the changes by audience\n- s3 | read | Review the wording and dates\n\n## completion_criteria\n- Every user-facing change appears exactly once\n- Entries are grouped by audience and dated\n\n## guidance\nWrite the changelog for readers, not for git.\n",
};

/** One project-scope skill for the 项目技能 card. */
const releaseCheck = {
  name: "release-check",
  source: "project",
  description: "Walk the pre-release checklist for the selected project",
  taskTypes: ["docs"],
  strategy: "docs-focused",
  policy: "none",
  workflow: [{ id: "s1", kind: "read", title: "Read the release notes" }],
  allowedTools: ["search", "read_file"],
  completionCriteria: ["The checklist is complete"],
  guidance: "Check the release before shipping.",
  markdown:
    "# Skill: release-check\n\n## description\nWalk the pre-release checklist for the selected project\n\n## applicable_task_types\n- docs\n\n## context_strategy\ndocs-focused\n\n## allowed_tools\n- search\n- read_file\n\n## verification_policy\nnone\n\n## workflow\n- s1 | read | Read the release notes\n\n## completion_criteria\n- The checklist is complete\n\n## guidance\nCheck the release before shipping.\n",
};

/** Parsed `read_skill` payload for the builtin row (builtins live in this map). */
const bugFixDetail = {
  name: "bug-fix",
  source: "builtin",
  description: "Reproduce first, fix second",
  taskTypes: ["bug-fix"],
  strategy: "repro-first",
  policy: "full",
  workflow: [
    { id: "s1", kind: "command", title: "Reproduce the failure" },
    { id: "s2", kind: "edit", title: "Apply the fix" },
    { id: "s3", kind: "verify", title: "Run the tests" },
  ],
  allowedTools: ["search", "read_file", "write_file", "run_command"],
  completionCriteria: ["The bug is fixed", "The tests pass"],
  guidance: "Reproduce first, fix second.",
  markdown: "# Skill: bug-fix\n\n## applicable_task_types\n- bug-fix\n",
};

const UPDATED_MARKDOWN = [
  "# Skill: changelog",
  "",
  "## description",
  "Group the changes by audience",
  "",
  "## applicable_task_types",
  "- docs",
  "",
  "## context_strategy",
  "docs-focused",
  "",
  "## allowed_tools",
  "- search",
  "",
  "## verification_policy",
  "none",
  "",
  "## workflow",
  "- s1 | read | Read the changes",
  "",
  "## completion_criteria",
  "- Updated entries",
  "",
  "## guidance",
  "更新后的说明 marker",
  "",
].join("\n");

function skillOps(page: import("@playwright/test").Page) {
  return page.evaluate(
    () =>
      (window as unknown as { __skillOps?: Array<{ op: string; name: string }> }).__skillOps ?? [],
  );
}

/** A minimal valid skill the project-card create flow fills in. */
const DEPLOY_GATE_MARKDOWN = [
  "# Skill: deploy-gate",
  "",
  "## description",
  "Gate deploys on the project checklist",
  "",
  "## applicable_task_types",
  "- docs",
  "",
  "## context_strategy",
  "docs-focused",
  "",
  "## allowed_tools",
  "- search",
  "",
  "## verification_policy",
  "none",
  "",
  "## workflow",
  "- s1 | read | Read the checklist",
  "",
  "## completion_criteria",
  "- The gate passed",
  "",
].join("\n");

test("builtin row opens a read-only detail with workflow rows", async ({ page }) => {
  await stubShell(page, { skillDetails: { "bug-fix": bugFixDetail } });
  await page.goto("/ui-demo/plugins");

  await page.locator('.skill-row[data-skill-id="bug-fix"]').click();
  const modal = page.locator('[data-testid="skill-detail"]');
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("缺陷修复");
  await expect(page.locator(".skill-detail-source")).toHaveText("内置");
  await expect(page.locator(".skill-detail-steps li")).toHaveCount(3);
  await expect(page.locator(".skill-detail-md")).toContainText("# Skill: bug-fix");
  // Builtins are read-only: no edit/delete affordance.
  await expect(page.locator('[data-testid="skill-detail-edit"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="skill-detail-delete"]')).toHaveCount(0);

  await page.locator('[data-testid="skill-detail-close"]').click();
  await expect(modal).toHaveCount(0);

  // The completed workflow skill shows up in the work space list.
  await switchMode(page, "work");
  await expect(page.locator('.skill-row[data-skill-id="workflow"]')).toContainText("工作流自动化");
});

test("new skill: inline validation, then a valid save lands in the user section", async ({ page }) => {
  await stubShell(page, { skills: [] });
  await page.goto("/ui-demo/plugins");

  await page.locator('[data-testid="skill-new"]').click();
  await expect(page.locator('[data-testid="skill-detail"]')).toBeVisible();

  // Frontend pre-checks fire before any invoke.
  await page.locator('[data-testid="skill-detail-save"]').click();
  await expect(page.locator('[data-testid="skill-detail-error"]')).toHaveText("请填写技能名");
  await page.locator('[data-testid="skill-name"]').fill("bad name");
  await page.locator('[data-testid="skill-detail-save"]').click();
  await expect(page.locator('[data-testid="skill-detail-error"]')).toContainText("技能名仅限");
  await page.locator('[data-testid="skill-name"]').fill("my-notes");
  await page.locator('[data-testid="skill-markdown"]').fill("");
  await page.locator('[data-testid="skill-detail-save"]').click();
  await expect(page.locator('[data-testid="skill-detail-error"]')).toHaveText("请填写技能内容");
  expect(await skillOps(page)).toEqual([]);

  await page.locator('[data-testid="skill-markdown"]').fill(
    "# Skill: my-notes\n\n## description\n随手记笔记\n\n## applicable_task_types\n- docs\n\n## context_strategy\ndocs-focused\n\n## allowed_tools\n- search\n\n## verification_policy\nnone\n\n## workflow\n- s1 | read | Read the inputs\n\n## completion_criteria\n- Done\n\n## guidance\n随手记。\n",
  );
  await page.locator('[data-testid="skill-detail-save"]').click();
  await expect(page.locator('[data-testid="skill-detail"]')).toHaveCount(0);
  await expect(page.locator('[data-user-skill-id="my-notes"]')).toBeVisible();
  expect(await skillOps(page)).toEqual([{ op: "save", name: "my-notes" }]);
});

test("backend save errors surface inline without closing the editor", async ({ page }) => {
  await stubShell(page, { skills: [], failSaveSkill: "技能内容解析失败：缺少 ## workflow" });
  await page.goto("/ui-demo/plugins");

  await page.locator('[data-testid="skill-new"]').click();
  await page.locator('[data-testid="skill-name"]').fill("broken");
  await page.locator('[data-testid="skill-detail-save"]').click();

  await expect(page.locator('[data-testid="skill-detail-error"]')).toHaveText(
    "技能内容解析失败：缺少 ## workflow",
  );
  await expect(page.locator('[data-testid="skill-detail"]')).toBeVisible();
  await expect(page.locator(".user-skill-row")).toHaveCount(0);
  expect(await skillOps(page)).toEqual([]);
});

test("edit round-trips content and the row delete is a two-step confirm", async ({ page }) => {
  await stubShell(page, { skills: [changelog] });
  await page.goto("/ui-demo/plugins");

  // Edit round-trip.
  await page.locator('[data-testid="skill-edit-changelog"]').click();
  await expect(page.locator('[data-testid="skill-name"]')).toHaveValue("changelog");
  await expect(page.locator('[data-testid="skill-markdown"]')).toContainText("# Skill: changelog");
  await page.locator('[data-testid="skill-markdown"]').fill(UPDATED_MARKDOWN);
  await page.locator('[data-testid="skill-detail-save"]').click();
  await expect(page.locator('[data-testid="skill-detail"]')).toHaveCount(0);
  expect(await skillOps(page)).toEqual([{ op: "save", name: "changelog" }]);

  // Reopen: the saved content is what the preview shows.
  await page.locator(".user-skill-open").click();
  await expect(page.locator(".skill-detail-md")).toContainText("更新后的说明 marker");
  await page.locator('[data-testid="skill-detail-close"]').click();

  // Two-step delete on the row.
  const remove = page.locator('[data-testid="skill-delete-changelog"]');
  await remove.click();
  await expect(remove).toContainText("确认删除？");
  await expect(page.locator(".user-skill-row")).toHaveCount(1);
  await remove.click();
  await expect(page.locator(".user-skill-row")).toHaveCount(0);
  expect(await skillOps(page)).toEqual([
    { op: "save", name: "changelog" },
    { op: "delete", name: "changelog" },
  ]);
});

test("user section shows the empty state when the space has no skills", async ({ page }) => {
  await stubShell(page, { skills: [] });
  await page.goto("/ui-demo/plugins");

  await expect(page.locator('[data-testid="skills-empty"]')).toHaveText(
    "当前空间还没有自建技能。",
  );
  await expect(page.locator(".user-skill-row")).toHaveCount(0);
});

test("project card holds project skills and scopes saves to the project store", async ({ page }) => {
  await stubShell(page, { skills: [releaseCheck, changelog] });
  await page.goto("/ui-demo/plugins");

  // The demo route has an active project, so the 项目技能 card renders and each
  // store row lands in its own card via the scope-specific data hooks.
  await expect(page.locator('[aria-label="项目技能"]')).toBeVisible();
  await expect(page.locator('[data-project-skill-id="release-check"]')).toBeVisible();
  await expect(page.locator('[data-user-skill-id="changelog"]')).toBeVisible();
  await expect(page.locator('[data-project-skill-id="changelog"]')).toHaveCount(0);
  await expect(page.locator('[data-user-skill-id="release-check"]')).toHaveCount(0);

  // The card's own + button saves with scope=project: the stub stamps `source`
  // from the request scope, so the row only lands in the project card when the
  // frontend passed it.
  await page.locator('[data-testid="skill-new-project"]').click();
  await expect(page.locator('[data-testid="skill-detail"]')).toBeVisible();
  await page.locator('[data-testid="skill-name"]').fill("deploy-gate");
  await page.locator('[data-testid="skill-markdown"]').fill(DEPLOY_GATE_MARKDOWN);
  await page.locator('[data-testid="skill-detail-save"]').click();
  await expect(page.locator('[data-testid="skill-detail"]')).toHaveCount(0);
  await expect(page.locator('[data-project-skill-id="deploy-gate"]')).toBeVisible();
  await expect(page.locator('[data-user-skill-id="deploy-gate"]')).toHaveCount(0);
  expect(await skillOps(page)).toEqual([{ op: "save", name: "deploy-gate" }]);
});

test("project card shows its empty state when the project has no skills", async ({ page }) => {
  await stubShell(page, { skills: [] });
  await page.goto("/ui-demo/plugins");

  await expect(page.locator('[data-testid="skills-project-empty"]')).toHaveText(
    "当前项目还没有项目技能。",
  );
});

test("composer offers the user skill in + and /, then serializes its chip", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")],
    },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [],
    },
    providers: [{ id: "p1", name: "Local", template: "openai", apiKey: "••••dead", endpoint: "", model: "gpt-test" }],
    settings: { "active-provider": "0" },
    skills: [changelog],
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("修复路由");

  // Slash discovery.
  await page.locator('[data-testid="composer-input"]').fill("/changelog");
  await expect(page.locator('[data-slash-id="changelog"]')).toBeVisible();
  await page.locator('[data-testid="composer-input"]').fill("");

  // + panel discovery and selection.
  await page.locator('.composer button[aria-label="添加内容"]').click();
  await page.locator(".composer-plus-menu").getByRole("menuitem", { name: "技能" }).click();
  await page.locator('.composer-plus-panel [data-skill-id="changelog"]').click();
  await expect(page.locator('.composer-skill[data-skill-id="changelog"]')).toContainText("changelog");

  await page.locator('[data-testid="composer-input"]').fill("整理变更日志");
  await page.locator('[data-testid="composer-input"]').press("Enter");
  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string }> }).__sendLog ?? [],
  );
  expect(log[0]?.text).toBe("【技能：changelog】\n整理变更日志");
});
