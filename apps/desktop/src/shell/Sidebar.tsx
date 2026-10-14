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
  LayoutGrid,
  MessageCircle,
  MoreHorizontal,
  PanelLeft,
  Pencil,
  Plus,
  Puzzle,
  Settings,
  Sparkles,
  Timer,
  Trash2,
  X,
} from "lucide-react";
import { useState } from "react";
import { pickFolder } from "../api";
import kodoCodeMark from "../assets/kodo-code-mark.png";
import kodoWorkMark from "../assets/kodo-work-mark.png";
import type { WorkspaceState } from "../data/workspace";
import { T } from "../i18n";
import { hrefTo, navigate, type RouteName } from "../routes";
import { Menu, MenuItem } from "./Menu";

/**
 * The entries above the project tree, in the order the first-level menu keeps:
 * 工作台 → 新建任务 → 插件 → 定时任务. New Task is an action (it starts a
 * conversation), so it never shows the selected-destination treatment; the
 * other three are destinations and do.
 */
const NAV: { name: RouteName; label: string; Icon: typeof Archive; action?: boolean }[] = [
  { name: "workbench", label: T.nav.workbench, Icon: LayoutGrid },
  { name: "conversation", label: T.nav.newTask, Icon: MessageCircle, action: true },
  { name: "plugins", label: T.nav.plugins, Icon: Puzzle },
  { name: "schedule", label: T.nav.schedule, Icon: Timer },
];

/** Conversations shown per group before 「加载全部会话」 takes over. */
const CONVERSATION_FOLD = 5;

type Props = {
  route: RouteName;
  inspectorOpen: boolean;
  activeConversationId: string | null;
  onSelectConversation: (id: string) => void;
  onNewTask: () => void;
  onOpenSettings: () => void;
  onToggleSidebar: () => void;
  /** Creates a new conversation under the given project and selects it. */
  onStartConversation: (projectPath: string) => void;
  onRenameConversation?: (id: string, title: string) => Promise<void> | void;
  onArchiveConversation?: (id: string) => Promise<void> | void;
  /** Deletes the session log outright (two-step confirm in the menu). */
  onDeleteConversation?: (id: string) => Promise<void> | void;
  /** The conversation with a live run — its delete row reads as disabled. */
  runningConversationId?: string | null;
  /** Listing-only removal; the owner uses it to clean up selection state. */
  onRemoveProject?: (path: string) => Promise<void> | void;
  /** Which agent space is showing (the switcher lives in the brand block). */
  agentMode: string;
  onAgentModeChange: (mode: string) => void;
  workspace: WorkspaceState;
};

