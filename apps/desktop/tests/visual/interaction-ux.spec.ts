import { expect, test } from "@playwright/test";
import { emit, project, sessionRef, stubShell } from "./shell";

function liveCore(extra: Record<string, unknown> = {}) {
  return {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "新对话")],
    },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false, turns: [] },
    providers: [
      {
        id: "p1",
        name: "Local",
        template: "deepseek",
        apiKey: "••••dead",
        endpoint: "",
        model: "deepseek-chat",
        modelId: "deepseek-chat",
        displayName: "DeepSeek-V3",
      },
    ],
    settings: { "active-provider": "0" },
    ...extra,
  };
}

async function openLiveConversation(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".conv-title")).toHaveText("新对话");
}

test("slash inserts a skill tag from the composer popup", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  await page.locator('[data-testid="composer-input"]').fill("/");
  await expect(page.locator('[data-testid="composer-slash"]')).toBeVisible();
  await page.locator('[data-slash-id="bug-fix"]').click();
  await expect(page.locator('[data-testid="composer-input"]')).toHaveValue("【技能：bug-fix】");
});

test("running composer queues the next ask and drains after turnComplete", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  const input = page.locator('[data-testid="composer-input"]');
  await input.fill("第一条");
  await input.press("Enter");
  await expect(page.locator(".reply-working")).toBeVisible();

  await input.fill("第二条排队");
  await input.press("Enter");
  await expect(page.locator('[data-testid="send-queue"]')).toContainText("第二条排队");
  await expect(page.locator('[data-testid="composer-status"]')).toContainText("队列 1");

  await emit(page, { type: "turnComplete", session: "s1" });
  await expect(page.locator(".msg-bubble")).toHaveText(["第一条", "第二条排队"]);
});

test("stopped and error turns are visible on the reply", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "新对话",
        at: 1_700_000_000,
        archived: false,
        turns: [
          { ask: "跑一下", context: [], items: [], done: false, stopped: true, error: null },
          { ask: "再跑", context: [], items: [], done: false, stopped: false, error: "provider unavailable" },
        ],
      },
    }),
  );

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator('[data-testid="reply-stopped"]')).toBeVisible();
  await expect(page.locator('[data-testid="reply-error"]')).toContainText("provider unavailable");
});

test("command step expands with cwd and exit code, failed stays open", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "新对话",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "检查",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "failed",
                duration: 1200,
                kind: "commandExecution",
                command: "cargo check",
                cwd: "/tmp/ws/alpha",
                output: "error: could not compile",
                exitCode: 101,
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

  await expect(page.locator(".trace-mark--failed")).toHaveCount(1);
  await expect(page.locator('[data-testid="trace-run-meta"]')).toContainText("/tmp/ws/alpha");
  await expect(page.locator('[data-testid="trace-run-meta"]')).toContainText("exit 101");
  await expect(page.locator('[data-testid="trace-exit-code"]')).toBeVisible();
});

test("first ask auto-titles the session from the message", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  await page.locator('[data-testid="composer-input"]').fill("修复 k2k 路由问题并补充测试");
  await page.locator('[data-testid="composer-input"]').press("Enter");

  await expect(page.locator(".conv-title")).toHaveText("修复 k2k 路由问题并补充测试");
});

test("model picker lists models inside a provider", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  await page.locator('[data-testid="composer-model"]').click();
  await page.locator('[data-provider-index="0"]').click();
  await expect(page.locator('[data-model-id="deepseek-reasoner"]')).toBeVisible();
  await page.locator('[data-model-id="deepseek-reasoner"]').click();
  await expect(page.locator('[data-testid="composer-model"]')).toHaveText(/DeepSeek-R1/);
});

test("+ project file panel searches and pins a context chip", async ({ page }) => {
  await stubShell(page, liveCore({ files: ["src/app.ts", "src/util.ts", "README.md"] }));
  await openLiveConversation(page);

  await page.locator('.composer button[aria-label="Composer menu"]').click();
  await page.locator('[data-testid="project-files-item"]').click();
  await expect(page.locator('[data-testid="context-file-list"]')).toBeVisible();
  await page.locator('.composer-files-search input').fill("app");
  await page.locator('[data-file-path="src/app.ts"]').click();
  await expect(page.locator('[data-context-path="src/app.ts"]')).toBeVisible();
});

test("final answer renders markdown code blocks", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "修复路由",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "怎么写",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 400,
                kind: "agentMessage",
                text: "用 `run` 即可。\n\n```ts\nrun();\n```\n\n- 第一步\n- 第二步",
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

  await expect(page.locator(".md-code")).toContainText("run();");
  await expect(page.locator(".md-inline-code")).toHaveText("run");
  await expect(page.locator(".md-list li")).toHaveCount(2);
});

test("regenerate re-sends the last ask", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "修复路由",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "检查路由",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 300,
                kind: "agentMessage",
                text: "已检查。",
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

  await expect(page.locator('[data-testid="reply-regenerate"]')).toBeVisible();
  await page.locator('[data-testid="reply-regenerate"]').click();
  await expect(page.locator(".msg-bubble")).toHaveCount(2);
  await expect(page.locator(".msg-bubble").nth(1)).toHaveText("检查路由");

  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string }> }).__sendLog ?? [],
  );
  expect(log.some((entry) => entry.text === "检查路由")).toBe(true);
});

test("approval offers session-wide allow and records it on the invoke", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  await page.locator(".composer-input").fill("跑一下");
  await page.locator(".composer-input").press("Enter");
  await emit(page, {
    type: "approvalRequest",
    session: "s1",
    step: 1,
    kind: "run command",
    detail: "cargo test  ·  测试",
    command: "cargo test",
  });

  await expect(page.locator('[data-testid="approval-allow-session"]')).toBeVisible();
  await page.locator('[data-testid="approval-allow-session"]').click();

  const log = await page.evaluate(
    () =>
      (window as unknown as { __approvalLog?: Array<{ approved: boolean; sessionWide: boolean }> })
        .__approvalLog ?? [],
  );
  expect(log).toHaveLength(1);
  expect(log[0].approved).toBe(true);
  expect(log[0].sessionWide).toBe(true);
});

test("edit step expands to the unified diff from turn_changes", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      turnChanges: [
        {
          path: "src/app.ts",
          diff: "--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1 +1 @@\n-old\n+new\n",
        },
      ],
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "改文件",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "改一下",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 400,
                kind: "fileChange",
                changes: [{ path: "src/app.ts", added: 1, removed: 1 }],
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

  await expect(page.locator('[data-testid="trace-file-list"]')).toBeVisible();
  await expect(page.locator('[data-testid="trace-diff-src/app.ts"]')).toContainText("+new");
});
