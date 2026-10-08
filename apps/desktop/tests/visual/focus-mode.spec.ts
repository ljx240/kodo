import { expect, test, type Page } from "@playwright/test";
import { project, sessionRef, stubShell } from "./shell";

/**
 * Focus mode (LAYOUT §5 / COMPONENTS §9): the deliberate exception to
 * "conversation is the largest visual region". Dragging the Inspector past
 * 900px swaps the conversation column down to a 300px state strip and
 * lets the Inspector grow up to 1100px. Triggered by drag, double-click,
 * or Shift+ArrowLeft; exited by dragging back below 900, double-clicking,
 * or loading on a viewport <1440px.
 */

function focusShell() {
  return {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "focus 模式")],
    },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "focus 模式",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "看下附件",
          context: [],
          items: [
            {
              id: 1,
              at: 1_700_000_000,
              status: "done",
              duration: 1,
              kind: "agentMessage",
              text: "看完。",
              checks: [],
            },
          ],
          done: true,
          stopped: false,
          error: null,
        },
        {
          ask: "继续",
          context: [],
          items: [
            {
              id: 2,
              at: 1_700_000_060,
              status: "done",
              duration: 1,
              kind: "agentMessage",
              text: "好。",
              checks: [],
            },
          ],
          done: true,
          stopped: false,
          error: null,
        },
      ],
    },
  };
}

async function openFocus(page: Page) {
  await stubShell(page, focusShell());
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
}

const panelWidth = (page: Page) =>
  page.locator(".ins-panel").evaluate((el) => Math.round(el.getBoundingClientRect().width));

const mainWidth = (page: Page) =>
  page.locator(".main").evaluate((el) => Math.round(el.getBoundingClientRect().width));

const focusModeActive = (page: Page) =>
  page.locator(".app.app--focus-mode").count();

test("drag past 900 enters focus mode; conversation collapses to 300px", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  await expect(sep).toBeVisible();
  await expect(sep).toHaveAttribute("aria-valuemax", "900");
  await expect(focusModeActive(page)).resolves.toBe(0);

  // 330 → drag the handle far enough left to overshoot the threshold.
  let box = (await sep.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 300);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 - 700, box.y + 300, { steps: 8 });
  await page.mouse.up();

  // Threshold + class + geometry: Inspector at 900, conversation at 300.
  await expect.poll(() => panelWidth(page)).toBeCloseTo(900, 0);
  await expect.poll(() => mainWidth(page)).toBeCloseTo(300, 1);
  await expect(focusModeActive(page)).resolves.toBe(1);
  await expect(sep).toHaveAttribute("aria-valuemax", "1100");
});

test("drag back below 900 exits focus mode", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');

  // Enter focus.
  let box = (await sep.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 300);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 - 700, box.y + 300, { steps: 8 });
  await page.mouse.up();
  await expect(focusModeActive(page)).resolves.toBe(1);

  // Pull back right past the threshold (now in focus mode, max=1100).
  box = (await sep.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 300);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 + 700, box.y + 300, { steps: 8 });
  await page.mouse.up();
  await expect.poll(() => panelWidth(page)).toBeLessThan(900);
  await expect(focusModeActive(page)).resolves.toBe(0);
  await expect.poll(() => mainWidth(page)).toBeGreaterThan(400);
  await expect(sep).toHaveAttribute("aria-valuemax", "900");
});

test("double-click toggles focus mode and back", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  await expect(focusModeActive(page)).resolves.toBe(0);

  await sep.dblclick();
  await expect(focusModeActive(page)).resolves.toBe(1);
  await expect.poll(() => panelWidth(page)).toBeCloseTo(900, 0);
  await expect.poll(() => mainWidth(page)).toBeCloseTo(300, 1);

  await sep.dblclick();
  await expect(focusModeActive(page)).resolves.toBe(0);
  await expect.poll(() => mainWidth(page)).toBeGreaterThan(400);
});

