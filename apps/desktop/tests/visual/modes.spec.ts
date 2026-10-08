import { expect, test } from "@playwright/test";
import { project, sessionRef, stubShell, switchMode } from "./shell";

/**
 * The Work/Code dual spaces (docs/design/DESIGN.md §11). The switcher is the
 * Codex-style brand dropdown (trigger reads `Kodo Code ˅` / `Kodo Work ˅`);
 * switching swaps the whole agent space — session list, skills, new-task
 * behavior — never mixing the two. The
 * mode is stamped on a session at creation and immutable; `agent-mode` (one
 * settings key with the CLI) only stamps new sessions.
 */
const PROJECTS = [project("/tmp/ws/alpha", "alpha"), project("/tmp/ws/beta", "beta")];

type SettingsRow = { key: string; value: string };
type SendRow = { id: string; text: string; project?: string };

test("the Work/Code switcher sits in the sidebar brand block and persists to the shared settings key", async ({
  page,
}) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    settings: { "agent-mode": "work" },
  });

  await page.goto("/");

  // The switcher is sidebar chrome (the brand dropdown), not composer chrome.
  await expect(page.locator("#kodo-sidebar [data-testid=\"mode-toggle\"]")).toHaveCount(1);
  await expect(page.locator(".composer-mode")).toHaveCount(0);

  // The trigger names the space it opens — the top-left always says which one.
  const brand = page.locator(".brand-name");
  await expect(brand).toHaveText("Kodo Work");

  // Stored mode wins on load: the open menu check-marks it.
  await page.locator('[data-testid="mode-toggle"]').click();
  await expect(page.locator('[data-testid="mode-work"]')).toHaveClass(/menu-item--selected/);
  await expect(page.locator('[data-testid="mode-code"]')).not.toHaveClass(/menu-item--selected/);

  await page.locator('[data-testid="mode-code"]').click();
  await expect(brand).toHaveText("Kodo Code");
  const log = await page.evaluate(
    () => (window as unknown as { __settingsLog?: SettingsRow[] }).__settingsLog ?? [],
  );
  expect(log).toContainEqual({ key: "agent-mode", value: "code" });

  // Reopening the menu shows the new space checked.
  await page.locator('[data-testid="mode-toggle"]').click();
  await expect(page.locator('[data-testid="mode-code"]')).toHaveClass(/menu-item--selected/);
  await expect(page.locator('[data-testid="mode-work"]')).not.toHaveClass(/menu-item--selected/);
});

test("code mode sends without a project — projectless Code is constrained Q&A", async ({
  page,
}) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    autoRun: { answer: "好的，我来帮你总结。", plain: true },
  });

  await page.goto("/");

  // Code (default) without a project: send stays enabled — the run is
  // constrained Q&A (zero tools, DESIGN §11), so no gate, no inline hint,
  // and the welcome copy stops demanding a project.
  await page.locator(".composer-input").fill("帮我写个总结");
  await expect(page.locator('[data-testid="composer-send"]')).toBeEnabled();
  await expect(page.locator('[data-testid="send-disabled-hint"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="welcome-sub"]')).toHaveText(
    "直接提问或粘贴报错都可以；选中项目后我会读代码、改代码。",
  );
  await page.locator(".composer-input").press("Enter");

  await expect(page.locator(".msg-bubble")).toHaveText("帮我写个总结");
  const sends = await page.evaluate(
    () => (window as unknown as { __sendLog?: SendRow[] }).__sendLog ?? [],
  );
  expect(sends).toHaveLength(1);
  // No project selected — the run is created project-less.
  expect(sends[0]?.project).toBe("");
});

test("work mode without a project sends pure conversation", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    autoRun: { answer: "好的，我来帮你总结。", plain: true },
  });

  await page.goto("/");

  // Work without a project: pure conversation is a first-class task.
  await switchMode(page, "work");
  await expect(page.locator('[data-testid="welcome-sub"]')).toHaveText(
    "写文档、做总结、提问都可以；选中项目后我还能读写项目里的文件。",
  );
  await expect(page.locator('[data-testid="send-disabled-hint"]')).toHaveCount(0);
  await page.locator(".composer-input").fill("帮我写个总结");
  await expect(page.locator('[data-testid="composer-send"]')).toBeEnabled();
  await page.locator(".composer-input").press("Enter");

  await expect(page.locator(".msg-bubble")).toHaveText("帮我写个总结");
  const sends = await page.evaluate(
    () => (window as unknown as { __sendLog?: SendRow[] }).__sendLog ?? [],
  );
  expect(sends).toHaveLength(1);
  // No project selected — the run is created project-less.
  expect(sends[0]?.project).toBe("");
  const settings = await page.evaluate(
    () => (window as unknown as { __settingsLog?: SettingsRow[] }).__settingsLog ?? [],
  );
  expect(settings).toContainEqual({ key: "agent-mode", value: "work" });
});

