import type { Page } from "@playwright/test";
import {
  DEMO_CALENDAR_EVENTS,
  DEMO_RESULTS,
  DEMO_TIMEZONE,
  DEMO_TODO_ACTIVITY,
  DEMO_TODO_ATTACHMENTS,
  DEMO_TODO_PEOPLE,
  DEMO_TODO_REMINDERS,
  DEMO_TODO_STATUSES,
  DEMO_TODO_TAGS,
  DEMO_TODOS,
  DEMO_WORKBENCH_APPS,
  DEMO_WORKBENCH_PREFS,
} from "../../src/data/workbench";

/**
 * The workbench rows the stub keeps in memory. Deliberately loose: the stub
 * re-implements the store's *behavior* (versions, idempotency, archive), not
 * its types — those live in `src/api.ts` and are checked there.
 */
type WbApp = {
  manifest: {
    appId: string;
    name: string;
    description: string;
    version: string;
    apiVersion: string;
    icon: string;
    builtin: boolean;
    capabilities: { ai: boolean; scheduler: boolean; summary: boolean };
  };
  preference: {
    appId: string;
    enabled: boolean;
    favorite: boolean;
    sortOrder: number;
    aiEnabled: boolean;
    scheduleEnabled: boolean;
    updatedAt: number;
  };
};

type WbTodo = {
  id: string;
  title: string;
  notes: string;
  /** An id into `WbStatus`, not one of three literals. */
  status: string;
  priority: string;
  dueDate: string | null;
  dueTime: string | null;
  parentId: string | null;
  assigneeId: string | null;
  itemType: string;
  module: string;
  estimateHours: number | null;
  actualHours: number | null;
  archived: boolean;
  completedAt: number | null;
  sortOrder: number;
  source: string;
  version: number;
  createdAt: number;
  updatedAt: number;
  tagIds: string[];
  linkIds: string[];
};

/** A board column. `isDone` is the only place "done" is defined. */
type WbStatus = {
  id: string;
  name: string;
  color: string;
  isDone: boolean;
  builtin: boolean;
  sortOrder: number;
  version: number;
  createdAt: number;
  updatedAt: number;
};

type WbTag = {
  id: string;
  name: string;
  color: string;
  sortOrder: number;
  version: number;
  createdAt: number;
  updatedAt: number;
};

type WbPerson = {
  id: string;
  name: string;
  color: string;
  createdAt: number;
};

type WbAttachment = {
  id: string;
  todoId: string;
  name: string;
  path: string;
  size: number | null;
  mime: string;
  version: number;
  createdAt: number;
  updatedAt: number;
};

type WbReminder = {
  id: string;
  todoId: string;
  kind: string;
  offsetMinutes: number | null;
  message: string;
  enabled: boolean;
  version: number;
  createdAt: number;
  updatedAt: number;
};

type WbActivity = {
  id: string;
  todoId: string;
  kind: string;
  authorId: string | null;
  body: string;
  meta: string;
  createdAt: number;
};

type WbEvent = {
  id: string;
  title: string;
  notes: string;
  date: string;
  startTime: string | null;
  endTime: string | null;
  allDay: boolean;
  timezone: string;
  version: number;
  createdAt: number;
  updatedAt: number;
};

type WbResult = {
  artifact: {
    id: string;
    title: string;
    kind: string;
    body: string | null;
    filePath: string | null;
    fileSize: number | null;
    fileModifiedAt: number | null;
    source: string;
    category: string;
    favorite: boolean;
    archived: boolean;
    version: number;
    createdAt: number;
    updatedAt: number;
  };
  fileStatus: string;
  currentSize: number | null;
  currentModifiedAt: number | null;
};

/** What the workbench commands are seeded with; overridable per test. */
type WorkbenchSeed = {
  apps: WbApp[];
  todos: WbTodo[];
  todoStatuses: WbStatus[];
  todoTags: WbTag[];
  todoPeople: WbPerson[];
  todoAttachments: WbAttachment[];
  todoReminders: WbReminder[];
  todoActivity: WbActivity[];
  events: WbEvent[];
  results: WbResult[];
  prefs: Record<string, string>;
  timezone: string;
};

/** What the stand-in core answers. Anything omitted gets an inert default. */
export type Core = {
  /** The `workspace` payload. `null` makes the call fail, as an unset HOME does. */
  workspace?: unknown;
  /** The `load_session` payload. */
  session?: unknown;
  /** Additional sessions keyed by id for project-switching tests. */
  sessions?: Record<string, unknown>;
  /** What `git_branch` reports. */
  branch?: string | null;
  /** `list_archived` payload. */
  archived?: unknown[];
  /** `load_providers` payload. */
  providers?: unknown[];
  /** `setting` values by key. */
  settings?: Record<string, string>;
  /** Project-relative files for Add context (`list_project_files`). */
  files?: string[];
  /** Paths returned by the OS file dialog (`pick_files`) for 添加照片和文件. */
  pickFiles?: string[];
  /** Path returned by the OS folder dialog for adding a project. */
  pickFolder?: string | null;
  /** Absolute external files the stub treats as readable. */
  externalFiles?: string[];
  /** When true, `send_message` always rejects (failed-send recovery UX). */
  failSend?: boolean;
  /** When true, `respond_approval` always rejects. */
  failApproval?: boolean;
  /** When true, `retitle_session` / `archive_session` reject. */
  failRename?: boolean;
  failArchive?: boolean;
  /** When true, `delete_session` rejects. */
  failDelete?: boolean;
  /** When true, `read_context_file` rejects (context read failure). */
  failContextRead?: boolean;
  /** Captured `send_message` payloads for assertions. `agents` is the full
   *  persona chain (length 1 = single, length > 1 = chain); `agent` is the
   *  chain head kept for backwards-compat assertions. */
  sendLog?: Array<{
    id: string;
    text: string;
    context: string[];
    project?: string;
    agent?: string | null;
    agents?: string[];
    budgetFamily?: string | null;
  }>;
  /**
   * Make `send_message` finish its run before resolving — the exact race a
   * create-send has against its own first turn (events land before the new
   * session id is known). `plain` marks a conversational answer.
   */
  autoRun?: { answer: string; plain?: boolean };
  /** Captured `respond_approval` payloads. */
  approvalLog?: Array<{
    id: string;
    step: number;
    approved: boolean;
    sessionWide: boolean;
    runId?: number;
  }>;
  /** `turn_changes` payload for inline diffs. */
  turnChanges?: Array<{
    path: string;
    diff: string;
    userPreexisting?: boolean;
    undoState?: string;
    conflict?: boolean;
  }>;
  /** Full file bodies for the file viewer (`read_project_file`). */
  fileBodies?: Record<string, string>;
  /** Image paths the stub treats as >4MB — `probe_context_file` then returns
   *  `preview: null`, like the Rust cap on inline image previews. */
  oversizedImages?: string[];
  /** `undo_turn` payload; default is an empty (nothing-to-undo) report. */
  undoReport?: {
    restored: string[];
    conflicts: Array<{ path: string; reason: string; message: string }>;
  };
  /** When true, `undo_turn` rejects. */
  failUndo?: boolean;
  /** `list_provider_models` payload by template id. */
  providerModels?: Record<string, Array<{ modelId: string; displayName: string }>>;
  /** When set, `list_provider_models` rejects with this reason. */
  failListModels?: string;
  /** `list_skills` payload — the user's own skills; stateful like the store. */
  skills?: unknown[];
  /** `read_skill` payload by name; builtins are configured here in tests. */
  skillDetails?: Record<string, unknown>;
  /** When set, `save_skill` rejects with this reason. */
  failSaveSkill?: string;
  /** `list_agents` payload — persona records; stateful like the skill store. */
  agents?: unknown[];
  /** When set, `save_agent` rejects with this reason. */
  failSaveAgent?: string;
  /** `load_mcp_servers` payload; stateful like the settings store. */
  mcpServers?: unknown[];
  /** `mcp_list_tools` payload by server id (name + description objects). */
  mcpTools?: Record<string, unknown[]>;
  /** When set, `mcp_list_tools` rejects with this reason. */
  failMcpTools?: string;
  /** 工作台 app views; defaults to the frozen `DEMO_WORKBENCH_APPS`. */
  workbenchApps?: WbApp[];
  /** 待办 rows; defaults to the frozen `DEMO_TODOS`. */
  todos?: WbTodo[];
  /** 待办 board columns; defaults to `DEMO_TODO_STATUSES`. */
  todoStatuses?: WbStatus[];
  /** 待办 tags; defaults to `DEMO_TODO_TAGS`. */
  todoTags?: WbTag[];
  /** 待办 people; defaults to `DEMO_TODO_PEOPLE`. */
  todoPeople?: WbPerson[];
  /** 待办 attachments; defaults to `DEMO_TODO_ATTACHMENTS`. */
  todoAttachments?: WbAttachment[];
  /** 待办 reminders; defaults to `DEMO_TODO_REMINDERS`. */
  todoReminders?: WbReminder[];
  /** 待办 activity log; defaults to `DEMO_TODO_ACTIVITY`. */
  todoActivity?: WbActivity[];
  /** 日历 events; defaults to the frozen `DEMO_CALENDAR_EVENTS`. */
  calendarEvents?: WbEvent[];
  /** 成果中心 rows; defaults to the frozen `DEMO_RESULTS`. */
  results?: WbResult[];
  /** 工作台 key/value preferences; defaults to `DEMO_WORKBENCH_PREFS`. */
  workbenchPrefs?: Record<string, string>;
  /** IANA zone a new event defaults to; defaults to `DEMO_TIMEZONE`. */
  calendarTimezone?: string;
};

