import { expect, test } from "@playwright/test";
import { DEMO_RESULTS, DEMO_WORKBENCH_APPS } from "../../src/data/workbench";

type MockIpcCall = { id: number; command: string; args: Record<string, unknown> };

const racePersonalBase = {
  id: "kb-personal-race",
  name: "个人库 A",
  description: "",
  projectPath: null,
  createdAt: 1,
  version: 1,
};
const raceProjectBase = {
  id: "kb-project-race",
  name: "项目库 B",
  description: "",
  projectPath: "/demo/kodo",
  createdAt: 1,
  version: 1,
};
const racePersonalDocument = {
  id: "doc-personal-race",
  baseId: racePersonalBase.id,
  title: "A 范围私有资料.md",
  sourceKind: "file",
  sourceRef: "A 范围私有资料.md",
  status: "ready",
  verified: false,
  version: 1,
  revisionId: "rev-personal-race",
  contentHash: "hash-personal-race",
  chunkCount: 1,
  createdAt: 1,
  updatedAt: 1,
};

async function installDelayedKnowledgeIpc(
  page: import("@playwright/test").Page,
  options: { projectBaseEmpty?: boolean } = {},
) {
  await page.addInitScript(
    ({ projectBaseEmpty, apps, results, personalBase, projectBase, personalDocument }) => {
      type Call = { id: number; command: string; args: Record<string, unknown> };
      type Pending = { command: string; resolve: (value: unknown) => void };
      type MockGlobal = {
        __TAURI_INTERNALS__?: {
          invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
          transformCallback: (callback: (value?: unknown) => void) => number;
          unregisterCallback: (id: number) => void;
        };
        __knowledgeIpc?: {
          calls: Call[];
          pendingIds: () => number[];
          resolve: (id: number, value: unknown) => void;
        };
      };
      const target = window as unknown as MockGlobal;
      const calls: Call[] = [];
      const pending = new Map<number, Pending>();
      const callbackIds = new Map<number, (value?: unknown) => void>();
      let nextId = 1;
      let createdBase: typeof projectBase | null = null;

      const resolve = (id: number, value: unknown) => {
        const item = pending.get(id);
        if (!item) throw new Error(`No pending mock IPC call ${id}`);
        if (item.command === "knowledge_create_base") createdBase = value as typeof projectBase;
        pending.delete(id);
        item.resolve(value);
      };

      target.__knowledgeIpc = { calls, pendingIds: () => [...pending.keys()], resolve };
      target.__TAURI_INTERNALS__ = {
        transformCallback: (callback) => {
          const id = nextId++;
          callbackIds.set(id, callback);
          return id;
        },
        unregisterCallback: (id) => callbackIds.delete(id),
        invoke: (command, args = {}) => {
          const id = nextId++;
          calls.push({ id, command, args });
          if (command === "workspace") {
            return Promise.resolve({ projects: [{ path: "/demo/kodo", name: "Kodo" }], sessions: [] });
          }
          if (command === "list_apps") return Promise.resolve(apps);
          if (command === "list_results") return Promise.resolve(results);
          if (command === "get_workbench_prefs") return Promise.resolve({ "workbench.showToday": "false" });
          if (command === "list_todos" || command === "list_calendar_events") return Promise.resolve([]);
          if (command === "knowledge_list_bases") {
            const projectRows = createdBase ? [createdBase] : projectBaseEmpty ? [] : [projectBase];
            return Promise.resolve(args.projectPath == null ? [personalBase] : [personalBase, ...projectRows]);
          }
          if (command === "knowledge_list_documents") {
            return Promise.resolve(args.baseId === personalBase.id ? [personalDocument] : []);
          }
          if (command.startsWith("plugin:event|")) return Promise.resolve(1);
          if (
            command === "knowledge_inspect_document" ||
            command === "knowledge_search" ||
            command === "knowledge_pick_import" ||
            command === "knowledge_confirm_import" ||
            command === "knowledge_create_base" ||
            command === "knowledge_import_text"
          ) {
            return new Promise((resolveCall) => pending.set(id, { command, resolve: resolveCall }));
          }
          return Promise.resolve(null);
        },
      };
    },
    {
      projectBaseEmpty: options.projectBaseEmpty ?? false,
      apps: DEMO_WORKBENCH_APPS,
      results: DEMO_RESULTS,
      personalBase: racePersonalBase,
      projectBase: raceProjectBase,
      personalDocument: racePersonalDocument,
    },
  );
}

