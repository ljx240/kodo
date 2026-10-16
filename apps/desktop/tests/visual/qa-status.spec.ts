import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { emit, project, sessionRef, stubShell, type Core } from "./shell";

/**
 * Q&A experience contract: the three-axis run state, merged failure causes,
 * honest counters, phase-grouped trace, conclusion-first answers, the file
 * ownership split, and the zh-CN label map. These assert the product rules the
 * redesign introduced — the other suites keep covering shell/layout/a11y.
 */

function liveCore(session: unknown, extra: Record<string, unknown> = {}): Core {
  return {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "新对话")],
    },
    session,
    ...extra,
  };
}

async function openLive(page: Page) {
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("新对话");
}

function runItem(id: number, over: Record<string, unknown> = {}) {
  return {
    id,
    at: 1_700_000_000,
    status: "done",
    duration: 100,
    kind: "commandExecution",
    command: "cargo check",
    cwd: "/tmp/ws/alpha",
    output: "ok",
    exitCode: 0,
    ...over,
  };
}

function answerItem(id: number, text = "做好了。", over: Record<string, unknown> = {}) {
  return { id, at: 1_700_000_000, status: "done", duration: 1, kind: "agentMessage", text, checks: [], ...over };
}

function turn(ask: string, items: unknown[], over: Record<string, unknown> = {}) {
  return { ask, context: [], items, done: true, stopped: false, interrupted: false, error: null, ...over };
}

test("status matrix: lifecycle × delivery × verification, tool failure ≠ answer failure", async ({ page }) => {
  await stubShell(
    page,
    liveCore(
      {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "新对话",
        at: 1_700_000_000,
        archived: false,
        turns: [
          // 1. Answer only → complete (nothing to verify, nothing written).
          turn("q1", [answerItem(1)]),
          // 2. Answer + files, no verification → partially completed.
          turn("q2", [
            { id: 1, at: 1_700_000_000, status: "done", duration: 5, kind: "fileChange", changes: [{ path: "src/a.ts", added: 2, removed: 0 }] },
            answerItem(2),
          ]),
          // 3. Answer + a passing check → completed.
          turn("q3", [runItem(1), answerItem(2)]),
          // 4. Usable answer with a failed check → partially completed, never 失败.
          turn("q4", [
            runItem(1),
            runItem(2, { command: "cargo test", exitCode: 1, output: "FAIL", failureClass: "non_zero_exit" }),
            answerItem(3, "修改已完成，测试没过。"),
          ]),
          // 5. Backend-computed axes win outright — no string guessing on the client.
          turn(
            "q5",
            [runItem(1, { exitCode: 2, output: "blocked", failureClass: "command_not_found", failureTool: "cargo" }), answerItem(2)],
            {
              status: {
                lifecycle: "completed",
                delivery: "ready",
                verification: "blocked",
                outcome: "blocked_by_environment",
              },
            },
          ),
        ],
      },
    ),
  );
  await openLive(page);

  await expect(page.locator('[data-testid="final-outcome-pill"]')).toHaveText([
    "已完成",
    "部分完成",
    "已完成",
    "部分完成",
    "环境阻塞",
  ]);
  await expect(page.locator('[data-testid="answer-axes"]')).toHaveText([
    "回答：已生成 · 验证：未验证",
    "回答：已生成 · 验证：未验证",
    "回答：已生成 · 验证：已通过",
    "回答：已生成 · 验证：未通过",
    "回答：已生成 · 验证：已阻塞",
  ]);

  // The q4 answer still renders its body and its own verification card.
  const q4 = page.locator(".reply").nth(3);
  await expect(q4.locator('[data-testid="answer-body"]')).toContainText("修改已完成，测试没过。");
  await expect(q4.locator('[data-testid="verify-pill"]')).toHaveText("未通过");
  await expect(q4.locator('[data-testid="verify-summary"]')).toHaveText("通过 1 · 失败 1 · 阻塞 0");
});

test("exit 127 chains merge into one environment blockage with recovery actions", async ({ page }) => {
  const cargoFail = (id: number, command: string) =>
    runItem(id, {
      command,
      exitCode: 127,
      output: "/bin/sh: cargo: command not found",
      failureClass: "command_not_found",
      failureTool: "cargo",
    });
  await stubShell(
    page,
    liveCore({
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "新对话",
      at: 1_700_000_000,
      archived: false,
      turns: [
        turn("跑验证", [
          cargoFail(1, "cargo build"),
          cargoFail(2, "cargo test"),
          cargoFail(3, "cargo clippy"),
          answerItem(4, "改完了，但验证被环境挡住。"),
        ]),
      ],
    }),
  );
  await openLive(page);

  await expect(page.locator('[data-testid="final-outcome-pill"]')).toHaveText("环境阻塞");
  await expect(page.locator('[data-testid="failure-group-command_not_found"]')).toHaveCount(1);
  await expect(page.locator('[data-testid="failure-label"]')).toHaveText("3 个验证步骤因缺少 cargo 而阻塞");
  await expect(page.locator('[data-testid="verify-summary"]')).toHaveText("通过 0 · 失败 0 · 阻塞 3");
  await expect(page.locator(".failure-reason").first()).toHaveText("原因：找不到命令");
  await expect(page.locator(".failure-advice").first()).toHaveText("建议：安装缺失的工具后重试");

  // Raw commands stay preserved behind 查看失败命令.
  const toggle = page.locator('[data-testid="failure-toggle-commands"]');
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  const commandsId = await toggle.getAttribute("aria-controls");
  expect(commandsId).toBeTruthy();
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator(`#${commandsId}`)).toHaveText("cargo build\ncargo test\ncargo clippy");

  await expect(page.locator('[data-testid="retry-environment"]')).toHaveText("修复环境后重试");
  await expect(page.locator('[data-testid="ignore-verification"]')).toHaveText("忽略验证并完成回答");
  await page.locator('[data-testid="ignore-verification"]').click();
  await expect(page.locator('[data-testid="verification-ignored"]')).toHaveText(
    "已忽略验证结果，回答按现状使用。",
  );

});

