import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { project, sessionRef, stubShell } from "./shell";

/**
 * The 文件 panel replaces the old modal (2026-09-30): same FileViewer, now a
 * panel in the Inspector's panel layer. This spec owns the panel's geometry
 * (widen/restore) and the conversation → panel click-throughs — known-good
 * entries, probe-gated entries, and the negatives that must stay inert.
 */

/** Live turn with one changed file and a final markdown answer. */
function filePanelShell(
  answer = "改好了。",
  extra: Record<string, unknown> = {},
) {
  return {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "改文件")],
    },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "改文件",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "改一下 app",
          context: [],
          items: [
            {
              id: 1,
              at: 1_700_000_000,
              status: "done",
              duration: 5,
              kind: "fileChange",
              changes: [
                { path: "src/app.ts", added: 2, removed: 1, edits: 1 },
                { path: "src/util.ts", added: 1, removed: 0, edits: 1 },
              ],
            },
            {
              id: 2,
              at: 1_700_000_000,
              status: "done",
              duration: 1,
              kind: "agentMessage",
              text: answer,
              checks: [],
            },
          ],
          done: true,
          stopped: false,
          error: null,
        },
      ],
    },
    files: ["src/app.ts", "src/util.ts", "README.md"],
    fileBodies: { "src/app.ts": "const a = 1;\nconst b = 2;\n" },
    turnChanges: [
      {
        path: "src/app.ts",
        diff:
          "--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1 +1 @@\n-const a = 0;\n+const a = 1;\n+const b = 2;\n",
      },
    ],
    ...extra,
  };
}

async function openFileTurn(page: Page) {
  await stubShell(page, filePanelShell());
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
}

const panelWidth = (page: Page) =>
  page.locator(".ins-panel").evaluate((el) => Math.round(el.getBoundingClientRect().width));

test("the file panel widens the column to ~640 and restores on demote", async ({ page }) => {
  await openFileTurn(page);

  // Default column: 330 (reference viewport, inspector open).
  await expect.poll(() => panelWidth(page)).toBeCloseTo(330, 0);

  await page.locator('[data-testid="review-files"]').click();
  await expect(page.locator('.ins-panel [data-testid="file-viewer"]')).toBeVisible();
  await expect(page.locator(".ins-tab", { hasText: "文件" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  // Tree + content side by side: the column borrows width while the panel is
  // open (geometry tolerance 8px).
  await expect.poll(() => panelWidth(page)).toBeCloseTo(640, -1);

  // Escape demotes to 概览 — the panel stays open and the column shrinks back.
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-testid="file-viewer"]')).toHaveCount(0);
  await expect(page.locator(".ins-panel")).toBeVisible();
  await expect.poll(() => panelWidth(page)).toBeCloseTo(330, 0);
});

test("path-shaped inline code opens the probed file; non-paths stay plain", async ({ page }) => {
  await stubShell(
    page,
    filePanelShell(
      "完成。主文件 `src/app.ts` 已更新，版本 `v1.2`，文档见 `https://example.com`。",
    ),
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  // Exactly one code span is a path; the version number and the URL in
  // backticks never become buttons.
  const pathButtons = page.locator('[data-testid="md-code-path"]');
  await expect(pathButtons).toHaveCount(1);
  await expect(pathButtons).toHaveText("src/app.ts");
  await expect(page.locator("code.md-inline-code")).toHaveText([
    "v1.2",
    "https://example.com",
  ]);

  await pathButtons.click();
  // Render-time shape gate + click-time probe: the panel opens the full tab.
  await expect(page.locator('.ins-panel [data-testid="file-viewer"]')).toBeVisible();
  await expect(page.locator('[data-testid="viewer-path"]')).toHaveText("src/app.ts");
  await expect(page.locator('[data-testid="file-tab-full"]')).toHaveAttribute(
    "aria-selected",
    "true",
  );
});

test("context cards open project files; external absolute paths stay static", async ({ page }) => {
  await stubShell(
    page,
    filePanelShell("看下附件。", {
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "改文件",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "看下这两个文件",
            context: ["src/app.ts", "/elsewhere/other.txt"],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 1,
                kind: "agentMessage",
                text: "看完了。",
                checks: [],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    }),
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  const inside = page.locator('[data-context-path="src/app.ts"]');
  const outside = page.locator('[data-context-path="/elsewhere/other.txt"]');
  await expect(inside).toHaveCount(1);
  await expect(outside).toHaveCount(1);
  // Project-internal path → the card is one button; an external absolute
  // path never renders as clickable (out of scope for the sidebar panel).
  expect(await inside.evaluate((el) => el.tagName)).toBe("BUTTON");
  expect(await outside.evaluate((el) => el.tagName)).toBe("DIV");

  await inside.click();
  await expect(page.locator('.ins-panel [data-testid="file-viewer"]')).toBeVisible();
  await expect(page.locator('[data-testid="viewer-path"]')).toHaveText("src/app.ts");
});

test("an edit-step file chip in the trace opens the diff tab", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "跑步骤")],
    },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "跑步骤",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "执行",
          context: [],
          items: [
            {
              id: 1,
              at: 1_700_000_000,
              status: "done",
              duration: 5,
              kind: "fileChange",
              changes: [{ path: "src/app.ts", added: 2, removed: 1, edits: 1 }],
            },
          ],
          done: true,
          stopped: false,
          error: null,
        },
      ],
    },
    files: ["src/app.ts", "src/util.ts", "README.md"],
    turnChanges: [
      {
        path: "src/app.ts",
        diff:
          "--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1 +1 @@\n-const a = 0;\n+const a = 1;\n+const b = 2;\n",
      },
    ],
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  // No final reply → the inline trace shows the edit step.
  await expect(page.locator(".trace-row")).toHaveCount(1);
  await page.locator(".trace-row-inner").click();
  const chip = page.locator('[data-testid="trace-file-chip"]');
  await expect(chip).toHaveCount(1);
  await expect(chip).toHaveText("src/app.ts");

  await chip.click();
  // Known-good path: opens straight on 变更, no probe round-trip.
  await expect(page.locator('.ins-panel [data-testid="file-viewer"]')).toBeVisible();
  await expect(page.locator('[data-testid="viewer-path"]')).toHaveText("src/app.ts");
  await expect(page.locator('[data-testid="file-tab-diff"]')).toHaveAttribute(
    "aria-selected",
    "true",
  );
});

