/**
 * 待办 — the board view.
 *
 * One column per row of the status catalog, each a drop target carrying
 * `data-todo-column` (the attribute the drag hit-test reads). The inline
 * "add a card" draft is local state: nothing outside this view can see it.
 *
 * The two empty states are different on purpose. A column that is not
 * finished yet offers a dashed drop target — there is more to do, and this is
 * where it goes. A column flagged `isDone` says 太棒了 instead: there is
 * nothing left to drop *into* it.
 */

import { Plus } from "lucide-react";
import { useState } from "react";
import type { TodoDto } from "../../../api";
import { T } from "../../../i18n";
import { TodoCard } from "./TodoCard";
import type { ApplyStatus } from "./TodoBits";
import { columnRows, type TodoRows } from "./todoModel";
import { statusName, type Catalogs, type Drag, type StatusId } from "./todoTypes";

const W = T.workbench.todo;

export function TodoBoard({
  rows,
  catalogs,
  today,
  ghost,
  onApply,
  onEdit,
  onDragStart,
  onCreate,
}: {
  rows: TodoRows;
  catalogs: Catalogs;
  today: string;
  ghost: Drag | null;
  onApply: ApplyStatus;
  onEdit: (todo: TodoDto) => void;
  onDragStart: (event: React.PointerEvent<HTMLElement>, todo: TodoDto) => void;
  onCreate: (title: string, status: StatusId) => Promise<boolean>;
}) {
  const [draftColumn, setDraftColumn] = useState<StatusId | null>(null);
  const [draftTitle, setDraftTitle] = useState("");

  const closeDraft = () => {
    setDraftTitle("");
    setDraftColumn(null);
  };

  const openDraft = (column: StatusId) => {
    setDraftTitle("");
    setDraftColumn(column);
  };

  const columnBody = (column: StatusId, isDone: boolean) => {
    const cards = columnRows(rows, column);
    return (
      <div
        className={`todo-column-body${
          ghost && ghost.over === column && ghost.status !== column ? " todo-column-body--over" : ""
        }`}
        data-todo-column={column}
        data-testid={`todo-column-${column}`}
      >
        {draftColumn === column && (
          <form
            className="todo-inline"
            onSubmit={(event) => {
              event.preventDefault();
              void onCreate(draftTitle, column).then((ok) => {
                if (ok) closeDraft();
              });
            }}
          >
            <input
              className="input"
              autoFocus
              value={draftTitle}
              placeholder={W.newPlaceholder}
              aria-label={W.addInColumn(statusName(catalogs.statuses, column))}
              onChange={(event) => setDraftTitle(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") closeDraft();
              }}
            />
          </form>
        )}

        {cards.map((todo) => (
          <TodoCard
            key={todo.id}
            todo={todo}
            rows={rows}
            catalogs={catalogs}
            today={today}
            dragging={ghost?.id === todo.id}
            onApply={onApply}
            onEdit={onEdit}
            onDragStart={onDragStart}
          />
        ))}

        {isDone ? (
          <div className="todo-empty" data-testid={`todo-empty-${column}`}>
            <InboxCheck />
            <p className="todo-empty-title">{W.emptyStatusTitle}</p>
            <p className="todo-empty-hint">{W.emptyStatusHint}</p>
          </div>
        ) : (
          <button
            type="button"
            className="todo-drop-zone"
            data-testid={`todo-drop-${column}`}
            onClick={() => openDraft(column)}
          >
            <span className="todo-drop-badge">
              <Plus size={14} strokeWidth={2} />
            </span>
            <span className="todo-drop-title">{W.dropTitle}</span>
            <span className="todo-drop-hint">{W.dropHint}</span>
          </button>
        )}
      </div>
    );
  };

  return (
    <div className="todo-board">
      {catalogs.statuses.map((status) => {
        const column = status.id;
        const label = status.name;
        const count = columnRows(rows, column).length;
        return (
          <section key={column} className="todo-column" data-testid={`todo-col-${column}`}>
            <header className="todo-column-head">
              <span className="todo-status-dot" data-color={status.color} />
              <span className="todo-column-name">{label}</span>
              <span className="todo-column-count">{W.count(count)}</span>
              <span className="spacer" />
              <button
                type="button"
                className="todo-more"
                title={W.addInColumn(label)}
                aria-label={W.addInColumn(label)}
                data-testid={`todo-add-${column}`}
                onClick={() => openDraft(column)}
              >
                <Plus size={14} strokeWidth={1.9} />
              </button>
            </header>
            <button
              type="button"
              className="todo-column-add"
              data-testid={`todo-add-row-${column}`}
              onClick={() => openDraft(column)}
            >
              <Plus size={13} strokeWidth={1.9} />
              {W.addRow}
            </button>
            {columnBody(column, status.isDone)}
          </section>
        );
      })}
    </div>
  );
}

/** The 太棒了 illustration: an inbox with a check over it. */
function InboxCheck() {
  return (
    <svg
      className="todo-empty-art"
      width="52"
      height="52"
      viewBox="0 0 52 52"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M10 16.5 20 11h12l10 5.5V35a4 4 0 0 1-4 4H14a4 4 0 0 1-4-4V16.5Z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
      <path
        d="M10 29h8.5l2.5 4h10l2.5-4H42"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
      <path
        d="m21 17.5 3.4 3.4L32 13.5"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
