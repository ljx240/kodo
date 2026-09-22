import { ChevronDown, Folder, GitBranch, PanelLeft, PanelRight, Plus, Sparkles } from "lucide-react";
import type { ProviderConfig } from "../data/providers";
import { templateById } from "../data/providers";
import type { Project } from "../data/types";
import { Menu, MenuItem } from "./Menu";

type Props = {
  projects: Project[];
  activeProjectId: string | null;
  onSelectProject: (id: string) => void;
  /** The project the conversation belongs to. */
  projectName: string;
  /** Its branch, or null when it is not a checkout. */
  branch: string | null;
  /** Active provider config. */
  provider: ProviderConfig | null;
  onNewChat: (projectPath?: string) => void;
  inspectorOpen: boolean;
  onToggleInspector: () => void;
  sidebarVisible: boolean;
  onToggleSidebar: () => void;
};

export function TopBar({
  projects,
  activeProjectId,
  onSelectProject,
  projectName,
  branch,
  provider,
  onNewChat,
  inspectorOpen,
  onToggleInspector,
  sidebarVisible,
  onToggleSidebar,
}: Props) {
  const tmpl = provider ? templateById(provider.template) : null;
  const modelLabel = provider?.model ?? "No model";

  return (
    <header className="topbar">
      <div className="topbar-left">
        <button
          type="button"
          className={`icon-btn${sidebarVisible ? "" : " icon-btn--active"}`}
          aria-label="Toggle sidebar"
          aria-expanded={sidebarVisible}
          aria-controls="kodo-sidebar"
          onClick={onToggleSidebar}
        >
          <PanelLeft size={16} strokeWidth={1.7} />
        </button>

        <Menu
          trigger={({ open, toggle }) => (
            <button type="button" className="chip" aria-expanded={open} onClick={toggle}>
              <Folder size={14} strokeWidth={1.7} />
              <span>{projectName}</span>
              <ChevronDown size={13} strokeWidth={1.9} />
            </button>
          )}
        >
          {(close) =>
            projects.map((project) => (
              <MenuItem
                key={project.id}
                icon={<Folder size={14} strokeWidth={1.8} />}
                label={project.name}
                onSelect={() => {
                  close();
                  onSelectProject(project.id);
                }}
              />
            ))
          }
        </Menu>

        {/* Not a picker: the branch is whatever the checkout says it is, and
            Kodo has no way to change it without running git. */}
        <span className="chip chip--static">
          <GitBranch size={14} strokeWidth={1.7} />
          <span>{branch ?? "无分支"}</span>
        </span>
      </div>

      <div className="topbar-actions">
        {/* Active provider / model chip — read-only display */}
        <span className="chip chip--static">
          {tmpl && <Sparkles size={14} strokeWidth={1.7} className="chip-icon" />}
          <span>{provider?.name ? `${provider.name} / ` : ""}{modelLabel}</span>
        </span>

        {activeProjectId ? (
          <button type="button" className="btn" onClick={() => onNewChat()}>
            <Plus size={15} strokeWidth={1.9} />
            <span>New chat</span>
          </button>
        ) : (
          <Menu
            trigger={({ open, toggle }) => (
              <button type="button" className="btn" aria-expanded={open} onClick={toggle}>
                <Plus size={15} strokeWidth={1.9} />
                <span>New chat</span>
              </button>
            )}
          >
            {(close) =>
              projects.map((project) => (
                <MenuItem
                  key={project.id}
                  icon={<Folder size={14} strokeWidth={1.8} />}
                  label={project.name}
                  onSelect={() => {
                    close();
                    onSelectProject(project.id);
                    onNewChat(project.id);
                  }}
                />
              ))
            }
          </Menu>
        )}

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
