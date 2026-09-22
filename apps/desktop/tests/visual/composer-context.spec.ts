import { expect, test } from "@playwright/test";
import { emit, project, sessionRef, stubShell } from "./shell";

/** Open the + menu and trigger "添加照片和文件" (OS / stubbed filesystem picker). */
async function attachFiles(page: import("@playwright/test").Page) {
  await page.locator('.composer button[aria-label="Composer menu"]').click();
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
  await expect(page.locator(".conv-title")).toHaveText("修复路由");
}

test("no decorative Mic button remains in the composer", async ({ page }) => {
  await stubShell(page, liveCore());
  await openLiveConversation(page);
  await expect(page.locator('.composer button[aria-label="Voice input"]')).toHaveCount(0);
  await expect(page.locator('.composer button[aria-label="Composer menu"]')).toHaveCount(1);
});

test("+ opens a menu, not an inline file list", async ({ page }) => {
  await stubShell(page, liveCore({ files: ["src/app.ts"] }));
  await openLiveConversation(page);

  await page.locator('.composer button[aria-label="Composer menu"]').click();
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

  await page.locator('.composer button[aria-label="Composer menu"]').click();
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

  await page.locator('.composer button[aria-label="Composer menu"]').click();
  await page.locator(".composer-plus-menu", { hasText: "技能" }).getByRole("menuitem", { name: "技能" }).click();
  await page.locator('[data-skill-id="bug-fix"]').click();

  await expect(page.locator(".composer-input")).toHaveValue("");
  await expect(page.locator('.composer-skill[data-skill-id="bug-fix"]')).toContainText("bug-fix");
  await expect(page.locator('[aria-label="Remove skill bug-fix"]')).toBeVisible();
  await page.locator('[aria-label="Remove skill bug-fix"]').click();
  await expect(page.locator(".composer-skill")).toHaveCount(0);

  await page.locator('.composer button[aria-label="Composer menu"]').click();
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
  await page.locator('[aria-label="Remove context src/app.ts"]').click();
  await expect(page.locator('[data-context-path="src/app.ts"]')).toHaveCount(0);
  await expect(page.locator('[data-context-path="README.md"]')).toBeVisible();
});

test("send passes structured context in the invoke payload, not in the ask text", async ({ page }) => {
  await stubShell(page, liveCore({ pickFiles: ["src/app.ts"] }));
  await openLiveConversation(page);

  await attachFiles(page);

  await page.locator(".composer-input").fill("看看这个文件");
  await page.locator(".composer-input").press("Enter");

  await expect(page.locator(".msg-bubble")).toHaveText("看看这个文件");
  await expect(page.locator(".msg-bubble")).not.toContainText("src/app.ts");
  await expect(page.locator('[data-testid="msg-contexts"] [data-context-path], .msg-contexts .chip')).toContainText(
    "src/app.ts",
  );
  // chips in the sent message are static (not removable after send)
  await expect(page.locator(".msg-user .chip--context")).toHaveCount(1);

  const log = await page.evaluate(
    () => (window as unknown as { __sendLog?: Array<{ text: string; context: string[] }> }).__sendLog ?? [],
  );
  expect(log).toHaveLength(1);
  expect(log[0].text).toBe("看看这个文件");
  expect(log[0].context).toEqual(["src/app.ts"]);
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

  await page.locator('[aria-label="Rename conversation"]').click();
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

  await page.locator('[aria-label="Conversation actions"]').click();
  await page.locator(".menu-item", { hasText: "Archive conversation" }).click();

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