test("counters are honest: 统计中… / empty / 等待步骤完成, never a bare 0", async ({ page }) => {
  await stubShell(page, liveCore({ id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false, turns: [] }));
  await openLive(page);
  await page.locator('[data-testid="composer-input"]').fill("统计 tokens");
  await page.locator('[data-testid="composer-input"]').press("Enter");
  await expect(page.locator(".reply-working")).toBeVisible();

  // Running with nothing finished yet: the summary never claims zeros.
  const summary = page.locator(".ins-content");
  await expect(summary).toContainText("统计中…");
  await expect(summary).toContainText("等待步骤完成");
  await expect(summary).toContainText("尚未产生文件修改");
  await expect(summary).not.toContainText("0 tokens");
  await expect(summary).not.toContainText("0 个步骤");

  // A model call in flight counts tokens as 统计中…, not 0.
  await emit(page, {
    type: "itemStarted",
    session: "s1",
    item: { id: 1, at: 1_700_000_000, status: "running", duration: null, kind: "modelCall", model: "deepseek-chat", inputTokens: 0, outputTokens: 0 },
  });
  await expect(page.getByTestId("process-tools")).not.toHaveAttribute("open", "");
  await page.locator('[data-testid="process-tools"] > summary').click();
  await expect(page.locator(".trace-tokens").first()).toContainText("统计中…");
  await expect(page.locator(".trace-duration").first()).toHaveText("进行中");
  // A running row opens on its own; the run status is announced politely.
  await expect(page.locator(".trace-row-inner").first()).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator('[data-testid="agent-progress"]')).toHaveAttribute("aria-live", "polite");

  // A finished call with nothing counted reads —, never 0 tokens.
  await emit(page, {
    type: "itemCompleted",
    session: "s1",
    item: { id: 1, at: 1_700_000_000, status: "done", duration: 0, kind: "modelCall", model: "deepseek-chat", inputTokens: 0, outputTokens: 0 },
  });
  await expect(page.locator(".trace-tokens").first()).toContainText("输入 — tokens");
  await expect(page.locator(".reply")).not.toContainText("0 tokens");
  // An unmeasurable step leaves the column empty — never a bare `—` placeholder.
  await expect(page.locator(".trace-duration").first()).toHaveText("");
  await emit(page, { type: "turnComplete", session: "s1" });
  await expect(summary).not.toContainText("统计中…");
});

test("collapse rules: failed and running rows open, completed rows stay closed", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "新对话",
      at: 1_700_000_000,
      archived: false,
      turns: [
        // Process-only turn (no final reply): the trace stays on screen after
        // the run — once a final reply lands the process is hidden by design.
        turn("检查", [
          runItem(1, { command: "cargo build", exitCode: 0, output: "Finished", status: "done" }),
          runItem(2, { command: "cargo test", exitCode: 1, output: "assertion failed", status: "failed", failureClass: "non_zero_exit" }),
        ]),
      ],
    }),
  );
  await openLive(page);

  // The two consecutive runs fold under one group row, and the group opens
  // itself because it holds the failure.
  await expect(page.locator(".trace-row--group > .trace-row-inner")).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  const rows = page.locator("[data-testid='trace-group-children'] > .trace-row > .trace-row-inner");
  await expect(rows.nth(0)).toHaveAttribute("aria-expanded", "false");
  await expect(rows.nth(1)).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator('[data-testid="trace-run-output"]')).toHaveCount(1);

  // Completed output stays hidden until asked for — then one click reveals it.
  await rows.nth(0).click();
  await expect(rows.nth(0)).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator('[data-testid="trace-run-output"]')).toHaveCount(2);

  // The failed row states its cause in words, not color alone.
  await expect(page.locator('[data-testid="trace-fail-reason"]')).toContainText("原因：命令返回非零退出码");
  await expect(page.locator('[data-testid="trace-exit-code"]')).toContainText("exit 1");
});

test("phases group the trace; internal scheduling stays out of the UI", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "新对话",
      at: 1_700_000_000,
      archived: false,
      turns: [
        turn("做修改", [
          {
            id: 1,
            at: 1_700_000_000,
            status: "done",
            duration: 247,
            kind: "reasoning",
            summary: "收集上下文",
            phase: "prepare",
            diagnostics: "rounds 2/6 · tools 4/32 · repairs 0/3 · repair_wall 0/120ms",
          },
          { id: 2, at: 1_700_000_000, status: "done", duration: 0, kind: "search", query: "router", detail: "查找路由实现" },
          { id: 3, at: 1_700_000_000, status: "done", duration: 50, kind: "reasoning", summary: "开始修改", phase: "execute" },
          runItem(4, { command: "cargo test", duration: 42_000 }),
        ]),
      ],
    }),
  );
  await openLive(page);

  await expect(page.locator('[data-testid="trace-phase-prepare"] .trace-phase-label')).toHaveText("准备项目上下文");
  await expect(page.locator('[data-testid="trace-phase-prepare"] .trace-phase-meta')).toContainText("2 个步骤");
  await expect(page.locator('[data-testid="trace-phase-prepare"] .trace-phase-meta')).toContainText("247ms");
  await expect(page.locator('[data-testid="trace-phase-execute"] .trace-phase-label')).toHaveText("执行修改");

  // No rounds/budget counters and no raw diagnostics anywhere in the reply.
  const replyText = (await page.locator(".reply").innerText()) ?? "";
  expect(replyText).not.toMatch(/rounds|repairs|repair_wall|tools \d/);
});

test("consecutive thinking steps fold into one row with a sub-step count", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "新对话",
      at: 1_700_000_000,
      archived: false,
      turns: [
        turn("想一想", [
          { id: 1, at: 1_700_000_000, status: "done", duration: 10, kind: "reasoning", summary: "一", phase: "prepare" },
          { id: 2, at: 1_700_000_000, status: "done", duration: 10, kind: "reasoning", summary: "二" },
          { id: 3, at: 1_700_000_000, status: "done", duration: 10, kind: "reasoning", summary: "三" },
        ]),
      ],
    }),
  );
  await openLive(page);
  await page.locator('[data-testid="process-tools"] > summary').click();

  await expect(page.locator(".trace-row")).toHaveCount(1);
  await expect(page.locator(".trace-row .code-chip").first()).toHaveText("3 个子步骤");
  await page.locator(".trace-row-inner").click();
  await expect(page.locator('[data-testid="trace-thinking-list"] .trace-thinking-item')).toHaveCount(3);
});

test("Inspector nav: panel tabs and rail never share a name", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");
  await expect(page.locator(".ins-tab")).toHaveCount(0);
  await page.getByTestId("rail-summary").click();

  // 文件 and 浏览 are panel chips now, so the rail section 文件 used to label
  // is 改动 — a panel and a rail section never share a name.
  await expect(page.locator(".ins-tab")).toHaveText(["Inspector", "终端", "文件", "浏览", "Memory"]);
  await expect(page.locator(".rail-btn")).toHaveCount(4);
  await expect(page.locator('[data-testid="rail-summary"]')).toHaveAttribute("aria-label", "概览");
  await expect(page.locator('[data-testid="rail-files"]')).toHaveAttribute("aria-label", "改动");
  await expect(page.locator('[data-testid="rail-tools"]')).toHaveAttribute("aria-label", "工具");
  await expect(page.locator('[data-testid="rail-llm"]')).toHaveAttribute("aria-label", "模型");
  await expect(page.locator('.rail-btn[aria-label="终端"]')).toHaveCount(0);
  await expect(page.locator('.rail-btn[aria-label="Inspector"]')).toHaveCount(0);
  await expect(page.locator('.rail-btn[aria-label="文件"]')).toHaveCount(0);
  await expect(page.locator('.rail-btn[aria-label="浏览"]')).toHaveCount(0);

  // 终端 is a whole panel of its own; the rail steps aside while it is open.
  await page.locator(".ins-tab", { hasText: "终端" }).click();
  await expect(page.locator(".ins-section-title")).toHaveText(["命令输出"]);
  await expect(page.locator(".ins-rail")).toHaveCount(0);

  await page.locator(".ins-tab", { hasText: "Inspector" }).click();
  await expect(page.locator(".ins-rail")).toBeVisible();
  await expect(page.locator(".ins-section-title").first()).toHaveText("本轮");

  // The rail's section buttons swap the content under the same panel tab.
  await page.locator('.rail-btn[aria-label="改动"]').click();
  await expect(page.locator(".ins-section-title")).toHaveText(["修改文件"]);
  await expect(page.locator('[data-testid="rail-files"]')).toHaveAttribute("aria-expanded", "true");
});