async function callsFor(page: import("@playwright/test").Page, command: string): Promise<MockIpcCall[]> {
  return page.evaluate((name) => {
    const state = (window as unknown as { __knowledgeIpc?: { calls: MockIpcCall[] } }).__knowledgeIpc;
    return (state?.calls ?? []).filter((call) => call.command === name);
  }, command);
}

async function resolveIpcCall(
  page: import("@playwright/test").Page,
  id: number,
  value: unknown,
) {
  await page.evaluate(({ callId, result }) => {
    const state = (window as unknown as {
      __knowledgeIpc?: { resolve: (requestId: number, response: unknown) => void };
    }).__knowledgeIpc;
    state?.resolve(callId, result);
  }, { callId: id, result: value });
}

const goTo = (page: import("@playwright/test").Page, url: string) =>
  page.evaluate((to) => {
    window.history.pushState(null, "", to);
    window.dispatchEvent(new PopStateEvent("popstate"));
  }, url);

test("资料详情、关键词检索和停用状态使用当前快照", async ({ page }) => {
  await page.goto("/ui-demo/workbench?app=knowledge");
  await expect(page.getByTestId("knowledge-app")).toBeVisible();
  await expect(page.getByTestId("knowledge-base")).toHaveValue("kb-personal-demo");
  await expect(page.locator(".knowledge-row")).toHaveCount(2);

  await page.getByTestId("knowledge-open-doc-compat-demo").click();
  await expect(page.getByTestId("knowledge-detail")).toContainText("接口兼容约定.md");
  await expect(page.getByTestId("knowledge-source-lines")).toContainText("旧客户端");
  await expect(page.getByTestId("knowledge-detail")).toContainText("第 3–5 行");

  await page.getByTestId("knowledge-tab-search").click();
  await page.getByTestId("knowledge-search-input").fill("旧客户端");
  await page.getByTestId("knowledge-search-submit").click();
  await expect(page.getByTestId("knowledge-search-results")).toContainText("接口兼容约定.md");
  await expect(page.locator(".knowledge-keyword-tag")).toHaveCount(2);
  await expect(page.locator(".knowledge-keyword-tag").first()).toHaveText("关键词");
  await expect(page.locator(".knowledge-hit blockquote").first()).toContainText("保留旧客户端仍在使用的字段");

  await page.getByTestId("knowledge-tab-documents").click();
  await page.getByTestId("knowledge-toggle-doc-compat-demo").click();
  await expect(page.getByTestId("knowledge-document-doc-compat-demo")).toContainText("已停用");
  await page.getByTestId("knowledge-tab-search").click();
  await page.getByTestId("knowledge-search-submit").click();
  await expect(page.getByTestId("knowledge-search-empty")).toBeVisible();
});

test("项目范围列出个人与项目库且管理列表只显示所选范围", async ({ page }) => {
  await page.goto("/ui-demo/workbench?app=knowledge");
  const available = await page.evaluate(async () => {
    const moduleUrl = new URL("/src/knowledgeApi.ts", window.location.origin).href;
    const api = await import(/* @vite-ignore */ moduleUrl) as {
      knowledgeListBases: (input: { projectPath: string | null }) => Promise<Array<{ id: string }>>;
    };
    return (await api.knowledgeListBases({ projectPath: "/demo/kodo" })).map((base) => base.id);
  });
  expect(available).toEqual(["kb-personal-demo", "kb-kodo-demo"]);
  expect(new Set(available).size).toBe(2);

  const scope = page.getByTestId("knowledge-scope");
  const baseSelect = page.getByTestId("knowledge-base");
  await scope.selectOption("/demo/kodo");
  await expect(baseSelect).toHaveValue("kb-kodo-demo");
  await expect(baseSelect.locator("option")).toHaveCount(1);
  await scope.selectOption("__personal__");
  await expect(baseSelect).toHaveValue("kb-personal-demo");
  await expect(baseSelect.locator("option")).toHaveCount(1);
});

