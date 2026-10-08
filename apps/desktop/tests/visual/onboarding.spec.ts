import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { project, stubShell } from "./shell";

/**
 * First-run Auto Onboarding (Dia-style): plays itself at startup, once.
 * These tests hold the product rules — the visual baselines never cover it
 * (demo routes and the plain browser skip onboarding by design).
 */

function firstRun(): Parameters<typeof stubShell>[1] {
  return {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [] },
    branch: "main",
    settings: { "onboarding-done": "false" },
  };
}

async function settingsLog(page: Page) {
  return page.evaluate(
    () =>
      (window as unknown as { __settingsLog?: Array<{ key: string; value: string }> }).__settingsLog ?? [],
  );
}

test("first launch plays the auto onboarding over the shell", async ({ page }) => {
  await stubShell(page, firstRun());
  await page.goto("/");

  const onb = page.locator('[data-testid="onboarding"]');
  await expect(onb).toBeVisible();
  await expect(onb).toContainText("Get code done.");
  await expect(page.locator('[data-testid="onboarding-skip"]')).toBeVisible();
  await expect(page.locator('[data-testid="onboarding-start"]')).toHaveCount(0);

  // Dia-style geometry: full-viewport dialog, hero title, bottom timing bars.
  const box = await onb.boundingBox();
  expect(box?.width).toBeGreaterThanOrEqual(1580);
  expect(box?.height).toBeGreaterThanOrEqual(980);
  const titleSize = await page
    .locator(".onb-title")
    .evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
  expect(titleSize).toBeGreaterThanOrEqual(32);
  await expect(page.locator(".onb-dot")).toHaveCount(4);

  // The stage sits centered in the viewport.
  const stage = await page.locator('[data-testid="onboarding-stage"]').boundingBox();
  expect(stage).not.toBeNull();
  expect(Math.abs((stage!.x + stage!.width / 2) - 1586 / 2)).toBeLessThanOrEqual(8);
  expect(Math.abs((stage!.y + stage!.height / 2) - 992 / 2)).toBeLessThanOrEqual(24);
});

test("stages advance themselves without any click", async ({ page }) => {
  await stubShell(page, firstRun());
  await page.goto("/");

  const onb = page.locator('[data-testid="onboarding"]');
  await expect(onb).toContainText("Get code done.");
  // ~3.2s cadence — no user input anywhere in this test.
  await expect(onb).toContainText("一句话，开始一个任务", { timeout: 6000 });
});

test("跳过 dismisses the onboarding and stamps it done", async ({ page }) => {
  await stubShell(page, firstRun());
  await page.goto("/");

  await expect(page.locator('[data-testid="onboarding"]')).toBeVisible();
  await page.locator('[data-testid="onboarding-skip"]').click();

  await expect(page.locator('[data-testid="onboarding"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  expect(await settingsLog(page)).toContainEqual({ key: "onboarding-done", value: "true" });
});

test("the last stage ends on 开始使用, which also stamps it done", async ({ page }) => {
  await stubShell(page, firstRun());
  await page.goto("/");

  await page.locator('[data-testid="onboarding-dot-3"]').click();
  const onb = page.locator('[data-testid="onboarding"]');
  await expect(onb).toContainText("从第一个问题开始");
  await expect(page.locator('[data-testid="onboarding-start"]')).toBeVisible();

  await page.locator('[data-testid="onboarding-start"]').click();
  await expect(onb).toHaveCount(0);
  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  expect(await settingsLog(page)).toContainEqual({ key: "onboarding-done", value: "true" });
});

test("Escape dismisses like 跳过", async ({ page }) => {
  await stubShell(page, firstRun());
  await page.goto("/");

  await expect(page.locator('[data-testid="onboarding"]')).toBeVisible();
  await page.keyboard.press("Escape");

  await expect(page.locator('[data-testid="onboarding"]')).toHaveCount(0);
  expect(await settingsLog(page)).toContainEqual({ key: "onboarding-done", value: "true" });
});

test("a done onboarding never returns on later launches", async ({ page }) => {
  // Default stub settings are pre-onboarded.
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [] },
    branch: "main",
  });
  await page.goto("/");

  await expect(page.locator('[data-testid="onboarding"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
});

test("demo routes never show the onboarding", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await expect(page.locator('[data-testid="onboarding"]')).toHaveCount(0);
  await expect(page.locator(".crumb-current")).toBeVisible();
});

test("capture: onboarding stages at the reference viewport", async ({ page }) => {
  await stubShell(page, firstRun());
  await page.goto("/");
  const onb = page.locator('[data-testid="onboarding"]');
  await expect(onb).toContainText("Get code done.");
  await onb.screenshot({ path: "../../.kodo/shots/onboarding-1-welcome.png" });

  for (let index = 1; index < 4; index += 1) {
    await page.locator(`[data-testid="onboarding-dot-${index}"]`).click();
    await expect(page.locator('[data-testid="onboarding-stage"]')).toBeVisible();
    // Let the entrance settle so the capture shows the resting state.
    await page.waitForTimeout(700);
    await onb.screenshot({ path: `../../.kodo/shots/onboarding-${index + 1}-stage.png` });
  }
});
