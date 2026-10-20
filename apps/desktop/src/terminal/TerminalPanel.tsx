import { useRef, useState, type KeyboardEvent } from "react";
import { Columns2, Plus, Rows2, Terminal as TerminalIcon, X } from "lucide-react";
import { T } from "../i18n";
import { TerminalPane } from "./TerminalPane";
import {
  firstPaneId,
  newId,
  paneLayout,
  paneLeaf,
  paneLeaves,
  removePane,
  splitPane,
  type SplitDirection,
  type TerminalTab,
} from "./paneTree";

function createTab(index: number): TerminalTab {
  const paneId = newId("pane");
  return {
    id: newId("tab"),
    name: T.terminal.tabName(index),
    root: paneLeaf(paneId),
    activePaneId: paneId,
  };
}

const tabDomId = (id: string) => `terminal-tab-${id}`;
const panelDomId = (id: string) => `terminal-tabpanel-${id}`;

export function TerminalPanel({
  projectPath,
  onClose,
}: {
  projectPath: string;
  onClose: () => void;
}) {
  const nextTabNumber = useRef(2);
  const [tabs, setTabs] = useState<TerminalTab[]>(() => [createTab(1)]);
  const [activeTabId, setActiveTabId] = useState(() => tabs[0].id);

  const addTab = () => {
    const tab = createTab(nextTabNumber.current++);
    setTabs((current) => [...current, tab]);
    setActiveTabId(tab.id);
  };

  const splitActivePane = (direction: SplitDirection) => {
    const tab = tabs.find((item) => item.id === activeTabId);
    if (!tab) return;
    const pane = paneLeaf(newId("pane"));
    const root = splitPane(tab.root, tab.activePaneId, pane, direction);
    setTabs((current) =>
      current.map((item) =>
        item.id === tab.id ? { ...item, root, activePaneId: pane.id } : item,
      ),
    );
  };

  const focusPane = (tabId: string, paneId: string) => {
    const tab = tabs.find((item) => item.id === tabId);
    if (tabId === activeTabId && tab?.activePaneId === paneId) return;
    setActiveTabId(tabId);
    setTabs((current) =>
      current.map((item) =>
        item.id === tabId && item.activePaneId !== paneId
          ? { ...item, activePaneId: paneId }
          : item,
      ),
    );
  };

  const closeTab = (tabId: string) => {
    const index = tabs.findIndex((item) => item.id === tabId);
    if (index < 0) return;
    const remaining = tabs.filter((item) => item.id !== tabId);
    if (remaining.length === 0) {
      onClose();
      return;
    }
    if (tabId === activeTabId) {
      setActiveTabId(remaining[Math.min(index, remaining.length - 1)].id);
    }
    setTabs(remaining);
  };

  const closePane = (tabId: string, paneId: string) => {
    const tab = tabs.find((item) => item.id === tabId);
    if (!tab) return;
    const root = removePane(tab.root, paneId);
    if (!root) {
      closeTab(tabId);
      return;
    }
    const activePaneId = tab.activePaneId === paneId ? firstPaneId(root) : tab.activePaneId;
    setTabs((current) =>
      current.map((item) => (item.id === tabId ? { ...item, root, activePaneId } : item)),
    );
  };

  const handleTabKey = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    let next = index;
    if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
    else if (event.key === "ArrowLeft") next = (index - 1 + tabs.length) % tabs.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = tabs.length - 1;
    else return;
    event.preventDefault();
    const target = tabs[next];
    setActiveTabId(target.id);
    requestAnimationFrame(() => document.getElementById(tabDomId(target.id))?.focus());
  };

  return (
    <section className="terminal-panel" aria-label={T.terminal.title} data-testid="terminal-panel">
      <header className="terminal-head">
        <TerminalIcon size={14} strokeWidth={1.8} />
        <div className="terminal-tabs" role="tablist" aria-label={T.terminal.title}>
          {tabs.map((tab, index) => {
            const selected = tab.id === activeTabId;
            return (
              <div
                key={tab.id}
                className={`terminal-tab${selected ? " terminal-tab--active" : ""}`}
              >
                <button
                  type="button"
                  role="tab"
                  id={tabDomId(tab.id)}
                  aria-selected={selected}
                  aria-controls={panelDomId(tab.id)}
                  tabIndex={selected ? 0 : -1}
                  className="terminal-tab-label"
                  data-testid="terminal-tab"
                  onClick={() => setActiveTabId(tab.id)}
                  onKeyDown={(event) => handleTabKey(event, index)}
                >
                  {tab.name}
                </button>
                <button
                  type="button"
                  className="icon-btn icon-btn--sm terminal-tab-close"
                  aria-label={T.terminal.closeTab(tab.name)}
                  title={T.terminal.closeTab(tab.name)}
                  data-testid="terminal-tab-close"
                  onClick={() => closeTab(tab.id)}
                >
                  <X size={11} strokeWidth={2} />
                </button>
              </div>
            );
          })}
        </div>
        <span className="spacer" />
        <div className="terminal-actions">
          <button
            type="button"
            className="icon-btn icon-btn--sm"
            aria-label={T.terminal.newTab}
            title={T.terminal.newTab}
            data-testid="terminal-new"
            onClick={addTab}
          >
            <Plus size={14} strokeWidth={1.8} />
          </button>
          <button
            type="button"
            className="icon-btn icon-btn--sm"
            aria-label={T.terminal.splitRight}
            title={T.terminal.splitRight}
            data-testid="terminal-split-right"
            onClick={() => splitActivePane("row")}
          >
            <Columns2 size={14} strokeWidth={1.8} />
          </button>
          <button
            type="button"
            className="icon-btn icon-btn--sm"
            aria-label={T.terminal.splitDown}
            title={T.terminal.splitDown}
            data-testid="terminal-split-down"
            onClick={() => splitActivePane("column")}
          >
            <Rows2 size={14} strokeWidth={1.8} />
          </button>
          <span className="terminal-actions-divider" aria-hidden="true" />
          <button
            type="button"
            className="icon-btn icon-btn--sm"
            aria-label={T.action.close}
            title={T.action.close}
            data-testid="terminal-close"
            onClick={onClose}
          >
            <X size={14} strokeWidth={1.8} />
          </button>
        </div>
      </header>
      <div className="terminal-stage">
        {tabs.map((tab) => {
          const shown = tab.id === activeTabId;
          const panes = paneLayout(tab.root);
          const closable = paneLeaves(tab.root).length > 1;
          return (
            <div
              key={tab.id}
              id={panelDomId(tab.id)}
              className="terminal-panes"
              role="tabpanel"
              aria-labelledby={tabDomId(tab.id)}
              hidden={!shown}
              data-testid="terminal-tabpanel"
            >
              {panes.map(({ pane, rect }) => (
                <TerminalPane
                  key={pane.id}
                  cwd={projectPath}
                  rect={rect}
                  visible={shown}
                  active={shown && tab.activePaneId === pane.id}
                  closable={closable}
                  onFocus={() => focusPane(tab.id, pane.id)}
                  onClose={() => closePane(tab.id, pane.id)}
                />
              ))}
            </div>
          );
        })}
      </div>
    </section>
  );
}
