import { BarChart3, FileText, Info, Wrench, X } from "lucide-react";
import { useEffect, useRef } from "react";
import type { RouteName } from "../routes";
import type { DemoState } from "../data/demoState";
import type { LiveSnapshot } from "../data/liveContext";
import { T } from "../i18n";
import { ArchiveOverview, type ArchiveSelection } from "./ArchiveInspector";
import { HelpOverview } from "./HelpInspector";
import {
  ChangedFilesSection,
  LlmSection,
  ResponseOverview,
  TerminalSection,
  ToolsSection,
} from "./ResponseInspector";
import { TraceOverview } from "./TraceInspector";

/**
 * One layer navigates panels (Inspector | 终端), the other navigates sections
 * inside the Inspector panel (概览 / 文件 / 工具 / 模型). The two never share a
 * name or a meaning: Terminal lives only at the panel layer, so the rail stays
 * four items and no label collides across layers.
 */
export type InspectorTab = "summary" | "files" | "tools" | "terminal" | "llm" | "help";

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
  ],
  skills: [{ tab: "summary", label: T.inspector.panel }],
  trace: [
    { tab: "summary", label: T.inspector.panel },
    { tab: "terminal", label: T.inspector.terminal },
  ],
  archive: [{ tab: "summary", label: T.inspector.panel }],
  settings: [{ tab: "help", label: T.inspector.help }],
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
) {
  const data = { demo, demoState, live };
  if (route === "conversation") {
    // Terminal is a panel of its own — never a rail section beside itself.
    if (tab === "terminal") return <TerminalSection {...data} />;
    if (tab === "files") return <ChangedFilesSection {...data} />;
    if (tab === "tools") return <ToolsSection {...data} />;
    if (tab === "llm") return <LlmSection {...data} detailed />;
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
  return <HelpOverview />;
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
  archiveSelection?: ArchiveSelection;
  onArchiveRestore?: () => void;
  onOpenTrace?: () => void;
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
  archiveSelection = null,
  onArchiveRestore = () => {},
  onOpenTrace = () => {},
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
  const showRail = rail.length > 0 && panelActive !== "terminal";

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
    <aside className="inspector" id="kodo-inspector">
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
              onClick={onClose}
            >
              <X size={16} strokeWidth={1.8} />
            </button>
          </header>

          <div className="ins-content" role="tabpanel">
            {contentFor(
              route,
              panelActive === "terminal" ? "terminal" : railActive,
              demo,
              demoState,
              live,
              archiveSelection,
              onArchiveRestore,
              onOpenTrace,
              onSelectTab,
            )}
          </div>
        </div>
      )}
    </aside>
  );
}
