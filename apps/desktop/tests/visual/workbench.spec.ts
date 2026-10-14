import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import type { CalendarEventDto, TodoDto } from "../../src/api";
import { DEMO_CALENDAR_EVENTS, DEMO_TODOS } from "../../src/data/workbench";
import { stubShell, type Core } from "./shell";

/**
 * 工作台 and the three apps it hosts.
 *
 * Every case drives the real UI against the stateful stub store in `shell.ts`,
 * so a status change has to survive the round trip and come back through
 * `list_todos` for the assertions to pass — nothing here reads component state.
 *
 * The stub makes `isDesktop()` true, which means `hrefTo` drops the `/ui-demo`
 * prefix and the apps read the **real** clock. Cases that depend on "today"
 * therefore seed their own rows for the current date instead of reusing the
 * frozen `DEMO_TODAY` fixtures.
 */

/** Local `YYYY-MM-DD`, the same helper the apps use (`pages/apps/time.ts`). */
function isoDate(offset = 0): string {
  const date = new Date();
  date.setDate(date.getDate() + offset);
  const pad = (n: number): string => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

const TODAY = isoDate();

/** A 待办 row shaped like the store's, seeded for a specific case. */
const todo = (over: Partial<TodoDto> & { id: string }): TodoDto => ({
  ...DEMO_TODOS[1],
  ...over,
});

/** A 日历 row shaped like the store's. */
const event = (over: Partial<CalendarEventDto> & { id: string }): CalendarEventDto => ({
  ...DEMO_CALENDAR_EVENTS[1],
  ...over,
});

/**
 * The 日历 marks an event as cross-zone when its IANA name differs from the
 * machine's, and the browser and this process share a machine — so the seed has
 * to be whichever of the two this host is *not* running on.
 */
const OTHER_ZONE =
  Intl.DateTimeFormat().resolvedOptions().timeZone === "America/Los_Angeles"
    ? "Asia/Shanghai"
    : "America/Los_Angeles";

/** Every command the page sent, in order. */
const wbLog = (page: Page) =>
  page.evaluate(
    () =>
      (
        window as unknown as {
          __wbLog?: Array<{ command: string; args: Record<string, unknown> }>;
        }
      ).__wbLog ?? [],
  );

const commands = async (page: Page): Promise<string[]> =>
  (await wbLog(page)).map((row) => row.command);

/**
 * Moves the SPA to another route **without reloading**: a real `goto` would
 * re-run the init script and reset the stub's state (an app disabled in this
 * session would come back enabled). This is exactly what `routes.ts`
 * `navigate()` does.
 */
const goTo = (page: Page, url: string) =>
  page.evaluate((to) => {
    window.history.pushState(null, "", to);
    window.dispatchEvent(new PopStateEvent("popstate"));
  }, url);

/** The workbench collection. */
async function openWorkbench(page: Page, core: Core = {}) {
  await stubShell(page, core);
  await page.goto("/");
  await page.getByRole("link", { name: "工作台" }).click();
  await expect(page.locator(".page-head h1")).toHaveText("工作台");
}

/** 待办, opened from the collection the way a user reaches it. */
async function openTodo(page: Page, core: Core = {}) {
  await openWorkbench(page, core);
  await page.getByTestId("wb-open-todo").click();
  await expect(page.locator(".todo-board")).toBeVisible();
}

async function openCalendar(page: Page, core: Core = {}) {
  await openWorkbench(page, core);
  await page.getByTestId("wb-open-calendar").click();
  await expect(page.getByTestId("cal-week")).toBeVisible();
}

async function openResults(page: Page, core: Core = {}) {
  await openWorkbench(page, core);
  await page.getByTestId("wb-open-artifacts").click();
  await expect(page.locator(".result-list")).toBeVisible();
}

test("新建任务 stays the landing page and 工作台 is the only new nav row", async ({ page }) => {
  await stubShell(page);
  await page.goto("/");

  // The default landing is unchanged: the composer, not the workbench.
  await expect(page.locator(".composer-box")).toBeVisible();
  await expect(page.getByTestId("workbench-search")).toHaveCount(0);
  await expect(page.locator(".nav .nav-item")).toHaveText([
    "工作台",
    "新建任务",
    "插件",
    "定时任务",
  ]);
});

test("the workbench opens from the sidebar and returns from an app", async ({ page }) => {
  await openTodo(page);

  // The app is the same route, so the sidebar keeps one 工作台 entry.
  await expect(page.locator(".page-head h1")).toHaveText("待办");
  await expect(page.locator(".nav .nav-item")).toHaveText([
    "工作台",
    "新建任务",
    "插件",
    "定时任务",
  ]);

  await page.getByTestId("app-back").click();
  await expect(page.locator(".page-head h1")).toHaveText("工作台");
  await expect(page).toHaveURL(/\/workbench/);
});

test("search, favorite and ordering change the collection", async ({ page }) => {
  await openWorkbench(page);

  // The four apps appear first, then the search narrows the rows.
  await expect(page.locator(".wb-app-row")).toHaveCount(4);
  await page.getByTestId("workbench-search").fill("日历");
  await expect(page.locator(".wb-app-row")).toHaveCount(1);
  await expect(page.locator(".wb-app-row")).toContainText("日历");
  await page.getByTestId("workbench-search").fill("");
  await expect(page.locator(".wb-app-row")).toHaveCount(4);

  // 待办 is the pre-starred fixture, so it heads the 收藏 section.
  const sections = page.locator(".wb-section");
  await expect(sections.nth(0).locator(".app-section-title")).toHaveText("收藏");
  await expect(sections.nth(0).locator(".wb-app-row")).toHaveCount(1);

  await page.getByTestId("workbench-manage").click();
  await page.getByTestId("wb-favorite-calendar").click();
  await page.getByTestId("wb-up-calendar").click();
  await page.getByTestId("workbench-manage-close").click();

  // Reordering posts the whole id list, and the collection re-reads it.
  expect(await commands(page)).toContain("set_app_order");
  const order = await page.evaluate(
    () =>
      (
        window as unknown as {
          __wbLog?: Array<{ command: string; args: Record<string, unknown> }>;
        }
      ).__wbLog
        ?.filter((row) => row.command === "set_app_order")
        .at(-1)?.args.appIds,
  );
  expect(order).toEqual(["calendar", "todo", "artifacts", "knowledge"]);
  // …and both starred apps now lead the list, favorites first.
  await expect(page.locator(".wb-app-row").first()).toContainText("日历");
});

test("disabling an app removes it from the collection and refuses to open it", async ({ page }) => {
  await openWorkbench(page);
  await page.getByTestId("workbench-manage").click();
  await page.getByTestId("wb-enabled-artifacts").click();
  await page.getByTestId("workbench-manage-close").click();

  await expect(page.getByTestId("wb-app-artifacts")).toHaveCount(0);
  await expect(page.locator(".wb-app-row")).toHaveCount(3);

  // A URL that still names the disabled app lands on the collection with a
  // hint instead of a dead screen — the data is kept, only the entry goes.
  await goTo(page, "/workbench?app=artifacts");
  await expect(page.getByTestId("workbench-app-notice")).toBeVisible();
  await expect(page.locator(".page-head h1")).toHaveText("工作台");

  // …and re-enabling brings it back. (Back to the collection first: on the app
  // URL the workbench itself is what the app replaces.)
  await goTo(page, "/workbench");
  await page.getByTestId("workbench-manage").click();
  await page.getByTestId("wb-enabled-artifacts").click();
  await page.getByTestId("workbench-manage-close").click();
  await expect(page.getByTestId("wb-app-artifacts")).toBeVisible();
});

test("今日概览 lists today's open items and today's events, and can be turned off", async ({
  page,
}) => {
  await openWorkbench(page, {
    todos: [
      todo({ id: "t-today-open", title: "今天要做的事", status: "todo", dueDate: TODAY }),
      todo({ id: "t-today-done", title: "今天已完成的事", status: "done", dueDate: TODAY }),
      todo({ id: "t-tomorrow", title: "明天的事", status: "todo", dueDate: isoDate(1) }),
    ],
    calendarEvents: [
      event({ id: "e-today", title: "今天的评审", date: TODAY, startTime: "14:00" }),
      event({ id: "e-other", title: "别处的事件", date: isoDate(3) }),
    ],
  });

  const today = page.getByTestId("workbench-today");
  await expect(today).toBeVisible();
  // Only today's *open* todos count, and only today's events.
  await expect(page.getByTestId("workbench-today-todos")).toHaveText("今日待办 · 1");
  await expect(page.getByTestId("workbench-today-events")).toHaveText("今日事件 · 1");
  await expect(today).toContainText("今天要做的事");
  await expect(today).not.toContainText("今天已完成的事");

  await page.getByTestId("workbench-manage").click();
  await page.getByTestId("workbench-summary-toggle").click();
  await page.getByTestId("workbench-manage-close").click();
  await expect(page.getByTestId("workbench-today")).toHaveCount(0);

  // The switch is a stored preference, not local state.
  const prefs = await page.evaluate(() =>
    (
      window as unknown as {
        __wbLog?: Array<{ command: string; args: Record<string, unknown> }>;
      }
    ).__wbLog
      ?.filter((row) => row.command === "set_workbench_pref")
      .map((row) => `${String(row.args.key)}=${String(row.args.value)}`),
  );
  expect(prefs).toContain("workbench.showToday=false");
});

test("今日概览 opens the apps it summarizes", async ({ page }) => {
  await openWorkbench(page, {
    todos: [todo({ id: "t-today", title: "今天要做的事", status: "todo", dueDate: TODAY })],
    calendarEvents: [event({ id: "e-today", title: "今天的评审", date: TODAY })],
  });

  await page.getByTestId("workbench-today-todos").click();
  await expect(page.locator(".page-head h1")).toHaveText("待办");

  await page.getByTestId("app-back").click();
  await page.getByTestId("workbench-today-events").click();
  await expect(page.locator(".page-head h1")).toHaveText("日历");
});

test("the board splits the fixture across the three fixed columns", async ({ page }) => {
  await openTodo(page);

  await expect(page.locator(".todo-column-name")).toHaveText(["待处理", "进行中", "已完成"]);
  await expect(page.locator('[data-testid="todo-column-todo"] .todo-card')).toHaveCount(3);
  await expect(page.locator('[data-testid="todo-column-doing"] .todo-card')).toHaveCount(1);
  await expect(page.locator('[data-testid="todo-column-done"] .todo-card')).toHaveCount(1);

  // A card carries its title, due date, priority and status.
  const card = page.getByTestId("todo-card-todo-1");
  await expect(card.locator(".todo-priority").first()).toHaveText("高");
  await expect(card.locator(".todo-due").first()).toContainText("09-30");
  await expect(page.getByTestId("todo-status-todo-1")).toHaveValue("doing");
  // Sub-items are counted on the parent and never float into a column.
  await expect(card.locator(".todo-subcount")).toHaveText("1 个子项");
  // The sub-item is listed under its parent and is not a column card of its own.
  await expect(page.getByTestId("todo-card-todo-4")).toHaveCount(0);
  await expect(card.getByTestId("todo-child-todo-4")).toContainText("补齐 k2k-rust 的回归用例");
});

test("board and list are two views of one dataset", async ({ page }) => {
  await openTodo(page);

  await page.getByTestId("todo-view-list").click();
  await expect(page.getByTestId("todo-list").locator(".todo-row")).toHaveCount(5);

  // The filter applies to both views from the same rows.
  await page.getByTestId("todo-filter-open").click();
  await expect(page.getByTestId("todo-list").locator(".todo-row")).toHaveCount(4);

  await page.getByTestId("todo-filter-done").click();
  await expect(page.getByTestId("todo-list").locator(".todo-row")).toHaveCount(1);
  await expect(page.getByTestId("todo-row-todo-6")).toContainText("归档 09-25 的路由排查记录");

  await page.getByTestId("todo-view-board").click();
  const done = page.locator('[data-testid="todo-column-done"] .todo-card');
  await expect(done).toHaveCount(1);
  await expect(done).toContainText("归档 09-25 的路由排查记录");
});

test("今天 filters on the record's own due date, whatever its status", async ({ page }) => {
  await openTodo(page, {
    todos: [
      todo({ id: "t-open", title: "今天要做的事", status: "todo", dueDate: TODAY }),
      todo({ id: "t-done", title: "今天已完成的事", status: "done", dueDate: TODAY }),
      todo({ id: "t-later", title: "之后的事", status: "todo", dueDate: isoDate(2) }),
    ],
  });

  await page.getByTestId("todo-filter-today").click();
  // Both of today's rows show — 今天 is a due-date filter, not a status one.
  await expect(page.locator(".todo-column-body .todo-card")).toHaveCount(2);
  await expect(page.getByTestId("todo-card-t-later")).toHaveCount(0);

  await page.getByTestId("todo-filter-inbox").click();
  // 收集箱 is the other half of the same rule: no due date at all.
  await expect(page.locator(".todo-column-body .todo-card")).toHaveCount(0);
});

test("creating needs only a title, and the column decides the status", async ({ page }) => {
  await openTodo(page);

  // Top-level create lands in 待处理.
  await page.getByTestId("todo-new-input").fill("写一条巡检结论");
  await page.getByTestId("todo-new-submit").click();
  await expect(page.locator('[data-testid="todo-column-todo"] .todo-card')).toHaveCount(4);

  // Per-column create lands in that column.
  await page.getByTestId("todo-add-doing").click();
  await page.getByRole("textbox", { name: "在「进行中」中新建" }).fill("补一条回归用例");
  await page.keyboard.press("Enter");
  await expect(page.locator('[data-testid="todo-column-doing"] .todo-card')).toHaveCount(2);

  const created = (await wbLog(page)).filter((row) => row.command === "create_todo");
  expect(created.map((row) => (row.args.draft as { status: string }).status)).toEqual([
    "todo",
    "doing",
  ]);
  // A create is idempotent: the client sends a request id with every draft.
  expect((created[0].args.draft as { requestId?: string }).requestId).toBeTruthy();
});

test("完成 moves a card to 已完成 and 恢复 brings it back to 待处理", async ({ page }) => {
  await openTodo(page);

  await page.getByTestId("todo-toggle-todo-2").click();
  await expect(page.locator('[data-testid="todo-column-done"] .todo-card')).toHaveCount(2);
  // The card itself moves: it is in 已完成 now, not in 待处理.
  await expect(
    page.locator('[data-testid="todo-column-done"] [data-testid="todo-card-todo-2"]'),
  ).toBeVisible();
  await expect(
    page.locator('[data-testid="todo-column-todo"] [data-testid="todo-card-todo-2"]'),
  ).toHaveCount(0);

  // The list view agrees, because both read the one status field.
  await page.getByTestId("todo-view-list").click();
  await page.getByTestId("todo-filter-done").click();
  await expect(page.getByTestId("todo-row-todo-2").locator(".todo-row-title--done")).toBeVisible();

  await page.getByTestId("todo-toggle-todo-2").click();
  await expect(page.getByTestId("todo-row-todo-2")).toHaveCount(0);
  await page.getByTestId("todo-filter-open").click();
  await expect(page.getByTestId("todo-row-todo-2")).toBeVisible();

  // 恢复 goes back to 待处理, not to 进行中.
  await page.getByTestId("todo-view-board").click();
  await page.getByTestId("todo-filter-all").click();
  await expect(
    page.locator('[data-testid="todo-column-todo"] [data-testid="todo-card-todo-2"]'),
  ).toBeVisible();
});

test("the status select is the same write path as drag and 完成", async ({ page }) => {
  await openTodo(page);

  await page.getByTestId("todo-status-todo-3").selectOption("done");
  await expect(page.locator('[data-testid="todo-column-done"] .todo-card')).toHaveCount(2);

  const writes = (await wbLog(page)).filter((row) => row.command === "set_todo_status");
  expect(writes).toHaveLength(1);
  expect(writes[0].args).toMatchObject({ id: "todo-3", status: "done" });
  // The version travels with the write, so a stale edit is refused.
  expect(typeof writes[0].args.expectedVersion).toBe("number");
});

test("dragging a card across columns writes the status once", async ({ page }) => {
  await openTodo(page);

  const handle = page.getByTestId("todo-handle-todo-2");
  const target = page.getByTestId("todo-column-doing");
  await handle.scrollIntoViewIfNeeded();
  const from = (await handle.boundingBox())!;
  const to = (await target.boundingBox())!;

  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  // The ghost names the card; the target column reports the pending drop.
  await expect(page.locator(".todo-drag-ghost")).toHaveText("整理实时数仓搭建流程的说明文档");
  await page.mouse.move(to.x + to.width / 2, to.y + 64, { steps: 8 });
  await expect(page.getByTestId("todo-column-doing")).toHaveClass(/todo-column-body--over/);
  await page.mouse.up();

  await expect(page.locator('[data-testid="todo-column-doing"] .todo-card')).toHaveCount(2);
  await expect(
    page.locator('[data-testid="todo-column-doing"] [data-testid="todo-card-todo-2"]'),
  ).toBeVisible();
  const writes = (await wbLog(page)).filter((row) => row.command === "set_todo_status");
  expect(writes).toHaveLength(1);
  expect(writes[0].args).toMatchObject({ id: "todo-2", status: "doing" });
});

test("a cancelled drag leaves the status alone", async ({ page }) => {
  await openTodo(page);

  const handle = page.getByTestId("todo-handle-todo-2");
  const box = (await handle.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await expect(page.locator(".todo-drag-ghost")).toBeVisible();
  await page.mouse.move(box.x + 40, box.y + 40, { steps: 4 });
  await page.keyboard.press("Escape");
  await expect(page.locator(".todo-drag-ghost")).toHaveCount(0);
  await page.mouse.up();

  await expect(page.locator('[data-testid="todo-column-todo"] .todo-card')).toHaveCount(3);
  expect((await commands(page)).filter((name) => name === "set_todo_status")).toHaveLength(0);
});

test("editing a card updates the record and its sub-items", async ({ page }) => {
  await openTodo(page);

  await page.getByTestId("todo-edit-todo-5").click();
  await page.getByTestId("todo-edit-title").fill("更新发布说明里的 v1.2 与 v1.3 段落");
  await page.getByTestId("todo-edit-priority").selectOption("high");
  await page.getByTestId("todo-edit-due").fill("2026-10-05");
  // A sub-item can be added from the editor and inherits the parent's status.
  await page.getByTestId("todo-add-subtask").fill("校对版本号");
  await page.getByTestId("todo-add-subtask").press("Enter");
  await page.getByTestId("todo-edit-save").click();

  const card = page.getByTestId("todo-card-todo-5");
  await expect(card.locator(".todo-card-title")).toHaveText("更新发布说明里的 v1.2 与 v1.3 段落");
  await expect(card.locator(".todo-priority")).toHaveText("高");
  await expect(card.locator(".todo-due")).toContainText("10-05");
  await expect(card.locator(".todo-subcount")).toHaveText("1 个子项");
  expect(await commands(page)).toContain("update_todo");
  // The sub-item was created, not saved as part of the parent's row.
  const subtask = (await wbLog(page)).filter((row) => row.command === "create_todo");
  expect((subtask[0].args.draft as { parentId?: string }).parentId).toBe("todo-5");
});

test("archiving hides a row from the board until the archived toggle is on", async ({ page }) => {
  await openTodo(page);

  await expect(page.getByTestId("todo-archived")).toHaveCount(0);
  await page.getByTestId("todo-edit-todo-6").click();
  await page.getByTestId("todo-edit-archive").click();

  await expect(page.getByTestId("todo-card-todo-6")).toHaveCount(0);
  await page.getByTestId("todo-archived-toggle").click();
  await expect(page.getByTestId("todo-archived")).toBeVisible();
  await expect(page.getByTestId("todo-row-todo-6")).toBeVisible();

  await page.getByTestId("todo-unarchive-todo-6").click();
  await expect(page.getByTestId("todo-row-todo-6")).toHaveCount(0);
  expect(await commands(page)).toContain("set_todo_archived");
});

test("the calendar creates, edits and deletes a real event", async ({ page }) => {
  await openCalendar(page);

  await page.getByTestId("cal-new").click();
  await page.getByTestId("cal-edit-title").fill("复盘路由兜底");
  await page.getByTestId("cal-edit-start").fill("16:00");
  await page.getByTestId("cal-edit-end").fill("16:45");
  await page.getByTestId("cal-edit-save").click();

  const created = page.locator('[data-testid^="cal-event-event-stub-"]');
  await expect(created).toHaveCount(1);
  await expect(created).toContainText("复盘路由兜底");

  await created.click();
  await page.getByTestId("cal-edit-title").fill("复盘路由兜底（改）");
  await page.getByTestId("cal-edit-save").click();
  await expect(page.locator('[data-testid^="cal-event-event-stub-"]')).toContainText(
    "复盘路由兜底（改）",
  );

  // Delete is a two-step confirm, and only the record goes.
  await page.locator('[data-testid^="cal-event-event-stub-"]').click();
  await page.getByTestId("cal-edit-delete").click();
  await expect(page.getByTestId("cal-edit-delete")).toContainText("确认删除？");
  await page.getByTestId("cal-edit-delete").click();
  await expect(page.locator('[data-testid^="cal-event-event-stub-"]')).toHaveCount(0);

  const sent = await commands(page);
  expect(sent).toContain("create_calendar_event");
  expect(sent).toContain("update_calendar_event");
  expect(sent).toContain("delete_calendar_event");
  // Editing an event never touches a 待办 record and never starts a run.
  expect(sent).not.toContain("update_todo");
  expect(sent).not.toContain("send_message");
});

test("a week block shows the lines it has room for and squashes none of them", async ({ page }) => {
  await openCalendar(page, {
    calendarEvents: [
      event({ id: "e-short", title: "站会", date: TODAY, startTime: "09:00", endTime: "09:30" }),
      event({
        id: "e-zone",
        title: "跨时区对齐",
        date: TODAY,
        startTime: "16:00",
        endTime: "17:00",
        timezone: OTHER_ZONE,
      }),
    ],
  });

  // A squeezed flex child clips its own glyphs mid-letter, which reads as a
  // rendering bug rather than as a shorter block.
  const squashed = await page.$$eval(".cal-event > span", (spans) =>
    spans.filter((node) => node.clientHeight !== node.scrollHeight).map((node) => node.textContent),
  );
  expect(squashed).toEqual([]);

  // Half an hour holds one line, and that line is the title: the grid's rows
  // already say when, so the time is what a short block drops.
  await expect(page.getByTestId("cal-event-e-short")).toHaveText("站会");
  await expect(page.getByTestId("cal-event-e-short")).toHaveAttribute("title", "09:00 – 09:30");

  // One hour fits the time as well, but the zone needs a third line, so it
  // stays in the tooltip.
  const block = page.getByTestId("cal-event-e-zone");
  await expect(block).toContainText("16:00");
  await expect(block).toContainText("跨时区对齐");
  await expect(block.locator(".cal-event-zone")).toHaveCount(0);
  await expect(block).toHaveAttribute("title", `16:00 ${OTHER_ZONE}`);

  await block.click();
  await page.getByTestId("cal-edit-end").fill("18:00");
  await page.getByTestId("cal-edit-save").click();
  await expect(page.getByTestId("cal-event-e-zone").locator(".cal-event-zone")).toHaveText(
    OTHER_ZONE,
  );
});

test("editing an event keeps the event's own timezone", async ({ page }) => {
  await openCalendar(page, {
    calendarEvents: [
      event({
        id: "e-zone",
        title: "跨时区对齐",
        date: TODAY,
        startTime: "16:00",
        endTime: "18:00",
        timezone: OTHER_ZONE,
      }),
    ],
  });

  await page.getByTestId("cal-event-e-zone").click();
  await page.getByTestId("cal-edit-title").fill("跨时区对齐（改）");
  await page.getByTestId("cal-edit-save").click();

  // The write carries the event's zone, not the calendar's — an edit must not
  // silently re-zone an event the user filed against another one.
  const update = (await wbLog(page)).find((row) => row.command === "update_calendar_event");
  expect((update?.args.event as { timezone?: string } | undefined)?.timezone).toBe(OTHER_ZONE);
  await expect(page.getByTestId("cal-event-e-zone")).toHaveAttribute(
    "title",
    `16:00 ${OTHER_ZONE}`,
  );
});

test("the month view shows the same events as the week", async ({ page }) => {
  await openCalendar(page, {
    todos: [todo({ id: "t-today", title: "今天到期的事", status: "todo", dueDate: TODAY })],
    calendarEvents: [event({ id: "e-today", title: "今天的评审", date: TODAY })],
  });

  await page.getByTestId("cal-view-month").click();
  await expect(page.getByTestId("cal-month")).toBeVisible();
  await expect(page.getByTestId(`cal-cell-${TODAY}`).locator(".cal-month-event")).toHaveCount(1);

  // 待办截止 rides along as a count, from the same rows the board shows.
  await expect(page.getByTestId(`cal-month-due-${TODAY}`)).toHaveCount(0);
  await page.getByTestId("cal-due-toggle").click();
  await expect(page.getByTestId(`cal-month-due-${TODAY}`)).toHaveText("待办截止 1");
});

test("the calendar's 待办截止 overlay follows the 待办 app's own records", async ({ page }) => {
  await openTodo(page, {
    todos: [todo({ id: "t-today", title: "今天到期的事", status: "todo", dueDate: TODAY })],
  });
  await page.getByTestId("todo-toggle-t-today").click();
  await page.getByTestId("app-back").click();

  await page.getByTestId("wb-open-calendar").click();
  await page.getByTestId("cal-due-toggle").click();

  const due = page.getByTestId(`cal-due-${TODAY}`);
  await expect(due).toBeVisible();
  await expect(due).toContainText("今天到期的事");
  // The completed item stays listed but reads as done — the record is shared,
  // not copied, so a status change shows up here without a second write.
  await expect(due.locator(".cal-due-item--done")).toHaveCount(1);

  const sent = await commands(page);
  expect(sent.filter((name) => name === "set_todo_status")).toHaveLength(1);
  expect(sent).not.toContain("update_todo");
  expect(sent).not.toContain("update_calendar_event");
});

test("成果中心 saves text, registers a file reference and reopens it by record", async ({
  page,
}) => {
  await openResults(page);
  await expect(page.locator(".result-row")).toHaveCount(4);

  // A text artifact needs only a title and a body.
  await page.getByTestId("result-new").click();
  await page.getByTestId("result-edit-title").fill("路由兜底结论");
  await page.getByTestId("result-edit-body").fill("结论：兜底分支吞掉了显式路由。");
  await page.getByTestId("result-edit-save").click();
  await expect(page.locator(".result-row")).toHaveCount(5);

  // A file reference goes through the OS picker and reports as present.
  await page.getByTestId("result-new").click();
  await page.getByTestId("result-edit-kind").selectOption("file");
  await page.getByTestId("result-edit-pick").click();
  await page.getByTestId("result-edit-title").fill("采样导出");
  await page.getByTestId("result-edit-save").click();
  await expect(page.getByTestId("result-toggle-result-stub-2")).toHaveText("采样导出");

  // Opening and revealing name the record, never a path.
  await page.getByTestId("result-toggle-result-stub-2").click();
  const row = page.getByTestId("result-result-stub-2");
  await expect(row.locator(".result-status").first()).toHaveText("文件正常");
  await row.getByRole("button", { name: "打开文件" }).click();
  await row.getByRole("button", { name: "在 Finder 中显示" }).click();

  const opened = (await wbLog(page)).filter(
    (entry) => entry.command === "open_result_file" || entry.command === "reveal_result_file",
  );
  expect(opened.map((entry) => entry.command)).toEqual(["open_result_file", "reveal_result_file"]);
  for (const entry of opened) expect(entry.args.id).toBe("result-stub-2");
});

test("成果中心 reports a moved or replaced file instead of hiding it", async ({ page }) => {
  await openResults(page);

  await expect(page.getByTestId("result-status-result-3")).toHaveText("文件已更改");
  await expect(page.getByTestId("result-status-result-4")).toHaveText("文件缺失");

  await page.getByTestId("result-toggle-result-4").click();
  const detail = page.getByTestId("result-detail-result-4");
  await expect(detail).toContainText("~/projects/realtime-lakehouse/docs/old-mapping.md");
  await expect(detail).toContainText("引用的文件不在原路径了，记录仍然保留。");

  // The record survives; its buttons stay the record-scoped ones.
  await expect(page.getByTestId("result-result-4")).toBeVisible();
});

test("deleting an artifact removes the record and leaves the file in place", async ({ page }) => {
  await openResults(page);

  const row = page.getByTestId("result-result-2");
  await page.getByTestId("result-delete-result-2").click();
  await expect(row).toContainText("只删除记录，磁盘上的原始文件不会被删除。");
  await page.getByTestId("result-delete-result-2").click();
  await expect(page.locator(".result-row")).toHaveCount(3);

  const sent = (await wbLog(page)).filter((entry) => entry.command === "delete_result");
  expect(sent).toHaveLength(1);
  // No command in the whole flow touches a file path.
  expect((await commands(page)).some((name) => name.includes("delete_file"))).toBe(false);
});

test("成果中心 keeps favorites and an archive, both on the record", async ({ page }) => {
  await openResults(page);

  await page.getByTestId("result-favorite-result-2").click();
  await page.getByTestId("result-filter-favorites").click();
  await expect(page.locator(".result-row")).toHaveCount(2);

  await page.getByTestId("result-filter-all").click();
  await page.getByTestId("result-archive-result-3").click();
  await expect(page.getByTestId("result-result-3")).toHaveCount(0);
  await page.getByTestId("result-archived-toggle").click();
  await expect(page.getByTestId("result-archived")).toContainText("延迟采样导出（已重新生成）");

  const sent = await commands(page);
  expect(sent).toContain("set_result_flags");
  expect(sent).not.toContain("delete_result");
});

test("the apps run standalone: no model call, no scheduler run, no project", async ({ page }) => {
  // Nothing here answers `send_message`, and the workbench was never given a
  // project — the three apps still do their whole job.
  await openTodo(page, { workspace: { projects: [], sessions: [] } });
  await page.getByTestId("todo-new-input").fill("离线也能记一条");
  await page.getByTestId("todo-new-submit").click();
  await expect(page.locator('[data-testid="todo-column-todo"] .todo-card')).toHaveCount(4);

  const sent = await commands(page);
  expect(sent).not.toContain("send_message");
  expect(sent.some((name) => name.includes("scheduled_task"))).toBe(false);
  expect(sent.some((name) => name.includes("project"))).toBe(false);
});

test("the AI link stays off and says why, rather than faking a write channel", async ({ page }) => {
  await openWorkbench(page);
  await page.getByTestId("workbench-manage").click();

  // 待办 declares the capability, so the switch is shown — and off.
  await expect(page.getByTestId("wb-ai-todo")).toHaveAttribute("aria-checked", "false");
  await expect(page.locator(".app-hint")).toContainText("暂无 AI 写入通道");
  // An app without the capability says so instead of offering a dead switch.
  await expect(page.getByTestId("wb-ai-artifacts")).toHaveCount(0);
  await expect(page.getByTestId("wb-manage-artifacts")).toContainText("此应用未声明该能力。");

  // The scheduler link is the existing one, and it writes through the store.
  await page.getByTestId("wb-schedule-calendar").click();
  const link = (await wbLog(page)).filter((row) => row.command === "set_app_links").at(-1);
  expect(link?.args).toMatchObject({ appId: "calendar", scheduleEnabled: true });
});

test("a version conflict surfaces as a message instead of a silent overwrite", async ({ page }) => {
  await openTodo(page);

  await page.getByTestId("todo-edit-todo-2").click();
  await page.getByTestId("todo-edit-title").fill("本地修改");
  // Someone else writes the same row first (the store, not the UI).
  await page.evaluate(() =>
    (
      window as unknown as { __TAURI_INTERNALS__: { invoke: (c: string, a: unknown) => unknown } }
    ).__TAURI_INTERNALS__.invoke("set_todo_status", {
      id: "todo-2",
      expectedVersion: 1,
      status: "doing",
    }),
  );
  await page.getByTestId("todo-edit-save").click();

  // The save is refused, the editor stays open with the reason, and what the
  // user typed is still there to retry.
  await expect(page.locator(".skill-detail-body .page-error")).toHaveText(
    "记录已被其他修改更新，请刷新后重试。",
  );
  await expect(page.getByTestId("todo-edit-title")).toHaveValue("本地修改");
});
