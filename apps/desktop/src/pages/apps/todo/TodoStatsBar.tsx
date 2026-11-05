import { T } from "../../../i18n";
import type { Stats } from "./todoModel";

const W = T.workbench.todo;

export function TodoStatsBar({ stats }: { stats: Stats }) {
  return (
    <div className="todo-stats" data-testid="todo-stats">
      <span className="todo-stat">{W.stats(stats.total)}</span>
      <span className="todo-stat">{W.statsOpen(stats.open)}</span>
      <span className="todo-stat">{W.statsDone(stats.done)}</span>
      <span className="todo-stat">{W.statsArchived(stats.archived)}</span>
    </div>
  );
}
