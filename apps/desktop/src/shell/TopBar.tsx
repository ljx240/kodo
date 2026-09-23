import {
  GitBranch,
  MessageCircle,
  PanelLeft,
  PanelRight,
  Settings,
  Sparkles,
} from "lucide-react";
import { T } from "../i18n";
import type { RouteName } from "../routes";
import { hrefTo, navigate } from "../routes";

type Props = {
  inspectorOpen: boolean;
  onToggleInspector: () => void;
  sidebarVisible: boolean;
  onToggleSidebar: () => void;
  /** New Task — only needed while the sidebar itself is collapsed. */
  onNewTask: () => void;
  onOpenSettings: () => void;
  route: RouteName;
  /** Project / conversation breadcrumb (LAYOUT §3); omitted on non-conversation routes. */
  crumbProject?: string | null;
  crumbTitle?: string | null;
  /** Git branch of the active project, shown only when detected. */
  branch?: string | null;
  /** Same value as Settings' Default model and the composer trigger (UI_ACCEPTANCE §5b). */
  modelLabel?: string | null;
};

export function TopBar({
  inspectorOpen,
  onToggleInspector,
  sidebarVisible,
  onToggleSidebar,
  onNewTask,
  onOpenSettings,
  route,
  crumbProject,
  crumbTitle,
  branch,
  modelLabel,
}: Props) {
  const goSkills = () => {
    navigate(hrefTo("skills", inspectorOpen));
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
                className={`icon-btn${route === "skills" ? " icon-btn--active" : ""}`}
                aria-label={T.nav.skills}
                title={T.nav.skills}
                onClick={goSkills}
              >
                <Sparkles size={16} strokeWidth={1.8} />
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

        {/* Conversation context: project / title, branch, model — small chips,
            never a large page header (LAYOUT §3). */}
        {route === "conversation" && (crumbProject || crumbTitle) && (
          <nav className="crumbs topbar-crumbs">
            {crumbProject && <span>{crumbProject}</span>}
            {crumbProject && crumbTitle && <span className="crumb-sep">/</span>}
            {crumbTitle && <span className="crumb-current">{crumbTitle}</span>}
          </nav>
        )}
        {route === "conversation" && branch && (
          <span className="chip topbar-chip" title={branch}>
            <GitBranch size={12} strokeWidth={1.8} className="chip-icon" />
            <span>{branch}</span>
          </span>
        )}
        {route === "conversation" && modelLabel && (
          <span className="chip topbar-chip" data-testid="topbar-model-chip" title={modelLabel}>
            {modelLabel}
          </span>
        )}
      </div>

      <div className="topbar-actions">
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
