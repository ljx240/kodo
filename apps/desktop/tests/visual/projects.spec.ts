import { expect, test } from "@playwright/test";
import { project, sessionRef, stubShell, switchMode } from "./shell";

/**
 * The GUI's half of the project contract. The core's half — that `create` really
 * makes a directory, that a name with a slash is refused — is covered by
 * `cargo test -p kodo-core`, because none of it is reachable from a browser.
 */
const PROJECTS = [project("/tmp/ws/alpha", "alpha"), project("/tmp/ws/beta", "beta")];

test("a live shell renders the projects the core listed", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    branch: "main",
  });

  await page.goto("/");

  await expect(page.locator(".tree-project-name")).toHaveText(["alpha", "beta"]);
  // The shell navigates the live routes, so every screen it reaches stays real.
  await expect(page.locator(".nav-item").first()).toHaveAttribute("href", "/conversation");
  // With no remembered project, a fresh task starts bare; the branch becomes
  // available after the user chooses one in the composer context row.
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("未选择项目");
  await expect(page.locator('[data-testid="composer-branch"]')).toHaveText("无分支");
});

test("the project picker reserves enough width for its label", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/realtime-lakehouse", "realtime-lakehouse")], sessions: [] },
    branch: "main",
  });

  await page.goto("/");
  const picker = page.locator('[data-testid="composer-project-picker"]');
  const box = await picker.boundingBox();

  expect(box).not.toBeNull();
  expect(box!.width).toBeGreaterThanOrEqual(160);
});

test("a fresh New Task revives the remembered project when it is still registered", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    branch: "main",
  });
  await page.addInitScript(() => {
    window.localStorage.setItem("kodo:last-project", "/tmp/ws/alpha");
  });

  await page.goto("/");

  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("alpha");
  await expect(page.locator('[data-testid="composer-branch"]')).toHaveText("main");
});

test("a remembered project outside the workspace is ignored", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    branch: "main",
  });
  await page.addInitScript(() => {
    window.localStorage.setItem("kodo:last-project", "/tmp/ws/gone");
  });

  await page.goto("/");

  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("未选择项目");
  await expect(page.locator('[data-testid="composer-branch"]')).toHaveText("无分支");
});

test("sessions hang under the project they belong to", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: PROJECTS,
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由"), sessionRef("s2", "/tmp/ws/beta", "搭建流程")],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main", { hasText: "alpha" }).click();

  // Only alpha is open, so only alpha's session is drawn.
  await expect(page.locator(".tree-conversation-title")).toHaveText(["修复路由"]);
});

test("a live shell with no readable state says so instead of showing the fixture", async ({ page }) => {
  const warnings: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "warning") warnings.push(message.text());
  });

  await stubShell(page, { workspace: null });

  await page.goto("/");

  await expect(page.locator(".tree-empty")).toBeVisible();
  await expect(page.locator(".tree-project-name")).toHaveCount(0);
  expect(warnings.join("\n")).toContain("kodo: workspace failed");
});

test("Kodo has no account row, and Settings sits at the foot", async ({ page }) => {
  await page.goto("/ui-demo/conversation");

  await expect(page.locator(".account")).toHaveCount(0);
  await expect(page.locator(".sidebar-foot .nav-item")).toHaveText("设置");
  // New Task and Plugins stay above the tree; Settings is a modal.
  await expect(page.locator(".nav .nav-item")).toHaveText(["新建任务", "插件"]);
});

test("Plugins is available directly below New Task", async ({ page }) => {
  await page.goto("/ui-demo/conversation");

  await page.getByRole("link", { name: "插件" }).click();
  await expect(page).toHaveURL(/\/ui-demo\/plugins$/);
  await expect(page.locator(".skill-row")).toHaveCount(7);
  await expect(page.locator(".skill-row").first()).toContainText("缺陷修复");
});

test("The legacy skills URL is a permanent alias of the plugins skills tab", async ({ page }) => {
  await page.goto("/ui-demo/skills");

  await expect(page.locator('[data-testid="plugins-tab-skills"]')).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.locator(".skill-row")).toHaveCount(7);
  // The alias keeps its own URL — navigation never rewrites it.
  await expect(page).toHaveURL(/\/ui-demo\/skills$/);
});

