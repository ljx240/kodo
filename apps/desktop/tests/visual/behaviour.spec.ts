import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { emit, project, sessionRef, stubShell } from "./shell";

/**
 * A process-only turn with six kinds, no consecutive same-type pairs to fold.
 */
function processOnlyShell() {
  const item = (id: number, fields: Record<string, unknown>) => ({
    id,
    at: 1_700_000_000,
    status: "done",
    duration: 10 * id,
    ...fields,
  });
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
      turns: [
        {
          ask: "执行",
          context: [],
          items: [
            item(1, { kind: "reasoning", summary: "分析需求", phase: "prepare" }),
            item(2, { kind: "search", query: "router", detail: "查找路由" }),
            item(3, { kind: "fileRead", path: "src/a.ts", detail: "查看实现" }),
            item(4, { kind: "commandExecution", command: "cargo check", cwd: "/tmp/ws/alpha", output: "ok", exitCode: 0 }),
            item(5, { kind: "modelCall", model: "deepseek-chat", inputTokens: 100, outputTokens: 10 }),
            item(6, { kind: "fileChange", changes: [{ path: "src/a.ts", added: 1, removed: 0 }] }),
          ],
          done: true,
          stopped: false,
          interrupted: false,
          error: null,
        },
      ],
    },
  };
}

async function openProcessTurn(page: Page) {
  await stubShell(page, processOnlyShell());
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator('[data-testid="process-tools"] > summary').click();
}

test("a step row opens from anywhere along it, not just its chevron", async ({ page }) => {
  await openProcessTurn(page);

  const rows = page.locator(".trace-row");
  // Six loose steps of six kinds — nothing folds away.
  await expect(rows).toHaveCount(6);

  // Completed steps hide their output/diffs until asked for — every open row
  // renders one `.trace-expand` panel, whatever kind of step it is.
  await expect(page.locator(".trace-expand")).toHaveCount(0);

  const first = rows.first();
  await first.locator(".trace-label").click();
  await expect(page.locator(".trace-expand")).toHaveCount(1);

  await first.locator(".trace-label").click();
  await expect(page.locator(".trace-expand")).toHaveCount(0);

  await first.locator(".trace-row-inner").focus();
  await page.keyboard.press("Enter");
  await expect(page.locator(".trace-expand")).toHaveCount(1);

  // Space toggles the row the same way Enter does.
  await page.keyboard.press(" ");
  await expect(page.locator(".trace-expand")).toHaveCount(0);
  await page.keyboard.press(" ");
  await expect(page.locator(".trace-expand")).toHaveCount(1);
});

test("every row reports whether it is open", async ({ page }) => {
  await openProcessTurn(page);

  await expect(page.locator(".trace-row-inner[aria-expanded]")).toHaveCount(6);
  // Only running / failed / denied rows open on their own; these rows are all
  // completed, so none of them starts open.
  await expect(page.locator('.trace-row-inner[aria-expanded="true"]')).toHaveCount(0);
});

test("after the final reply the folded process remains available beside the answer", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "收尾")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "收尾", at: 1_700_000_000, archived: false, turns: [] },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator('[data-testid="composer-input"]').fill("跑一轮");
  await page.locator('[data-testid="composer-input"]').press("Enter");

  await emit(page, { type: "turnStarted", session: "s1" });
  // While the run is live the process is on screen — alternating kinds so
  // nothing folds, and the reference viewport fits 6–8 of these compact rows
  // (UI_ACCEPTANCE §3).
  for (let i = 1; i <= 8; i += 1) {
    await emit(page, {
      type: "itemCompleted",
      session: "s1",
      item:
        i % 2 === 0
          ? { id: i, at: 1_700_000_000, status: "done", duration: 10, kind: "modelCall", model: "deepseek-chat", inputTokens: 5, outputTokens: 5 }
          : { id: i, at: 1_700_000_000, status: "done", duration: 10, kind: "commandExecution", command: `cmd-${i}`, cwd: "/tmp/ws/alpha", output: "ok", exitCode: 0 },
    });
  }
  const rows = page.locator(".trace-row");
  await expect(rows).toHaveCount(8);
  await expect(page.getByTestId("process-tools")).not.toHaveAttribute("open", "");
  await page.locator('[data-testid="process-tools"] > summary').click();
  const visibleRows = await rows.evaluateAll((els) =>
    els.filter((el) => el.getBoundingClientRect().bottom <= window.innerHeight).length,
  );
  expect(visibleRows).toBeGreaterThanOrEqual(6);

  await page.locator('[data-testid="process-tools"] > summary').click();
  // The answer lands; completed details stay folded and can be reopened.
  await emit(page, {
    type: "itemCompleted",
    session: "s1",
    item: { id: 9, at: 1_700_000_000, status: "done", duration: 1, kind: "agentMessage", text: "做好了。", checks: [] },
  });
  await emit(page, { type: "turnComplete", session: "s1" });

  await expect(page.locator('[data-testid="answer-body"]')).toContainText("做好了。");
  await expect(page.getByTestId("process-tools")).not.toHaveAttribute("open", "");
  await expect(rows.first()).not.toBeVisible();
  await page.locator('[data-testid="process-tools"] > summary').click();
  await expect(rows).toHaveCount(8);
  await expect(page.locator('[data-testid="reply-footer"]')).toHaveCount(0);
});

test("consecutive same-type steps fold under one group row, second level per call", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "折叠")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "折叠", at: 1_700_000_000, archived: false, turns: [] },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator(".composer-input").fill("调模型");
  await page.locator(".composer-input").press("Enter");

  const call = (id: number, input: number) => ({
    id,
    at: 1_700_000_000,
    status: "done" as const,
    duration: 1000 * id,
    kind: "modelCall" as const,
    model: "mimo-v2.6-flash",
    inputTokens: input,
    outputTokens: 10,
  });
  await emit(page, { type: "itemCompleted", session: "s1", item: call(1, 1000) });
  await emit(page, { type: "itemCompleted", session: "s1", item: call(2, 2000) });
  await emit(page, { type: "itemCompleted", session: "s1", item: call(3, 3000) });
  await page.locator('[data-testid="process-tools"] > summary').click();

  // One quiet group row — label, 次数, summed duration — collapsed by default
  // because every call is done.
  await expect(page.locator(".trace-row--group")).toHaveCount(1);
  await expect(page.locator(".trace-row--group .trace-label")).toHaveText("调用模型");
  await expect(page.locator('[data-testid="trace-group-count"]')).toHaveText("3 次");
  await expect(page.locator(".trace-row--group .trace-duration")).toHaveText("6.0s");
  await expect(page.locator('[data-testid="trace-group-children"]')).toHaveCount(0);

  // Second level: each call as its own row. Third level: that call's detail.
  await page.locator(".trace-row--group .trace-row-inner").click();
  await expect(page.locator("[data-testid='trace-group-children'] > .trace-row")).toHaveCount(3);
  await page.locator("[data-testid='trace-group-children'] .trace-row-inner").first().click();
  await expect(page.locator("[data-testid='trace-group-children'] .trace-expand")).toContainText(
    "输入 1.0k tokens · 输出 10 tokens",
  );
});

test("a group holding a failure opens itself and keeps the cause visible", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "失败组")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "失败组", at: 1_700_000_000, archived: false, turns: [] },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator(".composer-input").fill("跑测试");
  await page.locator(".composer-input").press("Enter");

  const run = (id: number, command: string, status: "done" | "failed", exitCode: number) => ({
    id,
    at: 1_700_000_000,
    status,
    duration: 100,
    kind: "commandExecution" as const,
    command,
    cwd: "/tmp/ws/alpha",
    output: status === "failed" ? "boom" : "ok",
    exitCode,
  });
  await emit(page, { type: "itemCompleted", session: "s1", item: run(1, "cargo test", "failed", 101) });
  await emit(page, { type: "itemCompleted", session: "s1", item: run(2, "cargo check", "done", 0) });

  const group = page.locator(".trace-row--group");
  await expect(group).toHaveCount(1);
  await expect(group.locator("> .trace-row-inner")).toHaveAttribute("aria-expanded", "true");
  await expect(group.locator("[data-testid='trace-group-children'] > .trace-row")).toHaveCount(2);
  await expect(group.locator('[data-testid="trace-fail-reason"]')).toBeVisible();
});

test("the composer sends on Enter, clears, and stops claiming to be working", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("修复路由");

  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  await expect(page.locator(".welcome-title")).toHaveText("今天想构建什么？");

  // Empty stage: welcome + composer sit in the middle of the pane, not stuck at the bottom.
  const stage = await page.evaluate(() => {
    const main = document.querySelector(".main");
    const welcome = document.querySelector('[data-testid="welcome"]');
    const composer = document.querySelector(".composer");
    if (!main || !welcome || !composer) return null;
    const mainBox = main.getBoundingClientRect();
    const welcomeBox = welcome.getBoundingClientRect();
    const composerBox = composer.getBoundingClientRect();
    return {
      mainMid: mainBox.top + mainBox.height / 2,
      welcomeMid: welcomeBox.top + welcomeBox.height / 2,
      composerBottomFromMainBottom: mainBox.bottom - composerBox.bottom,
      mainHeight: mainBox.height,
    };
  });
  expect(stage).not.toBeNull();
  // Centered within ~120px of the pane midline (heading + composer as one group).
  expect(Math.abs(stage!.welcomeMid - stage!.mainMid)).toBeLessThan(120);
  // Not flush to the bottom edge like the post-message layout.
  expect(stage!.composerBottomFromMainBottom).toBeGreaterThan(48);

  const composer = page.locator(".composer-input");
  await expect(page.locator(".composer-send")).toBeDisabled();

  await composer.fill("帮我看一下未知表路由");
  await expect(page.locator(".composer-send")).toBeEnabled();
  await composer.press("Enter");

  await expect(page.locator(".msg-bubble")).toHaveText("帮我看一下未知表路由");
  await expect(composer).toHaveValue("");

  await expect(page.locator(".reply-working")).toBeVisible();
  await expect(page.locator(".composer-send--stop")).toBeVisible();

  await page.locator(".composer-send--stop").click();
});

test("an empty live conversation keeps Inspector summary cards out of the response section", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  await expect(page.locator(".ins-section-title")).toHaveText(["本轮", "当前项目"]);
  await expect(page.locator(".ins-section", { hasText: "尚未开始响应" })).toBeVisible();
  await expect(page.locator(".status-pill")).toHaveCount(0);
  await expect(page.locator(".ins-section", { hasText: "修改文件" })).toHaveCount(0);
  await expect(page.locator(".ins-section", { hasText: "使用工具" })).toHaveCount(0);
  await expect(page.locator(".ins-section", { hasText: "模型调用" })).toHaveCount(0);
  await expect(page.locator(".ins-section", { hasText: "当前项目" })).toBeVisible();
});

test("Shift+Enter inserts a newline instead of sending", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  const composer = page.locator(".composer-input");
  await composer.click();
  await composer.press("Shift+Enter");
  await composer.type("line2");
  await expect(composer).toHaveValue("\nline2");
});