test("建库、笔记、原生导入预览和删除形成可管理流程", async ({ page }) => {
  await page.goto("/ui-demo/workbench?app=knowledge");
  await page.getByTestId("knowledge-new-base").click();
  await page.getByTestId("knowledge-create-name").fill("验证资料");
  await expect(page.getByTestId("knowledge-create-scope")).toHaveValue("__personal__");
  await page.getByTestId("knowledge-create-submit").click();
  await expect(page.getByTestId("knowledge-base")).toContainText("验证资料");
  await expect(page.getByTestId("knowledge-base")).toHaveValue(/kb-demo-/);

  await page.getByTestId("knowledge-import-note").click();
  await page.getByTestId("knowledge-note-title-input").fill("验证笔记");
  await page.getByTestId("knowledge-note-body").fill("# 检查\n\n确认原文快照与行号可以查看。");
  await page.getByTestId("knowledge-note-submit").click();
  const noteRow = page.locator(".knowledge-row").filter({ hasText: "验证笔记" });
  await expect(noteRow).toBeVisible();
  await noteRow.locator(".knowledge-row-open").click();
  await expect(page.getByTestId("knowledge-source-lines")).toContainText("原文快照与行号");

  await page.getByTestId("knowledge-import-file").click();
  await expect(page.getByTestId("knowledge-import-preview")).toBeVisible();
  await expect(page.getByTestId("knowledge-import-preview")).toContainText("接口验收清单.md");
  await page.getByRole("button", { name: "确认导入此快照" }).click();
  await expect(page.getByTestId("knowledge-import-preview")).toHaveCount(0);
  await expect(page.locator(".knowledge-row").filter({ hasText: "接口验收清单.md" })).toBeVisible();

  const note = page.locator(".knowledge-row").filter({ hasText: "验证笔记" });
  await note.getByRole("button", { name: "删除 验证笔记" }).click();
  await note.getByRole("button", { name: "确认删除" }).click();
  await expect(page.locator(".knowledge-row").filter({ hasText: "验证笔记" })).toHaveCount(0);
});

test("成果中心只提升文本；文件引用说明不读取原路径", async ({ page }) => {
  await page.goto("/ui-demo/workbench?app=artifacts");
  await page.getByTestId("result-toggle-result-1").click();
  await page.getByTestId("result-to-knowledge-result-1").click();
  await expect(page.getByRole("dialog", { name: "加入知识库" })).toBeVisible();
  await page.getByTestId("knowledge-save-confirm").click();
  await expect(page.getByRole("dialog", { name: "加入知识库" })).toHaveCount(0);

  await page.getByTestId("result-toggle-result-2").click();
  await expect(page.getByTestId("result-file-to-knowledge-result-2")).toContainText("不会读取该路径");
  await expect(page.getByTestId("result-to-knowledge-result-2")).toHaveCount(0);

  await goTo(page, "/ui-demo/workbench?app=knowledge");
  await expect(page.locator(".knowledge-row").filter({ hasText: "k2k-rust 路由排查结论" })).toBeVisible();
});

