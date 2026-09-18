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
  // The branch is read from the project's checkout, not hard-coded.
  await expect(page.locator(".topbar-left .chip--static")).toHaveText("main");
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
  await expect(page.locator(".sidebar-foot .nav-item")).toHaveText("Settings");
  // Conversations and Archive stay above the tree; Settings is not among them.
  await expect(page.locator(".nav .nav-item")).toHaveText(["Conversations", "Archive"]);
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