test("Shift+ArrowLeft on the separator walks into focus mode", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  await sep.focus();

  // 330 + 9 × 64 = 906, clamped at the threshold (900) → focus mode flips on.
  for (let i = 0; i < 9; i++) {
    await page.keyboard.press("Shift+ArrowLeft");
  }
  await expect(focusModeActive(page)).resolves.toBe(1);
  await expect.poll(() => panelWidth(page)).toBeCloseTo(900, 0);
});

test("focus mode survives a toggle-off/on cycle; turn rail stays visible", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');

  // Enter focus.
  await sep.dblclick();
  await expect(focusModeActive(page)).resolves.toBe(1);
  await expect(page.locator('[data-testid="turn-rail"]')).toBeVisible();

  // Exit and re-enter — the state machine stays consistent within a session.
  // (Reload-time persistence is owned by Tauri's setSetting, which is
  // in-memory only in the Playwright harness, so we exercise the cycle here
  // instead of across reload.)
  await sep.dblclick();
  await expect(focusModeActive(page)).resolves.toBe(0);
  await sep.dblclick();
  await expect(focusModeActive(page)).resolves.toBe(1);

  // Turn rail container-query hide is overridden — it stays visible even
  // though the conversation column dropped to 300px (its own <900px query).
  await expect(page.locator('[data-testid="turn-rail"]')).toBeVisible();
});

test("below 1200px the resizer unmounts, so focus mode cannot be triggered", async ({ page }) => {
  await openFocus(page);
  // Narrow viewport (below the wide-mode 1200 threshold) → the Inspector
  // overlays the conversation and there is no separator to drag or dblclick.
  await page.setViewportSize({ width: 1100, height: 992 });
  await expect(page.locator('[data-testid="inspector-resizer"]')).toHaveCount(0);

  // Even if focus mode were re-armed later, the class still requires a
  // wide+open Inspector at ≥1440px (LAYOUT §5). Sanity check: the body
  // has no app--focus-mode class while we're narrow.
  await expect(focusModeActive(page)).resolves.toBe(0);
});

test("focus mode keeps the base / wide widths clean — no bleed between the three keys", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  const widthOf = (key: string) =>
    page.evaluate(
      (k) =>
        ((window as unknown as { __settingsLog?: Array<{ key: string; value: string }> }).__settingsLog ?? []).find(
          (entry) => entry.key === k,
        )?.value,
      key,
    );

  // First, drag within the base range to plant a known inspector-width value.
  // The harness uses fallback defaults, so the key is unset until something
  // writes to it.
  let box = (await sep.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 300);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 + 50, box.y + 300, { steps: 4 });
  await page.mouse.up();
  await page.waitForTimeout(50);
  const baseBefore = await widthOf("inspector-width");
  expect(baseBefore).toBeDefined();

  // Enter focus, drag *within* the focus range (900 → 1050, both > threshold),
  // then exit. The focus gesture must not corrupt the persisted base width.
  await sep.dblclick();
  box = (await sep.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 300);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 - 200, box.y + 300, { steps: 6 });
  await page.mouse.up();
  await sep.dblclick();

  const baseAfter = await widthOf("inspector-width");
  expect(baseAfter).toBe(baseBefore);
  // And the focus key recorded the in-focus drag value.
  const focusWidth = await widthOf("inspector-focus-width");
  expect(Number(focusWidth)).toBeGreaterThanOrEqual(900);
});

test("at narrower viewports the focus class still applies and the Inspector is clamped to fit", async ({ page }) => {
  // 1280px is the common laptop viewport — below the 1440px threshold the
  // older gate used. The fix removed the gate; CSS now caps the Inspector
  // so it can't push past the available room.
  await page.setViewportSize({ width: 1280, height: 800 });
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  await expect(sep).toBeVisible();

  const box = (await sep.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 200);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 - 700, box.y + 200, { steps: 8 });
  await page.mouse.up();

  await expect(focusModeActive(page)).resolves.toBe(1);
  await expect.poll(() => mainWidth(page)).toBe(300);

  // Inspector width must be ≤ viewport - sidebar(260) - 300 (focus column)
  // - 6 (resizer). 1280 - 566 = 714.
  const panel = await panelWidth(page);
  expect(panel).toBeLessThanOrEqual(714);
  expect(panel).toBeGreaterThan(300);
});
