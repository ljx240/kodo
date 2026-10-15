import { expect, test } from "@playwright/test";
import { emit, project, sessionRef, stubShell } from "./shell";

function liveCore(extra: Record<string, unknown> = {}) {
  return {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [sessionRef("s1", "/tmp/ws/alpha", "新对话")],
    },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "新对话", at: 1_700_000_000, archived: false, turns: [] },
    providers: [
      {
        id: "p1",
        name: "Local",
        template: "deepseek",
        apiKey: "••••dead",
        endpoint: "",
        model: "deepseek-chat",
        modelId: "deepseek-chat",
        displayName: "DeepSeek-V3",
      },
    ],
    settings: { "active-provider": "0" },
    ...extra,
  };
}

async function openLiveConversation(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("新对话");
}

test("slash selects a structured skill chip from the composer popup", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  await page.locator('[data-testid="composer-input"]').fill("/");
  await expect(page.locator('[data-testid="composer-slash"]')).toBeVisible();
  await page.locator('[data-slash-id="bug-fix"]').click();
  await expect(page.locator('[data-testid="composer-input"]')).toHaveValue("");
  await expect(page.locator('.composer-skill[data-skill-id="bug-fix"]')).toContainText("bug-fix");

  await page.locator('[data-testid="composer-input"]').fill("修复这个问题");
  await page.locator('[data-testid="composer-input"]').press("Enter");
  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string; context: string[] }> }).__sendLog ?? [],
  );
  expect(log[0]?.text).toBe("【技能：bug-fix】\n修复这个问题");
});

test("running composer queues the next ask and drains after turnComplete", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  const input = page.locator('[data-testid="composer-input"]');
  await input.fill("第一条");
  await input.press("Enter");
  await expect(page.locator(".reply-working")).toBeVisible();

  await input.fill("第二条排队");
  await input.press("Enter");
  await expect(page.locator('[data-testid="send-queue"]')).toContainText("第二条排队");
  await expect(page.locator('[data-testid="composer-status"]')).toContainText("队列 1");

  await emit(page, { type: "turnComplete", session: "s1" });
  await expect(page.locator(".msg-bubble")).toHaveText(["第一条", "第二条排队"]);
});

test("stopped and error turns are visible on the reply", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "新对话",
        at: 1_700_000_000,
        archived: false,
        turns: [
          { ask: "跑一下", context: [], items: [], done: false, stopped: true, error: null },
          { ask: "再跑", context: [], items: [], done: false, stopped: false, error: "provider unavailable" },
        ],
      },
    }),
  );

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator('[data-testid="reply-stopped"]')).toBeVisible();
  await expect(page.locator('[data-testid="reply-error"]')).toContainText("provider unavailable");
});

test("command step expands with cwd and exit code, failed stays open", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "新对话",
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
                status: "failed",
                duration: 1200,
                kind: "commandExecution",
                command: "cargo check",
                cwd: "/tmp/ws/alpha",
                output: "error: could not compile",
                exitCode: 101,
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    }),
  );

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".trace-row--failed")).toHaveCount(1);
  await expect(page.locator('[data-testid="trace-run-meta"]')).toContainText("/tmp/ws/alpha");
  await expect(page.locator('[data-testid="trace-run-meta"]')).toContainText("exit 101");
  await expect(page.locator('[data-testid="trace-exit-code"]')).toBeVisible();
});

test("first ask auto-titles the session from the message", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  await page.locator('[data-testid="composer-input"]').fill("修复 k2k 路由问题并补充测试");
  await page.locator('[data-testid="composer-input"]').press("Enter");

  await expect(page.locator(".crumb-current")).toHaveText("修复 k2k 路由问题并补充测试");
});

