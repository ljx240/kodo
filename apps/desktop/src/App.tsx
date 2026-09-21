import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { coreInfo, gitBranch, setting, setSetting } from "./api";
import { loadDemoState, type DemoState } from "./data/demoState";
import { DEFAULT_MODEL, MODEL_SETTING, MODELS } from "./data/models";
import {
  type ProviderConfig,
  loadProviders,
  loadActiveIndex,
  saveActiveIndex,
  saveProviders,
} from "./data/providers";
import { useSetting } from "./data/useSetting";
import type { LiveSnapshot } from "./data/liveContext";
import { useWorkspace } from "./data/workspace";
import { navigate, useRoute } from "./routes";
import { Sidebar } from "./shell/Sidebar";
import { Inspector, type InspectorTab } from "./inspector/Inspector";
import type { ArchiveSelection } from "./inspector/ArchiveInspector";
import { ArchivePage } from "./pages/ArchivePage";
import { ConversationPage } from "./pages/ConversationPage";
import { SettingsPage } from "./pages/SettingsPage";
import { TracePage } from "./pages/TracePage";
import { TopBar } from "./shell/TopBar";

export function App() {
  const route = useRoute();
  const workspace = useWorkspace(route.demo);
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

  const [providers, setProviders] = useState<ProviderConfig[]>([]);
  const [activeProviderIndex, setActiveProviderIndex] = useState(0);
  const [sidebarVisible, setSidebarVisible] = useState(true);
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

  // Ensure a conversation is open so send / Add context and the live trace are
  // usable without an extra sidebar click. Prefer an existing session;
  // otherwise open one under the first project that has a real path.
  const sessionBootstrapped = useRef(false);
  const startSession = workspace.startSession;
  useEffect(() => {
    if (route.demo || (route.name !== "conversation" && route.name !== "trace") || activeConversationId) return;
    if (sessionBootstrapped.current) return;
    const existing = workspace.projects.flatMap((project) => project.conversations)[0];
    if (existing) {
      setActiveConversationId(existing.id);
      return;
    }
    const project = workspace.projects.find((item) => item.path);
    if (!project?.path || !workspace.live) return;
    sessionBootstrapped.current = true;
    void startSession(project.path).then((id) => {
      if (id) setActiveConversationId(id);
      else sessionBootstrapped.current = false;
    });
  }, [
    route.demo,
    route.name,
    activeConversationId,
    workspace.projects,
    workspace.live,
    startSession,
  ]);

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
      document.documentElement.dataset.theme = resolved;
    };
    apply(theme);
    if (theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => apply("system");
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [theme]);

  const reloadProviders = useCallback(() => {
    void loadProviders().then((list) => {
      setProviders(list);
      void loadActiveIndex().then((idx) => {
        const index = idx < list.length ? idx : 0;
        setActiveProviderIndex(index);
        const active = list[index];
        if (active?.model) setModel(active.model);
      });
    });
  }, []);

  useEffect(() => {
    void coreInfo()
      .catch(() => null)
      .then((info) => setCoreVersion(info ? `${info.name} ${info.version}` : null));

    void setting(MODEL_SETTING).then((stored) => {
      if (stored && MODELS.includes(stored)) setModel(stored);
    });

    reloadProviders();
  }, [reloadProviders]);

  const owner = workspace.projects.find((project) =>
    project.conversations.some((item) => item.id === activeConversationId),
  );
  const activeProject =
    owner ?? workspace.projects.find((project) => project.id === activeProjectId) ?? workspace.projects[0] ?? null;

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

  const newChat = async (projectPath?: string) => {
    const path = projectPath ?? activeProject?.path;
    if (!path) return;
    const id = await workspace.startSession(path);
    if (id) {
      setActiveConversationId(id);
      setActiveProjectId(path);
    }
  };

  const setInspectorOpen = (open: boolean) =>
    navigate(open ? `${location.pathname}?inspector=open` : `${location.pathname}?inspector=closed`);

  const openFiles = () => {
    setInspectorTab("files");
    setInspectorOpen(true);
  };

  const openTrace = () => {
    navigate(workspace.live ? "/trace" : "/ui-demo/trace");
  };

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
      }`}
      data-core={coreVersion ?? undefined}
      data-density={density}
      data-line-numbers={lineNumbers}
    >
      <Sidebar
        route={route.name}
        inspectorOpen={route.inspectorOpen}
        activeConversationId={activeConversationId}
        onSelectConversation={setActiveConversationId}
        onStartConversation={(path) => void newChat(path)}
        onRenameConversation={async (id, title) => {
          await workspace.retitle(id, title);
        }}
        onArchiveConversation={async (id) => {
          await workspace.archive(id);
          if (activeConversationId === id) setActiveConversationId(null);
        }}
        workspace={workspace}
      />

      <div className="content">
        {route.name === "conversation" && (
          <TopBar
            projects={workspace.projects}
            activeProjectId={activeProject?.id ?? null}
            onSelectProject={setActiveProjectId}
            projectName={projectName}
            branch={branch}
            provider={activeProvider}
            onNewChat={(path) => void newChat(path)}
            inspectorOpen={route.inspectorOpen}
            onToggleInspector={() => setInspectorOpen(!route.inspectorOpen)}
            sidebarVisible={sidebarVisible}
            onToggleSidebar={() => setSidebarVisible((value) => !value)}
          />
        )}

        <div className="content-body">
          {route.name === "conversation" && (
            <ConversationPage
              conversationId={activeConversationId}
              provider={activeProvider}
              providers={providers}
              onSelectProvider={selectProvider}
              onSelectProviderModel={selectProviderModel}
              onViewFiles={openFiles}
              onOpenTrace={openTrace}
              onSnapshot={onSnapshot}
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
                void loadActiveIndex().then((idx) => {
                  const index = idx < list.length ? idx : 0;
                  setActiveProviderIndex(index);
                  const active = list[index];
                  if (active?.model) {
                    setModel(active.model);
                    void setSetting(MODEL_SETTING, active.model);
                  }
                });
              }}
            />
          )}

          <Inspector
            route={route.name}
            open={route.inspectorOpen}
            tab={inspectorTab}
            onSelectTab={setInspectorTab}
            onOpen={() => setInspectorOpen(true)}
            onClose={() => setInspectorOpen(false)}
            demo={route.demo}
            demoState={demoState}
            live={liveSnapshot}
            archiveSelection={archiveSelection}
            onArchiveRestore={() => {
              if (!archiveSelection) return;
              void restoreArchive(archiveSelection.id);
            }}
            onOpenTrace={openTrace}
          />
        </div>
      </div>
    </div>
  );
}