test("expanders are real buttons with aria-expanded/aria-controls wiring", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "新对话",
      at: 1_700_000_000,
      archived: false,
      turns: [
        turn("看输出", [
          runItem(1, { command: "cargo build", exitCode: 127, output: "sh: cargo: command not found", failureClass: "command_not_found", failureTool: "cargo", status: "failed" }),
        ]),
      ],
    }),
  );
  await openLive(page);

  const row = page.locator(".trace-row-inner").first();
  await expect(row).toHaveAttribute("aria-expanded", "true");
  const panelId = await row.getAttribute("aria-controls");
  expect(panelId).toBeTruthy();
  expect(await row.evaluate((el) => el.tagName)).toBe("BUTTON");
  await expect(page.locator(`#${panelId}`)).toBeVisible();

  // Enter collapses; the panel unmounts so the keyboard cannot land inside it.
  await row.focus();
  await page.keyboard.press("Enter");
  await expect(row).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(`#${panelId}`)).toHaveCount(0);

  const phase = page.locator(".trace-phase-head").first();
  await expect(phase).toHaveAttribute("aria-expanded", "true");
  await phase.click();
  await expect(phase).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".trace-row")).toHaveCount(0);
});

test("errors announce with role=alert; long answers remain fully readable", async ({ page }) => {
  const p1 = "第一段。".repeat(40);
  const p2 = "第二段。".repeat(40);
  const p3 = "第三段收尾标记。".repeat(30);
  await stubShell(
    page,
    liveCore({
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "新对话",
      at: 1_700_000_000,
      archived: false,
      turns: [
        turn("长回答", [answerItem(1, `${p1}\n\n${p2}\n\n${p3}`)]),
        turn("出错了", [], { done: false, error: "provider unavailable" }),
      ],
    }),
  );
  await openLive(page);

  await expect(page.locator('[data-testid="reply-error"]')).toHaveAttribute("role", "alert");
  await expect(page.locator('[data-testid="reply-error"]')).toContainText("provider unavailable");

  await expect(page.locator('[data-testid="answer-expand"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="answer-body"]')).toContainText("收尾标记");
});

test("changed files merge by path with edit counts and fold past three", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "新对话",
      at: 1_700_000_000,
      archived: false,
      turns: [
        turn("改文件", [
          { id: 1, at: 1_700_000_000, status: "done", duration: 5, kind: "fileChange", changes: [{ path: "src/a.ts", added: 1, removed: 1 }] },
          {
            id: 2,
            at: 1_700_000_000,
            status: "done",
            duration: 5,
            kind: "fileChange",
            changes: [
              { path: "src/a.ts", added: 2, removed: 0 },
              { path: "src/b.ts", added: 5, removed: 1 },
            ],
          },
          answerItem(3, "改好了。"),
        ]),
        turn("再改", [
          {
            id: 1,
            at: 1_700_000_000,
            status: "done",
            duration: 5,
            kind: "fileChange",
            changes: [
              { path: "src/c.ts", added: 1, removed: 0 },
              { path: "src/d.ts", added: 1, removed: 0 },
              { path: "src/e.ts", added: 1, removed: 0 },
              { path: "src/f.ts", added: 1, removed: 0 },
            ],
          },
          answerItem(2, "又改了。"),
        ]),
      ],
    }),
  );
  await openLive(page);

  // One path → one row, with how many times this turn touched it.
  const firstCard = page.locator('[data-testid="changed-kodo"]').first();
  await expect(firstCard.locator(".changed-file-row")).toHaveCount(2);
  await expect(firstCard.locator(".changed-file-edits")).toHaveText(["2 次编辑", "1 次编辑"]);
  await expect(firstCard).toContainText("src/a.ts");

  // Past three files the list folds behind 查看全部.
  const secondCard = page.locator('[data-testid="changed-kodo"]').nth(1);
  await expect(secondCard.locator(".changed-file-row")).toHaveCount(3);
  const showAll = secondCard.locator('[data-testid="changed-show-all"]');
  await expect(showAll).toHaveText("查看全部");
  await showAll.click();
  await expect(secondCard.locator(".changed-file-row")).toHaveCount(4);

  await expect(page.locator('[data-testid="changed-net"]').first()).toContainText(
    "+/− 为本轮补丁累计增删，非工作区净 diff",
  );
  await expect(page.locator('[data-testid="review-files"]').first()).toHaveText("查看修改");
});

test("undo conflicts never promise to overwrite the user's later edits", async ({ page }) => {
  await stubShell(
    page,
    liveCore(
      {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "新对话",
        at: 1_700_000_000,
        archived: false,
        turns: [
          turn("改文件", [
            {
              id: 1,
              at: 1_700_000_000,
              status: "done",
              duration: 5,
              kind: "fileChange",
              changes: [
                { path: "src/kodo.ts", added: 2, removed: 0 },
                { path: "src/user.ts", added: 1, removed: 0 },
                { path: "src/clash.ts", added: 1, removed: 0 },
              ],
            },
            answerItem(2, "改好了。"),
          ]),
        ],
      },
      {
        turnChanges: [
          { path: "src/kodo.ts", diff: "--- a/src/kodo.ts\n+++ b/src/kodo.ts\n+k\n", userPreexisting: false, conflict: false },
          { path: "src/user.ts", diff: "--- a/src/user.ts\n+++ b/src/user.ts\n+u\n", userPreexisting: true, conflict: false },
          { path: "src/clash.ts", diff: "--- a/src/clash.ts\n+++ b/src/clash.ts\n+c\n", userPreexisting: false, conflict: true },
        ],
      },
    ),
  );
  await openLive(page);

  await expect(page.locator('[data-testid="pre-existing-count"]')).toHaveText("1 个用户原有");
  await expect(page.locator('[data-testid="undo-conflict-count"]')).toHaveText("1 个冲突");
  await expect(page.locator('[data-testid="changed-pre-existing"]')).toContainText("src/user.ts");
  await expect(page.locator('[data-testid="changed-pre-existing"] .changed-group-head')).toHaveText("用户原有修改");
  await expect(page.locator('[data-testid="changed-conflicts"]')).toContainText("src/clash.ts");
  await expect(page.locator('[data-testid="undo-conflict-note"]')).toHaveText(
    "撤销失败：工作区与 Kodo 记录不一致，将不会覆盖您的后续编辑。",
  );
  await expect(page.locator('[data-testid="undo-turn"]')).toHaveAttribute(
    "title",
    "撤销本轮 Kodo 修改（不会覆盖您的后续编辑）",
  );
});

