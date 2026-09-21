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

export type ChangeDto = {
  path: string;
  added: number;
  removed: number;
};

export type ItemDto = {
  id: number;
  at: number;
  status: StepStatusDto;
  duration: number | null;
} & (
  | { kind: "reasoning"; summary: string }
  | { kind: "search"; query: string; detail: string }
  | { kind: "fileRead"; path: string; detail: string }
  | {
      kind: "commandExecution";
      command: string;
      cwd: string;
      output: string;
      exitCode: number | null;
    }
  | { kind: "modelCall"; model: string; inputTokens: number; outputTokens: number }
  | { kind: "fileChange"; changes: ChangeDto[] }
  | { kind: "agentMessage"; text: string; checks: string[] }
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

export function sendMessage(id: string, text: string, context: string[] = []): Promise<void> {
  return invoke<void>("send_message", { id, text, context });
}

export function stopRun(id: string): Promise<void | null> {
  return write<void>("stop_run", { id });
}

export function respondApproval(id: string, step: number, approved: boolean): Promise<void> {
  return invoke<void>("respond_approval", { id, step, approved });
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
