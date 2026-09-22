import { expect, test } from "@playwright/test";

/**
 * Accessibility + multi-viewport smoke for the compact IDE shell.
 * Viewports required by the a11y/narrow-window task: 900 / 1100 / 1440 / 1586 / 1920.
 *
 * Token contrast math lives in scripts/contrast-audit.mjs (`npm run test:contrast`).
 */

const VIEWPORTS = [
  { width: 900, height: 800, sidebar: 220, inspectorDefaultOpen: false },
  { width: 1100, height: 800, sidebar: 220, inspectorDefaultOpen: false },
  { width: 1440, height: 900, sidebar: 260, inspectorDefaultOpen: true },
  { width: 1586, height: 992, sidebar: 260, inspectorDefaultOpen: true },
  { width: 1920, height: 1080, sidebar: 260, inspectorDefaultOpen: true },
];

for (const vp of VIEWPORTS) {
  test(`viewport smoke ${vp.width}×${vp.height}: no overflow, geometry holds`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await page.goto("/ui-demo/conversation");
    await page.waitForLoadState("networkidle");

    const m = await page.evaluate(() => {
      const box = (selector: string) =>
        document.querySelector(selector)?.getBoundingClientRect() ?? null;
      return {
        scrollWidth: document.documentElement.scrollWidth,
        innerWidth: window.innerWidth,
        sidebar: box(".sidebar")?.width ?? 0,
        hasPanel: document.querySelector(".ins-panel") !== null,
        rail: box(".ins-rail")?.width ?? 0,
        composerBottom: box(".composer")?.bottom ?? 0,
        composerRight: box(".composer")?.right ?? 0,
        composerWidth: box(".composer")?.width ?? 0,
      };
    });

    expect(m.scrollWidth).toBeLessThanOrEqual(vp.width);
    expect(m.sidebar).toBe(vp.sidebar);
    expect(m.hasPanel).toBe(vp.inspectorDefaultOpen);
    expect(m.composerBottom).toBeLessThanOrEqual(vp.height);
    expect(m.composerRight).toBeLessThanOrEqual(vp.width + 1);
    expect(m.composerWidth).toBeGreaterThan(0);
    if (!vp.inspectorDefaultOpen) expect(m.rail).toBe(40);
  });
}

test("900px: sidebar toggles; inspector opens as overlay without squeezing main", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 800 });
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  // Sidebar hideable via top-bar toggle (kept in DOM so aria-controls stays valid).
  await expect(page.locator(".sidebar")).toBeVisible();
  await page.locator('button[aria-label="Toggle sidebar"]').click();
  await expect(page.locator(".sidebar")).toBeHidden();
  await expect(page.locator('button[aria-label="Toggle sidebar"]')).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  await page.locator('button[aria-label="Toggle sidebar"]').click();
  await expect(page.locator(".sidebar")).toBeVisible();

  // Main width stays stable when the inspector panel opens (overlay).
  const before = await page.locator(".main").evaluate((el) => el.getBoundingClientRect().width);
  await page.locator(".rail-btn").first().click();
  await expect(page.locator(".ins-panel")).toBeVisible();
  const after = await page.locator(".main").evaluate((el) => el.getBoundingClientRect().width);
  expect(Math.abs(after - before)).toBeLessThanOrEqual(2);

  const scroll = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    innerWidth: window.innerWidth,
  }));
  expect(scroll.scrollWidth).toBeLessThanOrEqual(scroll.innerWidth);
});

test("Menu keyboard: Escape, arrows, Home/End, focus return", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  // Projects + menu (sidebar).
  const trigger = page.locator('.section-head button[aria-label="New project"]');
  await trigger.focus();
  await page.keyboard.press("Enter");
  await expect(page.locator('.menu[role="menu"]')).toBeVisible();

  const triggerId = await trigger.getAttribute("aria-controls");
  expect(triggerId).toBeTruthy();
  await expect(trigger).toHaveAttribute("aria-haspopup", "menu");
  await expect(trigger).toHaveAttribute("aria-expanded", "true");

  const items = page.locator('.menu[role="menu"] [role="menuitem"]');
  await expect(items).toHaveCount(2);

  // Focus starts on the first item after open.
  await expect(items.nth(0)).toBeFocused();

  await page.keyboard.press("ArrowDown");
  await expect(items.nth(1)).toBeFocused();

  await page.keyboard.press("ArrowDown"); // wraps
  await expect(items.nth(0)).toBeFocused();

  await page.keyboard.press("End");
  await expect(items.nth(1)).toBeFocused();

  await page.keyboard.press("Home");
  await expect(items.nth(0)).toBeFocused();

  await page.keyboard.press("ArrowUp"); // wraps up
  await expect(items.nth(1)).toBeFocused();

  await page.keyboard.press("Escape");
  await expect(page.locator('.menu[role="menu"]')).toHaveCount(0);
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
  await expect(trigger).toBeFocused();
});

test("Tab reaches core controls and focus is visible", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  // Keyboard-only path to the composer via Tab presses (bounded).
  await page.locator("body").click({ position: { x: 5, y: 5 } });
  let reachedComposer = false;
  for (let i = 0; i < 80; i += 1) {
    await page.keyboard.press("Tab");
    reachedComposer = await page.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      return el?.classList.contains("composer-input") ?? false;
    });
    if (reachedComposer) break;
  }
  // Keyboard focus reaches the draft box (ring itself is suppressed on the
  // input by design; other controls keep the global :focus-visible ring).
  expect(reachedComposer).toBe(true);

  // Ring is suppressed on the draft box by design; other controls keep it.
  const ring = await page.evaluate(() => {
    const el = document.querySelector(".composer-perm-trigger") as HTMLElement | null;
    if (!el) return null;
    el.focus();
    const style = getComputedStyle(el);
    return { style: style.outlineStyle, width: style.outlineWidth };
  });
  expect(ring).not.toBeNull();
  expect(ring!.style).not.toBe("none");
  expect(parseFloat(ring!.width)).toBeGreaterThanOrEqual(2);
});

test("reduced-motion disables transitions", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  const durations = await page.evaluate(() => {
    const probe = document.querySelector(".toggle-knob") ?? document.querySelector(".composer-perm-trigger");
    if (!probe) return null;
    const style = getComputedStyle(probe);
    return { transition: style.transitionDuration, animation: style.animationDuration };
  });
  if (durations) {
    // 0.01ms rule from prefers-reduced-motion; Chrome may report it as 1e-05s.
    expect(durations.transition.split(",")[0].trim()).toMatch(
      /^0(\.0+)?s$|^0\.01ms$|^1e-0?5s$/,
    );
  }
});

test("decorative muted is not used as informational body color on key text", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  const colors = await page.evaluate(() => {
    const pick = (sel: string) => {
      const el = document.querySelector(sel);
      return el ? getComputedStyle(el).color : null;
    };
    return {
      time: pick(".msg-time"),
      duration: pick(".trace-duration"),
      empty: pick(".empty-note"),
      brandTag: pick(".brand-tagline"),
    };
  });

  // Readable secondary #667085 → rgb(102, 112, 133); muted is #98a2b3.
  const readable = "rgb(102, 112, 133)";
  if (colors.time) expect(colors.time).toBe(readable);
  if (colors.duration) expect(colors.duration).toBe(readable);
  if (colors.empty) expect(colors.empty).toBe(readable);
  if (colors.brandTag) expect(colors.brandTag).toBe(readable);
});
