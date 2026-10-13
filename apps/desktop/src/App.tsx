import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { coreInfo, gitBranch, listAgents, pickFolder, setting, setSetting } from "./api";
import { loadDemoState, type DemoState } from "./data/demoState";
import { T } from "./i18n";
import { DEFAULT_MODEL, MODEL_SETTING } from "./data/models";
import {
  type ProviderConfig,
  loadProviders,
  loadActiveIndex,
  providerModelId,
  saveActiveIndex,
  saveProviders,
} from "./data/providers";
import { useSetting, useSettingBool } from "./data/useSetting";
import type { LiveSnapshot } from "./data/liveContext";
import { modeOf, useWorkspace } from "./data/workspace";
import { hrefTo, navigate, useRoute } from "./routes";
import { CommandPalette, type PaletteAction, type PaletteAgent, type PaletteSession } from "./shell/CommandPalette";
import { Sidebar } from "./shell/Sidebar";
import { AutoOnboarding } from "./shell/AutoOnboarding";
import { Inspector, type FilePanelRequest, type InspectorTab } from "./inspector/Inspector";
import { BrowsePanel } from "./inspector/BrowsePanel";
import { PanelResizer, clampWidth, CONVERSATION_WIDTH_MIN, WIDTH_MIN } from "./inspector/PanelResizer";
import { FileViewer } from "./conversation/FileViewer";
import type { ArchiveSelection } from "./inspector/ArchiveInspector";
import { ArchivePage } from "./pages/ArchivePage";
import { ConversationPage } from "./pages/ConversationPage";
import { PluginsPage } from "./pages/PluginsPage";
import { SchedulerPage } from "./pages/SchedulerPage";
import { SettingsPage } from "./pages/SettingsPage";
import { TracePage } from "./pages/TracePage";
import { TopBar, type ConvActions } from "./shell/TopBar";

const LAST_PROJECT_KEY = "kodo:last-project";

/** Parse a JSON-encoded array of primitives or `{name, at}` records into
 *  a typed list; tolerate malformed settings (legacy rows, partial writes)
 *  by returning an empty list rather than throwing. */
function parseJsonArray<T>(raw: string): T[] {
  if (!raw.trim()) return [];
  try {
    const value: unknown = JSON.parse(raw);
    return Array.isArray(value) ? (value as T[]) : [];
  } catch {
    return [];
  }
}

