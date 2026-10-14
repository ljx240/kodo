/**
 * 待办 — the list view, its table header and the 已归档 group under it.
 *
 * Grid rows, not a `<table>`: the columns are the user's to hide, every row
 * carries controls, and a grouping header has to be able to interrupt the
 * body — three things a real `<table>` fights rather than helps. The header
 * and the rows share one `--todo-cols` template set on the wrapper, so they
 * cannot drift apart.
 */

import { Check, ChevronDown, ChevronRight, RotateCcw, Settings2, Square } from "lucide-react";
import type { TodoDto } from "../../../api";
import { T } from "../../../i18n";
import { Menu, MenuItem } from "../../../shell/Menu";
import {
  TodoCompleteButton,
  TodoEditButton,
  type ApplyStatus,
} from "./TodoBits";
import { TodoRow } from "./TodoRow";
import { groupRows, isDone, progressOf, sortRows, type TodoRows } from "./todoModel";
import {
  COLUMNS,
  COLUMN_LABEL,
  PRIORITIES,
  PRIORITY_LABEL,
  listTemplate,
  statusName,
  type Catalogs,
  type ColumnKey,
  type Filter,
  type Group,
  type Sort,
} from "./todoTypes";

const W = T.workbench.todo;

/** `Columns3` at 14px, in the menu's own icon slot. */
const ColumnIcon = (): React.ReactNode => <Square size={14} strokeWidth={1.6} />;

/** The column list with one key flipped, kept in the design's own order so the
 *  stored pref never depends on the order they were switched on. */
function toggleColumn(columns: ColumnKey[], key: ColumnKey): ColumnKey[] {
  const next = columns.includes(key)
    ? columns.filter((one) => one !== key)
    : COLUMNS.filter((one) => columns.includes(one) || one === key);
  return next;
}

type RowProps = {
  rows: TodoRows;
  catalogs: Catalogs;
  today: string;
  columns: ColumnKey[];
  selection: Set<string>;
  onSelect: (id: string, next: boolean) => void;
  onApply: ApplyStatus;
  onEdit: (todo: TodoDto) => void;
};

/** One row, wired to the selection and to 完成 / 编辑. */
function Row({
  todo,
  rows,
  catalogs,
  today,
  columns,
  selection,
  onSelect,
  onApply,
  trailing,
}: RowProps & { todo: TodoDto; trailing: React.ReactNode }) {
  return (
    <li className="todo-list-item">
      <TodoRow
        todo={todo}
        catalogs={catalogs}
        today={today}
        columns={columns}
        progress={progressOf(rows, todo.id)}
        titleDone={isDone(todo)}
        selected={selection.has(todo.id)}
        onSelect={(next) => onSelect(todo.id, next)}
        onApply={onApply}
        trailing={trailing}
      />
    </li>
  );
}