test("范围切换后迟到的详情和检索不会显示 A 范围内容", async ({ page }) => {
  await installDelayedKnowledgeIpc(page);
  await page.goto("/ui-demo/workbench?app=knowledge");
  await expect(page.getByTestId("knowledge-base")).toHaveValue(racePersonalBase.id);

  await page.getByTestId(`knowledge-open-${racePersonalDocument.id}`).click();
  await expect.poll(async () => (await callsFor(page, "knowledge_inspect_document")).length).toBe(1);
  const detailCall = (await callsFor(page, "knowledge_inspect_document"))[0];
  await page.getByTestId("knowledge-scope").selectOption("/demo/kodo");
  await expect(page.getByTestId("knowledge-base")).toHaveValue(raceProjectBase.id);
  await resolveIpcCall(page, detailCall.id, {
    document: racePersonalDocument,
    body: "A 范围正文绝不能出现在 B 页面",
    chunks: [{ id: "chunk-personal-race", revisionId: "rev-personal-race", heading: "A 范围标题", lineStart: 1, lineEnd: 1, body: "A 范围正文绝不能出现在 B 页面" }],
  });
  await expect(page.getByTestId("knowledge-detail")).toHaveCount(0);
  await expect(page.getByText("A 范围正文绝不能出现在 B 页面")).toHaveCount(0);

  await page.getByTestId("knowledge-scope").selectOption("__personal__");
  await expect(page.getByTestId("knowledge-base")).toHaveValue(racePersonalBase.id);
  await page.getByTestId("knowledge-tab-search").click();
  await page.getByTestId("knowledge-search-input").fill("A 专属检索词");
  await page.getByTestId("knowledge-search-submit").click();
  await expect.poll(async () => (await callsFor(page, "knowledge_search")).length).toBe(1);
  const searchCall = (await callsFor(page, "knowledge_search"))[0];
  await page.getByTestId("knowledge-scope").selectOption("/demo/kodo");
  await expect(page.locator(".knowledge-scope-path")).toHaveText("/demo/kodo");
  await resolveIpcCall(page, searchCall.id, [{
    baseId: racePersonalBase.id,
    documentId: racePersonalDocument.id,
    revisionId: "rev-personal-race",
    chunkId: "chunk-personal-race",
    title: "A 范围检索命中",
    heading: "A 范围标题",
    lineStart: 1,
    lineEnd: 1,
    quote: "A 范围命中正文不能显示在 B 页面",
    channels: ["fts", "exact"],
  }]);
  await expect(page.getByTestId("knowledge-search-results")).toHaveCount(0);
  await expect(page.getByText("A 范围命中正文不能显示在 B 页面")).toHaveCount(0);
});

test("切换范围后迟到的文件预览失效，旧令牌的迟到确认不刷新新范围", async ({ page }) => {
  await installDelayedKnowledgeIpc(page);
  await page.goto("/ui-demo/workbench?app=knowledge");
  await expect(page.getByTestId("knowledge-base")).toHaveValue(racePersonalBase.id);

  await page.getByTestId("knowledge-import-file").click();
  await expect.poll(async () => (await callsFor(page, "knowledge_pick_import")).length).toBe(1);
  const stalePicker = (await callsFor(page, "knowledge_pick_import"))[0];
  await page.getByTestId("knowledge-scope").selectOption("/demo/kodo");
  await expect(page.getByTestId("knowledge-base")).toHaveValue(raceProjectBase.id);
  await resolveIpcCall(page, stalePicker.id, [{
    token: "token-personal-stale",
    title: "A 范围待确认文件.md",
    bodyPreview: "A 范围预览内容",
    characterCount: 8,
    chunkCount: 1,
  }]);
  await expect(page.getByTestId("knowledge-import-preview")).toHaveCount(0);
  await expect(page.getByTestId("knowledge-confirm-import-token-personal-stale")).toHaveCount(0);
  expect(await callsFor(page, "knowledge_confirm_import")).toHaveLength(0);

  await page.getByTestId("knowledge-scope").selectOption("__personal__");
  await expect(page.getByTestId("knowledge-base")).toHaveValue(racePersonalBase.id);
  await page.getByTestId("knowledge-import-file").click();
  await expect.poll(async () => (await callsFor(page, "knowledge_pick_import")).length).toBe(2);
  const currentPicker = (await callsFor(page, "knowledge_pick_import"))[1];
  await resolveIpcCall(page, currentPicker.id, [{
    token: "token-personal-confirming",
    title: "A 范围确认文件.md",
    bodyPreview: "等待确认的快照",
    characterCount: 8,
    chunkCount: 1,
  }]);
  await expect(page.getByTestId("knowledge-confirm-import-token-personal-confirming")).toBeVisible();
  await page.getByTestId("knowledge-confirm-import-token-personal-confirming").click();
  await expect.poll(async () => (await callsFor(page, "knowledge_confirm_import")).length).toBe(1);
  const confirmCall = (await callsFor(page, "knowledge_confirm_import"))[0];
  await page.getByTestId("knowledge-scope").selectOption("/demo/kodo");
  await expect(page.getByTestId("knowledge-import-preview")).toHaveCount(0);
  await expect(page.getByTestId("knowledge-base")).toHaveValue(raceProjectBase.id);
  const projectDocumentReads = (await callsFor(page, "knowledge_list_documents"))
    .filter((call) => call.args.baseId === raceProjectBase.id).length;
  await resolveIpcCall(page, confirmCall.id, { ...racePersonalDocument, id: "doc-confirmed-late" });
  await page.evaluate(() => new Promise((resolve) => window.setTimeout(resolve, 0)));
  await expect(page.getByTestId("knowledge-import-preview")).toHaveCount(0);
  expect((await callsFor(page, "knowledge_list_documents")).filter((call) => call.args.baseId === raceProjectBase.id)).toHaveLength(projectDocumentReads);
  expect(await callsFor(page, "knowledge_confirm_import")).toHaveLength(1);
});

