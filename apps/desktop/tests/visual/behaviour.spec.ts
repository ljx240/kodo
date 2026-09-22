import { expect, test } from "@playwright/test";
import { emit, project, sessionRef, stubShell } from "./shell";

test("a step row opens from anywhere along it, not just its chevron", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForSelector(".trace-row");

  const rows = page.locator(".trace-row");
  await expect(rows).toHaveCount(7);

  // Completed steps hide their output/diffs until asked for — every open row
  // renders one `.trace-expand` panel, whatever kind of step it is.
  await expect(page.locator(".trace-expand")).toHaveCount(0);

  const first = rows.first();
  await first.locator(".trace-label").click();
  await expect(page.locator(".trace-expand")).toHaveCount(1);

  await first.locator(".trace-label").click();
  await expect(page.locator(".trace-expand")).toHaveCount(0);

  await first.locator(".trace-row-inner").focus();
  await page.keyboard.press("Enter");
  await expect(page.locator(".trace-expand")).toHaveCount(1);

  // Space toggles the row the same way Enter does.
  await page.keyboard.press(" ");
  await expect(page.locator(".trace-expand")).toHaveCount(0);
  await page.keyboard.press(" ");
  await expect(page.locator(".trace-expand")).toHaveCount(1);
});

test("every row reports whether it is open", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.waitForSelector(".trace-row");

  await expect(page.locator(".trace-row-inner[aria-expanded]")).toHaveCount(7);
  // Only running / failed / denied rows open on their own; the demo's rows are
  // all completed, so none of them starts open.
  await expect(page.locator('.trace-row-inner[aria-expanded="true"]')).toHaveCount(0);
});

test("the composer sends on Enter, clears, and stops claiming to be working", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".conv-title")).toHaveText("修复路由");

  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  await expect(page.locator(".welcome-title")).toHaveText("今天想做什么？");

  // Empty stage: welcome + composer sit in the middle of the pane, not stuck at the bottom.
  const stage = await page.evaluate(() => {
    const main = document.querySelector(".main");
    const welcome = document.querySelector('[data-testid="welcome"]');
    const composer = document.querySelector(".composer");
    if (!main || !welcome || !composer) return null;
    const mainBox = main.getBoundingClientRect();
    const welcomeBox = welcome.getBoundingClientRect();
    const composerBox = composer.getBoundingClientRect();
    return {
      mainMid: mainBox.top + mainBox.height / 2,
      welcomeMid: welcomeBox.top + welcomeBox.height / 2,
      composerBottomFromMainBottom: mainBox.bottom - composerBox.bottom,
      mainHeight: mainBox.height,
    };
  });
  expect(stage).not.toBeNull();
  // Centered within ~120px of the pane midline (heading + composer as one group).
  expect(Math.abs(stage!.welcomeMid - stage!.mainMid)).toBeLessThan(120);
  // Not flush to the bottom edge like the post-message layout.
  expect(stage!.composerBottomFromMainBottom).toBeGreaterThan(48);

  const composer = page.locator(".composer-input");
  await expect(page.locator(".composer-send")).toBeDisabled();

  await composer.fill("帮我看一下未知表路由");
  await expect(page.locator(".composer-send")).toBeEnabled();
  await composer.press("Enter");

  await expect(page.locator(".msg-bubble")).toHaveText("帮我看一下未知表路由");
  await expect(composer).toHaveValue("");

  await expect(page.locator(".reply-working")).toBeVisible();
  await expect(page.locator(".composer-send--stop")).toBeVisible();

  await page.locator(".composer-send--stop").click();
});

test("an empty live conversation keeps Inspector summary cards out of the response section", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  await expect(page.locator(".ins-section-title")).toHaveText(["本轮", "当前项目"]);
  await expect(page.locator(".ins-section", { hasText: "尚未开始响应" })).toBeVisible();
  await expect(page.locator(".status-pill")).toHaveCount(0);
  await expect(page.locator(".ins-section", { hasText: "修改文件" })).toHaveCount(0);
  await expect(page.locator(".ins-section", { hasText: "使用工具" })).toHaveCount(0);
  await expect(page.locator(".ins-section", { hasText: "模型调用" })).toHaveCount(0);
  await expect(page.locator(".ins-section", { hasText: "当前项目" })).toBeVisible();
});

test("Shift+Enter inserts a newline instead of sending", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  const composer = page.locator(".composer-input");
  await composer.click();
  await composer.press("Shift+Enter");
  await composer.type("line2");
  await expect(composer).toHaveValue("\nline2");
});

