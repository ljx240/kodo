/**
 * 待办 — one list row.
 *
 * The same facts as a board card, laid out as a table: the title and its
 * notes lead, then whichever columns are on, then 操作. A cell is kept even
 * when it has nothing to show, so the columns stay in line down the table —
 * a blank is information, a missing cell is a wobble.
 */

import { Check } from "lucide-react";
import type { TodoDto } from "../../../api";
import { T } from "../../../i18n";
import {
  Avatar,
  DueChip,
  PriorityChip,
  TagChips,
  TodoProgress,
  TodoStatusSelect,
  type ApplyStatus,
} from "./TodoBits";
import { stampOf, type Progress } from "./todoModel";
import { personOf, type Catalogs, type ColumnKey } from "./todoTypes";

const W = T.workbench.todo;

export function TodoRow({
  as = "div",
  todo,
  catalogs,
  today,
  columns,
  progress,
  titleDone,
  selected,
  onSelect,
  onApply,
  trailing,
}: {
  /** The archived block renders its rows as `<li>`; the table already has one. */
  as?: "div" | "li";
  todo: TodoDto;
  catalogs: Catalogs;
  today: string;
  columns: ColumnKey[];
  progress: Progress;
  titleDone: boolean;
  selected: boolean;
  onSelect: (next: boolean) => void;
  onApply: ApplyStatus;
  trailing: React.ReactNode;
}) {
  const Tag = as;
  const assignee = personOf(catalogs.people, todo.assigneeId);
  const { days, time } = stampOf(todo.updatedAt, today);
  const updated =
    days <= 0
      ? `${W.updatedToday} ${time}`
      : days === 1
        ? `${W.updatedYesterday} ${time}`
        : `${W.updatedOn(todo.updatedAt)}`;

  const cell: Record<ColumnKey, React.ReactNode> = {
    status: <TodoStatusSelect todo={todo} catalogs={catalogs} onApply={onApply} />,
    priority: <PriorityChip priority={todo.priority} />,
    due: <DueChip todo={todo} today={today} />,
    tags: <TagChips tagIds={todo.tagIds} catalogs={catalogs} />,
    assignee: assignee ? <Avatar person={assignee} /> : null,
    progress: <TodoProgress done={progress.done} total={progress.total} compact />,
    updated: <span className="todo-updated">{updated}</span>,
  };

  return (
    <Tag
      className={`todo-row${selected ? " todo-row--selected" : ""}${titleDone ? " todo-row--done" : ""}`}
      data-testid={`todo-row-${todo.id}`}
    >
      <label className="todo-select">
        <input
          type="checkbox"
          checked={selected}
          aria-label={W.selectRow}
          onChange={(event) => onSelect(event.target.checked)}
          data-testid={`todo-select-${todo.id}`}
        />
        <span className="todo-select-box" aria-hidden="true">
          <Check size={11} strokeWidth={3} />
        </span>
      </label>

      <div className="todo-cell todo-cell--title">
        <span className={`todo-row-title${titleDone ? " todo-row-title--done" : ""}`}>
          {todo.title}
        </span>
        {todo.notes && <span className="todo-row-notes">{todo.notes}</span>}
      </div>

      {columns.map((key) => (
        <div key={key} className={`todo-cell todo-cell--${key}`}>
          {cell[key]}
        </div>
      ))}

      <div className="todo-cell todo-cell--actions">{trailing}</div>
    </Tag>
  );
}
