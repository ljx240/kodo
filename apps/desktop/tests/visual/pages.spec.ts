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
