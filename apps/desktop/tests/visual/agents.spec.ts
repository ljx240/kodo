import { expect, test } from "@playwright/test";
import { project, sessionRef, stubShell } from "./shell";

/** Builds an `AgentView`-shaped fixture with the §0.1 empty defaults. The
 *  real shell populates `defaults` / `variables` / `examples`; tests that
 *  ignore those can pass a raw partial instead. */
function persona(args: {
  name: string;
  source?: "builtin" | "user" | "project";
  description: string;
  instructions: string;
  defaults?: { provider?: string; model?: string; permission?: string; skills?: string[] };
  variables?: string[];
  examples?: Array<{ input: string; output: string }>;
}) {
  return {
    name: args.name,
    source: args.source ?? ("builtin" as const),
    description: args.description,
    instructions: args.instructions,
    markdown: `# Agent: ${args.name}\n\n## description\n${args.description}\n\n## instructions\n${args.instructions}\n`,
    defaults: { skills: [], ...(args.defaults ?? {}) },
    variables: args.variables ?? [],
    examples: args.examples ?? [],
  };
}

/** One persona record in the `list_agents` / `AgentView` wire shape. */
const reviewer = persona({
  name: "reviewer",
  description: "严格代码评审",
  instructions: "先指出真正的风险与缺陷，再给建议。",
});

const explainer = persona({
  name: "explainer",
  description: "把概念讲清楚",
  instructions: "先给结论再给理由。",
});

/** Compact set covering each branch: a 12-strong list mirrors the real shell. */
const builtinTwelve = [
  reviewer, explainer,
  persona({ name: "debugger", description: "根因定位", instructions: "复现" }),
  persona({ name: "refactorer", description: "重构与清理", instructions: "单点" }),
  persona({ name: "tester", description: "测试编写", instructions: "边界" }),
  persona({ name: "security-reviewer", description: "安全审查", instructions: "复现" }),
  persona({ name: "perf-reviewer", description: "性能审查", instructions: "瓶颈" }),
  persona({ name: "documenter", description: "文档撰写", instructions: "示例" }),
  persona({ name: "planner", description: "任务拆解", instructions: "可观察" }),
  persona({ name: "api-designer", description: "API 设计", instructions: "场景" }),
  persona({ name: "translator", description: "翻译与本地化", instructions: "术语" }),
  persona({ name: "ui-designer", description: "UI 设计（kodo-aware）", instructions: "复用" }),
];

type AgentOp = { op: "save" | "delete"; name: string };

const agentsLog = (page: import("@playwright/test").Page) =>
  page.evaluate(
    () => (window as unknown as { __agentsLog?: AgentOp[] }).__agentsLog ?? [],
  ) as Promise<AgentOp[]>;

const sendLog = (page: import("@playwright/test").Page) =>
  page.evaluate(
    () =>
      (
        window as unknown as {
          __sendLog?: Array<{ text: string; agent?: string | null; agents?: string[] }>;
        }
      ).__sendLog ?? [],
  ) as Promise<Array<{ text: string; agent?: string | null; agents?: string[] }>>;

/** Conversation page + composer with one session (send-flow tests). */
async function openConversation(page: import("@playwright/test").Page, agents: unknown[]) {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "评审")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "评审",
      at: 1_700_000_000,
      archived: false,
      turns: [],
    },
    agents,
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
}

test("the agents tab lists personas and shows the empty state with none", async ({ page }) => {
  await stubShell(page, { agents: [] });
  await page.goto("/ui-demo/plugins?type=agents");

  await expect(page.locator('[data-testid="agents-empty"]')).toHaveText("还没有智能体。");

  await stubShell(page, { agents: [reviewer, explainer] });
  await page.goto("/ui-demo/plugins?type=agents");

  await expect(page.locator(".agent-row")).toHaveCount(2);
  await expect(page.locator('.agent-row[data-agent-id="reviewer"]')).toContainText("严格代码评审");
  // Builtins are read-only: no edit/delete buttons on builtin rows.
  await expect(page.locator('[data-testid="agent-edit-reviewer"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="agent-delete-reviewer"]')).toHaveCount(0);
  // Class-split invariant: persona rows never masquerade as skill rows.
  await expect(page.locator(".agent-row .skill-row")).toHaveCount(0);
  await expect(page.locator(".agent-row .user-skill-row")).toHaveCount(0);
});

