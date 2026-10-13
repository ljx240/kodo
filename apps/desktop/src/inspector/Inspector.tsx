import { BarChart3, FileText, Info, Wrench, X } from "lucide-react";
import { useEffect, useRef, type ReactNode } from "react";
import type { ChangeDto } from "../api";
import type { PluginType, RouteName } from "../routes";
import type { DemoState } from "../data/demoState";
import type { LiveSnapshot } from "../data/liveContext";
import { T } from "../i18n";
import { ArchiveOverview, type ArchiveSelection } from "./ArchiveInspector";
import { HelpOverview } from "./HelpInspector";
import { MemoryOverview } from "./MemoryInspector";
import {
  ChangedFilesSection,
  LlmSection,
  ResponseOverview,
  TerminalSection,
  ToolsSection,
  type InspectorData,
} from "./ResponseInspector";
import { TraceOverview } from "./TraceInspector";

/**
 * One layer navigates panels (Inspector | 终端 | 文件 | 浏览), the other
 * navigates sections inside the Inspector panel (概览 / 改动 / 工具 / 模型).
 * The two never share a name or a meaning: Terminal, 文件 and 浏览 live only
 * at the panel layer, so the rail keeps four unique labels and nothing
 * collides across layers.
 */
export type InspectorTab =
  | "summary"
  | "files"
  | "tools"
  | "terminal"
  | "llm"
  | "help"
  | "file"
  | "browse"
  | "memory";

/**
 * One open-file request for the right-sidebar file panel. `nonce` bumps on
 * every ask so re-clicking the same path still re-syncs a panel the user
 * browsed away from (prop identity, not just path, drives the sync effect).
 */
export type FilePanelRequest = {
  path: string | null;
  tab: "full" | "diff";
  changes: ChangeDto[];
  nonce: number;
};

const RAIL: { tab: InspectorTab; label: string; Icon: typeof Info }[] = [
  { tab: "summary", label: T.inspector.summary, Icon: Info },
  { tab: "files", label: T.inspector.files, Icon: FileText },
  { tab: "tools", label: T.inspector.tools, Icon: Wrench },
  { tab: "llm", label: T.inspector.llm, Icon: BarChart3 },
];

/** Panel-layer tabs per route. Conversation switches whole panels here. */
const PANEL_TABS: Record<RouteName, { tab: InspectorTab; label: string }[]> = {
  conversation: [
    { tab: "summary", label: T.inspector.panel },
    { tab: "terminal", label: T.inspector.terminal },
    { tab: "file", label: T.inspector.file },
    { tab: "browse", label: T.inspector.browse },
    { tab: "memory", label: T.memory.section },
  ],
  plugins: [{ tab: "summary", label: T.inspector.panel }],
  trace: [
    { tab: "summary", label: T.inspector.panel },
    { tab: "terminal", label: T.inspector.terminal },
  ],
  archive: [{ tab: "summary", label: T.inspector.panel }],
  settings: [{ tab: "help", label: T.inspector.help }],
  // `schedule` is a self-contained page; App.tsx skips the Inspector entirely
  // on that route. The summary tab is here only to satisfy
  // `Record<RouteName, …>` and to guard `panelTabs[0].tab` if the route ever
  // re-enables the Inspector.
  schedule: [{ tab: "summary", label: T.inspector.panel }],
};

function Tabs({
  tabs,
  active,
  chips,
  onSelect,
}: {
  tabs: { tab: InspectorTab; label: string }[];
  active: InspectorTab;
  chips?: boolean;
  onSelect: (tab: InspectorTab) => void;
}) {
  return (
    <nav className={`ins-tabs${chips ? " ins-tabs--chips" : ""}`} role="tablist" aria-label={T.inspector.panel}>
      {tabs.map((item) => (
        <button
          key={item.tab}
          type="button"
          role="tab"
          aria-selected={item.tab === active}
          className={`ins-tab${item.tab === active ? " ins-tab--active" : ""}`}
          onClick={() => onSelect(item.tab)}
        >
          {item.label}
        </button>
      ))}
    </nav>
  );
}

function contentFor(
  route: RouteName,
  tab: InspectorTab,
  demo: boolean,
  demoState: DemoState | null,
  live: LiveSnapshot | null,
  archiveSelection: ArchiveSelection,
  onArchiveRestore: () => void,
  onOpenTrace: () => void,
  onSelectTab: (tab: InspectorTab) => void,
  project: InspectorData["project"],
  pluginType: PluginType,
) {
  const data = { demo, demoState, live, project };
  if (route === "conversation") {
    // Terminal is a panel of its own — never a rail section beside itself.
    if (tab === "terminal") return <TerminalSection {...data} />;
    if (tab === "files") return <ChangedFilesSection {...data} />;
    if (tab === "tools") return <ToolsSection {...data} />;
    if (tab === "llm") return <LlmSection {...data} detailed />;
    if (tab === "memory") {
      return (
        <MemoryOverview
          project={project?.path ?? null}
          onSelectTab={onSelectTab}
        />
      );
    }
    return <ResponseOverview {...data} onShowAllFiles={() => onSelectTab("files")} />;
  }
  if (route === "trace") {
    return tab === "terminal" ? <TerminalSection {...data} /> : <TraceOverview {...data} />;
  }
  if (route === "archive") {
    return (
      <ArchiveOverview
        demo={demo}
        demoState={demoState}
        selection={archiveSelection}
        onRestore={onArchiveRestore}
        onOpenTrace={onOpenTrace}
      />
    );
  }
  return <HelpOverview pluginType={route === "plugins" ? pluginType : undefined} />;
}

