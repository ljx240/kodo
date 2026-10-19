import { expect, test, type Page } from "@playwright/test";
import type { ScheduledTaskDto, TaskRunDto } from "../../src/api";
import { project, sessionRef, stubShell } from "./shell";

const now = Math.floor(Date.now() / 1000);
function task(id = "a", title = "每日检查"): ScheduledTaskDto {
  return {
    id,
    title,
    revision: 3,
    scheduleGeneration: 1,
    state: "enabled",
    schedule: { kind: "daily", hour: 9, minute: 0, tzName: "Asia/Shanghai" },
    task: {
      kind: "agent",
      project: "/tmp/scheduler-demo",
      message: "只读检查项目，输出报告。",
      personas: ["reviewer"],
      budgetFamily: "summary",
      resourceKey: "/tmp/scheduler-demo",
    },
    timezone: "Asia/Shanghai",
    misfirePolicy: "latestOnce",
    graceSeconds: 3600,
    overlapPolicy: "skip",
    timeoutSeconds: 600,
    createdAt: now,
    updatedAt: now,
    nextFireAt: now + 3600,
    archived: false,
  };
}
function run(taskId: string, id = `run-${taskId}`): TaskRunDto {
  return {
    id,
    taskId,
    occurrenceKey: id,
    triggerSource: "scheduled",
    scheduleGeneration: 1,
    scheduledFor: now,
    queuedAt: now,
    startedAt: now,
    finishedAt: null,
    configRevision: 3,
    configSnapshot: task().task,
    lifecycle: "running",
    outcome: null,
    delivery: null,
    verification: null,
    sessionId: null,
    reportRef: null,
    reasonCode: null,
    errorSummary: null,
    skippedCount: 0,
  };
}

type TestState = {
  tasks: ScheduledTaskDto[];
  runs: Record<string, TaskRunDto[]>;
  calls: Array<{ command: string; args: Record<string, unknown> }>;
  conflict: boolean;
  failCreate: boolean;
  delayHistory: string | null;
  pending: Array<() => void>;
  emit: (event: string) => void;
};
declare global {
  interface Window {
    __schedulerTest: TestState;
  }
}

/** Reuse the shell fixture, replacing only scheduler IPC after the demo mounts.
 *  Data lives in the test browser; no scheduler, provider or user store is run. */