test("Inspector changed-files diffs render colored rows, not raw headers", async ({ page }) => {
  await stubShell(
    page,
    liveCore(
      {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "新对话",
        at: 1_700_000_000,
        archived: false,
        turns: [
          turn("改一下", [
            {
              id: 1,
              at: 1_700_000_000,
              status: "done",
              duration: 5,
              kind: "fileChange",
              changes: [{ path: "src/app.ts", added: 1, removed: 1 }],
            },
            answerItem(2, "改好了。"),
          ]),
        ],
      },
      {
        turnChanges: [
          {
            path: "src/app.ts",
            diff: "--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1 +1 @@\n-old\n+new\n",
          },
        ],
      },
    ),
  );
  await openLive(page);

  // Overview → 变更文件 section → expand this file's body.
  await page.locator('[data-testid="ins-review-files"]').click();
  await page.locator('[data-testid="diff-toggle-src/app.ts"]').click();
  const body = page.locator('[data-testid="diff-body-src/app.ts"]');
  await expect(body).toBeVisible();
  await expect(body.locator(".is-add")).toContainText("new");
  await expect(body.locator(".is-del")).toContainText("old");
  await expect(body).not.toContainText("+++ b/");
  await expect(body).not.toContainText("--- a/");
});

test("no streaming draft up top — the answer surface appears only when final", async ({ page }) => {
  await stubShell(page, liveCore({ id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false, turns: [] }));
  await openLive(page);
  await page.locator('[data-testid="composer-input"]').fill("写回答");
  await page.locator('[data-testid="composer-input"]').press("Enter");

  await emit(page, { type: "turnStarted", session: "s1" });
  await emit(page, { type: "textDelta", session: "s1", text: "正在整理" });
  await emit(page, { type: "textDelta", session: "s1", text: "答案草稿内容。" });

  // While the run is live there is no answer surface at all — the draft of the
  // final reply is never hoisted to the top of the turn.
  await expect(page.locator('[data-testid="draft-badge"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="stream-preview"]')).toHaveCount(0);
  await expect(page.locator(".reply .final")).toHaveCount(0);
  await expect(page.locator('[data-testid="answer-conclusion"]')).toHaveCount(0);

  await emit(page, { type: "itemCompleted", session: "s1", item: answerItem(1, "最终答案。") });
  await emit(page, { type: "turnComplete", session: "s1" });

  await expect(page.locator('[data-testid="answer-conclusion"]')).toBeVisible();
  await expect(page.locator('[data-testid="answer-body"]')).toContainText("最终答案。");
  // Exactly one answer surface, and the discarded draft text never surfaces.
  await expect(page.locator(".reply .final")).toHaveCount(1);
  await expect(page.locator(".reply")).not.toContainText("正在整理答案草稿内容。");
});

test("interim stream prose flushes as notes in normal order", async ({ page }) => {
  await stubShell(page, liveCore({ id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false, turns: [] }));
  await openLive(page);
  await page.locator('[data-testid="composer-input"]').fill("跑起来");
  await page.locator('[data-testid="composer-input"]').press("Enter");

  await emit(page, { type: "turnStarted", session: "s1" });
  await emit(page, { type: "textDelta", session: "s1", text: "先看项目结构。" });
  await emit(page, { type: "itemStarted", session: "s1", item: { ...runItem(1), status: "running", output: "" } });
  await emit(page, { type: "itemCompleted", session: "s1", item: runItem(1) });
  await emit(page, { type: "textDelta", session: "s1", text: "编译通过，开始修改。" });
  await emit(page, { type: "itemStarted", session: "s1", item: { ...runItem(2, { command: "cargo test" }), status: "running", output: "" } });
  await emit(page, { type: "itemCompleted", session: "s1", item: runItem(2, { command: "cargo test" }) });

  // Order is read while the run is live — the trace is on screen then.
  const notes = page.locator('[data-testid="trace-note"]');
  await expect(notes).toHaveCount(2);
  await expect(notes.nth(0)).toContainText("先看项目结构。");
  await expect(notes.nth(1)).toContainText("编译通过，开始修改。");

  // Arrival order: note, step, note, step — the prose keeps its place in the flow.
  const order = await page.locator(".conversation-process > *").evaluateAll((rows) =>
    rows.map((row) => ({
      note: row.classList.contains("conversation-note"),
      text: (row.textContent ?? "").replace(/\s+/g, " ").trim(),
    })),
  );
  expect(order.map((row) => row.note)).toEqual([true, false, true, false]);
  expect(order[1].text).toContain("cargo check");
  expect(order[3].text).toContain("cargo test");
  const ids = await page.locator(".conversation-process [id]").evaluateAll((nodes) => nodes.map((n) => n.id));
  expect(new Set(ids).size).toBe(ids.length);

  // The trailing stream is the draft of the answer — discarded, never a note.
  await emit(page, { type: "textDelta", session: "s1", text: "总结草稿" });
  await emit(page, { type: "itemCompleted", session: "s1", item: answerItem(3, "最终回答。") });
  await emit(page, { type: "turnComplete", session: "s1" });

  await expect(page.locator('[data-testid="answer-body"]')).toContainText("最终回答。");
  await expect(page.locator(".reply")).not.toContainText("总结草稿");
  // Public updates retain their place after the final answer.
  await expect(page.locator('[data-testid="trace-note"]')).toHaveCount(2);
});

test("persisted progress survives the final answer and reload, with keyboard-accessible tool details", async ({ page }, testInfo) => {
  const note = (id: number, summary: string) => ({ id, at: 1_700_000_000, duration: 1, status: "done", kind: "reasoning", phase: "commentary", summary, diagnostics: "PRIVATE_DIAGNOSTICS" });
  await stubShell(page, liveCore({
    id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false,
    turns: [turn("评审权限设置的执行过程", [
      note(1, "已找到**权限入口**，接下来检查调用方。"),
      runItem(2),
      note(3, "<think>PRIVATE_REASONING</think>调用方已核对，开始整理评审结果。"),
      answerItem(4, "评审结果已整理。\n\n尚未覆盖的边界也已列出。"),
    ])],
  }));
  await openLive(page);
  const notes = page.getByTestId("trace-note");
  await expect(notes).toHaveCount(2);
  await expect(notes.first().locator("strong")).toHaveText("权限入口");
  await expect(notes.last()).toHaveText("调用方已核对，开始整理评审结果。");
  await expect(page.locator(".ins-content")).toContainText("1 个步骤");
  await expect(page.locator(".ins-section", { hasText: "模型调用" })).toContainText("尚未调用模型");
  await expect(page.locator(".reply")).not.toContainText("PRIVATE_");
  expect(await page.locator(".conversation-process > *").evaluateAll((nodes) => nodes.map((n) => n.tagName))).toEqual(["DIV", "DETAILS", "DIV"]);
  const tools = page.getByTestId("process-tools");
  await expect(tools).not.toHaveAttribute("open", "");
  await expect(page.locator(".trace-row-inner")).not.toBeVisible();
  await tools.locator("summary").focus();
  await page.keyboard.press("Enter");
  await expect(tools).toHaveAttribute("open", "");
  await page.locator(".trace-row-inner").click();
  await expect(page.getByTestId("trace-run-output")).toHaveText("ok");
  await tools.locator("summary").click();
  for (const width of [1440, 900]) {
    await page.setViewportSize({ width, height: 900 });
    const layout = await page.locator(".conversation-process").evaluate((el) => ({ width: el.clientWidth, scroll: el.scrollWidth }));
    expect(layout.scroll).toBeLessThanOrEqual(layout.width + 1);
    await page.screenshot({ path: testInfo.outputPath(`conversation-progress-${width}.png`) });
  }
  await page.reload();
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(notes).toHaveCount(2);
  await expect(notes.first()).toBeVisible();
  await expect(page.getByTestId("answer-body")).toContainText("尚未覆盖的边界");
});

