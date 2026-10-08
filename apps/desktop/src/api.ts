import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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
    }
);

export type ItemKindDto = ItemDto["kind"];

export type TurnDto = {
  ask: string;
  /** Project-relative paths pinned as context for this turn (not file bodies). */
  context: string[];
  items: ItemDto[];
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

export type RunEventDto =
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
    };

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

export function sendMessage(
  id: string,
  text: string,
  context: string[] = [],
  project?: string,
): Promise<string> {
  return invoke<string>("send_message", { id, text, context, project });
}

export function stopRun(id: string): Promise<void | null> {
  return write<void>("stop_run", { id });
}

export function respondApproval(
  id: string,
  step: number,
  approved: boolean,
  sessionWide = false,
): Promise<void> {
  return invoke<void>("respond_approval", { id, step, approved, sessionWide });
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

/** Two artifact kinds live under `traces/<session>/<turn>/`. */
export type ArtifactKind = "llm_io" | "file_span";

/** A single artifact surfaced by `list_artifacts` for one turn. */
export type ArtifactRefDto = {
  kind: ArtifactKind;
  /** 1-based sequence inside the turn. */
  seq: number;
  /** Display label such as `llm_io/001.json` or `spans/002-file-…json`. */
  label: string;
  sizeBytes: number;
  at: number;
};

export type LlmIoRequestDto = {
  system: string;
  /** Provider-shaped message array (kept opaque; the viewer renders it). */
  messages: unknown;
};

export type LlmIoResponseDto = {
  text: string;
  /** Anthropic-style reasoning_content; empty when the model didn't think. */
  reasoningContent: string;
  nativeToolCalls: unknown;
};

export type LlmIoDto = {
  seq: number;
  at: number;
  model: string;
  protocol: string;
  finishReason: string;
  /** Provider end-to-end latency, excluding failover retries. */
  providerLatencyMs: number;
  inputTokens: number;
  outputTokens: number;
  request: LlmIoRequestDto;
  response: LlmIoResponseDto;
};

/**
 * Before/after snapshot for one captured file write. `contentAfter` is null
 * when the operation created or deleted the file (no "after" body to record).
 */
export type FileSpanDto = {
  path: string;
  contentBefore: string;
  contentAfter: string | null;
  /** The tool-call id from the model that produced this span (for 回链). */
  toolCallId: string;
  /** Wall-clock time the span was captured. */
  at: number;
};

/** List every artifact the turn persisted. Null in demo mode (no traces dir). */
export function listArtifacts(
  sessionId: string,
  turnSeq: number,
): Promise<ArtifactRefDto[] | null> {
  return read<ArtifactRefDto[]>("list_artifacts", { sessionId, turnSeq });
}

/** True when the turn has any persisted artifact (LLM I/O or file spans). */
export function turnHasArtifacts(sessionId: string, turnSeq: number): Promise<boolean | null> {
  return read<boolean>("turn_has_artifacts", { sessionId, turnSeq });
}

/** Load the full I/O for one model call. */
export function loadLlmIo(
  sessionId: string,
  turnSeq: number,
  refPath: string,
): Promise<LlmIoDto | null> {
  return read<LlmIoDto>("load_llm_io", { sessionId, turnSeq, refPath });
}

/** Load the before/after snapshot for one file write. */
export function loadFileSpan(
  sessionId: string,
  turnSeq: number,
  refPath: string,
): Promise<FileSpanDto | null> {
  return read<FileSpanDto>("load_file_span", { sessionId, turnSeq, refPath });
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
  hasKey?: boolean;
};

export function loadProvidersCommand(): Promise<ProviderDto[] | null> {
  return read<ProviderDto[]>("load_providers");
}

export function saveProvidersCommand(providers: ProviderDto[]): Promise<ProviderDto[] | null> {
  return write<ProviderDto[]>("save_providers", { providers });
}

/** Project-relative file paths for the Add context picker (ignore rules applied). */
export function listProjectFiles(project: string, query?: string): Promise<string[]> {
  if (!isDesktop()) return Promise.resolve([]);
  return invoke<string[]>("list_project_files", { project, query: query ?? null });
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
    await invoke<string>("read_context_file", { project, path });
    return true;
  } catch {
    return false;
  }
}

export function onRunEvent(handler: (event: RunEventDto) => void): Promise<() => void> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen<RunEventDto>(RUN_EVENT, (event) => handler(event.payload));
}
