import { expect, test } from "@playwright/test";

/**
 * Deterministic screens captured at the reference viewport (1586×992) from
 * docs/design/UI_ACCEPTANCE.md. Compare them with docs/design/references/.
 */
const PAGES = [
  { name: "conversation-main", url: "/ui-demo/conversation" },
  { name: "conversation-inspector-closed", url: "/ui-demo/conversation?inspector=closed" },
  { name: "response-trace", url: "/ui-demo/trace" },
  { name: "archive", url: "/ui-demo/archive" },
  { name: "skills", url: "/ui-demo/skills" },
  { name: "settings", url: "/ui-demo/settings" },
];

for (const { name, url } of PAGES) {
  test(name, async ({ page }) => {
    await page.goto(url);
    await page.waitForLoadState("networkidle");
    await expect(page).toHaveScreenshot(`${name}.png`);
  });
}
