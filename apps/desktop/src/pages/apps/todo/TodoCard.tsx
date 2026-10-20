/**
 * 待办 — one board card.
 *
 * Linear / Things / Trello inspired layout:
 *   • Linear-style identifier chip (`TODO-F47A`) at the head
 *   • Priority accent stripe on the left edge (3px, color-coded)
 *   • Subtask counter pill (`3/5`) when there are children
 *   • Smart date pill: 今天 / 明天 / N 天后 / 已逾期 N 天 (Things-style)
 *   • Status dot in the head reads the column's `--todo-chip` so all four
 *     lanes share one visual language
 *
 * Two controls — status select + complete + edit — stay visible without
 * opening the editor (Linear / Trello convention). The handle is the only
 * pointer-down surface so the rest of the card keeps scrolling.
 */

import type { TodoDto } from "../../../api";
import {
  Avatar,
  DueChip,
  PriorityChip,
  TagChips,
  TodoCompleteButton,
  TodoDragHandle,
  TodoEditButton,
  TodoProgress,
  TodoStatusSelect,
  type ApplyStatus,
} from "./TodoBits";
import { isDone, progressOf, type TodoRows } from "./todoModel";
import { personOf, type Catalogs } from "./todoTypes";
import { todoIdentifier } from "./todoIdentifier";

export function TodoCard({
  todo,
  rows,
  catalogs,
  today,
  dragging,
  onApply,
  onEdit,
  onDragStart,
}: {
  todo: TodoDto;
  rows: TodoRows;
  catalogs: Catalogs;
  today: string;
  dragging: boolean;
  onApply: ApplyStatus;
  onEdit: (todo: TodoDto) => void;
  onDragStart: (event: React.PointerEvent<HTMLElement>, todo: TodoDto) => void;
}) {
  const progress = progressOf(rows, todo.id);
  const assignee = personOf(catalogs.people, todo.assigneeId);
  const identifier = todoIdentifier(todo.id);
  // Empty rows (chips / meta) collapse to zero height instead of reserving
  // their min-height — that's how a bare title card on the board stays
  // ~50px tall instead of ~110px.
  const hasChips = todo.tagIds.length > 0 || Boolean(todo.priority);
  const hasMeta = Boolean(todo.dueDate) || Boolean(assignee);
  // Linear-style priority accent on the left edge of the card. Width 0
  // when priority is `none` / absent so the chip-free cards don't carry a
  // visible stripe (which would feel like a bug, not design).
  const hasPriorityStripe =
    todo.priority === "high" || todo.priority === "normal" || todo.priority === "low";
  return (
    <article
      className={`todo-card${dragging ? " todo-card--dragging" : ""}${
        isDone(todo) ? " todo-card--done" : ""
      }${hasPriorityStripe ? ` todo-card--priority-${todo.priority}` : ""}`}
      data-testid={`todo-card-${todo.id}`}
      data-todo-card={todo.id}
      data-priority={todo.priority}
    >
      <div className="todo-card-head">
        <TodoDragHandle todo={todo} onDragStart={onDragStart} />
        <span className="todo-card-id">{identifier}</span>
        <p className="todo-card-title">{todo.title}</p>
      </div>

      {todo.notes && <p className="todo-card-notes">{todo.notes}</p>}

      {hasChips && (
        <div className="todo-card-chips">
          <PriorityChip priority={todo.priority} />
          <TagChips tagIds={todo.tagIds} catalogs={catalogs} />
        </div>
      )}

      {hasMeta && (
        <div className="todo-card-meta">
          <DueChip todo={todo} today={today} />
          {assignee && <Avatar person={assignee} />}
        </div>
      )}

      {/* The design's foot: subtask progress + controls the design keeps
          behind ⋯ — exposed here so a card can be re-staged and finished
          without opening the editor. */}
      <div className="todo-card-foot">
        <TodoProgress done={progress.done} total={progress.total} />
        <div className="todo-card-actions">
          <TodoStatusSelect todo={todo} catalogs={catalogs} onApply={onApply} />
          <TodoCompleteButton todo={todo} catalogs={catalogs} onApply={onApply} />
          <TodoEditButton todo={todo} onEdit={onEdit} />
        </div>
      </div>
    </article>
  );
}