test("new agent: validation ladder fires in order, a valid save lands the row", async ({ page }) => {
  await stubShell(page, { agents: [] });
  await page.goto("/ui-demo/plugins?type=agents");

  await page.locator('[data-testid="agent-new"]').click();
  await expect(page.locator('[data-testid="agent-detail"]')).toBeVisible();

  // 名称必填 → 非法名 → 描述必填 → 说明词必填, in that order.
  await page.locator('[data-testid="agent-detail-save"]').click();
  await expect(page.locator('[data-testid="agent-detail-error"]')).toHaveText("请填写智能体名");
  await page.locator('[data-testid="agent-name"]').fill("bad name");
  await page.locator('[data-testid="agent-detail-save"]').click();
  await expect(page.locator('[data-testid="agent-detail-error"]')).toContainText("智能体名仅限");
  // Non-builtin name so the validation ladder exercises field-shape rules.
  await page.locator('[data-testid="agent-name"]').fill("custom");
  await page.locator('[data-testid="agent-detail-save"]').click();
  await expect(page.locator('[data-testid="agent-detail-error"]')).toHaveText("请填写描述");
  await page.locator('[data-testid="agent-description"]').fill("用户自定义");
  await page.locator('[data-testid="agent-detail-save"]').click();
  await expect(page.locator('[data-testid="agent-detail-error"]')).toHaveText("请填写说明词");
  expect(await agentsLog(page)).toEqual([]);

  await page.locator('[data-testid="agent-instructions"]').fill("用户级 persona 说明词。");
  await page.locator('[data-testid="agent-detail-save"]').click();
  await expect(page.locator('[data-testid="agent-detail"]')).toHaveCount(0);
  await expect(page.locator('.agent-row[data-agent-id="custom"]')).toBeVisible();
  expect(await agentsLog(page)).toEqual([{ op: "save", name: "custom" }]);
});

test("editing a user agent round-trips; delete needs the second click", async ({ page }) => {
  const customAgent = persona({
    name: "custom",
    source: "user",
    description: "用户自定义",
    instructions: "原始说明词。",
  });
  await stubShell(page, { agents: [reviewer, customAgent] });
  await page.goto("/ui-demo/plugins?type=agents");

  await page.locator('[data-testid="agent-edit-custom"]').click();
  await expect(page.locator('[data-testid="agent-detail"]')).toBeVisible();
  await expect(page.locator('[data-testid="agent-name"]')).toHaveValue("custom");
  await page.locator('[data-testid="agent-description"]').fill("用户自定义（更新）");
  await page.locator('[data-testid="agent-detail-save"]').click();
  await expect(page.locator('[data-testid="agent-detail"]')).toHaveCount(0);
  await expect(page.locator('.agent-row[data-agent-id="custom"]')).toContainText(
    "用户自定义（更新）",
  );

  await page.locator('[data-testid="agent-delete-custom"]').click();
  await expect(page.locator('[data-testid="agent-delete-custom"]')).toContainText("确认删除？");
  await expect(page.locator(".agent-row")).toHaveCount(2);
  await page.locator('[data-testid="agent-delete-custom"]').click();
  await expect(page.locator(".agent-row")).toHaveCount(1);
  expect(await agentsLog(page)).toEqual([
    { op: "save", name: "custom" },
    { op: "delete", name: "custom" },
  ]);
});

test("the head action belongs to the active tab only; ?type= survives reload", async ({ page }) => {
  await stubShell(page, { agents: [reviewer] });
  await page.goto("/ui-demo/plugins");

  await expect(page.locator('[data-testid="skill-new"]')).toBeVisible();
  await expect(page.locator('[data-testid="agent-new"]')).toHaveCount(0);

  await page.locator('[data-testid="plugins-tab-mcp"]').click();
  await expect(page).toHaveURL(/\?type=mcp/);
  await expect(page.locator('[data-testid="skill-new"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="agent-new"]')).toHaveCount(0);

  await page.locator('[data-testid="plugins-tab-agents"]').click();
  await expect(page).toHaveURL(/\?type=agents/);
  await expect(page.locator('[data-testid="agent-new"]')).toBeVisible();
  await expect(page.locator('[data-testid="skill-new"]')).toHaveCount(0);

  await page.reload();
  await expect(page.locator('[data-testid="plugins-tab-agents"]')).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.locator(".agent-row")).toHaveCount(1);
});

test("the composer selector carries the agent into the send payload", async ({ page }) => {
  await openConversation(page, [reviewer]);
  await expect(page.locator('[data-testid="composer-agent-trigger"]')).toContainText("无智能体");

  await page.locator('[data-testid="composer-agent-trigger"]').click();
  await page.locator('[data-testid="composer-agent-reviewer"]').click();
  await expect(page.locator('[data-testid="composer-agent-trigger"]')).toContainText("reviewer");

  await page.locator(".composer-input").fill("评审这段代码");
  await page.locator(".composer-input").press("Enter");
  await expect
    .poll(async () => (await sendLog(page)).length)
    .toBeGreaterThan(0);
  expect((await sendLog(page))[0].agents).toEqual(["reviewer"]);
});

test("choosing 无智能体 sends a null agent; a new task starts unselected", async ({ page }) => {
  await openConversation(page, [reviewer]);

  await page.locator('[data-testid="composer-agent-trigger"]').click();
  await page.locator('[data-testid="composer-agent-none"]').click();
  await expect(page.locator('[data-testid="composer-agent-trigger"]')).toContainText("无智能体");

  await page.locator(".composer-input").fill("不带人设");
  await page.locator(".composer-input").press("Enter");
  await expect
    .poll(async () => (await sendLog(page)).length)
    .toBeGreaterThan(0);
  expect((await sendLog(page))[0].agents).toEqual([]);

  // Session-scoped: a different task does not inherit the selection.
  await page.getByRole("link", { name: "新建任务" }).click();
  await expect(page.locator('[data-testid="composer-agent-trigger"]')).toContainText("无智能体");
});