async function openScheduler(
  page: Page,
  tasks: ScheduledTaskDto[] = [],
  runs: Record<string, TaskRunDto[]> = {},
) {
  await stubShell(page, {
    workspace: {
      projects: [project("/tmp/scheduler-demo", "调度示例")],
      sessions: [sessionRef("result-session", "/tmp/scheduler-demo", "定时任务结果")],
    },
    session: {
      id: "result-session",
      project: "/tmp/scheduler-demo",
      title: "定时任务结果",
      at: now,
      archived: false,
      turns: [],
    },
    settings: { "onboarding-done": "true" },
  });
  await page.goto("/ui-demo/schedule");
  await page.evaluate(
    ({ tasks, runs }) => {
      type Internals = {
        invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
        transformCallback: (callback: (value: unknown) => void) => number;
      };
      const bridge = (window as unknown as { __TAURI_INTERNALS__: Internals }).__TAURI_INTERNALS__;
      const invoke = bridge.invoke.bind(bridge);
      const transform = bridge.transformCallback.bind(bridge);
      const callbacks = new Map<number, (value: unknown) => void>();
      const listeners = new Map<number, { name: string; handler: number }>();
      let nextListener = 100000;
      const state: TestState = {
        tasks,
        runs,
        calls: [],
        conflict: false,
        failCreate: false,
        delayHistory: null,
        pending: [],
        emit: (name) => {
          for (const [id, listener] of listeners)
            if (listener.name === name) {
              callbacks.get(listener.handler)?.({ event: name, id, payload: {} });
            }
        },
      };
      window.__schedulerTest = state;
      bridge.transformCallback = (callback) => {
        const id = transform(callback);
        callbacks.set(id, callback);
        return id;
      };
      bridge.invoke = async (command, args = {}) => {
        if (command === "plugin:event|listen" && String(args.event).startsWith("scheduler:")) {
          const id = nextListener++;
          listeners.set(id, { name: String(args.event), handler: Number(args.handler) });
          return id;
        }
        if (command === "plugin:event|unlisten" && listeners.has(Number(args.eventId))) {
          listeners.delete(Number(args.eventId));
          return null;
        }
        state.calls.push({ command, args: structuredClone(args) });
        switch (command) {
          case "list_scheduled_tasks":
            return structuredClone(state.tasks);
          case "list_job_types":
            return ["workspace_refresh", "memory_outbox_replay"];
          case "preview_schedule": {
            const schedule = args.schedule as ScheduledTaskDto["schedule"];
            const first = schedule.kind === "once" ? schedule.fireAt : Number(args.after) + 3600;
            return Array.from(
              { length: schedule.kind === "once" ? 1 : 5 },
              (_, i) => first + i * 86400,
            );
          }
          case "create_scheduled_task": {
            if (state.failCreate) throw "storage_unavailable";
            const saved = {
              ...(args.task as ScheduledTaskDto),
              revision: 1,
              scheduleGeneration: 1,
              nextFireAt: Math.floor(Date.now() / 1000) + 3600,
            };
            state.tasks.push(saved);
            return structuredClone(saved);
          }
          case "update_scheduled_task": {
            if (state.conflict) throw "revision_conflict";
            const saved = {
              ...(args.task as ScheduledTaskDto),
              revision: Number(args.expectedRevision) + 1,
            };
            state.tasks = state.tasks.map((row) => (row.id === saved.id ? saved : row));
            return structuredClone(saved);
          }
          case "set_scheduled_task_enabled": {
            const row = state.tasks.find((row) => row.id === args.id)!;
            row.state = args.enabled ? "enabled" : "paused";
            row.revision += 1;
            return structuredClone(row);
          }
          case "archive_scheduled_task": {
            const row = state.tasks.find((row) => row.id === args.id)!;
            row.archived = true;
            row.state = "archived";
            return null;
          }
          case "list_task_runs": {
            const rows = structuredClone(state.runs[String(args.taskId)] ?? []);
            if (state.delayHistory === args.taskId)
              return new Promise((resolve) => state.pending.push(() => resolve(rows)));
            return rows;
          }
          case "cancel_task_run": {
            const row = Object.values(state.runs)
              .flat()
              .find((row) => row.id === args.id)!;
            row.lifecycle = "cancelling";
            return true;
          }
          case "run_scheduled_task_now":
            return (state.runs[String(args.id)] ?? [])[0];
          default:
            return invoke(command, args);
        }
      };
      window.history.pushState(null, "", "/schedule");
      window.dispatchEvent(new PopStateEvent("popstate"));
    },
    { tasks, runs },
  );
  await expect(
    page.getByRole("main").getByRole("button", { name: "新建任务", exact: true }),
  ).toBeEnabled();
  await expect(page.getByRole("status").filter({ hasText: "正在加载定时任务" })).toHaveCount(0);
}

test("template creates a daily agent task with backend preview and saved configuration", async ({
  page,
}) => {
  await openScheduler(page);
  await page.getByRole("button", { name: "每日项目简报", exact: true }).click();
  const editor = page.getByRole("dialog", { name: "新建定时任务" });
  await expect(editor.getByLabel("任务名称")).toHaveValue("每日项目简报");
  await expect(editor.getByLabel("执行内容")).not.toBeEmpty();
  await editor.getByLabel("执行项目").selectOption("/tmp/scheduler-demo");
  await expect(editor.locator(".schedule-preview li")).toHaveCount(5);
  await editor.getByRole("button", { name: "创建任务", exact: true }).click();
  await expect(editor).toHaveCount(0);
  await expect(page.locator(".schedule-row-title")).toHaveText("每日项目简报");
  const state = await page.evaluate(() => ({
    task: window.__schedulerTest.tasks[0],
    preview: window.__schedulerTest.calls.find((c) => c.command === "preview_schedule"),
  }));
  expect(state.task.schedule).toEqual({
    kind: "daily",
    hour: 9,
    minute: 0,
    tzName: "Asia/Shanghai",
  });
  expect(state.task.task.kind).toBe("agent");
  expect(state.preview?.args.after).toBeGreaterThan(now - 5);
});

test("editing keeps interval anchor and agent settings and submits the original revision", async ({
  page,
}) => {
  const original = task();
  original.schedule = { kind: "interval", anchor: now + 1800, intervalSeconds: 3600 };
  await openScheduler(page, [original]);
  await page.getByRole("button", { name: "编辑 · 每日检查", exact: true }).click();
  const editor = page.getByRole("dialog");
  await editor.getByLabel("任务名称").fill("重命名后的任务");
  await expect(editor.getByRole("button", { name: "保存修改" })).toBeEnabled();
  await editor.getByRole("button", { name: "保存修改" }).click();
  const call = await page.evaluate(() =>
    window.__schedulerTest.calls.find((c) => c.command === "update_scheduled_task"),
  );
  expect(call?.args.expectedRevision).toBe(3);
  expect((call?.args.task as ScheduledTaskDto).schedule).toEqual(original.schedule);
  expect((call?.args.task as ScheduledTaskDto).task).toEqual(original.task);
});