test("model picker lists models saved on the provider, not the template", async ({ page }) => {
  // `deepseek-thinking` is not in PROVIDER_TEMPLATES.deepseek — it can only
  // come from the provider's own `models` array.
  await stubShell(
    page,
    liveCore({
      providers: [
        {
          id: "p1",
          name: "Local",
          template: "deepseek",
          apiKey: "••••dead",
          endpoint: "",
          model: "deepseek-chat",
          modelId: "deepseek-chat",
          displayName: "DeepSeek-V3",
          models: [
            { modelId: "deepseek-chat", displayName: "DeepSeek-V3" },
            { modelId: "deepseek-thinking", displayName: "DeepSeek Thinking" },
          ],
        },
      ],
    }),
  );
  await openLiveConversation(page);

  await page.locator('[data-testid="composer-model"]').click();
  await page.locator('[data-provider-index="0"]').click();
  await expect(page.locator('[data-model-id="deepseek-thinking"]')).toBeVisible();
  await page.locator('[data-model-id="deepseek-thinking"]').click();
  await expect(page.locator('[data-testid="composer-model"]')).toHaveText(/DeepSeek Thinking/);
  // The selection reaches the runtime consumer (`default-model`).
  const log = await page.evaluate(
    () =>
      (window as unknown as { __settingsLog?: Array<{ key: string; value: string }> }).__settingsLog ?? [],
  );
  expect(log).toContainEqual({ key: "default-model", value: "deepseek-thinking" });
});

test("+ project file panel searches and pins a context chip", async ({ page }) => {
  await stubShell(page, liveCore({ files: ["src/app.ts", "src/util.ts", "README.md"] }));
  await openLiveConversation(page);

  await page.locator('.composer button[aria-label="添加内容"]').click();
  await page.locator('[data-testid="project-files-item"]').click();
  await expect(page.locator('[data-testid="context-file-list"]')).toBeVisible();
  await page.locator('.composer-files-search input').fill("app");
  await page.locator('[data-file-path="src/app.ts"]').click();
  await expect(page.locator('[data-context-path="src/app.ts"]')).toBeVisible();
});

test("final answer renders markdown code blocks", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "修复路由",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "怎么写",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 400,
                kind: "agentMessage",
                text: "用 `run` 即可。\n\n```ts\nrun();\n```\n\n```toml\nname = \"kodo\"\n```\n\n- 第一步\n- 第二步",
                checks: [],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    }),
  );

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator(".md-code").first()).toContainText("run();");
  // The ```ts fence carries syntax color: `run` is a call title in hljs.
  await expect(page.locator('.md-code[data-lang="ts"] .hljs-title')).toContainText("run");
  await expect(page.locator(".md-inline-code")).toHaveText("run");
  await expect(page.locator(".md-list li")).toHaveCount(2);

  // Header strip: fence language label + copy (full body, not the fold).
  await expect(page.locator(".md-code-lang").first()).toHaveText("ts");
  await expect(page.locator('.md-code[data-lang="toml"] .hljs-string')).toContainText("kodo");
  await page.locator('[data-testid="md-code-copy"]').first().click();
  await expect(page.locator('[data-testid="md-code-copy"]').first()).toHaveAttribute(
    "title",
    "已复制",
  );
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("run();");
});