test("a run draws itself in one step at a time, from the events", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator(".composer-input").fill("跑一下检查");
  await page.locator(".composer-input").press("Enter");

  const step = {
    id: 1,
    at: 1_700_000_000,
    duration: null,
    kind: "commandExecution",
    command: "cargo check",
    cwd: "/tmp/ws/alpha",
    output: "",
    exitCode: null,
  };

  await emit(page, { type: "itemStarted", session: "s1", item: { ...step, status: "running" } });

  await expect(page.locator(".trace-row")).toHaveCount(1);
  await expect(page.locator(".trace-row--running")).toHaveCount(1);
  await expect(page.locator(".trace-duration")).toHaveText("进行中");
  await expect(page.locator(".trace-label")).toHaveText("运行命令");

  await emit(page, {
    type: "itemCompleted",
    session: "s1",
    item: { ...step, status: "done", duration: 1500, output: "Finished dev profile", exitCode: 0 },
  });

  await expect(page.locator(".trace-row--running")).toHaveCount(0);
  await expect(page.locator(".trace-row--done")).toHaveCount(1);
  await expect(page.locator(".trace-duration")).toHaveText("1.5s");
  // A finished command collapses its output until the reader asks for it.
  await expect(page.locator(".trace-output")).toHaveCount(0);
  await page.locator('[data-testid="process-tools"] > summary').click();
  await page.locator(".trace-row-inner").click();
  await expect(page.locator(".trace-output")).toContainText("Finished dev profile");

  await emit(page, { type: "turnComplete", session: "s1" });

  await expect(page.locator(".reply-working")).toHaveCount(0);
  await expect(page.locator(".reply-interrupted")).toHaveCount(0);
});

test("trace rows follow step ids when events arrive out of order", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "顺序")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "顺序", at: 1_700_000_000, archived: false, turns: [] },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator(".composer-input").fill("执行");
  await page.locator(".composer-input").press("Enter");
  const item = (id: number, command: string) => ({
    id,
    at: 1_700_000_000,
    status: "done" as const,
    duration: 1,
    kind: "commandExecution" as const,
    command,
    cwd: "/tmp/ws/alpha",
    output: "ok",
    exitCode: 0,
  });
  await emit(page, { type: "itemCompleted", session: "s1", item: item(2, "second") });
  await emit(page, { type: "itemCompleted", session: "s1", item: item(1, "first") });
  // The two runs are consecutive same-type steps, so they read as one group —
  // and the second level still follows step ids, not arrival order.
  await expect(page.locator(".trace-row--group")).toHaveCount(1);
  await expect(page.locator('[data-testid="trace-group-count"]')).toHaveText("2 次");
  await page.locator('[data-testid="process-tools"] > summary').click();
  await page.locator(".trace-row--group .trace-row-inner").click();
  await expect(page.locator(".code-chip").filter({ hasText: "first" }).first()).toBeVisible();
  const commands = await page
    .locator("[data-testid='trace-group-children'] .code-chip")
    .allTextContents();
  expect(commands).toEqual(["first", "second"]);
});

test("a killed run is reported as interrupted, not as finished or working", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑一下检查",
          items: [{ id: 1, at: 1_700_000_000, status: "running", duration: null, kind: "reasoning", summary: "先看目录" }],
          done: false,
          stopped: false,
          error: null,
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".trace-row--interrupted")).toHaveCount(1);
  await expect(page.locator(".trace-row--running")).toHaveCount(0);
  await expect(page.locator(".trace-row-inner")).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".trace-duration")).toHaveText("已中断");
  await expect(page.locator(".reply-interrupted")).toBeVisible();
  await expect(page.locator(".reply-working")).toHaveCount(0);
  await expect(page.locator(".status-pill--interrupted")).toContainText("已中断");
});

test("a user-stopped run is marked stopped in the trace and Inspector", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑一下检查",
          items: [{ id: 1, at: 1_700_000_000, status: "running", duration: null, kind: "reasoning", summary: "先看目录" }],
          done: false,
          stopped: true,
          error: null,
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".trace-row--stopped")).toHaveCount(1);
  await expect(page.locator(".trace-row--running")).toHaveCount(0);
  await expect(page.locator(".trace-row-inner")).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".reply-stopped")).toBeVisible();
  await expect(page.locator(".reply-working")).toHaveCount(0);
  await expect(page.locator(".reply-interrupted")).toHaveCount(0);
  await expect(page.locator(".status-pill--stopped")).toContainText("已停止");
});

test("a failed run does not leave a spinner on its last step", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑一下检查",
          items: [{ id: 1, at: 1_700_000_000, status: "running", duration: null, kind: "reasoning", summary: "先看目录" }],
          done: false,
          stopped: false,
          error: "模型服务不可用",
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".trace-row--failed")).toHaveCount(1);
  await expect(page.locator(".trace-row--running")).toHaveCount(0);
  await expect(page.locator(".reply-failed")).toContainText("模型服务不可用");
  await expect(page.locator(".status-pill--failed")).toContainText("失败");
});

test("a completed command with a non-zero exit code is rendered as failed", async ({ page }) => {
  await stubShell(
    page,
    {
      workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "检查")] },
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "检查",
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
                status: "done",
                duration: 40,
                kind: "commandExecution",
                command: "identity-check",
                cwd: "/tmp/ws/alpha",
                output: "FAIL",
                exitCode: 1,
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    },
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".trace-row--failed")).toHaveCount(1);
  await expect(page.locator(".trace-row--done")).toHaveCount(0);
});

test("choosing a project in a new task does not open an existing conversation", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha"), project("/tmp/ws/beta", "beta")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "Alpha 会话"), sessionRef("s2", "/tmp/ws/beta", "Beta 会话")],
    },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "Alpha 会话",
      at: 1_700_000_000,
      archived: false,
      turns: [],
    },
    sessions: {
      s2: {
        id: "s2",
        project: "/tmp/ws/beta",
        title: "Beta 会话",
        at: 1_700_000_001,
        archived: false,
        turns: [],
      },
    },
  });
  await page.goto("/");
  await page.getByRole("link", { name: "新建任务" }).click();
  await page.locator('[data-testid="topbar-project-picker"]').click();
  await page.getByRole("menuitem", { name: "beta" }).click();
  await expect(page.locator(".crumb-current")).toHaveText("新对话");
  await expect(page.locator(".tree-conversation--active")).toHaveCount(0);
  await expect(page.locator('[data-testid="topbar-project-name"]')).toHaveText("beta");
});

test("a project row's + starts a new task under that project in one click", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("修复路由");

  // Conversation rows sit tight under their project (UI_ACCEPTANCE §5a).
  const indent = await page.evaluate(() => {
    const tree = document.querySelector(".tree");
    const list = document.querySelector(".tree-conversations");
    return tree && list ? list.getBoundingClientRect().left - tree.getBoundingClientRect().left : null;
  });
  expect(indent).toBeGreaterThan(0);
  expect(indent).toBeLessThanOrEqual(20);

  // + sits left of ⋯ on the project row and is hover-revealed like it.
  await page.locator(".tree-project").hover();
  const add = page.locator(".tree-project-add");
  await expect(add).toHaveAttribute("aria-label", "新建任务");
  const addBox = await add.boundingBox();
  const moreBox = await page.locator(".tree-project .tree-more").boundingBox();
  expect(addBox!.x).toBeLessThan(moreBox!.x);

  await add.click();
  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  await expect(page.locator(".tree-conversation--active")).toHaveCount(0);
  await expect(page.locator('[data-testid="topbar-project-name"]')).toHaveText("alpha");
});

test("final replies hide internal tool protocol notes", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "身份")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "身份",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "你是谁",
          context: [],
          items: [
            {
              id: 1,
              at: 1_700_000_000,
              status: "done",
              duration: 0,
              kind: "agentMessage",
              text: "我是 Kodo。\n**工具协议**：不要展示\n{\"tool_calls\":[{\"name\":\"write_file\"}]}\n欢迎使用。",
              checks: [],
            },
          ],
          done: true,
          stopped: false,
          error: null,
        },
      ],
    },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".final")).toContainText("我是 Kodo。");
  await expect(page.locator(".final")).toContainText("欢迎使用。");
  await expect(page.locator(".final")).not.toContainText("tool_calls");
  await expect(page.locator(".final")).not.toContainText("工具协议");
});

test("tag tool protocol is stripped from the final reply", async ({ page }) => {
  // The mimo wire sample (open function=NAME / parameter=KEY pairs). Built at
  // runtime so this source never carries raw closing tag sequences.
  const lt = String.fromCharCode(60);
  const gt = String.fromCharCode(62);
  const openTool = lt + "tool_call";
  const closeTool = lt + "/tool_call" + gt;
  const openFn = lt + "function=";
  const closeFn = lt + "/function" + gt;
  const openParam = lt + "parameter=";
  const closeParam = lt + "/parameter" + gt;
  const wire =
    openTool +
    openFn + "search" + gt +
    openParam + "query" + gt + "--kodo-canvas" + closeParam +
    closeFn +
    openFn + "read_file" + gt +
    openParam + "path" + gt + "apps/desktop/src/styles/app.css" + closeParam +
    closeFn +
    closeTool;

  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "审查")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "审查",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "帮我审查下当前UI的设计水平并为我打分",
          context: [],
          items: [
            {
              id: 1,
              at: 1_700_000_000,
              status: "done",
              duration: 0,
              kind: "agentMessage",
              text: "审查完成。\n" + wire + "\nUI 设计整体达到可用标准。",
              checks: [],
            },
          ],
          done: true,
          stopped: false,
          error: null,
        },
      ],
    },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  // The prose answers; the protocol never leaks into the reply body.
  await expect(page.locator(".final")).toContainText("审查完成。");
  await expect(page.locator(".final")).toContainText("UI 设计整体达到可用标准。");
  await expect(page.locator(".final")).not.toContainText("function=search");
  await expect(page.locator(".final")).not.toContainText("parameter=query");
  await expect(page.locator(".final")).not.toContainText("--kodo-canvas");
  await expect(page.locator(".final")).not.toContainText("tool_call");
});

/** A live turn that changed one file and answered — enough to show the card. */
function fileTurnShell(extra: Record<string, unknown> = {}) {
  return {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "改文件")] },
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
            { id: 2, at: 1_700_000_000, status: "done", duration: 1, kind: "agentMessage", text: "改好了。", checks: [] },
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
        diff: "--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1 +1 @@\n-const a = 0;\n+const a = 1;\n+const b = 2;\n",
      },
    ],
    ...extra,
  };
}

