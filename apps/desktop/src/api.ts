import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Value edge api → data; data/skills.ts only type-imports from here, so the
// modules never form a runtime cycle.
import { DEMO_AGENTS } from "./data/agents";
import { DEMO_MCP_SERVERS, DEMO_MCP_TOOLS } from "./data/demo";
import { DEMO_PROJECT_SKILLS, DEMO_USER_SKILLS } from "./data/skills";

/** Mirrors the `CoreInfo` command payload returned by the Tauri shell. */
export type CoreInfo = {
  name: string;
  version: string;
};

/** False when the GUI runs in a plain browser (visual regression, `vite dev`). */
export function isDesktop(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

/** Returns null outside the desktop shell, where there is no core to reach. */
export function coreInfo(): Promise<CoreInfo | null> {
  return isDesktop() ? invoke<CoreInfo>("core_info") : Promise.resolve(null);
}

/** A project the user registered by hand. There is no scan and no default root. */
export type ProjectDto = {
  path: string;
  name: string;
};

/** A session as the sidebar needs it, without its events. */
export type SessionRefDto = {
  id: string;
  project: string;
  title: string;
  at: number;
  archived: boolean;
  /** Which agent world stamped it at creation; absent (or legacy) = code. */
  mode?: "work" | "code";
};

/** The project list and the session list in one payload, so they cannot disagree. */
export type WorkspaceDto = {
  projects: ProjectDto[];
  sessions: SessionRefDto[];
};

export type StepStatusDto = "running" | "done" | "failed";

/** Three orthogonal axes. Backend-computed; the UI maps codes to copy. */
export type LifecycleStatus =
  | "queued"
  | "working"
  | "awaiting_approval"
  | "completed"
  | "stopped"
  | "interrupted"
  | "failed";
export type DeliveryStatus = "ready" | "partial" | "blocked" | "failed";
export type VerificationStatus = "not_run" | "running" | "passed" | "failed" | "blocked";
export type OutcomeStatus =
  | "working"
  | "queued"
  | "awaiting_approval"
  | "partially_completed"
  | "blocked_by_environment"
  | "completed"
  | "failed"
  | "stopped"
  | "interrupted";

export type TurnStatusDto = {
  lifecycle: LifecycleStatus;
  delivery: DeliveryStatus;
  verification: VerificationStatus;
  outcome: OutcomeStatus;
};

export type ChangeDto = {
  path: string;
  added: number;
  removed: number;
  /** Times this turn edited the path (merged steps count once per edit). */
  edits?: number;
};

export type ItemDto = {
  id: number;
  at: number;
  status: StepStatusDto;
  duration: number | null;
} & (
  | {
      kind: "reasoning";
      summary: string;
      /** Public phase code; empty/absent on legacy rows. */
      phase?: string;
      /** Internal scheduling diagnostics — Debug surfaces only. */
      diagnostics?: string | null;
    }
  | { kind: "search"; query: string; detail: string }
  | { kind: "fileRead"; path: string; detail: string }
  | {
      kind: "commandExecution";
      command: string;
      cwd: string;
      output: string;
      exitCode: number | null;
      denied?: boolean;
      /** Structured failure taxonomy code from the backend. */
      failureClass?: string | null;
      /** Missing binary when failureClass is command_not_found. */
      failureTool?: string | null;
    }
  | {
      kind: "modelCall";
      model: string;
      inputTokens: number;
      outputTokens: number;
      /** Reference to `traces/<session>/<turn>/llm_io/<seq>.json`. */
      llmIoRef?: string | null;
    }
  | {
      kind: "fileChange";
      changes: ChangeDto[];
      /** Reference to `traces/<session>/<turn>/spans/<seq>-file-…json`. */
      fileSpanRef?: string | null;
    }
  | {
      kind: "agentMessage";
      text: string;
      checks: string[];
      /** Structured outcome axes written by the agent. */
      delivery?: string;
      verification?: string;
      /** Conversational answer — the UI renders the text alone, no report chrome. */
      plain?: boolean;
      /** Structured failure taxonomy (`budget_exhausted` / `acceptance_unmet` /
       *  `unrecoverable` / `provider_error` / `unknown`) when this answer
       *  ended the run in a failed state. Empty for ready/partial answers. */
      failureReason?: string | null;
    }
);

export type ItemKindDto = ItemDto["kind"];

/**
 * Interim prose the provider streamed between tool steps, kept in normal order.
 * Added by the UI layer only — the core never stores it, so it is live-only and
 * gone after a reload.
 */
export type InterimNote = {
  /** Monotonic id for stable list keys. */
  id: number;
  /** The streamed text between step boundaries. */
  text: string;
  /** Item id whose boundary flushed the note; it renders before that step. */
  beforeId: number;
};

export type TurnDto = {
  ask: string;
  /** Project-relative paths pinned as context for this turn (not file bodies). */
  context: string[];
  items: ItemDto[];
  /** Live-only interim stream notes (see [`InterimNote`]). */
  notes?: InterimNote[];
  done: boolean;
  stopped: boolean;
  /** Recovery stamped a killed run as interrupted (never Completed). */
  interrupted?: boolean;
  error: string | null;
  /** Backend-computed status axes (absent on live/demo turns — derived). */
  status?: TurnStatusDto;
};

export type SessionDto = {
  id: string;
  project: string;
  title: string;
  at: number;
  archived: boolean;
  mode?: "work" | "code";
  turns: TurnDto[];
};

export type ArchivedItemDto = {
  id: string;
  project: string;
  projectName: string;
  title: string;
  at: number;
  model: string;
  summary: string;
  filesChanged: number;
  added: number;
  removed: number;
};

type RunEventPayload =
  | { type: "turnStarted"; session: string }
  | { type: "itemStarted"; session: string; item: ItemDto }
  | { type: "itemCompleted"; session: string; item: ItemDto }
  | { type: "turnComplete"; session: string }
  | { type: "stopped"; session: string }
  | { type: "error"; session: string; message: string }
  | {
      type: "approvalRequest";
      session: string;
      step: number;
      kind: string;
      detail?: string;
      command?: string;
      cwd?: string;
      riskCategory?: string;
      reason?: string;
    }
  | { type: "textDelta"; session: string; text: string }
  | { type: "progress"; session: string; phase: string; detail: string }
  | {
      type: "failover";
      session: string;
      fromProvider: string;
      fromModel: string;
      errorClass: string;
      error: string;
      toProvider: string;
      toModel: string;
    }
  /** Compatibility alias for older runners; new runners emit `failover`. */
  | {
      type: "providerSwitch";
      session: string;
      fromProvider: string;
      fromModel: string;
      errorClass: string;
      error: string;
      toProvider: string;
      toModel: string;
    }
  | {
      type: "contextBudget";
      session: string;
      used: number;
      window: number;
      percent: number;
      cumulativeChars: number;
      compactedCount: number;
      lastModel: string;
    }
  | {
      type: "compacted";
      session: string;
      dropped: number;
      summarizedInto: number;
      freedPct: number;
    }
  | {
      type: "memoryRecalled";
      session: string;
      count: number;
      ids: string[];
    };

export type RunEventDto = RunEventPayload & { runId?: number };

export const RUN_EVENT = "run:event";

function read<T>(command: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!isDesktop()) return Promise.resolve(null);
  return invoke<T>(command, args).catch((error: unknown) => {
    console.warn(`kodo: ${command} failed:`, error);
    return null;
  });
}

