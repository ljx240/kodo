/**
 * 待办 — one board card.
 *
 * The card is read top to bottom the way the design reads: what it is, what
 * it says, how urgent it is, when it is due and who owns it, and finally how
 * far its sub-items have got. The two controls — the status pill and the
 * complete button — sit at the two ends of that stack so a card can be moved
 * and finished without opening anything.
 *
 * The drag handle is the only pointer-down surface (the rest of the card
 * keeps scrolling), and it is what makes a card movable on a trackpad, a
 * touchscreen and a mouse alike.
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
  return (
    <article
      className={`todo-card${dragging ? " todo-card--dragging" : ""}${
        isDone(todo) ? " todo-card--done" : ""
      }`}
      data-testid={`todo-card-${todo.id}`}
      data-todo-card={todo.id}
    >
      <div className="todo-card-head">
        <TodoDragHandle todo={todo} onDragStart={onDragStart} />
        <p className="todo-card-title">{todo.title}</p>
      </div>

      {todo.notes && <p className="todo-card-notes">{todo.notes}</p>}

      <div className="todo-card-chips">
        <PriorityChip priority={todo.priority} />
        <TagChips tagIds={todo.tagIds} catalogs={catalogs} />
      </div>

      <div className="todo-card-meta">
        <DueChip todo={todo} today={today} />
        {assignee && <Avatar person={assignee} />}
      </div>

      {/* The design's foot: how far the sub-items have got, then the controls
          the design keeps behind ⋯ — exposed here so a card can be re-staged
          and finished without opening the editor. */}
      <div className="todo-card-foot">
        <TodoProgress done={progress.done} total={progress.total} />
        <span className="spacer" />
        <TodoStatusSelect todo={todo} catalogs={catalogs} onApply={onApply} />
        <TodoCompleteButton todo={todo} catalogs={catalogs} onApply={onApply} />
        <TodoEditButton todo={todo} onEdit={onEdit} />
      </div>
    </article>
  );
}