test("review opens the sidebar file panel with a tree, full file and diff tabs", async ({ page }) => {
  await stubShell(page, fileTurnShell());
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await page.locator('[data-testid="review-files"]').click();
  await expect(page.locator('[data-testid="file-viewer"]')).toBeVisible();
  // The viewer lives inside the Inspector panel — never a modal over the chat.
  await expect(page.locator('.ins-panel [data-testid="file-viewer"]')).toHaveCount(1);
  await expect(page.locator(".ins-tab", { hasText: "文件" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.locator(".settings-modal-backdrop")).toHaveCount(0);

  // Tree: directories and files of the project.
  await expect(page.locator('[data-testid="tree-toggle-src"]')).toBeVisible();
  await expect(page.locator('[data-testid="tree-file-src/app.ts"]')).toBeVisible();
  await expect(page.locator('[data-testid="tree-file-README.md"]')).toBeVisible();

  // Opens at the first changed path, on its diff.
  await expect(page.locator('[data-testid="viewer-path"]')).toHaveText("src/app.ts");
  await expect(page.locator('[data-testid="viewer-diff"]')).toContainText("const a = 1;");
  await expect(page.locator('[data-testid="viewer-diff"]')).toContainText("const a = 0;");

  // 完整文件 shows the file body with line numbers, syntax-highlighted.
  await page.locator('[data-testid="file-tab-full"]').click();
  await expect(page.locator('[data-testid="viewer-code"]')).toContainText("const a = 1;");
  await expect(page.locator('[data-testid="viewer-code"] .viewer-ln').first()).toHaveText("1");
  await expect(page.locator('[data-testid="viewer-code"] .hljs-keyword').first()).toHaveText("const");
  await expect(page.locator('[data-testid="viewer-code"] .hljs-number').first()).toHaveText("1");

  // 变更 comes back for this file.
  await page.locator('[data-testid="file-tab-diff"]').click();
  await expect(page.locator('[data-testid="viewer-diff"]')).toBeVisible();

  // A changed file with no stored diff body still exposes 变更 — a pure
  // create synthesizes its additions from the file content (Image #11: a
  // +n -0 badge with no way to see the change was the bug).
  await page.locator('[data-testid="tree-file-src/util.ts"]').click();
  await expect(page.locator('[data-testid="file-tab-diff"]')).toHaveText("变更");
  await expect(page.locator('[data-testid="viewer-diff"] .is-add')).toHaveCount(2);
  await expect(page.locator('[data-testid="viewer-diff"]')).toContainText("full content of src/util.ts");

  // X from 文件 demotes to Inspector — the panel itself stays open.
  await page.locator('[data-testid="inspector-close"]').click();
  await expect(page.locator('[data-testid="file-viewer"]')).toHaveCount(0);
  await expect(page.locator(".ins-panel")).toBeVisible();
  await expect(page.locator(".ins-tab", { hasText: "Inspector" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});

test("a changed-file row opens the file viewer at that file", async ({ page }) => {
  await stubShell(page, fileTurnShell());
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await page.locator('[data-testid="changed-open-src/util.ts"]').click();
  await expect(page.locator('[data-testid="file-viewer"]')).toBeVisible();
  await expect(page.locator('[data-testid="viewer-path"]')).toHaveText("src/util.ts");
  // The row opens on 变更 even when the changeset stored no diff body.
  await expect(page.locator('[data-testid="file-tab-diff"]')).toHaveText("变更");
  await expect(page.locator('[data-testid="viewer-diff"]')).toContainText("full content of src/util.ts");

  // Escape demotes the panel to Inspector — it does not close the sidebar.
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-testid="file-viewer"]')).toHaveCount(0);
  await expect(page.locator(".ins-panel")).toBeVisible();
});

test("the top-bar terminal icon opens the bottom terminal panel", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  await page.locator('[data-testid="topbar-terminal"]').click();
  await expect(page.locator('[data-testid="terminal-panel"]')).toBeVisible();
  await expect(page.locator('[data-testid="terminal-panel"]')).toContainText(
    "终端仅在桌面应用中可用。",
  );
  await expect(page.locator('[data-testid="topbar-files"]')).toHaveCount(0);

  await page.locator('[data-testid="terminal-panel"] button[aria-label="关闭"]').click();
  await expect(page.locator('[data-testid="terminal-panel"]')).toHaveCount(0);
});

test("terminal tabs and splits share the bottom panel", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");
  await page.locator('[data-testid="topbar-terminal"]').click();

  const tabs = page.locator('[data-testid="terminal-tab"]');
  const panes = page.locator('[data-testid="terminal-pane"]:visible');
  await expect(tabs).toHaveCount(1);
  await expect(panes).toHaveCount(1);

  await page.locator('[data-testid="terminal-new"]').click();
  await expect(tabs).toHaveCount(2);
  await expect(tabs.nth(1)).toHaveAttribute("aria-selected", "true");
  await expect(panes).toHaveCount(1);

  await page.locator('[data-testid="terminal-split-right"]').click();
  await expect(panes).toHaveCount(2);
  await page.locator('[data-testid="terminal-split-down"]').click();
  await expect(panes).toHaveCount(3);

  // The hover-only X removes just that split, not the tab.
  const lastPane = panes.last();
  await expect(lastPane.locator('[data-testid="terminal-pane-close"]')).toHaveCSS(
    "opacity",
    "0",
  );
  await lastPane.hover();
  await expect(lastPane.locator('[data-testid="terminal-pane-close"]')).toHaveCSS(
    "opacity",
    "1",
  );
  await lastPane.locator('[data-testid="terminal-pane-close"]').click();
  await expect(panes).toHaveCount(2);

  // Closing a tab drops its panes and keeps the remaining tab.
  await page.locator('[data-testid="terminal-tab-close"]').nth(1).click();
  await expect(tabs).toHaveCount(1);
  await expect(panes).toHaveCount(1);
  await expect(page.locator('[data-testid="terminal-panel"]')).toBeVisible();
});

test("terminal split boundaries drag and reset on double click", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");
  await page.locator('[data-testid="topbar-terminal"]').click();
  await page.locator('[data-testid="terminal-split-right"]').click();

  const panes = page.locator('[data-testid="terminal-pane"]:visible');
  const divider = page.locator('[data-testid="terminal-divider"]').first();
  await expect(panes).toHaveCount(2);
  await expect(divider).toHaveCount(1);

  const before = await panes.nth(0).boundingBox();
  const handle = await divider.boundingBox();
  if (!before || !handle) throw new Error("terminal split geometry missing");
  const handleX = handle.x + handle.width / 2;
  const handleY = handle.y + handle.height / 2;

  await page.mouse.move(handleX, handleY);
  await page.mouse.down();
  await page.mouse.move(handleX + 140, handleY, { steps: 6 });
  await page.mouse.up();

  await expect
    .poll(async () => (await panes.nth(0).boundingBox())?.width ?? 0)
    .toBeGreaterThan(before.width + 100);

  // A double click restores the even split.
  await divider.dblclick();
  await expect
    .poll(async () => {
      const [left, right] = await Promise.all([
        panes.nth(0).boundingBox(),
        panes.nth(1).boundingBox(),
      ]);
      return left && right ? Math.abs(left.width - right.width) : Infinity;
    })
    .toBeLessThan(4);
});

test("the bottom terminal height drags and survives a close", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");
  await page.locator('[data-testid="topbar-terminal"]').click();

  const panel = page.locator('[data-testid="terminal-panel"]');
  const before = await panel.boundingBox();
  const grip = await page.locator(".terminal-resize").boundingBox();
  if (!before || !grip) throw new Error("terminal panel geometry missing");

  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(grip.x + grip.width / 2, grip.y - 90, { steps: 6 });
  await page.mouse.up();

  await expect
    .poll(async () => (await panel.boundingBox())?.height ?? 0)
    .toBeGreaterThan(before.height + 60);

  // The chosen height is stored and restored with the panel.
  await page.locator('[data-testid="terminal-close"]').click();
  await expect(panel).toHaveCount(0);
  await page.locator('[data-testid="topbar-terminal"]').click();
  await expect
    .poll(async () => (await panel.boundingBox())?.height ?? 0)
    .toBeGreaterThan(before.height + 60);
});