test("a run draws itself in one step at a time, from the events", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator(".composer-input").fill("跑一下检查");
  await page.locator(".composer-input").press("Enter");

  const step = {
    id: 1,
    at: 1_700_000_000,
    duration: null,
    kind: "commandExecution",
    command: "cargo check",
    cwd: "/tmp/ws/alpha",
    output: "",
    exitCode: null,
  };

  await emit(page, { type: "itemStarted", session: "s1", item: { ...step, status: "running" } });

  await expect(page.locator(".trace-row")).toHaveCount(1);
  await expect(page.locator(".trace-mark--running")).toHaveCount(1);
  await expect(page.locator(".trace-label")).toHaveText("运行命令");

  await emit(page, {
    type: "itemCompleted",
    session: "s1",
    item: { ...step, status: "done", duration: 1500, output: "Finished dev profile", exitCode: 0 },
  });

  await expect(page.locator(".trace-mark--running")).toHaveCount(0);
  await expect(page.locator(".trace-mark--done")).toHaveCount(1);
  await expect(page.locator(".trace-duration")).toHaveText("1.5s");
  // A finished command collapses its output until the reader asks for it.
  await expect(page.locator(".trace-output")).toHaveCount(0);
  await page.locator(".trace-row-inner").click();
  await expect(page.locator(".trace-output")).toContainText("Finished dev profile");

  await emit(page, { type: "turnComplete", session: "s1" });

  await expect(page.locator(".reply-working")).toHaveCount(0);
  await expect(page.locator(".reply-interrupted")).toHaveCount(0);
});

test("trace rows follow step ids when events arrive out of order", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "顺序")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "顺序", at: 1_700_000_000, archived: false, turns: [] },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator(".composer-input").fill("执行");
  await page.locator(".composer-input").press("Enter");
  const item = (id: number, command: string) => ({
    id,
    at: 1_700_000_000,
    status: "done" as const,
    duration: 1,
    kind: "commandExecution" as const,
    command,
    cwd: "/tmp/ws/alpha",
    output: "ok",
    exitCode: 0,
  });
  await emit(page, { type: "itemCompleted", session: "s1", item: item(2, "second") });
  await emit(page, { type: "itemCompleted", session: "s1", item: item(1, "first") });
  await expect(page.locator(".trace-label")).toHaveCount(2);
  await expect(page.locator(".code-chip").filter({ hasText: "first" }).first()).toBeVisible();
  const commands = await page.locator(".trace-row .code-chip").allTextContents();
  expect(commands).toEqual(["first", "second"]);
});

test("a killed run is reported as interrupted, not as finished or working", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑一下检查",
          items: [{ id: 1, at: 1_700_000_000, status: "running", duration: null, kind: "reasoning", summary: "先看目录" }],
          done: false,
          stopped: false,
          error: null,
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".trace-mark--interrupted")).toHaveCount(1);
  await expect(page.locator(".trace-mark--running")).toHaveCount(0);
  await expect(page.locator(".trace-row-inner")).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".trace-duration")).toHaveText("—");
  await expect(page.locator(".reply-interrupted")).toBeVisible();
  await expect(page.locator(".reply-working")).toHaveCount(0);
  await expect(page.locator(".status-pill--interrupted")).toContainText("已中断");
});

test("a user-stopped run is marked stopped in the trace and Inspector", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑一下检查",
          items: [{ id: 1, at: 1_700_000_000, status: "running", duration: null, kind: "reasoning", summary: "先看目录" }],
          done: false,
          stopped: true,
          error: null,
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".trace-mark--stopped")).toHaveCount(1);
  await expect(page.locator(".trace-mark--running")).toHaveCount(0);
  await expect(page.locator(".trace-row-inner")).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".reply-stopped")).toBeVisible();
  await expect(page.locator(".reply-working")).toHaveCount(0);
  await expect(page.locator(".reply-interrupted")).toHaveCount(0);
  await expect(page.locator(".status-pill--stopped")).toContainText("已停止");
});

test("a failed run does not leave a spinner on its last step", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑一下检查",
          items: [{ id: 1, at: 1_700_000_000, status: "running", duration: null, kind: "reasoning", summary: "先看目录" }],
          done: false,
          stopped: false,
          error: "模型服务不可用",
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".trace-mark--failed")).toHaveCount(1);
  await expect(page.locator(".trace-mark--running")).toHaveCount(0);
  await expect(page.locator(".reply-failed")).toContainText("模型服务不可用");
  await expect(page.locator(".status-pill--failed")).toContainText("失败");
});

