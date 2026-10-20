import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
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
  setSplitRatio,
  splitHandles,
  splitPane,
  type SplitDirection,
  type SplitHandle,
  type TerminalTab,
} from "./paneTree";

const PANEL_HEIGHT_KEY = "kodo.terminal.height";
const MIN_PANEL_HEIGHT = 160;
const MIN_PANE_PX = 96;
const SPLIT_STEP = 0.02;

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

function readPanelHeight(): number | null {
  const raw = window.localStorage.getItem(PANEL_HEIGHT_KEY);
  if (!raw) return null;
  const value = Number(raw);
  return Number.isFinite(value) && value > 0 ? Math.max(MIN_PANEL_HEIGHT, value) : null;
}

function clampPanelHeight(height: number): number {
  return Math.min(window.innerHeight * 0.72, Math.max(MIN_PANEL_HEIGHT, height));
}

export function TerminalPanel({
  projectPath,
  onClose,
}: {
  projectPath: string;
  onClose: () => void;
}) {
  const nextTabNumber = useRef(2);
  const panelRef = useRef<HTMLElement | null>(null);
  const stageRef = useRef<HTMLDivElement | null>(null);
  const [tabs, setTabs] = useState<TerminalTab[]>(() => [createTab(1)]);
  const [activeTabId, setActiveTabId] = useState(() => tabs[0].id);
  const [panelHeight, setPanelHeight] = useState<number | null>(readPanelHeight);

  useEffect(() => {
    document
      .getElementById(tabDomId(activeTabId))
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeTabId]);

  const setTabSplitRatio = (tabId: string, splitId: string, ratio: number) => {
    setTabs((current) =>
      current.map((item) =>
        item.id === tabId
          ? { ...item, root: setSplitRatio(item.root, splitId, ratio) }
          : item,
      ),
    );
  };

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

  const persistHeight = (height: number) => {
    try {
      window.localStorage.setItem(PANEL_HEIGHT_KEY, String(Math.round(height)));
    } catch {
      // Storage can be unavailable in a locked-down webview; resizing still works.
    }
  };

  const beginPanelResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const panel = panelRef.current;
    if (!panel) return;
    event.preventDefault();
    const startY = event.clientY;
    const startHeight = panel.getBoundingClientRect().height;
    let latest = startHeight;

    const move = (moveEvent: PointerEvent) => {
      latest = clampPanelHeight(startHeight + startY - moveEvent.clientY);
      setPanelHeight(latest);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      document.body.classList.remove("terminal-dimensioning", "terminal-dimensioning--row");
      persistHeight(latest);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    document.body.classList.add("terminal-dimensioning", "terminal-dimensioning--row");
  };

  const handlePanelResizeKey = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    const panel = panelRef.current;
    if (!panel) return;
    const next = clampPanelHeight(
      panel.getBoundingClientRect().height + (event.key === "ArrowUp" ? 24 : -24),
    );
    setPanelHeight(next);
    persistHeight(next);
  };

  const beginSplitResize = (
    event: ReactPointerEvent<HTMLDivElement>,
    tabId: string,
    handle: SplitHandle,
  ) => {
    if (event.button !== 0) return;
    const stage = stageRef.current;
    if (!stage) return;
    event.preventDefault();
    event.stopPropagation();

    const bounds = stage.getBoundingClientRect();
    const boxLeft = bounds.left + (bounds.width * handle.rect.left) / 100;
    const boxTop = bounds.top + (bounds.height * handle.rect.top) / 100;
    const boxWidth = (bounds.width * handle.rect.width) / 100;
    const boxHeight = (bounds.height * handle.rect.height) / 100;
    const row = handle.split.direction === "row";
    const minRatio = Math.min(0.45, MIN_PANE_PX / (row ? boxWidth : boxHeight));
    const cursor = row ? "terminal-dimensioning--col" : "terminal-dimensioning--row";

    const move = (moveEvent: PointerEvent) => {
      const raw = row
        ? (moveEvent.clientX - boxLeft) / boxWidth
        : (moveEvent.clientY - boxTop) / boxHeight;
      const ratio = Math.min(1 - minRatio, Math.max(minRatio, raw));
      setTabs((current) =>
        current.map((item) =>
          item.id === tabId
            ? { ...item, root: setSplitRatio(item.root, handle.split.id, ratio) }
            : item,
        ),
      );
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      document.body.classList.remove("terminal-dimensioning", cursor);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    document.body.classList.add("terminal-dimensioning", cursor);
  };

  const handleSplitKey = (
    event: KeyboardEvent<HTMLDivElement>,
    tabId: string,
    handle: SplitHandle,
  ) => {
    const row = handle.split.direction === "row";
    const grow = row ? "ArrowRight" : "ArrowDown";
    const shrink = row ? "ArrowLeft" : "ArrowUp";
    if (event.key !== grow && event.key !== shrink) return;
    event.preventDefault();
    const step = event.shiftKey ? 0.1 : SPLIT_STEP;
    const ratio = handle.split.ratio + (event.key === grow ? step : -step);
    setTabSplitRatio(tabId, handle.split.id, ratio);
  };

  return (
    <section
      ref={panelRef}
      className="terminal-panel"
      aria-label={T.terminal.title}
      data-testid="terminal-panel"
      style={panelHeight ? { height: `min(${Math.round(panelHeight)}px, 72vh)` } : undefined}
    >
      <div
        className="terminal-resize"
        role="separator"
        aria-orientation="horizontal"
        aria-label={T.terminal.resizePanel}
        title={T.terminal.resizePanel}
        tabIndex={0}
        onPointerDown={beginPanelResize}
        onKeyDown={handlePanelResizeKey}
      />
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
      <div className="terminal-stage" ref={stageRef}>
        {tabs.map((tab) => {
          const shown = tab.id === activeTabId;
          const panes = paneLayout(tab.root);
          const handles = splitHandles(tab.root);
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
              {handles.map((handle) => {
                const row = handle.split.direction === "row";
                const boundary = row
                  ? handle.rect.left + handle.rect.width * handle.split.ratio
                  : handle.rect.top + handle.rect.height * handle.split.ratio;
                return (
                  <div
                    key={handle.split.id}
                    className={`terminal-divider terminal-divider--${handle.split.direction}`}
                    style={
                      row
                        ? {
                            left: `${boundary}%`,
                            top: `${handle.rect.top}%`,
                            height: `${handle.rect.height}%`,
                          }
                        : {
                            top: `${boundary}%`,
                            left: `${handle.rect.left}%`,
                            width: `${handle.rect.width}%`,
                          }
                    }
                    role="separator"
                    aria-orientation={row ? "vertical" : "horizontal"}
                    aria-label={T.terminal.resizePane}
                    title={T.terminal.resizePane}
                    tabIndex={0}
                    data-testid="terminal-divider"
                    onPointerDown={(event) => beginSplitResize(event, tab.id, handle)}
                    onDoubleClick={() => setTabSplitRatio(tab.id, handle.split.id, 0.5)}
                    onKeyDown={(event) => handleSplitKey(event, tab.id, handle)}
                  />
                );
              })}
            </div>
          );
        })}
      </div>
    </section>
  );
}
