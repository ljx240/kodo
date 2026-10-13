import { expect, test } from "@playwright/test";
import { emit, project, sessionRef, stubShell } from "./shell";

/** Open the + menu and trigger "添加照片和文件" (OS / stubbed filesystem picker). */
async function attachFiles(page: import("@playwright/test").Page) {
  await page.locator('.composer button[aria-label="添加内容"]').click();
  await page.locator(".composer-plus-menu").getByRole("menuitem", { name: "添加照片和文件" }).click();
}

/** Common live conversation stub with one project + session. */
function liveCore(extra: Record<string, unknown> = {}) {
  return {
    workspace: { projects: [project("/tmp/ws/alpha", "alpha")], sessions: [sessionRef("s1", "/tmp/ws/alpha", "修复路由")] },
    session: { id: "s1", project: "/tmp/ws/alpha", title: "修复路由", at: 1_700_000_000, archived: false, turns: [] },
    providers: [{ id: "p1", name: "Local", template: "openai", apiKey: "••••dead", endpoint: "", model: "gpt-test" }],
    settings: { "active-provider": "0" },
    ...extra,
  };
}

async function openLiveConversation(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.locator(".tree-project-main").click();
  await page.locator(".tree-conversation").click();
  await expect(page.locator(".crumb-current")).toHaveText("修复路由");
}

test("no decorative Mic button remains in the composer", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);
  await expect(page.locator('.composer button[aria-label="Voice input"]')).toHaveCount(0);
  await expect(page.locator('.composer button[aria-label="添加内容"]')).toHaveCount(1);
});

test("+ opens a menu, not an inline file list", async ({ page }) => {
  await stubShell(page, liveCore({ files: ["src/app.ts"] }));
  await openLiveConversation(page);

  await page.locator('.composer button[aria-label="添加内容"]').click();
  await expect(page.locator('[data-testid="context-file-list"]')).toHaveCount(0);
  await expect(page.locator(".composer-plus-menu")).toBeVisible();
  await expect(page.locator(".composer-plus-menu").getByRole("menuitem", { name: "添加照片和文件" })).toBeVisible();

  // Menu sits just above the composer card (reference: tight to the box, not a tall gap).
  const gap = await page.evaluate(() => {
    const box = document.querySelector(".composer-box");
    const menu = document.querySelector(".composer-plus-menu");
    if (!box || !menu) return null;
    const boxTop = box.getBoundingClientRect().top;
    const menuBottom = menu.getBoundingClientRect().bottom;
    return boxTop - menuBottom;
  });
  expect(gap).not.toBeNull();
  expect(gap!).toBeGreaterThanOrEqual(0);
  expect(gap!).toBeLessThanOrEqual(16);
});

test("+ child panels stay joined to the action menu", async ({ page }) => {
  await stubShell(page, liveCore({ files: ["src/app.ts"] }));
  await openLiveConversation(page);

  await page.locator('.composer button[aria-label="添加内容"]').click();
  await page.locator(".composer-plus-menu").getByRole("menuitem", { name: "项目文件" }).click();
  await expect(page.locator(".composer-files-panel")).toBeVisible();

  const geometry = await page.evaluate(() => {
    const root = document.querySelector(".composer-plus-root")?.getBoundingClientRect();
    const menu = document.querySelector(".composer-plus-menu")?.getBoundingClientRect();
    const panel = document.querySelector(".composer-plus-panel")?.getBoundingClientRect();
    if (!root || !menu || !panel) return null;
    return {
      root,
      horizontalGap: panel.left - menu.right,
      bottomDelta: Math.abs(panel.bottom - menu.bottom),
    };
  });
  expect(geometry).not.toBeNull();
  expect(geometry!.root.width).toBeGreaterThan(geometry!.root.height);
  expect(geometry!.horizontalGap).toBe(0);
  expect(geometry!.bottomDelta).toBeLessThanOrEqual(1);
});

