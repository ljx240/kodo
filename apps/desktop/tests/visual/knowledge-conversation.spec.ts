import { expect, test } from "@playwright/test";

async function prepareApproval(page: import("@playwright/test").Page) {
  await page.goto("/conversation?knowledge-fixture=approval");
  await expect(page.getByTestId("knowledge-binding")).toBeVisible();
  await page.getByTestId("knowledge-binding-toggle").click();
  await page.getByTestId("knowledge-bind-kb-personal-demo").check();
  const input = page.getByTestId("composer-input");
  await input.fill("旧客户端");
  await input.press("Enter");
  await expect(page.getByTestId("knowledge-approval")).toBeVisible();
  return input;
}

async function installSendCapture(page: import("@playwright/test").Page) {
  await page.evaluate(() => {
    type InvokeArgs = Record<string, unknown>;
    type Internals = {
      transformCallback?: (callback: (event: unknown) => void) => number;
      invoke: (command: string, args?: InvokeArgs) => Promise<unknown>;
    };
    const target = window as unknown as {
      __TAURI_INTERNALS__?: Internals;
      __knowledgeIpcCalls?: Array<{ command: string; args: InvokeArgs }>;
    };
    const calls: Array<{ command: string; args: InvokeArgs }> = [];
    target.__knowledgeIpcCalls = calls;
    target.__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      invoke: async (command, args = {}) => {
        calls.push({ command, args });
        if (command === "send_message") return "knowledge-approved-session";
        if (command === "knowledge_list_bases") {
          return args.projectPath === null
            ? [{ id: "kb-personal-demo", name: "个人资料", description: "", projectPath: null, createdAt: 0, version: 1 }]
            : [];
        }
        if (command === "knowledge_get_bindings") return { baseIds: ["kb-personal-demo"], strict: true };
        if (command === "knowledge_turn_citations") return [];
        if (command === "plugin:event|listen") return 1;
        if (command === "plugin:event|unlisten") return null;
        return null;
      },
    };
  });
}

test("发送前审批可用 Escape 取消且不会创建空轮次", async ({ page }) => {
  const input = await prepareApproval(page);
  await expect(page.getByTestId("knowledge-approval-hits")).toContainText("旧客户端");
  await expect(page.getByTestId("knowledge-approval-hits")).toContainText("第 3–5 行");
  await page.keyboard.press("Escape");

  await expect(page.getByTestId("knowledge-approval")).toHaveCount(0);
  await expect(page.locator(".msg-bubble")).toHaveCount(0);
  await expect(input).toBeVisible();
  await expect(input).toHaveValue("旧客户端");
});

test("批准后 send_message 才收到预览 token 和明确批准标记", async ({ page }) => {
  await prepareApproval(page);
  await installSendCapture(page);
  await page.getByTestId("knowledge-approval-confirm").click();

  await expect(page.locator(".msg-bubble")).toContainText("旧客户端");
  await expect.poll(() =>
    page.evaluate(() => {
      const calls = (window as unknown as {
        __knowledgeIpcCalls?: Array<{ command: string; args: Record<string, unknown> }>;
      }).__knowledgeIpcCalls ?? [];
      return calls.filter((call) => call.command === "send_message").length;
    }),
  ).toBe(1);
  const send = await page.evaluate(() =>
    (window as unknown as {
      __knowledgeIpcCalls: Array<{ command: string; args: Record<string, unknown> }>;
    }).__knowledgeIpcCalls.find((call) => call.command === "send_message"),
  );
  expect(send?.args.knowledgeToken).toEqual(expect.any(String));
  expect(send?.args.knowledgeRemoteApproved).toBe(true);
});

