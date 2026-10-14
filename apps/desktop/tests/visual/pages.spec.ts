import { expect, test, type Page } from "@playwright/test";

/**
 * Deterministic screens captured at the reference viewport (1586×992) from
 * docs/design/UI_ACCEPTANCE.md. The snapshots pin the current rendering.
 *
 * `setup` drives deterministic UI state that has no URL (settings panes open
 * via clicks — no product routing change for the sake of a screenshot).
 */
const PAGES: Array<{ name: string; url: string; setup?: (page: Page) => Promise<void> }> = [
  { name: "conversation-main", url: "/ui-demo/conversation" },
  { name: "conversation-inspector-closed", url: "/ui-demo/conversation?inspector=closed" },
  { name: "response-trace", url: "/ui-demo/trace" },
  { name: "archive", url: "/ui-demo/archive" },
  { name: "plugins", url: "/ui-demo/plugins" },
  { name: "plugins-agents", url: "/ui-demo/plugins?type=agents" },
  { name: "plugins-mcp", url: "/ui-demo/plugins?type=mcp" },
  {
    name: "plugins-mcp-tools",
    url: "/ui-demo/plugins?type=mcp",
    setup: async (page) => {
      await expect(page.locator('.page-inner [aria-label="查看 filesystem 的工具清单"]')).toBeVisible();
      await page.locator('.page-inner [aria-label="查看 filesystem 的工具清单"]').click();
      await expect(page.locator(".mcp-tools .mcp-tool-row").first()).toBeVisible();
    },
  },
  {
    name: "plugins-mcp-editor",
    url: "/ui-demo/plugins?type=mcp",
    setup: async (page) => {
      await expect(page.locator('.page-inner [aria-label="编辑"]').first()).toBeVisible();
      await page.locator('.page-inner [aria-label="编辑"]').first().click();
      await expect(page.locator(".provider-editor")).toBeVisible();
    },
  },
  { name: "settings", url: "/ui-demo/settings" },
  // The workbench collection and the four apps it hosts. An open app is the
  // same route with `?app=`, so the sidebar keeps its single 工作台 entry.
  { name: "workbench", url: "/ui-demo/workbench" },
  {
    name: "workbench-knowledge",
    url: "/ui-demo/workbench?app=knowledge",
    setup: async (page) => {
      await page.getByTestId("knowledge-open-doc-compat-demo").click();
      await expect(page.getByTestId("knowledge-source-lines")).toBeVisible();
    },
  },
  { name: "workbench-todo", url: "/ui-demo/workbench?app=todo" },
  {
    name: "workbench-todo-list",
    url: "/ui-demo/workbench?app=todo",
    setup: async (page) => {
      await page.locator('[data-testid="todo-view-list"]').click();
      await expect(page.locator('[data-testid="todo-list"]')).toBeVisible();
    },
  },
  { name: "workbench-calendar", url: "/ui-demo/workbench?app=calendar" },
  {
    name: "workbench-calendar-month",
    url: "/ui-demo/workbench?app=calendar",
    setup: async (page) => {
      await page.locator('[data-testid="cal-view-month"]').click();
      await expect(page.locator('[data-testid="cal-month"]')).toBeVisible();
    },
  },
  {
    name: "workbench-results",
    url: "/ui-demo/workbench?app=artifacts",
    setup: async (page) => {
      // The file checks are on the expanded detail, which is collapsed by
      // default — open the first row so the missing/changed states are in the
      // frame.
      await page.locator(".result-title").first().click();
      await expect(page.locator(".result-detail").first()).toBeVisible();
    },
  },
];

for (const { name, url, setup } of PAGES) {
  test(name, async ({ page }) => {
    await page.goto(url);
    await page.waitForLoadState("networkidle");
    if (setup) {
      await setup(page);
      await page.waitForLoadState("networkidle");
    }
    await expect(page).toHaveScreenshot(`${name}.png`);
  });
}
