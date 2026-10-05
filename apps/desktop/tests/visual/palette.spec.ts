import { expect, test } from "@playwright/test";
import { project, sessionRef, stubShell } from "./shell";

/**
 * ⌘K quick switcher + ⌘N new task. Commands always show; the sessions group
 * only exists on a live workspace (the demo fixture keeps the palette to
 * commands alone).
 */

test("⌘K opens the palette, filters commands, and Enter runs one", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [] },
    branch: "main",
  });
  await page.goto("/");
  await page.waitForLoadState("networkidle");

  await page.keyboard.press("ControlOrMeta+k");
  const palette = page.locator('[data-testid="command-palette"]');
  await expect(palette).toBeVisible();
  await expect(palette).toHaveAttribute("aria-modal", "true");
  await expect(page.locator('[data-testid="palette-input"]')).toBeFocused();
  // Commands only — no conversation rows to show.
  await expect(page.locator(".palette-group")).toHaveText(["命令"]);

  await page.locator('[data-testid="palette-input"]').fill("设置");
  await expect(page.locator('[data-testid="palette-item-settings"]')).toBeVisible();
  await expect(page.locator('[data-testid="palette-item-plugins"]')).toHaveCount(0);
  await page.keyboard.press("Enter");

  await expect(palette).toBeHidden();
  await expect(page.locator('[role="dialog"][aria-label="Settings"]')).toBeVisible();
});

test("the palette lists sessions and opens one by keyboard", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [
        sessionRef("s1", "/tmp/ws/alpha", "修复路由"),
        sessionRef("s2", "/tmp/ws/alpha", "优化搭建"),
      ],
    },
    sessions: {
      s2: {
        id: "s2",
        project: "/tmp/ws/alpha",
        title: "优化搭建",
        at: 1_700_000_000,
        archived: false,
        turns: [],
      },
    },
    branch: "main",
  });
  await page.goto("/");
  await page.waitForLoadState("networkidle");

  await page.keyboard.press("ControlOrMeta+k");
  await expect(page.locator(".palette-group")).toHaveText(["命令", "会话"]);
  await expect(page.locator('[data-testid="palette-item-s2"]')).toContainText("alpha");

  await page.locator('[data-testid="palette-input"]').fill("优化搭建");
  await expect(page.locator('[data-testid="palette-item-s1"]')).toHaveCount(0);
  await page.keyboard.press("Enter");

  await expect(page.locator('[data-testid="command-palette"]')).toBeHidden();
  await expect(page.locator(".crumb-current")).toHaveText("优化搭建");
});

test("Escape closes the palette; the demo route keeps it commands + agents", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  await page.keyboard.press("ControlOrMeta+k");
  await expect(page.locator('[data-testid="command-palette"]')).toBeVisible();
  // Demo fixture has the 12 builtin personas loaded — palette lists them
  // under 智能体 (the §1.5 persona source). No session rows since this is
  // the demo route.
  await expect(page.locator(".palette-group")).toHaveText(["命令", "智能体"]);

  await page.keyboard.press("Escape");
  await expect(page.locator('[data-testid="command-palette"]')).toBeHidden();
});

test("⌘N starts a new task — but never steals a keystroke from a field", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "正在看的会话")],
    },
    sessions: {
      s1: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "正在看的会话",
        at: 1_700_000_000,
        archived: false,
        turns: [],
      },
    },
    branch: "main",
  });
  await page.goto("/");
  await page.waitForLoadState("networkidle");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("正在看的会话");

  // Typing in the composer: ⌘N must not interrupt.
  const input = page.locator('[data-testid="composer-input"]');
  await input.focus();
  await page.keyboard.press("ControlOrMeta+n");
  await expect(page.locator(".crumb-current")).toHaveText("正在看的会话");

  // With focus out of any field it lands on the new-task stage.
  await page.locator(".crumb-current").click();
  await page.keyboard.press("ControlOrMeta+n");
  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
});