test("invalid weekdays and timezone cannot save; job JSON errors preserve the draft", async ({
  page,
}) => {
  await openScheduler(page);
  await page.getByRole("main").getByRole("button", { name: "新建任务", exact: true }).click();
  const editor = page.getByRole("dialog");
  await editor.getByLabel("任务名称").fill("索引维护");
  await editor.getByLabel("执行类型").selectOption("job");
  await editor.getByLabel("后台作业", { exact: true }).selectOption("memory_outbox_replay");
  await editor.getByLabel("重复规则").selectOption("weekly");
  for (const day of ["一", "二", "三", "四", "五"])
    await editor.getByLabel(`周${day}`, { exact: true }).uncheck();
  await expect(editor.getByRole("alert")).toContainText("至少选择");
  await expect(editor.getByRole("button", { name: "创建任务", exact: true })).toBeDisabled();
  await editor.getByLabel("周一", { exact: true }).check();
  await editor.getByLabel("时区", { exact: true }).fill("Invalid/Zone");
  await expect(editor.getByRole("alert")).toContainText("有效的 IANA");
  await editor.getByLabel("时区", { exact: true }).fill("Asia/Shanghai");
  await editor.getByLabel("作业参数（JSON）").fill("{broken");
  await expect(editor.getByRole("button", { name: "创建任务", exact: true })).toBeEnabled();
  await editor.getByRole("button", { name: "创建任务", exact: true }).click();
  await expect(editor.getByRole("alert")).toContainText("合法 JSON");
  await editor.getByLabel("作业参数（JSON）").fill('{"batch": 1}');
  await editor.getByRole("button", { name: "创建任务", exact: true }).click();
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(() => window.__schedulerTest.tasks[0].task)).toMatchObject({
    kind: "job",
    jobType: "memory_outbox_replay",
    params: { batch: 1 },
  });
});

test("revision conflict and storage failure stay visible and retain unsaved fields", async ({
  page,
}) => {
  await openScheduler(page, [task()]);
  await page.evaluate(() => {
    window.__schedulerTest.conflict = true;
  });
  await page.getByRole("button", { name: "编辑 · 每日检查", exact: true }).click();
  const editor = page.getByRole("dialog");
  await editor.getByLabel("任务名称").fill("保留这份草稿");
  await expect(editor.getByRole("button", { name: "保存修改" })).toBeEnabled();
  await editor.getByRole("button", { name: "保存修改" }).click();
  await expect(editor.getByRole("alert")).toContainText("配置已被修改");
  await expect(editor.getByLabel("任务名称")).toHaveValue("保留这份草稿");
  await page.keyboard.press("Escape");
  await page.evaluate(() => {
    window.__schedulerTest.failCreate = true;
  });
  await page.getByRole("button", { name: "每日项目简报", exact: true }).click();
  await expect(editor.getByRole("button", { name: "创建任务", exact: true })).toBeEnabled();
  await editor.getByRole("button", { name: "创建任务", exact: true }).click();
  await expect(editor.getByRole("alert")).toContainText("存储不可用");
  await expect(editor.getByLabel("任务名称")).toHaveValue("每日项目简报");
  await page.evaluate(() => {
    window.__schedulerTest.failCreate = false;
  });
  await editor.getByRole("button", { name: "创建任务", exact: true }).click();
  await expect(editor).toHaveCount(0);
  const createIds = await page.evaluate(() =>
    window.__schedulerTest.calls
      .filter((call) => call.command === "create_scheduled_task")
      .map((call) => (call.args.task as ScheduledTaskDto).id),
  );
  expect(createIds).toHaveLength(2);
  expect(createIds[0]).toBe(createIds[1]);
});

test("late history from another task cannot overwrite the expanded task; events refresh results", async ({
  page,
}) => {
  const firstRun = run("a", "slow-run");
  firstRun.errorSummary = "A 的晚到记录";
  const secondRun = run("b", "current-run");
  secondRun.errorSummary = "B 正在执行";
  await openScheduler(page, [task("a", "任务 A"), task("b", "任务 B")], {
    a: [firstRun],
    b: [secondRun],
  });
  await page.evaluate(() => {
    window.__schedulerTest.delayHistory = "a";
  });
  await page.getByRole("button", { name: "运行历史 · 任务 A", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => window.__schedulerTest.pending.length))
    .toBeGreaterThan(0);
  await page.getByRole("button", { name: "运行历史 · 任务 B", exact: true }).click();
  await expect(page.getByTestId("schedule-run-current-run")).toContainText("B 正在执行");
  await page.evaluate(() => {
    window.__schedulerTest.pending.splice(0).forEach((resolve) => resolve());
  });
  await expect(page.getByText("A 的晚到记录", { exact: true })).toHaveCount(0);
  await page.evaluate(() => {
    const row = window.__schedulerTest.runs.b[0];
    row.lifecycle = "finished";
    row.outcome = "succeeded";
    row.errorSummary = "工作区刷新完成";
    row.reportRef = "/demo/report.json";
    window.__schedulerTest.emit("scheduler:run_changed");
  });
  await expect(page.getByTestId("schedule-run-current-run")).toContainText("成功");
  await expect(page.getByTestId("schedule-run-current-run")).toContainText("工作区刷新完成");
  await expect(page.getByTestId("schedule-run-current-run")).toContainText("报告引用");
});

