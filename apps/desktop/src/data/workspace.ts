import { useEffect, useState } from "react";
import {
  addProject,
  archiveSession,
  createProject,
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
import { projects as fixtureProjects } from "./fixture";
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
  refresh: () => Promise<void>;
};

/**
 * The one place that decides where the sidebar's projects come from.
 *
 * The demo routes and the plain browser (visual regression, `vite dev`) read the
 * deterministic fixture; only the desktop shell on a real route asks the core.
 */
export function useWorkspace(demo: boolean): WorkspaceState {
  const live = !demo && isDesktop();
  const [projects, setProjects] = useState<Project[]>(live ? [] : fixtureProjects);

  const apply = (payload: WorkspaceDto | null) => {
    if (!payload) return;
    setProjects((current) => adopt(payload, openOf(current)));
  };

  const refresh = async () => {
    if (!live) return;
    apply(await workspace());
  };

  useEffect(() => {
    setProjects(live ? [] : fixtureProjects);
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
    refresh,
  };
}

function openOf(projects: Project[]): Set<string> {
  return new Set(projects.filter((project) => project.expanded).map((project) => project.id));
}

function adopt(payload: WorkspaceDto, open: Set<string>): Project[] {
  return payload.projects.map((project) => ({
    id: project.path,
    path: project.path,
    name: project.name,
    expanded: open.has(project.path),
    conversations: payload.sessions
      .filter((session) => belongs(session, project.path))
      .map((session) => ({ id: session.id, title: session.title, time: ago(session.at) })),
  }));
}

function belongs(session: SessionRefDto, project: string): boolean {
  return session.project === project && !session.archived;
}

function ago(at: number): string {
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - at);
  if (seconds < 60) return "刚刚";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h`;
  return `${Math.floor(seconds / 86_400)}d`;
}
