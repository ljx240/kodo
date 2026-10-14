/**
 * 待办 — the workbench's first app.
 *
 * This file owns every piece of state and every write; the views under it are
 * presentational. One dataset (`listTodos`) backs both views and every chip,
 * and one write path (`setTodoStatus`) backs the drag, the status select and
 * 完成 / 恢复, so the board, the list, the column counts and 今日概览 can never
 * disagree: `status` is the only business state, and nothing else infers
 * "done".
 *
 * Drag uses pointer events rather than HTML5 drag-and-drop: HTML5 DnD never
 * fires on touch, and this table has to work on a trackpad, a touchscreen and
 * a mouse alike. It starts from the handle (the rest of the card keeps
 * scrolling), Escape cancels, and a release outside a column leaves the todo
 * exactly where it was. The status select is the keyboard path to the same
 * write.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  addTodoComment,
  bulkUpdateTodos,
  createTodo,
  createTodoAttachment,
  createTodoReminder,
  createTodoStatus,
  deleteTodoAttachment,
  deleteTodoReminder,
  deleteTodoStatus,
  getWorkbenchPrefs,
  listTodoActivity,
  listTodoAttachments,
  listTodoPeople,
  listTodoReminders,
  listTodoStatuses,
  listTodoTags,
  listTodos,
  pickTodoAttachment,
  reorderTodo,
  reorderTodoStatuses,
  setTodoArchived,
  setTodoLinks,
  setTodoReminderEnabled,
  setTodoStatus,
  setWorkbenchPref,
  updateTodo,
  updateTodoStatus,
  type TodoAttachmentDto,
  type TodoBulkOpDto,
  type TodoDto,
  type TodoReminderDto,
  type TodoReminderKindDto,
  type TodoStatusDefDto,
  type TodoStatusDraftDto,
} from "../../../api";
import { T } from "../../../i18n";
import { DEMO_TODAY } from "../../../data/workbench";
import { workbenchErrorText } from "../errors";
import { today as todayIso } from "../time";
import { TodoArchived, TodoList } from "./TodoList";
import { TodoBoard } from "./TodoBoard";
import { TodoEditorModal } from "./TodoEditorModal";
import { TodoSelectionBar } from "./TodoSelectionBar";
import { TodoStatsBar } from "./TodoStatsBar";
import { TodoStatusModal } from "./TodoStatusModal";
import { TodoToolbar } from "./TodoToolbar";
import { countsOf, partitionRows, statsOf, weekRangeOf } from "./todoModel";
import {
  COLUMNS_PREF,
  cardAt,
  columnAt,
  defaultStatus,
  readColumns,
  tagOf,
  type Catalogs,
  type ColumnKey,
  type Drag,
  type EditorState,
  type Filter,
  type Group,
  type Sort,
  type StatusId,
  type TodoAppProps,
  type View,
} from "./todoTypes";

/** Nothing loaded yet — the shape the first render passes down. */
const NO_CATALOGS: Catalogs = { statuses: [], people: [], tags: [] };

/** A blank row for 新建事项. `version: 0` is never sent: the id is what tells
 *  `saveEditor` this is a create, and `createTodo` ignores it. */
const blankTodo = (status: StatusId): TodoDto => ({
  id: "",
  title: "",
  notes: "",
  status,
  priority: "normal",
  dueDate: null,
  dueTime: null,
  parentId: null,
  assigneeId: null,
  itemType: "task",
  module: "",
  estimateHours: null,
  actualHours: null,
  archived: false,
  completedAt: null,
  sortOrder: 0,
  source: "manual",
  version: 0,
  createdAt: 0,
  updatedAt: 0,
  tagIds: [],
  linkIds: [],
  isDone: false,
});

const W = T.workbench.todo;