test("已记录轮次只链接真实引用并可查看原文与未核实地提升回答", async ({ page }) => {
  await page.goto("/ui-demo/conversation?knowledge-fixture=citations");
  const citations = page.getByTestId("knowledge-citations-1");
  await expect(citations).toContainText("[K1] 接口兼容约定.md");
  await expect(page.getByTestId("knowledge-citation-1-2")).toHaveCount(0);

  await page.getByTestId("knowledge-turn-promote-1").click();
  await expect(page.getByRole("dialog", { name: "加入知识库" })).toBeVisible();
  await expect(page.getByTestId("knowledge-save-verified")).not.toBeChecked();
  await expect(page.getByRole("dialog", { name: "加入知识库" })).toContainText("来源：会话回答");
  await page.getByTestId("knowledge-save-scope").selectOption("/demo/kodo");
  await expect(page.getByTestId("knowledge-save-base").locator("option")).toHaveCount(1);
  await expect(page.getByTestId("knowledge-save-base")).toHaveValue("kb-kodo-demo");
  await page.getByTestId("knowledge-save-close").click();
  await expect(page.getByRole("dialog", { name: "加入知识库" })).toHaveCount(0);

  await page.getByTestId("knowledge-citation-1-1").click();
  const source = page.getByTestId("knowledge-source-dialog");
  await expect(source).toContainText("旧客户端");
  await expect(source).toContainText("第 3–5 行");
  await expect(page.getByTestId("knowledge-source-snapshot")).toContainText("保留旧客户端仍在使用的字段");
});

test("知识库停用时普通发送继续走无资料路径", async ({ page }) => {
  await page.goto("/conversation?knowledge-fixture=disabled");
  await expect(page.getByTestId("knowledge-binding-error")).toContainText("知识库已停用，本轮不使用资料");
  const input = page.getByTestId("composer-input");
  await input.fill("不使用资料的普通问题");
  await expect(page.getByTestId("composer-send")).toBeEnabled();
  await installSendCapture(page);
  await page.getByTestId("composer-send").click();

  await expect(page.locator(".msg-bubble")).toContainText("不使用资料的普通问题");
  const send = await page.evaluate(() =>
    (window as unknown as {
      __knowledgeIpcCalls: Array<{ command: string; args: Record<string, unknown> }>;
    }).__knowledgeIpcCalls.find((call) => call.command === "send_message"),
  );
  expect(send?.args).not.toHaveProperty("knowledgeToken");
  expect(send?.args).not.toHaveProperty("knowledgeRemoteApproved");
});

test("知识库读取失败先阻止发送，明确选择无资料后可继续", async ({ page }) => {
  await page.goto("/conversation?knowledge-fixture=unavailable");
  await expect(page.getByTestId("knowledge-binding-error")).toContainText("知识库暂时不可用");
  const input = page.getByTestId("composer-input");
  await input.fill("明确选择无资料后的普通问题");
  await expect(page.getByTestId("composer-send")).toBeDisabled();
  await page.getByTestId("knowledge-continue-without").click();
  await expect(page.getByTestId("composer-send")).toBeEnabled();
  await installSendCapture(page);
  await page.getByTestId("composer-send").click();

  await expect(page.locator(".msg-bubble")).toContainText("明确选择无资料后的普通问题");
  const calls = await page.evaluate(() =>
    (window as unknown as {
      __knowledgeIpcCalls: Array<{ command: string; args: Record<string, unknown> }>;
    }).__knowledgeIpcCalls,
  );
  const send = calls.find((call) => call.command === "send_message");
  expect(calls.some((call) => call.command === "knowledge_prepare_turn")).toBe(false);
  expect(send?.args).not.toHaveProperty("knowledgeToken");
  expect(send?.args).not.toHaveProperty("knowledgeRemoteApproved");
});

test("来源检查被拒绝时不展示缓存片段", async ({ page }) => {
  await page.goto("/ui-demo/conversation?knowledge-fixture=citations&knowledge-inspect-error=scope_denied");
  await page.getByTestId("knowledge-citation-1-1").click();
  await expect(page.getByTestId("knowledge-source-dialog").getByRole("alert")).toContainText("当前项目范围无法访问这条资料");
  await expect(page.getByTestId("knowledge-source-quote")).toHaveCount(0);
});
