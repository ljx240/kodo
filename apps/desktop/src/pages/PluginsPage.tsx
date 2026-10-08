/**
 * 插件 page (COMPONENTS §13d): one sidebar destination hosting the three
 * plugin types as tabs — 技能 (§13b), MCP (§14) and 智能体. The URL is the
 * single source of truth for the active tab (`?type=`, default skills; the
 * old /skills and /mcp paths parse as aliases), so deep links and reloads
 * keep their tab. The page head carries the per-tab subtitle and action so
 * the tab contents stay page-agnostic; the 新建 button drives a tab-owned
 * modal through an incrementing nonce.
 */
import { Plus, Puzzle } from "lucide-react";
import { useState } from "react";
import { T } from "../i18n";
import { navigate, type PluginType } from "../routes";
import { AgentsTab } from "./AgentsTab";
import { McpTab } from "./McpPage";
import { SkillsTab } from "./SkillsPage";

const TABS: PluginType[] = ["skills", "mcp", "agents"];

export function PluginsPage({
  agentMode,
  projectPath,
  type,
  agentsVersion = 0,
  bumpAgentsVersion = () => undefined,
  favoriteAgents,
  onToggleFavorite,
}: {
  agentMode: string;
  projectPath?: string | null;
  type: PluginType;
  /** Persona list version — re-pulled when it changes. */
  agentsVersion?: number;
  /** Bump after AgentsTab CRUD so the Composer dropdown refetches. */
  bumpAgentsVersion?: () => void;
  /** Favorited persona names — the ⭐ toggle reads them from App. */
  favoriteAgents: string[];
  /** Toggle a persona's favorited state; App persists via `useSetting`. */
  onToggleFavorite: (name: string) => void;
}) {
  // Head-action nonces: each tab owns its modal + refresh state, so the head
  // only counts. The tabs skip their first effect run to ignore a stale nonce.
  const [openSkillNew, setOpenSkillNew] = useState(0);
  const [openAgentNew, setOpenAgentNew] = useState(0);

  const subtitle =
    type === "skills"
      ? T.page.skillsSubtitle
      : type === "mcp"
        ? T.page.mcpSubtitle
        : T.page.agentsSubtitle;

  /** Switch tabs by rewriting `?type=` in place — `?inspector=` survives. */
  const selectTab = (next: PluginType) => {
    const params = new URLSearchParams(window.location.search);
    if (next === "skills") params.delete("type");
    else params.set("type", next);
    const query = params.toString();
    navigate(query ? `${window.location.pathname}?${query}` : window.location.pathname);
  };

  return (
    <main className="main">
      <header className="page-head">
        <span className="page-head-mark">
          <Puzzle size={18} strokeWidth={1.7} />
        </span>
        <div className="page-head-text">
          <h1>{T.page.pluginsHead}</h1>
          <p>{subtitle}</p>
        </div>
        <span className="spacer" />
        {type === "skills" && (
          <button
            type="button"
            className="btn"
            data-testid="skill-new"
            onClick={() => setOpenSkillNew((n) => n + 1)}
          >
            <Plus size={14} strokeWidth={1.8} />
            {T.page.skillsNew}
          </button>
        )}
        {type === "agents" && (
          <button
            type="button"
            className="btn"
            data-testid="agent-new"
            onClick={() => setOpenAgentNew((n) => n + 1)}
          >
            <Plus size={14} strokeWidth={1.8} />
            {T.page.agentNew}
          </button>
        )}
      </header>
      <div className="scroll">
        <div className="page-inner page-inner--wide">
          <nav className="page-tabs" role="tablist" aria-label={T.page.pluginsHead}>
            {TABS.map((id) => (
              <button
                key={id}
                type="button"
                role="tab"
                data-testid={`plugins-tab-${id}`}
                className={`page-tab${id === type ? " page-tab--active" : ""}`}
                aria-selected={id === type}
                onClick={() => selectTab(id)}
              >
                {T.page.pluginsTabs[id]}
              </button>
            ))}
          </nav>

          {type === "skills" && (
            <SkillsTab
              agentMode={agentMode}
              projectPath={projectPath}
              openNew={openSkillNew}
            />
          )}
          {type === "mcp" && <McpTab />}
          {type === "agents" && (
            <AgentsTab
              openNew={openAgentNew}
              agentsVersion={agentsVersion}
              bumpAgentsVersion={bumpAgentsVersion}
              projectPath={projectPath ?? undefined}
              favoriteAgents={favoriteAgents}
              onToggleFavorite={onToggleFavorite}
            />
          )}
        </div>
      </div>
    </main>
  );
}