function write<T>(command: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!isDesktop()) return Promise.resolve(null);
  return invoke<T>(command, args);
}

/**
 * Send a user turn to the core. `agents` is the persona **chain** in
 * application order — a single element is the common case, shift-click on
 * the composer appends to grow the chain. The backend's `resolve_chain`
 * walks project → user → builtin for each name. `budgetFamily` lets the
 * desktop pass `"summary"` to lift the next run into read-only-summary
 * caps (used by the "继续本轮" affordance).
 */
export function sendMessage(
  id: string,
  text: string,
  context: string[] = [],
  project?: string,
  agents: string[] = [],
  budgetFamily?: string | null,
): Promise<string> {
  return invoke<string>("send_message", {
    id,
    text,
    context,
    project,
    agents,
    budgetFamily,
  });
}

export function stopRun(id: string): Promise<void | null> {
  return write<void>("stop_run", { id });
}

export function respondApproval(
  id: string,
  step: number,
  approved: boolean,
  sessionWide = false,
  runId?: number,
): Promise<void> {
  return invoke<void>("respond_approval", { id, step, approved, sessionWide, runId });
}

export type TurnChangeDto = {
  path: string;
  diff: string;
  userPreexisting: boolean;
  /** Live undo state vs recorded after-hash. */
  undoState: "clean" | "already_baseline" | "diverged" | "missing" | string;
  /** True when working tree diverged from Kodo's after-hash (state C risk). */
  conflict: boolean;
};

export type UndoConflictDto = {
  path: string;
  reason: string;
  message: string;
};

export type UndoReportDto = {
  restored: string[];
  conflicts: UndoConflictDto[];
};

/** Per-file unified diffs of Kodo's changes for this session. */
export function turnChanges(project: string, id: string): Promise<TurnChangeDto[] | null> {
  return read<TurnChangeDto[]>("turn_changes", { project, id });
}

