import { expect, test } from "@playwright/test";
import { DEMO_TODOS } from "../../src/data/workbench";
import { project, stubShell } from "./shell";

const projectPath = "/tmp/ws/alpha";
const models = [
  { modelId: "demo-flash", displayName: "Demo Flash" },
  { modelId: "demo-pro", displayName: "Demo Pro" },
];
const provider = {
  id: "demo",
  name: "Demo",
  template: "custom",
  apiKey: "••••demo",
  endpoint: "https://api.example.com/v1",
  model: "demo-flash",
  models,
};

test("scheduled task details fit narrow windows and lifecycle actions remain available", async ({ page }, testInfo) => {
  await stubShell(page);
  await page.addInitScript(() => {
    const host = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
      __scheduleLog: string[];
    };
    const invoke = host.__TAURI_INTERNALS__.invoke.bind(host.__TAURI_INTERNALS__);
    const task = {
      id: "schedule-check", title: "每日工作区状态检查", state: "enabled", archived: false,
      task: { kind: "agent", project: `/tmp/workspaces/${"long-project-name/".repeat(7)}`, message: "检查本地状态" },
      schedule: { kind: "daily", hour: 9, minute: 0, tzName: "Asia/Shanghai" },
      nextFireAt: 1_760_576_400,
    };
    host.__scheduleLog = [];
    host.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      switch (command) {
        case "list_scheduled_tasks": return task.archived ? [] : [{ ...task }];
        case "list_job_types": return ["workspace-check"];
        case "list_task_runs": return [{
          id: "run-check", lifecycle: "running", outcome: null,
          scheduledFor: 1_760_576_400, startedAt: 1_760_576_401, finishedAt: null, reasonCode: null,
        }];
        case "set_scheduled_task_enabled":
          task.state = args.enabled ? "enabled" : "paused";
          return { ...task };
        case "archive_scheduled_task": task.archived = true; return;
        case "run_scheduled_task_now": host.__scheduleLog.push(command); return {};
        case "cancel_task_run": host.__scheduleLog.push(command); return true;
        default: return invoke(command, args);
      }
    };
  });
  await page.goto("/schedule");
  const task = page.getByTestId("schedule-row-schedule-check");
  await expect(task).toContainText("每日工作区状态检查");
  await task.getByRole("button", { name: /^运行历史/ }).click();
  await expect(task.getByTestId("schedule-run-run-check")).toContainText("执行中");
  for (const theme of ["light", "dark"]) {
    await page.locator(".sidebar-foot").getByRole("button", { name: "设置", exact: true }).click();
    await page.locator(".settings-modal .settings-nav").getByRole("button", { name: "外观", exact: true }).click();
    await page.locator(".settings-modal").getByLabel("主题", { exact: true }).selectOption(theme);
    await page.locator('.settings-modal .icon-btn[aria-label="关闭设置"]').click();
    const contrast = await task.evaluate((row) => {
      const luminance = (color: string) => {
        const channels = color.match(/[\d.]+/g)!.slice(0, 3).map(Number).map((value) => {
          const channel = value / 255;
          return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
        });
        return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
      };
      const foreground = luminance(getComputedStyle(row.querySelector(".schedule-row-title")!).color);
      const background = luminance(getComputedStyle(row).backgroundColor);
      return (Math.max(foreground, background) + 0.05) / (Math.min(foreground, background) + 0.05);
    });
    expect(contrast, `${theme} task title contrast`).toBeGreaterThanOrEqual(4.5);
    for (const width of [900, 1586]) {
      await page.setViewportSize({ width, height: 900 });
      await page.screenshot({ path: testInfo.outputPath(`schedule-${theme}-${width}.png`) });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      const overflow = await task.evaluate((row) => {
        const boundary = row.getBoundingClientRect();
        return Array.from(row.querySelectorAll(".schedule-row-main button, .schedule-row-main .schedule-pill"))
          .some((item) => item.getBoundingClientRect().right > boundary.right + 1);
      });
      expect(overflow, `${theme} schedule controls at ${width}px`).toBe(false);
    }
  }
  await task.getByRole("button", { name: "立即执行", exact: true }).click();
  await task.getByRole("button", { name: "取消本次", exact: true }).click();
  expect(await page.evaluate(() => (window as unknown as { __scheduleLog: string[] }).__scheduleLog))
    .toEqual(["run_scheduled_task_now", "cancel_task_run"]);
  await task.getByRole("button", { name: "暂停", exact: true }).click();
  await page.getByTestId("schedule-filter-enabled").click();
  await expect(task).toHaveCount(0);
  await page.getByTestId("schedule-filter-paused").click();
  await expect(task.getByRole("button", { name: "恢复", exact: true })).toBeEnabled();
  await task.getByRole("button", { name: "归档", exact: true }).click();
  await expect(task).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: "没有符合条件的任务。" })).toBeVisible();
});