test("final answer renders tables, heading tiers, quotes, nested/task lists, TOC, rules and links", async ({ page }) => {
  const richAnswer = [
    "# 总览",
    "",
    "开篇段落。",
    "",
    "## 排期",
    "| 任务 | 工时 |",
    "| --- | ---: |",
    "| 设计 | 3 |",
    "| 开发 | 5 |",
    "",
    "### 设计要点",
    "> 第一段引用。",
    ">",
    "> 第二段引用。",
    "",
    "- 外层",
    "  - 内层",
    "- [ ] 待办项",
    "- [x] 完成项",
    "",
    "##### 附注",
    "见 [文档](https://example.com/docs)。",
    "",
    "---",
    "",
    "```diff",
    "+ 新增行",
    "- 删除行",
    "```",
    "",
    "价格 $5 起。",
  ].join("\n");

  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "格式验收",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "排个期",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 400,
                kind: "agentMessage",
                text: richAnswer,
                checks: [],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    }),
  );

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  // GFM table: header + two rows, right-aligned second column.
  await expect(page.locator(".md-table")).toHaveCount(1);
  await expect(page.locator(".md-table thead th")).toHaveCount(2);
  await expect(page.locator(".md-table tbody tr")).toHaveCount(2);
  await expect(page.locator(".md-table tbody tr td").nth(1)).toHaveCSS("text-align", "right");

  // Heading tiers: h1/h2 → h3, h3/h4 → h4, h5/h6 → h5.
  await expect(page.locator("h3.md-heading")).toHaveCount(2);
  await expect(page.locator("h4.md-heading")).toHaveCount(1);
  await expect(page.locator("h5.md-heading")).toHaveCount(1);

  await expect(page.locator(".md-quote p")).toHaveCount(2);
  await expect(page.locator(".md-list li .md-list li")).toHaveCount(1);
  await expect(page.locator('input[type="checkbox"]')).toHaveCount(2);
  await expect(page.locator('input[type="checkbox"]:checked')).toHaveCount(1);

  // 4 headings trigger the auto-TOC; anchors are plain in-page jumps.
  await expect(page.locator('[data-testid="md-toc"] a')).toHaveCount(4);
  await expect(page.locator('[data-testid="md-toc"] a').first()).toHaveAttribute("href", /^#md-h-/);

  await expect(page.locator("a.md-link")).toHaveAttribute("href", "https://example.com/docs");
  await expect(page.locator(".md-hr")).toHaveCount(1);

  // ```diff colors its hunks without leaking into plain code blocks.
  await expect(page.locator(".md-code .hljs-addition")).toContainText("新增行");
  await expect(page.locator(".md-code .hljs-deletion")).toContainText("删除行");

  // Currency stays literal — no closing `$`, so no KaTeX.
  await expect(page.locator(".md-para").filter({ hasText: "$5" })).toBeVisible();
});

test("final answer renders extended inline syntax, ordered starts, nested quotes, callouts and footnotes", async ({ page }) => {
  const extendedAnswer = [
    "~~删除~~ ==高亮== __粗体__ _斜体_ \\*原样星号\\*，联系 user@example.com 或访问 www.example.com。",
    "",
    "5. 第五",
    "6. 第六",
    "",
    "> 外层引用",
    "> > 内层引用",
    "",
    "> [!TIP]",
    "> 建议先跑测试。",
    "",
    "> [!NOTE] 同行标题",
    "> 正文接着写。",
    "",
    "脚注见[^1]，缺失[^9]保持原文。",
    "",
    "[^1]: 这是脚注说明。",
  ].join("\n");

  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "扩展语法",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "写点格式",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 400,
                kind: "agentMessage",
                text: extendedAnswer,
                checks: [],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    }),
  );

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  // Inline syntax completeness.
  await expect(page.locator("del")).toContainText("删除");
  await expect(page.locator("mark")).toContainText("高亮");
  await expect(page.locator("strong").filter({ hasText: "粗体" })).toBeVisible();
  await expect(page.locator("em").filter({ hasText: "斜体" })).toBeVisible();
  // Backslash escapes keep the literal characters.
  await expect(page.locator(".md-para").first()).toContainText("*原样星号*");
  // www + email autolinks.
  await expect(page.locator('a.md-link[href="https://www.example.com"]')).toBeVisible();
  await expect(page.locator('a.md-link[href="mailto:user@example.com"]')).toBeVisible();

  // Ordered list keeps its original start number.
  await expect(page.locator('ol.md-list[start="5"] li')).toHaveCount(2);
  await expect(page.locator('ol.md-list[start="5"] li').first()).toContainText("第五");

  // `>>` nests instead of flattening.
  await expect(page.locator(".md-quote .md-quote")).toHaveCount(1);
  await expect(page.locator(".md-quote .md-quote")).toContainText("内层引用");

  // `> [!TIP]` becomes a labeled callout.
  await expect(page.locator('.md-callout[data-callout="tip"]')).toContainText("提示");
  await expect(page.locator('.md-callout[data-callout="tip"]')).toContainText("建议先跑测试");
  // A marker with text on the same line keeps the text as the body's first line.
  await expect(page.locator('.md-callout[data-callout="note"]')).toContainText("同行标题");
  await expect(page.locator('.md-callout[data-callout="note"]')).toContainText("正文接着写");

  // Footnote: defined ref links out, missing ref stays literal, defs hoist.
  await expect(page.locator(".md-fnref a")).toHaveCount(1);
  await expect(page.locator(".md-fnref a")).toHaveAttribute("href", "#md-fn-1");
  await expect(page.locator(".md-para").last()).toContainText("[^9]");
  await expect(page.locator('[data-testid="md-footnotes"]')).toContainText("这是脚注说明");
  await expect(page.locator(".md-footnote-back")).toHaveAttribute("href", "#md-fnr-1");
  // The definition paragraph is hoisted — never rendered inline.
  await expect(page.locator(".md-para").filter({ hasText: "这是脚注说明" })).toHaveCount(0);
});

