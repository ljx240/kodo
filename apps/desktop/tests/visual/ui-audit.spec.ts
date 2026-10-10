import { expect, test } from "@playwright/test";
import { stubShell } from "./shell";

const categories = ["通用", "模型", "AI Provider", "工具与权限", "项目", "归档与存储", "外观"];

for (const width of [640, 900, 1280, 1586]) {
  test(`settings controls remain readable at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 720 });
    await page.goto("/ui-demo/settings");
    for (const category of categories) {
      await page.locator(".settings-nav").getByRole("button", { name: category, exact: true }).click();
      const layout = await page.locator(".settings-detail").evaluate((el) => {
        const box = el.getBoundingClientRect();
        const controls = Array.from(el.querySelectorAll("input, select, .toggle"));
        return {
          labels: Array.from(el.querySelectorAll(".setting-label")).map((node) => node.getBoundingClientRect().height),
          contained: controls.every((node) => {
            const r = node.getBoundingClientRect();
            return r.left >= box.left && r.right <= box.right + 1 && r.width > 20;
          }),
          overflow: el.scrollWidth > el.clientWidth,
        };
      });
      expect(layout.contained, category).toBe(true);
      expect(layout.overflow, category).toBe(false);
      expect(layout.labels.every((height) => height < 40), category).toBe(true);
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  });

  test(`trace metadata does not collide at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 720 });
    await page.goto("/ui-demo/trace");
    const layout = await page.locator(".reply-card").evaluate((el) => {
      const box = el.getBoundingClientRect();
      const meta = el.querySelector(".reply-meta")!.getBoundingClientRect();
      const summary = el.querySelector(".reply-summary")!.getBoundingClientRect();
      const copy = el.querySelector("[data-testid='copy-reply-id']")!.getBoundingClientRect();
      return { separated: meta.right <= summary.left || meta.bottom <= summary.top, contained: copy.right <= box.right };
    });
    expect(layout.separated).toBe(true);
    expect(layout.contained).toBe(true);
    await expect(page.getByRole("tab", { name: "LLM 输入输出", exact: true })).toBeVisible();
    await expect(page.getByRole("region", { name: "执行时间线" })).toHaveCSS("overflow-x", "auto");
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  });
}

test("appearance changes reach the shell immediately and survive category changes", async ({ page }) => {
  await stubShell(page);
  await page.goto("/settings");
  await page.locator(".settings-nav").getByRole("button", { name: "外观", exact: true }).click();
  await page.getByLabel("主题", { exact: true }).selectOption("dark");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByLabel("界面密度", { exact: true }).selectOption("comfortable");
  await expect(page.locator(".app")).toHaveAttribute("data-density", "comfortable");
  await page.getByRole("switch", { name: "显示行号", exact: true }).click();
  await expect(page.locator(".app")).toHaveAttribute("data-line-numbers", "false");
  await page.locator(".settings-nav").getByRole("button", { name: "通用", exact: true }).click();
  await page.locator(".settings-nav").getByRole("button", { name: "外观", exact: true }).click();
  await expect(page.getByLabel("主题", { exact: true })).toHaveValue("dark");
  await expect(page.getByRole("switch", { name: "显示行号", exact: true })).toHaveAttribute("aria-checked", "false");
  await page.getByLabel("主题", { exact: true }).selectOption("light");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("archive selection supports keyboard, Chinese details and honest empty results", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto("/ui-demo/archive");
  const first = page.locator(".archive-table .arc-title").first();
  const name = await first.innerText();
  await first.press("Enter");
  await expect(page.locator(".ins-title")).toHaveText(name);
  await expect(page.getByRole("button", { name: "恢复会话", exact: true })).toBeEnabled();
  await expect(page.getByRole("heading", { name: "会话详情", exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "归档会话列表" })).toHaveCSS("overflow-x", "auto");
  await page.getByPlaceholder("搜索归档的对话…").fill("没有匹配的归档记录");
  await expect(page.locator(".main .empty-note")).toHaveText("没有匹配的归档对话。");
});

for (const type of ["skills", "agents"]) {
  test(`${type} editor uses available height and keeps actions reachable`, async ({ page }) => {
    await page.setViewportSize({ width: 900, height: 600 });
    await page.goto(`/ui-demo/plugins?type=${type}`);
    await page.getByRole("button", { name: type === "skills" ? "新建技能" : "新建智能体", exact: true }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("button", { name: "保存", exact: true })).toBeVisible();
    const small = await dialog.locator(".skill-editor").boundingBox();
    await page.setViewportSize({ width: 900, height: 992 });
    const large = await dialog.locator(".skill-editor").boundingBox();
    expect(large!.height).toBeGreaterThan(small!.height + 90);
    expect((await dialog.getByRole("button", { name: "保存", exact: true }).boundingBox())!.y).toBeLessThan(992);
  });
}

test("all page families render in both themes without page errors", async ({ page }, testInfo) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const routes = ["conversation", "plugins", "plugins?type=agents", "plugins?type=mcp", "archive", "trace", "settings"];
  for (const theme of ["light", "dark"]) {
    for (const [index, route] of routes.entries()) {
      await page.goto(`/ui-demo/${route}`);
      // Drive the public settings controls; this also verifies subscription wiring.
      await page.locator(".sidebar-foot").getByRole("button", { name: "设置", exact: true }).click();
      await page.locator(".settings-modal .settings-nav").getByRole("button", { name: "外观", exact: true }).click();
      await page.locator(".settings-modal").getByLabel("主题", { exact: true }).selectOption(theme);
      await page.locator(".settings-modal .icon-btn[aria-label='关闭设置']").click();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await page.screenshot({ path: testInfo.outputPath(`${theme}-${index}.png`) });
    }
  }
  expect(errors).toEqual([]);
});

test("terminal preserves repeated commands without duplicate-key errors", async ({ page }) => {
  const errors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.goto("/ui-demo/conversation");
  await page.getByRole("tab", { name: "终端", exact: true }).click();
  await expect(page.locator(".terminal-run .terminal-cmd")).toHaveText(["Run command", "Run command"]);
  const outputs = await page.locator(".terminal-run .terminal-block").allTextContents();
  expect(outputs).toHaveLength(2);
  expect(outputs[0]).toContain("Finished dev");
  expect(outputs[1]).toContain("writing image");
  await page.getByRole("tab", { name: "Inspector", exact: true }).click();
  await page.getByRole("tab", { name: "终端", exact: true }).click();
  await expect(page.locator(".terminal-run .terminal-block")).toHaveText(outputs);
  expect(errors).toEqual([]);
});