test("New Task opens an empty composer without selecting an existing task", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: PROJECTS,
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "已有任务")],
    },
    branch: "main",
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "已有任务",
      at: 1_700_000_000,
      archived: false,
      turns: [],
    },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").first().click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("已有任务");
  await expect(page.locator('[data-testid="composer-project-context"]')).toHaveCount(0);
  await page.getByRole("link", { name: "新建任务" }).click();
  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  await expect(page.locator(".crumb-current")).toHaveText("新对话");
  await expect(page.locator(".tree-conversation--active")).toHaveCount(0);

  await expect(page.locator('[data-testid="composer-project-context"]')).toBeVisible();
  // The remembered project (written when 已有任务 was open) is revived — the
  // task starts with the project, but with no existing conversation selected.
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("alpha");
  await expect(page.locator('[data-testid="composer-branch"]')).toHaveText("main");
  const contextPlacement = await page.evaluate(() => {
    const input = document.querySelector(".composer-input")?.getBoundingClientRect();
    const context = document.querySelector('[data-testid="composer-project-context"]')?.getBoundingClientRect();
    const box = document.querySelector(".composer-box")?.getBoundingClientRect();
    return input && context && box
      ? { inputBottom: input.bottom, contextTop: context.top, boxBottom: box.bottom }
      : null;
  });
  expect(contextPlacement).not.toBeNull();
  expect(contextPlacement!.contextTop).toBeGreaterThanOrEqual(contextPlacement!.inputBottom);
  expect(contextPlacement!.contextTop).toBeGreaterThanOrEqual(contextPlacement!.boxBottom);
  expect(contextPlacement!.contextTop - contextPlacement!.boxBottom).toBeLessThanOrEqual(8);

  await page.locator('[data-testid="composer-project-picker"]').click();
  await page.getByRole("menuitem", { name: "alpha" }).click();
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("alpha");
  await expect(page.locator('[data-testid="composer-branch"]')).toHaveText("main");

  await page.locator(".composer-input").fill("开始任务");
  await page.locator(".composer-input").press("Enter");
  await expect(page.locator(".msg-bubble")).toHaveText("开始任务");
  await expect(page.locator('[data-testid="composer-project-context"]')).toHaveCount(0);
  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ id: string; text: string }> }).__sendLog ?? [],
  );
  expect(log[0]?.id).toBe("");
});

test("code mode sends without a project as constrained Q&A", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    branch: "main",
  });

  await page.goto("/");

  await page.locator(".composer-input").fill("未选择项目也可以提问");
  // The project gate is gone: send stays enabled with no inline hint — the
  // run itself is constrained (zero tools, DESIGN §11).
  await expect(page.locator('[data-testid="composer-send"]')).toBeEnabled();
  await expect(page.locator('[data-testid="send-disabled-hint"]')).toHaveCount(0);
  await page.locator(".composer-input").press("Enter");

  const log = await page.evaluate(
    () =>
      (window as unknown as { __sendLog?: Array<{ id: string; text: string; project?: string }> })
        .__sendLog ?? [],
  );
  expect(log).toHaveLength(1);
  // No project selected — the payload carries the empty path.
  expect(log[0]?.project).toBe("");
});

test("a project's new task action keeps that project selected", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    branch: "main",
  });

  await page.goto("/");
  await page.locator('button[aria-label="alpha 的操作"]').click();
  await page.getByRole("menuitem", { name: "新建任务" }).click();

  await expect(page.locator('[data-testid="composer-project-context"]')).toBeVisible();
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("alpha");
  await expect(page.locator('[data-testid="composer-branch"]')).toHaveText("main");
});

