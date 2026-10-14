/**
 * 待办 — the workbench's first app.
 *
 * One dataset (`listTodos`) backs both views and every filter, and one write
 * path (`setTodoStatus`) backs the drag, the status select and 完成 / 恢复, so
 * the board, the list, the column counts and 今日概览 can never disagree:
 * `status` is the only business state, and nothing else infers "done".
 *
 * Drag uses pointer events rather than HTML5 drag-and-drop: HTML5 DnD never
 * fires on touch, and this table has to work on a trackpad, a touchscreen and
 * a mouse alike. It starts from the handle (the rest of the card keeps
 * scrolling), Escape cancels, and a release outside a column leaves the todo
 * exactly where it was. The status select is the keyboard path to the same
 * write.
 */

import { Check, GripVertical, Pencil, Plus, RotateCcw, Undo2, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  createTodo,
  listTodos,
  setTodoArchived,
  setTodoStatus,
  updateTodo,
  type AppManifestDto,
  type TodoDto,
  type TodoPriorityDto,
  type TodoStatusDto,
} from "../../api";
import { T } from "../../i18n";
import { DEMO_TODAY } from "../../data/workbench";
import { workbenchErrorText } from "./errors";
import { today as todayIso } from "./time";

const W = T.workbench.todo;

type View = "board" | "list";
type Filter = "all" | "open" | "today" | "inbox" | "done";

const COLUMNS: TodoStatusDto[] = ["todo", "doing", "done"];
const COLUMN_LABEL: Record<TodoStatusDto, string> = {
  todo: W.columns.pending,
  doing: W.columns.doing,
  done: W.columns.done,
};
const FILTERS: Filter[] = ["all", "open", "today", "inbox", "done"];
const FILTER_LABEL: Record<Filter, string> = {
  all: W.filters.all,
  open: W.filters.open,
  today: W.filters.today,
  inbox: W.filters.inbox,
  done: W.filters.done,
};
const PRIORITIES: TodoPriorityDto[] = ["low", "normal", "high"];
const PRIORITY_LABEL: Record<TodoPriorityDto, string> = {
  low: W.priorities.low,
  normal: W.priorities.normal,
  high: W.priorities.high,
};

/** `MM-DD`, the form a board card can show without wrapping. */
function shortDate(date: string): string {
  return date.slice(5);
}

/** Read the drop column under the pointer. The ghost is `pointer-events:none`,
 *  so this sees the column beneath it. */
function columnAt(x: number, y: number): TodoStatusDto | null {
  const zone = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-todo-column]");
  const raw = zone?.dataset.todoColumn;
  return raw === "todo" || raw === "doing" || raw === "done" ? raw : null;
}

type Drag = {
  id: string;
  version: number;
  status: TodoStatusDto;
  title: string;
  x: number;
  y: number;
  over: TodoStatusDto | null;
};

type EditorState = {
  draft: TodoDto;
  /** Validation or backend failure shown inside the modal. */
  error: string | null;
  child: string;
};

type Props = {
  manifest: AppManifestDto;
  /** Demo routes freeze "today" so screenshots are reproducible. */
  demo: boolean;
  onBack: () => void;
};

