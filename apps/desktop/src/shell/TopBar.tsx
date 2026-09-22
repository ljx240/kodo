import {
  MessageCircle,
  PanelLeft,
  PanelRight,
  Settings,
  Sparkles,
} from "lucide-react";
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
};

export function TopBar({
  inspectorOpen,
  onToggleInspector,
  sidebarVisible,
  onToggleSidebar,
  onNewTask,
  onOpenSettings,
  route,
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
              aria-label="Toggle sidebar"
              aria-expanded={false}
              aria-controls="kodo-sidebar"
              onClick={onToggleSidebar}
            >
              <PanelLeft size={16} strokeWidth={1.7} />
            </button>

            <div className="topbar-nav-icons" role="navigation" aria-label="Sidebar destinations">
              <button
                type="button"
                className={`icon-btn${route === "conversation" ? " icon-btn--active" : ""}`}
                aria-label="New Task"
                title="New Task"
                onClick={onNewTask}
              >
                <MessageCircle size={16} strokeWidth={1.8} />
              </button>
              <button
                type="button"
                className={`icon-btn${route === "skills" ? " icon-btn--active" : ""}`}
                aria-label="Skills"
                title="Skills"
                onClick={goSkills}
              >
                <Sparkles size={16} strokeWidth={1.8} />
              </button>
              <button
                type="button"
                className="icon-btn"
                aria-label="Settings"
                title="Settings"
                onClick={onOpenSettings}
              >
                <Settings size={16} strokeWidth={1.8} />
              </button>
            </div>
          </>
        )}

      </div>

      <div className="topbar-actions">
        <button
          type="button"
          className={`icon-btn icon-btn--boxed${inspectorOpen ? " icon-btn--active" : ""}`}
          aria-label="Toggle inspector"
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