test("the project picker adds a project and 清除项目选择 deselects only", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    branch: "main",
    pickFolder: "/tmp/ws/gamma",
  });

  await page.goto("/");
  await page.locator('[data-testid="composer-project-picker"]').click();
  await expect(page.getByRole("menuitem", { name: "添加新项目", exact: true })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "清除项目选择", exact: true })).toHaveCount(0);

  await page.getByRole("menuitem", { name: "添加新项目", exact: true }).click();
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("gamma");
  await expect(page.locator(".tree-project-name")).toHaveText(["alpha", "beta", "gamma"]);

  await page.locator('[data-testid="composer-project-picker"]').click();
  await expect(page.getByRole("menuitem", { name: "清除项目选择", exact: true })).toBeVisible();
  await page.getByRole("menuitem", { name: "清除项目选择", exact: true }).click();

  // Deselects the session's project only — the project itself stays listed.
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("未选择项目");
  await expect(page.locator('[data-testid="composer-branch"]')).toHaveText("无分支");
  await expect(page.locator(".tree-project-name")).toHaveText(["alpha", "beta", "gamma"]);
  await page.locator('[data-testid="composer-project-picker"]').click();
  await expect(page.getByRole("menuitem", { name: "清除项目选择", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");

  // Clearing the selection never forgets the remembered project: the next
  // fresh task revives it again.
  await page.getByRole("link", { name: "新建任务" }).click();
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("gamma");
});

test("sessions without a project group under 未选择项目", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: PROJECTS,
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由"), sessionRef("n1", "", "纯对话")],
    },
    branch: "main",
    sessions: {
      n1: { id: "n1", project: "", title: "纯对话", at: 1_700_000_000, archived: false, turns: [] },
    },
  });

  await page.goto("/");

  const group = page.locator('[data-testid="projectless-group"]');
  await expect(group.locator(".tree-group-head")).toHaveText("未选择项目");
  await expect(group.locator(".tree-conversation-title")).toHaveText("纯对话");
  // Real project sessions stay under their project, not in the group.
  await page.locator(".tree-project-main", { hasText: "alpha" }).click();
  await expect(page.locator(".tree-conversations .tree-conversation-title").first()).toHaveText("修复路由");

  // Opening a project-less conversation keeps the task project-less.
  await group.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("纯对话");
  await expect(page.locator(".topbar-crumbs")).toContainText("未选择项目");
  await expect(page.locator(".tree-conversation--active")).toHaveCount(1);
});

test("the tree filters sessions per agent space and keeps project rows", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: PROJECTS,
      sessions: [
        sessionRef("c1", "/tmp/ws/alpha", "修复路由", 1_700_000_000, false, "code"),
        sessionRef("w1", "", "写项目周报", 1_700_000_000, false, "work"),
      ],
    },
    branch: "main",
    sessions: {
      w1: { id: "w1", project: "", title: "写项目周报", at: 1_700_000_000, archived: false, turns: [] },
    },
  });

  await page.goto("/");

  // Code space (default): only the code session; the work project-less session
  // does not gather a group.
  await expect(page.locator('[data-testid="projectless-group"]')).toHaveCount(0);
  await page.locator(".tree-project-main", { hasText: "alpha" }).click();
  await expect(page.locator(".tree-conversation-title")).toHaveText(["修复路由"]);

  // Work space: beta has no work sessions but its row stays (shared registry);
  // the work project-less session gathers under 未选择项目 and nowhere else.
  await switchMode(page, "work");
  await expect(page.locator(".tree-project-name")).toHaveText(["alpha", "beta"]);
  await expect(page.locator(".tree-conversation--active")).toHaveCount(0);
  await expect(page.locator(".tree .tree-conversation-title")).toHaveText(["写项目周报"]);
  const group = page.locator('[data-testid="projectless-group"]');
  await expect(group.locator(".tree-conversation-title")).toHaveText(["写项目周报"]);
});