test("+ lists builtin skills as a removable chip and serializes it on send", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);

  await page.locator('.composer button[aria-label="添加内容"]').click();
  await page.locator(".composer-plus-menu", { hasText: "技能" }).getByRole("menuitem", { name: "技能" }).click();
  await page.locator('[data-skill-id="bug-fix"]').click();

  await expect(page.locator(".composer-input")).toHaveValue("");
  await expect(page.locator('.composer-skill[data-skill-id="bug-fix"]')).toContainText("bug-fix");
  await expect(page.locator('[aria-label="移除技能 bug-fix"]')).toBeVisible();
  await page.locator('[aria-label="移除技能 bug-fix"]').click();
  await expect(page.locator(".composer-skill")).toHaveCount(0);

  await page.locator('.composer button[aria-label="添加内容"]').click();
  await page.locator(".composer-plus-menu", { hasText: "技能" }).getByRole("menuitem", { name: "技能" }).click();
  await page.locator('[data-skill-id="bug-fix"]').click();

  await page.locator('.composer-input').fill("修复这个问题");
  await page.locator('.composer-input').press("Enter");
  await expect(page.locator(".composer-skill")).toHaveCount(0);

  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string; context: string[] }> }).__sendLog ?? [],
  );
  expect(log).toHaveLength(1);
  expect(log[0].text).toBe("【技能：bug-fix】\n修复这个问题");
});

test("add context chip via 添加照片和文件 filesystem picker", async ({ page }) => {
  await stubShell(page, liveCore({ pickFiles: ["src/app.ts"] }));
  await openLiveConversation(page);

  await attachFiles(page);
  await expect(page.locator('[data-context-path="src/app.ts"]')).toBeVisible();
  // Message body must not include the path yet.
  await expect(page.locator(".msg-bubble")).toHaveCount(0);
});

test("add multiple context chips from one filesystem pick", async ({ page }) => {
  await stubShell(page, liveCore({ pickFiles: ["src/app.ts", "src/util.ts"] }));
  await openLiveConversation(page);

  await attachFiles(page);

  await expect(page.locator("[data-context-path]")).toHaveCount(2);
  await expect(page.locator('[data-context-path="src/app.ts"]')).toBeVisible();
  await expect(page.locator('[data-context-path="src/util.ts"]')).toBeVisible();
});

test("remove a context chip", async ({ page }) => {
  await stubShell(page, liveCore({ pickFiles: ["src/app.ts", "README.md"] }));
  await openLiveConversation(page);

  await attachFiles(page);
  await expect(page.locator("[data-context-path]")).toHaveCount(2);
  await page.locator('[aria-label="移除上下文 src/app.ts"]').click();
  await expect(page.locator('[data-context-path="src/app.ts"]')).toHaveCount(0);
  await expect(page.locator('[data-context-path="README.md"]')).toBeVisible();
});

