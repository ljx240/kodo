import { invoke } from "@tauri-apps/api/core";
import { isDesktop } from "./api";
import {
  DEMO_KNOWLEDGE_BASES,
  DEMO_CONVERSATION_KNOWLEDGE_HITS,
  DEMO_KNOWLEDGE_DETAILS,
  DEMO_KNOWLEDGE_DOCUMENTS,
  DEMO_KNOWLEDGE_FILE_PREVIEW,
} from "./data/knowledge";

export type KnowledgeBase = {
  id: string;
  name: string;
  description: string;
  projectPath: string | null;
  createdAt: number;
  version: number;
};

export type BaseDraft = {
  name: string;
  description: string;
  projectPath: string | null;
};

export type KnowledgeSourceKind = "note" | "file" | "result" | "answer";
export type KnowledgeDocumentStatus = "ready" | "disabled" | "quarantined";

export type KnowledgeDocument = {
  id: string;
  baseId: string;
  title: string;
  sourceKind: KnowledgeSourceKind;
  sourceRef: string | null;
  status: KnowledgeDocumentStatus;
  verified: boolean;
  version: number;
  revisionId: string;
  contentHash: string;
  chunkCount: number;
  createdAt: number;
  updatedAt: number;
};

export type ImportDraft = {
  title: string;
  body: string;
  sourceKind: KnowledgeSourceKind;
  sourceRef: string | null;
  verified: boolean;
};

export type KnowledgeChunk = {
  id: string;
  revisionId: string;
  heading: string;
  lineStart: number;
  lineEnd: number;
  body: string;
};

export type DocumentDetail = {
  document: KnowledgeDocument;
  body: string;
  chunks: KnowledgeChunk[];
};

export type SearchQuery = {
  baseIds: string[];
  projectPath: string | null;
  text: string;
  limit: number;
  budgetTokens: number;
};

export type KnowledgeHit = {
  baseId: string;
  documentId: string;
  revisionId: string;
  chunkId: string;
  title: string;
  heading: string;
  lineStart: number;
  lineEnd: number;
  quote: string;
  channels: string[];
};

export type KnowledgeSelection = {
  baseIds: string[];
  strict: boolean;
};

export type TurnKnowledge = {
  sessionId: string;
  turnSeq: number;
  hits: KnowledgeHit[];
};

export type ImportPreview = {
  token: string;
  title: string;
  bodyPreview: string;
  characterCount: number;
  chunkCount: number;
};

export type PreparedTurn = {
  token: string;
  hits: KnowledgeHit[];
  remote: boolean;
  providerLabel: string | null;
  strict: boolean;
};

const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

// Browser fixtures are mutable in memory and reset on reload. Desktop calls
// always go through Tauri IPC and never fall back to these values.
let demoBases = clone(DEMO_KNOWLEDGE_BASES);
let demoDocuments = clone(DEMO_KNOWLEDGE_DOCUMENTS);
const demoBodies = new Map(
  DEMO_KNOWLEDGE_DETAILS.map((detail) => [
    detail.document.id,
    { body: detail.body, chunks: clone(detail.chunks) },
  ]),
);
const demoBindings = new Map<string, KnowledgeSelection>();
const demoPendingImports = new Map<string, { baseId: string; projectPath: string | null }>();
const demoCitationRows = new Map<string, KnowledgeHit[]>();
let demoSequence = 0;

const nextDemoId = (prefix: string) => `${prefix}-demo-${++demoSequence}`;
const demoNow = () => Math.floor(Date.parse("2026-09-30T10:24:32Z") / 1000);
const bindingKey = (sessionId: string, projectPath: string | null) =>
  `${projectPath ?? "personal"}\u0000${sessionId}`;

function fail(code: string): never {
  throw new Error(code);
}

function findBase(baseId: string, projectPath: string | null): KnowledgeBase {
  const base = demoBases.find((row) => row.id === baseId);
  if (!base) return fail("not_found");
  if (base.projectPath !== null && base.projectPath !== projectPath) return fail("scope_denied");
  return base;
}