test("TOC scroll-follow highlights the active heading and caps its height", async ({ page }) => {
  const headings = Array.from({ length: 30 }, (_, index) => `## 标题 ${index + 1}`).join("\n\n");
  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "目录跟随",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "长文",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 400,
                kind: "agentMessage",
                text: headings,
                checks: [],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    }),
  );

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator('[data-testid="md-toc"] a')).toHaveCount(30);
  // Long directories scroll inside their bounded list.
  const scroll = await page.locator(".md-toc-list").evaluate((el) => ({
    scrollHeight: el.scrollHeight,
    clientHeight: el.clientHeight,
  }));
  expect(scroll.scrollHeight).toBeGreaterThan(scroll.clientHeight);

  // The conversation rests at the newest turn, so some lower heading is
  // already in the reading band.
  await expect(page.locator(".md-toc a.md-toc-active")).toHaveCount(1);

  // Jumping to the first heading lands it at the container top — inside the
  // band — so its entry lights up.
  const firstHeading = page.locator(".md-heading").first();
  const firstId = await firstHeading.getAttribute("id");
  await firstHeading.evaluate((el) => el.scrollIntoView({ block: "start" }));
  await expect(page.locator(".md-toc a.md-toc-active")).toHaveAttribute("href", `#${firstId}`);
});

