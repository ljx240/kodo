/** A board card with quick actions and a dedicated drag handle. */
import { Folder } from "lucide-react";
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
import { personOf, SOURCE_LABEL, type Catalogs } from "./todoTypes";
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
  return (
    <article
      className={`todo-card${dragging ? " todo-card--dragging" : ""}${
        isDone(todo) ? " todo-card--done" : ""
      }`}
      data-testid={`todo-card-${todo.id}`}
      data-todo-card={todo.id}
      data-priority={todo.priority}
    >
      <div className="todo-card-head">
        <TodoDragHandle todo={todo} onDragStart={onDragStart} />
        <span className="todo-card-id">{identifier}</span>
        <span className="todo-card-source">{SOURCE_LABEL[todo.source]}</span>
      </div>
      <button type="button" className="todo-card-title" onClick={() => onEdit(todo)}>
        {todo.title}
      </button>

      {todo.notes && <p className="todo-card-notes">{todo.notes}</p>}

      <div className="todo-card-chips">
        {todo.module && (
          <span className="todo-card-module" title={todo.module}>
            <Folder size={12} aria-hidden="true" />
            <span>{todo.module}</span>
          </span>
        )}
        <PriorityChip priority={todo.priority} />
        <TagChips tagIds={todo.tagIds} catalogs={catalogs} />
      </div>

      {(todo.dueDate || assignee) && (
        <div className="todo-card-meta">
          <DueChip todo={todo} today={today} />
          {assignee && <Avatar person={assignee} />}
        </div>
      )}

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
