import type { DemoState } from "../data/demoState";
import { T } from "../i18n";

/**
 * Aggregated reasoning view: every thinking / reasoning step from the timeline
 * stacked into a single scrollable column with phase labels. Lives next to
 * AgentTrace in the conversation but is also reused on the trace page when
 * the user wants the longer-form narrative without per-row chips.
 */
export function ThinkingView({
  demoMode,
  rows,
}: {
  demoMode: boolean;
  rows: DemoState["timeline"];
}) {
  if (demoMode) {
    return <p className="empty-note">{T.page.thinkingView.empty}</p>;
  }
  const thinking = rows.filter((row) => row.type === "Thinking");
  if (thinking.length === 0) {
    return <p className="empty-note">{T.page.thinkingView.empty}</p>;
  }
  return (
    <ol className="thinking-list">
      {thinking.map((row) => (
        <li key={`${row.n}-${row.title}`} className="thinking-item">
          <header className="thinking-head">
            <span className="muted mono">#{row.n}</span>
            <span>{row.title}</span>
            <span className="muted mono">{row.time}</span>
          </header>
          <p className="thinking-body">{row.note}</p>
          {row.chip && <code className="code-chip">{row.chip}</code>}
        </li>
      ))}
    </ol>
  );
}