test("final answer renders KaTeX formulas and mermaid diagrams after lazy load", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const formulaAnswer = [
    "质能方程为 $E=mc^2$。",
    "",
    "$$\\int_0^1 x^2 dx = \\frac{1}{3}$$",
    "",
    "```mermaid",
    "flowchart TD",
    "  A[开始] --> B[结束]",
    "```",
    "",
    "合计 $5。",
  ].join("\n");

  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "公式验收",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "画个图",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 400,
                kind: "agentMessage",
                text: formulaAnswer,
                checks: [],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    }),
  );

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  // KaTeX lazy-loads: inline + display render, currency stays literal.
  await expect(page.locator(".katex-display")).toHaveCount(1);
  await expect(page.locator(".katex")).toHaveCount(2);
  await expect(page.locator(".md-para").filter({ hasText: "$5" })).toBeVisible();

  // mermaid renders an SVG instead of falling back to the plain code block.
  await expect(page.locator(".md-mermaid svg")).toBeVisible();
  await expect(page.locator('.md-code[data-lang="mermaid"]')).toHaveCount(0);

  // Layout sanity: Playwright runs with prefers-reduced-motion, which used to
  // make mermaid's synchronous size reads return pre-transition values and
  // blow the graph up to a ~2700px viewBox with nodes clipped outside it.
  const viewBoxWidth = await page.locator(".md-mermaid svg").evaluate((el) => {
    const vb = el.getAttribute("viewBox")?.split(/\s+/).map(Number);
    return vb?.[2] ?? Number.POSITIVE_INFINITY;
  });
  expect(viewBoxWidth).toBeLessThan(1000);

  // Source view swaps the diagram for the mermaid fence, and back.
  await page.locator('[data-testid="md-mermaid-source"]').click();
  await expect(page.locator('.md-code[data-lang="mermaid"]')).toContainText("flowchart TD");
  await page.locator('[data-testid="md-mermaid-source"]').click();
  await expect(page.locator(".md-mermaid svg")).toBeVisible();

  // Fullscreen lightbox: opens, zooms a step, closes on Escape.
  await page.locator('[data-testid="md-mermaid-fullscreen"]').click();
  await expect(page.locator('[data-testid="md-lightbox"]')).toBeVisible();
  await page.locator('[data-testid="md-lightbox-in"]').click();
  await expect(page.locator(".md-lightbox-zoom")).toHaveAttribute("data-zoom", "1.5");
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-testid="md-lightbox"]')).toHaveCount(0);

  // Block formulas carry a LaTeX copy button; inline math stays bare.
  await expect(page.locator('[data-testid="md-formula-copy"]')).toHaveCount(1);
  await page.locator('[data-testid="md-formula-copy"]').click();
  await expect(page.locator('[data-testid="md-formula-copy"]')).toHaveAttribute("title", "已复制");
});

test("regenerate re-sends the last ask", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "修复路由",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "检查路由",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 300,
                kind: "agentMessage",
                text: "已检查。",
                checks: [],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    }),
  );

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  await expect(page.locator('[data-testid="reply-regenerate"]')).toBeVisible();
  await page.locator('[data-testid="reply-regenerate"]').click();
  await expect(page.locator(".msg-bubble")).toHaveCount(2);
  await expect(page.locator(".msg-bubble").nth(1)).toHaveText("检查路由");

  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string }> }).__sendLog ?? [],
  );
  expect(log.some((entry) => entry.text === "检查路由")).toBe(true);
});

test("approval offers session-wide allow and records it on the invoke", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  await page.locator(".composer-input").fill("跑一下");
  await page.locator(".composer-input").press("Enter");
  await emit(page, {
    type: "approvalRequest",
    session: "s1",
    step: 1,
    kind: "run command",
    detail: "cargo test  ·  测试",
    command: "cargo test",
  });

  await expect(page.locator('[data-testid="approval-allow-session"]')).toBeVisible();
  await page.locator('[data-testid="approval-allow-session"]').click();

  const log = await page.evaluate(
    () =>
      (window as unknown as { __approvalLog?: Array<{ approved: boolean; sessionWide: boolean }> })
        .__approvalLog ?? [],
  );
  expect(log).toHaveLength(1);
  expect(log[0].approved).toBe(true);
  expect(log[0].sessionWide).toBe(true);
});

test("edit step expands to the unified diff from turn_changes", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      turnChanges: [
        {
          path: "src/app.ts",
          diff: "--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1 +1 @@\n-old\n+new\n",
        },
      ],
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "改文件",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "改一下",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 400,
                kind: "fileChange",
                changes: [{ path: "src/app.ts", added: 1, removed: 1 }],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    }),
  );

  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();

  // Open the folded process first, then the completed edit's own details.
  await page.locator('[data-testid="process-tools"] > summary').click();
  await expect(page.locator('[data-testid="trace-file-list"]')).toHaveCount(0);
  await page.locator(".trace-label").click();
  await expect(page.locator('[data-testid="trace-file-list"]')).toBeVisible();
  await expect(page.locator('[data-testid="trace-diff-src/app.ts"]')).toContainText("+new");

  // Shared renderer: colored rows, headers stripped — never raw `--- a/`.
  await expect(page.locator('[data-testid="trace-diff-src/app.ts"] .is-add')).toContainText("new");
  await expect(page.locator('[data-testid="trace-diff-src/app.ts"] .is-del')).toContainText("old");
  await expect(page.locator('[data-testid="trace-diff-src/app.ts"]')).not.toContainText("--- a/");
});

