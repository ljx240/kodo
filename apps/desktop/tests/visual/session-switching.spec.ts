import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { emit, project, sessionRef, setLoadDelay, setRunState, setSession, stubShell } from "./shell";

/**
 * A run that is still alive when the user switches to another task. The log
 * carries the ask plus the in-flight step; `run_state` carries the live facts.
 */
function runningSession(ask: string) {
  return {
    id: "s1",
    project: "/tmp/ws/alpha",
    title: "长任务",
    at: 1_700_000_000,
    archived: false,
    turns: [
      {
        ask,
        context: [],
        items: [
          {
            id: 1,
            at: 1_700_000_000,
            status: "running",
            duration: null,
            kind: "reasoning",
            summary: "正在执行",
            phase: "execute",
          },
        ],
        done: false,
        stopped: false,
        interrupted: false,
        error: null,
      },
    ],
  };
}

function idleSession(id: string, title: string) {
  return {
    id,
    project: "/tmp/ws/alpha",
    title,
    at: 1_700_000_000,
    archived: false,
    turns: [],
  };
}

function switchParties() {
  return {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [
        sessionRef("s1", "/tmp/ws/alpha", "长任务"),
        sessionRef("s2", "/tmp/ws/alpha", "另一任务"),
      ],
    },
    sessions: {
      s1: idleSession("s1", "长任务"),
      s2: idleSession("s2", "另一任务"),
    },
  };
}

async function openFirstTask(page: Page) {
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation", { hasText: "长任务" }).click();
  await page.locator('[data-testid="composer-input"]').fill("跑一个长任务");
  await page.locator('[data-testid="composer-input"]').press("Enter");
  await emit(page, { type: "turnStarted", session: "s1" });
}

async function openOtherTask(page: Page) {
  await page.locator(".tree-conversation", { hasText: "另一任务" }).click();
  await expect(page.locator(".crumb-current")).toHaveText("另一任务");
}

test("switching away mid-run keeps the task live and restores it on return", async ({ page }) => {
  await stubShell(page, switchParties());
  await openFirstTask(page);
  await emit(page, { type: "progress", session: "s1", phase: "execute", detail: "正在运行测试" });
  await expect(page.locator(".composer-send--stop")).toBeVisible();

  // The runner keeps going while the user reads another task.
  await openOtherTask(page);
  await setSession(page, "s1", runningSession("跑一个长任务"));
  await setRunState(page, "s1", {
    running: true,
    startedAt: Date.now() - 42_000,
    progress: { phase: "execute", detail: "正在运行测试" },
    approval: null,
  });

  await page.locator(".tree-conversation", { hasText: "长任务" }).click();

  // Back on the live task: still running, never painted as interrupted.
  await expect(page.locator(".crumb-current")).toHaveText("长任务");
  await expect(page.locator(".composer-send--stop")).toBeVisible();
  await expect(page.locator('[data-testid="reply-interrupted"]')).toHaveCount(0);
  await expect(page.locator(".trace-row--running")).toHaveCount(1);

  // The rest of the turn lands normally on the restored page.
  await emit(page, {
    type: "itemCompleted",
    session: "s1",
    item: {
      id: 1,
      at: 1_700_000_001,
      status: "done",
      duration: 42_000,
      kind: "reasoning",
      summary: "正在执行",
      phase: "execute",
    },
  });
  await emit(page, {
    type: "itemCompleted",
    session: "s1",
    item: {
      id: 2,
      at: 1_700_000_002,
      status: "done",
      duration: 1,
      kind: "agentMessage",
      text: "长任务完成了。",
      checks: [],
      delivery: "ready",
      verification: "not_run",
      plain: false,
    },
  });
  await emit(page, { type: "turnComplete", session: "s1" });
  await expect(page.locator(".reply .final")).toContainText("长任务完成了。");
  await expect(page.locator(".composer-send--stop")).toHaveCount(0);
});

