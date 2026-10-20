/**
 * 待办 — the two rows above both views.
 *
 * Row one is what you are looking at: the chips, the three menus, the view
 * switch and 新建事项. Everything the chips do, 筛选 does too — the chips are
 * the quick path and the menu is the same six choices in one control, which
 * is what keeps the row usable once it has to wrap.
 *
 * 分组 is list-only by design: on the board the columns already are the
 * grouping, and a control that changed nothing visible would be a lie.
 */

import { ArrowUpDown, Check, LayoutGrid, Layers, ListFilter, Plus, Rows3, Settings2 } from "lucide-react";
import { T } from "../../../i18n";
import { Menu, MenuItem } from "../../../shell/Menu";
import {
  FILTERS,
  FILTER_LABEL,
  GROUPS,
  GROUP_LABEL,
  SORTS,
  SORT_LABEL,
  type Filter,
  type Group,
  type Sort,
  type View,
} from "./todoTypes";

const W = T.workbench.todo;

/** The icon each filter chip leads with, so the row scans without reading. */
const VIEW_ICON: Record<View, React.ReactNode> = {
  board: <LayoutGrid size={13} strokeWidth={1.9} />,
  list: <Rows3 size={13} strokeWidth={1.9} />,
};

export function TodoToolbar({
  view,
  onView,
  filter,
  onFilter,
  counts,
  sort,
  onSort,
  group,
  onGroup,
  onCreate,
  onManageStatuses,
}: {
  view: View;
  onView: (view: View) => void;
  filter: Filter;
  onFilter: (filter: Filter) => void;
  /** How many rows each chip would show, over the same rows the views read. */
  counts: Record<Filter, number>;
  sort: Sort;
  onSort: (sort: Sort) => void;
  group: Group;
  onGroup: (group: Group) => void;
  onCreate: () => void;
  onManageStatuses: () => void;
}) {
  return (
    <div className="todo-toolbar">
      <div className="todo-filters" role="group" aria-label={W.filterLabel}>
        {FILTERS.map((value) => (
          <button
            key={value}
            type="button"
            className={`todo-chip${filter === value ? " todo-chip--active" : ""}`}
            aria-pressed={filter === value}
            onClick={() => onFilter(value)}
            data-testid={`todo-filter-${value}`}
          >
            {FILTER_LABEL[value]}
            <span className="todo-chip-count">{counts[value]}</span>
          </button>
        ))}
      </div>

      <span className="todo-toolbar-divider" aria-hidden="true" />

      <Menu
        align="right"
        trigger={({ open, toggle }) => (
          <button
            type="button"
            className={`todo-pill${open ? " todo-pill--open" : ""}`}
            onClick={toggle}
            data-testid="todo-menu-filter"
          >
            <ListFilter size={13} strokeWidth={1.9} />
            {W.filterMenu}
          </button>
        )}
      >
        {(close) =>
          FILTERS.map((value) => (
            <MenuItem
              key={value}
              icon={
                filter === value ? <Check size={14} strokeWidth={2.2} /> : <ListFilter size={14} />
              }
              label={FILTER_LABEL[value]}
              hint={String(counts[value])}
              selected={filter === value}
              testId={`todo-menu-filter-${value}`}
              onSelect={() => {
                onFilter(value);
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
            data-testid="todo-menu-sort"
          >
            <ArrowUpDown size={13} strokeWidth={1.9} />
            {W.sortMenu}
          </button>
        )}
      >
        {(close) =>
          SORTS.map((value) => (
            <MenuItem
              key={value}
              icon={<ArrowUpDown size={14} />}
              label={SORT_LABEL[value]}
              selected={sort === value}
              testId={`todo-menu-sort-${value}`}
              onSelect={() => {
                onSort(value);
                close();
              }}
            />
          ))
        }
      </Menu>

      {view === "list" && (
        <Menu
          align="right"
          trigger={({ open, toggle }) => (
            <button
              type="button"
              className={`todo-pill${open ? " todo-pill--open" : ""}`}
              onClick={toggle}
              data-testid="todo-menu-group"
            >
              <Layers size={13} strokeWidth={1.9} />
              {W.groupMenu}
            </button>
          )}
        >
          {(close) =>
            GROUPS.map((value) => (
              <MenuItem
                key={value}
                icon={<Layers size={14} />}
                label={GROUP_LABEL[value]}
                selected={group === value}
                testId={`todo-menu-group-${value}`}
                onSelect={() => {
                  onGroup(value);
                  close();
                }}
              />
            ))
          }
        </Menu>
      )}

      <button
        type="button"
        className="btn todo-pill"
        onClick={onManageStatuses}
        data-testid="todo-manage-statuses"
        title={T.workbench.todo.statusManager.title}
      >
        <Settings2 size={13} strokeWidth={1.9} />
        {T.workbench.todo.statusManager.title}
      </button>

      <div className="app-seg" role="group" aria-label={W.viewLabel}>
        {(["board", "list"] as const).map((value) => (
          <button
            key={value}
            type="button"
            className={`app-seg-btn${view === value ? " app-seg-btn--active" : ""}`}
            aria-pressed={view === value}
            onClick={() => onView(value)}
            data-testid={`todo-view-${value}`}
          >
            {VIEW_ICON[value]}
            {value === "board" ? W.board : W.list}
          </button>
        ))}
      </div>

      <button
        type="button"
        className="btn btn--primary todo-new-btn"
        onClick={onCreate}
        data-testid="todo-new"
      >
        <Plus size={14} strokeWidth={2.2} />
        {W.add}
      </button>
    </div>
  );
}