test("todo controls stay inside narrow cards and summary labels stay on one line", async ({ page }) => {
  await page.goto("/ui-demo/workbench?app=todo");
  await expect(page.locator(".todo-card").first()).toBeVisible();
  for (const width of [900, 1280, 1586]) {
    await page.setViewportSize({ width, height: 900 });
    const overflow = await page.locator(".todo-card").evaluateAll((cards) =>
      cards.flatMap((card) => {
        const boundary = card.getBoundingClientRect();
        return Array.from(card.querySelectorAll(".todo-card-foot select, .todo-card-foot button"))
          .filter((control) => {
            const box = control.getBoundingClientRect();
            return box.left < boundary.left || box.right > boundary.right + 1 || box.bottom > boundary.bottom + 1;
          })
          .map((control) => control.className);
      }),
    );
    expect(overflow, `card controls at ${width}px`).toEqual([]);
    const stats = await page.locator(".todo-stat").evaluateAll((items) =>
      items.map((item) => item.getBoundingClientRect().height - Number.parseFloat(getComputedStyle(item).lineHeight)),
    );
    expect(stats.every((extraHeight) => extraHeight < 1)).toBe(true);
  }
});

test("related items beyond the twelfth are selectable, cancelable, and saved for new todos", async ({ page }) => {
  const todos = Array.from({ length: 14 }, (_, index) => ({
    ...DEMO_TODOS[0],
    id: `related-${index}`,
    title: `关联事项 ${index + 1}`,
    parentId: null,
    archived: false,
    linkIds: [],
  }));
  await stubShell(page, { todos });
  await page.goto("/workbench?app=todo");
  await page.getByTestId("todo-edit-related-0").click();
  const editor = page.getByRole("dialog");
  const add = editor.getByRole("button", { name: "添加关联", exact: true });
  await expect(add).toBeEnabled();
  await expect(editor.getByRole("menu")).toHaveCount(0);
  await expect(editor.getByRole("button", { name: "上传附件", exact: true })).toBeVisible();
  await expect(editor.getByRole("button", { name: "添加提醒", exact: true })).toBeVisible();
  await add.click();
  await editor.getByTestId("todo-related-option-related-13").click();
  await expect(editor.getByRole("menu")).toHaveCount(0);
  await expect(editor.getByTestId("todo-related-related-13")).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as {
    __wbLog: Array<{ command: string }>;
  }).__wbLog.filter((entry) => entry.command === "set_todo_links"))).toEqual([]);
  await editor.getByRole("button", { name: "取消", exact: true }).click();

  await page.getByTestId("todo-edit-related-0").click();
  await expect(editor.getByTestId("todo-related-related-13")).toHaveCount(0);
  await add.click();
  await editor.getByRole("menuitem").first().press("Escape");
  await expect(editor).toBeVisible();
  await expect(add).toHaveAttribute("aria-expanded", "false");
  await add.click();
  await editor.getByTestId("todo-related-option-related-13").click();
  await editor.getByRole("button", { name: "保存并关闭", exact: true }).click();
  await expect(editor).toHaveCount(0);
  await page.getByTestId("todo-edit-related-13").click();
  await expect(editor.getByTestId("todo-related-related-0")).toBeVisible();
  await editor.getByRole("button", { name: "取消", exact: true }).click();
  await page.getByTestId("todo-edit-related-0").click();
  await expect(editor.getByTestId("todo-related-related-13")).toBeVisible();
  await editor.getByRole("button", { name: "取消", exact: true }).click();

  await page.evaluate(() => {
    const tauri = (window as unknown as { __TAURI_INTERNALS__: {
      invoke: (command: string, args?: unknown) => Promise<unknown>;
    } }).__TAURI_INTERNALS__;
    const invoke = tauri.invoke.bind(tauri);
    let failLinks = true;
    tauri.invoke = (command, args) => {
      if (command === "set_todo_links" && failLinks) {
        failLinks = false;
        return Promise.reject("storage_unavailable");
      }
      return invoke(command, args);
    };
  });
  await page.getByRole("button", { name: "新建事项", exact: true }).click();
  await page.getByTestId("todo-edit-title").fill("保留新建关联");
  await add.click();
  await editor.getByTestId("todo-related-option-related-13").click();
  await editor.getByRole("button", { name: "保存并关闭", exact: true }).click();
  await expect(editor.getByRole("alert")).toHaveText("存储不可用，请稍后重试。");
  await editor.getByRole("button", { name: "保存并关闭", exact: true }).click();
  await expect(editor).toHaveCount(0);
  const created = page.locator(".todo-card").filter({ hasText: "保留新建关联" });
  await expect(created).toHaveCount(1);
  await created.getByRole("button", { name: "编辑", exact: true }).click();
  await expect(editor.getByTestId("todo-related-related-13")).toBeVisible();
});