export function App() {
  const route = useRoute();
  /** Which agent space is showing. Shared key: it also stamps new sessions/CLI runs. */
  const [agentMode, setAgentMode] = useSetting("agent-mode", "code");
  const workspace = useWorkspace(route.demo, agentMode);
  /** Fixture lives only behind `/ui-demo`; production pages get it as a prop. */
  const demoState = useMemo<DemoState | null>(() => (route.demo ? loadDemoState() : null), [route.demo]);

  const [activeConversationId, setActiveConversationId] = useState<string | null>(() =>
    route.demo && demoState ? demoState.conversation.id : null,
  );
  const [activeProjectId, setActiveProjectId] = useState<string | null>(() =>
    route.demo && demoState ? demoState.project.id : null,
  );
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>("summary");
  const [coreVersion, setCoreVersion] = useState<string | null>(null);
  const [model, setModel] = useState(DEFAULT_MODEL);
  const [branch, setBranch] = useState<string | null>(null);
  const [archiveSelection, setArchiveSelection] = useState<ArchiveSelection>(null);
  const [archiveReload, setArchiveReload] = useState(0);
  const [liveSnapshot, setLiveSnapshot] = useState<LiveSnapshot | null>(null);
  /** Right-sidebar 文件 panel: what to show and that it may show at all. */
  const [fileRequest, setFileRequest] = useState<FilePanelRequest | null>(null);
  /** Per-file unified diffs published up from ConversationPage (reply rows + panel). */
  const [fileDiffs, setFileDiffs] = useState<Record<string, string>>({});
  /** Wide = ≥1200px, where the panel is in-flow (below that it overlays). */
  const [wide, setWide] = useState(() => window.matchMedia("(min-width: 1200px)").matches);
  useEffect(() => {
    const mq = window.matchMedia("(min-width: 1200px)");
    const onChange = () => setWide(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);
  /** Right-sidebar width: base 330 / 文件·浏览 widen 640, each its own key. */
  const [inspectorWidth, setInspectorWidth] = useSetting("inspector-width", "330");
  const [inspectorWideWidth, setInspectorWideWidth] = useSetting("inspector-wide-width", "640");
  /** Keep the conversation mounted while the Inspector occupies its space. */
  const [inspectorFocusMode, setInspectorFocusMode] = useSettingBool("inspector-focus-mode", false);
  /** Transient drag value — committed to the active key on pointerup. */
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const [panelMaxWidth, setPanelMaxWidth] = useState(WIDTH_MIN);
  useEffect(() => {
    const content = contentRef.current;
    if (!wide || !content) return;
    const measure = () => {
      const inspector = content.querySelector<HTMLElement>(".inspector");
      const railWidth = inspector?.querySelector(".ins-rail")?.getBoundingClientRect().width ?? 0;
      const borderWidth = inspector ? Number.parseFloat(getComputedStyle(inspector).borderLeftWidth) || 0 : 0;
      setPanelMaxWidth(Math.max(WIDTH_MIN, Math.floor(content.getBoundingClientRect().width - railWidth - borderWidth)));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(content);
    return () => observer.disconnect();
  }, [wide, inspectorTab, route.name, route.inspectorOpen]);
  const baseWidth = clampWidth(Number.parseInt(inspectorWidth, 10) || 330);
  const wideWidth = clampWidth(Number.parseInt(inspectorWideWidth, 10) || 640);
  /** 文件 and 浏览 widen via their own key; the route guard mirrors Inspector. */
  const wideMode = (inspectorTab === "file" || inspectorTab === "browse") && route.name === "conversation";
  const restoreWidth = Math.max(WIDTH_MIN, panelMaxWidth - CONVERSATION_WIDTH_MIN);
  const focusModeActive = wide && route.inspectorOpen && route.name === "conversation" &&
    (dragWidth === null ? inspectorFocusMode : dragWidth > restoreWidth);
  const appliedWidth = focusModeActive ? panelMaxWidth : clampWidth(dragWidth ?? (wideMode ? wideWidth : baseWidth), restoreWidth);
  const commitInspectorWidth = (next: number) => {
    setDragWidth(null);
    if (next > restoreWidth) {
      setInspectorFocusMode(true);
    } else {
      setInspectorFocusMode(false);
      if (wideMode) setInspectorWideWidth(String(next));
      else setInspectorWidth(String(next));
    }
  };
  const toggleInspectorFocus = () => {
    setDragWidth(null);
    if (route.name === "conversation") setInspectorFocusMode(!focusModeActive);
  };
  /** Incremented by the top-bar 文件 icon; ConversationPage opens its viewer. */
  const [openFilesRequest, setOpenFilesRequest] = useState(0);
  /** Rename/archive entry points published by ConversationPage for the TopBar. */
  const [convActions, setConvActions] = useState<ConvActions | null>(null);
  /** Memory recall summary for the active turn. `-1` means "engine off"
   *  (no event was emitted this turn) — distinct from `0` ("engine on,
   *  nothing matched"). The TopBar renders different copy in each case. */
  const [memoryRecalledCount, setMemoryRecalledCount] = useState<number>(-1);
  /**
   * Composer persona selection per conversation — the chain in application
   * order. A single-element list is the common case; shift-click on the
   * composer's persona row grows the chain, last in wins for conflicts.
   * In-memory only — refresh resets (per the 2026-09-30 plugin page note).
   */
  const [personaListBySession, setPersonaListBySession] = useState<Record<string, string[]>>({});
  const selectAgents = useCallback((sessionKey: string, names: string[] | null) => {
    setPersonaListBySession((current) => {
      const next = { ...current };
      if (names === null || names.length === 0) {
        delete next[sessionKey];
      } else {
        next[sessionKey] = names;
      }
      return next;
    });
  }, []);

  /**
   * Favorited persona names, persisted via `useSetting` (JSON-encoded
   * string array). Shared between the AgentsTab star toggle and the
   * composer's 常用 section — the same source of truth on both sides.
   */
  const [favoriteAgentsRaw, setFavoriteAgents] = useSetting("kodo:favorite-agents", "[]");
  const favoriteAgents = useMemo<string[]>(
    () => parseJsonArray(favoriteAgentsRaw),
    [favoriteAgentsRaw],
  );
  const toggleFavoriteAgent = useCallback(
    (name: string) => {
      const set = new Set(favoriteAgents);
      if (set.has(name)) set.delete(name);
      else set.add(name);
      setFavoriteAgents(JSON.stringify(Array.from(set)));
    },
    [favoriteAgents, setFavoriteAgents],
  );

  /**
   * Recent personas (MRU), persisted via `useSetting`. JSON-encoded array
   * of `{name, at}` — the composer renders `最近用` as the chain's
   * primary "what did I just use" hint, capped at 5 entries.
   */
  const [recentAgentsRaw, setRecentAgents] = useSetting("kodo:recent-agents", "[]");
  const recentAgents = useMemo<Array<{ name: string; at: number }>>(
    () => parseJsonArray(recentAgentsRaw),
    [recentAgentsRaw],
  );
  const recordRecentAgent = useCallback(
    (name: string) => {
      const now = Date.now();
      const filtered = recentAgents.filter((entry) => entry.name !== name);
      const next = [{ name, at: now }, ...filtered].slice(0, 5);
      setRecentAgents(JSON.stringify(next));
    },
    [recentAgents, setRecentAgents],
  );

  /**
   * Persona list version. Bumped after AgentsTab CRUD so the Composer
   * dropdown re-pulls `listAgents()` (the dropdown lives in a different
   * page, so the tab can't refetch it directly).
   */
  const [agentsVersion, setAgentsVersion] = useState(0);
  const bumpAgentsVersion = useCallback(() => {
    setAgentsVersion((n) => n + 1);
  }, []);

  /** Read-only mirror of the persona slot key — the async listAgents()
   *  closure needs the latest value without re-running on every change. */
  const sessionKeyRef = useRef("new");
  sessionKeyRef.current = activeConversationId ?? "new";

  const [providers, setProviders] = useState<ProviderConfig[]>([]);
  const [activeProviderIndex, setActiveProviderIndex] = useState(0);
  const [sidebarVisible, setSidebarVisible] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [autoGitBranch] = useSetting("auto-detect-git-branch", "true");
  const [density] = useSetting("density", "compact");
  const [lineNumbers] = useSetting("show-line-numbers", "true");
  const [systemFont] = useSetting("use-system-font", "true");
  const [theme] = useSetting("theme", "light");

  const onSnapshot = useCallback((snapshot: LiveSnapshot | null) => {
    setLiveSnapshot(snapshot);
  }, []);

  useEffect(() => {
    if (route.demo && demoState) {
      setActiveConversationId(demoState.conversation.id);
      setActiveProjectId(demoState.project.id);
      return;
    }
    // Live route: never clobber a conversation the user already opened.
    if (!route.demo) return;
    setActiveConversationId(null);
    setActiveProjectId(null);
  }, [route.demo, demoState]);

  // Runtime consumer for the Appearance theme control (not just persisted state).
  useEffect(() => {
    const apply = (mode: string) => {
      const resolved =
        mode === "system"
          ? window.matchMedia("(prefers-color-scheme: dark)").matches
            ? "dark"
            : "light"
          : mode === "dark"
            ? "dark"
            : "light";
      document.documentElement.setAttribute("data-theme", resolved);
    };
    apply(theme);
    if (theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => apply("system");
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [theme]);

  /** One source of truth for picking the model after providers change.
   *  Stored `default-model` wins when it still names a configured model;
   *  a stale value self-heals onto the active provider's pointer so the
   *  backend never runs a model id that no longer exists. */
  const resolveModelFromProviders = useCallback((list: ProviderConfig[]) => {
    void loadActiveIndex().then((idx) => {
      const index = idx < list.length ? idx : 0;
      setActiveProviderIndex(index);
      const active = list[index];
      const pointer = active ? providerModelId(active) || active.model || "" : "";
      void setting(MODEL_SETTING).then((stored) => {
        const value = (stored ?? "").trim();
        const known =
          value !== "" &&
          list.some(
            (p) =>
              p.models?.some(
                (m) =>
                  m.model_id.toLowerCase() === value.toLowerCase() ||
                  m.display_name.toLowerCase() === value.toLowerCase(),
              ) ||
              providerModelId(p).toLowerCase() === value.toLowerCase() ||
              p.model.toLowerCase() === value.toLowerCase(),
          );
        if (known) {
          setModel(value);
        } else if (value && pointer) {
          setModel(pointer);
          void setSetting(MODEL_SETTING, pointer);
        } else if (pointer) {
          setModel(pointer);
        }
      });
    });
  }, []);

  const reloadProviders = useCallback(() => {
    void loadProviders().then((list) => {
      setProviders(list);
      resolveModelFromProviders(list);
    });
  }, [resolveModelFromProviders]);

  useEffect(() => {
    void coreInfo()
      .catch(() => null)
      .then((info) => setCoreVersion(info ? `${info.name} ${info.version}` : null));

    reloadProviders();
  }, [reloadProviders]);

  const owner = workspace.projects.find((project) =>
    project.conversations.some((item) => item.id === activeConversationId),
  );
  const activeProject =
    owner ?? (activeProjectId ? workspace.projects.find((project) => project.id === activeProjectId) : null);

  const traceBootstrapped = useRef(false);
  useEffect(() => {
    if (route.demo || route.name !== "trace") {
      traceBootstrapped.current = false;
      return;
    }
    if (activeConversationId || traceBootstrapped.current) return;

    const existing = activeProject?.conversations[0] ?? workspace.projects.flatMap((project) => project.conversations)[0];
    if (!existing) return;
    traceBootstrapped.current = true;
    setActiveConversationId(existing.id);
  }, [route.demo, route.name, activeConversationId, activeProject?.id, workspace.projects]);

  /**
   * Cached persona list for ⌘K. Refetched when the agentsVersion bumps or
   * the active project changes (project scope is only enumerable from
   * the backend). The list is shallow — persona rows render flat under
   * the commands section.
   */
  const [paletteAgents, setPaletteAgents] = useState<PaletteAgent[]>([]);
  useEffect(() => {
    let alive = true;
    void listAgents(activeProject?.path || undefined).then((list) => {
      if (!alive) return;
      setPaletteAgents(
        list.map((agent) => ({
          name: agent.name,
          description: agent.description,
          source: agent.source,
          run: () => {
            // Set the persona in the draft slot so the next send picks it
            // up; the conversation is whichever the user is currently on
            // (or "new" when they were on the welcome stage).
            selectAgents(sessionKeyRef.current, [agent.name]);
            if (!activeConversationId) {
              if (route.name !== "conversation") {
                navigate(workspace.live ? "/conversation" : "/ui-demo/conversation");
              }
            }
          },
        })),
      );
    });
    return () => {
      alive = false;
    };
  }, [agentsVersion, activeProject?.path, selectAgents, activeConversationId, route.name, workspace.live]);

  useEffect(() => {
    // The 「未选择项目」 pseudo group has an empty path — it never becomes the
    // remembered project.
    if (!owner?.path) return;
    setActiveProjectId(owner.id);
    window.localStorage.setItem(LAST_PROJECT_KEY, owner.id);
  }, [owner?.id]);

  // Boot is a fresh task too: preselect the remembered project once the
  // workspace arrives, unless the user already chose (or cleared) one.
  useEffect(() => {
    if (route.demo || projectChosen.current) return;
    if (activeConversationId !== null || activeProjectId !== null) return;
    if (workspace.projects.length === 0) return;
    projectChosen.current = true;
    const remembered = window.localStorage.getItem(LAST_PROJECT_KEY);
    if (remembered && workspace.projects.some((project) => project.id === remembered)) {
      setActiveProjectId(remembered);
    }
  }, [route.demo, activeConversationId, activeProjectId, workspace.projects]);

  useEffect(() => {
    if (route.demo) {
      setBranch(demoState?.project.branch ?? null);
      return;
    }
    if (!activeProject?.path || autoGitBranch !== "true") {
      setBranch(null);
      return;
    }

    let alive = true;
    void gitBranch(activeProject.path).then((found) => {
      if (alive) setBranch(found);
    });
    return () => {
      alive = false;
    };
  }, [route.demo, demoState, activeProject?.path, autoGitBranch]);

  const selectModel = (name: string) => {
    setModel(name);
    void setSetting(MODEL_SETTING, name);
    // `default-model` overrides only the primary model at runtime while the
    // request still rides the active provider's key/endpoint — switch the
    // active provider to the model's owner so credentials always match.
    const owner = providers.findIndex(
      (p) =>
        p.models?.some((m) => m.model_id.toLowerCase() === name.toLowerCase()) ||
        providerModelId(p).toLowerCase() === name.toLowerCase() ||
        p.model.toLowerCase() === name.toLowerCase(),
    );
    if (owner >= 0 && owner !== activeProviderIndex) {
      setActiveProviderIndex(owner);
      void saveActiveIndex(owner);
    }
  };

  const selectProvider = (index: number) => {
    setActiveProviderIndex(index);
    void saveActiveIndex(index);
    const p = providers[index];
    if (p?.model) {
      setModel(p.model);
      void setSetting(MODEL_SETTING, p.model);
    }
  };

  /** Switch model inside a provider from the composer without leaving the conversation. */
  const selectProviderModel = (providerIndex: number, modelId: string, displayName: string) => {
    const next = providers.map((item, index) =>
      index === providerIndex
        ? { ...item, model: modelId, modelId, displayName }
        : item,
    );
    setProviders(next);
    setActiveProviderIndex(providerIndex);
    void saveActiveIndex(providerIndex);
    void saveProviders(next).then(() => {
      setModel(modelId);
      void setSetting(MODEL_SETTING, modelId);
    });
  };

  /**
   * Start a fresh task.
   *
   * - no argument (新建任务) revives the remembered project when it is still
   *   registered in the workspace; otherwise the task starts without one;
   * - an explicit path selects that project and remembers it;
   * - `null` (清除项目选择) clears the selection only — it never revives and
   *   never forgets the remembered project.
   */
  /** True once the user (or boot) has decided the project for this task. */
  const projectChosen = useRef(false);

  const newChat = (projectPath?: string | null) => {
    projectChosen.current = true;
    setActiveConversationId(null);
    if (projectPath === undefined) {
      const remembered = window.localStorage.getItem(LAST_PROJECT_KEY);
      const known = Boolean(remembered) && workspace.projects.some((project) => project.id === remembered);
      setActiveProjectId(known ? remembered : null);
      return;
    }
    setActiveProjectId(projectPath);
    if (projectPath) window.localStorage.setItem(LAST_PROJECT_KEY, projectPath);
  };

  /** Listing-only removal: drop selection state the removed project owned. */
  const removeProject = async (path: string) => {
    await workspace.remove(path);
    if (window.localStorage.getItem(LAST_PROJECT_KEY) === path) {
      window.localStorage.removeItem(LAST_PROJECT_KEY);
    }
    if (activeProject?.path === path) {
      // The active conversation or the new-task selection belonged to it —
      // back to the welcome stage rather than a dangling reference.
      setActiveConversationId(null);
      setActiveProjectId(null);
    }
  };

  /**
   * Switch agent spaces. An open conversation lives only in its own space, so
   * leaving it drops back to the welcome stage — switching back finds it there.
   */
  const switchMode = (next: string) => {
    const current = workspace.projects
      .flatMap((project) => project.conversations)
      .find((item) => item.id === activeConversationId);
    if (activeConversationId !== null && (!current || modeOf(current) !== next)) {
      setActiveConversationId(null);
    }
    setAgentMode(next);
  };

  // Rebuild the query with URLSearchParams so `?type=` (the 插件 tab) and any
  // other param survive — string concatenation would silently reset the tab.
  const setInspectorOpen = (open: boolean) => {
    const params = new URLSearchParams(window.location.search);
    if (open) params.set("inspector", "open");
    else params.set("inspector", "closed");
    navigate(`${window.location.pathname}?${params.toString()}`);
  };

  const openFiles = () => {
    setInspectorTab("files");
    setInspectorOpen(true);
  };

  /** One entry for every "show this file in the sidebar" ask (rows, review, top bar). */
  const openFilePanel = (request: FilePanelRequest) => {
    setFileRequest(request);
    setInspectorTab("file");
    setInspectorOpen(true);
  };

  // The 文件 chip is a panel-layer request: demo / no project falls back to
  // Inspector Files (the fixture carries no file bodies), and a bare chip click
  // seeds an empty browse request so the tree — not a file — is the entry.
  const selectInspectorTab = (next: InspectorTab) => {
    if (next !== "file") {
      setInspectorTab(next);
      return;
    }
    if (route.demo || !activeProject?.path) {
      openFiles();
      return;
    }
    if (!fileRequest) {
      setFileRequest({
        path: null,
        tab: "full",
        changes: liveSnapshot?.reply?.changes ?? [],
        nonce: Date.now(),
      });
    }
    setInspectorTab("file");
  };

  // Switching conversations invalidates the open file request and its diff map;
  // the panel steps back to Inspector (浏览 is conversation-independent and stays).
  useEffect(() => {
    setFileRequest(null);
    setFileDiffs({});
    setInspectorTab((tab) => (tab === "file" ? "summary" : tab));
  }, [activeConversationId]);

  const openTrace = () => {
    navigate(workspace.live ? "/trace" : "/ui-demo/trace");
  };

  /** One path for "open this conversation" — sidebar rows and ⌘K share it. */
  const openConversation = (id: string) => {
    setActiveConversationId(id);
    // Opening a conversation always shows it — including from the
    // Response Trace / 插件 / Settings pages (how you get back).
    if (route.name !== "conversation") {
      navigate(workspace.live ? "/conversation" : "/ui-demo/conversation");
    }
  };

  // ⌘K opens the quick switcher; ⌘N starts a new task. A menu owns the
  // keyboard while open, and ⌘N never steals a keystroke from a text field.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();
      if (!(event.metaKey || event.ctrlKey) || event.altKey) return;
      if (key !== "k" && key !== "n") return;
      if (document.querySelector('[aria-haspopup="menu"][aria-expanded="true"]')) return;
      if (key === "n") {
        const element = document.activeElement;
        const typing =
          element instanceof HTMLInputElement ||
          element instanceof HTMLTextAreaElement ||
          (element instanceof HTMLElement && element.isContentEditable);
        if (typing) return;
      }
      event.preventDefault();
      if (key === "k") setPaletteOpen(true);
      else newChat();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  const paletteActions: PaletteAction[] = [
    { id: "new-task", label: T.nav.newTask, run: () => void newChat() },
    { id: "plugins", label: T.nav.plugins, run: () => navigate(hrefTo("plugins", route.inspectorOpen)) },
    { id: "archive", label: T.page.archive.title, run: () => navigate(hrefTo("archive", route.inspectorOpen)) },
    { id: "settings", label: T.nav.settings, run: () => setSettingsOpen(true) },
  ];

  // Current-space conversations, flattened (the sidebar is the tree; the
  // palette is a flat switcher over the same rows). Demo keeps commands only.
  const paletteSessions: PaletteSession[] = workspace.live
    ? workspace.projects.flatMap((project) =>
        project.conversations.map((conversation) => ({
          id: conversation.id,
          title: conversation.title,
          project: project.name,
          run: () => openConversation(conversation.id),
        })),
      )
    : [];

  const restoreArchive = async (id: string) => {
    await workspace.restore(id);
    setArchiveSelection(null);
    setArchiveReload((n) => n + 1);
    await workspace.refresh();
  };

  const activeProvider = providers[activeProviderIndex] ?? null;
  const projectName = activeProject?.name ?? (route.demo && demoState ? demoState.project.name : "未选择项目");

  return (
    <div
      className={`app${sidebarVisible ? "" : " app--sidebar-hidden"}${
        systemFont === "true" ? " app--system-font" : ""
      }${focusModeActive ? " app--focus-mode" : ""}${dragWidth === null ? "" : " app--resizing"}`}
      // Wide windows own both widths inline (inherited down to .ins-panel);
      // narrow ones fall back to the tokens so the overlay clamp stays intact.
      style={
        wide
          ? ({
              "--kodo-inspector-width": `${wideMode ? baseWidth : appliedWidth}px`,
              "--kodo-inspector-file-width": `${wideMode ? appliedWidth : wideWidth}px`,
              "--kodo-conversation-min-width": `${CONVERSATION_WIDTH_MIN}px`,
            } as CSSProperties)
          : undefined
      }
      data-core={coreVersion ?? undefined}
      data-density={density}
      data-line-numbers={lineNumbers}
    >
      <Sidebar
        route={route.name}
        inspectorOpen={route.inspectorOpen}
        activeConversationId={activeConversationId}
        onSelectConversation={openConversation}
        onNewTask={() => void newChat()}
        onOpenSettings={() => setSettingsOpen(true)}
        onToggleSidebar={() => setSidebarVisible((value) => !value)}
        onStartConversation={(path) => void newChat(path)}
        onRenameConversation={async (id, title) => {
          await workspace.retitle(id, title);
        }}
        onArchiveConversation={async (id) => {
          await workspace.archive(id);
          if (activeConversationId === id) setActiveConversationId(null);
        }}
        onDeleteConversation={async (id) => {
          await workspace.removeSession(id);
          // Deleting the open conversation lands on the new-task stage —
          // same as archive: the id must not outlive its session file.
          if (activeConversationId === id) setActiveConversationId(null);
        }}
        runningConversationId={
          liveSnapshot?.running ? liveSnapshot.conversationId : null
        }
        onRemoveProject={removeProject}
        agentMode={agentMode}
        onAgentModeChange={switchMode}
        workspace={workspace}
      />

      <div className="content">
        {/* Conversation always has a TopBar. Other routes only need one while
            the sidebar is collapsed, so the expand control + nav icons never
            disappear after navigating away from the conversation. */}
        {(route.name === "conversation" || !sidebarVisible) && (
          <TopBar
            inspectorOpen={route.inspectorOpen}
            onToggleInspector={() => setInspectorOpen(!route.inspectorOpen)}
            sidebarVisible={sidebarVisible}
            onToggleSidebar={() => setSidebarVisible((value) => !value)}
            onNewTask={() => void newChat()}
            onOpenSettings={() => setSettingsOpen(true)}
            onOpenFiles={
              route.name === "conversation" ? () => setOpenFilesRequest((n) => n + 1) : undefined
            }
            route={route.name}
            crumbProject={projectName}
            projectPath={activeProject?.path ?? ""}
            onOpenMemoryTab={
              route.name === "conversation"
                ? () => {
                    setInspectorOpen(true);
                    setInspectorTab("memory");
                  }
                : undefined
            }
            projectOptions={workspace.projects.filter((project) => Boolean(project.path))}
            onSelectProject={activeConversationId === null && !liveSnapshot?.running ? newChat : undefined}
            onAddProject={workspace.live ? async () => {
              const picked = await pickFolder();
              if (!picked) return;
              await workspace.add(picked);
              newChat(picked);
            } : undefined}
            crumbTitle={
              demoState?.conversation.title ??
              workspace.projects
                .flatMap((p) => p.conversations)
                .find((c) => c.id === activeConversationId)?.title ??
              // The title row left the content area (2026-09-30) — the crumb
              // is now the only title surface, so it always names the stage.
              T.page.conversationTitle
            }
            branch={branch}
            convActions={convActions}
            memoryRecalledCount={memoryRecalledCount}
          />
        )}

        <div className="content-body" ref={contentRef}>
          {/* A live run must survive opening the Response Trace: the page stays
              mounted (stowed) while the trace shows, so the run-event stream,
              queue and approval state keep flowing mid-answer (2026-09-30).
              Idle conversations unmount as before — reload is cheaper. */}
          {(route.name === "conversation" ||
            (route.name === "trace" && Boolean(liveSnapshot?.running))) && (
            <ConversationPage
              stowed={route.name !== "conversation"}
              conversationId={activeConversationId}
              agentMode={agentMode}
              provider={activeProvider}
              providers={providers}
              defaultModel={model}
              onSelectProvider={selectProvider}
              onSelectProviderModel={selectProviderModel}
              onViewFiles={openFiles}
              onOpenTrace={openTrace}
              openFilesRequest={openFilesRequest}
              onOpenFilePanel={openFilePanel}
              onFileDiffs={setFileDiffs}
              onSnapshot={onSnapshot}
              onConversationCreated={(id) => {
                setActiveConversationId(id);
                if (activeProject?.path) {
                  setActiveProjectId(activeProject.path);
                  window.localStorage.setItem(LAST_PROJECT_KEY, activeProject.path);
                }
              }}
              onConversationCommitted={() => workspace.refresh()}
              onOpenSettings={() => setSettingsOpen(true)}
              onOpenConversation={openConversation}
              projects={workspace.projects.filter((project) => project.path !== "")}
              projectName={projectName}
              projectPath={activeProject?.path ?? ""}
              demo={demoState}
              onRetitle={async (id, title) => {
                await workspace.retitle(id, title);
              }}
              onArchive={async (id) => {
                await workspace.archive(id);
                setActiveConversationId(null);
                setArchiveReload((n) => n + 1);
              }}
              onConvActions={setConvActions}
              onMemoryRecalled={setMemoryRecalledCount}
              personaListBySession={personaListBySession}
              onSelectAgents={selectAgents}
              favoriteAgents={favoriteAgents}
              recentAgents={recentAgents}
              recordRecentAgent={recordRecentAgent}
              agentsVersion={agentsVersion}
            />
          )}
          {route.name === "plugins" && (
            // The demo fixture's sidebar projects record no `path`, so the
            // skills tab falls back to the recorded demo project path — the
            // same compensation the header name and branch effects use.
            <PluginsPage
              agentMode={agentMode}
              projectPath={activeProject?.path ?? demoState?.project.path ?? null}
              type={route.pluginType}
              agentsVersion={agentsVersion}
              bumpAgentsVersion={bumpAgentsVersion}
              favoriteAgents={favoriteAgents}
              onToggleFavorite={toggleFavoriteAgent}
            />
          )}
          {route.name === "trace" && (
            <TracePage demo={demoState} conversationId={activeConversationId} />
          )}
          {route.name === "archive" && (
            <ArchivePage
              demo={route.demo}
              demoState={demoState}
              selectedId={archiveSelection?.id ?? null}
              reloadToken={archiveReload}
              onSelect={setArchiveSelection}
            />
          )}
          {route.name === "settings" && (
            <SettingsPage
              model={model}
              onSelectModel={selectModel}
              onProvidersSaved={(list) => {
                setProviders(list);
                resolveModelFromProviders(list);
              }}
              onOpenArchive={() => navigate(workspace.live ? "/archive" : "/ui-demo/archive")}
            />
          )}
          {route.name === "schedule" && <SchedulerPage demo={route.demo} />}

          {paletteOpen && (
            <CommandPalette
              actions={paletteActions}
              sessions={paletteSessions}
              agents={paletteAgents}
              onClose={() => setPaletteOpen(false)}
            />
          )}

          {settingsOpen && (
            <div className="settings-modal" role="dialog" aria-modal="true" aria-label="Settings">
              <button
                type="button"
                className="settings-modal-backdrop"
                aria-label={T.page.settings.rows.closeAria}
                onClick={() => setSettingsOpen(false)}
              />
              <SettingsPage
                modal
                onClose={() => setSettingsOpen(false)}
                onOpenArchive={() => {
                  setSettingsOpen(false);
                  navigate(workspace.live ? "/archive" : "/ui-demo/archive");
                }}
                model={model}
                onSelectModel={selectModel}
                onProvidersSaved={(list) => {
                  setProviders(list);
                  resolveModelFromProviders(list);
                }}
              />
            </div>
          )}

          {route.name !== "schedule" && (
            <Inspector
              route={route.name}
              open={route.inspectorOpen}
              tab={inspectorTab}
              onSelectTab={selectInspectorTab}
              onOpen={() => setInspectorOpen(true)}
              onClose={() => setInspectorOpen(false)}
              demo={route.demo}
              demoState={demoState}
              live={liveSnapshot}
              project={activeProject ? { name: activeProject.name, path: activeProject.path ?? "" } : null}
              pluginType={route.pluginType}
              archiveSelection={archiveSelection}
              onArchiveRestore={() => {
                if (!archiveSelection) return;
                void restoreArchive(archiveSelection.id);
              }}
              onOpenTrace={openTrace}
              filePanel={
                fileRequest && activeProject?.path ? (
                  <FileViewer
                    projectPath={activeProject.path}
                    diffs={fileDiffs}
                    changed={fileRequest.changes}
                    initialPath={fileRequest.path}
                    initialTab={fileRequest.tab}
                    nonce={fileRequest.nonce}
                    compact={!wide || appliedWidth < 560}
                    onClose={() => setInspectorTab("summary")}
                  />
                ) : null
              }
              browsePanel={<BrowsePanel onClose={() => setInspectorTab("summary")} />}
              resizer={
                wide ? (
                  <PanelResizer
                    value={appliedWidth}
                    onDrag={setDragWidth}
                    onCommit={commitInspectorWidth}
                    max={route.name === "conversation" ? panelMaxWidth : restoreWidth}
                    restoreWidth={restoreWidth}
                    onToggleFocus={toggleInspectorFocus}
                  />
                ) : null
              }
            />
          )}
        </div>
      </div>

      {/* First launch only; resolves its own setting so nothing flashes. */}
      <AutoOnboarding demo={route.demo} />
    </div>
  );
}
