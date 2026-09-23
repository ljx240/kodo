import { expect, test } from "@playwright/test";
import { project, sessionRef, stubShell } from "./shell";

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
  // A New Task starts without a project; the branch becomes available after
  // the user chooses one in the composer context row.
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

test("a fresh New Task does not reuse the remembered project", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    branch: "main",
  });
  await page.addInitScript(() => {
    window.localStorage.setItem("kodo:last-project", "/tmp/ws/alpha");
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
  // New Task and Skills stay above the tree; Settings is opened as a modal.
  await expect(page.locator(".nav .nav-item")).toHaveText(["新建任务", "技能"]);
});

test("Skills is available directly below New Task", async ({ page }) => {
  await page.goto("/ui-demo/conversation");

  await page.getByRole("link", { name: "技能" }).click();
  await expect(page).toHaveURL(/\/ui-demo\/skills$/);
  await expect(page.locator(".skill-row")).toHaveCount(7);
  await expect(page.locator(".skill-row").first()).toContainText("缺陷修复");
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
  await expect(page.locator(".conv-title")).toHaveText("已有任务");
  await expect(page.locator('[data-testid="composer-project-context"]')).toHaveCount(0);
  await page.getByRole("link", { name: "新建任务" }).click();
  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  await expect(page.locator(".conv-title")).toHaveText("新对话");
  await expect(page.locator(".tree-conversation--active")).toHaveCount(0);

  await expect(page.locator('[data-testid="composer-project-context"]')).toBeVisible();
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("未选择项目");
  await expect(page.locator('[data-testid="composer-branch"]')).toHaveText("无分支");
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

test("New Task blocks sending until a project is selected", async ({ page }) => {
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
  await page.locator(".tree-project-main").first().click();
  await page.locator(".tree-conversation").click();
  await page.getByRole("link", { name: "新建任务" }).click();

  await page.locator(".composer-input").fill("未选择项目也不能发送");
  await page.locator(".composer-input").press("Enter");

  await expect(page.locator('[data-testid="action-error"]')).toContainText("请选择项目");
  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: unknown[] }).__sendLog ?? [],
  );
  expect(log).toHaveLength(0);
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

test("the project picker adds a project and removes only the session selection", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    branch: "main",
    pickFolder: "/tmp/ws/gamma",
  });

  await page.goto("/");
  await page.locator('[data-testid="composer-project-picker"]').click();
  await expect(page.getByRole("menuitem", { name: "添加新项目", exact: true })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "移除当前项目", exact: true })).toHaveCount(0);

  await page.getByRole("menuitem", { name: "添加新项目", exact: true }).click();
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("gamma");
  await expect(page.locator(".tree-project-name")).toHaveText(["alpha", "beta", "gamma"]);

  await page.locator('[data-testid="composer-project-picker"]').click();
  await expect(page.getByRole("menuitem", { name: "移除当前项目", exact: true })).toBeVisible();
  await page.getByRole("menuitem", { name: "移除当前项目", exact: true }).click();

  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("未选择项目");
  await expect(page.locator('[data-testid="composer-branch"]')).toHaveText("无分支");
  await expect(page.locator(".tree-project-name")).toHaveText(["alpha", "beta", "gamma"]);
  await page.locator('[data-testid="composer-project-picker"]').click();
  await expect(page.getByRole("menuitem", { name: "移除当前项目", exact: true })).toHaveCount(0);
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
});