export function TodoApp({ manifest, demo, onBack }: TodoAppProps) {
  const today = demo ? DEMO_TODAY : todayIso();

  const [todos, setTodos] = useState<TodoDto[] | null>(null);
  const [catalogs, setCatalogs] = useState<Catalogs>(NO_CATALOGS);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<View>("board");
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState<Sort>("manual");
  const [group, setGroup] = useState<Group>("none");
  const [query, setQuery] = useState("");
  const [columns, setColumns] = useState<ColumnKey[]>(
    readColumns(null),
  );
  const [selection, setSelection] = useState<Set<string>>(new Set());
  const [archivedOpen, setArchivedOpen] = useState(false);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [ghost, setGhost] = useState<Drag | null>(null);
  const [statusModal, setStatusModal] = useState(false);
  const dragRef = useRef<Drag | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);
  /** Latest editor — read from event handlers that need to act on the open
   *  draft without depending on a stale closure (save, comment, attachment…). */
  const editorRef = useRef<EditorState | null>(null);
  useEffect(() => {
    editorRef.current = editor;
  }, [editor]);

  const reload = useCallback(async (): Promise<TodoDto[]> => {
    // The catalogs and the rows are one screen: the columns are the status
    // catalog, and every chip is a lookup into the other two — so a rename, a
    // new status or a recoloured tag has to land in the same render as the
    // rows it decorates.
    const [rows, statuses, people, tags] = await Promise.all([
      listTodos(true),
      listTodoStatuses(),
      listTodoPeople(),
      listTodoTags(),
    ]);
    setTodos(rows);
    setCatalogs({ statuses, people, tags });
    return rows;
  }, []);

  useEffect(() => {
    reload().then(
      () => setError(null),
      (failure: unknown) => setError(workbenchErrorText(failure)),
    );
  }, [reload]);

  // The column set is a preference, not a row: it is read once on mount and
  // written back on every change, through the same prefs the workbench uses.
  useEffect(() => {
    getWorkbenchPrefs()
      .then((prefs) => setColumns(readColumns(prefs[COLUMNS_PREF] ?? null)))
      .catch(() => {});
  }, []);

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
    (id: string, version: number, status: StatusId) => {
      void setTodoStatus(id, version, status)
        .then((next) => {
          put(next);
          setError(null);
        })
        .catch(fail);
    },
    [fail, put],
  );

  /** Drop a card on top of another one in the same column. The re-shuffled
   *  siblings arrive in `next`, and we replace the moved card plus the one
   *  whose slot changed with the freshly-stamped copies from the store. */
  const reorder = useCallback(
    (id: string, version: number, status: StatusId, beforeId: string | null) => {
      void reorderTodo(id, version, status, beforeId)
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
      const over = columnAt(event.clientX, event.clientY);
      // The card hit-test only matters within the same column; once the
      // pointer crosses into another column, the status change is what
      // matters and the position is the tail.
      const overCard = over === current.status ? cardAt(event.clientX, event.clientY) : null;
      const beforeId = overCard && overCard !== current.id ? overCard : null;
      const next: Drag = {
        ...current,
        x: event.clientX,
        y: event.clientY,
        over,
        beforeId,
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
      if (!over) return;
      if (over !== current.status) {
        applyStatus(current.id, current.version, over);
        return;
      }
      // Same column: only write when the drop target is a different card.
      const overCard = cardAt(event.clientX, event.clientY);
      if (!overCard || overCard === current.id) return;
      reorder(current.id, current.version, current.status, overCard);
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
  }, [applyStatus, isDragging, reorder]);

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
      beforeId: null,
    };
    dragRef.current = started;
    setGhost(started);
  };

  // --- ⌘K ------------------------------------------------------------------

  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    window.addEventListener("keydown", keydown);
    return () => window.removeEventListener("keydown", keydown);
  }, []);

  // --- derived rows ---------------------------------------------------------

  const week = useMemo(() => weekRangeOf(today), [today]);

  /** The ids the search lets through, or `null` while it is empty. Matching a
   *  row's tags means matching the names the chips show, not their ids. */
  const allowed = useMemo<Set<string> | null>(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return null;
    const hits = new Set<string>();
    for (const row of todos ?? []) {
      const tags = row.tagIds
        .map((id) => tagOf(catalogs.tags, id)?.name ?? "")
        .join(" ");
      if (`${row.title} ${row.notes} ${row.module} ${tags}`.toLowerCase().includes(needle)) {
        hits.add(row.id);
      }
    }
    return hits;
  }, [query, todos, catalogs.tags]);

  const rows = useMemo(
    () => partitionRows(todos, filter, today, week, allowed),
    [todos, filter, today, week, allowed],
  );
  const counts = useMemo(
    () => countsOf(todos, today, week, allowed),
    [todos, today, week, allowed],
  );
  const stats = useMemo(() => statsOf(todos), [todos]);

  // --- writes ---------------------------------------------------------------

  const create = useCallback(
    (title: string, status: StatusId, parentId: string | null): Promise<boolean> => {
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
    },
    [],
  );

  const saveEditor = useCallback(
    (draft: TodoDto) => {
      const trimmed = draft.title.trim();
      if (!trimmed) {
        setEditor((current) => (current ? { ...current, error: W.titleRequired } : current));
        return;
      }
      const showFail = (failure: unknown): void => {
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
      };

      // An id-less draft is a new row: it goes through the same create the
      // column's quick-add uses, then closes.
      if (draft.id === "") {
        void createTodo({
          title: trimmed,
          status: draft.status,
          parentId: draft.parentId,
          notes: draft.notes,
          priority: draft.priority,
          dueDate: draft.dueDate,
          dueTime: draft.dueTime,
          assigneeId: draft.assigneeId,
          itemType: draft.itemType,
          module: draft.module,
          estimateHours: draft.estimateHours,
          actualHours: draft.actualHours,
          tagIds: draft.tagIds,
          requestId: globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`,
        })
          .then((created) => {
            setTodos((current) => [...(current ?? []), created]);
            setError(null);
            setEditor(null);
          })
          .catch(showFail);
        return;
      }

      updateTodo(draft.version, { ...draft, title: trimmed })
        .then((next) => {
          put(next);
          setError(null);
          setEditor(null);
        })
        .catch(showFail);
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

  /** One round trip for the whole selection, then the rows are re-merged from
   *  the response — so the table shows what the store committed, not what we
   *  hoped it would. A delete returns nothing, so its rows are dropped here. */
  const bulk = useCallback(
    (op: TodoBulkOpDto) => {
      const ids = [...selection];
      if (ids.length === 0) return;
      void bulkUpdateTodos(ids, op)
        .then((next) => {
          if (op.op === "delete") {
            setTodos((current) =>
              (current ?? []).filter(
                (row) => !ids.includes(row.id) && !(row.parentId && ids.includes(row.parentId)),
              ),
            );
          } else {
            const changed = new Map(next.map((row) => [row.id, row]));
            setTodos((current) => (current ?? []).map((row) => changed.get(row.id) ?? row));
          }
          setSelection(new Set());
          setError(null);
        })
        .catch(fail);
    },
    [fail, selection],
  );

  const toggleSelect = useCallback((id: string, next: boolean) => {
    setSelection((current) => {
      const copy = new Set(current);
      if (next) copy.add(id);
      else copy.delete(id);
      return copy;
    });
  }, []);

  const selectAll = useCallback(
    (next: boolean) => {
      setSelection(next ? new Set(rows.tops.map((todo) => todo.id)) : new Set());
    },
    [rows.tops],
  );

  const saveColumns = useCallback((next: ColumnKey[]) => {
    setColumns(next);
    void setWorkbenchPref(COLUMNS_PREF, JSON.stringify(next)).catch(() => {});
  }, []);

  const openEditor = useCallback((todo: TodoDto) => {
    setEditor({
      draft: { ...todo },
      error: null,
      child: "",
      comment: "",
      attachments: [],
      reminders: [],
      activity: [],
      reminderDraft: null,
    });
    if (todo.id === "") return;
    void Promise.all([
      listTodoAttachments(todo.id),
      listTodoReminders(todo.id),
      listTodoActivity(todo.id),
    ])
      .then(([attachments, reminders, activity]) => {
        setEditor((current) =>
          current && current.draft.id === todo.id
            ? { ...current, attachments, reminders, activity }
            : current,
        );
      })
      .catch(fail);
  }, [fail]);

  const openNew = useCallback(() => {
    setEditor({
      draft: blankTodo(defaultStatus(catalogs.statuses)),
      error: null,
      child: "",
      comment: "",
      attachments: [],
      reminders: [],
      activity: [],
      reminderDraft: null,
    });
  }, [catalogs.statuses]);

  const pickFilter = useCallback((next: Filter) => {
    setFilter(next);
    // The board has no column an archived row could sit in, so asking for the
    // archived rows takes you to the one view that can show them.
    if (next === "archived") setView("list");
  }, []);

  // --- status manager ----------------------------------------------------

  const createStatus = useCallback(
    async (draft: TodoStatusDraftDto): Promise<void> => {
      try {
        await createTodoStatus(draft);
        await reload();
        setError(null);
      } catch (failure) {
        setError(workbenchErrorText(failure));
        throw failure;
      }
    },
    [reload],
  );

  const updateStatus = useCallback(
    async (status: TodoStatusDefDto): Promise<void> => {
      try {
        await updateTodoStatus(status.id, status.version, status);
        // Flipping `isDone` rewrites every todo in the column's `completedAt`,
        // so the rows have to be re-read before any view shows them.
        await reload();
        setError(null);
      } catch (failure) {
        setError(workbenchErrorText(failure));
        throw failure;
      }
    },
    [reload],
  );

  const deleteStatus = useCallback(
    async (id: string, version: number, reassignTo: string | null): Promise<void> => {
      try {
        await deleteTodoStatus(id, version, reassignTo);
        await reload();
        setError(null);
      } catch (failure) {
        setError(workbenchErrorText(failure));
        throw failure;
      }
    },
    [reload],
  );

  const reorderStatuses = useCallback(
    async (ids: string[]): Promise<void> => {
      try {
        await reorderTodoStatuses(ids);
        await reload();
        setError(null);
      } catch (failure) {
        setError(workbenchErrorText(failure));
        throw failure;
      }
    },
    [reload],
  );

  // --- editor sub-resource handlers ---------------------------------------

  const setComment = useCallback((value: string) => {
    setEditor((current) => (current ? { ...current, comment: value } : current));
  }, []);

  const sendComment = useCallback(async (): Promise<boolean> => {
    const current = editorRef.current;
    if (!current) return false;
    const body = current.comment.trim();
    if (!body) return false;
    try {
      const added = await addTodoComment(current.draft.id, current.draft.assigneeId, body);
      setEditor((state) =>
        state && state.draft.id === current.draft.id
          ? { ...state, comment: "", activity: [...state.activity, added] }
          : state,
      );
      return true;
    } catch (failure) {
      const code = workbenchErrorText(failure);
      setEditor((state) => (state ? { ...state, error: code } : state));
      return false;
    }
  }, []);

  const addAttachment = useCallback(async (): Promise<boolean> => {
    const current = editorRef.current;
    if (!current) return false;
    const path = await pickTodoAttachment().catch(() => null);
    if (!path) return false;
    const name = path.split(/[\\/]/).pop() ?? path;
    try {
      const created = await createTodoAttachment({
        todoId: current.draft.id,
        name,
        path,
        requestId: globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`,
      });
      setEditor((state) =>
        state && state.draft.id === current.draft.id
          ? { ...state, attachments: [...state.attachments, created] }
          : state,
      );
      return true;
    } catch (failure) {
      const code = workbenchErrorText(failure);
      setEditor((state) => (state ? { ...state, error: code } : state));
      return false;
    }
  }, []);

  const removeAttachment = useCallback((attachment: TodoAttachmentDto) => {
    void deleteTodoAttachment(attachment.id, attachment.version)
      .then(() => {
        setEditor((state) =>
          state
            ? { ...state, attachments: state.attachments.filter((one) => one.id !== attachment.id) }
            : state,
        );
      })
      .catch(fail);
  }, [fail]);

  const addReminder = useCallback(
    async (value: {
      kind: TodoReminderKindDto;
      offsetMinutes: number;
      message: string;
    }): Promise<boolean> => {
      const current = editorRef.current;
      if (!current) return false;
      try {
        const created = await createTodoReminder({
          todoId: current.draft.id,
          kind: value.kind,
          offsetMinutes: value.kind === "dueBefore" ? value.offsetMinutes : null,
          message: value.message,
        });
        setEditor((state) =>
          state && state.draft.id === current.draft.id
            ? { ...state, reminders: [...state.reminders, created] }
            : state,
        );
        return true;
      } catch (failure) {
        const code = workbenchErrorText(failure);
        setEditor((state) => (state ? { ...state, error: code } : state));
        return false;
      }
    },
    [],
  );

  const toggleReminder = useCallback(
    (reminder: TodoReminderDto, enabled: boolean) => {
      void setTodoReminderEnabled(reminder.id, reminder.version, enabled)
        .then((next) => {
          setEditor((state) =>
            state
              ? {
                  ...state,
                  reminders: state.reminders.map((one) => (one.id === next.id ? next : one)),
                }
              : state,
          );
        })
        .catch(fail);
    },
    [fail],
  );

  const removeReminder = useCallback(
    (reminder: TodoReminderDto) => {
      void deleteTodoReminder(reminder.id, reminder.version)
        .then(() => {
          setEditor((state) =>
            state
              ? { ...state, reminders: state.reminders.filter((one) => one.id !== reminder.id) }
              : state,
          );
        })
        .catch(fail);
    },
    [fail],
  );

  const toggleRelated = useCallback(
    (otherId: string) => {
      const current = editorRef.current;
      if (!current) return;
      const next = current.draft.linkIds.includes(otherId)
        ? current.draft.linkIds.filter((id) => id !== otherId)
        : [...current.draft.linkIds, otherId];
      const draft = { ...current.draft, linkIds: next };
      setEditor((state) => (state ? { ...state, draft } : state));
      if (current.draft.id === "") return;
      void setTodoLinks(current.draft.id, current.draft.version, next).catch(fail);
    },
    [fail],
  );

  return (
    <main className="main page-app todo-app">
      <header className="page-head">
        <div className="page-head-titles">
          <h1>{manifest.name}</h1>
          <p className="page-subtitle">{manifest.description}</p>
        </div>
        <div className="page-head-actions">
          <label className="todo-search">
            <input
              ref={searchRef}
              className="input"
              value={query}
              placeholder={W.searchPlaceholder}
              aria-label={W.searchLabel}
              onChange={(event) => setQuery(event.target.value)}
              data-testid="todo-search"
            />
            <span className="todo-search-key">{W.searchShortcut}</span>
          </label>
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

        {selection.size > 0 ? (
          <TodoSelectionBar
            count={selection.size}
            catalogs={catalogs}
            onBulk={bulk}
            onClear={() => setSelection(new Set())}
          />
        ) : (
          <TodoToolbar
            view={view}
            onView={setView}
            filter={filter}
            onFilter={pickFilter}
            counts={counts}
            sort={sort}
            onSort={setSort}
            group={group}
            onGroup={setGroup}
            onCreate={openNew}
            onManageStatuses={() => setStatusModal(true)}
          />
        )}

        {view === "board" ? (
          <TodoBoard
            rows={rows}
            catalogs={catalogs}
            today={today}
            ghost={ghost}
            onApply={applyStatus}
            onEdit={openEditor}
            onDragStart={startDrag}
            onCreate={(title, status) => create(title, status, null)}
          />
        ) : (
          <TodoList
            rows={rows}
            catalogs={catalogs}
            today={today}
            filter={filter}
            sort={sort}
            group={group}
            columns={columns}
            selection={selection}
            onSelect={toggleSelect}
            onSelectAll={selectAll}
            onColumns={saveColumns}
            onApply={applyStatus}
            onEdit={openEditor}
          />
        )}

        {/* The 已归档 fold belongs to the table, and the design puts it
            nowhere on the board — the board's columns are all live rows. */}
        {view === "list" && filter !== "archived" && rows.archived.length > 0 && (
          <TodoArchived
            rows={rows}
            catalogs={catalogs}
            today={today}
            columns={columns}
            open={archivedOpen}
            onToggle={() => setArchivedOpen((on) => !on)}
            selection={selection}
            onSelect={toggleSelect}
            onApply={applyStatus}
            onEdit={openEditor}
            onUnarchive={(todo) => setArchived(todo, false)}
          />
        )}

        <TodoStatsBar stats={stats} />
      </div>

      {ghost && (
        <div className="todo-drag-ghost" style={{ left: ghost.x + 12, top: ghost.y + 8 }}>
          {ghost.title}
        </div>
      )}

      {editor && (
        <TodoEditorModal
          editor={editor}
          todos={todos}
          catalogs={catalogs}
          onPatch={(patch) =>
            setEditor((current) =>
              current ? { ...current, draft: { ...current.draft, ...patch } } : current,
            )
          }
          onChildText={(value) =>
            setEditor((current) => (current ? { ...current, child: value } : current))
          }
          onCommentText={setComment}
          onCommentSend={sendComment}
          onAttachmentAdd={addAttachment}
          onAttachmentRemove={removeAttachment}
          onReminderAdd={addReminder}
          onReminderToggle={toggleReminder}
          onReminderRemove={removeReminder}
          onRelatedToggle={toggleRelated}
          onClose={() => setEditor(null)}
          onSave={() => saveEditor(editor.draft)}
          onArchive={() => setArchived(editor.draft, !editor.draft.archived)}
          onCreateChild={(title) =>
            create(title, editor.draft.status, editor.draft.id).then((ok) => {
              if (ok) setEditor((current) => (current ? { ...current, child: "" } : current));
              return ok;
            })
          }
          onApply={applyStatus}
        />
      )}

      {statusModal && (
        <TodoStatusModal
          statuses={catalogs.statuses}
          rows={todos ?? []}
          busyId={null}
          onCreate={createStatus}
          onUpdate={updateStatus}
          onDelete={deleteStatus}
          onReorder={reorderStatuses}
          onClose={() => setStatusModal(false)}
        />
      )}
    </main>
  );
}