test("a created file shows its change content even with no stored diff body", async ({ page }) => {
  await stubShell(
    page,
    fileTurnShell({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "写介绍",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "写一份介绍",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 5,
                kind: "fileChange",
                changes: [{ path: "docs/KODO_INTRO.md", added: 3, removed: 0, edits: 1 }],
              },
              {
                id: 2,
                at: 1_700_000_000,
                status: "done",
                duration: 1,
                kind: "agentMessage",
                text: "写好了。",
                checks: [],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
      files: ["docs/KODO_INTRO.md", "src/app.ts"],
      fileBodies: { "docs/KODO_INTRO.md": "# Kodo\n\n最小编程助手。\n" },
      // The changeset kept the path but no usable body — the historical
      // clobber/empty case. The badge alone must never be the whole story.
      turnChanges: [{ path: "docs/KODO_INTRO.md", diff: "" }],
    }),
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator('[data-testid="review-files"]').click();
  await expect(page.locator('[data-testid="file-viewer"]')).toBeVisible();

  // 变更 opens with the create rendered as its added lines.
  await expect(page.locator('[data-testid="viewer-path"]')).toHaveText("docs/KODO_INTRO.md");
  await expect(page.locator('[data-testid="file-tab-diff"]')).toHaveText("变更");
  await expect(page.locator('[data-testid="viewer-diff"] .is-add')).toHaveCount(3);
  await expect(page.locator('[data-testid="viewer-diff"]')).toContainText("# Kodo");
  await expect(page.locator('[data-testid="viewer-diff"]')).toContainText("最小编程助手。");
});

test("markdown and html files offer a rendered preview tab", async ({ page }) => {
  await stubShell(
    page,
    fileTurnShell({
      files: ["src/app.ts", "README.md", "page.html"],
      fileBodies: {
        "README.md": "# 标题\n\n这是 **粗体** 说明。\n",
        "page.html":
          "<h1>Hi</h1><script>window.__pwned = 1; document.querySelector('h1').textContent = 'Hi ok';</script>\n",
      },
    }),
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator('[data-testid="review-files"]').click();
  await expect(page.locator('[data-testid="file-viewer"]')).toBeVisible();

  // Markdown opens on the rendered preview; raw source stays one tab away.
  await page.locator('[data-testid="tree-file-README.md"]').click();
  await expect(page.locator('[data-testid="file-tab-preview"]')).toHaveText("预览");
  await expect(page.locator('[data-testid="viewer-preview"]')).toBeVisible();
  await expect(page.locator('[data-testid="viewer-preview"]')).toContainText("标题");
  await expect(page.locator('[data-testid="viewer-preview"]')).toContainText("粗体");
  await page.locator('[data-testid="file-tab-full"]').click();
  await expect(page.locator('[data-testid="viewer-code"]')).toContainText("# 标题");
  await expect(page.locator('[data-testid="viewer-code"] .viewer-ln').first()).toHaveText("1");

  // HTML runs its scripts inside an isolated sandbox — the effect is visible
  // in the frame, but the page cannot touch the parent window.
  await page.locator('[data-testid="tree-file-page.html"]').click();
  await expect(page.locator('[data-testid="viewer-html-preview"]')).toBeVisible();
  await expect(page.locator('[data-testid="viewer-html-preview"]')).toHaveAttribute(
    "sandbox",
    "allow-scripts",
  );
  const srcdoc = await page.locator('[data-testid="viewer-html-preview"]').getAttribute("srcdoc");
  expect(srcdoc ?? "").toContain("Hi");
  const frame = page.frameLocator('[data-testid="viewer-html-preview"]');
  await expect(frame.locator("h1")).toHaveText("Hi ok");
  expect(await page.evaluate(() => (window as { __pwned?: number }).__pwned)).toBeUndefined();
  await expect(page.locator('[data-testid="viewer-preview"]')).toContainText("脚本已启用");
});

test("an image file previews inline — no read failure, no 完整文件 tab", async ({ page }) => {
  await stubShell(
    page,
    fileTurnShell({
      files: ["src/app.ts", "logo.png", "big.png"],
      fileBodies: {},
      oversizedImages: ["big.png"],
    }),
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator('[data-testid="review-files"]').click();
  await expect(page.locator('[data-testid="file-viewer"]')).toBeVisible();

  // Regular image → the probe's inline data URL, never the binary-read error.
  await page.locator('[data-testid="tree-file-logo.png"]').click();
  await expect(page.locator('[data-testid="viewer-image"]')).toHaveAttribute("src", /^data:image\/png/);
  await expect(page.locator('[data-testid="file-tab-full"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="viewer-error"]')).toHaveCount(0);

  // >4MB (no inline preview) → kind card instead of a broken <img>.
  await page.locator('[data-testid="tree-file-big.png"]').click();
  await expect(page.locator('[data-testid="viewer-kind-card"]')).toBeVisible();
  await expect(page.locator('[data-testid="viewer-image"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="viewer-error"]')).toHaveCount(0);
});

test("office and binary files show a kind card with an open-external action", async ({ page }) => {
  await stubShell(
    page,
    fileTurnShell({
      files: ["src/app.ts", "spec.docx"],
      fileBodies: {},
    }),
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator('[data-testid="review-files"]').click();
  await expect(page.locator('[data-testid="file-viewer"]')).toBeVisible();

  await page.locator('[data-testid="tree-file-spec.docx"]').click();
  await expect(page.locator('[data-testid="viewer-kind-card"]')).toBeVisible();
  await expect(page.locator(".fv-kind-badge")).toHaveText("DOC");
  await expect(page.locator(".fv-kind-name")).toHaveText("spec.docx");
  await expect(page.locator(".fv-kind-size")).toHaveText("1.0 KB");
  await expect(page.locator('[data-testid="file-tab-full"]')).toHaveCount(0);

  await page.locator('[data-testid="viewer-open-external"]').click();
  const opened = await page.evaluate(
    () => (window as unknown as { __openFileLog?: string[] }).__openFileLog ?? [],
  );
  expect(opened).toContain("spec.docx");
});

test("csv files render as a table — quoted cells keep their commas", async ({ page }) => {
  await stubShell(
    page,
    fileTurnShell({
      files: ["src/app.ts", "data.csv"],
      fileBodies: {
        "data.csv": 'id,"name, with comma",note\n1,"say ""hi""",ok\n',
      },
    }),
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator('[data-testid="review-files"]').click();
  await expect(page.locator('[data-testid="file-viewer"]')).toBeVisible();

  await page.locator('[data-testid="tree-file-data.csv"]').click();
  const table = page.locator('[data-testid="viewer-csv"]');
  await expect(table).toBeVisible();
  await expect(table.locator("th")).toHaveCount(3);
  await expect(table.locator("th").nth(1)).toHaveText("name, with comma");
  await expect(table.locator("tbody tr")).toHaveCount(1);
  await expect(table.locator("tbody td").nth(1)).toHaveText('say "hi"');
  // Raw source stays one tab away.
  await page.locator('[data-testid="file-tab-full"]').click();
  await expect(page.locator('[data-testid="viewer-code"]')).toContainText("name, with comma");
});

test("the file tree filters by name and down to changed files", async ({ page }) => {
  await stubShell(page, fileTurnShell());
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator('[data-testid="review-files"]').click();
  await expect(page.locator('[data-testid="file-viewer"]')).toBeVisible();
  await expect(page.locator('[data-testid="tree-file-README.md"]')).toBeVisible();

  await page.locator('[data-testid="tree-filter"]').fill("util");
  await expect(page.locator('[data-testid="tree-file-src/util.ts"]')).toBeVisible();
  await expect(page.locator('[data-testid="tree-file-src/app.ts"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="tree-file-README.md"]')).toHaveCount(0);

  await page.locator('[data-testid="tree-filter"]').fill("zzz");
  await expect(page.locator('[data-testid="tree-no-match"]')).toHaveText("没有匹配文件");

  await page.locator('[data-testid="tree-filter"]').fill("");
  // 仅变更 keeps just this turn's files, not the rest of the project.
  await page.locator('[data-testid="tree-only-changed"]').click();
  await expect(page.locator('[data-testid="tree-only-changed"]')).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('[data-testid="tree-file-src/app.ts"]')).toBeVisible();
  await expect(page.locator('[data-testid="tree-file-README.md"]')).toHaveCount(0);
});

test("the diff folds long context and splits side by side", async ({ page }) => {
  const ctx = Array.from({ length: 12 }, (_, index) => ` keep ${index + 1}`).join("\n");
  await stubShell(
    page,
    fileTurnShell({
      turnChanges: [
        {
          path: "src/app.ts",
          diff: "--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1,13 +1,14 @@\n" + ctx + "\n-old\n+new\n",
        },
      ],
    }),
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator('[data-testid="review-files"]').click();
  await expect(page.locator('[data-testid="viewer-diff"]')).toBeVisible();

  // Stats for the selected file sit above the diff.
  await expect(page.locator('[data-testid="diff-stats"]')).toContainText("+2");
  await expect(page.locator('[data-testid="diff-stats"]')).toContainText("-1");

  // Twelve context lines fold to two each side behind 展开 N 行上下文.
  // Collapsed: hunk + 2 head + 2 tail + del + add = 7 lines; expanded +8.
  const fold = page.locator('[data-testid="diff-fold-0"]');
  await expect(fold).toHaveText("展开 8 行上下文");
  await expect(page.locator('[data-testid="viewer-diff"] .viewer-line')).toHaveCount(7);
  await fold.click();
  await expect(page.locator('[data-testid="viewer-diff"] .viewer-line')).toHaveCount(15);

  // 对照 pairs deletions left with additions right.
  await page.locator('[data-testid="diff-mode-split"]').click();
  await expect(page.locator('[data-testid="viewer-diff-split"]')).toBeVisible();
  await expect(page.locator('[data-testid="viewer-diff-split"] .viewer-half.is-del').first()).toContainText("old");
  await expect(page.locator('[data-testid="viewer-diff-split"] .viewer-half.is-add').first()).toContainText("new");
  await page.locator('[data-testid="diff-mode-unified"]').click();
  await expect(page.locator('[data-testid="viewer-diff"]')).toBeVisible();
});

test("undo with nothing to restore says so instead of claiming success", async ({ page }) => {
  await stubShell(page, fileTurnShell({ undoReport: { restored: [], conflicts: [] } }));
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await page.locator('[data-testid="undo-turn"]').click();
  await expect(page.locator('[data-testid="undo-result"]')).toHaveText("本轮没有可撤销的 Kodo 修改。");
});

test("undo reports exactly what it restored", async ({ page }) => {
  await stubShell(
    page,
    fileTurnShell({
      undoReport: {
        restored: ["src/app.ts"],
        conflicts: [{ path: "src/util.ts", reason: "diverged", message: "工作区与记录不一致" }],
      },
    }),
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await page.locator('[data-testid="undo-turn"]').click();
  await expect(page.locator('[data-testid="undo-result"]')).toContainText("已撤销本轮 Kodo 修改。");
  await expect(page.locator('[data-testid="undo-result"]')).toContainText("1 个冲突");
  await expect(page.locator('[data-testid="undo-result"]')).toContainText("src/util.ts");
});

test("a recovered turn stamped interrupted paints interrupted, never working", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑一下检查",
          items: [{ id: 1, at: 1_700_000_000, status: "running", duration: null, kind: "reasoning", summary: "先看目录" }],
          done: false,
          stopped: false,
          interrupted: true,
          error: null,
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".reply-interrupted")).toBeVisible();
  await expect(page.locator(".reply-working")).toHaveCount(0);
});

test("a failover event surfaces failed model, class, and next model", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator(".composer-input").fill("试一下 failover");
  await page.locator(".composer-input").press("Enter");
  await expect(page.locator(".reply-working")).toBeVisible();

  await emit(page, {
    type: "failover",
    session: "s1",
    fromProvider: "GPT-4o",
    fromModel: "gpt-4o",
    errorClass: "RateLimit",
    error: "429",
    toProvider: "Claude Sonnet 5",
    toModel: "claude-sonnet-4-5",
  });

  const note = page.locator('[data-testid="failover-note"]');
  await expect(note).toBeVisible();
  await expect(note).toContainText("GPT-4o");
  await expect(note).toContainText("gpt-4o");
  await expect(note).toContainText("RateLimit");
  await expect(note).toContainText("Claude Sonnet 5");
  await expect(note).toContainText("claude-sonnet-4-5");
});

test("the Trace page's three tabs swap the pane, none of them onto nothing", async ({ page }) => {
  await page.goto("/ui-demo/trace");

  await expect(page.locator(".timeline")).toBeVisible();
  await expect(page.locator(".artifact-list")).toHaveCount(0);

  await page.locator(".page-tab", { hasText: "日志" }).click();
  await expect(page.locator(".timeline")).toHaveCount(0);
  await expect(page.locator(".page-inner > .terminal-block")).toContainText("10:24:32");
  await expect(page.locator(".page-tab--active")).toHaveText("日志");

  await page.locator(".page-tab", { hasText: "产物" }).click();
  await expect(page.locator(".page-inner > .terminal-block")).toHaveCount(0);
  await expect(page.locator(".artifact-list .file-row")).toHaveCount(7);
  await expect(page.locator(".page-tab--active")).toHaveText("产物");

  await page.locator(".page-tab", { hasText: "时间线" }).click();
  await expect(page.locator(".timeline")).toBeVisible();
});

test("the Trace page's three transparency tabs surface the empty note in demo mode", async ({ page }) => {
  await page.goto("/ui-demo/trace");

  // Demo state now ships fixtures for LLM I/O captures and file spans (so
  // the LLM 输入输出 and 文件快照 tabs render real viewers), but no
  // thinking content — the 思考过程 tab is still expected to fall back
  // to its empty-note copy.
  const emptyTabs = ["思考过程"];
  const contentTabs = ["LLM 输入输出", "文件快照"];

  for (const label of emptyTabs) {
    await page.locator(".page-tab", { hasText: label }).click();
    await expect(page.locator(".page-tab--active")).toHaveText(label);
    await expect(page.locator(".empty-note").first()).toBeVisible();
  }

  for (const label of contentTabs) {
    await page.locator(".page-tab", { hasText: label }).click();
    await expect(page.locator(".page-tab--active")).toHaveText(label);
    // The tab body must not be the empty-note placeholder; the demo
    // fixtures are wired through and a real viewer / list is mounted.
    await expect(page.locator(".empty-note")).toHaveCount(0);
  }
});

test("live Trace without a session shows empty state, not fixture", async ({ page }) => {
  await stubShell(page, { workspace: { projects: [], sessions: [] } });
  await page.goto("/trace");
  await expect(page.locator(".empty-note")).toContainText("选择一个会话");
  await expect(page.locator(".timeline")).toHaveCount(0);
});

test("the Response Trace page returns to the conversation", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "返回测试")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "返回测试",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "看轨迹",
          context: [],
          items: [
            { id: 1, at: 1_700_000_000, status: "done", duration: 100, kind: "commandExecution", command: "cargo check", cwd: "/tmp/ws/alpha", output: "ok", exitCode: 0 },
            { id: 2, at: 1_700_000_000, status: "done", duration: 1, kind: "agentMessage", text: "做好了。", checks: [] },
          ],
          done: true,
          stopped: false,
          interrupted: false,
          error: null,
        },
      ],
    },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  // Enter the Response Trace from the reply's bottom action row.
  await page.locator('[data-testid="reply-trace"]').click();
  await expect(page.locator(".timeline")).toBeVisible();

  // One click on the back control lands back on the same conversation.
  await page.locator('[data-testid="trace-back"]').click();
  await expect(page.locator('[data-testid="composer-input"]')).toBeVisible();
  await expect(page.locator(".crumb-current")).toHaveText("返回测试");
});

test("opening the Response Trace mid-run keeps the Q&A alive", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "进行中的任务")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "进行中的任务",
      at: 1_700_000_000,
      archived: false,
      turns: [],
    },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  // A live run in flight: the reply row offers the trace action alone.
  await page.locator('[data-testid="composer-input"]').fill("继续跑");
  await page.locator('[data-testid="composer-input"]').press("Enter");
  await emit(page, { type: "turnStarted", session: "s1" });
  await expect(page.locator('[data-testid="reply-trace"]')).toBeVisible();
  await page.locator('[data-testid="reply-trace"]').click();
  await expect(page.locator('[data-testid="trace-back"]')).toBeVisible();

  // The answer lands while the trace page is showing — it must still be
  // delivered to the kept-alive conversation (2026-09-30).
  await emit(page, {
    type: "itemCompleted",
    session: "s1",
    item: { id: 1, at: 1_700_000_000, status: "done", duration: 5, kind: "agentMessage", text: "跑完了。", checks: [] },
  });

  await page.locator('[data-testid="trace-back"]').click();
  await expect(page.locator(".reply .final")).toContainText("跑完了。");
  await expect(page.locator('[data-testid="reply-interrupted"]')).toHaveCount(0);
  // Still the same live run — the stop control, not a fresh "not running" load.
  await expect(page.locator(".composer-send--stop")).toBeVisible();
  await expect(page.locator('[data-testid="composer-input"]')).toBeVisible();
});