test("attachment cards render per type: image thumbnail, PDF/Office/TXT badges", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      files: ["notes.txt"],
      pickFiles: [
        "/tmp/shot.png",
        "/tmp/report.pdf",
        "/tmp/budget.xlsx",
        "/tmp/spec.docx",
        "notes.txt",
      ],
      externalFiles: ["/tmp/shot.png", "/tmp/report.pdf", "/tmp/budget.xlsx", "/tmp/spec.docx"],
    }),
  );
  await openLiveConversation(page);

  await attachFiles(page);
  await expect(page.locator("[data-context-path]")).toHaveCount(5);

  // Image → thumbnail card with a real <img> preview.
  const image = page.locator('[data-context-path="/tmp/shot.png"]');
  await expect(image).toHaveClass(/ctx-card--image/);
  await expect(image.locator(".ctx-card-preview img")).toHaveAttribute("src", /^data:image\/png/);
  await expect(image.locator(".ctx-badge")).toHaveText("IMG");

  // PDF → gray card, dark badge, file name (Codex reference).
  const pdf = page.locator('[data-context-path="/tmp/report.pdf"]');
  await expect(pdf).toHaveClass(/ctx-card--pdf/);
  await expect(pdf.locator(".ctx-card-name")).toHaveText("report.pdf");
  await expect(pdf.locator(".ctx-badge")).toHaveText("PDF");
  await expect(pdf.locator(".ctx-badge")).toHaveCSS("background-color", "rgb(31, 35, 40)");

  // Office spreadsheets / docs get their own marks.
  await expect(page.locator('[data-context-path="/tmp/budget.xlsx"] .ctx-badge')).toHaveText("XLS");
  await expect(page.locator('[data-context-path="/tmp/budget.xlsx"] .ctx-badge')).toHaveCSS(
    "background-color",
    "rgb(33, 115, 70)",
  );
  await expect(page.locator('[data-context-path="/tmp/spec.docx"] .ctx-badge')).toHaveText("DOC");
  await expect(page.locator('[data-context-path="/tmp/spec.docx"] .ctx-badge')).toHaveCSS(
    "background-color",
    "rgb(43, 87, 154)",
  );

  // Plain text → TXT badge, still removable via the shared hook.
  const text = page.locator('[data-context-path="notes.txt"]');
  await expect(text).toHaveClass(/ctx-card--text/);
  await expect(text.locator(".ctx-badge")).toHaveText("TXT");
  await expect(text.locator(".ctx-card-name")).toHaveText("notes.txt");

  // Cards sit above the draft, not on the bottom control line.
  const order = await page.evaluate(() => {
    const contexts = document.querySelector(".composer-contexts")?.getBoundingClientRect();
    const row = document.querySelector(".composer-row")?.getBoundingClientRect();
    return contexts && row ? contexts.top < row.top : null;
  });
  expect(order).toBe(true);
});

test("a single image attachment shows one large thumbnail above the input", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      pickFiles: ["/tmp/shot.png"],
      externalFiles: ["/tmp/shot.png"],
    }),
  );
  await openLiveConversation(page);

  await attachFiles(page);
  const card = page.locator('[data-context-path="/tmp/shot.png"]');
  await expect(card).toBeVisible();
  const size = await card.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    const preview = el.querySelector(".ctx-card-preview")?.getBoundingClientRect();
    return { width: rect.width, previewHeight: preview?.height ?? 0 };
  });
  expect(size.width).toBeGreaterThanOrEqual(200);
  expect(size.previewHeight).toBeGreaterThanOrEqual(120);
});

test("send passes structured context in the invoke payload, not in the ask text", async ({ page }) => {
  await stubShell(page, liveCore({ pickFiles: ["src/app.ts"] }));
  await openLiveConversation(page);

  await attachFiles(page);

  await page.locator(".composer-input").fill("看看这个文件");
  await page.locator(".composer-input").press("Enter");

  await expect(page.locator(".msg-bubble")).toHaveText("看看这个文件");
  await expect(page.locator(".msg-bubble")).not.toContainText("src/app.ts");
  // Sent bubbles render the same attachment card as the composer: filename in
  // the card, full path on the title/attribute, no bare-path chip.
  const card = page.locator('.msg-user [data-context-path="src/app.ts"]');
  await expect(card).toBeVisible();
  await expect(card).toHaveAttribute("title", "src/app.ts");
  await expect(card.locator(".ctx-card-name")).toHaveText("app.ts");
  await expect(card).toHaveClass(/ctx-card--text/);
  // Static in the bubble — no remove control, no legacy path chip.
  await expect(card.locator(".chip-remove")).toHaveCount(0);
  await expect(page.locator(".msg-user .chip--context")).toHaveCount(0);

  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string; context: string[] }> }).__sendLog ?? [],
  );
  expect(log).toHaveLength(1);
  expect(log[0].text).toBe("看看这个文件");
  expect(log[0].context).toEqual(["src/app.ts"]);
});

