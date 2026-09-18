import type { Page } from "@playwright/test";

/** What the stand-in core answers. Anything omitted gets an inert default. */
export type Core = {
  /** The `workspace` payload. `null` makes the call fail, as an unset HOME does. */
  workspace?: unknown;
  /** The `load_session` payload. */
  session?: unknown;
  /** What `git_branch` reports. */
  branch?: string | null;
  /** `list_archived` payload. */
  archived?: unknown[];
  /** `load_providers` payload. */
  providers?: unknown[];
  /** `setting` values by key. */
  settings?: Record<string, string>;
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
    let next = 1;
    const settings: Record<string, string> = { ...(config.settings ?? {}) };
    let session = config.session ?? null;
    let workspace = config.workspace ?? { projects: [], sessions: [] };

    (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
      transformCallback: (callback: (event: unknown) => void) => {
        const id = next++;
        callbacks[id] = callback;
        return id;
      },
      invoke: async (command: string, args?: Record<string, unknown>) => {
        switch (command) {
          case "plugin:event|listen":
            return next++;
          case "core_info":
            return { name: "kodo-core", version: "0.0.1" };
          case "workspace":
            if (config.workspace === null) throw "HOME is not set, so there is nowhere to keep the project list";
            return workspace;
          case "load_session":
            return session;
          case "git_branch":
            return config.branch ?? null;
          case "list_archived":
            return config.archived ?? [];
          case "load_providers":
            return (config.providers ?? []).map((p) => ({
              ...((p as Record<string, unknown>) ?? {}),
              apiKey: (p as { apiKey?: string })?.apiKey ?? "",
              hasKey: Boolean((p as { apiKey?: string })?.apiKey),
            }));
          case "save_providers":
            return args?.providers ?? config.providers ?? [];
          case "setting":
            return settings[args?.key as string] ?? null;
          case "set_setting":
            if (args?.key) settings[args.key as string] = String(args.value ?? "");
            return null;
          case "retitle_session": {
            if (session && typeof session === "object") {
              session = { ...(session as Record<string, unknown>), title: args?.title };
            }
            return session;
          }
          case "archive_session":
            return workspace;
          case "restore_session":
            return workspace;
          case "respond_approval":
            return null;
          case "send_message":
          case "stop_run":
            return null;
          default:
            return null;
        }
      },
    };

    (window as unknown as { __emit: unknown }).__emit = (payload: unknown) => {
      for (const callback of Object.values(callbacks)) callback({ event: "run:event", id: 1, payload });
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

/** A project as the core lists it. */
export function project(path: string, name: string) {
  return { path, name };
}

/** A session as the core lists it. */
export function sessionRef(id: string, projectPath: string, title: string, at = 1_700_000_000, archived = false) {
  return { id, project: projectPath, title, at, archived };
}