test("the width separator drags, clamps, and keeps base/文件 keys apart", async ({ page }) => {
  await openFileTurn(page);

  const sep = page.locator('[data-testid="inspector-resizer"]');
  await expect(sep).toHaveCount(1);
  await expect(sep).toHaveAttribute("role", "separator");
  await expect(sep).toHaveAttribute("aria-orientation", "vertical");
  await expect(sep).toHaveAttribute("aria-valuemin", "320");
  await expect(sep).toHaveAttribute("aria-valuemax", "900");
  await expect(sep).toHaveAttribute("aria-valuenow", "330");

  // Keyboard: ← widens 16 (Shift 64), → narrows back.
  await sep.focus();
  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => panelWidth(page)).toBeCloseTo(346, 0);
  await expect(sep).toHaveAttribute("aria-valuenow", "346");
  await page.keyboard.press("Shift+ArrowLeft");
  await expect.poll(() => panelWidth(page)).toBeCloseTo(410, 0);
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => panelWidth(page)).toBeCloseTo(394, 0);

  // Pointer drag: pull the handle 100px left → +100, shell in drag state.
  let box = (await sep.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 300);
  await page.mouse.down();
  await expect(page.locator(".app.app--resizing")).toHaveCount(1);
  await page.mouse.move(box.x + 3 - 100, box.y + 300, { steps: 4 });
  await expect(sep).toHaveAttribute("aria-valuenow", "494");
  await page.mouse.up();
  await expect(page.locator(".app.app--resizing")).toHaveCount(0);
  await expect.poll(() => panelWidth(page)).toBeCloseTo(494, 0);

  // Clamp high: a long pull stops at the focus threshold (900), and the
  // commit flips focus mode on — the next pull-back exits it again.
  box = (await sep.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 300);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 - 500, box.y + 300, { steps: 6 });
  await page.mouse.up();
  await expect.poll(() => panelWidth(page)).toBeCloseTo(900, 0);
  await expect(page.locator(".app.app--focus-mode")).toHaveCount(1);

  // … and a pull back right stops at 320 (focus mode exits on commit).
  box = (await sep.boundingBox())!;
  await page.mouse.move(box.x + 3, box.y + 300);
  await page.mouse.down();
  await page.mouse.move(box.x + 3 + 600, box.y + 300, { steps: 6 });
  await page.mouse.up();
  await expect.poll(() => panelWidth(page)).toBeCloseTo(320, 0);
  await expect(page.locator(".app.app--focus-mode")).toHaveCount(0);

  // The 文件 panel widens from its own key: 640, independently editable.
  await page.locator('[data-testid="review-files"]').click();
  await expect.poll(() => panelWidth(page)).toBeCloseTo(640, 0);
  await expect(sep).toHaveAttribute("aria-valuenow", "640");
  await sep.focus();
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => panelWidth(page)).toBeCloseTo(624, 0);

  // Demote back: the base key kept its own value (320 from the clamp drag).
  await page.keyboard.press("Escape");
  await expect.poll(() => panelWidth(page)).toBeCloseTo(320, 0);
  await expect(sep).toHaveAttribute("aria-valuenow", "320");

  // Re-open 文件 → its key (624) still stands; the keys never cross.
  await page.locator('[data-testid="review-files"]').click();
  await expect.poll(() => panelWidth(page)).toBeCloseTo(624, 0);
});

