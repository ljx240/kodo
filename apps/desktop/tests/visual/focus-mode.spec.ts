import { expect, test, type Page } from "@playwright/test";
import { project, sessionRef, stubShell } from "./shell";

/** The conversation stays mounted when resizing gives its space to the Inspector. */

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

test("drag reaches the 400px minimum, then hides the mounted conversation before release", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  const max = Number(await sep.getAttribute("aria-valuemax"));
  const box = (await sep.boundingBox())!;
  const x = box.x + 3;
  const y = box.y + 300;
  const boundary = x - (max - 400 - 330);

  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(boundary, y);
  await expect.poll(() => mainWidth(page)).toBe(400);
  await expect(page.locator(".main")).toBeVisible();

  await page.mouse.move(boundary - 1, y);
  await expect(page.locator(".main")).toBeHidden();
  await expect(page.locator(".main")).toHaveCount(1);
  await expect.poll(() => panelWidth(page)).toBe(max);
  await expect(page.locator(".app.app--resizing")).toHaveCount(1);
  await page.mouse.up();
  await expect(focusModeActive(page)).resolves.toBe(1);
  await expect(sep).toHaveAttribute("aria-valuenow", String(max));
});

test("reversing the same drag immediately restores the conversation, then continues resizing", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  const max = Number(await sep.getAttribute("aria-valuemax"));
  const box = (await sep.boundingBox())!;
  const x = box.x + 3 - (max - 400 - 330) - 50;
  const y = box.y + 300;

  await page.mouse.move(box.x + 3, y);
  await page.mouse.down();
  await page.mouse.move(x, y);
  await expect(page.locator(".main")).toBeHidden();

  await page.mouse.move(x + 1, y);
  await expect(page.locator(".main")).toBeVisible();
  await expect.poll(() => mainWidth(page)).toBe(401);
  await page.mouse.move(x + 49, y);
  await expect.poll(() => mainWidth(page)).toBe(449);
  await page.mouse.up();
  await expect.poll(() => panelWidth(page)).toBe(max - 449);
  await expect(focusModeActive(page)).resolves.toBe(0);
});

test("a new rightward drag restores a hidden conversation on its first pixel", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  await sep.dblclick();
  await expect(page.locator(".main")).toBeHidden();
  const box = (await sep.boundingBox())!;
  const y = box.y + 300;

  await page.mouse.move(box.x + 3, y);
  await page.mouse.down();
  await page.mouse.move(box.x + 4, y);
  await expect.poll(() => mainWidth(page)).toBe(401);
  await page.mouse.up();
  await expect(page.locator(".main")).toBeVisible();
});

test("double-click restores the saved width and keeps the draft", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  await page.locator(".composer-input").fill("保留这份草稿");
  await sep.dblclick();
  await expect(page.locator(".main")).toBeHidden();
  await sep.dblclick();
  await expect(page.locator(".main")).toBeVisible();
  await expect.poll(() => panelWidth(page)).toBe(330);
  await expect(page.locator(".composer-input")).toHaveValue("保留这份草稿");
});

test("keyboard resizing hides and immediately restores the conversation", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  const max = Number(await sep.getAttribute("aria-valuemax"));
  await sep.focus();
  for (let i = 0; i < Math.ceil((max - 400 - 330) / 64); i++) {
    await page.keyboard.press("Shift+ArrowLeft");
  }
  await expect(page.locator(".main")).toBeHidden();
  await expect.poll(() => panelWidth(page)).toBe(max);
  await page.keyboard.press("ArrowRight");
  await expect(page.locator(".main")).toBeVisible();
  await expect.poll(() => mainWidth(page)).toBe(416);
});

test("below 1200px focus mode yields to the overlay and returns on widening", async ({ page }) => {
  await openFocus(page);
  await page.locator('[data-testid="inspector-resizer"]').dblclick();
  await page.setViewportSize({ width: 1100, height: 992 });
  await expect(page.locator('[data-testid="inspector-resizer"]')).toHaveCount(0);
  await expect(focusModeActive(page)).resolves.toBe(0);
  await expect(page.locator(".main")).toBeVisible();

  await page.setViewportSize({ width: 1586, height: 992 });
  await expect(focusModeActive(page)).resolves.toBe(1);
  await expect(page.locator(".main")).toBeHidden();
});

test("focus mode leaves the saved base width intact", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  await sep.focus();
  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => panelWidth(page)).toBe(346);
  await sep.dblclick();
  const box = (await sep.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 300);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 - 100, box.y + 300);
  await page.mouse.up();
  await sep.dblclick();
  await expect.poll(() => panelWidth(page)).toBe(346);
});

test("focus width adapts to the window and sidebar without a fixed cap", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  await sep.dblclick();
  await expect(page.locator(".main")).toBeHidden();
  await expect.poll(() => panelWidth(page)).toBe(1280 - 260 - 40 - 1);

  await page.setViewportSize({ width: 1920, height: 1080 });
  await expect.poll(() => panelWidth(page)).toBe(1920 - 260 - 40 - 1);
  await page.locator(".sidebar-strip button").click();
  await expect.poll(() => panelWidth(page)).toBe(1920 - 40 - 1);
  await expect(sep).toHaveAttribute("aria-valuemax", String(1920 - 40 - 1));
});

test("legacy focus settings fill the current workspace without retaining the old width cap", async ({ page }) => {
  await stubShell(page, {
    ...focusShell(),
    settings: { "inspector-focus-mode": "true", "inspector-focus-width": "900" },
  });
  await page.goto("/");
  await expect(page.locator(".main")).toBeHidden();
  await expect.poll(() => panelWidth(page)).toBe(1586 - 260 - 40 - 1);
  await page.locator('[data-testid="inspector-close"]').click();
  await expect(page.locator(".main")).toBeVisible();
});

test("a narrower window clamps the normal width instead of squeezing the conversation", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  await stubShell(page, { ...focusShell(), settings: { "inspector-width": "1100" } });
  await page.goto("/");
  await expect.poll(() => panelWidth(page)).toBe(1100);
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect.poll(() => mainWidth(page)).toBe(400);
  await expect(page.locator(".main")).toBeVisible();
  await expect.poll(() => panelWidth(page)).toBe(1280 - 260 - 40 - 1 - 400);

  await page.setViewportSize({ width: 1920, height: 1080 });
  await expect.poll(() => panelWidth(page)).toBe(1100);
});

test("narrowing the window during a drag clears drag state and keeps the conversation usable", async ({ page }) => {
  await openFocus(page);
  const box = (await page.locator('[data-testid="inspector-resizer"]').boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 300);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 - 700, box.y + 300);
  await expect(page.locator(".main")).toBeHidden();
  await page.setViewportSize({ width: 1100, height: 800 });
  await expect(page.locator(".app.app--resizing")).toHaveCount(0);
  await expect(page.locator(".main")).toBeVisible();
  await page.mouse.up();
});

test("cancelling a drag restores the saved layout without committing the hidden state", async ({ page }) => {
  await openFocus(page);
  const sep = page.locator('[data-testid="inspector-resizer"]');
  const box = (await sep.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 300);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 - 700, box.y + 300);
  await expect(page.locator(".main")).toBeHidden();
  await sep.dispatchEvent("pointercancel", { pointerId: 1 });
  await page.mouse.up();
  await expect(page.locator(".main")).toBeVisible();
  await expect(page.locator(".app.app--resizing")).toHaveCount(0);
  await expect.poll(() => panelWidth(page)).toBe(330);
});