test("an approval raised while another task is open is actionable after returning", async ({
  page,
}) => {
  await stubShell(page, switchParties());
  await openFirstTask(page);
  await expect(page.locator(".composer-send--stop")).toBeVisible();

  await openOtherTask(page);
  await setSession(page, "s1", runningSession("跑一个长任务"));
  await setRunState(page, "s1", {
    running: true,
    startedAt: Date.now() - 5_000,
    progress: null,
    approval: {
      runId: 7,
      step: 1,
      kind: "Command",
      detail: "rm -rf build  ·  删除构建目录",
      command: "rm -rf build",
      cwd: "/tmp/ws/alpha",
      riskCategory: "DestructiveGit",
      reason: "删除构建目录",
    },
  });

  await page.locator(".tree-conversation", { hasText: "长任务" }).click();

  // The card was never delivered to this page as an event — it comes from the
  // backend's pending-approval snapshot.
  await expect(page.locator(".approval-bar")).toBeVisible();
  await expect(page.locator('[data-testid="approval-command"]')).toContainText("rm -rf build");
  await page.locator('[data-testid="approval-allow"]').click();

  const decision = await page.evaluate(
    () =>
      (
        window as unknown as {
          __approvalLog: Array<{ id: string; step: number; approved: boolean; runId?: number }>;
        }
      ).__approvalLog.at(-1),
  );
  expect(decision).toMatchObject({ id: "s1", step: 1, approved: true, runId: 7 });

  await emit(page, { type: "turnComplete", session: "s1", runId: 7 });
  await expect(page.locator(".composer-send--stop")).toHaveCount(0);
  await expect(page.locator('[data-testid="reply-interrupted"]')).toHaveCount(0);
});

test("a task that finished while away shows its answer on return", async ({ page }) => {  await stubShell(page, switchParties());
  await openFirstTask(page);

  await openOtherTask(page);
  await setSession(page, "s1", {
    ...runningSession("跑一个长任务"),
    turns: [
      {
        ask: "跑一个长任务",
        context: [],
        items: [
          {
            id: 1,
            at: 1_700_000_000,
            status: "done",
            duration: 42_000,
            kind: "agentMessage",
            text: "任务在后台完成了。",
            checks: [],
            delivery: "ready",
            verification: "not_run",
            plain: false,
          },
        ],
        done: true,
        stopped: false,
        interrupted: false,
        error: null,
      },
    ],
  });
  await setRunState(page, "s1", {
    running: false,
    startedAt: null,
    progress: null,
    approval: null,
  });

  await page.locator(".tree-conversation", { hasText: "长任务" }).click();

  await expect(page.locator(".reply .final")).toContainText("任务在后台完成了。");
  await expect(page.locator(".composer-send--stop")).toHaveCount(0);
  await expect(page.locator('[data-testid="reply-interrupted"]')).toHaveCount(0);
});

test("events that land while the log is loading are replayed after it", async ({ page }) => {
  // A slow log read opens the exact gap where a step can finish: the event
  // must not be applied before the snapshot, or the stale log erases it.
  await stubShell(page, switchParties());
  await openFirstTask(page);

  await openOtherTask(page);
  await setSession(page, "s1", runningSession("跑一个长任务"));
  await setRunState(page, "s1", {
    running: true,
    startedAt: Date.now() - 1_000,
    progress: null,
    approval: null,
  });
  await setLoadDelay(page, 400);

  await page.locator(".tree-conversation", { hasText: "长任务" }).click();
  await expect(page.locator(".crumb-current")).toHaveText("长任务");
  await page.waitForTimeout(50);

  await emit(page, {
    type: "itemCompleted",
    session: "s1",
    item: {
      id: 2,
      at: 1_700_000_002,
      status: "done",
      duration: 1,
      kind: "agentMessage",
      text: "加载期间完成了。",
      checks: [],
      delivery: "ready",
      verification: "not_run",
      plain: false,
    },
  });
  await emit(page, { type: "turnComplete", session: "s1" });

  await expect(page.locator(".reply .final")).toContainText("加载期间完成了。");
  await expect(page.locator(".composer-send--stop")).toHaveCount(0);
  await expect(page.locator('[data-testid="reply-interrupted"]')).toHaveCount(0);
});
