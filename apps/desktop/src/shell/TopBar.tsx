import { useState } from "react";
import {
  Archive,
  Brain,
  Check,
  ChevronDown,
  Folder,
  FolderPlus,
  GitBranch,
  MessageCircle,
  MoreVertical,
  PanelLeft,
  PanelRight,
  Pencil,
  Puzzle,
  Settings,
  Terminal,
  X,
} from "lucide-react";
import { T } from "../i18n";
import type { RouteName } from "../routes";
import { hrefTo, navigate } from "../routes";
import { Menu, MenuItem } from "./Menu";
import type { Project } from "../data/types";

/**
 * Conversation-level controls, owned by ConversationPage — it keeps the
 * rename/archive handlers and their failure banners; the top bar only hosts
 * the buttons, since 2026-09-30 the content area has no title header.
 */
export type ConvActions = {
  rename: (title: string) => void;
  archive: () => void;
};

type Props = {
  inspectorOpen: boolean;
  onToggleInspector: () => void;
  sidebarVisible: boolean;
  onToggleSidebar: () => void;
  /** New Task — only needed while the sidebar itself is collapsed. */
  onNewTask: () => void;
  onOpenSettings: () => void;
  /** Bottom terminal toggle (conversation route only). */
  terminalOpen?: boolean;
  onToggleTerminal?: () => void;
  route: RouteName;
  /** Project / conversation breadcrumb (LAYOUT §3); omitted on non-conversation routes. */
  crumbProject?: string | null;
  crumbTitle?: string | null;
  /** Git branch of the active project, shown only when detected. */
  branch?: string | null;
  /** A new task can choose its project without opening an existing session. */
  projectOptions?: Project[];
  projectPath?: string;
  onSelectProject?: (path: string | null) => void;
  onAddProject?: () => Promise<void>;
  /** Present only while a live conversation is open — gates rename/archive. */
  convActions?: ConvActions | null;
  /** Click handler for the Memory chip — opens the Memory tab in the
   *  Inspector. The TopBar doesn't own navigation; App does. */
  onOpenMemoryTab?: () => void;
  /** Live memory recall count for the active turn. `undefined` means the
   *  engine is off (no `memoryRecalled` event has been emitted yet);
   *  `0` means the engine is on but nothing matched; `n > 0` shows the
   *  number of memories the agent actually saw. */
  memoryRecalledCount?: number;
};