export function TodoApp({ manifest, demo, onBack }: Props) {
  const today = demo ? DEMO_TODAY : todayIso();

  const [todos, setTodos] = useState<TodoDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<View>("board");
  const [filter, setFilter] = useState<Filter>("all");
  const [showArchived, setShowArchived] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [columnDraft, setColumnDraft] = useState<TodoStatusDto | null>(null);
  const [columnTitle, setColumnTitle] = useState("");
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [ghost, setGhost] = useState<Drag | null>(null);
  const dragRef = useRef<Drag | null>(null);

  const reload = useCallback(async (): Promise<TodoDto[]> => {
    const rows = await listTodos(true);
    setTodos(rows);
    return rows;
  }, []);

  useEffect(() => {
    reload().then(
      () => setError(null),
      (failure: unknown) => setError(workbenchErrorText(failure)),
    );
  }, [reload]);

  /** Replace one row in place — a write response is the newest truth. */
  const put = useCallback((next: TodoDto) => {
    setTodos((rows) => rows?.map((row) => (row.id === next.id ? next : row)) ?? rows);
  }, []);

  const fail = useCallback(
    (failure: unknown) => {
      setError(workbenchErrorText(failure));
      // A stale or missing row means our copy is out of date: re-read and let
      // the user retry against the current version.
      void reload().catch(() => {});
    },
    [reload],
  );

  /** The single status write path: drag, select and 完成 / 恢复 all land here. */
  const applyStatus = useCallback(
    (id: string, version: number, status: TodoStatusDto) => {
      void setTodoStatus(id, version, status)
        .then((next) => {
          put(next);
          setError(null);
        })
        .catch(fail);
    },
    [fail, put],
  );

  // --- drag -----------------------------------------------------------------

  const isDragging = ghost !== null;

  useEffect(() => {
    if (!isDragging) return;
    const move = (event: PointerEvent) => {
      const current = dragRef.current;
      if (!current) return;
      const next: Drag = {
        ...current,
        x: event.clientX,
        y: event.clientY,
        over: columnAt(event.clientX, event.clientY),
      };
      dragRef.current = next;
      setGhost(next);
    };
    const end = (event: PointerEvent) => {
      const current = dragRef.current;
      dragRef.current = null;
      setGhost(null);
      if (!current) return;
      const over = columnAt(event.clientX, event.clientY);
      // Cancelled or dropped outside a column: the status does not change.
      if (!over || over === current.status) return;
      applyStatus(current.id, current.version, over);
    };
    const cancel = () => {
      dragRef.current = null;
      setGhost(null);
    };
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") cancel();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("keydown", keydown);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", keydown);
    };
  }, [isDragging, applyStatus]);

  const startDrag = (event: React.PointerEvent<HTMLElement>, todo: TodoDto) => {
    if (event.pointerType === "mouse" && event.button !== 0) return;
    event.preventDefault();
    const started: Drag = {
      id: todo.id,
      version: todo.version,
      status: todo.status,
      title: todo.title,
      x: event.clientX,
      y: event.clientY,
      over: todo.status,
    };
    dragRef.current = started;
    setGhost(started);
  };

  // --- derived rows ---------------------------------------------------------

  const rows = useMemo(() => {
    const all = todos ?? [];
    const archived = all.filter((row) => row.archived);
    const live = all.filter((row) => !row.archived);
    const matches = (todo: TodoDto): boolean => {
      switch (filter) {
        case "open":
          return todo.status !== "done";
        case "today":
          return todo.dueDate === today;
        case "inbox":
          return todo.dueDate === null;
        case "done":
          return todo.status === "done";
        default:
          return true;
      }
    };
    // Children follow their parent: filtering only top-level rows keeps a
    // sub-item from floating into a column on its own.
    const tops = live
      .filter((row) => row.parentId === null && matches(row))
      .sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt - b.createdAt);
    // A child is returned only alongside its parent, so a sub-item whose
    // parent the filter dropped never appears on its own.
    const childrenOf = (id: string): TodoDto[] =>
      live.filter((row) => row.parentId === id).sort((a, b) => a.createdAt - b.createdAt);
    return { tops, childrenOf, archived };
  }, [todos, filter, today]);

  const boardRows = rows.tops;

  // --- writes ---------------------------------------------------------------

  const create = useCallback((title: string, status: TodoStatusDto, parentId: string | null) => {
    const trimmed = title.trim();
    if (!trimmed) {
      setError(W.titleRequired);
      return Promise.resolve(false);
    }
    return createTodo({
      title: trimmed,
      status,
      parentId,
      // One key per attempt: a double submit cannot create two todos.
      requestId: globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`,
    })
      .then((created) => {
        setTodos((current) => [...(current ?? []), created]);
        setError(null);
        return true;
      })
      .catch((failure: unknown) => {
        setError(workbenchErrorText(failure));
        return false;
      });
  }, []);

  const saveEditor = useCallback(
    (draft: TodoDto) => {
      const trimmed = draft.title.trim();
      if (!trimmed) {
        setEditor((current) => (current ? { ...current, error: W.titleRequired } : current));
        return;
      }
      updateTodo(draft.version, { ...draft, title: trimmed })
        .then((next) => {
          put(next);
          setError(null);
          setEditor(null);
        })
        .catch((failure: unknown) => {
          const code = workbenchErrorText(failure);
          setEditor((current) => (current ? { ...current, error: code } : current));
          // Re-read so a retry saves against the version that actually won.
          void reload()
            .then((fresh) => {
              const stored = fresh.find((row) => row.id === draft.id);
              if (!stored) return;
              setEditor((current) =>
                current
                  ? { ...current, draft: { ...current.draft, version: stored.version } }
                  : current,
              );
            })
            .catch(() => {});
        });
    },
    [put, reload],
  );

  const setArchived = useCallback(
    (todo: TodoDto, archived: boolean) => {
      void setTodoArchived(todo.id, todo.version, archived)
        .then((next) => {
          put(next);
          setError(null);
          setEditor(null);
        })
        .catch(fail);
    },
    [fail, put],
  );

  // --- rendering ------------------------------------------------------------

  const subCount = (id: string): number => rows.childrenOf(id).length;

  const meta = (todo: TodoDto) => (
    <span className="todo-meta">
      {todo.dueDate ? (
        <span
          className={`todo-due${todo.dueDate < today && todo.status !== "done" ? " todo-due--late" : ""}`}
        >
          {W.due} {shortDate(todo.dueDate)}
        </span>
      ) : (
        <span className="todo-due todo-due--none">{W.noDue}</span>
      )}
      <span className={`todo-priority todo-priority--${todo.priority}`}>
        {PRIORITY_LABEL[todo.priority]}
      </span>
      {subCount(todo.id) > 0 && (
        <span className="todo-subcount">{W.children(subCount(todo.id))}</span>
      )}
      {todo.source !== "manual" && (
        <span className="todo-source">
          {todo.source === "scheduled" ? W.sourceScheduled : W.sourceAi}
        </span>
      )}
      {todo.archived && <span className="todo-source">{W.archived}</span>}
    </span>
  );

  const statusSelect = (todo: TodoDto) => (
    <select
      className="select todo-status"
      aria-label={W.status}
      value={todo.status}
      data-testid={`todo-status-${todo.id}`}
      onChange={(event) => {
        const next = event.target.value as TodoStatusDto;
        if (next !== todo.status) applyStatus(todo.id, todo.version, next);
      }}
    >
      {COLUMNS.map((column) => (
        <option key={column} value={column}>
          {COLUMN_LABEL[column]}
        </option>
      ))}
    </select>
  );

  const completeButton = (todo: TodoDto) => {
    const done = todo.status === "done";
    const label = done ? W.restore : W.complete;
    return (
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        title={label}
        aria-label={label}
        data-testid={`todo-toggle-${todo.id}`}
        // 完成 → 已完成; 恢复 → 待处理, both through the one status path.
        onClick={() => applyStatus(todo.id, todo.version, done ? "todo" : "done")}
      >
        {done ? <Undo2 size={14} strokeWidth={1.8} /> : <Check size={14} strokeWidth={1.8} />}
      </button>
    );
  };

  const editButton = (todo: TodoDto) => (
    <button
      type="button"
      className="icon-btn icon-btn--sm"
      title={W.edit}
      aria-label={W.edit}
      data-testid={`todo-edit-${todo.id}`}
      onClick={() => setEditor({ draft: { ...todo }, error: null, child: "" })}
    >
      <Pencil size={13} strokeWidth={1.8} />
    </button>
  );

  const childRows = (todo: TodoDto) =>
    rows.childrenOf(todo.id).map((child) => (
      <li key={child.id} className="todo-child" data-testid={`todo-child-${child.id}`}>
        <span
          className={`todo-child-title${child.status === "done" ? " todo-child-title--done" : ""}`}
        >
          {child.title}
        </span>
        {statusSelect(child)}
        {completeButton(child)}
        {editButton(child)}
      </li>
    ));

  const card = (todo: TodoDto) => (
    <article
      key={todo.id}
      className={`todo-card${ghost?.id === todo.id ? " todo-card--dragging" : ""}${
        todo.status === "done" ? " todo-card--done" : ""
      }`}
      data-testid={`todo-card-${todo.id}`}
    >
      <div className="todo-card-top">
        <button
          type="button"
          className="todo-handle"
          aria-label={W.dragHandle}
          title={W.dragHandle}
          data-testid={`todo-handle-${todo.id}`}
          onPointerDown={(event) => startDrag(event, todo)}
        >
          <GripVertical size={14} strokeWidth={1.8} />
        </button>
        <div className="todo-card-text">
          <p className="todo-card-title">{todo.title}</p>
          {meta(todo)}
        </div>
      </div>
      <div className="todo-card-actions">
        {statusSelect(todo)}
        {completeButton(todo)}
        {editButton(todo)}
      </div>
      {subCount(todo.id) > 0 && <ul className="todo-children">{childRows(todo)}</ul>}
    </article>
  );

  // A `div`, not an `li`: the row is nested inside `li.todo-list-item` so the
  // sub-item list can sit under it without breaking the list's markup.
  const listRow = (todo: TodoDto) => (
    <div className="todo-row" data-testid={`todo-row-${todo.id}`}>
      <div className="todo-row-main">
        <span className={`todo-row-title${todo.status === "done" ? " todo-row-title--done" : ""}`}>
          {todo.title}
        </span>
        {meta(todo)}
      </div>
      {statusSelect(todo)}
      {completeButton(todo)}
      {editButton(todo)}
    </div>
  );

  const columnBody = (column: TodoStatusDto) => {
    const cards = boardRows.filter((row) => row.status === column);
    return (
      <div
        className={`todo-column-body${
          ghost && ghost.over === column && ghost.status !== column ? " todo-column-body--over" : ""
        }`}
        data-todo-column={column}
        data-testid={`todo-column-${column}`}
      >
        {columnDraft === column && (
          <form
            className="todo-inline"
            onSubmit={(event) => {
              event.preventDefault();
              void create(columnTitle, column, null).then((ok) => {
                if (ok) {
                  setColumnTitle("");
                  setColumnDraft(null);
                }
              });
            }}
          >
            <input
              className="input"
              autoFocus
              value={columnTitle}
              placeholder={W.newPlaceholder}
              aria-label={W.addInColumn(COLUMN_LABEL[column])}
              onChange={(event) => setColumnTitle(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  setColumnTitle("");
                  setColumnDraft(null);
                }
              }}
            />
          </form>
        )}
        {cards.length === 0 && columnDraft !== column ? (
          <p className="todo-column-empty">{W.empty}</p>
        ) : (
          cards.map(card)
        )}
        {ghost && ghost.over === column && ghost.status !== column && (
          <p className="todo-drop-hint">{W.dropHere}</p>
        )}
      </div>
    );
  };

  return (
    <main className="main page-app">
      <header className="page-head">
        <div className="page-head-titles">
          <h1>{manifest.name}</h1>
          <p className="page-subtitle">{manifest.description}</p>
        </div>
        <div className="page-head-actions">
          <button type="button" className="btn" onClick={onBack} data-testid="app-back">
            {T.workbench.back}
          </button>
        </div>
      </header>

      <div className="app-body">
        {error && (
          <p className="page-error" role="alert">
            {error}
          </p>
        )}

        <div className="app-toolbar">
          <div className="app-seg" role="group" aria-label={W.viewLabel}>
            {(["board", "list"] as const).map((value) => (
              <button
                key={value}
                type="button"
                className={`app-seg-btn${view === value ? " app-seg-btn--active" : ""}`}
                aria-pressed={view === value}
                onClick={() => setView(value)}
                data-testid={`todo-view-${value}`}
              >
                {value === "board" ? W.board : W.list}
              </button>
            ))}
          </div>

          <div className="app-filters" role="group" aria-label={W.filterLabel}>
            {FILTERS.map((value) => (
              <button
                key={value}
                type="button"
                className={`app-filter${filter === value ? " app-filter--active" : ""}`}
                aria-pressed={filter === value}
                onClick={() => setFilter(value)}
                data-testid={`todo-filter-${value}`}
              >
                {FILTER_LABEL[value]}
              </button>
            ))}
          </div>

          <button
            type="button"
            className={`app-filter${showArchived ? " app-filter--active" : ""}`}
            aria-pressed={showArchived}
            onClick={() => setShowArchived((on) => !on)}
            data-testid="todo-archived-toggle"
          >
            {W.archived}
          </button>

          <span className="spacer" />

          <form
            className="todo-new"
            onSubmit={(event) => {
              event.preventDefault();
              void create(newTitle, "todo", null).then((ok) => {
                if (ok) setNewTitle("");
              });
            }}
          >
            <input
              className="input"
              value={newTitle}
              placeholder={W.newPlaceholder}
              aria-label={W.add}
              onChange={(event) => setNewTitle(event.target.value)}
              data-testid="todo-new-input"
            />
            <button
              type="submit"
              className="btn btn--primary"
              disabled={!newTitle.trim()}
              data-testid="todo-new-submit"
            >
              {W.add}
            </button>
          </form>
        </div>

        {view === "board" ? (
          <div className="todo-board">
            {COLUMNS.map((column) => (
              <section key={column} className="todo-column" data-testid={`todo-col-${column}`}>
                <header className="todo-column-head">
                  <span className="todo-column-name">{COLUMN_LABEL[column]}</span>
                  <span className="todo-column-count">
                    {W.count(boardRows.filter((row) => row.status === column).length)}
                  </span>
                  <span className="spacer" />
                  <button
                    type="button"
                    className="icon-btn icon-btn--sm"
                    title={W.addInColumn(COLUMN_LABEL[column])}
                    aria-label={W.addInColumn(COLUMN_LABEL[column])}
                    data-testid={`todo-add-${column}`}
                    onClick={() => {
                      setColumnTitle("");
                      setColumnDraft(column);
                    }}
                  >
                    <Plus size={14} strokeWidth={1.8} />
                  </button>
                </header>
                {columnBody(column)}
              </section>
            ))}
          </div>
        ) : (
          <ul className="todo-list" data-testid="todo-list">
            {rows.tops.length === 0 && (
              <li className="todo-column-empty">{filter === "all" ? W.empty : W.emptyFiltered}</li>
            )}
            {rows.tops.map((todo) => (
              <li key={todo.id} className="todo-list-item">
                {listRow(todo)}
                {subCount(todo.id) > 0 && <ul className="todo-children">{childRows(todo)}</ul>}
              </li>
            ))}
          </ul>
        )}

        {showArchived && rows.archived.length > 0 && (
          <section className="todo-archived" data-testid="todo-archived">
            <h2 className="app-section-title">{W.archived}</h2>
            <ul className="todo-list">
              {rows.archived.map((todo) => (
                <li key={todo.id} className="todo-row" data-testid={`todo-row-${todo.id}`}>
                  <div className="todo-row-main">
                    <span className="todo-row-title">{todo.title}</span>
                    {meta(todo)}
                  </div>
                  {statusSelect(todo)}
                  <button
                    type="button"
                    className="icon-btn icon-btn--sm"
                    title={W.unarchive}
                    aria-label={W.unarchive}
                    data-testid={`todo-unarchive-${todo.id}`}
                    onClick={() => setArchived(todo, false)}
                  >
                    <RotateCcw size={13} strokeWidth={1.8} />
                  </button>
                  {editButton(todo)}
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>

      {ghost && (
        <div className="todo-drag-ghost" style={{ left: ghost.x + 12, top: ghost.y + 8 }}>
          {ghost.title}
        </div>
      )}

      {editor && (
        <div className="settings-modal" role="dialog" aria-modal="true" aria-label={W.edit}>
          <div className="settings-modal-backdrop" onClick={() => setEditor(null)} />
          <div className="settings-modal-panel wb-modal-panel">
            <header className="modal-head">
              <span className="modal-title">{W.edit}</span>
              <span className="spacer" />
              <button
                type="button"
                className="icon-btn icon-btn--sm"
                title={T.action.close}
                aria-label={T.action.close}
                onClick={() => setEditor(null)}
              >
                <X size={15} strokeWidth={1.8} />
              </button>
            </header>

            <div className="skill-detail-body skill-detail-body--edit">
              {editor.error && (
                <p className="page-error" role="alert">
                  {editor.error}
                </p>
              )}

              <label className="field field--wide">
                <span className="field-label">{W.titleLabel}</span>
                <input
                  className="input"
                  value={editor.draft.title}
                  aria-label={W.titleLabel}
                  data-testid="todo-edit-title"
                  onChange={(event) =>
                    setEditor((current) =>
                      current
                        ? { ...current, draft: { ...current.draft, title: event.target.value } }
                        : current,
                    )
                  }
                />
              </label>

              <div className="app-form-row">
                <label className="field">
                  <span className="field-label">{W.status}</span>
                  <select
                    className="select"
                    value={editor.draft.status}
                    aria-label={W.status}
                    data-testid="todo-edit-status"
                    onChange={(event) =>
                      setEditor((current) =>
                        current
                          ? {
                              ...current,
                              draft: {
                                ...current.draft,
                                status: event.target.value as TodoStatusDto,
                              },
                            }
                          : current,
                      )
                    }
                  >
                    {COLUMNS.map((column) => (
                      <option key={column} value={column}>
                        {COLUMN_LABEL[column]}
                      </option>
                    ))}
                  </select>
                </label>

                <label className="field">
                  <span className="field-label">{W.priority}</span>
                  <select
                    className="select"
                    value={editor.draft.priority}
                    aria-label={W.priority}
                    data-testid="todo-edit-priority"
                    onChange={(event) =>
                      setEditor((current) =>
                        current
                          ? {
                              ...current,
                              draft: {
                                ...current.draft,
                                priority: event.target.value as TodoPriorityDto,
                              },
                            }
                          : current,
                      )
                    }
                  >
                    {PRIORITIES.map((value) => (
                      <option key={value} value={value}>
                        {PRIORITY_LABEL[value]}
                      </option>
                    ))}
                  </select>
                </label>

                <label className="field">
                  <span className="field-label">{W.due}</span>
                  <input
                    type="date"
                    className="input"
                    value={editor.draft.dueDate ?? ""}
                    aria-label={W.due}
                    data-testid="todo-edit-due"
                    onChange={(event) =>
                      setEditor((current) =>
                        current
                          ? {
                              ...current,
                              draft: { ...current.draft, dueDate: event.target.value || null },
                            }
                          : current,
                      )
                    }
                  />
                </label>
              </div>

              <label className="field field--wide">
                <span className="field-label">{W.notes}</span>
                <textarea
                  className="input todo-notes"
                  rows={4}
                  value={editor.draft.notes}
                  aria-label={W.notes}
                  data-testid="todo-edit-notes"
                  onChange={(event) =>
                    setEditor((current) =>
                      current
                        ? { ...current, draft: { ...current.draft, notes: event.target.value } }
                        : current,
                    )
                  }
                />
              </label>

              <label className="field field--wide">
                <span className="field-label">{W.parent}</span>
                <select
                  className="select"
                  value={editor.draft.parentId ?? ""}
                  aria-label={W.parent}
                  data-testid="todo-edit-parent"
                  onChange={(event) =>
                    setEditor((current) =>
                      current
                        ? {
                            ...current,
                            draft: { ...current.draft, parentId: event.target.value || null },
                          }
                        : current,
                    )
                  }
                >
                  <option value="">{W.noParent}</option>
                  {(todos ?? [])
                    .filter((row) => row.parentId === null && row.id !== editor.draft.id)
                    .map((row) => (
                      <option key={row.id} value={row.id}>
                        {row.title}
                      </option>
                    ))}
                </select>
              </label>

              <section className="todo-editor-children">
                <h3 className="app-section-title">{W.subtasks}</h3>
                <ul className="todo-children">
                  {(todos ?? [])
                    .filter((row) => row.parentId === editor.draft.id)
                    .map((child) => (
                      <li key={child.id} className="todo-child">
                        <span className="todo-child-title">{child.title}</span>
                        {statusSelect(child)}
                        {completeButton(child)}
                      </li>
                    ))}
                </ul>
                <form
                  className="todo-inline"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void create(editor.child, editor.draft.status, editor.draft.id).then((ok) => {
                      if (ok)
                        setEditor((current) => (current ? { ...current, child: "" } : current));
                    });
                  }}
                >
                  <input
                    className="input"
                    value={editor.child}
                    placeholder={W.addSubtask}
                    aria-label={W.addSubtask}
                    data-testid="todo-add-subtask"
                    onChange={(event) =>
                      setEditor((current) =>
                        current ? { ...current, child: event.target.value } : current,
                      )
                    }
                  />
                </form>
              </section>
            </div>

            <footer className="skill-detail-foot">
              <button
                type="button"
                className="btn"
                onClick={() => setArchived(editor.draft, !editor.draft.archived)}
                data-testid="todo-edit-archive"
              >
                {editor.draft.archived ? W.unarchive : W.archive}
              </button>
              <span className="spacer" />
              <button type="button" className="btn" onClick={() => setEditor(null)}>
                {W.cancel}
              </button>
              <button
                type="button"
                className="btn btn--primary"
                onClick={() => saveEditor(editor.draft)}
                data-testid="todo-edit-save"
              >
                {W.save}
              </button>
            </footer>
          </div>
        </div>
      )}
    </main>
  );
}
