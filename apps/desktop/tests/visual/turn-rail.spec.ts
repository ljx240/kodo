import { expect, test } from "@playwright/test";
import { emit, project, sessionRef, stubShell } from "./shell";

/**
 * TurnRail (COMPONENTS §16): the overlay rail in the conversation's right
 * gutter — ≥2 turns, ticks per turn, hover/focus preview, click-to-scroll,
 * active tracking, hidden below 2 turns and under a 900px container.
 */

function liveCore(turns: unknown[], extra: Record<string, unknown> = {}) {
  return {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "新对话")],
    },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "新对话",
      at: 1_700_000_000,
      archived: false,
      turns,
    },
    ...extra,
  };
}

function doneTurn(index: number, ask: string, answer: string) {
  return {
    ask,
    context: [],
    items: [
      {
        id: index + 1,
        at: 1_700_000_000 + index,
        status: "done",
        duration: 400,
        kind: "agentMessage",
        text: answer,
        checks: [],
      },
    ],
    done: true,
    stopped: false,
    error: null,
  };
}

async function openLiveConversation(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("新对话");
}

const scrollTop = (page: import("@playwright/test").Page) =>
  page.evaluate(
    () =>
      (document.querySelector('[data-testid="conversation-scroll"]') as HTMLElement | null)
        ?.scrollTop ?? -1,
  );

test("the demo conversation shows the rail with one tick per turn", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  const rail = page.locator('[data-testid="turn-rail"]');
  await expect(rail).toBeVisible();
  await expect(page.locator('[data-testid="turn-rail-mark"]')).toHaveCount(3);
  // At the top the first turn owns the reading band.
  await expect(page.locator('[data-testid="turn-rail-mark"]').first()).toHaveAttribute(
    "aria-current",
    "true",
  );
  // Overlay: the frame is 28px wide, flush in the stage's right gutter.
  const geometry = await page.evaluate(() => {
    const frame = document.querySelector('[data-testid="turn-rail"]')?.getBoundingClientRect();
    const stage = document.querySelector(".conv-stage")?.getBoundingClientRect();
    const composer = document.querySelector(".composer")?.getBoundingClientRect();
    return {
      width: frame?.width ?? 0,
      rightGap: frame && stage ? stage.right - frame.right : -1,
      frame: frame ?? null,
      composer: composer ?? null,
    };
  });
  expect(Math.abs(geometry.width - 28)).toBeLessThanOrEqual(1);
  expect(Math.abs(geometry.rightGap - 12)).toBeLessThanOrEqual(1);
  // The overlay never reaches the composer's band.
  if (geometry.frame && geometry.composer) {
    expect(geometry.frame.bottom).toBeLessThanOrEqual(geometry.composer.top + 1);
  }
});

test("the rail hides with fewer than two turns and under a 900px container", async ({ page }) => {
  // Zero turns → welcome stage, no rail.
  await stubShell(page, liveCore([]));
  await openLiveConversation(page);
  await expect(page.locator('[data-testid="turn-rail"]')).toHaveCount(0);

  // One turn → nothing to navigate.
  await stubShell(page, liveCore([doneTurn(0, "只问一句", "只答一句。")]));
  await openLiveConversation(page);
  await expect(page.locator('[data-testid="turn-rail"]')).toHaveCount(0);

  // Narrow container (stage ≈ 799px at 1100 viewport): hidden; wide: visible.
  await page.goto("/ui-demo/conversation");
  await page.setViewportSize({ width: 1100, height: 800 });
  await page.waitForLoadState("networkidle");
  await expect(page.locator('[data-testid="turn-rail"]')).toHaveCount(1);
  await expect(page.locator('[data-testid="turn-rail"]')).toBeHidden();
  await page.setViewportSize({ width: 1586, height: 992 });
  await expect(page.locator('[data-testid="turn-rail"]')).toBeVisible();
});

test("clicking a tick jumps to that turn and releases stick-to-bottom", async ({ page }) => {
  await stubShell(
    page,
    liveCore([
      doneTurn(0, "第一问", "第一答。"),
      doneTurn(1, "第二问", "第二答。"),
      doneTurn(2, "第三问", "第三答。"),
    ]),
  );
  await openLiveConversation(page);
  const rail = page.locator('[data-testid="turn-rail"]');
  await expect(rail).toBeVisible();

  // Sit at the bottom of a live run with follow armed — only a composer send
  // sets running, which is what arms the follow effect and .reply-working.
  const input = page.locator('[data-testid="composer-input"]');
  await input.fill("继续");
  await input.press("Enter");
  await expect(page.locator(".reply-working")).toBeVisible();
  await expect.poll(() => scrollTop(page)).toBeGreaterThan(0);

  // …then click tick 0 (frame-relative: inset 6px puts its centre at y=6).
  await rail.click({ position: { x: 14, y: 6 } });
  await expect.poll(() => scrollTop(page)).toBeLessThan(50);
  const atTop = await scrollTop(page);

  // A progress event re-runs the follow effect — with follow released (the
  // 48px threshold in onScroll did it on the jump) it must not yank the view.
  await emit(page, { type: "progress", session: "s1", phase: "搜索", detail: "扫描路由" });
  await expect(page.locator(".reply-working")).toContainText("扫描路由");
  await page.waitForTimeout(100);
  expect(await scrollTop(page)).toBe(atTop);
});

test("hover and focus raise the preview; it never eats pointer events", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  const rail = page.locator('[data-testid="turn-rail"]');
  const preview = page.locator('[data-testid="turn-rail-preview"]');
  await rail.hover({ position: { x: 14, y: 6 } });
  await expect(preview).toBeVisible();
  await expect(preview).toContainText("请检查 k2k-rust 项目的未知表路由实现");
  await expect(preview).toContainText("基于对 k2k-rust 项目的检查");
  const pointerEvents = await preview.evaluate((el) => getComputedStyle(el).pointerEvents);
  expect(pointerEvents).toBe("none");

  await page.mouse.move(4, 4);
  await expect(preview).toHaveCount(0);

  // Focus opens the same preview; the active tick carries aria-current.
  await page.locator('[data-testid="turn-rail-mark"]').nth(1).focus();
  await expect(preview).toBeVisible();
  await expect(preview).toContainText("heartbeat 处理这块只给了结论");
  await expect(page.locator('[data-testid="turn-rail-mark"]').nth(1)).toHaveAttribute(
    "aria-describedby",
    /.+/,
  );
});

test("the active tick tracks the reading position while scrolling", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  // Bring turn 2's user message into the reading band (top 30% of the
  // scrollport — rootMargin "0px 0px -70% 0px", same rule as the Markdown TOC):
  // block:"start" lands it at the band's top edge; center would sit below it.
  await page.locator('.msg-user[data-turn-index="1"]').evaluate((el) =>
    el.scrollIntoView({ block: "start" }),
  );
  await expect(page.locator('[data-testid="turn-rail-mark"]').nth(1)).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(page.locator('[data-testid="turn-rail-mark"]').first()).not.toHaveAttribute(
    "aria-current",
    "true",
  );
});