export function TopBar({
  inspectorOpen,
  onToggleInspector,
  sidebarVisible,
  onToggleSidebar,
  onNewTask,
  onOpenSettings,
  terminalOpen = false,
  onToggleTerminal,
  route,
  crumbProject,
  crumbTitle,
  branch,
  projectOptions = [],
  projectPath = "",
  onSelectProject,
  onAddProject,
  convActions = null,
  onOpenMemoryTab,
  memoryRecalledCount,
}: Props) {
  // Inline title edit in the crumb — the pencil and the ⋯ menu below are its
  // triggers (the content area lost its 64px title header on 2026-09-30).
  const [renaming, setRenaming] = useState(false);
  const [renameDraft, setRenameDraft] = useState("");
  const [projectError, setProjectError] = useState<string | null>(null);

  const startRename = () => {
    setRenameDraft(crumbTitle ?? "");
    setRenaming(true);
  };
  const commitRename = () => {
    const next = renameDraft.trim();
    if (!next) return;
    convActions?.rename(next);
    setRenaming(false);
  };

  const goPlugins = () => {
    navigate(hrefTo("plugins", inspectorOpen));
  };

  return (
    <header className="topbar">
      <div className="topbar-left">
        {/* Expanded: the collapse control lives at the top of the sidebar.
            Collapsed: only the expand control + destinations sit here, clear of
            the traffic lights via .app--sidebar-hidden .topbar padding. */}
        {!sidebarVisible && (
          <>
            <button
              type="button"
              className="icon-btn icon-btn--active"
              aria-label={T.nav.toggleSidebar}
              aria-expanded={false}
              aria-controls="kodo-sidebar"
              onClick={onToggleSidebar}
            >
              <PanelLeft size={16} strokeWidth={1.7} />
            </button>

            <div className="topbar-nav-icons" role="navigation" aria-label={T.nav.sidebarDestinations}>
              <button
                type="button"
                className="icon-btn"
                aria-label={T.nav.newTask}
                title={T.nav.newTask}
                onClick={onNewTask}
              >
                <MessageCircle size={16} strokeWidth={1.8} />
              </button>
              <button
                type="button"
                className={`icon-btn${route === "plugins" ? " icon-btn--active" : ""}`}
                aria-label={T.nav.plugins}
                title={T.nav.plugins}
                onClick={goPlugins}
              >
                <Puzzle size={16} strokeWidth={1.8} />
              </button>
              <button
                type="button"
                className="icon-btn"
                aria-label={T.nav.settings}
                title={T.nav.settings}
                onClick={onOpenSettings}
              >
                <Settings size={16} strokeWidth={1.8} />
              </button>
            </div>
          </>
        )}

        {/* Conversation context: project / title and branch — small chips,
            never a large page header (LAYOUT §3). The title is the single
            title surface since 2026-09-30 and edits in place. The model lives
            on the composer only; the top bar does not repeat it (2026-09-23). */}
        {route === "conversation" && (crumbProject || crumbTitle) && (
          <nav className="crumbs topbar-crumbs">
            {onSelectProject ? (
              <div className="topbar-project-context" data-testid="topbar-project-context">
                <Menu
                  trigger={({ toggle }) => (
                    <button
                      type="button"
                      className="topbar-project-picker"
                      data-testid="topbar-project-picker"
                      aria-label={T.nav.selectProject}
                      title={projectPath || T.nav.selectProject}
                      onClick={toggle}
                    >
                      <span data-testid="topbar-project-name">{crumbProject || T.nav.noProject}</span>
                      <ChevronDown size={12} strokeWidth={1.8} />
                    </button>
                  )}
                >
                  {(close) => (
                    <>
                      {projectOptions.map((project) => (
                        <MenuItem
                          key={project.id}
                          icon={project.path === projectPath ? <Check size={14} /> : <Folder size={14} />}
                          label={project.name}
                          selected={project.path === projectPath}
                          onSelect={() => {
                            close();
                            setProjectError(null);
                            onSelectProject(project.path ?? project.id);
                          }}
                        />
                      ))}
                      {onAddProject && (
                        <MenuItem
                          icon={<FolderPlus size={14} />}
                          label={T.nav.addProject}
                          onSelect={() => {
                            close();
                            setProjectError(null);
                            void onAddProject().catch((error) => setProjectError(String(error)));
                          }}
                        />
                      )}
                      {projectPath && (
                        <MenuItem
                          icon={<X size={14} />}
                          label={T.nav.clearProject}
                          onSelect={() => {
                            close();
                            setProjectError(null);
                            onSelectProject(null);
                          }}
                        />
                      )}
                    </>
                  )}
                </Menu>
                {projectError && <p className="topbar-project-error" role="alert">{projectError}</p>}
              </div>
            ) : crumbProject && <span data-testid="topbar-project-name" title={projectPath}>{crumbProject}</span>}
            {crumbProject && crumbTitle && <span className="crumb-sep">/</span>}
            {renaming ? (
              <input
                className="conv-title-input"
                aria-label={T.conv.rename}
                value={renameDraft}
                autoFocus
                onChange={(event) => setRenameDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && renameDraft.trim()) commitRename();
                  if (event.key === "Escape") setRenaming(false);
                }}
                onBlur={() => setRenaming(false)}
              />
            ) : (
              crumbTitle && <span className="crumb-current">{crumbTitle}</span>
            )}
          </nav>
        )}
        {route === "conversation" && branch && (
          <span className="chip topbar-chip" title={branch} data-testid="topbar-branch">
            <GitBranch size={12} strokeWidth={1.8} className="chip-icon" />
            <span>{branch}</span>
          </span>
        )}
        {route === "conversation" && onOpenMemoryTab && (
          <button
            type="button"
            className="chip topbar-chip topbar-chip--button"
            title={
              memoryRecalledCount === undefined
                ? T.memory.section
                : memoryRecalledCount > 0
                  ? T.memory.recalled(memoryRecalledCount)
                  : T.memory.empty
            }
            data-testid="topbar-memory"
            data-memory-count={memoryRecalledCount ?? "off"}
            onClick={onOpenMemoryTab}
          >
            <Brain size={12} strokeWidth={1.8} className="chip-icon" />
            <span>
              {memoryRecalledCount === undefined
                ? T.memory.section
                : memoryRecalledCount > 0
                  ? T.memory.recalled(memoryRecalledCount)
                  : T.memory.empty}
            </span>
          </button>
        )}
      </div>

      <div className="topbar-actions">
        {route === "conversation" && convActions && (
          <>
            <button
              type="button"
              className="icon-btn"
              aria-label={T.conv.rename}
              title={T.conv.rename}
              onClick={startRename}
            >
              <Pencil size={15} strokeWidth={1.8} />
            </button>
            <Menu
              align="right"
              trigger={({ toggle }) => (
                <button
                  type="button"
                  className="icon-btn"
                  aria-label={T.conv.actions}
                  title={T.conv.actions}
                  onClick={toggle}
                >
                  <MoreVertical size={16} strokeWidth={1.8} />
                </button>
              )}
            >
              {(close) => (
                <>
                  <MenuItem
                    icon={<Pencil size={14} strokeWidth={1.8} />}
                    label={T.conv.rename}
                    onSelect={() => {
                      close();
                      startRename();
                    }}
                  />
                  <MenuItem
                    icon={<Archive size={14} strokeWidth={1.8} />}
                    label={T.conv.archive}
                    danger
                    onSelect={() => {
                      close();
                      convActions.archive();
                    }}
                  />
                </>
              )}
            </Menu>
          </>
        )}
        {route === "conversation" && onToggleTerminal && (
          <button
            type="button"
            className={`icon-btn icon-btn--boxed${terminalOpen ? " icon-btn--active" : ""}`}
            aria-label={T.nav.terminal}
            title={T.nav.terminal}
            aria-pressed={terminalOpen}
            data-testid="topbar-terminal"
            onClick={onToggleTerminal}
          >
            <Terminal size={16} strokeWidth={1.7} />
          </button>
        )}
        <button
          type="button"
          className={`icon-btn icon-btn--boxed${inspectorOpen ? " icon-btn--active" : ""}`}
          aria-label={T.nav.toggleInspector}
          aria-expanded={inspectorOpen}
          aria-controls={inspectorOpen ? "kodo-inspector-panel" : undefined}
          onClick={onToggleInspector}
        >
          <PanelRight size={16} strokeWidth={1.7} />
        </button>
      </div>
    </header>
  );
}
