import { expect, test } from "@playwright/test";
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
