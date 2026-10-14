/**
 * 待办 — the sticky bar under both views.
 *
 * The four numbers come from `statsOf` over the rows already loaded, so the
 * bar can never disagree with the board or the list it sits under. It is the
 * one place that states the whole dataset, archived rows included.
 */

import { Lightbulb } from "lucide-react";
import { T } from "../../../i18n";
import type { Stats } from "./todoModel";

const W = T.workbench.todo;

export function TodoStatsBar({ stats }: { stats: Stats }) {
  return (
    <footer className="todo-stats" data-testid="todo-stats">
      <span className="todo-stat">{W.stats(stats.total)}</span>
      <span className="todo-stat-sep" />
      <span className="todo-stat">{W.statsOpen(stats.open)}</span>
      <span className="todo-stat-sep" />
      <span className="todo-stat">{W.statsDone(stats.done)}</span>
      <span className="todo-stat-sep" />
      <span className="todo-stat">{W.statsArchived(stats.archived)}</span>
      <span className="spacer" />
      <span className="todo-stats-hint">
        <Lightbulb size={14} strokeWidth={1.7} />
        {W.statsHint}
      </span>
    </footer>
  );
}
