import type { Page } from "@playwright/test";

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
  approvalLog?: Array<{ id: string; step: number; approved: boolean; sessionWide: boolean; runId?: number }>;
  /** `turn_changes` payload for inline diffs. */
  turnChanges?: Array<{ path: string; diff: string; userPreexisting?: boolean; undoState?: string; conflict?: boolean }>;
  /** Full file bodies for the file viewer (`read_project_file`). */
  fileBodies?: Record<string, string>;
  /** Image paths the stub treats as >4MB — `probe_context_file` then returns
   *  `preview: null`, like the Rust cap on inline image previews. */
  oversizedImages?: string[];
  /** `undo_turn` payload; default is an empty (nothing-to-undo) report. */
  undoReport?: { restored: string[]; conflicts: Array<{ path: string; reason: string; message: string }> };
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
};

/**
 * Stands in for the IPC bridge.
 *
 * The suite runs in a plain browser, where there is no core at all:
 * `@tauri-apps/api` reaches `window.__TAURI_INTERNALS__` directly and
 * `isDesktop()` only checks that the key exists.
 */
export async function stubShell(page: Page, core: Core = {}): Promise<void> {
  await page.addInitScript((config: Core) => {
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
    const approvalLog: Array<{ id: string; step: number; approved: boolean; sessionWide: boolean; runId?: number }> = [];
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
            if (config.workspace === null) throw "HOME is not set, so there is nowhere to keep the project list";
            return workspace;
          case "load_session":
            return (config.sessions as Record<string, unknown> | undefined)?.[String(args?.id ?? "")] ?? session;
          case "git_branch":
            return config.branch ?? null;
          case "pick_folder":
            return config.pickFolder ?? null;
          case "add_project": {
            const path = String(args?.path ?? "");
            const current = workspace as { projects?: Array<{ path: string; name: string }> };
            if (path && Array.isArray(current.projects) && !current.projects.some((item) => item.path === path)) {
              const segments = path.split("/").filter(Boolean);
              const name = segments[segments.length - 1] ?? path;
              workspace = { ...(workspace as Record<string, unknown>), projects: [...current.projects, { path, name }] };
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
            const kind =
              ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "avif", "heic"].includes(ext)
                ? "image"
                : ext === "pdf"
                  ? "pdf"
                  : ["doc", "docx", "rtf", "odt", "xls", "xlsx", "ods", "ppt", "pptx", "odp"].includes(ext)
                    ? "office"
                    : ["txt", "md", "markdown", "json", "jsonl", "yml", "yaml", "toml", "xml", "html",
                       "css", "js", "jsx", "ts", "tsx", "rs", "py", "go", "java", "kt", "c", "h",
                       "cpp", "hpp", "sh", "zsh", "bash", "sql", "log", "csv", "tsv", "ini", "cfg",
                       "conf"].includes(ext)
                      ? "text"
                      : "other";
            // Rust only inlines image data URLs (≤4MB); oversized images and
            // every other kind carry no preview.
            const oversized = ((config as { oversizedImages?: string[] }).oversizedImages ?? []).includes(path);
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
              if (p.startsWith("/") && !(config as { externalFiles?: string[] }).externalFiles?.includes(p)) {
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
              agent: Array.isArray(args?.agents) && args.agents.length > 0
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
                for (const callback of Object.values(callbacks)) callback({ event: "run:event", id: 1, payload });
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
              const match = markdown.match(
                new RegExp(`## ${heading}\\n([\\s\\S]*?)(?=\\n## |$)`),
              );
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
              const match = markdown.match(
                new RegExp(`## ${heading}\\n([\\s\\S]*?)(?=\\n## |$)`),
              );
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
          default:
            return null;
        }
      },
    };

    (window as unknown as { __emit: unknown }).__emit = (payload: unknown) => {
      for (const callback of Object.values(callbacks)) callback({ event: "run:event", id: 1, payload });
    };

    // The event plugin's local registry — `listen`'s unlisten calls this before
    // its invoke. Without it StrictMode's double-mount leaves two live handlers
    // and every event applies twice.
    (window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      registerListener: () => {},
      unregisterListener: (_event: string, eventId: number) => {
        const handlerId = listenerByEventId[eventId];
        if (handlerId) delete callbacks[handlerId];
        delete listenerByEventId[eventId];
      },
    };
  }, core);
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