export function TodoList({
  rows,
  catalogs,
  today,
  filter,
  sort,
  group,
  columns,
  selection,
  onSelect,
  onSelectAll,
  onColumns,
  onApply,
  onEdit,
}: RowProps & {
  filter: Filter;
  sort: Sort;
  group: Group;
  onSelectAll: (next: boolean) => void;
  onColumns: (next: ColumnKey[]) => void;
}) {
  const ordered = sortRows(rows.tops, sort);
  const buckets = groupRows(
    ordered,
    group,
    catalogs.statuses.map((status) => status.id),
    [...PRIORITIES].reverse(),
  );
  const allSelected = ordered.length > 0 && ordered.every((todo) => selection.has(todo.id));

  const row = (todo: TodoDto) => (
    <Row
      key={todo.id}
      todo={todo}
      rows={rows}
      catalogs={catalogs}
      today={today}
      columns={columns}
      selection={selection}
      onSelect={onSelect}
      onApply={onApply}
      onEdit={onEdit}
      trailing={
        <>
          <TodoCompleteButton todo={todo} catalogs={catalogs} onApply={onApply} />
          <TodoEditButton todo={todo} onEdit={onEdit} />
        </>
      }
    />
  );

  return (
    <div
      className="todo-table"
      data-testid="todo-table"
      style={{ ["--todo-cols" as string]: listTemplate(columns) }}
    >
      <div className="todo-list-head">
        <label className="todo-select">
          <input
            type="checkbox"
            checked={allSelected}
            aria-label={W.selectAll}
            onChange={(event) => onSelectAll(event.target.checked)}
            data-testid="todo-select-all"
          />
          <span className="todo-select-box" aria-hidden="true">
            <Check size={11} strokeWidth={3} />
          </span>
        </label>
        <span className="todo-head-cell todo-head-cell--title">{W.columns.title}</span>
        {columns.map((key) => (
          <span key={key} className="todo-head-cell">
            {COLUMN_LABEL[key]}
          </span>
        ))}
        <span className="todo-head-cell todo-head-cell--actions">
          {W.columns.actions}
          {/* The gear is the always-there way in: the selection bar that also
              offers it is only up while rows are selected. */}
          <Menu
            align="right"
            trigger={({ open, toggle }) => (
              <button
                type="button"
                className="todo-more"
                title={W.customColumns}
                aria-label={W.customColumns}
                aria-expanded={open}
                onClick={toggle}
                data-testid="todo-columns"
              >
                <Settings2 size={13} strokeWidth={1.9} />
              </button>
            )}
          >
            {() =>
              COLUMNS.map((key) => (
                <MenuItem
                  key={key}
                  icon={
                    columns.includes(key) ? (
                      <Check size={14} strokeWidth={2.2} />
                    ) : (
                      <ColumnIcon />
                    )
                  }
                  label={COLUMN_LABEL[key]}
                  selected={columns.includes(key)}
                  testId={`todo-column-${key}`}
                  // Stays open across clicks: hiding three columns is one
                  // visit to the menu, not three.
                  onSelect={() => onColumns(toggleColumn(columns, key))}
                />
              ))
            }
          </Menu>
        </span>
      </div>

      {ordered.length === 0 ? (
        <p className="todo-column-empty">{filter === "all" ? W.empty : W.noResults}</p>
      ) : (
        buckets.map((bucket) => (
          <ul
            key={bucket.key ?? "__flat"}
            className="todo-list"
            data-testid={bucket.key === null ? "todo-list" : `todo-group-${bucket.key}`}
          >
            {bucket.key !== null && (
              <li className="todo-group-head" role="presentation">
                <span className="todo-group-name">
                  {group === "status"
                    ? statusName(catalogs.statuses, bucket.key)
                    : PRIORITY_LABEL[bucket.key as "low" | "normal" | "high"]}
                </span>
                <span className="todo-group-count">{bucket.rows.length}</span>
              </li>
            )}
            {bucket.rows.map(row)}
          </ul>
        ))
      )}
    </div>
  );
}

/**
 * The 已归档 rows, folded away under the table.
 *
 * Not a filter result but its complement: these are the rows the active chip
 * is *not* about, which is why they sit below the table rather than in it.
 * Under the 已归档 chip they are the table, so the group is left out
 * entirely — see the caller.
 */
export function TodoArchived({
  rows,
  catalogs,
  today,
  columns,
  open,
  onToggle,
  selection,
  onSelect,
  onApply,
  onEdit,
  onUnarchive,
}: RowProps & { open: boolean; onToggle: () => void; onUnarchive: (todo: TodoDto) => void }) {
  return (
    <section className="todo-archived" data-testid="todo-archived">
      <button
        type="button"
        className="todo-archived-head"
        aria-expanded={open}
        onClick={onToggle}
        data-testid="todo-archived-toggle"
      >
        {open ? (
          <ChevronDown size={14} strokeWidth={2} />
        ) : (
          <ChevronRight size={14} strokeWidth={2} />
        )}
        <span className="todo-group-name">{W.archived}</span>
        <span className="todo-group-count">{rows.archived.length}</span>
      </button>

      {open && (
        <ul
          className="todo-list"
          data-testid="todo-archived-list"
          style={{ ["--todo-cols" as string]: listTemplate(columns) }}
        >
          {rows.archived.map((todo) => (
            <li key={todo.id} className="todo-list-item">
              <TodoRow
                todo={todo}
                catalogs={catalogs}
                today={today}
                columns={columns}
                progress={progressOf(rows, todo.id)}
                titleDone={isDone(todo)}
                selected={selection.has(todo.id)}
                onSelect={(next) => onSelect(todo.id, next)}
                onApply={onApply}
                trailing={
                  <>
                    <button
                      type="button"
                      className="todo-more"
                      title={W.unarchive}
                      aria-label={W.unarchive}
                      data-testid={`todo-unarchive-${todo.id}`}
                      onClick={() => onUnarchive(todo)}
                    >
                      <RotateCcw size={13} strokeWidth={1.9} />
                    </button>
                    <TodoEditButton todo={todo} onEdit={onEdit} />
                  </>
                }
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