test("sent image keeps its real thumbnail in the bubble", async ({ page }) => {
  await stubShell(page, liveCore({ files: ["src/shot.png"], pickFiles: ["src/shot.png"] }));
  await openLiveConversation(page);

  await attachFiles(page);
  await expect(page.locator('.composer-contexts [data-context-path="src/shot.png"] img')).toBeVisible();

  await page.locator(".composer-input").fill("看看这张图");
  await page.locator(".composer-input").press("Enter");

  // The probe taken at pick time survives into the bubble — same card, same thumbnail.
  const card = page.locator('.msg-user [data-context-path="src/shot.png"]');
  await expect(card).toBeVisible();
  await expect(card).toHaveClass(/ctx-card--image/);
  await expect(card.locator("img")).toHaveAttribute("src", /^data:image\//);
  await expect(card.locator(".ctx-card-name")).toHaveText("shot.png");
  await expect(card.locator(".chip-remove")).toHaveCount(0);
});

test("preloaded turn degrades an unprobed PDF to a typed card, not a path chip", async ({ page }) => {
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
            ask: "这份规范怎么说",
            context: ["docs/spec.pdf"],
            items: [],
            done: true,
            stopped: false,
            error: null,
          },
        ],
      },
    }),
  );
  await openLiveConversation(page);

  // Reload path: only the stored path is known — extension guess supplies the
  // kind/badge, and the lazy re-probe skips non-images on purpose.
  const card = page.locator('.msg-user [data-context-path="docs/spec.pdf"]');
  await expect(card).toBeVisible();
  await expect(card).toHaveClass(/ctx-card--pdf/);
  await expect(card.locator(".ctx-card-name")).toHaveText("spec.pdf");
  await expect(card.locator("img")).toHaveCount(0);
  await expect(card).toHaveAttribute("title", "docs/spec.pdf");
  await expect(page.locator(".msg-user .chip--context")).toHaveCount(0);
});

test("can attach an external absolute file from the filesystem picker", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      pickFiles: ["/Users/lijianxue/Downloads/test003_java"],
      externalFiles: ["/Users/lijianxue/Downloads/test003_java"],
    }),
  );
  await openLiveConversation(page);

  await attachFiles(page);

  await expect(page.locator('[data-context-path="/Users/lijianxue/Downloads/test003_java"]')).toBeVisible();
});

test("illegal external path is rejected and surfaces recovery", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      pickFiles: ["src/app.ts"],
      failContextRead: true,
    }),
  );
  await openLiveConversation(page);

  await attachFiles(page);
  // The listed file fails read_context_file (failContextRead), so validation
  // rejects it and the recovery banner appears instead of a context chip.
  await expect(page.locator('[data-testid="action-error"]')).toBeVisible();
  await expect(page.locator('[data-testid="action-error"]')).toContainText("无法读取");
  await expect(page.locator("[data-context-path]")).toHaveCount(0);
});

test("failed send displays a recovery action and retry works", async ({ page }) => {
  await stubShell(
    page,
    liveCore({
      failSend: true,
      files: ["src/app.ts"],
    }),
  );
  await openLiveConversation(page);

  await page.locator(".composer-input").fill("发一条会失败的消息");
  await page.locator(".composer-input").press("Enter");

  const banner = page.locator('[data-testid="action-error"]');
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("发送失败");
  await expect(page.locator('[data-testid="action-error-retry"]')).toBeVisible();

  // Flip the stub to succeed, then retry.
  await page.evaluate(() => {
    // Recreate internals: easiest is to mark failSend false on a custom flag.
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: (c: string, a?: unknown) => Promise<unknown> } }).__TAURI_INTERNALS__;
    const original = internals.invoke.bind(internals);
    internals.invoke = async (command: string, args?: unknown) => {
      if (command === "send_message") {
        const sendLog =
          ((window as unknown as { __sendLog?: unknown }).__sendLog as unknown[]) ??
          (((window as unknown as { __sendLog?: unknown }).__sendLog = []) as unknown[]);
        sendLog.push(args as never);
        return null;
      }
      return original(command, args);
    };
  });

  await page.locator('[data-testid="action-error-retry"]').click();
  await expect(banner).toHaveCount(0);
  await expect(page.locator(".reply-working")).toBeVisible();
  await page.locator(".composer-send--stop").click();
});

