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

  await expect(page.locator(".trace-row")).toHaveCount(1);
  await expect(page.locator(".trace-row .code-chip").first()).toHaveText("3 个子步骤");
  await page.locator(".trace-row-inner").click();
  await expect(page.locator('[data-testid="trace-thinking-list"] .trace-thinking-item')).toHaveCount(3);
});

test("Inspector nav: panel tabs and rail never share a name", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForLoadState("networkidle");

  // 文件 and 浏览 are panel chips now, so the rail section 文件 used to label
  // is 改动 — a panel and a rail section never share a name.
  await expect(page.locator(".ins-tab")).toHaveText(["Inspector", "终端", "文件", "浏览"]);
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

test("errors announce with role=alert; long answers summarize with an expand", async ({ page }) => {
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

  const expand = page.locator('[data-testid="answer-expand"]');
  await expect(expand).toHaveText("展开完整回答");
  await expect(page.locator('[data-testid="answer-body"]')).not.toContainText("收尾标记");
  await expand.click();
  await expect(expand).toHaveText("收起回答");
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
  const order = await page.locator(".trace-groups .trace-row").evaluateAll((rows) =>
    rows.map((row) => ({
      note: row.classList.contains("trace-row--note"),
      text: (row.textContent ?? "").replace(/\s+/g, " ").trim(),
    })),
  );
  expect(order.map((row) => row.note)).toEqual([true, false, true, false]);
  expect(order[1].text).toContain("cargo check");
  expect(order[3].text).toContain("cargo test");

  // The trailing stream is the draft of the answer — discarded, never a note.
  await emit(page, { type: "textDelta", session: "s1", text: "总结草稿" });
  await emit(page, { type: "itemCompleted", session: "s1", item: answerItem(3, "最终回答。") });
  await emit(page, { type: "turnComplete", session: "s1" });

  await expect(page.locator('[data-testid="answer-body"]')).toContainText("最终回答。");
  await expect(page.locator(".reply")).not.toContainText("总结草稿");
  // Final reply on screen → the process has left the conversation.
  await expect(page.locator('[data-testid="trace-note"]')).toHaveCount(0);
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
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("alpha");

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
  await expect(first.locator('[data-testid="next-review"]')).toHaveText("查看修改");
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