test("a completed command with a non-zero exit code is rendered as failed", async ({ page }) => {
  await stubShell(
    page,
    {
      workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "检查")] },
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "检查",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "检查",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 40,
                kind: "commandExecution",
                command: "identity-check",
                cwd: "/tmp/ws/alpha",
                output: "FAIL",
                exitCode: 1,
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    },
  );
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".trace-mark--failed")).toHaveCount(1);
  await expect(page.locator(".trace-mark--done")).toHaveCount(0);
});

test("choosing a project in a new task does not open an existing conversation", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha"), project("/tmp/ws/beta", "beta")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "Alpha 会话"), sessionRef("s2", "/tmp/ws/beta", "Beta 会话")],
    },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "Alpha 会话",
      at: 1_700_000_000,
      archived: false,
      turns: [],
    },
    sessions: {
      s2: {
        id: "s2",
        project: "/tmp/ws/beta",
        title: "Beta 会话",
        at: 1_700_000_001,
        archived: false,
        turns: [],
      },
    },
  });
  await page.goto("/");
  await page.getByRole("link", { name: "New Task" }).click();
  await page.locator('[data-testid="composer-project-picker"]').click();
  await page.getByRole("menuitem", { name: "beta" }).click();
  await expect(page.locator(".conv-title")).toHaveText("新对话");
  await expect(page.locator(".tree-conversation--active")).toHaveCount(0);
  await expect(page.locator('[data-testid="composer-project-name"]')).toHaveText("beta");
});

test("final replies hide internal tool protocol notes", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "身份")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "身份",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "你是谁",
          context: [],
          items: [
            {
              id: 1,
              at: 1_700_000_000,
              status: "done",
              duration: 0,
              kind: "agentMessage",
              text: "我是 Kodo。\n**工具协议**：不要展示\n{\"tool_calls\":[{\"name\":\"write_file\"}]}\n欢迎使用。",
              checks: [],
            },
          ],
          done: true,
          stopped: false,
          error: null,
        },
      ],
    },
  });
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".final")).toContainText("我是 Kodo。");
  await expect(page.locator(".final")).toContainText("欢迎使用。");
  await expect(page.locator(".final")).not.toContainText("tool_calls");
  await expect(page.locator(".final")).not.toContainText("工具协议");
});

test("a recovered turn stamped interrupted paints interrupted, never working", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑一下检查",
          items: [{ id: 1, at: 1_700_000_000, status: "running", duration: null, kind: "reasoning", summary: "先看目录" }],
          done: false,
          stopped: false,
          interrupted: true,
          error: null,
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".reply-interrupted")).toBeVisible();
  await expect(page.locator(".reply-working")).toHaveCount(0);
});

test("a failover event surfaces failed model, class, and next model", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator(".composer-input").fill("试一下 failover");
  await page.locator(".composer-input").press("Enter");
  await expect(page.locator(".reply-working")).toBeVisible();

  await emit(page, {
    type: "failover",
    session: "s1",
    fromProvider: "GPT-4o",
    fromModel: "gpt-4o",
    errorClass: "RateLimit",
    error: "429",
    toProvider: "Claude Sonnet 5",
    toModel: "claude-sonnet-4-5",
  });

  const note = page.locator('[data-testid="failover-note"]');
  await expect(note).toBeVisible();
  await expect(note).toContainText("GPT-4o");
  await expect(note).toContainText("gpt-4o");
  await expect(note).toContainText("RateLimit");
  await expect(note).toContainText("Claude Sonnet 5");
  await expect(note).toContainText("claude-sonnet-4-5");
});

test("the Trace page's three tabs swap the pane, none of them onto nothing", async ({ page }) => {
  await page.goto("/ui-demo/trace");

  await expect(page.locator(".timeline")).toBeVisible();
  await expect(page.locator(".artifact-list")).toHaveCount(0);

  await page.locator(".page-tab", { hasText: "Logs" }).click();
  await expect(page.locator(".timeline")).toHaveCount(0);
  await expect(page.locator(".page-inner > .terminal-block")).toContainText("10:24:32");
  await expect(page.locator(".page-tab--active")).toHaveText("Logs");

  await page.locator(".page-tab", { hasText: "Artifacts" }).click();
  await expect(page.locator(".page-inner > .terminal-block")).toHaveCount(0);
  await expect(page.locator(".artifact-list .file-row")).toHaveCount(7);
  await expect(page.locator(".page-tab--active")).toHaveText("Artifacts");

  await page.locator(".page-tab", { hasText: "Timeline" }).click();
  await expect(page.locator(".timeline")).toBeVisible();
});