test("selecting a conversation in the sidebar returns to the conversation view", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "返回测试")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "返回测试",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "看轨迹",
          context: [],
          items: [
            { id: 1, at: 1_700_000_000, status: "done", duration: 1, kind: "agentMessage", text: "做好了。", checks: [] },
          ],
          done: true,
          stopped: false,
          interrupted: false,
          error: null,
        },
      ],
    },
  });
  // Arrive on the Response Trace with this conversation selected.
  await page.goto("/trace");
  await expect(page.locator(".timeline")).toBeVisible();

  // Clicking the conversation shows the conversation — never a dead end.
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator('[data-testid="composer-input"]')).toBeVisible();
  await expect(page.locator(".crumb-current")).toHaveText("返回测试");
});

test("an Inspector card's chevron opens and closes it", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.locator(".rail-btn").first().click();

  // Match the section by its own title — 使用工具 also lists a 修改文件 tool row.
  const card = page
    .locator(".ins-section")
    .filter({ has: page.locator(".ins-section-title", { hasText: /^修改文件$/ }) });
  const head = card.locator(".ins-section-head");
  await expect(head).toHaveAttribute("aria-expanded", "true");
  await expect(card.locator(".file-row").first()).toBeVisible();

  await head.click();
  await expect(head).toHaveAttribute("aria-expanded", "false");
  await expect(card.locator(".file-row")).toHaveCount(0);

  await head.click();
  await expect(card.locator(".file-row").first()).toBeVisible();
});

test("each Settings category swaps the detail pane", async ({ page }) => {
  await page.goto("/ui-demo/settings");

  const categories = page.locator(".settings-nav-item");
  await expect(categories).toHaveText([
    "通用",
    "模型",
    "AI Provider",
    "工具与权限",
    "项目",
    "归档与存储",
    "Memory",
    "外观",
    "决策辅助",
  ]);

  for (const name of ["模型", "AI Provider", "工具与权限", "项目", "归档与存储", "Memory", "外观", "决策辅助", "通用"]) {
    await page.locator(".settings-nav-item", { hasText: name }).click();
    await expect(page.locator(".settings-detail h2")).toHaveText(name);
    await expect(page.locator(".settings-detail").locator(".setting-row, .provider-row, .provider-editor, button").first()).toBeVisible();
  }
});

test("an approval request is shown and can be allowed or denied", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator(".composer-input").fill("跑一下检查");
  await page.locator(".composer-input").press("Enter");

  await emit(page, {
    type: "approvalRequest",
    session: "s1",
    step: 1,
    kind: "run command",
    detail: "rm -rf build  ·  删除文件",
  });

  const bar = page.locator(".approval-bar");
  await expect(bar).toBeVisible();
  await expect(bar).toContainText("rm -rf build");
  await bar.locator('[data-testid="approval-deny"]').click();
  await expect(bar).toHaveCount(0);

  await emit(page, {
    type: "approvalRequest",
    session: "s1",
    step: 2,
    kind: "run command",
    detail: "cargo test",
  });
  await expect(bar).toBeVisible();
  await bar.locator(".btn--primary").click();
  await expect(bar).toHaveCount(0);
});

test("permission settings stay synchronized with the composer in both directions", async ({ page }) => {
  await stubShell(page, { settings: { permission: "ask" } });
  await page.goto("/");
  const trigger = page.locator(".composer-perm-trigger");
  await expect(trigger).toHaveText("请求批准");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "工具与权限" }).click();
  const select = page.getByRole("combobox", { name: "权限模式" });
  await select.selectOption("full");
  await expect(trigger).toHaveText("项目内自动执行");
  await expect(trigger).toHaveAttribute("title", /下一轮生效/);
  await expect(select).toHaveValue("full");
  await page.locator('.settings-modal .icon-btn[aria-label="关闭设置"]').click();
  await trigger.click();
  await page.getByRole("menuitem", { name: /^自动批准安全操作/ }).click();
  await expect(trigger).toHaveText("自动批准安全操作");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "工具与权限" }).click();
  await expect(select).toHaveValue("auto");
  await expect(page.locator(".settings-detail")).toContainText("其他命令和外部调用仍需批准");
});

test("a late permission startup read cannot overwrite a new composer choice", async ({ page }) => {
  await stubShell(page, { settings: { permission: "ask" } });
  await page.addInitScript(() => {
    const state = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
      __releasePermissionRead?: () => void;
    };
    const invoke = state.__TAURI_INTERNALS__.invoke;
    state.__TAURI_INTERNALS__.invoke = (command, args) => command === "setting" && args?.key === "permission"
      ? new Promise((resolve) => { state.__releasePermissionRead = () => resolve("ask"); })
      : invoke(command, args);
  });
  await page.goto("/");
  const trigger = page.locator(".composer-perm-trigger");
  await expect.poll(() => page.evaluate(() => typeof (window as unknown as { __releasePermissionRead?: unknown }).__releasePermissionRead)).toBe("function");
  await trigger.click();
  await page.getByRole("menuitem", { name: /^项目内自动执行/ }).click();
  await expect(trigger).toHaveText("项目内自动执行");
  await page.evaluate(() => (window as unknown as { __releasePermissionRead: () => void }).__releasePermissionRead());
  await expect(trigger).toHaveText("项目内自动执行");
});

test("settings toggles persist through the stubbed settings store", async ({ page }) => {
  await stubShell(page, { settings: { "auto-detect-git-branch": "false" } });
  await page.goto("/");

  await page.locator(".sidebar-foot .nav-item").click();
  await expect(page.locator(".settings-modal")).toBeVisible();
  await expect(page).not.toHaveURL(/\/settings$/);

  await page.locator(".settings-nav-item", { hasText: "项目" }).click();
  const toggle = page.locator(".settings-detail .toggle").first();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
});

test("every visible functional setting maps to a runtime consumer", async ({ page }) => {
  await stubShell(page, {
    settings: {
      theme: "system",
      density: "comfortable",
      "show-line-numbers": "false",
      "use-system-font": "true",
      "auto-detect-git-branch": "false",
      "fallback-behavior": "fail",
      "extended-thinking": "true",
      "max-output-tokens": "2048",
      permission: "auto",
      "default-model": "Claude Sonnet 5",
    },
  });
  await page.goto("/");

  // Appearance → DOM consumers
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "外观" }).click();
  await expect(page.locator('.settings-detail select[aria-label="主题"]')).toHaveValue("system");
  await expect(page.locator(".app")).toHaveAttribute("data-density", "comfortable");
  await expect(page.locator(".app")).toHaveAttribute("data-line-numbers", "false");
  await expect(page.locator(".app")).toHaveClass(/app--system-font/);

  // Models → fallback / thinking / max tokens keys
  await page.locator(".settings-nav-item", { hasText: "模型" }).click();
  await expect(page.locator(".settings-detail select").first()).toHaveValue("fail");
  await expect(page.locator('.settings-detail input[role="switch"], .settings-detail .toggle').first()).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await expect(page.locator(".settings-detail input.input").first()).toHaveValue("2048");

  // Projects → auto git branch (App skips gitBranch when false)
  await page.locator(".settings-nav-item", { hasText: "项目" }).click();
  await expect(page.locator(".settings-detail .toggle").first()).toHaveAttribute("aria-checked", "false");

  // Storage → auto-save / trace persistence path documented as append-only
  await page.locator(".settings-nav-item", { hasText: "归档与存储" }).click();
  await expect(page.locator(".settings-detail")).toContainText("settings.log");
});

/** A configured provider whose model list is NOT the template preset. */
const providerFixture = {
  id: "p1",
  name: "OpenAI",
  template: "openai",
  apiKey: "••••1234",
  endpoint: "",
  model: "gpt-4o",
  modelId: "gpt-4o",
  displayName: "GPT-4o",
  models: [
    { modelId: "gpt-4o", displayName: "GPT-4o" },
    { modelId: "o1", displayName: "o1" },
  ],
};

test("context window validates, saves and restores automatic presets", async ({ page }) => {
  await stubShell(page, { settings: { "context-window-tokens": "64000" } });
  await page.goto("/");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "模型" }).click();
  const input = page.getByRole("textbox", { name: "上下文窗口 tokens", exact: true });
  await expect(input).toHaveValue("64000");
  await input.focus();
  await input.press("Tab");
  for (const value of ["0", "-1", "1.5", "1e5", "4294967296", "invalid"]) {
    await input.fill(value);
    await input.press("Tab");
    await expect(input).toHaveAttribute("aria-invalid", "true");
    await expect(page.locator(".settings-detail").getByRole("alert")).toContainText("请输入正整数");
  }
  const writes = () => page.evaluate(() =>
    ((window as unknown as { __settingsLog?: Array<{ key: string; value: string }> }).__settingsLog ?? [])
      .filter((item) => item.key === "context-window-tokens"),
  );
  expect(await writes()).toEqual([]);
  await input.fill("96000");
  await input.press("Enter");
  await expect.poll(writes).toContainEqual({ key: "context-window-tokens", value: "96000" });
  await page.locator(".settings-nav-item", { hasText: "通用" }).click();
  await page.locator(".settings-nav-item", { hasText: "模型" }).click();
  await expect(input).toHaveValue("96000");
  await input.fill("");
  await input.press("Tab");
  await expect.poll(writes).toContainEqual({ key: "context-window-tokens", value: "" });
  await page.locator(".settings-nav-item", { hasText: "通用" }).click();
  await page.locator(".settings-nav-item", { hasText: "模型" }).click();
  await expect(input).toHaveValue("");
  await expect(input).toHaveAttribute("placeholder", "自动（服务商预设）");
  await expect(page.locator(".settings-detail").getByRole("alert")).toHaveCount(0);
});

test("context window reports a failed save and preserves the stored value", async ({ page }) => {
  await stubShell(page, { settings: { "context-window-tokens": "64000" } });
  await page.goto("/");
  await page.evaluate(() => {
    const shell = (window as unknown as {
      __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
    }).__TAURI_INTERNALS__;
    const invoke = shell.invoke;
    shell.invoke = (command, args) => command === "set_setting" && args?.key === "context-window-tokens"
      ? Promise.reject("synthetic settings write failure")
      : invoke(command, args);
  });
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "模型" }).click();
  const input = page.getByRole("textbox", { name: "上下文窗口 tokens", exact: true });
  await expect(input).toHaveValue("64000");
  await input.fill("96000");
  await input.press("Enter");
  await expect(page.locator(".settings-detail").getByRole("alert")).toHaveText("保存失败，请重试。");
  await expect(input).toBeEnabled();
  await expect(input).toHaveValue("96000");
  await page.locator(".settings-nav-item", { hasText: "通用" }).click();
  await page.locator(".settings-nav-item", { hasText: "模型" }).click();
  await expect(input).toHaveValue("64000");
});

test("Models category lists the configured provider models and self-heals a stale default", async ({ page }) => {
  await stubShell(page, {
    providers: [providerFixture],
    settings: { "active-provider": "0", "default-model": "Gone Model" },
  });
  await page.goto("/");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "模型" }).click();

  const select = page.locator('.settings-detail select[aria-label="默认模型"]');
  await expect(select).toHaveValue("gpt-4o");
  await expect(select.locator("optgroup")).toHaveAttribute("label", "OpenAI");
  const log = await page.evaluate(
    () => (window as unknown as { __settingsLog?: Array<{ key: string; value: string }> }).__settingsLog ?? [],
  );
  expect(log).toContainEqual({ key: "default-model", value: "gpt-4o" });
});

