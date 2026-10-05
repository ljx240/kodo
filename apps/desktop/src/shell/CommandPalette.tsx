import { useEffect, useMemo, useRef, useState } from "react";
import { T } from "../i18n";

export type PaletteAction = {
  id: string;
  label: string;
  run: () => void;
};

export type PaletteSession = {
  id: string;
  title: string;
  project: string;
  run: () => void;
};

/** Persona pick — App builds these rows so the palette can run a chain
 *  swap on selection (creates a new conversation with the persona set). */
export type PaletteAgent = {
  name: string;
  description: string;
  source: "builtin" | "user" | "project";
  run: () => void;
};

type Props = {
  actions: PaletteAction[];
  /** Current-space conversations; empty in demo (commands only). */
  sessions: PaletteSession[];
  /** Persona picks (one row per agent); empty hides the section. */
  agents?: PaletteAgent[];
  onClose: () => void;
};

/**
 * ⌘K quick switcher: one overlay, one input — commands first, then agents,
 * then sessions, all filtered by a case-insensitive substring. Arrow keys
 * move the active row, Enter runs it, Escape closes. Geometry follows the
 * settings modal (backdrop + panel), scaled down to a palette.
 */
export function CommandPalette({ actions, sessions, agents = [], onClose }: Props) {
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.focus();
  }, []);

  const filteredActions = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return actions;
    return actions.filter((action) => action.label.toLowerCase().includes(needle));
  }, [actions, query]);

  const filteredAgents = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return agents;
    return agents.filter(
      (agent) =>
        agent.name.toLowerCase().includes(needle) ||
        agent.description.toLowerCase().includes(needle),
    );
  }, [agents, query]);

  const filteredSessions = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return sessions;
    return sessions.filter(
      (session) =>
        session.title.toLowerCase().includes(needle) ||
        session.project.toLowerCase().includes(needle),
    );
  }, [sessions, query]);

  const rows = useMemo(
    () => [
      ...filteredActions.map((action) => ({ kind: "action" as const, item: action })),
      ...filteredAgents.map((agent) => ({ kind: "agent" as const, item: agent })),
      ...filteredSessions.map((session) => ({ kind: "session" as const, item: session })),
    ],
    [filteredActions, filteredAgents, filteredSessions],
  );

  // Clamp first so a shrinking result set never leaves the cursor in the void.
  const active = rows.length === 0 ? 0 : Math.min(cursor, rows.length - 1);

  const run = (index: number) => {
    const row = rows[index];
    if (!row) return;
    row.item.run();
    onClose();
  };

  const agentOffset = filteredActions.length;
  const sessionOffset = agentOffset + filteredAgents.length;

  return (
    <div
      className="palette-modal"
      data-testid="palette-backdrop"
      onMouseDown={onClose}
    >
      <div
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label={T.palette.title}
        data-testid="command-palette"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <input
          ref={input}
          className="palette-input"
          data-testid="palette-input"
          value={query}
          placeholder={T.palette.placeholder}
          aria-label={T.palette.title}
          onChange={(event) => {
            setQuery(event.target.value);
            setCursor(0);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              onClose();
            }
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setCursor((current) => (rows.length ? (current + 1) % rows.length : 0));
            }
            if (event.key === "ArrowUp") {
              event.preventDefault();
              setCursor((current) =>
                rows.length ? (current - 1 + rows.length) % rows.length : 0,
              );
            }
            if (event.key === "Enter") {
              event.preventDefault();
              run(active);
            }
          }}
        />
        <div className="palette-list" data-testid="palette-list">
          {filteredActions.length > 0 && (
            <div className="palette-group">{T.palette.actions}</div>
          )}
          {filteredActions.map((action, index) => (
            <button
              key={action.id}
              type="button"
              id={`palette-item-${index}`}
              className={`palette-item${index === active ? " palette-item--active" : ""}`}
              data-testid={`palette-item-${action.id}`}
              onMouseEnter={() => setCursor(index)}
              onClick={() => run(index)}
            >
              <span className="palette-item-label">{action.label}</span>
            </button>
          ))}
          {filteredAgents.length > 0 && (
            <div className="palette-group">{T.palette.agents}</div>
          )}
          {filteredAgents.map((agent, index) => (
            <button
              key={`agent:${agent.name}`}
              type="button"
              id={`palette-item-${agentOffset + index}`}
              className={`palette-item${agentOffset + index === active ? " palette-item--active" : ""}`}
              data-testid={`palette-item-agent-${agent.name}`}
              onMouseEnter={() => setCursor(agentOffset + index)}
              onClick={() => run(agentOffset + index)}
            >
              <span className="palette-item-label">{agent.name}</span>
              <span className="palette-item-project">{agent.description}</span>
            </button>
          ))}
          {filteredSessions.length > 0 && (
            <div className="palette-group">{T.palette.sessions}</div>
          )}
          {filteredSessions.map((session, index) => (
            <button
              key={session.id}
              type="button"
              id={`palette-item-${sessionOffset + index}`}
              className={`palette-item${sessionOffset + index === active ? " palette-item--active" : ""}`}
              data-testid={`palette-item-${session.id}`}
              onMouseEnter={() => setCursor(sessionOffset + index)}
              onClick={() => run(sessionOffset + index)}
            >
              <span className="palette-item-label">{session.title}</span>
              <span className="palette-item-project">{session.project}</span>
            </button>
          ))}
          {rows.length === 0 && <p className="palette-empty">{T.palette.empty}</p>}
        </div>
        <div className="palette-hint">{T.palette.hint}</div>
      </div>
    </div>
  );
}