test("welcome stays inside the window and attaches files with the selected project", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project(projectPath, "alpha")], sessions: [] },
    pickFiles: [`${projectPath}/README.md`],
  });
  await page.addInitScript((path) => localStorage.setItem("kodo:last-project", path), projectPath);
  await page.goto("/conversation?inspector=open");
  await expect(page.locator(".project-name")).toHaveText("alpha");
  await expect(page.locator(".project-path")).toHaveText(projectPath);

  for (const width of [900, 1586, 1920]) {
    await page.setViewportSize({ width, height: 900 });
    const layout = await page.evaluate(() => {
      const box = (selector: string) => document.querySelector(selector)!.getBoundingClientRect();
      return {
        main: box(".main").height,
        stage: box(".conv-stage").height,
        inputBottom: box(".composer-input").bottom,
        controlsTop: box(".composer-row").top,
        inputWidth: box(".composer-input").width,
        composerWidth: box(".composer-box").width,
        composerBottom: box(".composer-box").bottom,
        viewportHeight: innerHeight,
        viewportWidth: innerWidth,
        scrollWidth: document.documentElement.scrollWidth,
      };
    });
    expect(Math.abs(layout.stage - layout.main)).toBeLessThanOrEqual(1);
    expect(layout.inputBottom).toBeLessThanOrEqual(layout.controlsTop);
    expect(layout.composerWidth - layout.inputWidth).toBeLessThanOrEqual(3);
    expect(layout.composerBottom).toBeLessThanOrEqual(layout.viewportHeight);
    expect(layout.scrollWidth).toBeLessThanOrEqual(layout.viewportWidth);
  }
  await page.getByRole("button", { name: "切换侧边栏", exact: true }).click();
  await expect(page.locator(".sidebar")).toBeHidden();
  await page.getByTestId("welcome-hint-attach").click();
  await expect(page.getByTestId("composer-contexts")).toContainText("README.md");
});