test("failed rename shows retry", async ({ page }) => {
  await stubShell(page, liveCore({ failRename: true }));
  await openLiveConversation(page);

  await page.locator('[aria-label="重命名会话"]').click();
  const input = page.locator(".conv-title-input");
  await input.fill("新标题");
  await input.press("Enter");

  const banner = page.locator('[data-testid="action-error"]');
  await expect(banner).toContainText("重命名失败");
  await expect(page.locator('[data-testid="action-error-retry"]')).toBeVisible();
  await page.locator('[data-testid="action-error-dismiss"]').click();
  await expect(banner).toHaveCount(0);
});

test("failed archive shows retry", async ({ page }) => {
  await stubShell(page, liveCore({ failArchive: true }));
  await openLiveConversation(page);

  await page.locator('[aria-label="会话操作"]').click();
  await page.locator(".menu-item", { hasText: "归档会话" }).click();

  const banner = page.locator('[data-testid="action-error"]');
  await expect(banner).toContainText("归档失败");
  await expect(page.locator('[data-testid="action-error-retry"]')).toBeVisible();
});

test("provider unavailable shows Settings recovery, not a dead control", async ({ page }) => {
  await stubShell(page, liveCore({ providers: [] }));
  await openLiveConversation(page);

  const warning = page.locator('[data-testid="provider-warning"]');
  await expect(warning).toBeVisible();
  await expect(warning).toContainText("尚未配置 AI Provider");
  await warning.locator("button").click();
  // Settings opens as an overlay — the same contract as the sidebar entry,
  // which must not navigate away from the conversation either.
  await expect(page.locator(".settings-modal")).toBeVisible();
  await expect(page).not.toHaveURL(/\/settings$/);
});

test("approval response failure surfaces recovery", async ({ page }) => {
  await stubShell(page, liveCore({ failApproval: true }));
  await openLiveConversation(page);

  await page.locator(".composer-input").fill("跑一下");
  await page.locator(".composer-input").press("Enter");
  await emit(page, {
    type: "approvalRequest",
    session: "s1",
    step: 1,
    kind: "run command",
    detail: "cargo test",
  });

  await page.locator(".approval-bar .btn--primary").click();
  await expect(page.locator('[data-testid="approval-error"]')).toBeVisible();
  await expect(page.locator('[data-testid="approval-error"]')).toContainText("审批响应失败");
});

/** Build a `liveCore`-shaped core with a vision-incapable model selected.
 *  Image attachments on this provider should trigger the first-time
 *  vision-skip notice. */
function liveCoreTextOnlyModel(extra: Record<string, unknown> = {}) {
  return liveCore({
    providers: [
      {
        id: "p1",
        name: "Local",
        // `template: "openai"` gives `modelId` a known catalog seed; the
        // `models` list overrides it with a text-only entry (`vision: false`)
        // so the composer's vision gate trips.
        template: "openai",
        apiKey: "••••dead",
        endpoint: "",
        modelId: "deepseek-reasoner",
        model: "deepseek-reasoner",
        displayName: "DeepSeek-R1",
        models: [{ modelId: "deepseek-reasoner", displayName: "DeepSeek-R1", vision: false }],
      },
    ],
    ...extra,
  });
}

