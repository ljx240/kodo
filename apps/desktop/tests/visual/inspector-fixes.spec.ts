import { test, expect } from "@playwright/test";

/**
 * One-off regression check for the two inspector bugs the user reported on
 * 2026-10-16:
 *   1. Memory panel rendered the same content as Inspector (根因:
 *      contentFor 收到 railActive="summary" 而非 panelActive="memory").
 *   2. Browse tab spawned a separate OS child window (根因:
 *      Tauri WebviewWindowBuilder 创建一个 OS 子窗口, 然后 set_position
 *      把它盖到 panel 位置).
 *
 * Both are part of the existing project Playwright setup; this file just
 * drives the demo route and asserts the right surface is in the DOM.
 */

const VIEWPORT = { width: 1586, height: 992 };

test.use({ viewport: VIEWPORT });

test("memory panel renders the Memory overview, not the response overview", async ({ page }) => {
  await page.goto("/ui-demo/conversation?inspector=open");
  await expect(page.locator(".ins-panel")).toBeVisible();

  // Click the panel-layer "Memory" chip (the i18n label is "Memory" — the
  // English brand-name, not "记忆"). The rail stays hidden while we are on
  // a non-summary panel tab.
  await page.locator(".ins-tab", { hasText: "Memory" }).click();

  // The Memory inspector renders the MemoryOverview component. With a
  // project selected in demo mode the toolbar (search + refresh) and the
  // list of entries are mounted; both are unique to MemoryOverview and
  // never appear in ResponseOverview. The "跳转 Inspector" button that
  // used to sit in the toolbar is gone too (it was the one outbound jump
  // the user asked us to remove).
  await expect(page.locator(".ins-content .memory-toolbar").first()).toBeVisible();
  await expect(page.locator(".ins-content .memory-row").first()).toBeVisible();
  await expect(page.locator('.ins-content button:has-text("Inspector")')).toHaveCount(0);

  // Belt-and-suspenders: SummarySection (only ResponseOverview renders it)
  // must not be present while the Memory tab is the active panel. The
  // pre-fix bug had the Inspector rail section still being rendered, so
  // [data-testid="ins-summary-actions"] leaked through.
  await expect(page.locator('[data-testid="ins-summary-actions"]')).toHaveCount(0);
});

test("browse tab stays inside the inspector (iframe, no OS child window)", async ({ page }) => {
  await page.route("https://example.com/**", (route) => route.fulfill({
    contentType: "text/html",
    body: "<h1>Local browse fixture</h1>",
  }));
  await page.goto("/ui-demo/conversation?inspector=open");
  await page.locator(".ins-tab", { hasText: "浏览" }).click();

  // The url bar lives inside the panel DOM.
  await expect(page.locator('[data-testid="browse-url"]')).toBeVisible();
  // After the user types a URL and presses 前往, an <iframe data-testid="browse-frame">
  // is mounted inside the panel. Pre-fix: this div was empty because Tauri overlaid
  // a child WebviewWindow — so the locator here used to match the empty stub.
  await page.locator('[data-testid="browse-url"]').fill("https://example.com");
  await page.locator('[data-testid="browse-go"]').click();

  const frame = page.locator('[data-testid="browse-frame"]');
  await expect(frame).toBeVisible();
  await expect(frame).toHaveCount(1);
  await expect(page.frameLocator('[data-testid="browse-frame"]').getByRole("heading", {
    name: "Local browse fixture",
  })).toBeVisible();
  const tag = await frame.evaluate((node) => node.tagName.toLowerCase());
  expect(tag).toBe("iframe");
});
