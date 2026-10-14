import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import type { CalendarEventDto, TodoDto, TodoStatusDefDto } from "../../src/api";
import {
  DEMO_CALENDAR_EVENTS,
  DEMO_TODO_STATUSES,
  DEMO_TODOS,
} from "../../src/data/workbench";
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

  // All three apps first, then the search narrows the rows.
  await expect(page.locator(".wb-app-row")).toHaveCount(3);
  await page.getByTestId("workbench-search").fill("日历");
  await expect(page.locator(".wb-app-row")).toHaveCount(1);
  await expect(page.locator(".wb-app-row")).toContainText("日历");
  await page.getByTestId("workbench-search").fill("");
  await expect(page.locator(".wb-app-row")).toHaveCount(3);

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
  expect(order).toEqual(["calendar", "todo", "artifacts"]);
  // …and both starred apps now lead the list, favorites first.
  await expect(page.locator(".wb-app-row").first()).toContainText("日历");
});

test("disabling an app removes it from the collection and refuses to open it", async ({ page }) => {
  await openWorkbench(page);
  await page.getByTestId("workbench-manage").click();
  await page.getByTestId("wb-enabled-artifacts").click();
  await page.getByTestId("workbench-manage-close").click();

  await expect(page.getByTestId("wb-app-artifacts")).toHaveCount(0);
  await expect(page.locator(".wb-app-row")).toHaveCount(2);

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

test("the board renders one column per status, not a fixed three", async ({ page }) => {
  await openTodo(page);

  // The column set is the status catalog — 待确认 only exists because the
  // fixture's catalog has it.
  await expect(page.locator(".todo-column-name")).toHaveText([
    "待处理",
    "进行中",
    "待确认",
    "已完成",
  ]);
  await expect(page.locator('[data-testid="todo-column-todo"] .todo-card')).toHaveCount(2);
  await expect(page.locator('[data-testid="todo-column-doing"] .todo-card')).toHaveCount(2);
  await expect(page.locator('[data-testid="todo-column-review"] .todo-card')).toHaveCount(1);
  await expect(page.locator('[data-testid="todo-column-done"] .todo-card')).toHaveCount(1);

  // A card carries its title, due date, priority and status.
  const card = page.getByTestId("todo-card-todo-1");
  await expect(card.locator(".todo-priority").first()).toHaveText("高");
  await expect(card.locator(".todo-due").first()).toContainText("09/30");
  await expect(page.getByTestId("todo-status-todo-1")).toHaveValue("doing");
  // Sub-items are counted on the parent and never float into a column.
  await expect(card.locator(".todo-subcount")).toHaveText("2/5 子项");
  // The design keeps a card to its progress line; the sub-items themselves
  // live in the editor's 子项清单, so neither the board nor the list expands
  // them. `todo-1a` is a child, never a card.
  await expect(page.getByTestId("todo-card-todo-1a")).toHaveCount(0);
  await expect(card.getByTestId("todo-child-todo-1a")).toHaveCount(0);
  await page.getByTestId("todo-edit-todo-1").click();
  await expect(page.getByTestId("todo-child-todo-1a")).toContainText(
    "核对 catalog 的注册顺序",
  );
});

test("a card wears the catalog's own chips, and its due chip reddens only when due", async ({
  page,
}) => {
  // Seeded against the real clock rather than the frozen fixture, so all four
  // due-date branches are on screen at once.
  await openTodo(page, {
    todos: [
      todo({
        id: "t-chips",
        title: "带标签的事项",
        status: "todo",
        priority: "high",
        dueDate: TODAY,
        dueTime: "12:00",
        assigneeId: "person-me",
        tagIds: ["tag-k2k", "tag-lakehouse"],
      }),
      todo({ id: "t-late", title: "已经逾期的事项", status: "todo", dueDate: isoDate(-3) }),
      todo({ id: "t-soon", title: "还没到期的事项", status: "todo", dueDate: isoDate(3) }),
      todo({ id: "t-none", title: "没有截止的事项", status: "todo", dueDate: null }),
    ],
  });

  const card = page.getByTestId("todo-card-t-chips");
  // Priority and tags are the catalog's names and colours, read off the row.
  await expect(card.locator(".todo-priority")).toHaveText("高");
  await expect(card.locator(".todo-tag")).toHaveText(["k2k-rust", "实时数仓"]);
  // The assignee is their own name's first character, not a photo.
  await expect(card.locator(".todo-avatar")).toHaveText("我");

  // Today reads as 今天 plus the time, and it is red.
  await expect(card.locator(".todo-due")).toHaveText("今天 12:00");
  await expect(card.locator(".todo-due")).toHaveClass(/todo-due--late/);

  // So is a date already behind us…
  await expect(page.getByTestId("todo-card-t-late").locator(".todo-due")).toHaveClass(
    /todo-due--late/,
  );
  // …but a date still ahead is plain, and none at all says so.
  await expect(page.getByTestId("todo-card-t-soon").locator(".todo-due")).not.toHaveClass(
    /todo-due--late/,
  );
  await expect(page.getByTestId("todo-card-t-none").locator(".todo-due")).toHaveText(
    "无截止日期",
  );
  await expect(page.getByTestId("todo-card-t-none").locator(".todo-due")).toHaveClass(
    /todo-due--none/,
  );
});

test("a card's foot is its sub-item progress, and only when it has sub-items", async ({
  page,
}) => {
  await openTodo(page);

  const withKids = page.getByTestId("todo-card-todo-1");
  await expect(withKids.locator(".todo-subcount")).toHaveText("2/5 子项");
  await expect(withKids.locator(".todo-progress-fill")).toHaveAttribute(
    "style",
    /width:\s*40%/,
  );

  // A row with none carries no foot at all — not a `0/0 子项` line.
  const noKids = page.getByTestId("todo-card-todo-2");
  await expect(noKids.locator(".todo-subcount")).toHaveCount(0);
  await expect(noKids.locator(".todo-progress")).toHaveCount(0);
});

test("a column ends in a drop zone, and a 已完成 one in 太棒了", async ({ page }) => {
  await openTodo(page, {
    todos: [
      todo({ id: "t-todo", title: "还在待处理", status: "todo" }),
      todo({ id: "t-done", title: "已经做完", status: "done" }),
    ],
  });

  // A column that cannot complete anything invites a drop…
  const drop = page.getByTestId("todo-drop-todo");
  await expect(drop).toContainText("拖拽事项到此");
  await expect(drop).toContainText("或点击上方按钮添加");
  await expect(page.getByTestId("todo-empty-todo")).toHaveCount(0);
  // …and clicking it opens the same draft the column head's ＋ does.
  await drop.click();
  await expect(page.getByRole("textbox", { name: "在「待处理」中新建" })).toBeFocused();

  // The one that means "done" celebrates instead — even with a card in it.
  await expect(page.getByTestId("todo-column-done").locator(".todo-card")).toHaveCount(1);
  await expect(page.getByTestId("todo-empty-done")).toContainText("太棒了！");
  await expect(page.getByTestId("todo-empty-done")).toContainText("该状态下没有更多事项了");
  await expect(page.getByTestId("todo-drop-done")).toHaveCount(0);
});

test("the stats bar counts the top level and never the sub-items", async ({ page }) => {
  await openTodo(page);

  // Seven top-level rows: five open, one done, one archived. The nine
  // sub-items are rows too, and none of them is counted.
  const stats = page.getByTestId("todo-stats");
  await expect(stats).toContainText("共 7 个事项");
  await expect(stats).toContainText("未完成 5");
  await expect(stats).toContainText("已完成 1");
  await expect(stats).toContainText("已归档 1");
  await expect(stats).toContainText("支持拖拽调整顺序，自定义列状态，提升团队协作效率");

  // Creating a row moves the total, and 未完成 with it — one source of truth.
  await page.getByTestId("todo-new").click();
  await page.getByTestId("todo-edit-title").fill("再加一条");
  await page.getByTestId("todo-edit-save").click();
  await expect(stats).toContainText("共 8 个事项");
  await expect(stats).toContainText("未完成 6");
});

test("a status added to the catalog becomes a column of its own", async ({ page }) => {
  const statuses: TodoStatusDefDto[] = [
    ...DEMO_TODO_STATUSES.slice(0, 2),
    {
      ...DEMO_TODO_STATUSES[0],
      id: "blocked",
      name: "被阻塞",
      color: "red",
      builtin: false,
      sortOrder: 2,
    },
  ];
  await openTodo(page, {
    todoStatuses: statuses,
    todos: [todo({ id: "t-blocked", title: "等上游修复", status: "blocked" })],
  });

  await expect(page.locator(".todo-column-name")).toHaveText(["待处理", "进行中", "被阻塞"]);
  // The card landed in the new column and kept its status id.
  await expect(page.locator('[data-testid="todo-column-blocked"] .todo-card')).toHaveCount(1);
  await expect(page.getByTestId("todo-status-t-blocked")).toHaveValue("blocked");
});

test("board and list are two views of one dataset", async ({ page }) => {
  await openTodo(page);

  await page.getByTestId("todo-view-list").click();
  await expect(page.getByTestId("todo-list").locator(".todo-row")).toHaveCount(6);

  // The filter applies to both views from the same rows.
  await page.getByTestId("todo-filter-open").click();
  await expect(page.getByTestId("todo-list").locator(".todo-row")).toHaveCount(5);

  await page.getByTestId("todo-filter-high").click();
  await expect(page.getByTestId("todo-list").locator(".todo-row")).toHaveCount(2);
  await expect(page.getByTestId("todo-row-todo-1")).toBeVisible();
  await expect(page.getByTestId("todo-row-todo-3")).toBeVisible();

  await page.getByTestId("todo-filter-all").click();
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
      todo({ id: "t-far", title: "很久以后的事", status: "todo", dueDate: isoDate(30) }),
      todo({ id: "t-past", title: "很久以前的事", status: "todo", dueDate: isoDate(-30) }),
    ],
  });

  await page.getByTestId("todo-filter-today").click();
  // Both of today's rows show — 今天 is a due-date filter, not a status one.
  await expect(page.locator(".todo-column-body .todo-card")).toHaveCount(2);
  await expect(page.getByTestId("todo-filter-today").locator(".todo-chip-count")).toHaveText("2");
  await expect(page.getByTestId("todo-card-t-far")).toHaveCount(0);

  // 本周 is the same rule one week wide: it still holds today's rows and still
  // excludes both dates a month away, whatever weekday today happens to be.
  await page.getByTestId("todo-filter-week").click();
  const scheduled = await page.locator(".todo-column-body .todo-card").count();
  expect(scheduled).toBeGreaterThanOrEqual(2);
  await expect(page.getByTestId("todo-card-t-far")).toHaveCount(0);
  await expect(page.getByTestId("todo-card-t-past")).toHaveCount(0);
  // The chip's count is the number clicking it reveals — one source of truth.
  await expect(page.getByTestId("todo-filter-week").locator(".todo-chip-count")).toHaveText(
    String(scheduled),
  );
});

