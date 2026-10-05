import { useEffect, useRef, useState } from "react";
import {
  addProject,
  archiveSession,
  createProject,
  deleteSession,
  isDesktop,
  openSession as openSessionCommand,
  removeProject,
  renameProject,
  reorderProject,
  restoreSession,
  retitleSession,
  revealProject,
  workspace,
  type SessionRefDto,
  type WorkspaceDto,
} from "../api";
import { loadDemoState } from "./demoState";
import type { Project } from "./types";

export type WorkspaceState = {
  projects: Project[];
  /** True when this list came from the core rather than the fixture. */
  live: boolean;
  toggleProject: (id: string) => void;
  add: (path: string) => Promise<void>;
  create: (parent: string, name: string) => Promise<void>;
  remove: (id: string) => Promise<void>;
  rename: (id: string, name: string) => Promise<void>;
  reorder: (from: number, to: number) => Promise<void>;
  reveal: (id: string) => Promise<void>;
  startSession: (project: string) => Promise<string | null>;
  retitle: (sessionId: string, title: string) => Promise<void>;
  archive: (sessionId: string) => Promise<void>;
  restore: (sessionId: string) => Promise<void>;
  removeSession: (sessionId: string) => Promise<void>;
  refresh: () => Promise<void>;
};

/**
 * The one place that decides where the sidebar's projects come from.
 *
 * The demo routes and the plain browser (visual regression, `vite dev`) read the
 * deterministic fixture via the demo boundary; only the desktop shell on a real
 * route asks the core. `mode` picks which agent space's conversations show —
 * projects themselves belong to both worlds.
 */
export function useWorkspace(demo: boolean, mode: string): WorkspaceState {
  const live = !demo && isDesktop();
  const [projects, setProjects] = useState<Project[]>(() => (demo ? demoProjects(mode) : []));
  /** Last live payload, so a mode switch can re-filter without a refetch. */
  const payloadRef = useRef<WorkspaceDto | null>(null);
  /** Latest mode, so an in-flight fetch resolves against the current space. */
  const modeRef = useRef(mode);
  modeRef.current = mode;

  const apply = (payload: WorkspaceDto | null) => {
    if (!payload) return;
    payloadRef.current = payload;
    setProjects((current) => adopt(payload, openOf(current), modeRef.current));
  };

  const refresh = async () => {
    if (!live) return;
    apply(await workspace());
  };

  // Fetch once per shell; the mode effect below re-filters what is in hand.
  useEffect(() => {
    if (!live) return;
    let alive = true;
    void workspace().then((payload) => {
      if (!alive) return;
      apply(payload);
    });
    return () => {
      alive = false;
    };
  }, [live]);

  // Re-filter on demo/live switch or mode change from the payload already in
  // hand — expansion state survives and nothing refetches (StrictMode-safe:
  // pure derivation).
  useEffect(() => {
    if (demo) {
      setProjects(demoProjects(mode));
      return;
    }
    setProjects((current) =>
      payloadRef.current ? adopt(payloadRef.current, openOf(current), mode) : [],
    );
  }, [demo, mode]);

  const toggleProject = (id: string) =>
    setProjects((current) =>
      current.map((project) => (project.id === id ? { ...project, expanded: !project.expanded } : project)),
    );

  const run = async (command: Promise<WorkspaceDto | null>) => apply(await command);

  return {
    projects,
    live,
    toggleProject,
    add: (path) => run(addProject(path)),
    create: (parent, name) => run(createProject(parent, name)),
    remove: (id) => run(removeProject(id)),
    rename: (id, name) => run(renameProject(id, name)),
    reveal: async (id) => {
      await revealProject(id);
    },
    reorder: async (from, to) => {
      const moving = projects[from];
      if (!moving || from === to) return;
      const previous = projects;
      const next = [...projects];
      next.splice(to, 0, ...next.splice(from, 1));
      setProjects(next);
      try {
        const payload = await reorderProject(moving.id, to);
        if (!payload) throw new Error("reorder_project failed");
        apply(payload);
      } catch {
        setProjects(previous);
      }
    },
    startSession: async (project) => {
      // Demo routes and the browser shell must never write live sessions.
      if (!live) return null;
      const session = await openSessionCommand(project, "新对话");
      if (!session) return null;
      await refresh();
      return session.id;
    },
    retitle: async (sessionId, title) => {
      if (!live) return;
      try {
        await retitleSession(sessionId, title);
      } catch (error) {
        throw error instanceof Error ? error : new Error(String(error));
      }
      await refresh();
    },
    archive: async (sessionId) => {
      if (!live) return;
      try {
        apply(await archiveSession(sessionId));
      } catch (error) {
        throw error instanceof Error ? error : new Error(String(error));
      }
    },
    restore: async (sessionId) => {
      if (!live) return;
      apply(await restoreSession(sessionId));
    },
    removeSession: async (sessionId) => {
      if (!live) return;
      try {
        apply(await deleteSession(sessionId));
      } catch (error) {
        throw error instanceof Error ? error : new Error(String(error));
      }
    },
    refresh,
  };
}

function openOf(projects: Project[]): Set<string> {
  return new Set(projects.filter((project) => project.expanded).map((project) => project.id));
}

/** A session's agent space; legacy rows predate the tag and are code. */
export function modeOf(conversation: { mode?: string }): string {
  return conversation.mode ?? "code";
}

/** The fixture's tree narrowed to one agent space (demo routes only). */
function demoProjects(mode: string): Project[] {
  return loadDemoState().projects
    .map((project) => ({
      ...project,
      conversations: project.conversations.filter((conversation) => modeOf(conversation) === mode),
    }))
    // The 「未选择项目」 pseudo group exists only to hold projectless
    // conversations — with none left it disappears, unlike real project rows.
    .filter((project) => project.id !== "" || project.conversations.length > 0);
}

function adopt(payload: WorkspaceDto, open: Set<string>, mode: string): Project[] {
  const projects = payload.projects.map((project) => ({
    id: project.path,
    path: project.path,
    name: project.name,
    expanded: open.has(project.path),
    conversations: payload.sessions
      .filter((session) => belongs(session, project.path, mode))
      .map((session) => ({
        id: session.id,
        title: session.title,
        time: ago(session.at),
        mode: session.mode ?? "code",
        at: session.at,
      })),
  }));
  // Sessions created without a project live in their own 「未选择项目」 group at
  // the foot of the tree (empty `id`/`path` marks the pseudo entry — the sidebar
  // renders it as a plain group row, never as a real project). Only the current
  // mode's sessions gather here.
  const projectless = payload.sessions
    .filter((session) => session.project === "" && !session.archived && modeOf(session) === mode)
    .map((session) => ({
      id: session.id,
      title: session.title,
      time: ago(session.at),
      mode: session.mode ?? "code",
      at: session.at,
    }));
  if (projectless.length > 0) {
    projects.push({
      id: "",
      path: "",
      name: "未选择项目",
      expanded: true,
      conversations: projectless,
    });
  }
  return projects;
}

function belongs(session: SessionRefDto, project: string, mode: string): boolean {
  return session.project === project && !session.archived && modeOf(session) === mode;
}

function ago(at: number): string {
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - at);
  if (seconds < 60) return "刚刚";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h`;
  return `${Math.floor(seconds / 86_400)}d`;
}