test("provider editor auto-loads models and refresh re-fetches", async ({ page }) => {
  await stubShell(page, {
    providers: [providerFixture],
    settings: { "active-provider": "0" },
    providerModels: {
      openai: [
        { modelId: "gpt-4o", displayName: "GPT-4o" },
        { modelId: "gpt-4o-mini", displayName: "GPT-4o mini" },
      ],
    },
  });
  await page.goto("/");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "AI Provider" }).click();
  await page.locator(".provider-row .icon-btn--sm").first().click();

  const listCalls = () =>
    page.evaluate(
      () => (window as unknown as { __listModelCalls?: unknown[] }).__listModelCalls ?? [],
    );
  await expect.poll(async () => (await listCalls()).length).toBe(1);
  await expect(page.locator(".model-chip-id", { hasText: "gpt-4o-mini" })).toBeVisible();

  await page.locator('.model-toolbar .icon-btn[aria-label="刷新模型列表"]').click();
  await expect.poll(async () => (await listCalls()).length).toBe(2);
});

test("a failed model fetch keeps the presets and reports the reason", async ({ page }) => {
  await stubShell(page, {
    providers: [providerFixture],
    settings: { "active-provider": "0" },
    failListModels: "网络请求失败或超时",
  });
  await page.goto("/");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "AI Provider" }).click();
  await page.locator(".provider-row .icon-btn--sm").first().click();

  const status = page.locator(".model-status");
  await expect(status).toContainText("获取模型列表失败");
  await expect(status).toContainText("网络请求失败或超时");
  // Presets stay: the provider's saved models still render as chips.
  await expect(page.locator(".model-chip-id", { hasText: "gpt-4o" })).toBeVisible();
  await expect(page.locator('.model-toolbar .icon-btn[aria-label="刷新模型列表"]')).toBeVisible();
});

test("a manually added model round-trips through save and reopen", async ({ page }) => {
  await stubShell(page, {
    providers: [providerFixture],
    settings: { "active-provider": "0" },
    providerModels: { openai: [{ modelId: "gpt-4o", displayName: "GPT-4o" }] },
  });
  await page.goto("/");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "AI Provider" }).click();
  await page.locator(".provider-row .icon-btn--sm").first().click();

  await page.locator(".model-toolbar .input").fill("my-model");
  await page.locator(".model-toolbar .btn", { hasText: "添加" }).click();
  await page.locator(".provider-editor .btn--primary").click();

  const saved = await page.evaluate(
    () =>
      (window as unknown as {
        __providersLog?: Array<Array<{ models?: Array<{ modelId: string }>; modelId?: string }>>;
      }).__providersLog ?? [],
  );
  const last = saved[saved.length - 1]?.[0];
  expect(last?.models?.some((m) => m.modelId === "my-model")).toBe(true);
  expect(last?.modelId).toBe("my-model");

  // Reopen: the chip is present and marked active.
  await page.locator(".provider-row .icon-btn--sm").first().click();
  const chip = page.locator(".model-chip", { has: page.locator(".model-chip-id", { hasText: "my-model" }) });
  await expect(chip).toHaveClass(/model-chip--active/);
});

test("a provider without a key never fetches and refresh explains why", async ({ page }) => {
  await stubShell(page, {
    providers: [{ ...providerFixture, apiKey: "" }],
    settings: { "active-provider": "0" },
    providerModels: { openai: [{ modelId: "gpt-4o", displayName: "GPT-4o" }] },
  });
  await page.goto("/");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "AI Provider" }).click();
  await page.locator(".provider-row .icon-btn--sm").first().click();

  await page.waitForTimeout(600); // past the 400ms auto-fetch debounce
  let calls = await page.evaluate(
    () => (window as unknown as { __listModelCalls?: unknown[] }).__listModelCalls ?? [],
  );
  expect(calls).toHaveLength(0);

  await page.locator('.model-toolbar .icon-btn[aria-label="刷新模型列表"]').click();
  await expect(page.locator(".model-status")).toContainText("填写 API 密钥后可自动获取模型列表");
  calls = await page.evaluate(
    () => (window as unknown as { __listModelCalls?: unknown[] }).__listModelCalls ?? [],
  );
  expect(calls).toHaveLength(0);
});

/** Mirrors DEMO_MCP_SERVERS: one enabled stdio server with a masked env value
 *  and one disabled http server with a masked header. */
const mcpFixtures = [
  {
    id: "demo-filesystem",
    name: "filesystem",
    enabled: true,
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
    env: { API_TOKEN: "••••9f3a" },
    url: "",
    headers: {},
  },
  {
    id: "demo-remote",
    name: "remote-tools",
    enabled: false,
    transport: "http",
    command: "",
    args: [],
    env: {},
    url: "https://mcp.example.com/mcp",
    headers: { Authorization: "••••c21b" },
  },
];

type McpLogEntry = {
  id?: string;
  name?: string;
  enabled?: boolean;
  transport?: string;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  headers?: Record<string, string>;
};

const mcpLog = (page: import("@playwright/test").Page) =>
  page.evaluate(
    () => (window as unknown as { __mcpServersLog?: unknown[] }).__mcpServersLog ?? [],
  ) as Promise<Array<Array<McpLogEntry>>>;

const mcpToolsCalls = (page: import("@playwright/test").Page) =>
  page.evaluate(
    () => (window as unknown as { __mcpToolsCalls?: unknown[] }).__mcpToolsCalls ?? [],
  ) as Promise<string[]>;

/** MCP is a tab on the 插件 page — one sidebar click plus one tab click. */
test("the sidebar lists 插件 and navigates to its MCP tab", async ({ page }) => {
  await stubShell(page, { mcpServers: mcpFixtures });
  await page.goto("/");

  const nav = page.locator(".nav .nav-item");
  // First-level order per the product spec: 工作台 leads, then the three
  // existing entries keep their behavior. 待办/日历/成果中心 open *inside* the
  // workbench and never appear here.
  await expect(nav.nth(0)).toContainText("工作台");
  await expect(nav.nth(1)).toContainText("新建任务");
  await expect(nav.nth(2)).toContainText("插件");
  await expect(nav.nth(3)).toContainText("定时任务");

  await nav.nth(2).click();
  await expect(page.locator(".page-head h1")).toHaveText("插件");
  await expect(page.locator(".page-head p")).not.toContainText("mcp__服务器__工具名");

  await page.getByTestId("plugins-tab-mcp").click();
  await expect(page).toHaveURL(/type=mcp/);
  await expect(page.locator(".page-head p")).toHaveText("连接外部工具与数据源，管理可用的 MCP 服务器。");
  await expect(page.locator(".page-head p")).not.toContainText("credentials.log");
});

test("MCP servers list shows transport chips, targets and enable toggles", async ({ page }) => {
  await stubShell(page, { mcpServers: mcpFixtures });
  await page.goto("/plugins?type=mcp");

  const rows = page.locator(".page-inner .provider-row");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText("filesystem");
  await expect(rows.nth(0).locator(".code-chip")).toHaveText("stdio");
  await expect(rows.nth(0)).toContainText("npx");
  await expect(rows.nth(0).locator(".toggle")).toHaveAttribute("aria-checked", "true");
  await expect(rows.nth(1)).toContainText("remote-tools");
  await expect(rows.nth(1).locator(".code-chip")).toHaveText("http");
  await expect(rows.nth(1)).toContainText("https://mcp.example.com/mcp");
  await expect(rows.nth(1).locator(".toggle")).toHaveAttribute("aria-checked", "false");
});

test("the MCP editor opens with the stdio server's fields and masked env row", async ({ page }) => {
  await stubShell(page, { mcpServers: mcpFixtures });
  await page.goto("/plugins?type=mcp");
  await page.locator('.page-inner [aria-label="编辑"]').first().click();

  const editor = page.locator(".provider-editor");
  await expect(editor.locator('select[aria-label="传输方式"]')).toHaveValue("stdio");
  await expect(editor.locator('input[placeholder="filesystem"]')).toHaveValue("filesystem");
  await expect(editor.locator('input[placeholder="npx"]')).toHaveValue("npx");
  await expect(editor.locator("input.input").nth(2)).toHaveValue("-y @modelcontextprotocol/server-filesystem /tmp");
  const kv = editor.locator(".kv-row");
  await expect(kv).toHaveCount(1);
  await expect(kv.locator('input[placeholder="名称"]')).toHaveValue("API_TOKEN");
  await expect(kv.locator('input[type="password"]')).toHaveValue("••••9f3a");
  // http-only fields stay out of the stdio form.
  await expect(editor.locator('input[placeholder="https://mcp.example.com/mcp"]')).toHaveCount(0);
});

test("switching transport swaps the fields and clears the other half on save", async ({ page }) => {
  await stubShell(page, { mcpServers: mcpFixtures });
  await page.goto("/plugins?type=mcp");
  await page.locator('.page-inner [aria-label="编辑"]').first().click();

  const editor = page.locator(".provider-editor");
  await editor.locator('select[aria-label="传输方式"]').selectOption("http");
  await expect(editor.locator('input[placeholder="https://mcp.example.com/mcp"]')).toBeVisible();
  await expect(editor.locator('input[placeholder="npx"]')).toHaveCount(0);
  await expect(editor.locator(".kv-row")).toHaveCount(0);

  await editor.locator('input[placeholder="https://mcp.example.com/mcp"]').fill("https://mcp.example.com/mcp");
  await editor.locator(".btn--primary").click();

  const last = (await mcpLog(page)).at(-1)?.find((s) => s.id === "demo-filesystem");
  expect(last?.transport).toBe("http");
  expect(last?.url).toBe("https://mcp.example.com/mcp");
  expect(last?.command).toBe("");
  expect(last?.args).toEqual([]);
  expect(last?.env).toEqual({});
});

test("MCP key/value rows add and remove", async ({ page }) => {
  await stubShell(page, { mcpServers: mcpFixtures });
  await page.goto("/plugins?type=mcp");
  await page.locator('.page-inner [aria-label="编辑"]').first().click();

  const editor = page.locator(".provider-editor");
  const kv = editor.locator(".kv-row");
  await expect(kv).toHaveCount(1);
  await editor.locator(".btn", { hasText: "添加一行" }).click();
  await expect(kv).toHaveCount(2);
  // The fresh row carries the placeholder remove label; clicking it drops the row.
  await editor.locator('[aria-label="删除 名称"]').click();
  await expect(kv).toHaveCount(1);
  await expect(kv.locator('input[placeholder="名称"]')).toHaveValue("API_TOKEN");
});

