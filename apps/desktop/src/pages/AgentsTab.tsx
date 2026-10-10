/**
 * 插件 · 智能体 tab (AgentsTab) under PluginsPage: three skills-card blocks —
 * 12 read-only builtins (with the 内置 badge, edit/delete hidden), then
 * project personas when a project is selected, then user-defined personas.
 * Rows never carry `.skill-row` / `.user-skill-row` (the class-split
 * invariant skills tests count against). The page head's 新建智能体
 * button arrives as the `openNew` nonce (first-run guarded).
 *
 * CRUD on user rows bumps `bumpAgentsVersion` so the Composer dropdown
 * (in a different page) refetches — keeps both views in sync without a
 * shared store. The ⭐ toggle is wired through `favoriteAgents` +
 * `onToggleFavorite`; the 试用 button jumps to a new conversation with
 * the persona pre-selected (caller owns `onTryAgent`).
 */
import { Bot, Pencil, Star, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { deleteAgent, listAgents, type AgentView } from "../api";
import { T } from "../i18n";
import { AgentDetailModal } from "./AgentDetailModal";

type ModalState =
  | { mode: "view"; name: string; initial: AgentView }
  | { mode: "edit"; name: string; initial: AgentView }
  | { mode: "new" }
  | null;

export function AgentsTab({
  openNew,
  agentsVersion = 0,
  bumpAgentsVersion = () => undefined,
  projectPath,
  favoriteAgents,
  onToggleFavorite,
}: {
  openNew: number;
  agentsVersion?: number;
  bumpAgentsVersion?: () => void;
  /** Active project path — when present, a project-scope section appears. */
  projectPath?: string;
  /** Favorited persona names — the ⭐ star reads them. */
  favoriteAgents: string[];
  /** Toggle a persona's favorited state (App persists via `useSetting`). */
  onToggleFavorite: (name: string) => void;
}) {
  const [agents, setAgents] = useState<AgentView[]>([]);
  const [modal, setModal] = useState<ModalState>(null);
  /** Armed two-step delete on a row: the agent name awaiting confirmation. */
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const refresh = useCallback(() => {
    void listAgents(projectPath || undefined).then(setAgents);
  }, [projectPath]);

  // Refetch when this tab becomes visible or when an external bump signals
  // a CRUD happened elsewhere (covers both fresh mounts and stale caches).
  useEffect(() => {
    refresh();
  }, [refresh, agentsVersion]);

  // The head button belongs to PluginsPage, which unmounts with the tab. Skip
  // the first run so a nonce left over from an earlier mount never reopens
  // the modal the moment this tab comes back.
  const firstOpen = useRef(true);
  useEffect(() => {
    if (firstOpen.current) {
      firstOpen.current = false;
      return;
    }
    if (openNew > 0) setModal({ mode: "new" });
  }, [openNew]);

  const projectAgents = agents.filter((agent) => agent.source === "project");
  const builtins = agents.filter((agent) => agent.source === "builtin");
  const userAgents = agents.filter((agent) => agent.source === "user");

  const renderStar = (name: string) => {
    const active = favoriteAgents.includes(name);
    return (
      <button
        type="button"
        className={`icon-btn icon-btn--sm agent-row-favorite${active ? " agent-row-favorite--on" : ""}`}
        aria-label={
          active
            ? `${name} 取消常用`
            : `${name} 加入常用`
        }
        aria-pressed={active}
        data-testid={`agent-favorite-${name}`}
        data-favorite={active ? "true" : "false"}
        onClick={(event) => {
          event.stopPropagation();
          onToggleFavorite(name);
        }}
      >
        <Star size={14} strokeWidth={1.8} fill={active ? "currentColor" : "none"} />
      </button>
    );
  };

  return (
    <>
      {projectPath && (
        <section
          className="skills-card"
          aria-label={`${T.agent.card} ${T.agent.groupProject}`}
        >
          <div className="skills-card-head">{T.agent.groupProject}</div>
          {projectAgents.length === 0 ? (
            <div className="skills-empty" data-testid="agents-project-empty">
              {T.agent.projectEmpty}
            </div>
          ) : (
            projectAgents.map((agent) => (
              <div
                className="agent-row agent-row--project"
                key={agent.name}
                data-agent-id={agent.name}
                data-agent-source="project"
                data-testid={`agent-project-${agent.name}`}
              >
                <button
                  type="button"
                  className="agent-open"
                  onClick={() => {
                    setConfirmDelete(null);
                    setModal({ mode: "view", name: agent.name, initial: agent });
                  }}
                >
                  <Bot size={17} strokeWidth={1.7} />
                  <span className="agent-row-name">{agent.name}</span>
                  <span className="agent-row-desc">{agent.description}</span>
                  <span
                    className="agent-row-badge"
                    data-testid="agent-badge-project"
                  >
                    {T.agent.badgeProject}
                  </span>
                </button>
                {renderStar(agent.name)}
              </div>
            ))
          )}
        </section>
      )}

      <section
        className="skills-card"
        aria-label={`${T.agent.card} ${T.agent.groupBuiltin}`}
      >
        <div className="skills-card-head">{T.agent.groupBuiltin}</div>
        {builtins.length === 0 ? (
          <div className="skills-empty" data-testid="agents-builtin-empty">
            {T.agent.empty}
          </div>
        ) : (
          builtins.map((agent) => (
            <div
              className="agent-row agent-row--builtin"
              key={agent.name}
              data-agent-id={agent.name}
              data-agent-source="builtin"
              data-testid={`agent-builtin-${agent.name}`}
            >
              <button
                type="button"
                className="agent-open"
                onClick={() => {
                  setConfirmDelete(null);
                  setModal({ mode: "view", name: agent.name, initial: agent });
                }}
              >
                <Bot size={17} strokeWidth={1.7} />
                <span className="agent-row-name">{agent.name}</span>
                <span className="agent-row-desc">{agent.description}</span>
                <span
                  className="agent-row-badge"
                  data-testid="agent-badge-builtin"
                >
                  {T.agent.badgeBuiltin}
                </span>
              </button>
              {renderStar(agent.name)}
            </div>
          ))
        )}
      </section>

      <section
        className="skills-card"
        aria-label={`${T.agent.card} ${T.agent.groupUser}`}
      >
        <div className="skills-card-head">{T.agent.groupUser}</div>
        {userAgents.length === 0 ? (
          <div className="skills-empty" data-testid="agents-empty">
            {T.agent.empty}
          </div>
        ) : (
          userAgents.map((agent) => (
            <div className="agent-row" key={agent.name} data-agent-id={agent.name}>
              <button
                type="button"
                className="agent-open"
                onClick={() => {
                  setConfirmDelete(null);
                  setModal({ mode: "view", name: agent.name, initial: agent });
                }}
              >
                <Bot size={17} strokeWidth={1.7} />
                <span className="agent-row-name">{agent.name}</span>
                <span className="agent-row-desc">{agent.description}</span>
              </button>
              <span className="skill-row-actions">
                {renderStar(agent.name)}
                <button
                  type="button"
                  className="icon-btn icon-btn--sm"
                  aria-label={`${agent.name} ${T.agent.edit}`}
                  data-testid={`agent-edit-${agent.name}`}
                  onClick={() =>
                    setModal({ mode: "edit", name: agent.name, initial: agent })
                  }
                >
                  <Pencil size={14} strokeWidth={1.8} />
                </button>
                <button
                  type="button"
                  className={`icon-btn icon-btn--sm icon-btn--danger${
                    confirmDelete === agent.name ? " icon-btn--armed" : ""
                  }`}
                  aria-label={
                    confirmDelete === agent.name
                      ? `${agent.name} ${T.skill.deleteConfirm}`
                      : `${agent.name} ${T.agent.delete}`
                  }
                  data-testid={`agent-delete-${agent.name}`}
                  onClick={() => {
                    if (confirmDelete !== agent.name) {
                      setConfirmDelete(agent.name);
                      return;
                    }
                    setConfirmDelete(null);
                    void deleteAgent(agent.name)
                      .then(() => {
                        refresh();
                        bumpAgentsVersion();
                      })
                      .catch(() => undefined);
                  }}
                >
                  {confirmDelete === agent.name ? (
                    <span className="skill-row-confirm">{T.skill.deleteConfirm}</span>
                  ) : (
                    <Trash2 size={14} strokeWidth={1.8} />
                  )}
                </button>
              </span>
            </div>
          ))
        )}
      </section>

      {modal && (
        <AgentDetailModal
          {...modal}
          projectPath={projectPath}
          onClose={() => setModal(null)}
          onChanged={() => {
            refresh();
            bumpAgentsVersion();
          }}
        />
      )}
    </>
  );
}