test("committed progress replaces its draft once, and internal progress counters remain out of the status line", async ({ page }) => {
  await stubShell(page, liveCore({ id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false, turns: [] }));
  await openLive(page);
  await page.getByTestId("composer-input").fill("检查调用方");
  await page.getByTestId("composer-input").press("Enter");
  await emit(page, { type: "turnStarted", session: "s1" });
  await emit(page, { type: "progress", session: "s1", phase: "analyze", detail: "6 context spans · repo map ready" });
  await expect(page.getByTestId("agent-progress")).toContainText("分析任务");
  await expect(page.getByTestId("agent-progress")).not.toContainText("context spans");
  await emit(page, { type: "progress", session: "s1", phase: "internal-wrap-up", detail: "rounds 2/6" });
  await expect(page.getByTestId("agent-progress")).toContainText("正在处理您的请求");
  await expect(page.getByTestId("agent-progress")).not.toContainText("internal-wrap-up");
  await emit(page, { type: "textDelta", session: "s1", text: "已找到调用方。" });
  await emit(page, { type: "itemCompleted", session: "s1", item: { id: 9, at: 1_700_000_000, duration: 0, status: "done", kind: "reasoning", phase: "budget", summary: "context 20%" } });
  const note = { id: 1, at: 1_700_000_000, duration: 1, status: "done", kind: "reasoning", phase: "commentary", summary: "已找到调用方。" };
  await emit(page, { type: "itemStarted", session: "s1", item: { ...note, status: "running" } });
  await emit(page, { type: "itemCompleted", session: "s1", item: note });
  await emit(page, { type: "itemCompleted", session: "s1", item: runItem(2) });
  await emit(page, { type: "textDelta", session: "s1", text: "最终答复的草稿" });
  await emit(page, { type: "itemCompleted", session: "s1", item: { ...note, id: 10, phase: "analyze", summary: "输出被截断，正在恢复" } });
  await expect(page.getByTestId("trace-note")).toHaveCount(1);
  const model = { id: 11, at: 1_700_000_000, duration: 1, status: "running", kind: "modelCall", model: "fake", inputTokens: 0, outputTokens: 0 };
  await emit(page, { type: "itemStarted", session: "s1", item: model });
  await emit(page, { type: "itemCompleted", session: "s1", item: { ...model, status: "done" } });
  await emit(page, { type: "itemCompleted", session: "s1", item: runItem(12) });
  await emit(page, { type: "itemCompleted", session: "s1", item: answerItem(13, "检查已完成。") });
  await emit(page, { type: "turnComplete", session: "s1" });
  await expect(page.getByTestId("trace-note")).toHaveText("已找到调用方。");
  await expect(page.locator(".reply")).not.toContainText("最终答复的草稿");
});

test("command output renders without ANSI escape garbage", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "新对话",
      at: 1_700_000_000,
      archived: false,
      turns: [
        // Process-only turn (no final reply) keeps the run row on screen.
        turn("检查", [
          runItem(1, { output: `${String.fromCharCode(27)}[1m${String.fromCharCode(27)}[92mok${String.fromCharCode(27)}[0m done` }),
        ]),
      ],
    }),
  );
  await openLive(page);
  await page.locator('[data-testid="process-tools"] > summary').click();
  await page.locator(".trace-row-inner").click();
  const output = page.locator('[data-testid="trace-run-output"]');
  await expect(output).toHaveText("ok done");
});

test("a new conversation's first answer lands live — even when the run finishes before its id exists", async ({
  page,
}) => {
  // autoRun completes the whole turn inside send_message, before the create
  // resolves — the exact race that used to strand the first turn at 执行中.
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [] },
    branch: "main",
    autoRun: { answer: "做好了。", plain: false },
  });
  await page.goto("/");
  await page.locator('button[aria-label="alpha 的操作"]').click();
  await page.getByRole("menuitem", { name: "新建任务" }).click();
  await expect(page.locator(".topbar-crumbs")).toContainText("alpha");

  await page.locator('[data-testid="composer-input"]').fill("你是谁");
  await page.locator('[data-testid="composer-input"]').press("Enter");
  await expect(page.locator(".msg-bubble")).toHaveText("你是谁");

  // The answer displays without switching conversations away and back.
  await expect(page.locator('[data-testid="answer-body"]')).toContainText("做好了。");
  // And the turn is finished — never stuck at 执行中 with a stop button.
  await expect(page.locator('[data-testid="agent-progress"]')).toHaveCount(0);
  // The Codex-style context ring lives in the composer and must clear once
  // the turn ends too — otherwise an idle reply carries stale tokens.
  await expect(page.locator('[data-testid="composer-context-ring"]')).toHaveCount(0);
  await expect(page.locator(".composer-send--stop")).toHaveCount(0);
  await expect(page.locator('[data-testid="composer-send"]')).toBeVisible();
  // The first ask also titles the session.
  await expect(page.locator(".crumb-current")).toHaveText("你是谁");
  const sent = await page.evaluate(() => (window as unknown as { __sendLog: Array<{ project?: string }> }).__sendLog);
  expect(sent.at(-1)?.project).toBe("/tmp/ws/alpha");
});

test("a conversational answer renders as the sentence alone, without report chrome", async ({ page }) => {
  await stubShell(
    page,
    liveCore({ id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false, turns: [] }),
  );
  await openLive(page);
  await page.locator('[data-testid="composer-input"]').fill("你是谁");
  await page.locator('[data-testid="composer-input"]').press("Enter");

  await emit(page, { type: "turnStarted", session: "s1" });
  await emit(page, {
    type: "itemCompleted",
    session: "s1",
    item: answerItem(1, "我是 Kodo，一个帮助你阅读、修改和验证当前项目代码的编程助手。", {
      delivery: "ready",
      verification: "not_run",
      plain: true,
    }),
  });
  await emit(page, { type: "turnComplete", session: "s1" });

  await expect(page.locator('[data-testid="answer-plain"]')).toContainText(
    "我是 Kodo，一个帮助你阅读、修改和验证当前项目代码的编程助手。",
  );
  // No conclusion card, no axes line, no done-this-turn, no next steps.
  await expect(page.locator('[data-testid="answer-conclusion"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="answer-axes"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="final-outcome-pill"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="answer-body"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="answer-next"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="next-review"]')).toHaveCount(0);
  // Same invariant as the card path: exactly one answer surface.
  await expect(page.locator(".reply .final")).toHaveCount(1);
});

