import { expect, test } from "@playwright/test";

/**
 * UI_ACCEPTANCE.md §1 asks for a smoke test at these two viewports. This asserts
 * the layout invariants the acceptance criteria name — panel widths, no
 * horizontal overflow, the composer staying usable, and a completed reply
 * showing its answer without the process — rather than comparing pixels.
 *
 * 900 / 1100 / 1586 are covered in a11y.spec.ts (multi-viewport smoke).
 */
const VIEWPORTS = [
  { width: 1440, height: 900 },
  { width: 1920, height: 1080 },
];

const SIDEBAR_WIDTH = 260;
const INSPECTOR_WIDTH = 330;
const RAIL_WIDTH = 40;

for (const viewport of VIEWPORTS) {
  const label = `${viewport.width}×${viewport.height}`;

  test(`layout holds at ${label}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.goto("/ui-demo/conversation?inspector=open");
    await page.waitForLoadState("networkidle");

    const m = await page.evaluate(() => {
      const box = (selector: string) => document.querySelector(selector)?.getBoundingClientRect() ?? null;
      return {
        scrollWidth: document.documentElement.scrollWidth,
        innerWidth: window.innerWidth,
        innerHeight: window.innerHeight,
        sidebar: box(".sidebar")?.width ?? 0,
        inspector: box(".ins-panel")?.width ?? 0,
        composerBottom: box(".composer")?.bottom ?? 0,
        composerWidth: box(".composer")?.width ?? 0,
        hasFinal: document.querySelector('[data-testid="answer-conclusion"]') !== null,
      };
    });

    expect(m.scrollWidth).toBeLessThanOrEqual(m.innerWidth);
    expect(m.sidebar).toBe(SIDEBAR_WIDTH);
    expect(m.inspector).toBe(INSPECTOR_WIDTH);
    expect(m.composerBottom).toBeLessThanOrEqual(m.innerHeight);
    expect(m.composerWidth).toBeGreaterThan(0);
    // Completed replies keep the execution record folded so it takes no space.
    // Running row density is covered in behaviour.spec (UI_ACCEPTANCE §3/§4).
    expect(m.hasFinal).toBe(true);
    await expect(page.locator(".main .trace-row:visible")).toHaveCount(0);
  });

  test(`collapsed inspector becomes a rail at ${label}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.goto("/ui-demo/conversation?inspector=closed");
    await page.waitForLoadState("networkidle");

    const m = await page.evaluate(() => {
      const box = (selector: string) => document.querySelector(selector)?.getBoundingClientRect() ?? null;
      return {
        rail: box(".ins-rail")?.width ?? 0,
        inspector: box(".inspector")?.width ?? 0,
        hasPanel: document.querySelector(".ins-panel") !== null,
        mainRight: box(".main")?.right ?? 0,
        railLeft: box(".ins-rail")?.left ?? 0,
      };
    });

    expect(m.rail).toBe(RAIL_WIDTH);
    expect(m.hasPanel).toBe(false);
    // The collapsed region is the rail plus the inspector's own left border,
    // and the conversation pane runs right up to that border.
    expect(m.inspector).toBe(RAIL_WIDTH + 1);
    expect(m.railLeft - m.mainRight).toBe(1);
  });
}