async function sendLog(page: import("@playwright/test").Page) {
  return page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string }> }).__sendLog ?? [],
  );
}

test("Enter confirming an IME composition never sends — only a plain Enter does", async ({
  page,
}) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  const input = page.locator('[data-testid="composer-input"]');
  await input.fill("中文输入中");
  await input.focus();
  // Chrome/WebKit report the confirming Enter as keyCode 229 or through the
  // composition lifecycle — both paths sit behind the same guard.
  await input.evaluate((el) => {
    el.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    el.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        keyCode: 229,
        bubbles: true,
        cancelable: true,
      }),
    );
  });
  expect(await sendLog(page)).toHaveLength(0);
  await expect(input).toHaveValue("中文输入中");

  await input.evaluate((el) =>
    el.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })),
  );
  await input.press("Enter");
  const log = await sendLog(page);
  expect(log).toHaveLength(1);
  expect(log[0]?.text).toBe("中文输入中");
});

test("drafts persist per conversation, survive reload, and clear on send", async ({
  page,
}) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  const input = page.locator('[data-testid="composer-input"]');
  await input.fill("没发出去的草稿");
  const keys = await page.evaluate(() =>
    Object.keys(localStorage).filter((key) => key.startsWith("kodo:draft:")),
  );
  expect(keys).toHaveLength(1);
  expect(keys[0]).toContain("s1");

  // A reload keeps the draft — the key loads back on mount.
  await page.reload();
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(input).toHaveValue("没发出去的草稿");

  // Key change isolation: the new-task composer starts blank…
  await page.locator(".tree-project").hover();
  await page.locator(".tree-project-add").click();
  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  await expect(input).toHaveValue("");

  // …and coming back restores this conversation's draft.
  await page.locator(".tree-conversation").click();
  await expect(input).toHaveValue("没发出去的草稿");

  // Sending consumes the draft — it must not resurrect next visit.
  await input.press("Enter");
  await expect(input).toHaveValue("");
  const remaining = await page.evaluate(() =>
    Object.keys(localStorage).filter((key) => key.startsWith("kodo:draft:")),
  );
  expect(remaining).toHaveLength(0);
});

test("user turns expose copy and 编辑并发送 — edit prefills the composer and resends", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "新对话",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "帮我修复路由问题",
            context: [],
            items: [],
            done: true,
            stopped: false,
            interrupted: false,
            error: null,
          },
        ],
      },
    }),
  );
  await openLiveConversation(page);

  const copy = page.locator('[data-testid="msg-copy"]');
  await expect(copy).toBeVisible();
  await copy.click();
  await expect(copy).toHaveAttribute("title", "已复制");
  await expect
    .poll(() =>
      page.evaluate(() => navigator.clipboard.readText().catch(() => "")),
    )
    .toBe("帮我修复路由问题");

  // 编辑并发送 pre-fills and focuses — the append-only log gains a new turn,
  // it never truncates the old one.
  await page.locator('[data-testid="msg-edit"]').click();
  const input = page.locator('[data-testid="composer-input"]');
  await expect(input).toHaveValue("帮我修复路由问题");
  await expect(input).toBeFocused();
  await input.press("Enter");
  const log = await sendLog(page);
  expect(log).toHaveLength(1);
  expect(log[0]?.text).toBe("帮我修复路由问题");
  // The original bubble stays and the resend appends beside it — the log is
  // append-only, never truncated.
  await expect(page.locator(".msg-user")).toHaveCount(2);
  await expect(page.locator(".msg-bubble").first()).toHaveText("帮我修复路由问题");
});

