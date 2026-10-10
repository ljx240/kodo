import { expect, test, type Page } from "@playwright/test";
import { project, sessionRef, stubShell } from "./shell";

const code = Array.from({ length: 16 }, (_, i) => `阶段 ${i + 1} → 处理本地输入并保留执行结果`).join("\n");
const answer = [
  "# 项目架构说明",
  "这是一份可阅读的技术说明。**核心流程**连接入口、任务调度与本地工具，文件名 `src/main.ts` 可以直接打开。",
  "## 1. 总体架构",
  "界面负责交互，调度层负责状态和执行，工具层提供可核对的结果。",
  "### 1.1 执行流程",
  `\`\`\`text\n${code}\n\`\`\``,
  "### 1.2 模块职责",
  [
    "| 模块 | 职责 | 输入 | 输出 | 状态 | 验证 | 所属层 | 运行位置 |",
    "| --- | --- | --- | --- | --- | --- | --- | --- |",
    "| `entry.ts` | 接收请求 | 用户消息 | 任务计划 | 就绪 | 类型检查 | 界面入口 | 本地桌面 |",
    "| `tools.rs` | 执行工具 | 已批准调用 | 结构化结果 | 已完成 | 单元测试 | 工具执行 | 本地进程 |",
  ].join("\n"),
  "> [!NOTE]\n> 这里描述的是虚拟示例，不会调用真实模型或读取工作区文件。",
  "- 展示清晰的模块职责\n  - 保持嵌套列表层级\n- 保留原始代码与复制能力",
  "##### 阅读提示",
  "路径很长时也应保持在正文区域：`src/" + "module/".repeat(35) + "index.ts`。",
  "---",
  ...Array.from({ length: 20 }, (_, i) => `## ${i + 2}. 设计章节\n\n这里补充边界和模块关系。\n\n### ${i + 2}.1 说明\n\n使用段落与标题组织内容。`),
].join("\n\n");

async function openAnswers(page: Page, texts: string[]) {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "架构说明")] },
    session: {
      id: "s1", project: "/tmp/ws/alpha", title: "架构说明", at: 1_700_000_000, archived: false,
      turns: texts.map((text) => ({
        ask: "说明项目架构", context: [], done: true, stopped: false, interrupted: false, error: null,
        items: [{ id: 1, at: 1_700_000_000, status: "done", duration: 1, kind: "agentMessage", text, plain: true, delivery: "ready", verification: "not_run", checks: [] }],
      })),
    },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".md")).toHaveCount(texts.length);
}

test("long answer typography, navigation and wide content stay readable", async ({ page }, testInfo) => {
  await openAnswers(page, [answer]);
  await page.getByTestId("conversation-scroll").evaluate((el) => { el.scrollTop = 0; });
  await page.locator(".main").screenshot({ path: testInfo.outputPath("answer-start.png") });
  const toc = page.getByTestId("md-toc");
  expect((await toc.boundingBox())?.height).toBeLessThan(280);
  const typography = await page.locator(".md").evaluate((el) => ({
    body: parseFloat(getComputedStyle(el).fontSize),
    tiers: [1, 2, 3].map((tier) => parseFloat(getComputedStyle(el.querySelector(`.md-heading--${tier}`)!).fontSize)),
  }));
  expect(typography.tiers[0]).toBeGreaterThan(typography.tiers[1]);
  expect(typography.tiers[1]).toBeGreaterThan(typography.tiers[2]);
  expect(typography.tiers[2]).toBeGreaterThanOrEqual(typography.body);
  for (const width of [1586, 1280, 900]) {
    await page.setViewportSize({ width, height: 992 });
    const layout = await page.locator(".md").evaluate((el) => {
      const head = el.querySelector(".md-code-head")!.getBoundingClientRect();
      const copy = el.querySelector(".md-code-copy")!.getBoundingClientRect();
      const table = el.querySelector(".md-table-wrap")!;
      return { pageWidth: document.documentElement.scrollWidth, viewport: innerWidth, headBottom: head.bottom, headMiddle: head.left + head.width / 2, copyBottom: copy.bottom, copyLeft: copy.left, tableWidth: table.clientWidth, tableScrollWidth: table.scrollWidth };
    });
    expect(layout.pageWidth).toBeLessThanOrEqual(layout.viewport);
    expect(layout.copyBottom).toBeLessThanOrEqual(layout.headBottom);
    expect(layout.copyLeft).toBeGreaterThan(layout.headMiddle);
    if (width === 900) expect(layout.tableScrollWidth).toBeGreaterThan(layout.tableWidth);
  }
  await page.setViewportSize({ width: 1586, height: 992 });
  const summary = toc.locator("summary");
  await summary.focus();
  await page.keyboard.press("Enter");
  await expect(toc.locator("details")).not.toHaveAttribute("open");
  await page.keyboard.press("Enter");
  await expect(toc.locator("details")).toHaveAttribute("open", "");
  await page.locator(".md-code-head").evaluate((el) => el.scrollIntoView({ block: "start" }));
  await page.locator(".main").screenshot({ path: testInfo.outputPath("answer-body.png") });
  await page.getByTestId("md-code-toggle").click();
  await expect(page.locator(".md-code")).toContainText("阶段 16");
  await page.getByTestId("md-code-toggle").click();
  await expect(page.locator(".md-code")).not.toContainText("阶段 16");
});

test("each answer's directory jumps to its own heading", async ({ page }) => {
  const short = "# 总览\n\n说明。\n\n## 模块\n\n模块说明。\n\n### 流程\n\n流程说明。\n\n##### 边界\n\n边界说明。";
  const tail = Array.from({ length: 40 }, () => "保留足够正文，目录跳转可以将当前章节放到阅读区顶部。").join("\n\n");
  await openAnswers(page, [short, short + "\n\n" + tail]);
  const headings = await page.locator(".md-heading").evaluateAll((nodes) => nodes.map((node) => node.id));
  expect(new Set(headings).size).toBe(headings.length);
  const second = page.locator(".md").nth(1);
  const targetId = await second.locator(".md-heading").first().getAttribute("id");
  const link = second.getByTestId("md-toc").locator("a").first();
  await expect(link).toHaveAttribute("href", `#${targetId}`);
  await link.click();
  await expect(second.locator(".md-toc-active")).toHaveAttribute("href", `#${targetId}`);
  const top = (await second.locator(".md-heading").first().boundingBox())!.y;
  const scrollTop = (await page.getByTestId("conversation-scroll").boundingBox())!.y;
  expect(top).toBeGreaterThanOrEqual(scrollTop);
  expect(top - scrollTop).toBeLessThan(60);
});