export function Sidebar({
  route,
  inspectorOpen,
  activeConversationId,
  onSelectConversation,
  onNewTask,
  onOpenSettings,
  onToggleSidebar,
  onStartConversation,
  onRenameConversation,
  onArchiveConversation,
  onDeleteConversation,
  runningConversationId = null,
  onRemoveProject,
  agentMode,
  onAgentModeChange,
  workspace,
}: Props) {
  const { projects, add, create, remove, rename, reorder, reveal } = workspace;

  const [renaming, setRenaming] = useState<string | null>(null);
  const [renamingConversation, setRenamingConversation] = useState<string | null>(null);
  /** Conversation pending a second click on 「确认删除」 (two-step, no modal). */
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  /** Case-insensitive title filter over the conversation tree. */
  const [conversationFilter, setConversationFilter] = useState("");
  const [draft, setDraft] = useState("");
  /** The parent the user picked, held while they type the new folder's name. */
  const [parent, setParent] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dragFrom, setDragFrom] = useState<number | null>(null);
  /** Project pending a second click on 「确认移除」 (two-step, no modal). */
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  /** Groups whose conversation list the user opened past the fold. */
  const [showAllConversations, setShowAllConversations] = useState<Set<string>>(new Set());

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

  /** Commits an armed removal (the menu item arms the first click). */
  const removeProject = (path: string) => {
    setConfirmRemove(null);
    void attempt(() => Promise.resolve(onRemoveProject ? onRemoveProject(path) : remove(path)));
  };

  // Shared by real project rows and the 「未选择项目」 group.
  const conversationRow = (conversation: { id: string; title: string; time: string }) => (
    <div
      key={conversation.id}
      className={`tree-conversation-row${
        conversation.id === activeConversationId ? " tree-conversation-row--active" : ""
      }`}
    >
      {renamingConversation === conversation.id ? (
        <input
          className="tree-input"
          autoFocus
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              const next = draft.trim();
              setRenamingConversation(null);
              if (next) void onRenameConversation?.(conversation.id, next);
            }
            if (event.key === "Escape") setRenamingConversation(null);
          }}
          onBlur={() => setRenamingConversation(null)}
        />
      ) : (
        <>
          <button
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
          {(onRenameConversation || onArchiveConversation || onDeleteConversation) && (
            <Menu
              align="right"
              trigger={({ toggle }) => (
                <button
                  type="button"
                  className="icon-btn icon-btn--sm tree-more tree-conversation-more"
                  aria-label={`${conversation.title} 的操作`}
                  onClick={(event) => {
                    event.stopPropagation();
                    // Opening a menu resets an armed 「确认删除」 — the click-away
                    // path lands here on the next open.
                    setConfirmDelete(null);
                    toggle();
                  }}
                >
                  <MoreHorizontal size={13} strokeWidth={1.9} />
                </button>
              )}
            >
              {(close) => (
                <>
                  {onRenameConversation && (
                    <MenuItem
                      icon={<Pencil size={14} strokeWidth={1.8} />}
                      label={T.conv.rename}
                      onSelect={() => {
                        close();
                        setDraft(conversation.title);
                        setRenamingConversation(conversation.id);
                      }}
                    />
                  )}
                  {onArchiveConversation && (
                    <MenuItem
                      danger
                      icon={<Archive size={14} strokeWidth={1.8} />}
                      label={T.conv.archive}
                      onSelect={() => {
                        close();
                        void onArchiveConversation(conversation.id);
                      }}
                    />
                  )}
                  {onDeleteConversation && (
                    <MenuItem
                      danger
                      icon={<Trash2 size={14} strokeWidth={1.8} />}
                      testId="conv-delete"
                      label={
                        runningConversationId === conversation.id
                          ? T.conv.deleteRunning
                          : confirmDelete === conversation.id
                            ? T.conv.confirmDelete
                            : T.conv.delete
                      }
                      onSelect={() => {
                        if (runningConversationId === conversation.id) {
                          // A live run owns the file — the refusal also stands
                          // in Rust (delete_session checks runs.is_live).
                          close();
                          return;
                        }
                        if (confirmDelete !== conversation.id) {
                          // First click arms; the menu stays open and the
                          // label flips to 「确认删除」.
                          setConfirmDelete(conversation.id);
                          return;
                        }
                        close();
                        setConfirmDelete(null);
                        void attempt(() =>
                          Promise.resolve(onDeleteConversation(conversation.id)),
                        );
                      }}
                    />
                  )}
                </>
              )}
            </Menu>
          )}
        </>
      )}
    </div>
  );

  // Shared list renderer: the first 5 conversations stay visible, the rest
  // wait behind 「加载全部会话」. The open conversation may sit past the fold,
  // so it forces the list open — the active row must never be invisible.
  const conversationList = (project: {
    id: string;
    conversations: { id: string; title: string; time: string }[];
  }) => {
    const { conversations } = project;
    // Filtering runs before the fold: every match shows (no 加载全部会话 in the
    // way), and the open conversation always passes — it must never vanish
    // under a filter the user typed.
    const query = conversationFilter.trim().toLowerCase();
    const matched = query
      ? conversations.filter(
          (conversation) =>
            conversation.title.toLowerCase().includes(query) ||
            conversation.id === activeConversationId,
        )
      : conversations;
    if (query && conversations.length > 0 && matched.length === 0) {
      return <p className="tree-no-match">{T.nav.noMatch}</p>;
    }
    const activeInTail =
      activeConversationId !== null &&
      matched
        .slice(CONVERSATION_FOLD)
        .some((conversation) => conversation.id === activeConversationId);
    const visible =
      query || showAllConversations.has(project.id) || activeInTail
        ? matched
        : matched.slice(0, CONVERSATION_FOLD);
    return (
      <>
        {visible.map(conversationRow)}
        {visible.length < matched.length && (
          <button
            type="button"
            className="tree-load-all"
            data-testid="tree-load-all"
            onClick={() => setShowAllConversations((ids) => new Set(ids).add(project.id))}
          >
            <ChevronDown size={14} strokeWidth={1.9} />
            {T.action.loadAll}
          </button>
        )}
      </>
    );
  };

  const navItem = (name: RouteName, label: string, Icon: typeof Archive, action?: boolean) => (
    <a
      key={name}
      className={`nav-item${!action && route === name ? " nav-item--active" : ""}`}
      href={hrefTo(name, inspectorOpen)}
      onClick={(event) => {
        event.preventDefault();
        if (name === "conversation") onNewTask();
        navigate(hrefTo(name, inspectorOpen));
      }}
    >
      <Icon size={16} strokeWidth={1.8} />
      <span>{label}</span>
    </a>
  );

  return (
    <aside className="sidebar" id="kodo-sidebar">
      {/* The window draws its title bar over this strip, so the space beside the
          traffic lights has to carry the drag region itself. The collapse control
          sits at the top-right of the left menu, clear of the traffic lights. */}
      <div className="sidebar-strip" data-tauri-drag-region>
        <button
          type="button"
          className="icon-btn"
          aria-label={T.nav.toggleSidebar}
          aria-expanded
          aria-controls="kodo-sidebar"
          onClick={onToggleSidebar}
        >
          <PanelLeft size={16} strokeWidth={1.7} />
        </button>
      </div>

      {/* Brand + Work/Code switcher, Codex-style: the brand row is a dropdown
          ("Kodo ˅") whose menu picks the agent space. Switching swaps the whole
          space below it (session list, skills, new-task behavior). */}
      <div className="brand">
        <Menu
          trigger={({ open, toggle }) => (
            <button
              type="button"
              className="brand-switcher"
              data-testid="mode-toggle"
              aria-label={T.nav.modeToggle}
              onClick={toggle}
            >
              <img
                className="brand-mark"
                src={agentMode === "work" ? kodoWorkMark : kodoCodeMark}
                alt=""
                width={36}
                height={36}
              />
              {/* The trigger names the current space — the top-left always says
                  whether Kodo Work or Kodo Code is showing. */}
              <span className="brand-name">
                {agentMode === "work" ? T.nav.brandWork : T.nav.brandCode}
              </span>
              <ChevronDown
                size={14}
                strokeWidth={2}
                className={open ? "brand-caret brand-caret--open" : "brand-caret"}
              />
            </button>
          )}
        >
          {(close) => (
            <>
              <MenuItem
                icon={
                  agentMode === "work" ? (
                    <Check size={14} strokeWidth={2} />
                  ) : (
                    <span className="menu-item-spacer" aria-hidden />
                  )
                }
                label={T.nav.modeWork}
                testId="mode-work"
                selected={agentMode === "work"}
                onSelect={() => {
                  close();
                  onAgentModeChange("work");
                }}
              />
              <MenuItem
                icon={
                  agentMode === "code" ? (
                    <Check size={14} strokeWidth={2} />
                  ) : (
                    <span className="menu-item-spacer" aria-hidden />
                  )
                }
                label={T.nav.modeCode}
                testId="mode-code"
                selected={agentMode === "code"}
                onSelect={() => {
                  close();
                  onAgentModeChange("code");
                }}
              />
            </>
          )}
        </Menu>
      </div>

      <nav className="nav">{NAV.map(({ name, label, Icon, action }) => navItem(name, label, Icon, action))}</nav>

      <div className="projects">
        <div className="section-head">
          <span>{T.nav.projects}</span>
          <Menu
            trigger={({ open, toggle }) => (
              <button
                type="button"
                className="icon-btn"
                aria-label={T.nav.newProject}
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

        {/* Conversation filter — only worth a row once there is something to
            filter (the tree may be projects-only). */}
        {projects.some((project) => project.conversations.length > 0) && (
          <div className="tree-filter-bar">
            <input
              className="tree-filter"
              data-testid="tree-conversation-filter"
              value={conversationFilter}
              placeholder={T.nav.filterConversations}
              aria-label={T.nav.filterConversations}
              onChange={(event) => setConversationFilter(event.target.value)}
            />
          </div>
        )}

        <div className="tree">
          {projects.map((project, index) =>
            project.id === "" ? (
              /* Pseudo group for project-less conversations — a plain group row
                 inside the same tree, never a real project (no menu, no add). */
              <div key="projectless" className="tree-group" data-testid="projectless-group">
                <div className="tree-group-head">{project.name}</div>
                <div className="tree-conversations">{conversationList(project)}</div>
              </div>
            ) : (
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
                  <>
                    {/* Quick new task under this project — one click, no menu. */}
                    <button
                      type="button"
                      className="icon-btn icon-btn--sm tree-project-add"
                      aria-label={T.nav.newTask}
                      onClick={() => onStartConversation(project.id)}
                    >
                      <Plus size={14} strokeWidth={1.9} />
                    </button>
                    <Menu
                      align="right"
                      trigger={({ open, toggle }) => (
                        <button
                          type="button"
                          className="icon-btn icon-btn--sm tree-more"
                          aria-label={`${project.name} 的操作`}
                          aria-expanded={open}
                          onClick={() => {
                            // Opening a menu resets any armed 「确认移除」 — the
                            // click-away path lands here on the next open.
                            setConfirmRemove(null);
                            toggle();
                          }}
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
                            label={confirmRemove === project.id ? "确认移除" : "从列表移除"}
                            onSelect={() => {
                              if (confirmRemove !== project.id) {
                                // First click arms; the menu stays open and the
                                // label flips to 「确认移除」.
                                setConfirmRemove(project.id);
                                return;
                              }
                              close();
                              removeProject(project.id);
                            }}
                          />
                        </>
                      )}
                    </Menu>
                  </>
                )}
              </div>

              {project.expanded && (
                <div className="tree-conversations">{conversationList(project)}</div>
              )}
            </div>
            )
          )}

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
      <div className="sidebar-foot">
        <button type="button" className="nav-item sidebar-settings" onClick={onOpenSettings}>
          <Settings size={16} strokeWidth={1.8} />
          <span>{T.nav.settings}</span>
        </button>
      </div>
    </aside>
  );
}

function lastSegment(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}
