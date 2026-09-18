import {
  Archive,
  Check,
  ChevronDown,
  ChevronRight,
  Clock,
  ExternalLink,
  Folder,
  FolderOpen,
  FolderPlus,
  MessageCircle,
  MoreHorizontal,
  Pencil,
  Plus,
  Search,
  Settings,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { useState } from "react";
import { pickFolder } from "../api";
import type { WorkspaceState } from "../data/workspace";
import { hrefTo, navigate, type RouteName } from "../routes";
import { Menu, MenuItem } from "./Menu";

/** The two destinations that sit above the project tree. */
const NAV: { name: RouteName; label: string; Icon: typeof Archive }[] = [
  { name: "conversation", label: "Conversations", Icon: MessageCircle },
  { name: "archive", label: "Archive", Icon: Archive },
];

type Props = {
  route: RouteName;
  inspectorOpen: boolean;
  activeConversationId: string | null;
  onSelectConversation: (id: string) => void;
  /** Creates a new conversation under the given project and selects it. */
  onStartConversation: (projectPath: string) => void;
  workspace: WorkspaceState;
};

export function Sidebar({ route, inspectorOpen, activeConversationId, onSelectConversation, onStartConversation, workspace }: Props) {
  const { projects, add, create, remove, rename, reorder, reveal } = workspace;

  const [renaming, setRenaming] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  /** The parent the user picked, held while they type the new folder's name. */
  const [parent, setParent] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dragFrom, setDragFrom] = useState<number | null>(null);

  // A rejected write leaves the tree untouched, so the message is the only
  // thing that changes. Reporting it here is what keeps a button that failed
  // from looking like one that worked.
  const attempt = async (action: () => Promise<unknown>): Promise<boolean> => {
    try {
      await action();
      setError(null);
      return true;
    } catch (failure) {
      setError(String(failure));
      return false;
    }
  };

  const addExisting = async () => {
    const picked = await pickFolder();
    if (picked) await attempt(() => add(picked));
  };

  const beginCreate = async () => {
    const picked = await pickFolder();
    if (!picked) return;
    setDraft("");
    setParent(picked);
  };

  const commitCreate = async () => {
    if (!parent || !draft.trim()) return;
    if (await attempt(() => create(parent, draft.trim()))) setParent(null);
  };

  const commitRename = async (path: string) => {
    const name = draft.trim();
    setRenaming(null);
    if (name) await attempt(() => rename(path, name));
  };

  const navItem = (name: RouteName, label: string, Icon: typeof Archive) => (
    <a
      key={name}
      className={`nav-item${route === name ? " nav-item--active" : ""}`}
      href={hrefTo(name, inspectorOpen)}
      onClick={(event) => {
        event.preventDefault();
        navigate(hrefTo(name, inspectorOpen));
      }}
    >
      <Icon size={16} strokeWidth={1.8} />
      <span>{label}</span>
    </a>
  );

  return (
    <aside className="sidebar">
      {/* The window draws its title bar over this strip, so the space beside the
          traffic lights has to carry the drag region itself. */}
      <div className="sidebar-strip" data-tauri-drag-region>
        <button type="button" className="icon-btn" aria-label="Search">
          <Search size={17} strokeWidth={1.7} />
        </button>
      </div>

      <div className="brand">
        <span className="brand-mark">K</span>
        <span className="brand-lines">
          <span className="brand-name">Kodo</span>
          <span className="brand-tagline">Get code done.</span>
        </span>
      </div>

      <button type="button" className="sidebar-search">
        <Search size={15} strokeWidth={1.7} />
        <span>Search...</span>
        <kbd>⌘ K</kbd>
      </button>

      <nav className="nav">{NAV.map(({ name, label, Icon }) => navItem(name, label, Icon))}</nav>

      <div className="projects">
        <div className="section-head">
          <span>Projects</span>
          <Menu
            trigger={({ open, toggle }) => (
              <button
                type="button"
                className="icon-btn"
                aria-label="New project"
                aria-expanded={open}
                onClick={toggle}
              >
                <Plus size={15} strokeWidth={1.8} />
              </button>
            )}
          >
            {(close) => (
              <>
                <MenuItem
                  icon={<FolderOpen size={14} strokeWidth={1.8} />}
                  label="添加已有文件夹…"
                  onSelect={() => {
                    close();
                    void addExisting();
                  }}
                />
                <MenuItem
                  icon={<FolderPlus size={14} strokeWidth={1.8} />}
                  label="新建项目…"
                  onSelect={() => {
                    close();
                    void beginCreate();
                  }}
                />
              </>
            )}
          </Menu>
        </div>

        <div className="tree">
          {projects.map((project, index) => (
            <div
              key={project.id}
              onDragOver={(event) => {
                if (dragFrom !== null) event.preventDefault();
              }}
              onDrop={(event) => {
                event.preventDefault();
                if (dragFrom !== null && dragFrom !== index) void attempt(() => reorder(dragFrom, index));
                setDragFrom(null);
              }}
            >
              <div
                className={`tree-project${project.expanded ? " tree-project--open" : ""}`}
                draggable={renaming !== project.id}
                onDragStart={() => setDragFrom(index)}
                onDragEnd={() => setDragFrom(null)}
              >
                <button type="button" className="tree-project-main" onClick={() => workspace.toggleProject(project.id)}>
                  {project.expanded ? (
                    <ChevronDown size={14} strokeWidth={2} />
                  ) : (
                    <ChevronRight size={14} strokeWidth={2} />
                  )}
                  <Folder size={15} strokeWidth={1.7} />
                  {renaming === project.id ? (
                    <input
                      className="tree-input"
                      autoFocus
                      value={draft}
                      onChange={(event) => setDraft(event.target.value)}
                      onClick={(event) => event.stopPropagation()}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") void commitRename(project.id);
                        if (event.key === "Escape") setRenaming(null);
                      }}
                      onBlur={() => setRenaming(null)}
                    />
                  ) : (
                    <span className="tree-project-name">{project.name}</span>
                  )}
                </button>

                {renaming === project.id ? (
                  <button
                    type="button"
                    className="icon-btn icon-btn--sm"
                    aria-label="确认重命名"
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => void commitRename(project.id)}
                  >
                    <Check size={13} strokeWidth={2.2} />
                  </button>
                ) : (
                  <Menu
                    align="right"
                    trigger={({ open, toggle }) => (
                      <button
                        type="button"
                        className="icon-btn icon-btn--sm tree-more"
                        aria-label={`${project.name} 的操作`}
                        aria-expanded={open}
                        onClick={toggle}
                      >
                        <MoreHorizontal size={14} strokeWidth={1.9} />
                      </button>
                    )}
                  >
                    {(close) => (
                      <>
                        <MenuItem
                          icon={<Sparkles size={14} strokeWidth={1.8} />}
                          label="新建任务"
                          onSelect={() => {
                            close();
                            onStartConversation(project.id);
                          }}
                        />
                        <MenuItem
                          icon={<Pencil size={14} strokeWidth={1.8} />}
                          label="重命名"
                          onSelect={() => {
                            close();
                            setDraft(project.name);
                            setRenaming(project.id);
                          }}
                        />
                        <MenuItem
                          icon={<ExternalLink size={14} strokeWidth={1.8} />}
                          label="在 Finder 中打开"
                          onSelect={() => {
                            close();
                            void attempt(() => reveal(project.id));
                          }}
                        />
                        <MenuItem
                          danger
                          icon={<Trash2 size={14} strokeWidth={1.8} />}
                          label="从列表移除"
                          onSelect={() => {
                            close();
                            void attempt(() => remove(project.id));
                          }}
                        />
                      </>
                    )}
                  </Menu>
                )}
              </div>

              {project.expanded && (
                <div className="tree-conversations">
                  {project.conversations.map((conversation) => (
                    <button
                      key={conversation.id}
                      type="button"
                      className={`tree-conversation${
                        conversation.id === activeConversationId ? " tree-conversation--active" : ""
                      }`}
                      onClick={() => onSelectConversation(conversation.id)}
                    >
                      <Clock size={14} strokeWidth={1.7} />
                      <span className="tree-conversation-title">{conversation.title}</span>
                      <span className="tree-conversation-time">{conversation.time}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          ))}

          {parent !== null && (
            <div className="tree-new">
              <FolderPlus size={14} strokeWidth={1.8} />
              <input
                className="tree-input"
                autoFocus
                placeholder={`在 ${lastSegment(parent)}/ 下新建…`}
                value={draft}
                onChange={(event) => setDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void commitCreate();
                  if (event.key === "Escape") setParent(null);
                }}
              />
              <button
                type="button"
                className="icon-btn icon-btn--sm"
                aria-label="取消"
                onClick={() => setParent(null)}
              >
                <X size={13} strokeWidth={2.2} />
              </button>
            </div>
          )}

          {projects.length === 0 && parent === null && (
            <p className="tree-empty">还没有项目 · 点上面的 + 添加</p>
          )}
        </div>

        {error && <p className="tree-error">{error}</p>}
      </div>

      {/* Kodo has no account, so the foot of the sidebar carries the way into
          Settings instead of a user row. */}
      <div className="sidebar-foot">{navItem("settings", "Settings", Settings)}</div>
    </aside>
  );
}

function lastSegment(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}