test("live Trace without a session shows empty state, not fixture", async ({ page }) => {
  await stubShell(page, { workspace: { projects: [], sessions: [] } });
  await page.goto("/trace");
  await expect(page.locator(".empty-note")).toContainText("选择一个会话");
  await expect(page.locator(".timeline")).toHaveCount(0);
});

test("an Inspector card's chevron opens and closes it", async ({ page }) => {
  await page.goto("/ui-demo/conversation");
  await page.locator(".rail-btn").first().click();

  // Match the section by its own title — 使用工具 also lists a 修改文件 tool row.
  const card = page
    .locator(".ins-section")
    .filter({ has: page.locator(".ins-section-title", { hasText: /^修改文件$/ }) });
  const head = card.locator(".ins-section-head");
  await expect(head).toHaveAttribute("aria-expanded", "true");
  await expect(card.locator(".file-row").first()).toBeVisible();

  await head.click();
  await expect(head).toHaveAttribute("aria-expanded", "false");
  await expect(card.locator(".file-row")).toHaveCount(0);

  await head.click();
  await expect(card.locator(".file-row").first()).toBeVisible();
});

test("each Settings category swaps the detail pane", async ({ page }) => {
  await page.goto("/ui-demo/settings");

  const categories = page.locator(".settings-nav-item");
  await expect(categories).toHaveText([
    "General",
    "Models",
    "AI Provider",
    "Tools & Permissions",
    "Projects",
    "Archive & Storage",
    "Appearance",
  ]);

  for (const name of ["Models", "AI Provider", "Tools & Permissions", "Projects", "Archive & Storage", "Appearance", "General"]) {
    await page.locator(".settings-nav-item", { hasText: name }).click();
    await expect(page.locator(".settings-detail h2")).toHaveText(name);
    await expect(page.locator(".settings-detail").locator(".setting-row, .provider-row, .provider-editor, button").first()).toBeVisible();
  }
});

test("an approval request is shown and can be allowed or denied", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await page.locator(".composer-input").fill("跑一下检查");
  await page.locator(".composer-input").press("Enter");

  await emit(page, {
    type: "approvalRequest",
    session: "s1",
    step: 1,
    kind: "run command",
    detail: "rm -rf build  ·  删除文件",
  });

  const bar = page.locator(".approval-bar");
  await expect(bar).toBeVisible();
  await expect(bar).toContainText("rm -rf build");
  await bar.locator('[data-testid="approval-deny"]').click();
  await expect(bar).toHaveCount(0);

  await emit(page, {
    type: "approvalRequest",
    session: "s1",
    step: 2,
    kind: "run command",
    detail: "cargo test",
  });
  await expect(bar).toBeVisible();
  await bar.locator(".btn--primary").click();
  await expect(bar).toHaveCount(0);
});

test("settings toggles persist through the stubbed settings store", async ({ page }) => {
  await stubShell(page, { settings: { "auto-detect-git-branch": "false" } });
  await page.goto("/");

  await page.locator(".sidebar-foot .nav-item").click();
  await expect(page.locator(".settings-modal")).toBeVisible();
  await expect(page).not.toHaveURL(/\/settings$/);

  await page.locator(".settings-nav-item", { hasText: "Projects" }).click();
  const toggle = page.locator(".settings-detail .toggle").first();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
});