test("removing a project from the list takes a second confirming click", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    branch: "main",
  });

  await page.goto("/");
  await page.locator('button[aria-label="alpha 的操作"]').click();
  await page.getByRole("menuitem", { name: "从列表移除" }).click();

  // First click arms: the menu stays open and the label flips.
  await expect(page.getByRole("menuitem", { name: "确认移除" })).toBeVisible();
  await expect(page.locator(".tree-project-name")).toHaveText(["alpha", "beta"]);

  // Clicking away resets the armed state.
  await page.keyboard.press("Escape");
  await page.locator('button[aria-label="alpha 的操作"]').click();
  await expect(page.getByRole("menuitem", { name: "从列表移除" })).toBeVisible();
  await page.keyboard.press("Escape");

  // Arm again and commit on the second click.
  await page.locator('button[aria-label="alpha 的操作"]').click();
  await page.getByRole("menuitem", { name: "从列表移除" }).click();
  await page.getByRole("menuitem", { name: "确认移除" }).click();
  await expect(page.locator(".tree-project-name")).toHaveText(["beta"]);
});

test("removing the active conversation's project returns to the new task", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: PROJECTS,
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "已有任务")],
    },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "已有任务",
      at: 1_700_000_000,
      archived: false,
      turns: [],
    },
    branch: "main",
  });

  await page.goto("/");
  await page.locator(".tree-project-main", { hasText: "alpha" }).click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("已有任务");

  await page.locator('button[aria-label="alpha 的操作"]').click();
  await page.getByRole("menuitem", { name: "从列表移除" }).click();
  await page.getByRole("menuitem", { name: "确认移除" }).click();

  // No dangling conversation: back to the welcome stage, project-less.
  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  await expect(page.locator(".crumb-current")).toHaveText("新对话");
  await expect(page.locator(".tree-project-name")).toHaveText(["beta"]);
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("未选择项目");
});

test("the demo routes keep the deterministic fixture", async ({ page }) => {
  await page.goto("/ui-demo/conversation");

  await expect(page.locator(".tree-project-name")).toHaveText([
    "realtime-lakehouse",
    "chixiao",
    "realtime",
    "zhike",
    "insursight",
  ]);
  await expect(page.locator(".tree-conversation-title")).toHaveText([
    "修复 k2k-rust 未知表路由",
    "优化实时数仓搭建流程",
    "排查 Flink 任务延迟问题",
    "完善监控告警规则",
  ]);
  // The browser keeps the demo family, which is what the snapshots load.
  await expect(page.locator(".nav-item").first()).toHaveAttribute("href", "/ui-demo/conversation");
});

test("expanding a project reveals its conversations", async ({ page }) => {
  await page.goto("/ui-demo/conversation");

  await page.locator(".tree-project-main", { hasText: "chixiao" }).click();

  await expect(page.locator(".tree-project--open")).toHaveCount(2);
  await expect(page.locator(".tree-conversation-title")).toHaveCount(4);
  // 4 ≤ 5 — a short list never grows a fold control.
  await expect(page.locator('[data-testid="tree-load-all"]')).toHaveCount(0);
});

test("a project with more than 5 sessions folds the rest behind 加载全部会话", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: PROJECTS,
      sessions: Array.from({ length: 7 }, (_, index) =>
        sessionRef(`s${index + 1}`, "/tmp/ws/alpha", `会话${index + 1}`, 1_700_000_000 + index),
      ),
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main", { hasText: "alpha" }).click();

  // Only the first five stay visible; the tail waits behind the fold control.
  await expect(page.locator(".tree-conversation-title")).toHaveText([
    "会话1",
    "会话2",
    "会话3",
    "会话4",
    "会话5",
  ]);
  const loadAll = page.locator('[data-testid="tree-load-all"]');
  await expect(loadAll).toHaveCount(1);
  await expect(loadAll).toHaveText("加载全部会话");

  // One click reveals the whole list and the control retires.
  await loadAll.click();
  await expect(page.locator(".tree-conversation-title")).toHaveText([
    "会话1",
    "会话2",
    "会话3",
    "会话4",
    "会话5",
    "会话6",
    "会话7",
  ]);
  await expect(page.locator('[data-testid="tree-load-all"]')).toHaveCount(0);
});