test("MCP secrets round-trip masked and only a typed value overwrites them", async ({ page }) => {
  await stubShell(page, { mcpServers: mcpFixtures });
  await page.goto("/plugins?type=mcp");

  // Untouched save: the mask goes back unchanged and args split on whitespace.
  await page.locator('.page-inner [aria-label="编辑"]').first().click();
  await page.locator(".provider-editor .btn--primary").click();
  let last = (await mcpLog(page)).at(-1)?.find((s) => s.id === "demo-filesystem");
  expect(last?.env).toEqual({ API_TOKEN: "••••9f3a" });
  expect(last?.args).toEqual(["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]);

  // A typed value replaces the stored secret; the second server keeps its header mask.
  await page.locator('.page-inner [aria-label="编辑"]').first().click();
  await page.locator('.kv-row input[type="password"]').fill("plain-new-value");
  await page.locator(".provider-editor .btn--primary").click();
  await page.locator('.page-inner [aria-label="编辑"]').nth(1).click();
  await page.locator(".provider-editor .btn--primary").click();
  const entries = await mcpLog(page);
  last = entries.at(-1)?.find((s) => s.id === "demo-filesystem");
  expect(last?.env).toEqual({ API_TOKEN: "plain-new-value" });
  expect(entries.at(-1)?.find((s) => s.id === "demo-remote")?.headers).toEqual({
    Authorization: "••••c21b",
  });
});

test("the row toggle flips a server's enabled flag on save", async ({ page }) => {
  await stubShell(page, { mcpServers: mcpFixtures });
  await page.goto("/plugins?type=mcp");

  const rows = page.locator(".page-inner .provider-row");
  await rows.nth(0).locator(".toggle").click();
  await expect(rows.nth(0).locator(".toggle")).toHaveAttribute("aria-checked", "false");
  await rows.nth(1).locator(".toggle").click();
  await expect(rows.nth(1).locator(".toggle")).toHaveAttribute("aria-checked", "true");

  const entries = await mcpLog(page);
  expect(entries.at(-1)?.find((s) => s.id === "demo-filesystem")?.enabled).toBe(false);
  expect(entries.at(-1)?.find((s) => s.id === "demo-remote")?.enabled).toBe(true);
});

test("adding an MCP server starts from the empty hint and creates it enabled", async ({ page }) => {
  await stubShell(page, { mcpServers: [] });
  await page.goto("/plugins?type=mcp");

  await expect(page.locator(".page-inner")).toContainText("还没有配置任何 MCP 服务器");
  await page.locator(".page-inner .btn", { hasText: "添加 MCP 服务器" }).click();

  const editor = page.locator(".provider-editor");
  // The blank custom seed: no catalog prefill, no secret rows.
  await expect(editor.locator('select[aria-label="预设"]')).toHaveCount(0);
  await expect(editor.locator('input[placeholder="filesystem"]')).toHaveValue("");
  await expect(editor.locator('input[placeholder="npx"]')).toHaveValue("");
  await expect(editor.locator(".kv-row")).toHaveCount(0);
  await editor.locator('input[placeholder="filesystem"]').fill("my-server");
  await editor.locator('input[placeholder="npx"]').fill("node");
  await editor.locator(".btn--primary").click();

  const row = page.locator(".page-inner .provider-row");
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("my-server");
  await expect(row.locator(".toggle")).toHaveAttribute("aria-checked", "true");

  const last = (await mcpLog(page)).at(-1)?.[0];
  expect(typeof last?.id).toBe("string");
  expect(last?.id).toBeTruthy();
  expect(last?.name).toBe("my-server");
  expect(last?.enabled).toBe(true);
  expect(last?.transport).toBe("stdio");
  expect(last?.command).toBe("node");
});

test("a catalog card prefills the add editor and its secret keys stay blank", async ({ page }) => {
  await stubShell(page, { mcpServers: [] });
  await page.goto("/plugins?type=mcp");
  await page.locator('.mcp-card[aria-label="添加 github"]').click();

  const editor = page.locator(".provider-editor");
  // The catalog replaced the preset select entirely.
  await expect(editor.locator('select[aria-label="预设"]')).toHaveCount(0);
  await expect(editor).toContainText("来自连接器目录：github");
  await expect(editor.locator('input[placeholder="filesystem"]')).toHaveValue("github");
  await expect(editor.locator('input[placeholder="npx"]')).toHaveValue("npx");
  await expect(editor.locator("input.input").nth(2)).toHaveValue("-y @modelcontextprotocol/server-github");
  const kv = editor.locator(".kv-row");
  await expect(kv).toHaveCount(1);
  await expect(kv.locator('input[placeholder="名称"]')).toHaveValue("GITHUB_PERSONAL_ACCESS_TOKEN");
  await expect(kv.locator('input[type="password"]')).toHaveValue("");

  await editor.locator(".btn--primary").click();
  const last = (await mcpLog(page)).at(-1)?.[0];
  expect(last?.name).toBe("github");
  expect(last?.args).toEqual(["-y", "@modelcontextprotocol/server-github"]);
  expect(last?.env).toEqual({ GITHUB_PERSONAL_ACCESS_TOKEN: "" });
});

test("the connector catalog shows twenty cards with icon, description and transport chip", async ({ page }) => {
  await stubShell(page, { mcpServers: mcpFixtures });
  await page.goto("/plugins?type=mcp");

  const cards = page.locator(".mcp-card");
  await expect(cards).toHaveCount(20);
  await expect(page.locator(".mcp-card-icon svg")).toHaveCount(20);
  // Every card carries its one-line description — a missing catalogDesc key
  // would surface as an empty line here.
  const descs = await page.locator(".mcp-card-desc").allTextContents();
  expect(descs).toHaveLength(20);
  for (const desc of descs) expect(desc.trim().length).toBeGreaterThan(0);

  await expect(page.locator('.mcp-card[aria-label="添加 github"] .code-chip')).toHaveText("stdio");
  await expect(page.locator('.mcp-card[aria-label="添加 notion"] .code-chip')).toHaveText("http");
  const labels = await cards.evaluateAll((nodes) =>
    nodes.map((node) => node.getAttribute("aria-label") ?? ""),
  );
  for (const label of labels) expect(label).toMatch(/^(添加|已添加) /);
});

test("添加 MCP 服务器 opens a blank custom editor with no preset select", async ({ page }) => {
  await stubShell(page, { mcpServers: [] });
  await page.goto("/plugins?type=mcp");
  await page.locator(".page-inner .btn", { hasText: "添加 MCP 服务器" }).click();

  const editor = page.locator(".provider-editor");
  await expect(editor.locator('select[aria-label="预设"]')).toHaveCount(0);
  await expect(editor).not.toContainText("来自连接器目录");
  await expect(editor.locator('input[placeholder="filesystem"]')).toHaveValue("");
  await expect(editor.locator('input[placeholder="npx"]')).toHaveValue("");
  await expect(editor.locator(".kv-row")).toHaveCount(0);
  await editor.locator(".btn", { hasText: "取消" }).click();
  await expect(editor).toHaveCount(0);
});

test("a configured template's card shows 已添加 and the catalog refuses a second instance", async ({ page }) => {
  await stubShell(page, { mcpServers: mcpFixtures });
  await page.goto("/plugins?type=mcp");
  // The fixture already configures filesystem (same name and package), so its
  // card flips to 已添加 and cannot open the add editor again.
  const added = page.locator('.mcp-card[aria-label="已添加 filesystem"]');
  await expect(added).toBeDisabled();
  await expect(added.locator(".mcp-card-state")).toHaveText("已添加");
  await added.click({ force: true });
  await expect(page.locator(".provider-editor")).toHaveCount(0);
  await expect(page.locator(".page-inner .provider-row")).toHaveCount(2);
  expect(await mcpLog(page)).toHaveLength(0);

  // Unconfigured templates stay addable; custom 添加 MCP 服务器 is unlimited and
  // can still create a deliberate second instance (e.g. another fs root).
  await expect(page.locator('.mcp-card[aria-label="添加 github"]')).toBeEnabled();
  await page.locator(".page-inner .btn", { hasText: "添加 MCP 服务器" }).click();
  const editor = page.locator(".provider-editor");
  await editor.locator('input[placeholder="filesystem"]').fill("filesystem-2");
  await editor.locator('input[placeholder="npx"]').fill("npx");
  await editor.locator(".btn--primary").click();
  await expect(page.locator(".page-inner .provider-row")).toHaveCount(3);

  // Deleting the configured instance frees its card again.
  await page.locator('.page-inner [aria-label="删除"]').first().click();
  await expect(page.locator('.mcp-card[aria-label="添加 filesystem"]')).toBeEnabled();
  await expect(page.locator('.mcp-card[aria-label="已添加 filesystem"]')).toHaveCount(0);
});

test("the row chevron lists a server's tools via mcp_list_tools", async ({ page }) => {
  await stubShell(page, {
    mcpServers: mcpFixtures,
    mcpTools: {
      "demo-filesystem": [
        { name: "read_file", description: "Read the complete contents of a file." },
        { name: "list_directory", description: "List entries with [FILE] / [DIR] markers." },
      ],
    },
  });
  await page.goto("/plugins?type=mcp");

  const row = page.locator(".page-inner .provider-row").first();
  await row.locator('[aria-label="查看 filesystem 的工具清单"]').click();

  const tools = page.locator(".mcp-tools .mcp-tool-row");
  await expect(tools).toHaveCount(2);
  await expect(tools.nth(0).locator(".code-chip")).toHaveText("read_file");
  await expect(tools.nth(0)).toContainText("Read the complete contents of a file.");
  // Only this server was contacted (StrictMode double-invokes the effect in
  // dev, so the call count is not asserted — the target is).
  const calls = await mcpToolsCalls(page);
  expect(calls.length).toBeGreaterThan(0);
  expect(new Set(calls)).toEqual(new Set(["demo-filesystem"]));

  // Collapse hides the list without refetching on re-expand… it refetches on
  // mount, which is fine for a manual inspection surface — just re-open.
  await row.locator('[aria-label="查看 filesystem 的工具清单"]').click();
  await expect(page.locator(".mcp-tools")).toHaveCount(0);
});

test("a broken server shows one zh-CN tool-list error line", async ({ page }) => {
  await stubShell(page, { mcpServers: mcpFixtures, failMcpTools: "spawn failed" });
  await page.goto("/plugins?type=mcp");

  await page.locator('.page-inner [aria-label="查看 filesystem 的工具清单"]').click();
  await expect(page.locator(".mcp-tools")).toContainText("无法读取工具清单");
});

test("tools & permissions keeps only the permission mode — the MCP block moved out", async ({ page }) => {
  await stubShell(page, { mcpServers: mcpFixtures });
  await page.goto("/");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "工具与权限" }).click();

  const detail = page.locator(".settings-detail");
  await expect(detail).toContainText("权限模式");
  await expect(detail).not.toContainText("MCP 服务器");
  await expect(detail).not.toContainText("添加 MCP 服务器");
});

test("provider failover switch is visible to the conversation UI", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑一下",
          context: [],
          items: [],
          done: false,
          stopped: false,
          interrupted: false,
          error: null,
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await emit(page, {
    type: "providerSwitch",
    session: "s1",
    fromProvider: "openai",
    fromModel: "gpt-4o",
    errorClass: "RateLimit",
    error: "429 too many requests",
    toProvider: "anthropic",
    toModel: "claude-sonnet-4-5",
  });

  const notice = page.locator('[data-testid="failover-notice"]');
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("openai/gpt-4o");
  await expect(notice).toContainText("anthropic/claude-sonnet-4-5");
  await expect(notice).toContainText("RateLimit");
  await notice.locator('[data-testid="failover-dismiss"]').click();
  await expect(notice).toHaveCount(0);
});

test("recovered interrupted turn is not shown as completed or working", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "中断会话")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "中断会话",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑检查",
          context: [],
          items: [{ id: 1, at: 1_700_000_000, status: "running", duration: null, kind: "reasoning", summary: "先看" }],
          done: false,
          stopped: false,
          interrupted: true,
          error: null,
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".reply-interrupted")).toBeVisible();
  await expect(page.locator(".reply-working")).toHaveCount(0);
  // Trace status on a live session never claims 已完成 for interrupted turn.
  await page.goto("/trace");
  await expect(page.locator(".status-pill")).toContainText("已中断");
});