test("work mode with a project sends with that project and shows the work voice", async ({
  page,
}) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
    branch: "main",
    autoRun: { answer: "总结写好了。", plain: true },
  });
  await page.addInitScript(() => {
    window.localStorage.setItem("kodo:last-project", "/tmp/ws/alpha");
  });

  await page.goto("/");

  // Code + project keeps the classic coding copy; work + project switches it.
  await expect(page.locator('[data-testid="welcome-sub"]')).toHaveText(
    "描述任务或粘贴报错，我会在当前项目里读代码、改代码。",
  );
  await switchMode(page, "work");
  await expect(page.locator('[data-testid="welcome-sub"]')).toHaveText(
    "写文档、做总结、提问都可以；选中项目后我还能读写项目里的文件。",
  );

  // The revived project rides along on the send payload.
  await page.locator(".composer-input").fill("总结一下这个项目");
  await expect(page.locator('[data-testid="composer-send"]')).toBeEnabled();
  await page.locator(".composer-input").press("Enter");
  const sends = await page.evaluate(
    () => (window as unknown as { __sendLog?: SendRow[] }).__sendLog ?? [],
  );
  expect(sends[0]?.project).toBe("/tmp/ws/alpha");
});

test("switching modes switches the whole session space — the two never mix", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: PROJECTS,
      sessions: [
        sessionRef("c1", "/tmp/ws/alpha", "修复路由", 1_700_000_000, false, "code"),
        sessionRef("w1", "/tmp/ws/alpha", "整理架构说明", 1_700_000_000, false, "work"),
        sessionRef("w2", "", "写项目周报", 1_700_000_000, false, "work"),
      ],
    },
  });

  await page.goto("/");

  // Code space: only the code session, no project-less group.
  await page.locator(".tree-project-main", { hasText: "alpha" }).click();
  await expect(page.locator(".tree-conversation-title")).toHaveText(["修复路由"]);
  await expect(page.locator('[data-testid="projectless-group"]')).toHaveCount(0);

  // Work space: only the work sessions — project rows stay (shared registry).
  await switchMode(page, "work");
  await expect(page.locator(".tree-project-name")).toHaveText(["alpha", "beta"]);
  // Conversations render beside the project row inside one wrapper per project.
  await expect(
    page.locator('div:has(> .tree-project .tree-project-name:text-is("alpha")) .tree-conversation-title'),
  ).toHaveText(["整理架构说明"]);
  await expect(page.locator('[data-testid="projectless-group"] .tree-conversation-title')).toHaveText([
    "写项目周报",
  ]);

  // Back to code: its sessions come back, the work ones leave again.
  await switchMode(page, "code");
  await expect(page.locator(".tree-conversation-title").first()).toHaveText("修复路由");
  await expect(page.locator('[data-testid="projectless-group"]')).toHaveCount(0);
});

test("switching away from a conversation of the other space falls back to the welcome stage", async ({
  page,
}) => {
  await stubShell(page, {
    workspace: {
      projects: PROJECTS,
      sessions: [
        sessionRef("c1", "/tmp/ws/alpha", "修复路由", 1_700_000_000, false, "code"),
        sessionRef("w1", "/tmp/ws/alpha", "整理架构说明", 1_700_000_000, false, "work"),
      ],
    },
    branch: "main",
    session: {
      id: "c1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main", { hasText: "alpha" }).click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("修复路由");

  // Leaving the code space: the conversation stays in its own list, the view
  // falls back to the welcome stage with nothing active.
  await switchMode(page, "work");
  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  await expect(page.locator(".tree-conversation--active")).toHaveCount(0);

  // Switching back finds it there and it opens again.
  await switchMode(page, "code");
  await expect(page.locator(".tree-conversation-title")).toHaveText(["修复路由"]);
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("修复路由");
});

test("the welcome stage names the current space with its identity logo", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: PROJECTS, sessions: [] },
  });

  await page.goto("/");

  // Default space is Code: the block carries the wordmark and the space tag.
  const identity = page.locator('[data-testid="welcome-identity"]');
  await expect(identity).toHaveAttribute("data-mode", "code");
  await expect(identity).toHaveText("Kodo Code");
  await expect(identity.locator("img")).toHaveAttribute("src", /kodo-code-mark/);

  // Switching spaces swaps the whole stage — logo and wordmark follow.
  await switchMode(page, "work");
  await expect(identity).toHaveAttribute("data-mode", "work");
  await expect(identity).toHaveText("Kodo Work");
  await expect(identity.locator("img")).toHaveAttribute("src", /kodo-work-mark/);
});