test("creating needs only a title, and the status decides the column", async ({ page }) => {
  await openTodo(page);

  // 新建事项 opens the editor on a blank draft; the title is the only required
  // field, and the status it carries is where the row lands.
  await page.getByTestId("todo-new").click();
  await expect(page.getByTestId("todo-edit-title")).toHaveValue("");
  await page.getByTestId("todo-edit-title").fill("写一条巡检结论");
  await page.getByTestId("todo-edit-status").selectOption("doing");
  await page.getByTestId("todo-edit-save").click();
  await expect(page.locator('[data-testid="todo-column-doing"] .todo-card')).toHaveCount(3);
  await expect(page.locator('[data-testid="todo-column-todo"] .todo-card')).toHaveCount(2);

  // Per-column create stays the quick path: a title and Enter, in that column.
  await page.getByTestId("todo-add-review").click();
  await page.getByRole("textbox", { name: "在「待确认」中新建" }).fill("补一条回归用例");
  await page.keyboard.press("Enter");
  await expect(page.locator('[data-testid="todo-column-review"] .todo-card')).toHaveCount(2);

  const created = (await wbLog(page)).filter((row) => row.command === "create_todo");
  expect(created.map((row) => (row.args.draft as { status: string }).status)).toEqual([
    "doing",
    "review",
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

  // The list view agrees, because both read the one status field: the row is
  // still there, and it wears the done styling.
  await page.getByTestId("todo-view-list").click();
  await page.getByTestId("todo-filter-all").click();
  await expect(page.getByTestId("todo-row-todo-2").locator(".todo-row-title--done")).toBeVisible();

  // 未完成 is the chip that parts the two out.
  await page.getByTestId("todo-filter-open").click();
  await expect(page.getByTestId("todo-row-todo-2")).toHaveCount(0);

  await page.getByTestId("todo-filter-all").click();
  await page.getByTestId("todo-toggle-todo-2").click();
  await expect(page.getByTestId("todo-row-todo-2").locator(".todo-row-title--done")).toHaveCount(0);

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

  await expect(page.locator('[data-testid="todo-column-doing"] .todo-card')).toHaveCount(3);
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

  await expect(page.locator('[data-testid="todo-column-todo"] .todo-card')).toHaveCount(2);
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
  await expect(card.locator(".todo-due")).toContainText("10/05");
  await expect(card.locator(".todo-subcount")).toHaveText("0/1 子项");
  expect(await commands(page)).toContain("update_todo");
  // The sub-item was created, not saved as part of the parent's row.
  const subtask = (await wbLog(page)).filter((row) => row.command === "create_todo");
  expect((subtask[0].args.draft as { parentId?: string }).parentId).toBe("todo-5");
});

test("the two-column editor round-trips every new field on save", async ({ page }) => {
  await openTodo(page);

  await page.getByTestId("todo-edit-todo-5").click();
  // The two columns are visible — main + rail — and the rail carries the
  // catalog-derived sections.
  await expect(page.locator(".todo-editor-main")).toBeVisible();
  await expect(page.locator(".todo-editor-rail")).toBeVisible();
  await expect(page.getByTestId("todo-edit-assignee")).toBeVisible();
  await expect(page.getByTestId("todo-add-attachment")).toBeVisible();
  await expect(page.getByTestId("todo-add-reminder")).toBeVisible();
  await expect(page.getByTestId("todo-comment-input")).toBeVisible();

  // The new fields all carry their values to the save.
  await page.getByTestId("todo-edit-title").fill("更新发布说明里的 v1.2 段落");
  await page.getByTestId("todo-edit-due").fill("2026-10-15");
  await page.getByTestId("todo-edit-due-time").fill("18:00");
  await page.getByTestId("todo-edit-item-type").selectOption("story");
  await page.getByTestId("todo-edit-module").fill("docs/release-notes");
  await page.getByTestId("todo-edit-estimate").fill("2.5");
  await page.getByTestId("todo-edit-assignee").selectOption("person-zhang");
  // Add a tag the row doesn't already carry — the picker toggles, so the
  // chip's `aria-pressed` flips on. todo-5's fixture carries `tag-docs` only.
  await page.getByTestId("todo-edit-tag-tag-lakehouse").click();
  await page.getByTestId("todo-edit-save").click();

  // The card reflects the new title.
  const card = page.getByTestId("todo-card-todo-5");
  await expect(card.locator(".todo-card-title")).toHaveText(
    "更新发布说明里的 v1.2 段落",
  );

  // Reopen and confirm: the rail reads the same data back, not a stale copy.
  await page.getByTestId("todo-edit-todo-5").click();
  await expect(page.getByTestId("todo-edit-due-time")).toHaveValue("18:00");
  await expect(page.getByTestId("todo-edit-item-type")).toHaveValue("story");
  await expect(page.getByTestId("todo-edit-module")).toHaveValue("docs/release-notes");
  await expect(page.getByTestId("todo-edit-estimate")).toHaveValue("2.5");
  await expect(page.getByTestId("todo-edit-assignee")).toHaveValue("person-zhang");
  await expect(page.getByTestId("todo-edit-tag-tag-lakehouse")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  // The save and close button is the one labelled "保存并关闭", not the
  // short "保存" any more.
  await expect(page.getByTestId("todo-edit-save")).toHaveText("保存并关闭");
  await page.getByTestId("todo-edit-save").click();

  // The wire carried every new field — `update_todo`'s `todo` payload has them.
  const update = (await wbLog(page)).find((row) => row.command === "update_todo");
  const payload = update?.args.todo as Record<string, unknown>;
  expect(payload.itemType).toBe("story");
  expect(payload.module).toBe("docs/release-notes");
  expect(payload.dueTime).toBe("18:00");
  expect(payload.assigneeId).toBe("person-zhang");
  expect(payload.tagIds).toEqual(["tag-docs", "tag-lakehouse"]);
  expect(payload.estimateHours).toBe(2.5);
});

test("the status manager adds, renames and recolours statuses", async ({ page }) => {
  await openTodo(page);

  // The four built-in + custom statuses seed the modal's rows.
  await page.getByTestId("todo-manage-statuses").click();
  const rows = page.locator(".todo-status-modal .todo-status-row");
  await expect(rows).toHaveCount(4);

  // A new status becomes a column on the board once the modal closes.
  await page.getByTestId("todo-status-new-name").fill("已搁置");
  await page.getByTestId("todo-status-new-color").selectOption("purple");
  await page.getByTestId("todo-status-new").click();
  await expect(rows).toHaveCount(5);
  expect(await commands(page)).toContain("create_todo_status");
  await page.getByTestId("todo-status-close").click();
  await expect(page.locator(".todo-column-name")).toHaveText([
    "待处理",
    "进行中",
    "待确认",
    "已完成",
    "已搁置",
  ]);

  // Renaming a status flips the column header in the same render.
  await page.getByTestId("todo-manage-statuses").click();
  const nameInput = page.getByTestId("todo-status-name-review");
  await nameInput.fill("待评审");
  await nameInput.blur();
  expect(
    (await wbLog(page))
      .filter((row) => row.command === "update_todo_status")
      .map((row) => (row.args.status as { name: string }).name),
  ).toContain("待评审");
  await page.getByTestId("todo-status-close").click();
  await expect(page.getByTestId("todo-col-review").locator(".todo-column-name")).toHaveText("待评审");

  // Recolouring writes the change, the chip picks it up.
  await page.getByTestId("todo-manage-statuses").click();
  await page.getByTestId("todo-status-color-select-review").selectOption("teal");
  expect(
    (await wbLog(page))
      .filter((row) => row.command === "update_todo_status")
      .map((row) => (row.args.status as { color: string }).color),
  ).toContain("teal");
  await page.getByTestId("todo-status-close").click();
  await expect(
    page.getByTestId("todo-col-review").locator(".todo-column-head .todo-status-dot"),
  ).toHaveAttribute("data-color", "teal");
});

test("an empty user status can be deleted with no reassignment target", async ({ page }) => {
  await openTodo(page);

  // Create a brand-new status with no todos attached, then delete it.
  await page.getByTestId("todo-manage-statuses").click();
  await page.getByTestId("todo-status-new-name").fill("已搁置");
  await page.getByTestId("todo-status-new").click();
  expect(await commands(page)).toContain("create_todo_status");
  // The delete is straight: nothing in this column, no picker surfaces.
  // The stub assigns `st-stub-N` for new statuses, so we click the matching
  // row without depending on the exact id.
  const created = page.locator('[data-testid^="todo-status-delete-st-stub-"]').first();
  await created.click({ force: true });
  expect(await commands(page)).toContain("delete_todo_status");
  await page.getByTestId("todo-status-close").click();
  await expect(page.locator(".todo-column-name")).toHaveText([
    "待处理",
    "进行中",
    "待确认",
    "已完成",
  ]);
});

test("a status with rows can only be deleted after a reassignment", async ({ page }) => {
  // Add a non-builtin status that actually holds a todo, so the delete
  // path has to ask for a target rather than going through.
  const blocked: TodoStatusDefDto = {
    ...DEMO_TODO_STATUSES[2],
    id: "blocked",
    name: "被阻塞",
    color: "red",
    builtin: false,
    sortOrder: 4,
  };
  await openTodo(page, {
    todoStatuses: [...DEMO_TODO_STATUSES, blocked],
    todos: [todo({ id: "t-blocked", title: "等上游修复", status: "blocked" })],
  });

  await page.getByTestId("todo-manage-statuses").click();
  await page.getByTestId("todo-status-delete-blocked").click();
  // The reassign selector appears because the column is non-empty.
  await expect(page.getByTestId("todo-status-reassign-blocked")).toBeVisible();
  // Picking a target commits the move in one call, not two.
  await page.getByTestId("todo-status-reassign-blocked").selectOption("todo");
  const deletes = (await wbLog(page)).filter(
    (row) => row.command === "delete_todo_status",
  );
  expect(deletes).toHaveLength(1);
  const args = deletes[0].args as { reassignTo: string | null };
  expect(args.reassignTo).toBe("todo");
  await page.getByTestId("todo-status-close").click();
  // The card that used to live under 被阻塞 is now under 待处理.
  await expect(page.getByTestId("todo-col-todo").locator(".todo-card")).toHaveCount(1);
  await expect(page.locator(".todo-column-name")).toHaveText([
    "待处理",
    "进行中",
    "待确认",
    "已完成",
  ]);
});

test("a built-in status cannot be deleted and the buttons are honest about it", async ({ page }) => {
  await openTodo(page);
  await page.getByTestId("todo-manage-statuses").click();
  await expect(page.getByTestId("todo-status-delete-todo")).toBeDisabled();
  await expect(page.getByTestId("todo-status-delete-doing")).toBeDisabled();
  await expect(page.getByTestId("todo-status-delete-done")).toBeDisabled();
  await expect(page.getByTestId("todo-status-delete-review")).toBeEnabled();
  await page.getByTestId("todo-status-close").click();
});

test("arrow keys on a status grip reorder the catalog", async ({ page }) => {
  await openTodo(page);
  await page.getByTestId("todo-manage-statuses").click();

  // `review` starts at index 2; two ArrowUp presses should walk it past
  // `doing` and `todo` to the head of the list. Each press writes a reorder.
  const grip = page.getByTestId("todo-status-grip-review");
  await grip.focus();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("ArrowUp");
  const reorders = (await wbLog(page)).filter(
    (row) => row.command === "reorder_todo_statuses",
  );
  expect(reorders).toHaveLength(2);
  const last = reorders[reorders.length - 1].args as { ids: string[] };
  expect(last.ids).toEqual(["review", "todo", "doing", "done"]);
  await page.getByTestId("todo-status-close").click();
  await expect(page.locator(".todo-column-name")).toHaveText([
    "待确认",
    "待处理",
    "进行中",
    "已完成",
  ]);
});

test("dragging a card within its column reorders it via reorder_todo", async ({ page }) => {
  await openTodo(page);

  // todo-2 and todo-3 both sit in 待处理; dropping todo-2 on top of todo-3
  // should call reorder_todo with beforeId=todo-3, not set_todo_status.
  const handle = page.getByTestId("todo-handle-todo-2");
  await handle.scrollIntoViewIfNeeded();
  const target = page.getByTestId("todo-card-todo-3");
  await target.scrollIntoViewIfNeeded();
  const from = (await handle.boundingBox())!;
  const to = (await target.boundingBox())!;

  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  // Stay inside the same column while moving, so the columnAt() never crosses.
  await expect(page.locator(".todo-drag-ghost")).toHaveText("整理实时数仓搭建流程的说明文档");
  await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 8 });
  await page.mouse.up();

  const reorders = (await wbLog(page)).filter(
    (row) => row.command === "reorder_todo",
  );
  expect(reorders).toHaveLength(1);
  const args = reorders[0].args as { id: string; status: string; beforeId: string | null };
  expect(args.id).toBe("todo-2");
  expect(args.status).toBe("todo");
  expect(args.beforeId).toBe("todo-3");
  // The move did not also rewrite the status.
  const statusWrites = (await commands(page)).filter(
    (name) => name === "set_todo_status",
  );
  expect(statusWrites).toEqual([]);
});

test("a cancelled in-column drag leaves the order alone", async ({ page }) => {
  await openTodo(page);

  const handle = page.getByTestId("todo-handle-todo-2");
  const box = (await handle.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  // Move inside the same column, then cancel with Escape.
  await expect(page.locator(".todo-drag-ghost")).toBeVisible();
  await page.mouse.move(box.x + 40, box.y + 40, { steps: 4 });
  await page.keyboard.press("Escape");
  await expect(page.locator(".todo-drag-ghost")).toHaveCount(0);
  await page.mouse.up();

  expect(
    (await commands(page)).filter(
      (name) => name === "reorder_todo" || name === "set_todo_status",
    ),
  ).toEqual([]);
});

test("archiving takes a row off the board and into the list's 已归档 fold", async ({ page }) => {
  await openTodo(page);

  // The board has no column an archived row could sit in, so the fixture's one
  // archived row is simply not on it.
  await expect(page.getByTestId("todo-card-todo-7")).toHaveCount(0);
  await expect(page.getByTestId("todo-archived")).toHaveCount(0);

  // The list folds them away: the count is there, the rows are not.
  await page.getByTestId("todo-view-list").click();
  await expect(page.getByTestId("todo-archived-toggle")).toContainText("1");
  await expect(page.getByTestId("todo-archived-list")).toHaveCount(0);
  await page.getByTestId("todo-archived-toggle").click();
  await expect(page.getByTestId("todo-archived-list").locator(".todo-row")).toHaveCount(1);

  // Archiving a live card takes it off the board and into the same fold.
  await page.getByTestId("todo-view-board").click();
  await page.getByTestId("todo-edit-todo-6").click();
  await page.getByTestId("todo-edit-archive").click();
  await expect(page.getByTestId("todo-card-todo-6")).toHaveCount(0);

  await page.getByTestId("todo-view-list").click();
  await expect(page.getByTestId("todo-archived-list").locator(".todo-row")).toHaveCount(2);

  await page.getByTestId("todo-unarchive-todo-6").click();
  await expect(page.getByTestId("todo-archived-list").locator(".todo-row")).toHaveCount(1);
  await expect(page.getByTestId("todo-list").getByTestId("todo-row-todo-6")).toBeVisible();
  expect(await commands(page)).toContain("set_todo_archived");
});

test("every chip counts what clicking it reveals, and the search narrows both", async ({ page }) => {
  await openTodo(page);

  // Six live rows — five unfinished, two 高优先级 — and the one archived row.
  await expect(page.getByTestId("todo-filter-all").locator(".todo-chip-count")).toHaveText("6");
  await expect(page.getByTestId("todo-filter-open").locator(".todo-chip-count")).toHaveText("5");
  await expect(page.getByTestId("todo-filter-high").locator(".todo-chip-count")).toHaveText("2");
  await expect(page.getByTestId("todo-filter-archived").locator(".todo-chip-count")).toHaveText("1");
  await expect(page.getByTestId("todo-filter-all")).toHaveAttribute("aria-pressed", "true");

  // The search is a row filter rather than a seventh chip: it narrows the rows
  // and the counts with them, so a chip can never promise a number the board
  // cannot deliver.
  await page.getByTestId("todo-search").fill("k2k");
  await expect(page.getByTestId("todo-filter-all").locator(".todo-chip-count")).toHaveText("3");
  await expect(page.locator(".todo-card")).toHaveCount(3);
  await expect(page.getByTestId("todo-filter-archived").locator(".todo-chip-count")).toHaveText("0");

  // ⌘K is the keyboard way to the same box.
  await page.getByTestId("todo-search").fill("");
  await page.keyboard.press("Meta+k");
  await expect(page.getByTestId("todo-search")).toBeFocused();
});

test("排序 reorders the list and writes nothing", async ({ page }) => {
  await openTodo(page);
  await page.getByTestId("todo-view-list").click();
  const rows = page.getByTestId("todo-list").locator(".todo-row-title");

  // 按优先级: both 高 rows lead, and no 高 is left behind them.
  await page.getByTestId("todo-menu-sort").click();
  await page.getByTestId("todo-menu-sort-priority").click();
  const byPriority = await rows.allTextContents();
  expect(byPriority.slice(0, 2)).toEqual([
    "复核 k2k-rust 未知表路由的兜底分支",
    "确认 Flink 任务的延迟基线",
  ]);

  // 按截止时间: soonest first, and the undated rows keep to the end in their
  // stored order rather than being scattered among the dated ones.
  await page.getByTestId("todo-menu-sort").click();
  await page.getByTestId("todo-menu-sort-due").click();
  const byDue = await rows.allTextContents();
  expect(byDue[0]).toBe("确认 Flink 任务的延迟基线");
  expect(byDue.slice(-2)).toEqual(["归档 09-25 的路由排查记录", "补齐 k2k-rust 的回归用例"]);

  // An order is a view, not an edit.
  const writes = (await commands(page)).filter(
    (name) => name === "update_todo" || name === "reorder_todo" || name === "bulk_update_todos",
  );
  expect(writes).toEqual([]);
});

test("分组 splits the list into catalog-ordered buckets, and only in the list", async ({
  page,
}) => {
  await openTodo(page);

  // The board is grouped by status because its columns are statuses; the list
  // only groups when asked, which is why the menu is not on the board.
  await expect(page.getByTestId("todo-menu-group")).toHaveCount(0);
  await page.getByTestId("todo-view-list").click();
  await expect(page.getByTestId("todo-group-todo")).toHaveCount(0);

  await page.getByTestId("todo-menu-group").click();
  await page.getByTestId("todo-menu-group-status").click();

  // Catalog order, one bucket per status that has rows, and no rows left over.
  await expect(page.locator(".todo-list .todo-group-name")).toHaveText([
    "待处理",
    "进行中",
    "待确认",
    "已完成",
  ]);
  await expect(page.getByTestId("todo-group-todo").locator(".todo-row")).toHaveCount(2);
  await expect(page.getByTestId("todo-group-doing").locator(".todo-row")).toHaveCount(2);
  await expect(page.getByTestId("todo-group-review").locator(".todo-row")).toHaveCount(1);
  await expect(page.getByTestId("todo-group-done").locator(".todo-row")).toHaveCount(1);
});

test("the selection bar writes the whole selection in one command", async ({ page }) => {
  await openTodo(page);
  await page.getByTestId("todo-view-list").click();

  await page.getByTestId("todo-select-todo-2").check();
  await page.getByTestId("todo-select-todo-3").check();
  // The bar replaces the toolbar, so 筛选/排序 are not fighting it for the row.
  await expect(page.getByTestId("todo-selection")).toContainText("已选择 2 项");
  await expect(page.getByTestId("todo-menu-sort")).toHaveCount(0);

  await page.getByTestId("todo-bulk-priority").click();
  await page.getByTestId("todo-bulk-priority-high").click();

  // One round trip for both rows, and the bar closes on the response.
  const writes = (await wbLog(page)).filter((row) => row.command === "bulk_update_todos");
  expect(writes).toHaveLength(1);
  expect(writes[0].args).toMatchObject({
    ids: ["todo-2", "todo-3"],
    op: { op: "setPriority", priority: "high" },
  });
  await expect(page.getByTestId("todo-selection")).toHaveCount(0);
  await expect(page.getByTestId("todo-row-todo-2").locator(".todo-priority")).toHaveText("高");
  await expect(page.getByTestId("todo-row-todo-3").locator(".todo-priority")).toHaveText("高");
});

test("自定义列 is a stored preference, not a session-only view", async ({ page }) => {
  await openTodo(page);
  await page.getByTestId("todo-view-list").click();

  // The full table starts on — the design shows the whole thing, and the menu
  // is there to take a column away. Hiding 更新时间 drops it from both the head
  // and the rows, and the column is no longer the design's first choice.
  const updatedHead = page.locator(".todo-list-head .todo-head-cell", { hasText: "更新时间" });
  await expect(updatedHead).toBeVisible();
  await page.getByTestId("todo-columns").click();
  await page.getByTestId("todo-column-updated").click();
  await page.keyboard.press("Escape");
  await expect(updatedHead).toHaveCount(0);
  await expect(page.getByTestId("todo-row-todo-4").locator(".todo-updated")).toHaveCount(0);

  // The toggle flips it back on, in the design's order.
  await page.getByTestId("todo-columns").click();
  await page.getByTestId("todo-column-updated").click();
  await page.keyboard.press("Escape");
  await expect(updatedHead).toBeVisible();

  // It lands in the workbench prefs in the design's own order, whatever order
  // the columns were switched on in.
  const pref = (await wbLog(page)).filter((row) => row.command === "set_workbench_pref").at(-1);
  expect(pref?.args).toMatchObject({ key: "todo.columns" });
  expect(JSON.parse(String(pref?.args.value))).toEqual([
    "status",
    "priority",
    "due",
    "tags",
    "assignee",
    "progress",
    "updated",
  ]);
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
  await page.getByTestId("todo-new").click();
  await page.getByTestId("todo-edit-title").fill("离线也能记一条");
  await page.getByTestId("todo-edit-save").click();
  await expect(page.locator('[data-testid="todo-column-todo"] .todo-card')).toHaveCount(3);

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
  await expect(page.locator(".todo-editor-body .page-error")).toHaveText(
    "记录已被其他修改更新，请刷新后重试。",
  );
  await expect(page.getByTestId("todo-edit-title")).toHaveValue("本地修改");
});
