/**
 * 待办 — the bar that replaces the toolbar's right-hand side once rows are
 * selected.
 *
 * Every action sends one `bulkUpdateTodos` for the whole selection: the store
 * applies it in a single transaction, so the table cannot end up half moved.
 * The count is the selection's own size and not a re-count of the rows still
 * on screen, because a chip can hide a row that is selected.
 */

import { Archive, ArrowUpDown, ListFilter, Trash2 } from "lucide-react";
import type { TodoBulkOpDto } from "../../../api";
import { T } from "../../../i18n";
import { Menu, MenuItem } from "../../../shell/Menu";
import { PRIORITIES, PRIORITY_LABEL, type Catalogs } from "./todoTypes";

const W = T.workbench.todo;

export function TodoSelectionBar({
  count,
  catalogs,
  onBulk,
  onClear,
}: {
  count: number;
  catalogs: Catalogs;
  onBulk: (op: TodoBulkOpDto) => void;
  onClear: () => void;
}) {
  return (
    <div className="todo-selection" data-testid="todo-selection">
      <button type="button" className="todo-selection-count" onClick={onClear} data-testid="todo-clear">
        {W.selected(count)}
      </button>

      <Menu
        align="right"
        trigger={({ open, toggle }) => (
          <button
            type="button"
            className={`todo-pill${open ? " todo-pill--open" : ""}`}
            onClick={toggle}
            data-testid="todo-bulk-status"
          >
            <ListFilter size={13} strokeWidth={1.9} />
            {W.moveStatus}
          </button>
        )}
      >
        {(close) =>
          catalogs.statuses.map((status) => (
            <MenuItem
              key={status.id}
              icon={<span className="todo-status-dot" data-color={status.color} />}
              label={status.name}
              testId={`todo-bulk-status-${status.id}`}
              onSelect={() => {
                onBulk({ op: "setStatus", status: status.id });
                close();
              }}
            />
          ))
        }
      </Menu>

      <Menu
        align="right"
        trigger={({ open, toggle }) => (
          <button
            type="button"
            className={`todo-pill${open ? " todo-pill--open" : ""}`}
            onClick={toggle}
            data-testid="todo-bulk-priority"
          >
            <ArrowUpDown size={13} strokeWidth={1.9} />
            {W.setPriority}
          </button>
        )}
      >
        {(close) =>
          PRIORITIES.map((priority) => (
            <MenuItem
              key={priority}
              icon={<ArrowUpDown size={14} />}
              label={PRIORITY_LABEL[priority]}
              testId={`todo-bulk-priority-${priority}`}
              onSelect={() => {
                onBulk({ op: "setPriority", priority });
                close();
              }}
            />
          ))
        }
      </Menu>

      <button
        type="button"
        className="todo-pill"
        onClick={() => onBulk({ op: "archive", archived: true })}
        data-testid="todo-bulk-archive"
      >
        <Archive size={13} strokeWidth={1.9} />
        {W.bulkArchive}
      </button>

      <button
        type="button"
        className="todo-pill todo-pill--danger"
        onClick={() => onBulk({ op: "delete" })}
        data-testid="todo-bulk-delete"
      >
        <Trash2 size={13} strokeWidth={1.9} />
        {W.bulkDelete}
      </button>
    </div>
  );
}