test("成果保存创建期间锁定范围，创建与保存都留在选定项目库", async ({ page }) => {
  await installDelayedKnowledgeIpc(page, { projectBaseEmpty: true });
  await page.goto("/ui-demo/workbench?app=artifacts");
  await page.getByTestId("result-toggle-result-1").click();
  await page.getByTestId("result-to-knowledge-result-1").click();
  const scope = page.getByTestId("knowledge-save-scope");
  await expect(scope).toHaveValue("__personal__");
  await scope.selectOption("/demo/kodo");
  await expect(page.getByText("先在当前范围创建一个知识库，再保存这份文本。")).toBeVisible();
  await page.getByRole("button", { name: "新建知识库" }).click();
  await page.getByTestId("knowledge-save-new-base-name").fill("项目成果资料");
  await page.getByRole("button", { name: "创建知识库" }).click();
  await expect.poll(async () => (await callsFor(page, "knowledge_create_base")).length).toBe(1);
  const createCall = (await callsFor(page, "knowledge_create_base"))[0];
  expect(createCall.args).toMatchObject({ draft: { projectPath: "/demo/kodo", name: "项目成果资料" } });
  await expect(scope).toBeDisabled();

  const createdBase = { ...raceProjectBase, id: "kb-created-save", name: "项目成果资料" };
  await resolveIpcCall(page, createCall.id, createdBase);
  await expect(page.getByTestId("knowledge-save-base")).toHaveValue(createdBase.id);
  await expect(scope).toHaveValue("/demo/kodo");
  await page.getByTestId("knowledge-save-confirm").click();
  await expect.poll(async () => (await callsFor(page, "knowledge_import_text")).length).toBe(1);
  const saveCall = (await callsFor(page, "knowledge_import_text"))[0];
  expect(saveCall.args).toMatchObject({ baseId: createdBase.id, projectPath: "/demo/kodo" });
  await expect(scope).toBeDisabled();
  await resolveIpcCall(page, saveCall.id, {
    ...racePersonalDocument,
    id: "doc-saved-project-result",
    baseId: createdBase.id,
    title: "成果文本",
    sourceKind: "result",
    sourceRef: "result-1",
  });
  await expect(page.getByRole("dialog", { name: "加入知识库" })).toHaveCount(0);
  expect(await callsFor(page, "send_message")).toHaveLength(0);
});

test("窄窗口下不横向溢出，笔记表单可键盘操作", async ({ page }) => {
  await page.setViewportSize({ width: 860, height: 760 });
  await page.goto("/ui-demo/workbench?app=knowledge");
  await expect(page.getByTestId("knowledge-app")).toBeVisible();
  const dimensions = await page.getByTestId("knowledge-app").evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth + 1);

  await page.getByTestId("knowledge-import-note").click();
  await expect(page.getByTestId("knowledge-note-title-input")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("knowledge-note-dialog")).toHaveCount(0);
});