for (const vp of [
  { width: 900, height: 800 },
  { width: 1100, height: 800 },
]) {
  test(`at ${vp.width}px the inspector overlays and focus returns to its trigger`, async ({ page }) => {
    await page.setViewportSize(vp);
    await page.goto("/ui-demo/conversation");
    await page.waitForLoadState("networkidle");

    // Narrow windows default the panel closed — nothing hidden is focusable.
    await expect(page.locator(".ins-panel")).toHaveCount(0);
    await expect(page.locator('[data-testid="inspector-close"]')).toHaveCount(0);

    const before = await page.locator(".main").evaluate((el) => el.getBoundingClientRect().width);
    const trigger = page.locator('[data-testid="rail-summary"]');
    await trigger.click();
    await expect(page.locator(".ins-panel")).toBeVisible();
    const after = await page.locator(".main").evaluate((el) => el.getBoundingClientRect().width);
    expect(Math.abs(after - before)).toBeLessThanOrEqual(2);

    // Opening parks focus on the close control inside the panel.
    await expect(page.locator('[data-testid="inspector-close"]')).toBeFocused();
    await page.locator('[data-testid="inspector-close"]').click();
    await expect(page.locator(".ins-panel")).toHaveCount(0);
    await expect(trigger).toBeFocused();
  });
}

test("zh-CN copy: step labels and actions come from the central map", async ({ page }) => {
  // Step labels now live where the process lives after the final reply: the
  // Response Trace page (type pills) and the inline trace of a live run
  // (phase heads). Answer chrome stays on the completed conversation.
  await page.goto("/ui-demo/trace");
  await page.waitForLoadState("networkidle");

  await expect(page.locator(".type-pill span")).toHaveText([
    "思考",
    "搜索代码",
    "读取文件",
    "运行命令",
    "命令输出",
    "调用模型",
    "修改文件",
    "运行命令",
    "调用模型",
    "整理回答",
  ]);
  const pillLabels = await page.locator(".type-pill span").allTextContents();
  expect(pillLabels.join("|")).not.toMatch(/Thinking|Search codebase|Read file|Run command|Call model|Edit files/);

  // Phase heads come from the same map on a live run (a run with no final
  // reply keeps its trace on screen).
  await stubShell(
    page,
    liveCore({
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "新对话",
      at: 1_700_000_000,
      archived: false,
      turns: [
        turn("想一想", [
          { id: 1, at: 1_700_000_000, status: "done", duration: 10, kind: "reasoning", summary: "一", phase: "prepare" },
          { id: 2, at: 1_700_000_000, status: "done", duration: 10, kind: "reasoning", summary: "二", phase: "analyze" },
          { id: 3, at: 1_700_000_000, status: "done", duration: 10, kind: "reasoning", summary: "三", phase: "execute" },
          { id: 4, at: 1_700_000_000, status: "done", duration: 10, kind: "reasoning", summary: "四", phase: "summarize" },
        ]),
      ],
    }),
  );
  await openLive(page);
  await expect(page.locator(".trace-phase-label")).toHaveText([
    "准备项目上下文",
    "分析任务",
    "执行修改",
    "整理回答",
  ]);

  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  // The fixture holds three turns; scope every per-reply assertion to turn 1.
  const first = page.locator(".reply").first();
  await expect(first.locator('[data-testid="review-files"]')).toHaveText("查看修改");
  await expect(first.locator('[data-testid="next-review"]')).toHaveCount(0);
  // The reply actions are icon-only — the label moves to the tooltip (2026-09-30).
  await expect(first.locator('[data-testid="reply-trace"]')).toHaveAttribute(
    "title",
    "查看响应轨迹",
  );
  await expect(first.locator('[data-testid="reply-copy"]')).toHaveAttribute("title", "复制回复");
  await expect(first.locator('[data-testid="verify-pill"]')).toHaveText("已通过");
  await expect(first.locator('[data-testid="final-outcome-pill"]')).toHaveText("已完成");
  await expect(first.locator('[data-testid="answer-axes"]')).toHaveText(
    "回答：已生成 · 验证：已通过",
  );
  await expect(page.locator('[data-testid="topbar-files"]')).toHaveAttribute("aria-label", "查看项目文件");
});


test("a budget-limited plain answer stays failed and long Inspector cards never overlap", async ({ page }, testInfo) => {
  const items = [
    ...Array.from({ length: 25 }, (_, i) => ({
      id: i + 1, at: 1_700_000_000, status: "done", duration: 1,
      kind: "fileRead", path: `src/module-${i}.rs`, detail: "module source",
    })),
    ...Array.from({ length: 12 }, (_, i) => ({
      id: i + 26, at: 1_700_000_000, status: "done", duration: 1200,
      kind: "modelCall", model: "Demo Flash", inputTokens: 1000 + i * 100, outputTokens: 100,
    })),
    answerItem(38, "任务未完成（本轮执行预算耗尽：轮次 12/12）。\n\n尚未生成完整答复。完整记录可在「查看响应轨迹」中查看。", {
      plain: true, delivery: "failed", verification: "not_run",
    }),
  ];
  await stubShell(page, liveCore({
    id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false,
    turns: [turn("为我输出该项目的总体架构设计", items)],
  }));
  await openLive(page);
  await expect(page.getByTestId("final-outcome-pill")).toHaveText("失败");
  await expect(page.getByTestId("ins-status-pill")).toHaveText("失败");
  await expect(page.locator(".ins-content")).toContainText("未生成");
  await expect(page.getByTestId("ins-undo")).toHaveCount(0);
  await expect(page.locator(".reply .final")).toHaveCount(1);
  await expect(page.getByTestId("process-tools")).not.toHaveAttribute("open", "");

  const buttons = await page.locator(".reply-actions button").evaluateAll((nodes) => nodes.map((el) => el.getBoundingClientRect().top));
  expect(buttons).toHaveLength(4);
  await expect(page.getByTestId("reply-promote-answer")).toBeVisible();
  expect(Math.max(...buttons) - Math.min(...buttons)).toBeLessThanOrEqual(1);
  for (const width of [1920, 1280, 900]) {
    await page.setViewportSize({ width, height: 700 });
    if (await page.locator(".ins-panel").count() === 0) await page.getByTestId("rail-summary").click();
    const layout = await page.locator(".ins-content").evaluate((el) => ({
      overflowing: el.scrollHeight > el.clientHeight,
      cards: [...el.querySelectorAll(".ins-section")].map((card) => ({
        top: card.getBoundingClientRect().top,
        bottom: card.getBoundingClientRect().bottom,
        contentBottom: Math.max(...[...card.querySelectorAll(".ins-body")].map((body) => body.getBoundingClientRect().bottom)),
      })),
    }));
    expect(layout.cards).toHaveLength(5);
    expect(layout.overflowing).toBe(true);
    for (const [i, card] of layout.cards.entries()) {
      expect(card.contentBottom).toBeLessThanOrEqual(card.bottom);
      if (i) expect(card.top).toBeGreaterThanOrEqual(layout.cards[i - 1].bottom);
    }
  }
  await page.setViewportSize({ width: 1586, height: 992 });
  await page.screenshot({ path: testInfo.outputPath("qa-budget.png") });
  await page.getByTestId("reply-regenerate").click();
  await expect(page.locator(".composer-send--stop")).toBeVisible();
  const sent = await page.evaluate(() => (window as unknown as { __sendLog: Array<{ text: string }> }).__sendLog);
  expect(sent.at(-1)?.text).toBe("为我输出该项目的总体架构设计");
});