test("⌘F opens the find bar, counts hits, cycles turns and outlines the active one", async ({
  page,
}) => {
  await stubShell(
    page,
    liveCore({
      session: {
        id: "s1",
        project: "/tmp/ws/alpha",
        title: "新对话",
        at: 1_700_000_000,
        archived: false,
        turns: [
          {
            ask: "修复路由问题",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_000_000,
                status: "done",
                duration: 400,
                kind: "agentMessage",
                text: "路由已经修好。",
                checks: [],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
          {
            ask: "优化启动速度",
            context: [],
            items: [
              {
                id: 2,
                at: 1_700_000_100,
                status: "done",
                duration: 400,
                kind: "agentMessage",
                text: "路由逻辑保持不变。",
                checks: [],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    }),
  );
  await openLiveConversation(page);

  await page.keyboard.press("ControlOrMeta+f");
  const bar = page.locator('[data-testid="find-bar"]');
  await expect(bar).toBeVisible();
  const input = page.locator('[data-testid="find-input"]');
  await expect(input).toBeFocused();

  await input.fill("路由");
  // Hits: turn 0 ask, turn 0 reply, turn 1 reply — the active one outlines.
  await expect(page.locator('[data-testid="find-count"]')).toHaveText("1/3");
  await expect(page.locator(".find-hit")).toHaveCount(1);
  await expect(page.locator(".msg-user").first()).toHaveClass(/find-hit/);

  await input.press("Enter");
  await expect(page.locator('[data-testid="find-count"]')).toHaveText("2/3");
  await expect(page.locator(".reply").first()).toHaveClass(/find-hit/);

  await input.press("Enter");
  await expect(page.locator('[data-testid="find-count"]')).toHaveText("3/3");
  await input.press("Enter");
  await expect(page.locator('[data-testid="find-count"]')).toHaveText("1/3");
  // Shift+Enter walks backwards (wraps).
  await input.press("Shift+Enter");
  await expect(page.locator('[data-testid="find-count"]')).toHaveText("3/3");

  await input.fill("不存在的词");
  await expect(page.locator('[data-testid="find-count"]')).toHaveText("无匹配");
  await expect(page.locator(".find-hit")).toHaveCount(0);

  await input.press("Escape");
  await expect(bar).toBeHidden();
});

test("welcome offers continuing the most recent conversation of this space", async ({ page }) => {
  await stubShell(page, {
    workspace: {
      projects: [project("/tmp/ws/alpha", "alpha")],
      sessions: [
        sessionRef("s1", "/tmp/ws/alpha", "更早的会话", 1_700_000_000),
        sessionRef("s2", "/tmp/ws/alpha", "最新的会话", 1_700_001_000),
      ],
    },
    sessions: {
      s2: {
        id: "s2",
        project: "/tmp/ws/alpha",
        title: "最新的会话",
        at: 1_700_001_000,
        archived: false,
        turns: [
          {
            ask: "上次的问题",
            context: [],
            items: [
              {
                id: 1,
                at: 1_700_001_000,
                status: "done",
                duration: 100,
                kind: "agentMessage",
                text: "上次的回答。",
                checks: [],
              },
            ],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    },
    branch: "main",
  });
  await page.goto("/");
  await page.waitForLoadState("networkidle");

  const cta = page.locator('[data-testid="welcome-continue"]');
  await expect(cta).toBeVisible();
  await cta.click();

  await expect(page.locator('[data-testid="welcome"]')).toHaveCount(0);
  await expect(page.locator(".crumb-current")).toHaveText("最新的会话");
});

test("welcome hides the continue CTA when the space has no history", async ({ page }) => {
  await stubShell(page, {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [] },
    branch: "main",
  });
  await page.goto("/");
  await page.waitForLoadState("networkidle");

  await expect(page.locator('[data-testid="welcome"]')).toBeVisible();
  await expect(page.locator('[data-testid="welcome-continue"]')).toHaveCount(0);
});