test("deleting a conversation takes a second confirming click", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [sessionRef("s1", "/tmp/ws/alpha", "删掉这条")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "删掉这条",
      at: 1_700_000_000,
      archived: false,
      turns: [],
    },
    branch: "main",
  });

  await page.goto("/");
  await page.locator(".tree-project-main", { hasText: "alpha" }).click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("删掉这条");

  await page.locator('button[aria-label="删掉这条 的操作"]').click();
  await page.getByRole("menuitem", { name: "删除会话" }).click();

  // First click arms: the menu stays open and the label flips.
  await expect(page.getByRole("menuitem", { name: "确认删除" })).toBeVisible();
  await expect(page.locator(".tree-conversation")).toHaveCount(1);

  await page.getByRole("menuitem", { name: "确认删除" }).click();
  // Row gone, the delete reached the core, and the open conversation lands
  // back on the new-task stage — no dangling id.
  await expect(page.locator(".tree-conversation")).toHaveCount(0);
  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  const log = await page.evaluate(
    () => (window as unknown as { __deleteLog?: string[] }).__deleteLog ?? [],
  );
  expect(log).toEqual(["s1"]);
});

test("a rejected delete keeps the conversation and reports the failure", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [sessionRef("s1", "/tmp/ws/alpha", "删不掉")] },
    failDelete: true,
    branch: "main",
  });

  await page.goto("/");
  await page.locator(".tree-project-main", { hasText: "alpha" }).click();
  await page.locator('button[aria-label="删不掉 的操作"]').click();
  await page.getByRole("menuitem", { name: "删除会话" }).click();
  await page.getByRole("menuitem", { name: "确认删除" }).click();

  await expect(page.locator(".tree-error")).toContainText("delete failed");
  await expect(page.locator(".tree-conversation")).toHaveCount(1);
});

test("the sidebar filter matches titles, keeps the open conversation, and folds aside", async ({
  page,
}) => {
  await stubShell(page, {
    workspace: {
      projects: PROJECTS,
      sessions: [
        sessionRef("s1", "/tmp/ws/alpha", "修复路由"),
        sessionRef("s2", "/tmp/ws/alpha", "优化搭建"),
        sessionRef("s3", "/tmp/ws/beta", "排查延迟"),
      ],
    },
    sessions: {
      s2: {
        id: "s2",
        project: "/tmp/ws/alpha",
        title: "优化搭建",
        at: 1_700_000_000,
        archived: false,
        turns: [],
      },
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main", { hasText: "alpha" }).click();
  await page.locator(".tree-project-main", { hasText: "beta" }).click();

  const filter = page.locator('[data-testid="tree-conversation-filter"]');
  await filter.fill("路由");
  // Case-insensitive substring over titles — one hit across both groups.
  await expect(page.locator(".tree-conversation-title")).toHaveText(["修复路由"]);
  await expect(page.locator(".tree-no-match")).toHaveCount(1);

  await filter.fill("不存在的名字");
  await expect(page.locator(".tree-conversation-title")).toHaveCount(0);
  await expect(page.locator(".tree-no-match")).toHaveCount(2);

  await filter.fill("");
  await expect(page.locator(".tree-conversation-title")).toHaveCount(3);

  // The open conversation always passes — filtering never hides where you are.
  await page.locator(".tree-conversation", { hasText: "优化搭建" }).click();
  await expect(page.locator(".crumb-current")).toHaveText("优化搭建");
  await filter.fill("路由");
  await expect(page.locator(".tree-conversation-title")).toHaveText(["修复路由", "优化搭建"]);

  await filter.fill("不存在的名字");
  // The open row survives; only beta (no active row, no match) reports 无匹配.
  await expect(page.locator(".tree-conversation-title")).toHaveText(["优化搭建"]);
  await expect(page.locator(".tree-no-match")).toHaveCount(1);
});

test("filtering shows every match past the fold without 加载全部会话", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: PROJECTS,
      sessions: Array.from({ length: 7 }, (_, index) =>
        sessionRef(`s${index + 1}`, "/tmp/ws/alpha", `会话${index + 1}`, 1_700_000_000 + index),
      ),
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main", { hasText: "alpha" }).click();

  await page.locator('[data-testid="tree-conversation-filter"]').fill("会话7");
  await expect(page.locator(".tree-conversation-title")).toHaveText(["会话7"]);
  await expect(page.locator('[data-testid="tree-load-all"]')).toHaveCount(0);
});