test("a document follow-up waiting for confirmation shows a result without claiming completion", async ({ page }, testInfo) => {
  const question = "需要确认后继续：AGENTS.md 要求确认现有改动归属，请确认是否继续当前文档任务。\n\n请在输入框回复；本轮已暂停等待确认后的操作。";
  await stubShell(page, liveCore({
    id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false,
    turns: [turn("输出到 docs/总体架构设计.md", [
      { id: 1, at: 1_700_000_000, status: "done", duration: 1, kind: "reasoning", summary: "已询问用户", phase: "ask", diagnostics: "ask_user · AGENTS.md 要求确认现有改动归属" },
      answerItem(2, question, { delivery: "blocked", verification: "blocked", plain: false }),
    ])],
  }));
  await openLive(page);
  await expect(page.getByTestId("final-outcome-pill")).toHaveText("待确认");
  await expect(page.getByTestId("answer-body").locator(".answer-section-head")).toHaveText("本轮结果");
  await expect(page.getByTestId("answer-body")).toContainText("请在输入框回复");
  await expect(page.getByTestId("ins-undo")).toHaveCount(0);
  await expect(page.getByTestId("next-retry-env")).toHaveCount(0);
  await expect(page.getByTestId("reply-regenerate")).toHaveCount(0);
  await expect(page.getByTestId("ins-status-pill")).toContainText("待确认");
  await expect(page.getByTestId("ins-regenerate")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("document-confirmation.png") });
});

test("a budget failure offers task recovery without false environment or verification actions", async ({ page }, testInfo) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const ask = "请分析该项目的总体架构设计并输出到docs/总体架构设计.md";
  await stubShell(page, liveCore({
    id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false,
    turns: [turn(ask, [answerItem(1, "任务未完成。本批工具调用超过剩余额度：申请 10 次，剩余 9 次；本批未执行。", {
      delivery: "failed", verification: "not_run", failureReason: "budget_exhausted", plain: false,
      checks: ["已明确标记为失败，未伪装成功"],
    })])],
  }, {
    providers: [{
      id: "p1", template: "openai", name: "OpenAI", apiKey: "••••1234", endpoint: "",
      model: "gpt-4o", modelId: "gpt-4o", displayName: "GPT-4o",
      models: [{ modelId: "gpt-4o", displayName: "GPT-4o" }],
    }],
    settings: { "active-provider": "0" },
  }));
  await openLive(page);
  await expect(page.getByTestId("final-outcome-pill")).toHaveText("失败");
  await expect(page.getByTestId("answer-axes")).toHaveText("回答：未生成 · 验证：未验证");
  await expect(page.getByTestId("verify-card")).toHaveCount(0);
  await expect(page.getByTestId("next-review")).toHaveCount(0);
  await expect(page.getByTestId("next-retry-env")).toHaveCount(0);
  const recover = page.getByTestId("next-continue");
  await expect(recover).toHaveText("继续任务");
  await page.screenshot({ path: testInfo.outputPath("budget-recovery.png") });
  await recover.click();
  await expect(page.locator(".composer-send--stop")).toBeVisible();
  const sent = await page.evaluate(() => (window as unknown as { __sendLog: Array<{ text: string; budgetFamily: string }> }).__sendLog);
  expect(sent.at(-1)).toMatchObject({ text: ask, budgetFamily: "extended" });
  expect(errors).toEqual([]);
});

test("a budget failure retains real changed files and failed verification evidence", async ({ page }) => {
  await stubShell(page, liveCore({
    id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false,
    turns: [turn("完成文档", [
      { id: 1, at: 1_700_000_000, status: "done", duration: 1, kind: "fileChange", changes: [{ path: "docs/a.md", added: 3, removed: 0 }] },
      runItem(2, { command: "verify-doc", output: "mismatch", exitCode: 1, failureClass: "test_failure" }),
      answerItem(3, "已保存草稿，回读发现仍需修正，预算达到边界。", { delivery: "partial", verification: "failed", failureReason: "budget_exhausted", plain: false }),
    ])],
  }));
  await openLive(page);
  await expect(page.getByTestId("review-files")).toBeVisible();
  await expect(page.getByTestId("next-review")).toHaveCount(0);
  await expect(page.getByTestId("verify-card")).toBeVisible();
  await expect(page.getByTestId("verify-pill")).toHaveText("未通过");
  await expect(page.getByTestId("next-continue")).toBeVisible();
  await expect(page.getByTestId("next-retry-env")).toHaveCount(0);
});

test("a long project tree keeps one row per project while scrolling", async ({ page }) => {
  const paths = Array.from({ length: 16 }, (_, i) => `/tmp/ws/project-${i}`);
  await stubShell(page, {
    workspace: {
      projects: paths.map((path, i) => project(path, `project-${i}`)),
      sessions: paths.map((path, i) => sessionRef(`s${i}`, path, `会话-${i}`)),
    },
    session: { id: "s0", project: paths[0], title: "会话-0", at: 1_700_000_000, archived: false, turns: [] },
  });
  await page.setViewportSize({ width: 1280, height: 700 });
  await page.goto("/");
  for (const row of await page.locator(".tree-project-main").all()) await row.click();
  await expect(page.locator(".tree-project-main")).toHaveCount(16);
  await expect(page.locator(".tree-conversation")).toHaveCount(16);
  const layout = await page.locator(".tree").evaluate((el) => {
    el.scrollTop = el.scrollHeight;
    return { overflowing: el.scrollHeight > el.clientHeight, groups: [...el.children].map((child) => {
      const box = child.getBoundingClientRect();
      return { top: box.top, bottom: box.bottom };
    }) };
  });
  expect(layout.overflowing).toBe(true);
  for (let i = 1; i < layout.groups.length; i++) expect(layout.groups[i].top).toBeGreaterThanOrEqual(layout.groups[i - 1].bottom);
  await expect(page.locator(".tree-conversation-title").last()).toHaveText("会话-15");
});

