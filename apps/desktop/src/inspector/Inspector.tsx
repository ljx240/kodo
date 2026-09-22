import { BarChart3, ChevronRight, FileText, Info, Terminal, Wrench, X } from "lucide-react";
import type { RouteName } from "../routes";
import type { DemoState } from "../data/demoState";
import type { LiveSnapshot } from "../data/liveContext";
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

export type InspectorTab = "summary" | "files" | "tools" | "terminal" | "llm" | "help";

const RAIL: { tab: InspectorTab; label: string; Icon: typeof Info }[] = [
  { tab: "summary", label: "Summary", Icon: Info },
  { tab: "files", label: "Files", Icon: FileText },
  { tab: "tools", label: "Tools", Icon: Wrench },
  { tab: "terminal", label: "Terminal", Icon: Terminal },
  { tab: "llm", label: "LLM", Icon: BarChart3 },
];

const TABS: Record<RouteName, { tab: InspectorTab; label: string }[]> = {
  conversation: [
    { tab: "summary", label: "Summary" },
    { tab: "files", label: "Files" },
    { tab: "tools", label: "Tools" },
    { tab: "llm", label: "LLM" },
  ],
  trace: [
    { tab: "summary", label: "Inspector" },
    { tab: "terminal", label: "Terminal" },
  ],
  archive: [{ tab: "summary", label: "Inspector" }],
  settings: [{ tab: "help", label: "Help" }],
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
    <nav className={`ins-tabs${chips ? " ins-tabs--chips" : ""}`} role="tablist" aria-label="Inspector sections">
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
  const tabs = TABS[route];
  const allowed = new Set(tabs.map((item) => item.tab));
  // Rail only offers destinations this route actually has.
  const rail = RAIL.filter((item) => allowed.has(item.tab));
  const active = allowed.has(tab) ? tab : tabs[0].tab;

  return (
    <aside className="inspector" id="kodo-inspector">
      <div className="ins-rail">
        {rail.map(({ tab: railTab, label, Icon }) => (
          <button
            key={railTab}
            type="button"
            className={`rail-btn${open && active === railTab ? " rail-btn--active" : ""}`}
            title={label}
            aria-label={label}
            aria-expanded={open && active === railTab}
            aria-controls={open ? "kodo-inspector-panel" : undefined}
            onClick={() => {
              onSelectTab(railTab);
              if (!open) onOpen();
            }}
          >
            <Icon size={18} strokeWidth={1.8} />
          </button>
        ))}
      </div>

      {open && (
        <div className="ins-panel" id="kodo-inspector-panel" role="region" aria-label="Inspector panel">
          {route === "conversation" ? (
            <>
              <header className="ins-panel-head">
                <h2 className="ins-panel-title">Inspector</h2>
                <span className="spacer" />
                <button type="button" className="icon-btn" aria-label="Collapse inspector" onClick={onClose}>
                  <X size={16} strokeWidth={1.8} />
                </button>
              </header>
              <Tabs tabs={tabs} active={active} onSelect={onSelectTab} />
            </>
          ) : (
            <header className="ins-panel-head ins-panel-head--chips">
              <Tabs tabs={tabs} active={active} chips onSelect={onSelectTab} />
              <span className="spacer" />
              <button type="button" className="icon-btn" aria-label="Collapse inspector" onClick={onClose}>
                <ChevronRight size={16} strokeWidth={1.8} />
              </button>
            </header>
          )}

          <div className="ins-content" role="tabpanel">
            {contentFor(
              route,
              active,
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