function chunkText(body: string, revisionId: string): KnowledgeChunk[] {
  const lines = body.split("\n");
  const chunks: KnowledgeChunk[] = [];
  let heading = "正文";
  let sectionStart = 0;
  let sectionLines: string[] = [];

  const flush = (endIndex: number) => {
    if (sectionLines.length === 0) return;
    const section = sectionLines.join("\n").trim();
    if (section) {
      chunks.push({
        id: nextDemoId("chunk"),
        revisionId,
        heading,
        lineStart: sectionStart + 1,
        lineEnd: Math.max(sectionStart + 1, endIndex),
        body: section,
      });
    }
    sectionLines = [];
  };

  lines.forEach((line, index) => {
    const match = /^(#{1,6})\s+(.+)$/.exec(line);
    if (match) {
      flush(index);
      heading = match[2].trim();
      sectionStart = index;
    } else if (sectionLines.length === 0 && line.trim()) {
      sectionStart = index;
      sectionLines.push(line);
    } else {
      sectionLines.push(line);
    }
  });
  flush(lines.length);
  return chunks;
}

function addDemoDocument(
  baseId: string,
  projectPath: string | null,
  draft: ImportDraft,
): KnowledgeDocument {
  const base = findBase(baseId, projectPath);
  const title = draft.title.trim();
  const body = draft.body.trim();
  if (!title || !body) return fail("invalid_input");

  const id = nextDemoId("doc");
  const revisionId = nextDemoId("rev");
  const chunks = chunkText(body, revisionId);
  const createdAt = demoNow();
  const document: KnowledgeDocument = {
    id,
    baseId: base.id,
    title,
    sourceKind: draft.sourceKind,
    sourceRef: draft.sourceRef,
    status: "ready",
    verified: draft.verified,
    version: 1,
    revisionId,
    contentHash: `demo-sha256-${id}`,
    chunkCount: chunks.length,
    createdAt,
    updatedAt: createdAt,
  };
  demoDocuments = [document, ...demoDocuments];
  demoBodies.set(id, { body, chunks });
  return clone(document);
}

function eligibleDemoDocument(doc: KnowledgeDocument, query: SearchQuery): boolean {
  if (!query.baseIds.includes(doc.baseId) || doc.status !== "ready") return false;
  const base = demoBases.find((row) => row.id === doc.baseId);
  if (!base) return false;
  if (base.projectPath !== null && base.projectPath !== query.projectPath) return false;
  // Personal bases can cross project scope only when their ids were explicitly
  // selected in `baseIds`, which is already checked above.
  return true;
}

function searchDemo(query: SearchQuery): KnowledgeHit[] {
  const needle = query.text.trim().toLocaleLowerCase();
  if (!needle || query.limit <= 0 || query.budgetTokens <= 0) return [];
  const terms = needle.split(/\s+/).filter(Boolean);
  const hits: KnowledgeHit[] = [];
  let usedTokens = 0;

  for (const document of demoDocuments) {
    if (!eligibleDemoDocument(document, query)) continue;
    const detail = demoBodies.get(document.id);
    if (!detail) continue;
    const matching = detail.chunks.filter((chunk) => {
      const haystack = `${document.title}\n${chunk.heading}\n${chunk.body}`.toLocaleLowerCase();
      return terms.some((term) => haystack.includes(term));
    });
    for (const chunk of matching) {
      const tokens = Math.ceil(chunk.body.length / 4);
      if (usedTokens + tokens > query.budgetTokens) continue;
      hits.push({
        baseId: document.baseId,
        documentId: document.id,
        revisionId: chunk.revisionId,
        chunkId: chunk.id,
        title: document.title,
        heading: chunk.heading,
        lineStart: chunk.lineStart,
        lineEnd: chunk.lineEnd,
        quote: chunk.body,
        channels: ["fts", "exact"],
      });
      usedTokens += tokens;
      if (hits.length >= query.limit) return hits;
    }
  }
  return hits;
}

export function knowledgeListBases(input: { projectPath: string | null }): Promise<KnowledgeBase[]> {
  if (!isDesktop()) {
    const fixture = new URLSearchParams(window.location.search).get("knowledge-fixture");
    if (fixture === "disabled") return Promise.reject(new Error("app_disabled"));
    if (fixture === "unavailable") return Promise.reject(new Error("storage_unavailable"));
    return Promise.resolve(
      clone(demoBases.filter((base) => base.projectPath === null || base.projectPath === input.projectPath)),
    );
  }
  return invoke<KnowledgeBase[]>("knowledge_list_bases", input);
}

export function knowledgeCreateBase(input: { draft: BaseDraft }): Promise<KnowledgeBase> {
  if (!isDesktop()) {
    const name = input.draft.name.trim();
    if (!name) return Promise.reject(new Error("invalid_input"));
    const base: KnowledgeBase = {
      id: nextDemoId("kb"),
      name,
      description: input.draft.description.trim(),
      projectPath: input.draft.projectPath,
      createdAt: demoNow(),
      version: 1,
    };
    demoBases = [...demoBases, base];
    return Promise.resolve(clone(base));
  }
  return invoke<KnowledgeBase>("knowledge_create_base", input);
}

export function knowledgeListDocuments(input: {
  baseId: string;
  projectPath: string | null;
}): Promise<KnowledgeDocument[]> {
  if (!isDesktop()) {
    try {
      findBase(input.baseId, input.projectPath);
      return Promise.resolve(clone(demoDocuments.filter((doc) => doc.baseId === input.baseId)));
    } catch (failure) {
      return Promise.reject(failure);
    }
  }
  return invoke<KnowledgeDocument[]>("knowledge_list_documents", input);
}

export function knowledgeInspectDocument(input: {
  id: string;
  projectPath: string | null;
}): Promise<DocumentDetail> {
  if (!isDesktop()) {
    const inspectError = new URLSearchParams(window.location.search).get("knowledge-inspect-error");
    if (inspectError === "not_found" || inspectError === "scope_denied") {
      return Promise.reject(new Error(inspectError));
    }
    try {
      const document = demoDocuments.find((row) => row.id === input.id);
      if (!document) return Promise.reject(new Error("not_found"));
      const base = findBase(document.baseId, input.projectPath);
      const snapshot = demoBodies.get(document.id);
      if (!snapshot) return Promise.reject(new Error("not_found"));
      return Promise.resolve(clone({ document: { ...document, baseId: base.id }, ...snapshot }));
    } catch (failure) {
      return Promise.reject(failure);
    }
  }
  return invoke<DocumentDetail>("knowledge_inspect_document", input);
}

export function knowledgeImportText(input: {
  baseId: string;
  projectPath: string | null;
  draft: ImportDraft;
}): Promise<KnowledgeDocument> {
  if (!isDesktop()) {
    try {
      return Promise.resolve(addDemoDocument(input.baseId, input.projectPath, input.draft));
    } catch (error) {
      return Promise.reject(error);
    }
  }
  return invoke<KnowledgeDocument>("knowledge_import_text", input);
}

export function knowledgePickImport(input: {
  baseId: string;
  projectPath: string | null;
}): Promise<ImportPreview[]> {
  if (!isDesktop()) {
    try {
      findBase(input.baseId, input.projectPath);
    } catch (error) {
      return Promise.reject(error);
    }
    const token = nextDemoId("preview");
    const body = DEMO_KNOWLEDGE_FILE_PREVIEW.body;
    demoPendingImports.set(token, input);
    return Promise.resolve([
      {
        token,
        title: DEMO_KNOWLEDGE_FILE_PREVIEW.title,
        bodyPreview: body,
        characterCount: body.length,
        chunkCount: chunkText(body, "preview").length,
      },
    ]);
  }
  return invoke<ImportPreview[]>("knowledge_pick_import", input);
}

export function knowledgeConfirmImport(input: { token: string }): Promise<KnowledgeDocument> {
  if (!isDesktop()) {
    const preview = demoPendingImports.get(input.token);
    if (!preview) return Promise.reject(new Error("preview_expired"));
    demoPendingImports.delete(input.token);
    return Promise.resolve(
      addDemoDocument(preview.baseId, preview.projectPath, {
        title: DEMO_KNOWLEDGE_FILE_PREVIEW.title,
        body: DEMO_KNOWLEDGE_FILE_PREVIEW.body,
        sourceKind: "file",
        sourceRef: DEMO_KNOWLEDGE_FILE_PREVIEW.title,
        verified: false,
      }),
    );
  }
  return invoke<KnowledgeDocument>("knowledge_confirm_import", input);
}

export function knowledgeSetEnabled(input: {
  id: string;
  projectPath: string | null;
  expectedVersion: number;
  enabled: boolean;
}): Promise<KnowledgeDocument> {
  if (!isDesktop()) {
    const current = demoDocuments.find((row) => row.id === input.id);
    if (!current) return Promise.reject(new Error("not_found"));
    try {
      findBase(current.baseId, input.projectPath);
    } catch (error) {
      return Promise.reject(error);
    }
    if (current.version !== input.expectedVersion) return Promise.reject(new Error("revision_conflict"));
    if (current.status === "quarantined") return Promise.reject(new Error("invalid_input"));
    const next = {
      ...current,
      status: input.enabled ? "ready" as const : "disabled" as const,
      version: current.version + 1,
      updatedAt: demoNow(),
    };
    demoDocuments = demoDocuments.map((row) => (row.id === next.id ? next : row));
    return Promise.resolve(clone(next));
  }
  return invoke<KnowledgeDocument>("knowledge_set_enabled", input);
}

export function knowledgeDeleteDocument(input: {
  id: string;
  projectPath: string | null;
  expectedVersion: number;
}): Promise<void> {
  if (!isDesktop()) {
    const current = demoDocuments.find((row) => row.id === input.id);
    if (!current) return Promise.reject(new Error("not_found"));
    try {
      findBase(current.baseId, input.projectPath);
    } catch (error) {
      return Promise.reject(error);
    }
    if (current.version !== input.expectedVersion) return Promise.reject(new Error("revision_conflict"));
    demoDocuments = demoDocuments.filter((row) => row.id !== input.id);
    demoBodies.delete(input.id);
    return Promise.resolve();
  }
  return invoke<void>("knowledge_delete_document", input);
}

export function knowledgeSearch(input: { query: SearchQuery }): Promise<KnowledgeHit[]> {
  if (!isDesktop()) return Promise.resolve(searchDemo(input.query));
  return invoke<KnowledgeHit[]>("knowledge_search", input);
}

export function knowledgeGetBindings(input: {
  sessionId: string;
  projectPath: string | null;
}): Promise<KnowledgeSelection> {
  if (!isDesktop()) {
    return Promise.resolve(
      clone(demoBindings.get(bindingKey(input.sessionId, input.projectPath)) ?? { baseIds: [], strict: true }),
    );
  }
  return invoke<KnowledgeSelection>("knowledge_get_bindings", input);
}

export function knowledgeSetBindings(input: {
  sessionId: string;
  projectPath: string | null;
  selection: KnowledgeSelection;
}): Promise<void> {
  if (!isDesktop()) {
    demoBindings.set(bindingKey(input.sessionId, input.projectPath), clone(input.selection));
    return Promise.resolve();
  }
  return invoke<void>("knowledge_set_bindings", input);
}

export function knowledgePrepareTurn(input: {
  sessionId: string;
  projectPath: string | null;
  selection: KnowledgeSelection;
  text: string;
}): Promise<PreparedTurn> {
  if (!isDesktop()) {
    if (input.sessionId) {
      const persisted = demoBindings.get(bindingKey(input.sessionId, input.projectPath));
      const sameIds =
        persisted?.baseIds.length === input.selection.baseIds.length &&
        input.selection.baseIds.every((baseId) => persisted?.baseIds.includes(baseId));
      if (!persisted || persisted.strict !== input.selection.strict || !sameIds) {
        return Promise.reject(new Error("invalid_input"));
      }
    }
    const hits = searchDemo({
      baseIds: input.selection.baseIds,
      projectPath: input.projectPath,
      text: input.text,
      limit: 8,
      budgetTokens: 1800,
    });
    const token = nextDemoId("turn");
    const remote =
      hits.length > 0 &&
      new URLSearchParams(window.location.search).get("knowledge-fixture") === "approval";
    return Promise.resolve({
      token,
      hits,
      // The browser-only fixture exercises the approval UX without a model
      // connection. Desktop always receives the real Tauri preparation result.
      remote,
      providerLabel: remote ? "浏览器预览模型端点（测试夹具）" : null,
      strict: input.selection.strict,
    });
  }
  return invoke<PreparedTurn>("knowledge_prepare_turn", input);
}

export function knowledgeTurnCitations(input: {
  sessionId: string;
  turnSeq: number;
}): Promise<KnowledgeHit[]> {
  if (!isDesktop()) {
    const fixtureMode = new URLSearchParams(window.location.search).get("knowledge-fixture");
    if (fixtureMode === "citations" && input.sessionId === "c1" && input.turnSeq === 1) {
      return Promise.resolve(clone(DEMO_CONVERSATION_KNOWLEDGE_HITS));
    }
    return Promise.resolve(clone(demoCitationRows.get(`${input.sessionId}\u0000${input.turnSeq}`) ?? []));
  }
  return invoke<KnowledgeHit[]>("knowledge_turn_citations", input);
}