test("a delivered document has one compact review card and no invented verification", async ({ page }, testInfo) => {
  const path = "docs/总体架构设计.md";
  const text = `已输出至 \`${path}\`。\n\n文档涵盖模块分层、任务流程、权限与数据存储、测试体系及后续演进方向。\n\n- 已确认目标文档位于项目 \`docs\` 目录内\n- 已重读文档\n- 本次仅写入该文档`;
  await stubShell(page, liveCore({
    id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false,
    turns: [turn("分析项目并输出架构文档", [
      { id: 1, at: 1_700_000_000, status: "done", duration: 1, kind: "fileChange", fileSpanRef: "span1", changes: [{ path, added: 415, removed: 0 }] },
      { id: 2, at: 1_700_000_000, status: "done", duration: 1, kind: "fileRead", path, detail: "# 总体架构设计\n## 模块分层" },
      answerItem(3, text, { delivery: "ready", verification: "not_run", plain: false, checks: ["已确认目标文档位于项目 `docs` 目录内", "已重读文档", "本次仅写入该文档"] }),
    ])],
  }, { files: [path], fileBodies: { [path]: "# 总体架构设计\n\n模块分层" }, turnChanges: [{ path, diff: "@@ -0,0 +1,2 @@\n+# 总体架构设计\n+模块分层" }] }));
  await openLive(page);
  await expect(page.getByTestId("final-outcome-pill")).toHaveText("已完成");
  await expect(page.getByTestId("ins-status-pill")).toHaveText("已完成");
  await expect(page.getByTestId("answer-axes")).toHaveText("文档：已保存 · 验证：未运行命令验证");
  await expect(page.getByTestId("answer-body").locator(".answer-section-head")).toHaveText("交付结果");
  await expect(page.getByTestId(`document-state-${path}`)).toHaveText("回读已记录");
  await expect(page.getByTestId("verify-card")).toHaveCount(0);
  await expect(page.getByTestId("next-review")).toHaveCount(0);
  await expect(page.getByTestId("next-retry-env")).toHaveCount(0);
  await expect(page.getByTestId("answer-next")).toHaveCount(0);
  await expect(page.getByTestId("review-files")).toHaveCount(1);
  await expect(page.locator(".changed-summary .delta-del")).toHaveCount(0);
  await expect(page.getByTestId(`changed-open-${path}`)).toHaveCSS("text-align", "left");

  const details = page.getByTestId("changed-net");
  await expect(details).not.toHaveAttribute("open", "");
  await expect(details.locator("p").first()).not.toBeVisible();
  await details.locator("summary").focus();
  await page.keyboard.press("Enter");
  await expect(details.locator("p").last()).toBeVisible();
  await details.locator("summary").click();
  if (await page.getByTestId("inspector-close").isVisible()) await page.getByTestId("inspector-close").click();
  for (const theme of ["light", "dark"]) {
    await page.evaluate((value) => document.documentElement.setAttribute("data-theme", value), theme);
    await page.screenshot({ path: testInfo.outputPath(`document-result-${theme}.png`) });
  }
  await page.setViewportSize({ width: 640, height: 800 });
  const geometry = await page.getByTestId("changed-summary").evaluate((card) => {
    const box = card.getBoundingClientRect();
    return { left: box.left, right: box.right, width: window.innerWidth, overflow: card.scrollWidth - card.clientWidth };
  });
  expect(geometry.left).toBeGreaterThanOrEqual(0);
  expect(geometry.right).toBeLessThanOrEqual(geometry.width);
  expect(geometry.overflow).toBeLessThanOrEqual(1);
  await page.screenshot({ path: testInfo.outputPath("document-result-narrow.png") });
  await page.setViewportSize({ width: 1586, height: 992 });
  await page.getByTestId(`changed-open-${path}`).click();
  await expect(page.getByTestId("file-viewer")).toContainText("总体架构设计");
});

test("document readback badges follow the last write rather than the answer's claims", async ({ page }) => {
  const path = "docs/a.md";
  const change = (id: number) => ({ id, at: 1_700_000_000, status: "done", duration: 1, kind: "fileChange", changes: [{ path, added: 3, removed: 0 }] });
  const read = (id: number, status = "done") => ({ id, at: 1_700_000_000, status, duration: 1, kind: "fileRead", path, detail: "content" });
  const answer = answerItem(9, "已重读并确认文档完整。", { delivery: "ready", verification: "not_run", plain: false });
  await stubShell(page, liveCore({
    id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false,
    turns: [
      turn("已读取", [change(1), read(2), answer]),
      turn("读取早于写入", [read(1), change(2), answer]),
      turn("旧回读后再次写入", [change(1), read(2), change(3), answer]),
      turn("回读失败", [change(1), read(2, "failed"), answer]),
    ],
  }));
  await openLive(page);
  await expect(page.getByTestId(`document-state-${path}`)).toHaveText(["回读已记录", "已写入", "已写入", "已写入"]);
  await expect(page.getByTestId("verify-card")).toHaveCount(0);
});

test("explicit document status preserves failed checks and ignores exploratory command success", async ({ page }) => {
  const path = "docs/a.md";
  const change = { id: 2, at: 1_700_000_000, status: "done", duration: 1, kind: "fileChange", changes: [{ path, added: 3, removed: 0 }] };
  await stubShell(page, liveCore({
    id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false,
    turns: [
      turn("完成文档", [
        { id: 1, at: 1_700_000_000, status: "done", duration: 1, kind: "reasoning", phase: "execute", summary: "检查工作区" },
        runItem(2, { command: "git status --short" }),
        { ...change, id: 3 },
        answerItem(4, "已交付文档", { delivery: "ready", verification: "not_run", plain: false }),
        answerItem(5, "后续说明", { plain: false }),
      ]),
      turn("未交付", [change, answerItem(3, "已交付文档", { delivery: "failed", verification: "not_run", plain: false })]),
      turn("实际验证失败", [
        change, runItem(3, { command: "verify-doc", exitCode: 1, output: "bad link", failureClass: "test_failure" }),
        answerItem(4, "已交付文档。\n\n- 已检查 `docs/a.md`\n\n文档尚未通过验证。", {
          delivery: "ready", verification: "failed", plain: false,
          checks: ["已检查 `docs/a.md`", "需修复链接"],
        }),
      ]),
    ],
  }));
  await openLive(page);
  await expect(page.getByTestId("final-outcome-pill")).toHaveText(["已完成", "失败", "部分完成"]);
  const replies = page.locator(".reply");
  await expect(replies.nth(0).getByTestId("answer-axes")).toHaveText("文档：已保存 · 验证：未运行命令验证");
  await expect(replies.nth(0).getByTestId("verify-card")).toHaveCount(0);
  await expect(replies.nth(2).getByTestId("verify-pill")).toHaveText("未通过");
  await expect(replies.nth(2).getByTestId("verify-summary")).toHaveText("通过 0 · 失败 1 · 阻塞 0");
  await expect(replies.nth(2).getByTestId("checks-list").locator("li")).toHaveText(["需修复链接"]);
  await expect(page.getByTestId("next-retry-env")).toHaveCount(0);
});


test("stale run events and approvals cannot alter the current turn", async ({ page }) => {
  await stubShell(page, liveCore({ id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false, turns: [] }));
  await openLive(page);
  await page.locator('[data-testid="composer-input"]').fill("继续执行");
  await page.locator('[data-testid="composer-input"]').press("Enter");
  await emit(page, { type: "turnStarted", session: "s1", runId: 22 });
  await emit(page, { type: "approvalRequest", session: "s1", runId: 21, step: 1, kind: "run command", detail: "stale approval" });
  await expect(page.locator(".approval-bar")).toHaveCount(0);
  await emit(page, { type: "approvalRequest", session: "s1", runId: 22, step: 2, kind: "run command", detail: "current approval" });
  await expect(page.locator(".approval-bar")).toContainText("current approval");
  await page.locator('.approval-bar [data-testid="approval-deny"]').click();
  const reply = await page.evaluate(() => (window as unknown as { __approvalLog: Array<{ runId?: number; step: number }> }).__approvalLog.at(-1));
  expect(reply?.runId).toBe(22); expect(reply?.step).toBe(2);
  await emit(page, { type: "itemCompleted", session: "s1", runId: 21, item: answerItem(1, "stale answer") });
  await expect(page.getByText("stale answer", { exact: true })).toHaveCount(0);
  await emit(page, { type: "itemCompleted", session: "s1", runId: 22, item: answerItem(1, "当前运行的答案。") });
  await expect(page.locator('[data-testid="answer-body"]')).toContainText("当前运行的答案。");
});