/** Undo only Kodo's changes; user-only and post-turn user edits are never overwritten. */
export function undoTurn(project: string, id: string): Promise<UndoReportDto | null> {
  return write<UndoReportDto>("undo_turn", { project, id });
}

// ---------------------------------------------------------------------------
// Trace artifacts (Phase D — transparency UI)
// ---------------------------------------------------------------------------

export type ArtifactKind = "llm_io" | "file_span";

export type ArtifactRefDto = {
  kind: ArtifactKind;
  seq: number;
  label: string;
  sizeBytes: number;
  at: number;
};

export type LlmMessageDto = {
  role: string;
  content?: unknown;
  toolCalls?: unknown;
};

export type LlmIoRequestDto = {
  system: string;
  messages: unknown;
  tools?: unknown;
};

export type LlmIoResponseDto = {
  text: string;
  reasoningContent: string;
  nativeToolCalls: unknown;
};

export type LlmIoDto = {
  seq: number;
  model: string;
  protocol: string;
  retryIndex: number;
  providerLatencyMs: number;
  finishReason: string;
  inputTokens: number;
  outputTokens: number;
  sawReasoning: boolean;
  at: number;
  request: LlmIoRequestDto;
  response: LlmIoResponseDto;
};

export type FileSpanDto = {
  path: string;
  contentBefore: string;
  /** `null` when the tool created or deleted the file (no prior content). */
  contentAfter: string | null;
  toolCallId: string;
  seq: number;
};

/** Lists every persisted trace artifact under one turn (LLM I/O + file spans). */
export function listArtifacts(
  sessionId: string,
  turnSeq: number,
): Promise<ArtifactRefDto[] | null> {
  return read<ArtifactRefDto[]>("list_artifacts", { sessionId, turnSeq });
}

/** Loads one captured LLM I/O body — used by the LLM I/O tab in TracePage. */
export function loadLlmIo(
  sessionId: string,
  turnSeq: number,
  refPath: string,
): Promise<LlmIoDto | null> {
  return read<LlmIoDto>("load_llm_io", { sessionId, turnSeq, refPath });
}

/** Loads one captured file-span snapshot (before/after for one tool write). */
export function loadFileSpan(
  sessionId: string,
  turnSeq: number,
  refPath: string,
): Promise<FileSpanDto | null> {
  return read<FileSpanDto>("load_file_span", { sessionId, turnSeq, refPath });
}

/** Cheap probe so the trace page can decide whether to render its tab. */
export function turnHasArtifacts(
  sessionId: string,
  turnSeq: number,
): Promise<boolean | null> {
  return read<boolean>("turn_has_artifacts", { sessionId, turnSeq });
}

export function workspace(): Promise<WorkspaceDto | null> {
  return read<WorkspaceDto>("workspace");
}

export function addProject(path: string): Promise<WorkspaceDto | null> {
  return write<WorkspaceDto>("add_project", { path });
}

export function createProject(parent: string, name: string): Promise<WorkspaceDto | null> {
  return write<WorkspaceDto>("create_project", { parent, name });
}

export function removeProject(path: string): Promise<WorkspaceDto | null> {
  return write<WorkspaceDto>("remove_project", { path });
}

export function renameProject(path: string, name: string): Promise<WorkspaceDto | null> {
  return write<WorkspaceDto>("rename_project", { path, name });
}

export function reorderProject(path: string, index: number): Promise<WorkspaceDto | null> {
  return write<WorkspaceDto>("reorder_project", { path, index });
}

export function pickFolder(): Promise<string | null> {
  return read<string>("pick_folder");
}

export function setting(key: string): Promise<string | null> {
  return read<string>("setting", { key });
}

export function setSetting(key: string, value: string): Promise<void | null> {
  return write<void>("set_setting", { key, value });
}

export function gitBranch(path: string): Promise<string | null> {
  return read<string>("git_branch", { path });
}

export function revealProject(path: string): Promise<void | null> {
  return write<void>("reveal_project", { path });
}

/** Reveal one project file in the system file manager. Same path guards as
 *  `open_project_file`; nothing is read. No-op outside the desktop shell. */
export function revealFile(project: string, path: string): Promise<void | null> {
  return write<void>("reveal_file", { project, path });
}

/** Completion ping: Rust asks the OS for attention only while the window is
 *  in the background and owns the native title marker itself. */
export function taskFeedback(): Promise<void | null> {
  return write<void>("task_feedback", {});
}

export function openSession(project: string, title: string): Promise<SessionDto | null> {
  return write<SessionDto>("open_session", { project, title });
}

export function loadSession(id: string): Promise<SessionDto | null> {
  return read<SessionDto>("load_session", { id });
}

export function retitleSession(id: string, title: string): Promise<SessionDto | null> {
  return write<SessionDto>("retitle_session", { id, title });
}