test("plugin lists align and MCP search leaves configured servers available", async ({ page }) => {
  await stubShell(page, {
    skills: [{ name: "notes", source: "user", taskTypes: ["docs"] }],
    mcpServers: [{
      id: "filesystem", name: "filesystem", enabled: true, transport: "stdio",
      command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      env: {}, url: "", headers: {},
    }],
  });
  await page.goto("/plugins?type=skills&inspector=open");
  await expect(page.locator(".user-skill-row")).toBeVisible();
  await expect(page.locator(".ins-content")).toContainText("使用技能");
  const row = await page.locator(".user-skill-row").evaluate((el) => {
    const rowBox = el.getBoundingClientRect();
    const actions = el.querySelector(".skill-row-actions")!.getBoundingClientRect();
    return { height: rowBox.height, actionsBottom: actions.bottom, rowBottom: rowBox.bottom };
  });
  expect(row.height).toBeLessThanOrEqual(56);
  expect(row.actionsBottom).toBeLessThanOrEqual(row.rowBottom);
  const builtin = await page.locator(".skill-row").first().boundingBox();
  const card = await page.locator(".skills-card").first().boundingBox();
  expect(card!.width - builtin!.width).toBeLessThanOrEqual(3);

  await page.getByTestId("plugins-tab-mcp").click();
  await expect(page.locator(".ins-content")).toContainText("连接外部工具");
  const servers = page.getByRole("region", { name: "已配置服务器" });
  const catalog = page.getByRole("region", { name: "连接器目录" });
  expect((await servers.boundingBox())!.y).toBeLessThan((await catalog.boundingBox())!.y);
  const search = page.getByRole("searchbox", { name: "搜索连接器名称或用途…" });
  await search.fill(" GITHUB ");
  await expect(page.locator(".mcp-card")).toHaveCount(1);
  await expect(page.locator(".mcp-card")).toContainText("github");
  await expect(servers).toContainText("filesystem");
  await search.fill("不存在的连接器");
  await expect(catalog.getByRole("status")).toContainText("没有匹配的连接器");
  await search.fill("浏览器");
  await expect(page.locator(".mcp-card")).toHaveCount(2);
  await page.locator(".mcp-card").filter({ hasText: "puppeteer" }).click();
  await expect(servers.locator(".provider-editor")).toBeVisible();
  await expect(servers.locator('input[placeholder="filesystem"]')).toHaveValue("puppeteer");
  await page.getByTestId("plugins-tab-agents").click();
  await expect(page.locator(".ins-content")).toContainText("使用智能体");
});

test("provider editor fits narrow windows and removal requires a second click", async ({ page }) => {
  await stubShell(page, { providers: [provider], providerModels: { custom: models } });
  await page.goto("/");
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "AI Provider" }).click();
  await page.getByRole("button", { name: "Demo 删除", exact: true }).click();
  await expect(page.locator(".provider-row")).toHaveCount(1);
  expect(await page.evaluate(() => (window as unknown as { __providersLog: unknown[] }).__providersLog)).toEqual([]);
  await page.getByRole("button", { name: "Demo 编辑", exact: true }).click();
  const editor = page.locator(".provider-editor");
  await expect(editor.getByLabel("API 密钥", { exact: true })).toHaveValue("••••demo");
  for (const width of [1586, 900]) {
    await page.setViewportSize({ width, height: 900 });
    const field = await editor.getByLabel("接入地址", { exact: true }).boundingBox();
    const box = await editor.boundingBox();
    expect(field!.width).toBeGreaterThan(box!.width * 0.6);
    expect(field!.x + field!.width).toBeLessThanOrEqual(box!.x + box!.width);
    expect(await editor.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  }
  const pro = editor.locator('.model-chip-label').filter({ hasText: "Demo Pro" });
  await pro.click();
  await expect(pro).toHaveAttribute("aria-pressed", "true");
  await expect(editor.locator('.model-chip-label[aria-pressed="true"]')).toHaveCount(1);
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  await expect(page.locator(".provider-row")).toContainText("Demo Pro");
  await page.getByRole("button", { name: "Demo 删除", exact: true }).click();
  await expect(page.locator(".provider-row")).toHaveCount(1);
  await page.getByRole("button", { name: "Demo 确认删除？", exact: true }).click();
  await expect(page.locator(".provider-row")).toHaveCount(0);
});