test("below 1200px there is no handle — the overlay keeps its own clamp", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 992 });
  await openFileTurn(page);

  await expect(page.locator('[data-testid="inspector-resizer"]')).toHaveCount(0);

  // 文件 opens as an overlay; the ≥1200 widen rule never fires down here.
  await page.locator('[data-testid="review-files"]').click();
  await expect(page.locator('.ins-panel [data-testid="file-viewer"]')).toBeVisible();
  expect(await panelWidth(page)).toBeLessThanOrEqual(340);
});

test("浏览 loads an http(s) site in the widened sandboxed frame", async ({ page }) => {
  await stubShell(page, filePanelShell());
  await page.route("https://example.com/**", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<html><body><h1>hello-embed</h1></body></html>",
    }),
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  // The chip is not demo-gated — it opens the whole panel, rail steps aside.
  await page.locator(".ins-tab", { hasText: "浏览" }).click();
  await expect(page.locator('.ins-panel [data-testid="browse-panel"]')).toBeVisible();
  await expect(page.locator('[data-testid="browse-empty"]')).toBeVisible();
  await expect(page.locator(".ins-rail")).toHaveCount(0);
  // 浏览 widens from the same wide key as 文件.
  await expect.poll(() => panelWidth(page)).toBeCloseTo(640, -1);

  await page.locator('[data-testid="browse-url"]').fill("https://example.com/page");
  await page.locator('[data-testid="browse-go"]').click();

  const frame = page.locator('[data-testid="browse-frame"]');
  await expect(frame).toHaveAttribute("src", "https://example.com/page");
  await expect(
    page.frameLocator('[data-testid="browse-frame"]').locator("h1"),
  ).toHaveText("hello-embed");
  await expect(page.locator('[data-testid="browse-external"]')).toBeVisible();

  // Escape demotes to Inspector — the panel stays open (文件 contract).
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-testid="browse-panel"]')).toHaveCount(0);
  await expect(page.locator(".ins-panel")).toBeVisible();
  await expect(page.locator('.ins-tab[aria-selected="true"]')).toHaveText("Inspector");
});

test("browse refuses unsafe schemes and banners a hanging embed", async ({ page }) => {
  await stubShell(page, filePanelShell());
  // A host that never answers: the frame never fires load.
  await page.route("https://slow.example/**", () => {});
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator(".ins-tab", { hasText: "浏览" }).click();

  // javascript: fails the http(s) whitelist and never reaches an iframe.
  await page.locator('[data-testid="browse-url"]').fill("javascript:alert(1)");
  await page.locator('[data-testid="browse-go"]').click();
  await expect(page.locator('[data-testid="browse-error"]')).toBeVisible();
  await expect(page.locator('[data-testid="browse-frame"]')).toHaveCount(0);

  // A hanging embed surfaces the blocked banner with both escape hatches.
  await page.clock.install();
  await page.locator('[data-testid="browse-url"]').fill("https://slow.example/x");
  await page.locator('[data-testid="browse-go"]').click();
  await expect(page.locator('[data-testid="browse-frame"]')).toBeVisible();
  await page.clock.fastForward(6001);
  await expect(page.locator('[data-testid="browse-blocked"]')).toBeVisible();
  await expect(page.locator('[data-testid="browse-external"]')).toBeVisible();
  await expect(page.locator('[data-testid="browse-retry"]')).toBeVisible();
});