test("pause, manual execution, cancellation, archive, and search use the live APIs", async ({
  page,
}) => {
  await openScheduler(page, [task()], { a: [run("a")] });
  const row = page.getByTestId("schedule-row-a");
  await row.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(row.getByRole("button", { name: "恢复", exact: true })).toBeEnabled();
  await row.getByRole("button", { name: "立即执行", exact: true }).click();
  await expect(row).toContainText("执行中");
  await row.getByRole("button", { name: "取消本次", exact: true }).click();
  await expect(row.getByRole("button", { name: "正在取消", exact: true })).toBeDisabled();
  await page.evaluate(() => {
    window.__schedulerTest.runs.a[0].lifecycle = "finished";
    window.__schedulerTest.emit("scheduler:run_changed");
  });
  await row.getByRole("button", { name: "归档", exact: true }).click();
  await expect(row).toHaveCount(0);
  await page.getByTestId("schedule-filter-archived").click();
  await expect(row).toContainText("已归档");
  await expect(row.getByRole("button", { name: "立即执行", exact: true })).toBeDisabled();
  await page.getByLabel("搜索定时任务").fill("不存在的任务");
  await expect(row).toHaveCount(0);
});

test("run results open the associated conversation", async ({ page }) => {
  const result = run("a");
  result.lifecycle = "finished";
  result.outcome = "succeeded";
  result.sessionId = "result-session";
  await openScheduler(page, [task()], { a: [result] });
  await page.getByRole("button", { name: "运行历史 · 每日检查", exact: true }).click();
  await page.getByRole("button", { name: "查看会话", exact: true }).click();
  await expect(page).toHaveURL(/\/conversation$/);
  await expect(page.locator(".topbar")).toContainText("定时任务结果");
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__schedulerTest.calls.some(
          (call) => call.command === "load_session" && call.args.id === "result-session",
        ),
      ),
    )
    .toBe(true);
});

test("timezone conversion rejects DST gaps and chooses the earlier overlap", async ({ page }) => {
  await openScheduler(page);
  const result = await page.evaluate(async () => {
    const path = "/src/pages/schedulerModel.ts";
    const { wallTimeToUnix } = await import(path);
    let gap = "";
    try {
      wallTimeToUnix("2030-03-10T02:30", "America/Los_Angeles");
    } catch (error) {
      gap = String(error);
    }
    return {
      shanghai: wallTimeToUnix("2030-01-02T09:00", "Asia/Shanghai"),
      overlap: wallTimeToUnix("2030-11-03T01:30", "America/Los_Angeles"),
      gap,
    };
  });
  expect(result.shanghai).toBe(Date.parse("2030-01-02T01:00:00Z") / 1000);
  expect(result.overlap).toBe(Date.parse("2030-11-03T08:30:00Z") / 1000);
  expect(result.gap).toContain("不存在");
  await page.getByRole("main").getByRole("button", { name: "新建任务", exact: true }).click();
  const editor = page.getByRole("dialog");
  await editor.getByLabel("重复规则").selectOption("once");
  await editor.getByLabel("时区", { exact: true }).fill("Asia/Shanghai");
  await editor.getByLabel("执行日期和时间").fill("2030-01-02T09:00");
  await expect(editor.locator(".schedule-preview li")).toHaveCount(1);
  await expect(editor.locator(".schedule-preview li")).toContainText("09:00:00");
});

test("narrow editor fits the viewport and Escape restores focus without creating a task", async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 680, height: 820 });
  await openScheduler(page);
  const trigger = page.getByRole("main").getByRole("button", { name: "新建任务", exact: true });
  await trigger.click();
  const editor = page.getByRole("dialog");
  await expect(editor.getByLabel("任务名称")).toBeFocused();
  expect(await editor.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath("scheduler-editor-narrow.png") });
  await page.keyboard.press("Escape");
  await expect(editor).toHaveCount(0);
  await expect(trigger).toBeFocused();
  expect(await page.evaluate(() => window.__schedulerTest.tasks.length)).toBe(0);
});
