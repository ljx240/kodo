import { useState } from "react";
import {
  Archive,
  Folder,
  GitBranch,
  MessageCircle,
  MoreVertical,
  PanelLeft,
  PanelRight,
  Pencil,
  Puzzle,
  Settings,
} from "lucide-react";
import { T } from "../i18n";
import type { RouteName } from "../routes";
import { hrefTo, navigate } from "../routes";
import { Menu, MenuItem } from "./Menu";

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
  /** Project file browser entry (conversation route only). */
  onOpenFiles?: () => void;
  route: RouteName;
  /** Project / conversation breadcrumb (LAYOUT §3); omitted on non-conversation routes. */
  crumbProject?: string | null;
  crumbTitle?: string | null;
  /** Git branch of the active project, shown only when detected. */
  branch?: string | null;
  /** Present only while a live conversation is open — gates rename/archive. */
  convActions?: ConvActions | null;
};

export function TopBar({
  inspectorOpen,
  onToggleInspector,
  sidebarVisible,
  onToggleSidebar,
  onNewTask,
  onOpenSettings,
  onOpenFiles,
  route,
  crumbProject,
  crumbTitle,
  branch,
  convActions = null,
}: Props) {
  // Inline title edit in the crumb — the pencil and the ⋯ menu below are its
  // triggers (the content area lost its 64px title header on 2026-09-30).
  const [renaming, setRenaming] = useState(false);
  const [renameDraft, setRenameDraft] = useState("");

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
            {crumbProject && <span>{crumbProject}</span>}
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
          <span className="chip topbar-chip" title={branch}>
            <GitBranch size={12} strokeWidth={1.8} className="chip-icon" />
            <span>{branch}</span>
          </span>
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
        {route === "conversation" && onOpenFiles && (
          <button
            type="button"
            className="icon-btn icon-btn--boxed"
            aria-label={T.nav.viewFiles}
            title={T.nav.viewFiles}
            data-testid="topbar-files"
            onClick={onOpenFiles}
          >
            <Folder size={16} strokeWidth={1.7} />
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
