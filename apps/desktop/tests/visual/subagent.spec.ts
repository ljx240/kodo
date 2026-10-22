import { expect, test, type Page } from "@playwright/test";
import type { ItemDto, SubagentTraceDto, TurnDto } from "../../src/api";
import { toReply } from "../../src/conversation/trace";
import { buildTraceMap } from "../../src/traceMap/build";
import { emit, project, sessionRef, stubShell } from "./shell";

const trace: SubagentTraceDto = {
  id: "child-1", parentId: "s1", agent: "reviewer", task: "检查路由并返回证据",
  status: "completed", output: "子任务结论：已检查路由。", inputTokens: 120, outputTokens: 30,
  omittedSteps: 0,
  steps: [{ kind: "fileRead", label: "src/router.ts", status: "done", output: "synthetic routing evidence", durationMs: 8 }],
};

function child(over: Partial<SubagentTraceDto> = {}): ItemDto {
  return { id: 1, at: 1_700_000_000, status: "done", duration: 100, kind: "subagent", trace: { ...trace, ...over } };
}

function core(items: ItemDto[] = [], done = false) {
  return {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "子任务验证")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "子任务验证", at: 1_700_000_000, archived: false,
      turns: items.length ? [{ ask: "检查路由", context: [], items, done, stopped: false, interrupted: false, error: null }] : [] },
    providers: [{ id: "p1", name: "Local", template: "openai", apiKey: "synthetic-key", endpoint: "", model: "gpt-test" }],
    settings: { "active-provider": "0" },
  };
}

async function open(page: Page) {
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("子任务验证");
}

test("child output stays separate from parent answer, evidence and token accounting", () => {
  const turn = { ask: "检查路由", context: [], items: [child()], done: true, stopped: false, error: null } as TurnDto;
  const reply = toReply(turn, false);
  expect(reply.final).toBeNull();
  expect(reply.verify).toEqual({ passed: 0, failed: 0, blocked: 0, notRun: 0 });
  expect(reply.tokens).toBe("120 → 30");
  expect(reply.steps[0].type).toBe("subagent");
  expect(buildTraceMap(turn.items).nodes[0].label).toBe("子 Agent");
});

test("slash command is discoverable by keyboard and retains the task text", async ({ page }) => {
  await stubShell(page, core());
  await open(page);
  const input = page.locator(".composer-input");
  await input.fill("/sub");
  await expect(page.locator('[data-slash-id="subagent"]')).toBeVisible();
  await input.press("Tab");
  await expect(input).toHaveValue("/subagent ");
  await expect(page.locator(".composer-skill")).toHaveCount(0);
  await input.fill("/subagent @reviewer 检查路由");
  await input.press("Enter");
  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __sendLog?: { text: string }[] }).__sendLog?.at(-1)?.text,
  )).toContain("/subagent @reviewer 检查路由");
});

test("live child updates one row and its result never becomes the parent answer", async ({ page }) => {
  await stubShell(page, core());
  await open(page);
  await page.locator(".composer-input").fill("检查路由");
  await page.locator(".composer-input").press("Enter");
  await emit(page, { type: "itemStarted", session: "s1", item: { ...child({ status: "running", output: "" }), status: "running" } });
  await page.locator('[data-testid="process-tools"] > summary').click();
  const details = page.getByTestId("subagent-details");
  await expect(details).toContainText("执行中");
  await expect(details).toContainText("child-1");
  await emit(page, { type: "itemCompleted", session: "s1", item: child() });
  await expect(page.locator(".trace-row")).toHaveCount(1);
  await page.locator(".trace-row-inner").click();
  await expect(details).toContainText("已完成");
  await expect(details.getByTestId("subagent-result")).toHaveText(trace.output);
  await expect(page.locator(".reply-working")).toBeVisible();
  await details.locator("summary").focus();
  await details.locator("summary").press("Enter");
  await expect(details).toContainText("synthetic routing evidence");
});

test("saved child trace reloads in its dedicated tab and stale running is interrupted", async ({ page }) => {
  await stubShell(page, core([{ ...child({ status: "running", output: "partial" }), status: "running" }], false));
  await open(page);
  await page.goto("/trace?tab=agents");
  const tab = page.getByRole("tab", { name: "子 Agent", exact: true });
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("subagent-details")).toContainText("已中断");
  await page.reload();
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("subagent-result")).toHaveText("partial");
  await page.setViewportSize({ width: 800, height: 900 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("subagent-trace.png"), fullPage: true });
});

test("trace page follows a live child while retaining one persisted identity", async ({ page }) => {
  const live = core([{ ...child({ status: "running", output: "" }), status: "running" }]);
  await stubShell(page, { ...live, runStates: { s1: { running: true } } });
  await open(page);
  await page.goto("/trace?tab=agents");
  await expect(page.getByTestId("subagent-details")).toContainText("执行中");
  await expect(page.getByTestId("subagent-details")).not.toContainText("已中断");
  await emit(page, { type: "itemStarted", session: "s1", runId: 10, item: { ...child({ status: "running", output: "" }), status: "running" } });
  await emit(page, { type: "itemCompleted", session: "s1", runId: 9, item: child({ output: "stale output" }) });
  await expect(page.getByTestId("subagent-result")).toHaveCount(0);
  await emit(page, { type: "itemCompleted", session: "s1", runId: 10, item: child() });
  await expect(page.getByTestId("subagent-details")).toHaveCount(1);
  await expect(page.getByTestId("subagent-result")).toHaveText(trace.output);
  await emit(page, { type: "turnComplete", session: "s1", runId: 10 });
  await expect(page.getByTestId("subagent-details")).toContainText("已完成");
});