test("trace page reads persisted session items (trace persistence chain)", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "有痕迹")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "有痕迹",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "看日志",
          context: [],
          items: [
            { id: 1, at: 1_700_000_000, status: "done", duration: 1200, kind: "commandExecution", command: "cargo check", cwd: "/tmp/ws/alpha", output: "ok", exitCode: 0 },
            { id: 2, at: 1_700_000_001, status: "done", duration: 800, kind: "agentMessage", text: "完成", checks: ["通过"] },
          ],
          done: true,
          stopped: false,
          interrupted: false,
          error: null,
        },
      ],
    },
  });

  await page.goto("/trace");
  await expect(page.locator(".status-pill")).toContainText("已完成");
  await expect(page.locator(".timeline, .trace").first()).toBeVisible();
});

test("archive restore is an Inspector action, not a row button", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "已归档", 1_700_000_000, true)],
    },
    archived: [
      {
        id: "s1",
        project: "/tmp/ws/alpha",
        projectName: "alpha",
        title: "已归档",
        at: 1_700_000_000,
        model: "Claude 3.5 Sonnet",
        summary: "排查路由问题",
        filesChanged: 2,
        added: 10,
        removed: 3,
      },
    ],
  });

  await page.goto("/archive");
  await expect(page.locator(".archive-table .arc-title")).toContainText("已归档");
  await expect(page.locator('.archive-table button[aria-label="Restore"]')).toHaveCount(0);

  await page.locator(".archive-table tbody tr").first().click();
  await page.locator(".rail-btn").first().click();
  await expect(page.locator(".ins-actions .btn--primary")).toBeEnabled();
});

test("right-click never shows the WebView default menu", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  const prevented = await page.evaluate(() => {
    let seen = false;
    document.addEventListener("contextmenu", (event) => {
      seen = event.defaultPrevented;
    });
    document.body.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    return seen;
  });
  expect(prevented).toBe(true);
});

test("completion pings the native window on turnComplete and error — not on stop", async ({
  page,
}) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "反馈")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "反馈", at: 1_700_000_000, archived: false, turns: [] },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator('[data-testid="composer-input"]').fill("跑一轮");
  await page.locator('[data-testid="composer-input"]').press("Enter");

  const pings = () =>
    page.evaluate(
      () => (window as unknown as { __attentionLog?: string[] }).__attentionLog ?? [],
    );

  await emit(page, { type: "turnStarted", session: "s1" });
  await emit(page, { type: "turnComplete", session: "s1" });
  await expect.poll(async () => (await pings()).length).toBe(1);

  // A user stop is not a completion — no ping.
  await emit(page, { type: "turnStarted", session: "s1" });
  await emit(page, { type: "stopped", session: "s1" });
  await page.waitForTimeout(100);
  expect(await pings()).toHaveLength(1);

  // Failures ping too (nothing left to wait for).
  await emit(page, { type: "turnStarted", session: "s1" });
  await emit(page, { type: "error", session: "s1", message: "boom" });
  await expect.poll(async () => (await pings()).length).toBe(2);
});

test("file viewer copies the absolute path and reveals the file in Finder", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await stubShell(page, fileTurnShell());
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator('[data-testid="review-files"]').click();
  await expect(page.locator('[data-testid="file-viewer"]')).toBeVisible();

  await page.locator('[data-testid="tree-file-src/app.ts"]').click();
  await expect(page.locator('[data-testid="viewer-copy-path"]')).toBeVisible();

  await page.locator('[data-testid="viewer-copy-path"]').click();
  await expect(page.locator('[data-testid="viewer-copy-path"]')).toHaveAttribute("title", "已复制");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
    "/tmp/ws/alpha/src/app.ts",
  );

  await page.locator('[data-testid="viewer-reveal"]').click();
  const revealed = await page.evaluate(
    () =>
      (window as unknown as { __revealFileLog?: Array<{ project: string; path: string }> })
        .__revealFileLog ?? [],
  );
  expect(revealed).toEqual([{ project: "/tmp/ws/alpha", path: "src/app.ts" }]);
});

test("a provider row tests its connection without opening the editor", async ({ page }) => {
  await stubShell(page, {
    providers: [providerFixture],
    settings: { "active-provider": "0" },
    providerModels: {
      openai: [
        { modelId: "gpt-4o", displayName: "GPT-4o" },
        { modelId: "o1", displayName: "o1" },
      ],
    },
  });
  await page.goto("/");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "AI Provider" }).click();

  await page.locator('[data-testid="provider-test"]').click();
  const result = page.locator('[data-testid="provider-test-result"]');
  await expect(result).toHaveAttribute("data-status", "ok");
  await expect(result).toHaveText("连接成功 · 2 个模型");
  await expect(page.locator(".provider-editor")).toHaveCount(0);

  const calls = await page.evaluate(
    () => (window as unknown as { __listModelCalls?: unknown[] }).__listModelCalls ?? [],
  );
  expect(calls).toHaveLength(1);
});

test("a failed row connection test reports the reason inline", async ({ page }) => {
  await stubShell(page, {
    providers: [providerFixture],
    settings: { "active-provider": "0" },
    failListModels: "网络请求失败或超时",
  });
  await page.goto("/");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "AI Provider" }).click();

  await page.locator('[data-testid="provider-test"]').click();
  const result = page.locator('[data-testid="provider-test-result"]');
  await expect(result).toHaveAttribute("data-status", "fail");
  await expect(result).toHaveText("连接失败（网络请求失败或超时）");
});

test("the composer context ring mirrors the latest ContextBudget event and clears on turn end", async ({ page }) => {
  await stubShell(page, {
    providers: [providerFixture],
    settings: { "active-provider": "0" },
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
      turns: [],
    },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  // Start a turn so the ring is allowed to render.
  await page.locator('[data-testid="composer-input"]').fill("看下进度");
  await page.locator('[data-testid="composer-input"]').press("Enter");
  await emit(page, { type: "turnStarted", session: "s1" });
  await emit(page, { type: "progress", session: "s1", phase: "execute", detail: "step 1" });

  // 6% — muted tone, neutral arc.
  await emit(page, {
    type: "contextBudget",
    session: "s1",
    used: 12_400,
    window: 200_000,
    percent: 6,
    cumulativeChars: 18_400,
    compactedCount: 0,
    lastModel: "Claude Sonnet 5",
  });
  const ring = page.locator('[data-testid="composer-context-ring"]');
  await expect(ring).toBeVisible();
  await expect(ring).toHaveAttribute("data-tone", "muted");
  await expect(ring).toHaveAttribute("data-percent", "6");

  // 75% — warn tone.
  await emit(page, {
    type: "contextBudget",
    session: "s1",
    used: 150_000,
    window: 200_000,
    percent: 75,
    cumulativeChars: 21_000,
    compactedCount: 0,
    lastModel: "Claude Sonnet 5",
  });
  await expect(ring).toHaveAttribute("data-tone", "warn");
  await expect(ring).toHaveAttribute("data-percent", "75");

  // 85% — danger tone.
  await emit(page, {
    type: "contextBudget",
    session: "s1",
    used: 170_000,
    window: 200_000,
    percent: 85,
    cumulativeChars: 24_000,
    compactedCount: 1,
    lastModel: "Claude Sonnet 5",
  });
  await expect(ring).toHaveAttribute("data-tone", "danger");
  await expect(ring).toHaveAttribute("data-percent", "85");

  // Compaction toast appears.
  await emit(page, {
    type: "compacted",
    session: "s1",
    dropped: 8,
    summarizedInto: 1,
    freedPct: 28,
  });
  await expect(page.locator('[data-testid="compact-toast"]')).toContainText("已压缩 8 条");
  await expect(page.locator('[data-testid="compact-toast"]')).toContainText("释放 28%");

  // End the turn — ring and toast must clear together.
  await emit(page, {
    type: "itemCompleted",
    session: "s1",
    item: { id: 1, at: 1_700_000_000, status: "done", duration: 5, kind: "agentMessage", text: "ok。", checks: [] },
  });
  await emit(page, { type: "turnComplete", session: "s1" });
  await expect(ring).toHaveCount(0);
});

test("hovering the composer context ring shows the three-line Codex-style tooltip", async ({ page }, testInfo) => {
  await stubShell(page, {
    providers: [providerFixture],
    settings: { "active-provider": "0" },
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
      turns: [],
    },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await page.locator('[data-testid="composer-input"]').fill("看下进度");
  await page.locator('[data-testid="composer-input"]').press("Enter");
  await emit(page, { type: "turnStarted", session: "s1" });
  await emit(page, { type: "progress", session: "s1", phase: "execute", detail: "step 1" });

  // 44% — neutral band, used 114k / window 258k.
  await emit(page, {
    type: "contextBudget",
    session: "s1",
    used: 114_000,
    window: 258_000,
    percent: 44,
    cumulativeChars: 180_000,
    compactedCount: 0,
    lastModel: "Claude Sonnet 5",
  });

  const ring = page.locator('[data-testid="composer-context-ring"]');
  const tooltip = page.locator('[data-testid="composer-context-ring-tooltip"]');
  // Hidden text must never wrap into a stray character beside the model.
  await expect(tooltip).toHaveCount(1);
  await expect(tooltip).toBeHidden();
  await expect(ring).toHaveAttribute("role", "progressbar");
  await expect(ring).toHaveAttribute("aria-valuenow", "44");
  for (const circle of [".composer-context-ring-bg", ".composer-context-ring-fg"]) {
    expect(await ring.locator(circle).evaluate((el) => getComputedStyle(el).stroke)).not.toBe("none");
  }
  await page.locator(".composer-box").screenshot({ path: testInfo.outputPath("context-ring.png") });
  await ring.hover();
  await expect(tooltip).toBeVisible();
  await expect(tooltip).toContainText("背景信息窗口");
  await expect(tooltip).toContainText("44% 已用");
  await expect(tooltip).toContainText("剩余 56%");
  await expect(tooltip).toContainText("114.0k");
  await expect(tooltip).toContainText("258.0k");
  const control = (await page.locator(".composer-box").boundingBox())!;
  const tip = (await tooltip.boundingBox())!;
  await page.screenshot({
    path: testInfo.outputPath("context-tooltip.png"),
    clip: { x: control.x, y: tip.y - 8, width: control.width, height: control.y + control.height - tip.y + 16 },
  });

  // The popover is readable in either theme, even when the controls wrap.
  for (const theme of ["light", "dark"]) {
    await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
    for (const width of [1586, 900, 640]) {
      await page.setViewportSize({ width, height: 992 });
      await page.mouse.move(0, 0);
      await page.getByTestId("composer-input").focus();
      await expect(tooltip).toBeHidden();
      const before = (await page.locator(".composer-box").boundingBox())!;
      await ring.focus();
      await expect(tooltip).toBeVisible();
      await expect(page.locator(".composer-box")).toHaveCSS("overflow", "visible");
      const after = (await page.locator(".composer-box").boundingBox())!;
      expect(after.height).toBe(before.height);
      const ringBox = (await ring.boundingBox())!;
      const modelBox = (await page.getByTestId("composer-model").boundingBox())!;
      const tooltipBox = (await tooltip.boundingBox())!;
      expect(ringBox.x + ringBox.width).toBeLessThanOrEqual(modelBox.x);
      expect(Math.abs(ringBox.y + ringBox.height / 2 - modelBox.y - modelBox.height / 2)).toBeLessThan(1);
      expect(tooltipBox.y + tooltipBox.height).toBeLessThan(ringBox.y);
      expect(tooltipBox.x).toBeGreaterThanOrEqual(0);
      expect(tooltipBox.x + tooltipBox.width).toBeLessThanOrEqual(width);
      const lines = await tooltip.locator(".composer-context-ring-tooltip-line").evaluateAll((nodes) =>
        nodes.map((node) => node.getBoundingClientRect().y),
      );
      expect(lines).toHaveLength(3);
      expect(lines[0]).toBeLessThan(lines[1]);
      expect(lines[1]).toBeLessThan(lines[2]);
    }
  }
});