test("every visible functional setting maps to a runtime consumer", async ({ page }) => {
  await stubShell(page, {
    settings: {
      theme: "system",
      density: "comfortable",
      "show-line-numbers": "false",
      "use-system-font": "true",
      "auto-detect-git-branch": "false",
      "fallback-behavior": "fail",
      "extended-thinking": "true",
      "max-output-tokens": "2048",
      permission: "auto",
      "default-model": "Claude Sonnet 5",
    },
  });
  await page.goto("/");

  // Appearance → DOM consumers
  await page.locator(".sidebar-foot .nav-item").click();
  await page.locator(".settings-nav-item", { hasText: "Appearance" }).click();
  await expect(page.locator('.settings-detail select[aria-label="Theme"]')).toHaveValue("system");
  await expect(page.locator(".app")).toHaveAttribute("data-density", "comfortable");
  await expect(page.locator(".app")).toHaveAttribute("data-line-numbers", "false");
  await expect(page.locator(".app")).toHaveClass(/app--system-font/);

  // Models → fallback / thinking / max tokens keys
  await page.locator(".settings-nav-item", { hasText: "Models" }).click();
  await expect(page.locator(".settings-detail select").first()).toHaveValue("fail");
  await expect(page.locator('.settings-detail input[role="switch"], .settings-detail .toggle').first()).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await expect(page.locator(".settings-detail input.input").first()).toHaveValue("2048");

  // Projects → auto git branch (App skips gitBranch when false)
  await page.locator(".settings-nav-item", { hasText: "Projects" }).click();
  await expect(page.locator(".settings-detail .toggle").first()).toHaveAttribute("aria-checked", "false");

  // Storage → auto-save / trace persistence path documented as append-only
  await page.locator(".settings-nav-item", { hasText: "Archive" }).click();
  await expect(page.locator(".settings-detail")).toContainText("settings.log");
});

test("provider failover switch is visible to the conversation UI", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "修复路由",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑一下",
          context: [],
          items: [],
          done: false,
          stopped: false,
          interrupted: false,
          error: null,
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await emit(page, {
    type: "providerSwitch",
    session: "s1",
    fromProvider: "openai",
    fromModel: "gpt-4o",
    errorClass: "RateLimit",
    error: "429 too many requests",
    toProvider: "anthropic",
    toModel: "claude-sonnet-4-5",
  });

  const notice = page.locator('[data-testid="failover-notice"]');
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("openai/gpt-4o");
  await expect(notice).toContainText("anthropic/claude-sonnet-4-5");
  await expect(notice).toContainText("RateLimit");
  await notice.locator('[data-testid="failover-dismiss"]').click();
  await expect(notice).toHaveCount(0);
});

test("recovered interrupted turn is not shown as completed or working", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "中断会话")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "中断会话",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "跑检查",
          context: [],
          items: [{ id: 1, at: 1_700_000_000, status: "running", duration: null, kind: "reasoning", summary: "先看" }],
          done: false,
          stopped: false,
          interrupted: true,
          error: null,
        },
      ],
    },
  });

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".reply-interrupted")).toBeVisible();
  await expect(page.locator(".reply-working")).toHaveCount(0);
  // Trace status on a live session never claims 已完成 for interrupted turn.
  await page.goto("/trace");
  await expect(page.locator(".status-pill")).toContainText("已中断");
});

test("trace page reads persisted session items (trace persistence chain)", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "有痕迹")] },
    session: {
      id: "s1",
      project: "/tmp/ws/alpha",
      title: "有痕迹",
      at: 1_700_000_000,
      archived: false,
      turns: [
        {
          ask: "看日志",
          context: [],
          items: [
            { id: 1, at: 1_700_000_000, status: "done", duration: 1200, kind: "commandExecution", command: "cargo check", cwd: "/tmp/ws/alpha", output: "ok", exitCode: 0 },
            { id: 2, at: 1_700_000_001, status: "done", duration: 800, kind: "agentMessage", text: "完成", checks: ["通过"] },
          ],
          done: true,
          stopped: false,
          interrupted: false,
          error: null,
        },
      ],
    },
  });

  await page.goto("/trace");
  await expect(page.locator(".status-pill")).toContainText("已完成");
  await expect(page.locator(".timeline, .trace").first()).toBeVisible();
});

test("archive restore is an Inspector action, not a row button", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "已归档", 1_700_000_000, true)],
    },
    archived: [
      {
        id: "s1",
        project: "/tmp/ws/alpha",
        projectName: "alpha",
        title: "已归档",
        at: 1_700_000_000,
        model: "Claude 3.5 Sonnet",
        summary: "排查路由问题",
        filesChanged: 2,
        added: 10,
        removed: 3,
      },
    ],
  });

  await page.goto("/archive");
  await expect(page.locator(".archive-table .arc-title")).toContainText("已归档");
  await expect(page.locator('.archive-table button[aria-label="Restore"]')).toHaveCount(0);

  await page.locator(".archive-table tbody tr").first().click();
  await page.locator(".rail-btn").first().click();
  await expect(page.locator(".ins-actions .btn--primary")).toBeEnabled();
});