export function archiveSession(id: string): Promise<WorkspaceDto | null> {
  return write<WorkspaceDto>("archive_session", { id });
}

export function restoreSession(id: string): Promise<WorkspaceDto | null> {
  return write<WorkspaceDto>("restore_session", { id });
}

export function deleteSession(id: string): Promise<WorkspaceDto | null> {
  return write<WorkspaceDto>("delete_session", { id });
}

export function listArchived(): Promise<ArchivedItemDto[] | null> {
  return read<ArchivedItemDto[]>("list_archived");
}

export type ProviderDto = {
  id: string;
  name: string;
  template: string;
  /** Masked when loaded (`••••abcd`). Empty means no secret stored. */
  apiKey: string;
  endpoint: string;
  /** Legacy field — display name or model id. */
  model: string;
  /** Backend API model id. */
  modelId?: string;
  /** UI display label. */
  displayName?: string;
  /** Configured model list; `null`/absent on legacy records.
   *  `vision` rides the wire too — Tauri seeds it from the template +
   *  `ModelSpec::vision_override` and the composer reads it to gate
   *  the first-time image-skip notice. Undefined for legacy records. */
  models?: Array<{ modelId: string; displayName: string; vision?: boolean; suitableForLong?: boolean }> | null;
  hasKey?: boolean;
};

export function loadProvidersCommand(): Promise<ProviderDto[] | null> {
  return read<ProviderDto[]>("load_providers");
}

export function saveProvidersCommand(providers: ProviderDto[]): Promise<ProviderDto[] | null> {
  return write<ProviderDto[]>("save_providers", { providers });
}

/** Auto-load the provider's supported models from `GET /models`.
 *  Empty list outside desktop (demo route / browser). */
export function listProviderModels(
  template: string,
  endpoint: string,
  apiKey: string,
  providerId?: string,
): Promise<Array<{ modelId: string; displayName: string; vision: boolean }>> {
  if (!isDesktop()) return Promise.resolve([]);
  return invoke<Array<{ modelId: string; displayName: string; vision: boolean }>>(
    "list_provider_models",
    {
      template,
      endpoint,
      apiKey,
      providerId: providerId ?? null,
    },
  );
}

/** One configured MCP server (a `mcp-servers` entry in settings.log).
 *  Secret halves (env / header values) arrive masked (`••••abcd`); sending a
 *  masked or blank value back keeps the stored secret — mask writeback, same
 *  contract as `ProviderDto.apiKey`. */
export type McpServerDto = {
  id: string;
  name: string;
  enabled: boolean;
  /** `stdio` spawns a local process; `http` posts to a Streamable HTTP URL. */
  transport: "stdio" | "http";
  command: string;
  args: string[];
  env: Record<string, string>;
  url: string;
  headers: Record<string, string>;
};

export function loadMcpServersCommand(): Promise<McpServerDto[] | null> {
  return read<McpServerDto[]>("load_mcp_servers");
}

export function saveMcpServersCommand(servers: McpServerDto[]): Promise<McpServerDto[] | null> {
  return write<McpServerDto[]>("save_mcp_servers", { servers });
}

/** The configured servers; outside desktop (demo route / plain browser) the
 *  frozen fixture, the same way `listSkills` feeds `/ui-demo`. */
export function loadMcpServers(): Promise<McpServerDto[]> {
  if (!isDesktop()) return Promise.resolve(DEMO_MCP_SERVERS);
  return loadMcpServersCommand().then((list) => list ?? []);
}

/** Persists the full list; returns the backend's masked copy. */
export async function saveMcpServers(servers: McpServerDto[]): Promise<McpServerDto[]> {
  if (!isDesktop()) return servers;
  return (await saveMcpServersCommand(servers)) ?? [];
}

/** One advertised tool as the MCP page lists it — name and description only;
 *  schemas and secrets never cross this boundary. */
export type McpToolDto = {
  name: string;
  description: string;
};

/** Connects the named server once (secrets stay server-side), lists its tools,
 *  drops the transport. Failure is a rejected promise with the shell's reason —
 *  `write`, not `read`, so the page can tell a broken server from an empty one. */
export function listMcpToolsCommand(id: string): Promise<McpToolDto[] | null> {
  return write<McpToolDto[]>("mcp_list_tools", { id });
}

/** The tool list for one server; outside desktop (demo route / plain browser)
 *  the frozen fixture, the same way `loadMcpServers` feeds `/ui-demo`. */
export async function listMcpTools(id: string): Promise<McpToolDto[]> {
  if (!isDesktop()) return DEMO_MCP_TOOLS[id] ?? [];
  return (await listMcpToolsCommand(id)) ?? [];
}