test("the agents tab lists 12 builtins with 内置 badges and no edit/delete controls", async ({ page }) => {
  await stubShell(page, { agents: builtinTwelve });
  await page.goto("/ui-demo/plugins?type=agents");

  await expect(page.locator(".agent-row")).toHaveCount(12);
  await expect(page.locator('[data-testid="agent-badge-builtin"]')).toHaveCount(12);
  await expect(page.locator('[data-testid="agent-edit-reviewer"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="agent-delete-explainer"]')).toHaveCount(0);
  // 内置 group renders before 自定义 (project may be first when a project
  // path is active — that's the §2.7 ordering, not a regression).
  const heads = await page
    .locator("section.skills-card .skills-card-head")
    .allTextContents();
  const builtinIdx = heads.findIndex((text) => text === "内置");
  const userIdx = heads.findIndex((text) => text === "自定义");
  expect(builtinIdx).toBeGreaterThanOrEqual(0);
  expect(userIdx).toBeGreaterThan(builtinIdx);
});

test("the composer agent dropdown groups 内置 / 自定义 and typeahead filters", async ({ page }) => {
  await openConversation(page, builtinTwelve);
  await page.locator('[data-testid="composer-agent-trigger"]').click();

  await expect(page.locator('[data-testid="composer-section-builtin"]')).toBeVisible();
  await expect(page.locator('[data-testid="composer-section-user"]')).toHaveCount(0);

  await page.locator('[data-testid="composer-agent-filter"]').fill("代码评审");
  await expect(page.locator('[data-testid="composer-agent-reviewer"]')).toBeVisible();
  // Filtered out: every other builtin must be gone.
  await expect(page.locator('[data-testid="composer-agent-explainer"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="composer-agent-debugger"]')).toHaveCount(0);
});

test("creating a user agent in the tab refreshes the composer dropdown", async ({ page }) => {
  await stubShell(page, {
    agents: [reviewer],
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "评审")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "评审",
      at: 1_700_000_000,
      archived: false,
      turns: [],
    },
  });
  // Single document load — the test walks the SPA's history.pushState to
  // switch between Plugins and Conversation so the stub's window-scoped
  // `agentsState` survives the round trip.
  await page.goto("/");
  await page.locator('[data-testid="composer-agent-trigger"]').click();
  await expect(page.locator('[data-testid="composer-agent-reviewer"]')).toBeVisible();

  await page.evaluate(() => {
    window.history.pushState(null, "", "/ui-demo/plugins?type=agents");
    window.dispatchEvent(new PopStateEvent("popstate"));
  });
  await page.locator('[data-testid="plugin-tab-agents"], [data-testid="plugins-tab-agents"]').click();
  await page.locator('[data-testid="agent-new"]').click();
  await page.locator('[data-testid="agent-name"]').fill("custom");
  await page.locator('[data-testid="agent-description"]').fill("用户自定义");
  await page.locator('[data-testid="agent-instructions"]').fill("用户级 persona。");
  await page.locator('[data-testid="agent-detail-save"]').click();
  await expect(page.locator(".agent-row")).toHaveCount(2);

  await page.evaluate(() => {
    window.history.pushState(null, "", "/");
    window.dispatchEvent(new PopStateEvent("popstate"));
  });
  await page.locator('[data-testid="composer-agent-trigger"]').click();
  await expect(page.locator('[data-testid="composer-agent-custom"]')).toBeVisible();
  await expect(page.locator('[data-testid="composer-section-user"]')).toBeVisible();
});

test("saving under a builtin name is rejected with zh-CN error", async ({ page }) => {
  // Drive the failure through the shell stub so the assert mirrors the
  // desktop shell's reserved-name rejection.
  await stubShell(page, {
    agents: [reviewer],
    failSaveAgent: "`reviewer` 是内置智能体名，请换一个名称",
  });
  await page.goto("/ui-demo/plugins?type=agents");
  // Open the modal via 编辑… but builtins don't expose 编辑. The next-best
  // path is invoking save directly via the head's `新建` modal — change the
  // name to "reviewer" and submit.
  await page.locator('[data-testid="agent-new"]').click();
  await page.locator('[data-testid="agent-name"]').fill("reviewer");
  await page.locator('[data-testid="agent-description"]').fill("用户改写 builtin");
  await page.locator('[data-testid="agent-instructions"]').fill("shadow");
  await page.locator('[data-testid="agent-detail-save"]').click();
  await expect(page.locator('[data-testid="agent-detail-error"]')).toContainText(
    "内置智能体名",
  );
});