/**
 * Stands in for the IPC bridge.
 *
 * The suite runs in a plain browser, where there is no core at all:
 * `@tauri-apps/api` reaches `window.__TAURI_INTERNALS__` directly and
 * `isDesktop()` only checks that the key exists.
 */
export async function stubShell(page: Page, core: Core = {}): Promise<void> {
  // The workbench fixtures ride in the same payload as the rest of the stub:
  // `addInitScript` serializes the function alone, so nothing can be imported
  // inside it and every default has to travel in as data.
  const seeded: Core & { workbench: WorkbenchSeed } = {
    ...core,
    workbench: {
      apps: core.workbenchApps ?? DEMO_WORKBENCH_APPS,
      todos: core.todos ?? DEMO_TODOS,
      todoStatuses: core.todoStatuses ?? DEMO_TODO_STATUSES,
      todoTags: core.todoTags ?? DEMO_TODO_TAGS,
      todoPeople: core.todoPeople ?? DEMO_TODO_PEOPLE,
      todoAttachments: core.todoAttachments ?? DEMO_TODO_ATTACHMENTS,
      todoReminders: core.todoReminders ?? DEMO_TODO_REMINDERS,
      todoActivity: core.todoActivity ?? DEMO_TODO_ACTIVITY,
      events: core.calendarEvents ?? DEMO_CALENDAR_EVENTS,
      results: core.results ?? DEMO_RESULTS,
      prefs: core.workbenchPrefs ?? DEMO_WORKBENCH_PREFS,
      timezone: core.calendarTimezone ?? DEMO_TIMEZONE,
    },
  };
  await page.addInitScript((config: Core & { workbench: WorkbenchSeed }) => {
    // E2E / visual-regression flag (2026-10-13): when the production
    // Inspector default was flipped to "closed on every viewport",
    // `routes.ts` started honoring `__KODO_INSPECTOR_AUTOPEN__` as the
    // carve-out that keeps the wide-viewport autopen behavior the
    // visual suite was already asserting. The desktop / web paths
    // never set this flag, so production users see the new default
    // (rail only) and the test suite keeps its existing geometry
    // assertions. Mirrored in `routes.ts::parse`.
    (window as unknown as { __KODO_INSPECTOR_AUTOPEN__?: boolean }).__KODO_INSPECTOR_AUTOPEN__ =
      true;
    const callbacks: Record<number, (event: unknown) => void> = {};
    /** plugin:event|listen eventId → transformCallback handler id. */
    const listenerByEventId: Record<number, number> = {};
    let next = 1;
    const settings: Record<string, string> = {
      // Pre-onboarded unless a test opts back in — the first-run overlay would
      // otherwise block every stubbed flow.
      "onboarding-done": "true",
      ...(config.settings ?? {}),
    };
    const settingsLog: Array<{ key: string; value: string }> = [];
    (window as unknown as { __settingsLog?: unknown }).__settingsLog = settingsLog;
    let session = config.session ?? null;
    let workspace = config.workspace ?? { projects: [], sessions: [] };
    const files = config.files ?? ["src/app.ts", "src/util.ts", "README.md", "package.json"];
    const sendLog: Array<{
      id: string;
      text: string;
      context: string[];
      project?: string;
      agent?: string | null;
      agents?: string[];
      budgetFamily?: string | null;
    }> = [];
    (window as unknown as { __sendLog?: unknown }).__sendLog = sendLog;
    const approvalLog: Array<{
      id: string;
      step: number;
      approved: boolean;
      sessionWide: boolean;
      runId?: number;
    }> = [];
    (window as unknown as { __approvalLog?: unknown }).__approvalLog = approvalLog;
    const listModelCalls: Array<{ template: string; endpoint: string; providerId?: string }> = [];
    (window as unknown as { __listModelCalls?: unknown }).__listModelCalls = listModelCalls;
    const openFileLog: string[] = [];
    (window as unknown as { __openFileLog?: unknown }).__openFileLog = openFileLog;
    const revealFileLog: Array<{ project: string; path: string }> = [];
    (window as unknown as { __revealFileLog?: unknown }).__revealFileLog = revealFileLog;
    /** One entry per `task_feedback` completion ping. */
    const attentionLog: string[] = [];
    (window as unknown as { __attentionLog?: unknown }).__attentionLog = attentionLog;
    const deleteLog: string[] = [];
    (window as unknown as { __deleteLog?: unknown }).__deleteLog = deleteLog;
    const providersLog: unknown[] = [];
    (window as unknown as { __providersLog?: unknown }).__providersLog = providersLog;
    const browseLog: Array<{ command: string; args?: Record<string, unknown> }> = [];
    (window as unknown as { __browseLog?: unknown }).__browseLog = browseLog;
    let providersState = (config.providers ?? []) as Array<Record<string, unknown>>;
    /** `save_skill` / `delete_skill` calls, for asserting CRUD reached the core. */
    const skillOps: Array<{ op: "save" | "delete"; name: string }> = [];
    (window as unknown as { __skillOps?: unknown }).__skillOps = skillOps;
    let skillsState = (config.skills ?? []) as Array<Record<string, unknown>>;
    /** Persona CRUD (`save_agent` / `delete_agent`) + `__agentsLog` order log. */
    let agentsState = (config.agents ?? []) as Array<Record<string, unknown>>;
    // The frontend stub doesn't load the 12 `include_str!` builtins — the
    // real Tauri shell does. Tests that need them pass them through
    // `config.agents` with `source: "builtin"`; everything else defaults
    // to `"user"` (matches what the desktop shell does for any save).
    agentsState = agentsState.map((record) => ({
      ...record,
      source: record.source ?? "user",
    }));
    // Persist across `page.goto` reloads — `addInitScript` re-runs on every
    // navigation, but tests like `creating a user agent…` navigate between
    // Plugins and Conversation. Mirror the latest state on `window` so the
    // next init script picks up where the previous stub left off. Only
    // inherit when this call doesn't override `agents` explicitly — a fresh
    // `config.agents` always wins so each test starts from a clean slate.
    type _Refresh = { __agentsState?: Array<Record<string, unknown>> };
    const win = window as unknown as _Refresh;
    if (!("agents" in config) && Array.isArray(win.__agentsState)) {
      agentsState = win.__agentsState;
    }
    const commitAgents = () => {
      win.__agentsState = [...agentsState];
    };
    commitAgents();
    const agentsLog: Array<{ op: "save" | "delete"; name: string }> = [];
    (window as unknown as { __agentsLog?: unknown }).__agentsLog = agentsLog;
    const mcpServersLog: unknown[] = [];
    (window as unknown as { __mcpServersLog?: unknown }).__mcpServersLog = mcpServersLog;
    let mcpServersState = (config.mcpServers ?? []) as Array<Record<string, unknown>>;
    /** Server ids `mcp_list_tools` was asked about, in order. */
    const mcpToolsCalls: string[] = [];
    (window as unknown as { __mcpToolsCalls?: unknown }).__mcpToolsCalls = mcpToolsCalls;

    // ── 工作台 ─────────────────────────────────────────────────────────────
    // Stateful like `kodo_shell::apps::AppStore`: versions advance on every
    // write, an unknown id is `not_found`, a stale `expectedVersion` is
    // `revision_conflict`, and a repeated `requestId` replays the first row.
    // The whole state is JSON round-tripped so nothing the page does can reach
    // back into the payload Playwright handed over.
    const deepCopy = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
    const seedWb = config.workbench;
    let wbApps = deepCopy(seedWb.apps);
    let wbTodos = deepCopy(seedWb.todos);
    let wbStatuses = deepCopy(seedWb.todoStatuses);
    let wbTags = deepCopy(seedWb.todoTags);
    let wbPeople = deepCopy(seedWb.todoPeople);
    let wbAttachments = deepCopy(seedWb.todoAttachments);
    let wbReminders = deepCopy(seedWb.todoReminders);
    let wbActivity = deepCopy(seedWb.todoActivity);
    let wbEvents = deepCopy(seedWb.events);
    let wbResults = deepCopy(seedWb.results);
    const wbPrefs = { ...seedWb.prefs };
    const wbRequests = new Map<string, string>();
    let wbSeq = 0;
    // The fixture's clock, so `completedAt` / `updatedAt` are reproducible.
    const wbNow = (): number => Math.floor(Date.parse("2026-09-30T10:24:32Z") / 1000);
    const wbId = (prefix: string): string => `${prefix}-stub-${(wbSeq += 1)}`;
    /** Every workbench command the page sent, for order/arguments assertions. */
    const wbLog: Array<{ command: string; args: Record<string, unknown> }> = [];
    (window as unknown as { __wbLog?: unknown }).__wbLog = wbLog;

    const wbStale = (actual: number, expected: number): void => {
      if (actual !== expected) throw "revision_conflict";
    };
    const wbStamp = <T extends { version: number; updatedAt: number }>(row: T): T => {
      row.version += 1;
      row.updatedAt = wbNow();
      return row;
    };
    const wbApp = (appId: string): WbApp => {
      const view = wbApps.find((app) => app.manifest.appId === appId);
      if (!view) throw "unknown_app";
      return view;
    };
    const wbTodo = (id: string): WbTodo => {
      const row = wbTodos.find((todo) => todo.id === id);
      if (!row) throw "not_found";
      return row;
    };
    const wbStatus = (id: string): WbStatus => {
      const row = wbStatuses.find((status) => status.id === id);
      if (!row) throw "not_found";
      return row;
    };
    /** `isDone` lives on the status row and nowhere else. */
    const wbIsDone = (status: string): boolean =>
      Boolean(wbStatuses.find((row) => row.id === status)?.isDone);
    /** What a todo looks like on the wire: the stored row plus derived `isDone`. */
    const wbTodoOut = (row: WbTodo): WbTodo & { isDone: boolean } => ({
      ...row,
      isDone: wbIsDone(row.status),
    });
    /** Entering a done status stamps the moment; leaving one clears it. */
    const wbCompletedFor = (
      status: string,
      wasDone: boolean,
      previous: number | null,
    ): number | null => {
      if (!wbIsDone(status)) return null;
      return wasDone ? previous : wbNow();
    };
    const wbEvent = (id: string): WbEvent => {
      const row = wbEvents.find((event) => event.id === id);
      if (!row) throw "not_found";
      return row;
    };
    const wbResult = (id: string): WbResult => {
      const row = wbResults.find((entry) => entry.artifact.id === id);
      if (!row) throw "not_found";
      return row;
    };
    /** Favorites first, then `sortOrder`, then id — the grid's order. */
    const wbSort = (apps: WbApp[]): WbApp[] =>
      [...apps].sort((a, b) => {
        const star = Number(b.preference.favorite) - Number(a.preference.favorite);
        if (star !== 0) return star;
        const order = a.preference.sortOrder - b.preference.sortOrder;
        if (order !== 0) return order;
        return a.manifest.appId.localeCompare(b.manifest.appId);
      });

    (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
      transformCallback: (callback: (event: unknown) => void) => {
        const id = next++;
        callbacks[id] = callback;
        return id;
      },
      invoke: async (command: string, args?: Record<string, unknown>) => {
        switch (command) {
          case "plugin:event|listen": {
            const eventId = next++;
            listenerByEventId[eventId] = Number(args?.handler ?? 0);
            return eventId;
          }
          case "plugin:event|unlisten": {
            const handlerId = listenerByEventId[Number(args?.eventId ?? 0)];
            if (handlerId) delete callbacks[handlerId];
            delete listenerByEventId[Number(args?.eventId ?? 0)];
            return null;
          }
          case "core_info":
            return { name: "kodo-core", version: "0.0.1" };
          case "workspace":
            if (config.workspace === null)
              throw "HOME is not set, so there is nowhere to keep the project list";
            return workspace;
          case "load_session":
            return (
              (config.sessions as Record<string, unknown> | undefined)?.[String(args?.id ?? "")] ??
              session
            );
          case "git_branch":
            return config.branch ?? null;
          case "pick_folder":
            return config.pickFolder ?? null;
          case "add_project": {
            const path = String(args?.path ?? "");
            const current = workspace as { projects?: Array<{ path: string; name: string }> };
            if (
              path &&
              Array.isArray(current.projects) &&
              !current.projects.some((item) => item.path === path)
            ) {
              const segments = path.split("/").filter(Boolean);
              const name = segments[segments.length - 1] ?? path;
              workspace = {
                ...(workspace as Record<string, unknown>),
                projects: [...current.projects, { path, name }],
              };
            }
            return workspace;
          }
          case "remove_project": {
            const path = String(args?.path ?? "");
            const current = workspace as { projects?: Array<{ path: string; name: string }> };
            if (path && Array.isArray(current.projects)) {
              workspace = {
                ...(workspace as Record<string, unknown>),
                projects: current.projects.filter((item) => item.path !== path),
              };
            }
            return workspace;
          }
          case "list_archived":
            return config.archived ?? [];
          case "load_providers":
            return providersState.map((p) => ({
              ...(p as Record<string, unknown>),
              apiKey: (p as { apiKey?: string })?.apiKey ?? "",
              hasKey: Boolean((p as { apiKey?: string })?.apiKey),
            }));
          case "save_providers": {
            // Stateful like the real store: later `load_providers` sees it.
            const incoming = (args?.providers ?? providersState) as Array<Record<string, unknown>>;
            providersState = incoming;
            providersLog.push(incoming);
            return providersState;
          }
          case "load_mcp_servers":
            return mcpServersState;
          case "save_mcp_servers": {
            // Full-list save (arg key `servers`), like `save_mcp_servers` in the
            // Tauri shell; the log records what the UI sent for mask writeback
            // assertions.
            const incoming = (args?.servers ?? mcpServersState) as Array<Record<string, unknown>>;
            mcpServersState = incoming;
            mcpServersLog.push(incoming);
            return mcpServersState;
          }
          case "mcp_list_tools": {
            // Connect-and-list, stubbed: unknown ids answer empty like a
            // server with no tools; `failMcpTools` models a broken transport.
            const id = String(args?.id ?? "");
            mcpToolsCalls.push(id);
            if (config.failMcpTools) throw config.failMcpTools;
            return (config.mcpTools ?? {})[id] ?? [];
          }
          case "list_provider_models": {
            listModelCalls.push({
              template: String(args?.template ?? ""),
              endpoint: String(args?.endpoint ?? ""),
              providerId: (args?.providerId as string | undefined) ?? undefined,
            });
            if (config.failListModels) throw config.failListModels;
            return config.providerModels?.[String(args?.template ?? "")] ?? [];
          }
          case "setting":
            return settings[args?.key as string] ?? null;
          case "set_setting":
            if (args?.key) {
              settings[args.key as string] = String(args.value ?? "");
              settingsLog.push({ key: args.key as string, value: String(args.value ?? "") });
            }
            return null;
          case "retitle_session": {
            if (config.failRename) throw "rename failed";
            if (session && typeof session === "object") {
              session = { ...(session as Record<string, unknown>), title: args?.title };
            }
            // The core re-scans session logs into the workspace list, so a
            // retitle lands there too — the top-bar crumb reads that payload.
            const listed = workspace as { sessions?: Array<Record<string, unknown>> };
            if (Array.isArray(listed.sessions)) {
              workspace = {
                ...(workspace as Record<string, unknown>),
                sessions: listed.sessions.map((item) =>
                  item.id === String(args?.id ?? "") ? { ...item, title: args?.title } : item,
                ),
              };
            }
            return session;
          }
          case "archive_session":
            if (config.failArchive) throw "archive failed";
            return workspace;
          case "restore_session":
            return workspace;
          case "delete_session": {
            if (config.failDelete) throw "delete failed";
            const id = String(args?.id ?? "");
            deleteLog.push(id);
            const current = workspace as { projects?: unknown[]; sessions?: Array<{ id: string }> };
            if (Array.isArray(current.sessions)) {
              workspace = {
                ...(workspace as Record<string, unknown>),
                sessions: current.sessions.filter((item) => item.id !== id),
              };
            }
            return workspace;
          }
          case "respond_approval":
            if (config.failApproval) throw "approval response failed";
            approvalLog.push({
              id: String(args?.id ?? ""),
              step: Number(args?.step ?? 0),
              approved: Boolean(args?.approved),
              sessionWide: Boolean((args as { sessionWide?: unknown })?.sessionWide),
              ...(args?.runId !== undefined ? { runId: Number(args.runId) } : {}),
            });
            return null;
          case "turn_changes":
            return (config.turnChanges ?? []).map((c) => ({
              path: c.path,
              diff: c.diff,
              userPreexisting: c.userPreexisting ?? false,
              undoState: c.undoState ?? "clean",
              conflict: c.conflict ?? false,
            }));
          case "undo_turn":
            if (config.failUndo) throw "undo failed";
            return config.undoReport ?? { restored: [], conflicts: [] };
          case "open_project_file": {
            const path = String(args?.path ?? "");
            if (!path || path.includes("..") || path.startsWith("/")) {
              throw `file outside project refused: ${path}`;
            }
            openFileLog.push(path);
            return null;
          }
          case "reveal_file": {
            const path = String(args?.path ?? "");
            if (!path || path.includes("..") || path.startsWith("/")) {
              throw `file outside project refused: ${path}`;
            }
            revealFileLog.push({ project: String(args?.project ?? ""), path });
            return null;
          }
          case "task_feedback":
            attentionLog.push("ping");
            return null;
          case "read_project_file": {
            if (config.failContextRead) throw "cannot read file outside project";
            const path = String(args?.path ?? "");
            if (!path || path.includes("..") || path.startsWith("/")) {
              throw "file outside project refused";
            }
            if (!files.includes(path)) throw `file not found: ${path}`;
            return config.fileBodies?.[path] ?? `// full content of ${path}\nconst stub = true;\n`;
          }
          case "list_project_files": {
            const q = String(args?.query ?? "").toLowerCase();
            // Mirror the real command: filter, then clamp+truncate the limit.
            const limit = Math.min(Math.max(Number(args?.limit ?? 200), 1), 5000);
            return files.filter((f) => !q || f.toLowerCase().includes(q)).slice(0, limit);
          }
          case "pick_files":
            // "添加照片和文件" — absolute paths the user would pick in the OS dialog.
            return (config as { pickFiles?: string[] }).pickFiles ?? files;
          case "read_context_file": {
            if (config.failContextRead) throw "cannot read file outside project";
            const path = String(args?.path ?? "");
            if (!path || path.includes("..")) {
              throw `invalid path: ${path}`;
            }
            if (path.startsWith("/")) {
              // Absolute external path is allowed when listed as pickable in the stub.
              const external = (config as { externalFiles?: string[] }).externalFiles ?? [];
              if (!external.includes(path)) throw `file not found: ${path}`;
              return `// preview of ${path}`;
            }
            if (!files.includes(path)) throw `file not found: ${path}`;
            return `// preview of ${path}`;
          }
          case "probe_context_file": {
            if (config.failContextRead) throw "cannot read file outside project";
            const path = String(args?.path ?? "");
            if (!path || path.includes("..")) {
              throw `invalid path: ${path}`;
            }
            const external = (config as { externalFiles?: string[] }).externalFiles ?? [];
            const inFiles = files.includes(path);
            const isExternal = path.startsWith("/") && external.includes(path);
            if (!inFiles && !isExternal) throw `file not found: ${path}`;
            const name = path.split("/").pop() ?? path;
            const ext = name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
            // Mirrors Rust `context_kind` (main.rs): unknown extensions land
            // here only after the stub's stand-in sniff hits — treated as
            // binary, exactly like Rust's NUL outcome.
            const kind = [
              "png",
              "jpg",
              "jpeg",
              "gif",
              "webp",
              "bmp",
              "svg",
              "ico",
              "avif",
              "heic",
            ].includes(ext)
              ? "image"
              : ext === "pdf"
                ? "pdf"
                : [
                      "doc",
                      "docx",
                      "rtf",
                      "odt",
                      "xls",
                      "xlsx",
                      "ods",
                      "ppt",
                      "pptx",
                      "odp",
                    ].includes(ext)
                  ? "office"
                  : [
                        "txt",
                        "md",
                        "markdown",
                        "json",
                        "jsonl",
                        "yml",
                        "yaml",
                        "toml",
                        "xml",
                        "html",
                        "css",
                        "js",
                        "jsx",
                        "ts",
                        "tsx",
                        "rs",
                        "py",
                        "go",
                        "java",
                        "kt",
                        "c",
                        "h",
                        "cpp",
                        "hpp",
                        "sh",
                        "zsh",
                        "bash",
                        "sql",
                        "log",
                        "csv",
                        "tsv",
                        "ini",
                        "cfg",
                        "conf",
                      ].includes(ext)
                    ? "text"
                    : "other";
            // Rust only inlines image data URLs (≤4MB); oversized images and
            // every other kind carry no preview.
            const oversized = (
              (config as { oversizedImages?: string[] }).oversizedImages ?? []
            ).includes(path);
            const preview =
              kind === "image" && !oversized
                ? // 1×1 PNG so image cards render a real thumbnail in tests.
                  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
                : kind === "text"
                  ? `// preview of ${path}`
                  : null;
            return { kind, name, size: oversized ? 5 * 1024 * 1024 : 1024, preview };
          }
          case "send_message": {
            if (config.failSend) throw "send failed: provider unavailable";
            const contextPaths = Array.isArray(args?.context) ? (args.context as string[]) : [];
            for (const p of contextPaths) {
              if (!p || p.includes("..")) throw `invalid context path: ${p}`;
              if (
                p.startsWith("/") &&
                !(config as { externalFiles?: string[] }).externalFiles?.includes(p)
              ) {
                throw `context file not found: ${p}`;
              }
            }
            sendLog.push({
              id: String(args?.id ?? ""),
              text: String(args?.text ?? ""),
              context: contextPaths,
              budgetFamily: typeof args?.budgetFamily === "string" ? args.budgetFamily : null,
              project: String(args?.project ?? ""),
              // Persona resolved at send time — null reaches the backend as
              // "no agent selected" (插件 · 智能体). `agents` is the full chain
              // (§2.6): length 1 = single persona, length > 1 = chain. We mirror
              // the head into `agent` for legacy single-selection assertions.
              agents: Array.isArray(args?.agents) ? (args.agents as string[]) : [],
              agent:
                Array.isArray(args?.agents) && args.agents.length > 0
                  ? String(args.agents[0])
                  : null,
            });
            const sessionId = String(args?.id ?? "") || "created-session";
            // The real core opens the session file before send resolves, so a
            // later workspace scan lists it — the crumb and sidebar read that.
            if (!String(args?.id ?? "")) {
              const listed = workspace as { sessions?: Array<Record<string, unknown>> };
              workspace = {
                ...(workspace as Record<string, unknown>),
                sessions: [
                  ...(Array.isArray(listed.sessions) ? listed.sessions : []),
                  {
                    id: sessionId,
                    project: String(args?.project ?? ""),
                    title: "新对话",
                    at: 1_700_000_000,
                    archived: false,
                  },
                ],
              };
            }
            const autoRun = (config as { autoRun?: { answer: string; plain?: boolean } }).autoRun;
            if (autoRun) {
              // Emit synchronously, before the promise resolves — the runner's
              // thread can complete the whole turn this fast.
              const emitToAll = (payload: unknown) => {
                for (const callback of Object.values(callbacks))
                  callback({ event: "run:event", id: 1, payload });
              };
              emitToAll({ type: "turnStarted", session: sessionId });
              emitToAll({
                type: "itemCompleted",
                session: sessionId,
                item: {
                  id: 1,
                  at: 1_700_000_000,
                  status: "done",
                  duration: 1,
                  kind: "agentMessage",
                  text: autoRun.answer,
                  checks: [],
                  delivery: "ready",
                  verification: "not_run",
                  plain: autoRun.plain ?? false,
                },
              });
              emitToAll({ type: "turnComplete", session: sessionId });
            }
            return sessionId;
          }
          case "stop_run":
            return null;
          case "list_skills":
            return skillsState;
          case "read_skill": {
            const name = String(args?.name ?? "");
            const fromStore = skillsState.find((skill) => skill.name === name);
            const detail = fromStore ?? config.skillDetails?.[name];
            if (!detail) throw "技能不存在";
            return detail;
          }
          case "save_skill": {
            if (config.failSaveSkill) throw config.failSaveSkill;
            const name = String(args?.name ?? "");
            const markdown = String(args?.markdown ?? "");
            // Derive the fields list/filter rely on, the way parse_skill would.
            const section = (heading: string) => {
              const match = markdown.match(new RegExp(`## ${heading}\\n([\\s\\S]*?)(?=\\n## |$)`));
              return match ? match[1] : "";
            };
            const taskTypes = section("applicable_task_types")
              .split("\n")
              .map((line) => line.trim())
              .filter((line) => line.startsWith("- "))
              .map((line) => line.slice(2));
            const firstBullet = (heading: string) =>
              section(heading)
                .split("\n")
                .map((line) => line.trim())
                .find((line) => line.startsWith("- "))
                ?.slice(2) ?? "";
            const view = {
              name,
              source: args?.scope === "project" ? "project" : "user",
              description:
                section("description").trim().split("\n")[0] || firstBullet("completion_criteria"),
              taskTypes: taskTypes.length > 0 ? taskTypes : ["docs"],
              strategy: section("context_strategy").trim().split("\n")[0] || "docs-focused",
              policy: section("verification_policy").trim().split("\n")[0] || "none",
              workflow: [],
              allowedTools: [],
              completionCriteria: [],
              guidance: section("guidance").trim(),
              markdown,
            };
            skillsState = [...skillsState.filter((skill) => skill.name !== name), view];
            skillOps.push({ op: "save", name });
            return view;
          }
          case "delete_skill": {
            const name = String(args?.name ?? "");
            skillOps.push({ op: "delete", name });
            skillsState = skillsState.filter((skill) => skill.name !== name);
            return null;
          }
          case "list_agents":
            return agentsState;
          case "read_agent": {
            const name = String(args?.name ?? "");
            const found = agentsState.find((record) => record.name === name);
            if (!found) throw "智能体不存在";
            return found;
          }
          case "save_agent": {
            if (config.failSaveAgent) throw config.failSaveAgent;
            const name = String(args?.name ?? "");
            const markdown = String(args?.markdown ?? "");
            // Derive the two sections the way parse_agent would.
            const section = (heading: string) => {
              const match = markdown.match(new RegExp(`## ${heading}\\n([\\s\\S]*?)(?=\\n## |$)`));
              return match ? match[1].trim() : "";
            };
            const view = {
              name,
              // The core rejects builtin names; the stub mirrors that for
              // tests that drive the failure path through `save_agent`.
              source: "user" as const,
              description: section("description"),
              instructions: section("instructions"),
              markdown,
            };
            agentsState = [...agentsState.filter((record) => record.name !== name), view];
            commitAgents();
            agentsLog.push({ op: "save", name });
            return view;
          }
          case "delete_agent": {
            const name = String(args?.name ?? "");
            if (!agentsState.some((record) => record.name === name)) throw "智能体不存在";
            agentsState = agentsState.filter((record) => record.name !== name);
            commitAgents();
            agentsLog.push({ op: "delete", name });
            return null;
          }
          case "browse_open":
          case "browse_close":
          case "browse_set_url":
          case "browse_set_position":
            browseLog.push({ command, args });
            return null;

          // ── 工作台 ───────────────────────────────────────────────────────
          case "list_apps":
            wbLog.push({ command, args: args ?? {} });
            return deepCopy(wbSort(wbApps));
          case "set_app_enabled": {
            wbLog.push({ command, args: args ?? {} });
            const view = wbApp(String(args?.appId ?? ""));
            view.preference.enabled = Boolean(args?.enabled);
            view.preference.updatedAt = wbNow();
            return deepCopy(view);
          }
          case "set_app_favorite": {
            wbLog.push({ command, args: args ?? {} });
            const view = wbApp(String(args?.appId ?? ""));
            view.preference.favorite = Boolean(args?.favorite);
            view.preference.updatedAt = wbNow();
            return deepCopy(view);
          }
          case "set_app_order": {
            wbLog.push({ command, args: args ?? {} });
            const ids = Array.isArray(args?.appIds) ? (args.appIds as string[]) : [];
            ids.forEach((appId, index) => {
              wbApp(appId).preference.sortOrder = index;
            });
            return deepCopy(wbSort(wbApps));
          }
          case "set_app_links": {
            wbLog.push({ command, args: args ?? {} });
            const view = wbApp(String(args?.appId ?? ""));
            const caps = view.manifest.capabilities;
            // The store refuses a link the manifest never declared.
            if (args?.aiEnabled && !caps.ai) throw "invalid_input";
            if (args?.scheduleEnabled && !caps.scheduler) throw "invalid_input";
            view.preference.aiEnabled = Boolean(args?.aiEnabled);
            view.preference.scheduleEnabled = Boolean(args?.scheduleEnabled);
            view.preference.updatedAt = wbNow();
            return deepCopy(view);
          }
          case "get_workbench_prefs":
            wbLog.push({ command, args: args ?? {} });
            return { ...wbPrefs };
          case "set_workbench_pref":
            wbLog.push({ command, args: args ?? {} });
            wbPrefs[String(args?.key ?? "")] = String(args?.value ?? "");
            return null;
          case "list_todos": {
            wbLog.push({ command, args: args ?? {} });
            const includeArchived = Boolean(args?.includeArchived);
            const rows = wbTodos
              .filter((todo) => includeArchived || !todo.archived)
              .sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt - b.createdAt);
            return deepCopy(rows.map(wbTodoOut));
          }
          case "create_todo": {
            wbLog.push({ command, args: args ?? {} });
            const draft = (args?.draft ?? {}) as Record<string, unknown>;
            const requestId = draft.requestId ? String(draft.requestId) : "";
            const replay = requestId ? wbRequests.get(requestId) : undefined;
            const seen = replay ? wbTodos.find((todo) => todo.id === replay) : undefined;
            // A repeated request replays the first row instead of adding one.
            if (seen) return deepCopy(wbTodoOut(seen));
            const title = String(draft.title ?? "").trim();
            if (!title) throw "invalid_input";
            const status = wbStatus(String(draft.status ?? "todo")).id;
            const todo: WbTodo = {
              id: wbId("todo"),
              title,
              notes: String(draft.notes ?? ""),
              status,
              priority: String(draft.priority ?? "normal"),
              dueDate: draft.dueDate ? String(draft.dueDate) : null,
              dueTime: draft.dueTime ? String(draft.dueTime) : null,
              parentId: draft.parentId ? String(draft.parentId) : null,
              assigneeId: draft.assigneeId ? String(draft.assigneeId) : null,
              itemType: String(draft.itemType ?? "task"),
              module: String(draft.module ?? ""),
              estimateHours: draft.estimateHours == null ? null : Number(draft.estimateHours),
              actualHours: draft.actualHours == null ? null : Number(draft.actualHours),
              archived: false,
              completedAt: wbIsDone(status) ? wbNow() : null,
              sortOrder: wbTodos.filter((row) => row.status === status).length,
              source: String(draft.source ?? "manual"),
              version: 1,
              createdAt: wbNow(),
              updatedAt: wbNow(),
              tagIds: (draft.tagIds as string[] | undefined) ?? [],
              linkIds: [],
            };
            wbTodos = [...wbTodos, todo];
            if (requestId) wbRequests.set(requestId, todo.id);
            return deepCopy(wbTodoOut(todo));
          }
          case "update_todo": {
            wbLog.push({ command, args: args ?? {} });
            const incoming = (args?.todo ?? {}) as WbTodo & { isDone?: boolean };
            const row = wbTodo(String(incoming.id ?? ""));
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            const title = String(incoming.title ?? "").trim();
            if (!title) throw "invalid_input";
            if (incoming.parentId && incoming.parentId === row.id) throw "invalid_input";
            const status = wbStatus(String(incoming.status ?? row.status)).id;
            const wasDone = wbIsDone(row.status);
            // Archived is not the editor's to change, and the links have their
            // own write path — the store ignores both, so the stub does too.
            Object.assign(row, {
              title,
              notes: incoming.notes ?? "",
              status,
              priority: incoming.priority ?? row.priority,
              dueDate: incoming.dueDate ?? null,
              dueTime: incoming.dueTime ?? null,
              parentId: incoming.parentId ?? null,
              assigneeId: incoming.assigneeId ?? null,
              itemType: incoming.itemType ?? "task",
              module: incoming.module ?? "",
              estimateHours: incoming.estimateHours ?? null,
              actualHours: incoming.actualHours ?? null,
              completedAt: wbCompletedFor(status, wasDone, row.completedAt),
              tagIds: incoming.tagIds ?? row.tagIds,
            });
            return deepCopy(wbTodoOut(wbStamp(row)));
          }
          case "set_todo_status": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbTodo(String(args?.id ?? ""));
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            const status = wbStatus(String(args?.status ?? "todo")).id;
            const wasDone = wbIsDone(row.status);
            row.status = status;
            row.completedAt = wbCompletedFor(status, wasDone, row.completedAt);
            return deepCopy(wbTodoOut(wbStamp(row)));
          }
          case "set_todo_archived": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbTodo(String(args?.id ?? ""));
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            row.archived = Boolean(args?.archived);
            return deepCopy(wbTodoOut(wbStamp(row)));
          }
          case "delete_todo": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbTodo(String(args?.id ?? ""));
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            // Sub-items first: `parent_id` would otherwise promote them.
            wbTodos = wbTodos.filter((entry) => entry.parentId !== row.id && entry.id !== row.id);
            return null;
          }
          case "reorder_todo": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbTodo(String(args?.id ?? ""));
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            const status = wbStatus(String(args?.status ?? row.status)).id;
            const beforeId = args?.beforeId ? String(args.beforeId) : "";
            const ordered = wbTodos
              .filter((entry) => entry.status === status && !entry.archived && entry.id !== row.id)
              .sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt - b.createdAt)
              .map((entry) => entry.id);
            const at = beforeId ? ordered.indexOf(beforeId) : -1;
            ordered.splice(at < 0 ? ordered.length : at, 0, row.id);
            ordered.forEach((id, slot) => {
              const entry = wbTodo(id);
              if (id === row.id) {
                const wasDone = wbIsDone(entry.status);
                entry.status = status;
                entry.completedAt = wbCompletedFor(status, wasDone, entry.completedAt);
                entry.sortOrder = slot;
                wbStamp(entry);
              } else {
                // A shifting slot is invisible to the sibling: no version bump.
                entry.sortOrder = slot;
              }
            });
            return deepCopy(wbTodoOut(row));
          }
          case "set_todo_tags": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbTodo(String(args?.id ?? ""));
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            const tagIds = (args?.tagIds as string[] | undefined) ?? [];
            for (const tagId of tagIds) {
              if (!wbTags.some((tag) => tag.id === tagId)) throw "not_found";
            }
            row.tagIds = [...tagIds];
            return deepCopy(wbTodoOut(wbStamp(row)));
          }
          case "set_todo_links": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbTodo(String(args?.id ?? ""));
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            const linkIds = (args?.linkIds as string[] | undefined) ?? [];
            for (const other of linkIds) {
              if (other === row.id) throw "invalid_input";
              wbTodo(other);
            }
            for (const entry of wbTodos) {
              if (entry.id === row.id) {
                entry.linkIds = [...linkIds];
                continue;
              }
              const kept = entry.linkIds.filter((link) => link !== row.id);
              for (const other of linkIds) {
                if (other === entry.id && !kept.includes(row.id)) kept.push(row.id);
              }
              entry.linkIds = kept;
            }
            return deepCopy(wbTodoOut(wbStamp(row)));
          }
          case "bulk_update_todos": {
            wbLog.push({ command, args: args ?? {} });
            const ids = (args?.ids as string[] | undefined) ?? [];
            const op = (args?.op ?? {}) as { op?: string; status?: string; priority?: string; archived?: boolean };
            const rows = ids.map((id) => wbTodo(id));
            if (rows.length === 0) return [];
            if (op.op === "setStatus") {
              const status = wbStatus(String(op.status ?? "todo")).id;
              const done = wbIsDone(status);
              for (const row of rows) {
                row.status = status;
                // Already-done rows keep the moment they became done.
                row.completedAt = done ? (row.completedAt ?? wbNow()) : null;
                wbStamp(row);
              }
            } else if (op.op === "setPriority") {
              for (const row of rows) {
                row.priority = String(op.priority ?? row.priority);
                wbStamp(row);
              }
            } else if (op.op === "archive") {
              for (const row of rows) {
                row.archived = Boolean(op.archived);
                wbStamp(row);
              }
            } else {
              const doomed = new Set(rows.map((row) => row.id));
              wbTodos = wbTodos.filter(
                (entry) => !doomed.has(entry.id) && !(entry.parentId && doomed.has(entry.parentId)),
              );
              return [];
            }
            return deepCopy(rows.map(wbTodoOut));
          }
          case "list_todo_statuses":
            wbLog.push({ command, args: args ?? {} });
            return deepCopy(
              [...wbStatuses].sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt - b.createdAt),
            );
          case "create_todo_status": {
            wbLog.push({ command, args: args ?? {} });
            const draft = (args?.draft ?? {}) as Record<string, unknown>;
            const name = String(draft.name ?? "").trim();
            if (!name) throw "invalid_input";
            const status: WbStatus = {
              id: wbId("st"),
              name,
              color: String(draft.color ?? "gray"),
              isDone: Boolean(draft.isDone),
              builtin: false,
              sortOrder: wbStatuses.length,
              version: 1,
              createdAt: wbNow(),
              updatedAt: wbNow(),
            };
            wbStatuses = [...wbStatuses, status];
            return deepCopy(status);
          }
          case "update_todo_status": {
            wbLog.push({ command, args: args ?? {} });
            const incoming = (args?.status ?? {}) as WbStatus;
            const row = wbStatus(String(args?.id ?? ""));
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            const name = String(incoming.name ?? "").trim();
            if (!name) throw "invalid_input";
            const isDone = Boolean(incoming.isDone);
            // Flipping `isDone` restates every todo in the status.
            if (isDone !== row.isDone) {
              for (const todo of wbTodos) {
                if (todo.status !== row.id) continue;
                todo.completedAt = isDone ? (todo.completedAt ?? wbNow()) : null;
              }
            }
            row.name = name;
            row.color = String(incoming.color ?? row.color);
            row.isDone = isDone;
            return deepCopy(wbStamp(row));
          }
          case "delete_todo_status": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbStatus(String(args?.id ?? ""));
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            if (row.builtin) throw "invalid_input";
            const reassign = args?.reassignTo ? String(args.reassignTo) : "";
            const held = wbTodos.filter((todo) => todo.status === row.id);
            if (reassign === row.id) throw "invalid_input";
            if (reassign) wbStatus(reassign);
            if (held.length > 0 && !reassign) throw "status_in_use";
            if (reassign) {
              const done = wbIsDone(reassign);
              for (const todo of held) {
                todo.status = reassign;
                todo.completedAt = done ? (todo.completedAt ?? wbNow()) : null;
                wbStamp(todo);
              }
            }
            wbStatuses = wbStatuses.filter((status) => status.id !== row.id);
            return null;
          }
          case "reorder_todo_statuses": {
            wbLog.push({ command, args: args ?? {} });
            const ids = (args?.ids as string[] | undefined) ?? [];
            if (ids.length !== wbStatuses.length) throw "invalid_input";
            if (!ids.every((id) => wbStatuses.some((status) => status.id === id))) {
              throw "invalid_input";
            }
            ids.forEach((id, slot) => {
              wbStatus(id).sortOrder = slot;
            });
            wbStatuses = [...wbStatuses].sort((a, b) => a.sortOrder - b.sortOrder);
            return deepCopy(wbStatuses);
          }
          case "list_todo_tags":
            wbLog.push({ command, args: args ?? {} });
            return deepCopy([...wbTags].sort((a, b) => a.sortOrder - b.sortOrder || a.name.localeCompare(b.name)));
          case "create_todo_tag": {
            wbLog.push({ command, args: args ?? {} });
            const draft = (args?.draft ?? {}) as Record<string, unknown>;
            const name = String(draft.name ?? "").trim();
            if (!name) throw "invalid_input";
            const tag: WbTag = {
              id: wbId("tag"),
              name,
              color: String(draft.color ?? "blue"),
              sortOrder: wbTags.length,
              version: 1,
              createdAt: wbNow(),
              updatedAt: wbNow(),
            };
            wbTags = [...wbTags, tag];
            return deepCopy(tag);
          }
          case "update_todo_tag": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbTags.find((tag) => tag.id === String(args?.id ?? ""));
            if (!row) throw "not_found";
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            const name = String(args?.name ?? "").trim();
            if (!name) throw "invalid_input";
            row.name = name;
            row.color = String(args?.color ?? row.color);
            return deepCopy(wbStamp(row));
          }
          case "delete_todo_tag": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbTags.find((tag) => tag.id === String(args?.id ?? ""));
            if (!row) throw "not_found";
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            for (const todo of wbTodos) {
              if (!todo.tagIds.includes(row.id)) continue;
              todo.tagIds = todo.tagIds.filter((id) => id !== row.id);
              wbStamp(todo);
            }
            wbTags = wbTags.filter((tag) => tag.id !== row.id);
            return null;
          }
          case "list_todo_people":
            wbLog.push({ command, args: args ?? {} });
            return deepCopy(wbPeople);
          case "create_todo_person": {
            wbLog.push({ command, args: args ?? {} });
            const name = String(args?.name ?? "").trim();
            if (!name) throw "invalid_input";
            const person: WbPerson = {
              id: wbId("person"),
              name,
              color: String(args?.color ?? "blue"),
              createdAt: wbNow(),
            };
            wbPeople = [...wbPeople, person];
            return deepCopy(person);
          }
          case "list_todo_attachments":
            wbLog.push({ command, args: args ?? {} });
            return deepCopy(
              wbAttachments.filter((row) => row.todoId === String(args?.todoId ?? "")),
            );
          case "create_todo_attachment": {
            wbLog.push({ command, args: args ?? {} });
            const draft = (args?.draft ?? {}) as Record<string, unknown>;
            const requestId = draft.requestId ? String(draft.requestId) : "";
            const replay = requestId ? wbRequests.get(requestId) : undefined;
            const seen = replay ? wbAttachments.find((row) => row.id === replay) : undefined;
            if (seen) return deepCopy(seen);
            const name = String(draft.name ?? "").trim();
            const path = String(draft.path ?? "").trim();
            if (!name || !path) throw "invalid_input";
            wbTodo(String(draft.todoId ?? ""));
            const attachment: WbAttachment = {
              id: wbId("att"),
              todoId: String(draft.todoId ?? ""),
              name,
              path,
              size: draft.size == null ? null : Number(draft.size),
              mime: String(draft.mime ?? "").trim(),
              version: 1,
              createdAt: wbNow(),
              updatedAt: wbNow(),
            };
            wbAttachments = [...wbAttachments, attachment];
            if (requestId) wbRequests.set(requestId, attachment.id);
            return deepCopy(attachment);
          }
          case "update_todo_attachment": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbAttachments.find((entry) => entry.id === String(args?.id ?? ""));
            if (!row) throw "not_found";
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            const name = String(args?.name ?? "").trim();
            if (!name) throw "invalid_input";
            row.name = name;
            return deepCopy(wbStamp(row));
          }
          case "delete_todo_attachment": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbAttachments.find((entry) => entry.id === String(args?.id ?? ""));
            if (!row) throw "not_found";
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            wbAttachments = wbAttachments.filter((entry) => entry.id !== row.id);
            return null;
          }
          case "pick_todo_attachment":
            wbLog.push({ command, args: args ?? {} });
            // The browser has no native picker; `null` is "cancelled".
            return null;
          case "list_todo_reminders":
            wbLog.push({ command, args: args ?? {} });
            return deepCopy(wbReminders.filter((row) => row.todoId === String(args?.todoId ?? "")));
          case "create_todo_reminder": {
            wbLog.push({ command, args: args ?? {} });
            const draft = (args?.draft ?? {}) as Record<string, unknown>;
            const kind = String(draft.kind ?? "dueBefore");
            if (kind === "dueBefore") {
              const minutes = draft.offsetMinutes == null ? null : Number(draft.offsetMinutes);
              if (minutes == null || minutes < 0 || minutes > 525_600) throw "invalid_input";
            }
            wbTodo(String(draft.todoId ?? ""));
            const reminder: WbReminder = {
              id: wbId("rem"),
              todoId: String(draft.todoId ?? ""),
              kind,
              offsetMinutes: draft.offsetMinutes == null ? null : Number(draft.offsetMinutes),
              message: String(draft.message ?? ""),
              enabled: draft.enabled == null ? true : Boolean(draft.enabled),
              version: 1,
              createdAt: wbNow(),
              updatedAt: wbNow(),
            };
            wbReminders = [...wbReminders, reminder];
            return deepCopy(reminder);
          }
          case "set_todo_reminder_enabled": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbReminders.find((entry) => entry.id === String(args?.id ?? ""));
            if (!row) throw "not_found";
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            row.enabled = Boolean(args?.enabled);
            return deepCopy(wbStamp(row));
          }
          case "delete_todo_reminder": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbReminders.find((entry) => entry.id === String(args?.id ?? ""));
            if (!row) throw "not_found";
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            wbReminders = wbReminders.filter((entry) => entry.id !== row.id);
            return null;
          }
          case "list_todo_activity":
            wbLog.push({ command, args: args ?? {} });
            return deepCopy(
              wbActivity
                .filter((row) => row.todoId === String(args?.todoId ?? ""))
                .sort((a, b) => a.createdAt - b.createdAt || a.id.localeCompare(b.id)),
            );
          case "add_todo_comment": {
            wbLog.push({ command, args: args ?? {} });
            const todoId = String(args?.todoId ?? "");
            const body = String(args?.body ?? "").trim();
            if (!body) throw "invalid_input";
            wbTodo(todoId);
            const authorId = args?.authorId ? String(args.authorId) : null;
            if (authorId && !wbPeople.some((person) => person.id === authorId)) throw "not_found";
            const entry: WbActivity = {
              id: wbId("act"),
              todoId,
              kind: "comment",
              authorId,
              body,
              meta: "",
              createdAt: wbNow(),
            };
            wbActivity = [...wbActivity, entry];
            return deepCopy(entry);
          }
          case "list_calendar_events":
            wbLog.push({ command, args: args ?? {} });
            return deepCopy(wbEvents);
          case "create_calendar_event": {
            wbLog.push({ command, args: args ?? {} });
            const draft = (args?.draft ?? {}) as Record<string, unknown>;
            const title = String(draft.title ?? "").trim();
            if (!title) throw "invalid_input";
            const allDay = Boolean(draft.allDay);
            const event: WbEvent = {
              id: wbId("event"),
              title,
              notes: String(draft.notes ?? ""),
              date: String(draft.date ?? ""),
              startTime: allDay ? null : draft.startTime ? String(draft.startTime) : null,
              endTime: allDay ? null : draft.endTime ? String(draft.endTime) : null,
              allDay,
              timezone: String(draft.timezone ?? seedWb.timezone),
              version: 1,
              createdAt: wbNow(),
              updatedAt: wbNow(),
            };
            wbEvents = [...wbEvents, event];
            return deepCopy(event);
          }
          case "update_calendar_event": {
            wbLog.push({ command, args: args ?? {} });
            const incoming = (args?.event ?? {}) as WbEvent;
            const row = wbEvent(String(incoming.id ?? ""));
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            if (!String(incoming.title ?? "").trim()) throw "invalid_input";
            Object.assign(row, incoming);
            return deepCopy(wbStamp(row));
          }
          case "delete_calendar_event": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbEvent(String(args?.id ?? ""));
            wbStale(row.version, Number(args?.expectedVersion ?? 0));
            wbEvents = wbEvents.filter((event) => event.id !== row.id);
            return null;
          }
          case "list_results": {
            wbLog.push({ command, args: args ?? {} });
            const includeArchived = Boolean(args?.includeArchived);
            return deepCopy(wbResults.filter((row) => includeArchived || !row.artifact.archived));
          }
          case "create_result": {
            wbLog.push({ command, args: args ?? {} });
            const draft = (args?.draft ?? {}) as Record<string, unknown>;
            const title = String(draft.title ?? "").trim();
            if (!title) throw "invalid_input";
            const kind = String(draft.kind ?? "text");
            const body = String(draft.body ?? "").trim();
            const path = String(draft.filePath ?? "").trim();
            if (kind === "text" && !body) throw "invalid_input";
            if (kind === "file" && !path) throw "invalid_input";
            const artifact = {
              id: wbId("result"),
              title,
              kind,
              body: kind === "text" ? body : null,
              filePath: kind === "file" ? path : null,
              fileSize: null,
              fileModifiedAt: null,
              source: String(draft.source ?? "manual"),
              category: String(draft.category ?? ""),
              favorite: false,
              archived: false,
              version: 1,
              createdAt: wbNow(),
              updatedAt: wbNow(),
            };
            // No disk in the browser: a fresh reference reports as present.
            wbResults = [
              ...wbResults,
              {
                artifact,
                fileStatus: kind === "file" ? "ok" : "notApplicable",
                currentSize: null,
                currentModifiedAt: null,
              },
            ];
            return deepCopy(artifact);
          }
          case "update_result": {
            wbLog.push({ command, args: args ?? {} });
            const incoming = (args?.artifact ?? {}) as WbResult["artifact"];
            const row = wbResult(String(incoming.id ?? ""));
            wbStale(row.artifact.version, Number(args?.expectedVersion ?? 0));
            if (!String(incoming.title ?? "").trim()) throw "invalid_input";
            Object.assign(row.artifact, incoming);
            return deepCopy(wbStamp(row.artifact));
          }
          case "set_result_flags": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbResult(String(args?.id ?? ""));
            wbStale(row.artifact.version, Number(args?.expectedVersion ?? 0));
            if (args?.favorite !== null && args?.favorite !== undefined) {
              row.artifact.favorite = Boolean(args.favorite);
            }
            if (args?.archived !== null && args?.archived !== undefined) {
              row.artifact.archived = Boolean(args.archived);
            }
            return deepCopy(wbStamp(row.artifact));
          }
          case "delete_result": {
            wbLog.push({ command, args: args ?? {} });
            const row = wbResult(String(args?.id ?? ""));
            wbStale(row.artifact.version, Number(args?.expectedVersion ?? 0));
            // The record only — the referenced file is never touched.
            wbResults = wbResults.filter((entry) => entry.artifact.id !== row.artifact.id);
            return null;
          }
          case "open_result_file":
          case "reveal_result_file":
            // The record is the allow-list; the GUI never names a path.
            wbLog.push({ command, args: args ?? {} });
            wbResult(String(args?.id ?? ""));
            return null;
          default:
            return null;
        }
      },
    };

    (window as unknown as { __emit: unknown }).__emit = (payload: unknown) => {
      for (const callback of Object.values(callbacks))
        callback({ event: "run:event", id: 1, payload });
    };

    // The event plugin's local registry — `listen`'s unlisten calls this before
    // its invoke. Without it StrictMode's double-mount leaves two live handlers
    // and every event applies twice.
    (
      window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__: unknown }
    ).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      registerListener: () => {},
      unregisterListener: (_event: string, eventId: number) => {
        const handlerId = listenerByEventId[eventId];
        if (handlerId) delete callbacks[handlerId];
        delete listenerByEventId[eventId];
      },
    };
  }, seeded);
}

/** Pushes one `run:event`, the way the runner's emitter would. */
export function emit(page: Page, payload: unknown): Promise<void> {
  return page.evaluate(
    (value) => (window as unknown as { __emit: (payload: unknown) => void }).__emit(value),
    payload,
  );
}

/** Opens the brand dropdown and picks an agent space (the Codex-style switcher). */
export async function switchMode(page: Page, mode: "work" | "code"): Promise<void> {
  await page.locator('[data-testid="mode-toggle"]').click();
  await page.locator(`[data-testid="mode-${mode}"]`).click();
}

/** A project as the core lists it. */
export function project(path: string, name: string) {
  return { path, name };
}

/** A session as the core lists it. Legacy rows without a mode are code. */
export function sessionRef(
  id: string,
  projectPath: string,
  title: string,
  at = 1_700_000_000,
  archived = false,
  mode: "work" | "code" = "code",
) {
  return { id, project: projectPath, title, at, archived, mode };
}