/** One workflow step inside a skill preview. */
export type SkillStepDto = {
  id: string;
  /** `read` | `edit` | `command` | `verify`. */
  kind: string;
  title: string;
};

/** One skill as the Skills page previews it — builtin, project and user files
 *  share this shape. `markdown` is the raw SKILL.md source. */
export type SkillView = {
  name: string;
  source: "builtin" | "user" | "project";
  /** One-line `## description` (backend falls back to the first criterion). */
  description: string;
  /** Task type labels (`bug-fix`, `work`, …) — which space(s) it belongs to. */
  taskTypes: string[];
  strategy: string;
  policy: string;
  workflow: SkillStepDto[];
  allowedTools: string[];
  completionCriteria: string[];
  guidance: string;
  markdown: string;
};

/** Which store a CRUD call targets: the user dir or the project's
 *  `.kodo/skills` (needs a project path). */
export type SkillScope = "user" | "project";

/** All skills: builtins first (backend), then project and user files.
 *  Outside desktop the demo fixtures keep both sections visible (single
 *  source for the `!isDesktop()` path). */
export function listSkills(project?: string | null): Promise<SkillView[]> {
  if (!isDesktop()) {
    return Promise.resolve(
      project ? [...DEMO_PROJECT_SKILLS, ...DEMO_USER_SKILLS] : DEMO_USER_SKILLS,
    );
  }
  return read<SkillView[]>("list_skills", { project: project ?? null }).then((list) => list ?? []);
}

/** One skill's detail for the preview modal; null when it does not exist.
 *  `scope` narrows the lookup to one store (builtin rows omit it — the
 *  backend answers builtins first). */
export function readSkill(
  name: string,
  opts?: { scope?: SkillScope; project?: string | null },
): Promise<SkillView | null> {
  return read<SkillView>("read_skill", {
    name,
    scope: opts?.scope ?? null,
    project: opts?.project ?? null,
  });
}

/** Validate and save a skill into the scoped store (user dir by default).
 *  Rejects with the backend's zh-CN error (parse failure, reserved name) so
 *  the editor can show it inline. */
export function saveSkill(
  name: string,
  markdown: string,
  opts?: { scope?: SkillScope; project?: string | null },
): Promise<SkillView | null> {
  return write<SkillView>("save_skill", {
    name,
    markdown,
    scope: opts?.scope ?? null,
    project: opts?.project ?? null,
  });
}

/** Delete a skill file from the scoped store; rejects with zh-CN errors. */
export function deleteSkill(
  name: string,
  opts?: { scope?: SkillScope; project?: string | null },
): Promise<void | null> {
  return write<void>("delete_skill", {
    name,
    scope: opts?.scope ?? null,
    project: opts?.project ?? null,
  });
}

/** One persona record as the 插件 · 智能体 tab previews it. `source`
 *  distinguishes the 12 read-only built-ins from user-defined files. */
export type PersonaDefaults = {
  /** Provider id (`openai`, `anthropic`, …) — overrides apply only when
   *  this matches the active provider. */
  provider?: string;
  /** Model id (`gpt-4o`, `claude-sonnet-4-5`, …). */
  model?: string;
  /** Permission level — `"ask"` | `"auto"` | `"full"`. */
  permission?: string;
  /** Skill ids pre-selected when this persona is picked. The composer's
   *  user-driven selection is merged on top. */
  skills: string[];
};

export type PersonaExample = {
  input: string;
  output: string;
};

export type AgentView = {
  name: string;
  /** `"builtin"` for the 12 embedded records (COMPONENTS.md §13d),
   *  `"user"` for files written to `state_dir()/agents/`,
   *  `"project"` for `<project>/.kodo/agents/`. */
  source: "builtin" | "user" | "project";
  /** One-line `## description` (the selector hint and row subtitle). */
  description: string;
  /** `## instructions` body — what gets appended to the system prompt. */
  instructions: string;
  /** Raw agent markdown — the detail modal's editor base. */
  markdown: string;
  /** Per-persona session defaults (§0.1 / B.1-B.3). Empty fields mean
   *  "leave the composer's choice alone". */
  defaults: PersonaDefaults;
  /** Declared template variables the `## instructions` may use
   *  (e.g. `project_path`, `branch`). Unknown placeholders stay verbatim. */
  variables: string[];
  /** Few-shot examples rendered after the persona block at run time. */
  examples: PersonaExample[];
};

/** Browser/demo fallback: a mutable copy so the tab round-trips without Tauri
 *  (reset on reload; the visual suite stubs the real commands instead). */
let demoAgents: AgentView[] = DEMO_AGENTS.map((a) => ({ ...a }));

/** Derive the two section fields from composed markdown — the demo path only;
 *  the desktop shell parses authoritatively in `agents::parse_agent`. */