test("vision-incapable model + image attachment shows the first-time skip notice", async ({ page }) => {
  await stubShell(
    page,
    liveCoreTextOnlyModel({
      pickFiles: ["/tmp/shot.png"],
      externalFiles: ["/tmp/shot.png"],
    }),
  );
  await openLiveConversation(page);

  await attachFiles(page);
  await expect(page.locator('[data-context-path="/tmp/shot.png"]')).toBeVisible();

  // First send attempt: the notice gates the dispatch — nothing should reach
  // the core yet, the banner is visible, and the ack button is wired.
  await page.locator(".composer-input").fill("看看这张图");
  await page.locator(".composer-input").press("Enter");

  const banner = page.locator('[data-testid="vision-skip-notice"]');
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("当前模型不支持图像");

  // No send landed yet — no assistant reply, sendLog stays empty.
  let log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string; context: string[] }> }).__sendLog ?? [],
  );
  expect(log).toHaveLength(0);

  // Ack the notice: send dispatches with the original text + image path.
  await page.locator('[data-testid="vision-skip-ack"]').click();
  await expect(banner).toHaveCount(0);
  log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string; context: string[] }> }).__sendLog ?? [],
  );
  expect(log).toHaveLength(1);
  expect(log[0].text).toBe("看看这张图");
  expect(log[0].context).toEqual(["/tmp/shot.png"]);

  // sessionStorage flag persisted for the page session.
  const acked = await page.evaluate(() =>
    window.sessionStorage.getItem("kodo:vision-img-skip-acked"),
  );
  expect(acked).toBe("1");
});

test("vision-skip notice stays silent once acked (same session)", async ({ page }) => {
  // sessionStorage is per-origin and survives the navigation to "/"; set it
  // before the React tree mounts so the initial state picks up the flag.
  await page.addInitScript(() => {
    window.sessionStorage.setItem("kodo:vision-img-skip-acked", "1");
  });
  await stubShell(
    page,
    liveCoreTextOnlyModel({
      pickFiles: ["/tmp/shot.png"],
      externalFiles: ["/tmp/shot.png"],
    }),
  );
  await openLiveConversation(page);

  await attachFiles(page);
  await page.locator(".composer-input").fill("看图");
  await page.locator(".composer-input").press("Enter");

  // Banner never appears; send goes straight through.
  await expect(page.locator('[data-testid="vision-skip-notice"]')).toHaveCount(0);
  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string; context: string[] }> }).__sendLog ?? [],
  );
  expect(log).toHaveLength(1);
  expect(log[0].text).toBe("看图");
  expect(log[0].context).toEqual(["/tmp/shot.png"]);
});

test("vision-skip notice does not fire on text-only attachments (e.g. .ts files)", async ({ page }) => {
  await stubShell(page, liveCoreTextOnlyModel({ pickFiles: ["src/app.ts"] }));
  await openLiveConversation(page);

  await attachFiles(page);
  await page.locator(".composer-input").fill("看这个文件");
  await page.locator(".composer-input").press("Enter");

  // The vision-skip gate keys on `kindFromPath(path) === "image"`, so
  // text files sail through even with a text-only model selected.
  await expect(page.locator('[data-testid="vision-skip-notice"]')).toHaveCount(0);
  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string; context: string[] }> }).__sendLog ?? [],
  );
  expect(log).toHaveLength(1);
  expect(log[0].context).toEqual(["src/app.ts"]);
});

test("vision-skip notice cancel keeps the draft in the composer", async ({ page }) => {
  await stubShell(
    page,
    liveCoreTextOnlyModel({
      pickFiles: ["/tmp/shot.png"],
      externalFiles: ["/tmp/shot.png"],
    }),
  );
  await openLiveConversation(page);

  await attachFiles(page);
  await page.locator(".composer-input").fill("看看");
  await page.locator(".composer-input").press("Enter");

  const banner = page.locator('[data-testid="vision-skip-notice"]');
  await expect(banner).toBeVisible();

  // Cancel: banner closes, no send dispatches, the draft + chip stay put so
  // the user can re-trigger (e.g. after switching to a vision-capable model).
  await page.locator('[data-testid="vision-skip-cancel"]').click();
  await expect(banner).toHaveCount(0);
  await expect(page.locator('[data-context-path="/tmp/shot.png"]')).toBeVisible();
  await expect(page.locator(".composer-input")).toHaveValue("看看");

  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string; context: string[] }> }).__sendLog ?? [],
  );
  expect(log).toHaveLength(0);

  // Cancel must not have armed the ack flag — the next send should still see
  // the banner (the user didn't actually choose to remember).
  const acked = await page.evaluate(() =>
    window.sessionStorage.getItem("kodo:vision-img-skip-acked"),
  );
  expect(acked).not.toBe("1");
});