type Props = {
  route: RouteName;
  open: boolean;
  tab: InspectorTab;
  onSelectTab: (tab: InspectorTab) => void;
  onOpen: () => void;
  onClose: () => void;
  demo: boolean;
  demoState?: DemoState | null;
  live: LiveSnapshot | null;
  project?: InspectorData["project"];
  pluginType?: PluginType;
  archiveSelection?: ArchiveSelection;
  onArchiveRestore?: () => void;
  onOpenTrace?: () => void;
  /** Slot body for the 文件 panel — FileViewer de-modalized (App mounts it). */
  filePanel?: ReactNode;
  /** Slot body for the 浏览 panel — embedded browser (App mounts it). */
  browsePanel?: ReactNode;
  /** Width handle on the boundary (App gates it to ≥1200px windows). */
  resizer?: ReactNode;
};

export function Inspector({
  route,
  open,
  tab,
  onSelectTab,
  onOpen,
  onClose,
  demo,
  demoState = null,
  live,
  project,
  pluginType = "skills",
  archiveSelection = null,
  onArchiveRestore = () => {},
  onOpenTrace = () => {},
  filePanel = null,
  browsePanel = null,
  resizer = null,
}: Props) {
  const panelTabs = PANEL_TABS[route];
  const allowed = new Set(panelTabs.map((item) => item.tab));
  // The rail lists the sections that exist on this route: all four inside the
  // conversation's Inspector panel, only 概览 elsewhere. Settings draws its own
  // help panel and has no rail at all.
  const rail = RAIL.filter((item) => {
    if (route === "settings") return false;
    if (route === "conversation") return true;
    return item.tab === "summary";
  });
  const railActive: InspectorTab = rail.some((item) => item.tab === tab) ? tab : "summary";
  // Terminal is a panel-level tab; a rail section always lives inside the
  // Inspector panel, so the panel layer stays on 概览 while the rail picks the
  // section. Anything else falls back to the route's first panel tab.
  const panelActive: InspectorTab =
    tab === "terminal" && allowed.has("terminal")
      ? "terminal"
      : allowed.has(tab)
        ? tab
        : rail.some((item) => item.tab === tab)
          ? "summary"
          : panelTabs[0].tab;
  // The rail lists sections inside the Inspector panel only — Terminal, 文件
  // and 浏览 are whole panels of their own, so they show no rail beside themselves.
  const showRail = rail.length > 0 && panelActive === "summary";
  // 文件 and 浏览 both own the whole column: they widen it and drop the padding.
  const wholePanel = panelActive === "file" || panelActive === "browse";

  // Narrow viewports draw the panel as an overlay. Opening parks focus inside
  // it; closing hands focus back to whatever opened it, so the keyboard never
  // sits under the overlay or inside unmounted content.
  const panelRef = useRef<HTMLDivElement | null>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const wasOpen = useRef(open);

  useEffect(() => {
    if (open && !wasOpen.current) {
      restoreFocusRef.current =
        document.activeElement instanceof HTMLElement ? document.activeElement : null;
      // Focus after paint so the panel exists when we reach for its controls.
      requestAnimationFrame(() => {
        const close = panelRef.current?.querySelector<HTMLElement>(".ins-panel-close");
        (close ?? panelRef.current)?.focus();
      });
    }
    if (!open && wasOpen.current) {
      restoreFocusRef.current?.focus();
      restoreFocusRef.current = null;
    }
    wasOpen.current = open;
  }, [open]);

  return (
    <aside
      className={`inspector${wholePanel ? " inspector--wide" : ""}`}
      id="kodo-inspector"
    >
      {open && resizer}
      {showRail && (
        <div className="ins-rail">
          {rail.map(({ tab: railTab, label, Icon }) => (
            <button
              key={railTab}
              type="button"
              className={`rail-btn${open && railActive === railTab ? " rail-btn--active" : ""}`}
              title={label}
              aria-label={label}
              aria-expanded={open && railActive === railTab}
              aria-controls={open ? "kodo-inspector-panel" : undefined}
              data-testid={`rail-${railTab}`}
              onClick={() => {
                onSelectTab(railTab);
                if (!open) onOpen();
              }}
            >
              <Icon size={18} strokeWidth={1.8} />
            </button>
          ))}
        </div>
      )}

      {open && (
        <div
          className="ins-panel"
          id="kodo-inspector-panel"
          role="region"
          aria-label={T.inspector.panel}
          tabIndex={-1}
          ref={panelRef}
        >
          <header className="ins-panel-head ins-panel-head--chips">
            <Tabs
              tabs={panelTabs}
              active={panelActive}
              chips
              onSelect={(next) => {
                onSelectTab(next);
                if (!open) onOpen();
              }}
            />
            <span className="spacer" />
            <button
              type="button"
              className="icon-btn ins-panel-close"
              aria-label={T.action.close}
              title={T.action.close}
              data-testid="inspector-close"
              // In 文件/浏览 the X steps back to Inspector (the panel stays
              // usable); from the other panels it closes the panel as before.
              onClick={() => {
                if (wholePanel) onSelectTab("summary");
                else onClose();
              }}
            >
              <X size={16} strokeWidth={1.8} />
            </button>
          </header>

          <div
            key={panelActive}
            className={`ins-content${wholePanel ? " ins-content--flush" : ""}`}
            role="tabpanel"
          >
            {panelActive === "file" ? (
              filePanel
            ) : panelActive === "browse" ? (
              browsePanel
            ) : (
              contentFor(
                route,
                panelActive === "terminal" ? "terminal" : railActive,
                demo,
                demoState,
                live,
                archiveSelection,
                onArchiveRestore,
                onOpenTrace,
                onSelectTab,
                project,
                pluginType,
              )
            )}
          </div>
        </div>
      )}
    </aside>
  );
}