function demoSections(markdown: string): { description: string; instructions: string } {
  const description = markdown.match(/^## description\n(.*)$/m)?.[1]?.trim() ?? "";
  const instructions = markdown.split(/^## instructions\n/m)[1]?.trim() ?? "";
  return { description, instructions };
}

/**
 * All persona records. Outside desktop the demo fixtures back the tab;
 * inside, the shell merges builtins → project (when `project` is given)
 * → user files so the UI can group them by source.
 */
export function listAgents(project?: string): Promise<AgentView[]> {
  if (!isDesktop()) return Promise.resolve([...demoAgents]);
  return read<AgentView[]>("list_agents", { project }).then((list) => list ?? []);
}

/**
 * One agent's full detail for the preview modal; null when it does not
 * exist. The optional `project` resolves project-scope personas first.
 */
export function readAgent(name: string, project?: string): Promise<AgentView | null> {
  if (!isDesktop()) {
    return Promise.resolve(demoAgents.find((a) => a.name === name) ?? null);
  }
  return read<AgentView>("read_agent", { name, project });
}

/** Validate and save a persona; rejects with the backend's zh-CN error so the
 *  editor can show it inline. */
export function saveAgent(name: string, markdown: string): Promise<AgentView | null> {
  if (!isDesktop()) {
    const { description, instructions } = demoSections(markdown);
    const next: AgentView = {
      name,
      source: "user",
      description,
      instructions,
      markdown,
      defaults: { skills: [] },
      variables: [],
      examples: [],
    };
    demoAgents = [...demoAgents.filter((a) => a.name !== name), next];
    return Promise.resolve(next);
  }
  return write<AgentView>("save_agent", { name, markdown });
}

/** Delete a persona file; rejects with zh-CN errors. */
export function deleteAgent(name: string): Promise<void | null> {
  if (!isDesktop()) {
    demoAgents = demoAgents.filter((a) => a.name !== name);
    return Promise.resolve(null);
  }
  return write<void>("delete_agent", { name });
}

/** Project-relative file paths (ignore rules applied). The composer picker
 *  omits `limit` and the core caps at 200; the file viewer passes a larger one. */
export function listProjectFiles(project: string, query?: string, limit?: number): Promise<string[]> {
  if (!isDesktop()) return Promise.resolve([]);
  return invoke<string[]>("list_project_files", {
    project,
    query: query ?? null,
    limit: limit ?? null,
  });
}

/** Full text of a project file for the file viewer (line-capped by the core). */
export function readProjectFile(project: string, path: string): Promise<string> {
  if (!isDesktop()) return Promise.resolve("");
  return invoke<string>("read_project_file", { project, path });
}

/** Open a project file with the system default application (PDF/Office/binary
 *  escape hatch from the file viewer). No-op outside the desktop shell. */
export function openProjectFile(project: string, path: string): Promise<void> {
  if (!isDesktop()) return Promise.resolve();
  return invoke<void>("open_project_file", { project, path });
}

/** Open an http(s)/mailto link from an answer in the system browser (the
 *  webview never navigates). Plain-browser dev falls back to a new tab. */
export function openUrl(url: string): Promise<void> {
  if (!isDesktop()) {
    window.open(url, "_blank", "noopener,noreferrer");
    return Promise.resolve();
  }
  return invoke<void>("open_url", { url });
}

/** Embedded browser backing the 浏览 inspector panel. Native Tauri webview
 *  parented to the main window (NSWindow.addChildWindow: on macOS, owned
 *  + skip_taskbar on Win/Linux) — not an iframe, so cross-origin
 *  X-Frame-Options never refuse the load. No-op outside the desktop shell. */
export function browseOpen(url: string): Promise<void> {
  if (!isDesktop()) return Promise.resolve();
  return invoke<void>("browse_open", { url });
}

/** Navigate an existing browse webview to a new URL (panel re-use — the OS
 *  surface stays; only the document changes). */
export function browseSetUrl(url: string): Promise<void> {
  if (!isDesktop()) return Promise.resolve();
  return invoke<void>("browse_set_url", { url });
}

/** Destroy the embedded browser window — called when the inspector closes or
 *  the panel switches away. */
export function browseClose(): Promise<void> {
  if (!isDesktop()) return Promise.resolve();
  return invoke<void>("browse_close", {});
}

/** Resize and reposition the embedded browser to track `.ins-content--flush`.
 *  Coords are logical pixels relative to the main window content area
 *  (matches DOMRect from `getBoundingClientRect` when the scale factor is 1). */
export function browseSetPosition(
  x: number,
  y: number,
  width: number,
  height: number,
): Promise<void> {
  if (!isDesktop()) return Promise.resolve();
  return invoke<void>("browse_set_position", { x, y, width, height });
}

/** Absolute paths from the OS "添加照片和文件" dialog; null when cancelled. */
export function pickFiles(): Promise<string[] | null> {
  if (!isDesktop()) return Promise.resolve(null);
  return invoke<string[] | null>("pick_files", {}).catch(() => null);
}

/** Short preview of a project file; rejects when the path leaves the project. */
export function readContextFile(project: string, path: string): Promise<string> {
  if (!isDesktop()) return Promise.resolve("");
  return invoke<string>("read_context_file", { project, path });
}

/** True when the path is readable (project-relative or an absolute external file). */
export async function validateContextPath(project: string, path: string): Promise<boolean> {
  if (!path || path.includes("..")) return false;
  if (!isDesktop()) return false;
  try {
    await invoke<ContextProbe>("probe_context_file", { project, path });
    return true;
  } catch {
    return false;
  }
}

/** Attachment kind for the composer cards. `preview` is a data URL (image) or a text snippet. */
export type ContextProbe = {
  kind: "image" | "text" | "pdf" | "office" | "other";
  name: string;
  size: number;
  preview: string | null;
};

/**
 * Probes any attachment type (image, text, PDF, Office…); rejects only when the
 * path is missing or invalid. Binary files attach fine — they just carry no
 * inline text preview.
 */
export function probeContextFile(project: string, path: string): Promise<ContextProbe> {
  return invoke<ContextProbe>("probe_context_file", { project, path });
}

export function onRunEvent(handler: (event: RunEventDto) => void): Promise<() => void> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen<RunEventDto>(RUN_EVENT, (event) => handler(event.payload));
}

// ---- Memory Engine IPC ---------------------------------------------------

export type MemoryKind =
  | "working"
  | "episodic"
  | "semantic"
  | "procedural"
  | "rule"
  | "evidence_ref";

export type MemoryStatus =
  | "candidate"
  | "active"
  | "stale"
  | "superseded"
  | "quarantined"
  | "rejected";

export type MemoryAuthority = "historical" | "user_rule" | "system";
export type SourceTrust =
  | "untrusted"
  | "tool_verified"
  | "test_verified"
  | "user_confirmed";

export type MemoryEntryDto = {
  id: string;
  kind: MemoryKind;
  status: MemoryStatus;
  authority: MemoryAuthority;
  sourceTrust: SourceTrust;
  title: string;
  abstractText: string;
  body: unknown;
  contentHash: string;
  confidence: number;
  verified: boolean;
  conditions: string[];
  sourceUris: string[];
  scopeProjectId: string | null;
  scopeRepositoryId: string | null;
  scopeBranch: string | null;
  useCount: number;
  version: number;
  createdAt: number;
  updatedAt: number;
};

export type MemoryStatusDto = {
  path: string;
  rowCounts: Record<string, number>;
  outboxPending: number;
  tombstones: number;
  dbSizeBytes: number;
  mode: string;
};

export type MemorySnippetDto = {
  id: string;
  kind: MemoryKind;
  title: string;
  abstractText: string;
  sourceUris: string[];
  validity: string;
  authority: MemoryAuthority;
  scopeLabel: string;
};

export type MemoryDoctorDto = {
  ok: boolean;
  integrity: string[];
  rowCounts: Record<string, number>;
  outboxPending: number;
  outboxReplayed: number;
  tombstones: number;
  notes: string[];
  dbSizeBytes: number;
};

export function memoryStatus(): Promise<MemoryStatusDto> {
  return invoke<MemoryStatusDto>("memory_status", {});
}

export type MemoryListArgs = {
  project?: string;
  kind?: MemoryKind;
  status?: MemoryStatus;
  limit?: number;
  includeHistory?: boolean;
};

export function memoryList(args: MemoryListArgs): Promise<MemoryEntryDto[]> {
  return invoke<MemoryEntryDto[]>("memory_list", { args });
}

export function memorySearch(
  project: string | undefined,
  query: string,
  limit?: number,
): Promise<MemorySnippetDto[]> {
  return invoke<MemorySnippetDto[]>("memory_search", {
    args: { project, query, limit },
  });
}

export function memoryInspect(id: string): Promise<MemoryEntryDto | null> {
  return invoke<MemoryEntryDto | null>("memory_inspect", { id });
}

export function memoryDisable(id: string): Promise<void> {
  return invoke<void>("memory_disable", { id });
}

export function memoryDoctor(apply: boolean): Promise<MemoryDoctorDto> {
  return invoke<MemoryDoctorDto>("memory_doctor", { apply });
}

// ---------------------------------------------------------------------------
// Scheduled tasks
// ---------------------------------------------------------------------------

/**
 * Mirrors `kodo_core::scheduler::Schedule` from the Tauri command
 * payload. Only the first version is exposed; advanced Cron does not
 * ship until the second version per docs/design/SCHEDULED_TASKS_IMPLEMENTATION.md.
 */
export type ScheduleDto =
  | { kind: "once"; fireAt: number }
  | { kind: "interval"; anchor: number; intervalSeconds: number }
  | { kind: "daily"; hour: number; minute: number; tzName: string }
  | { kind: "weekly"; weekdays: number[]; hour: number; minute: number; tzName: string };

export type TaskStateDto = "enabled" | "paused" | "finished" | "archived";
export type MisfirePolicyDto = "latestOnce" | "skip" | "coalesce";
export type OverlapPolicyDto = "skip" | "queueLatest" | "allow";
export type TriggerSourceDto = "scheduled" | "manual";
export type LifecycleDto = "queued" | "running" | "cancelling" | "finished";
export type OutcomeDto =
  | "succeeded"
  | "partial"
  | "blocked"
  | "failed"
  | "cancelled"
  | "timedOut"
  | "interrupted"
  | "skipped";

/** Mirrors `kodo_core::scheduler::TaskKind` (Agent / Job). */
export type TaskKindDto =
  | {
      kind: "agent";
      project: string;
      message: string;
      personas: string[];
      budgetFamily: string | null;
      resourceKey: string | null;
    }
  | {
      kind: "job";
      jobType: string;
      params: unknown;
      resources: string[];
    };

/** Mirrors `kodo_core::scheduler::ScheduledTask` from the Tauri command payload. */
export type ScheduledTaskDto = {
  id: string;
  title: string;
  state: TaskStateDto;
  revision: number;
  scheduleGeneration: number;
  schedule: ScheduleDto;
  task: TaskKindDto;
  timezone: string;
  misfirePolicy: MisfirePolicyDto;
  graceSeconds: number;
  overlapPolicy: OverlapPolicyDto;
  timeoutSeconds: number | null;
  createdAt: number;
  updatedAt: number;
  nextFireAt: number | null;
  archived: boolean;
};

/** Mirrors `kodo_core::scheduler::TaskRun` from the Tauri command payload. */
export type TaskRunDto = {
  id: string;
  taskId: string;
  occurrenceKey: string;
  triggerSource: TriggerSourceDto;
  scheduleGeneration: number;
  scheduledFor: number;
  queuedAt: number;
  startedAt: number | null;
  finishedAt: number | null;
  configRevision: number;
  configSnapshot: TaskKindDto;
  lifecycle: LifecycleDto;
  outcome: OutcomeDto | null;
  delivery: string | null;
  verification: string | null;
  sessionId: string | null;
  reportRef: string | null;
  reasonCode: string | null;
  errorSummary: string | null;
  skippedCount: number;
};

export function listScheduledTasks(includeArchived = false): Promise<ScheduledTaskDto[]> {
  return invoke<ScheduledTaskDto[]>("list_scheduled_tasks", { includeArchived });
}

export function getScheduledTask(id: string): Promise<ScheduledTaskDto | null> {
  return invoke<ScheduledTaskDto | null>("get_scheduled_task", { id });
}

export function createScheduledTask(task: ScheduledTaskDto): Promise<ScheduledTaskDto> {
  return invoke<ScheduledTaskDto>("create_scheduled_task", { task });
}

export function updateScheduledTask(
  expectedRevision: number,
  task: ScheduledTaskDto,
): Promise<ScheduledTaskDto> {
  return invoke<ScheduledTaskDto>("update_scheduled_task", { expectedRevision, task });
}

export function setScheduledTaskEnabled(id: string, enabled: boolean): Promise<ScheduledTaskDto> {
  return invoke<ScheduledTaskDto>("set_scheduled_task_enabled", { id, enabled });
}

export function archiveScheduledTask(id: string): Promise<void> {
  return invoke<void>("archive_scheduled_task", { id });
}

export function previewSchedule(schedule: ScheduleDto, after?: number): Promise<number[]> {
  return invoke<number[]>("preview_schedule", { schedule, after });
}

export function runScheduledTaskNow(id: string): Promise<TaskRunDto> {
  return invoke<TaskRunDto>("run_scheduled_task_now", { id });
}

export function listTaskRuns(taskId: string, limit = 50): Promise<TaskRunDto[]> {
  return invoke<TaskRunDto[]>("list_task_runs", { taskId, limit });
}

export function cancelTaskRun(id: string): Promise<boolean> {
  return invoke<boolean>("cancel_task_run", { id });
}

export function listJobTypes(): Promise<string[]> {
  return invoke<string[]>("list_job_types");
}

