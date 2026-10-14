/**
 * 待办 — the pure part.
 *
 * Everything here is a plain function over the loaded rows: no React, no
 * `T`, no writes. The board, the list, the column counts and the editor all
 * read the same helpers, so they cannot disagree about what is "done", what
 * a filter lets through, or how children attach to a parent.
 */

import type { TodoDto } from "../../../api";
import type { Filter, Group, Sort } from "./todoTypes";

/** The one place "this todo is done" is decided.
 *
 *  `isDone` rides along on the row, derived by the store from the status
 *  catalog — so renaming or re-flagging a status moves every todo with it,
 *  and nothing here has to know which ids mean "done". */
export function isDone(todo: TodoDto): boolean {
  return todo.isDone;
}

/**
 * The Monday-to-Sunday week containing `today`, as dates.
 *
 * This mirrors the 日历's own week grid, which is also Monday-based; the two
 * have to agree for 本周 here to mean the week the calendar is showing. The
 * caller passes `today` in, so a demo route can freeze it.
 */
export function weekRangeOf(today: string): { start: string; end: string } {
  const date = new Date(`${today}T00:00:00`);
  const weekday = (date.getDay() + 6) % 7;
  const start = new Date(date);
  start.setDate(start.getDate() - weekday);
  const end = new Date(start);
  end.setDate(end.getDate() + 6);
  const iso = (day: Date): string =>
    `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(
      day.getDate(),
    ).padStart(2, "0")}`;
  return { start: iso(start), end: iso(end) };
}

/** Does one row survive the active filter? */
export function matchesFilter(
  todo: TodoDto,
  filter: Filter,
  today: string,
  week: { start: string; end: string },
): boolean {
  switch (filter) {
    case "open":
      return !isDone(todo);
    case "today":
      return todo.dueDate === today;
    case "week":
      return todo.dueDate !== null && todo.dueDate >= week.start && todo.dueDate <= week.end;
    case "high":
      return todo.priority === "high";
    default:
      return true;
  }
}

export type TodoRows = {
  /** Top-level rows the filter kept, in board order. */
  tops: TodoDto[];
  /** Live children keyed by parent id, oldest first. */
  children: Map<string, TodoDto[]>;
  /** Archived top-level rows, filter-independent — the list's 已归档 group
   *  shows them whatever chip is active, so it cannot be built from `tops`. */
  archived: TodoDto[];
};

const byManual = (a: TodoDto, b: TodoDto): number =>
  a.sortOrder - b.sortOrder || a.createdAt - b.createdAt;

/**
 * Split the loaded rows three ways.
 *
 * Children follow their parent: filtering only top-level rows keeps a
 * sub-item from floating into a column on its own, and a child is reachable
 * only through the parent the filter kept. The children map is always built
 * from the live rows, so an archived parent still shows its progress bar —
 * archiving a parent does not archive its sub-items.
 */
export function partitionRows(
  todos: TodoDto[] | null,
  filter: Filter,
  today: string,
  week: { start: string; end: string },
  allow: Set<string> | null = null,
): TodoRows {
  const all = todos ?? [];
  const live = all.filter((row) => !row.archived);
  const archived = all.filter((row) => row.archived && row.parentId === null);

  // 已归档 is the one chip that reaches the archived rows; every other chip
  // works within the live ones.
  const pool = filter === "archived" ? all.filter((row) => row.archived) : live;

  const tops = pool
    .filter(
      (row) =>
        row.parentId === null &&
        matchesFilter(row, filter, today, week) &&
        // The search narrows what the chip already let through. A child that
        // matches on its own still shows only through the parent that kept it.
        (allow === null || allow.has(row.id)),
    )
    .sort(byManual);

  const children = new Map<string, TodoDto[]>();
  for (const row of live) {
    if (row.parentId === null) continue;
    const bucket = children.get(row.parentId);
    if (bucket) bucket.push(row);
    else children.set(row.parentId, [row]);
  }
  for (const bucket of children.values()) {
    bucket.sort((a, b) => a.createdAt - b.createdAt);
  }

  return { tops, children, archived };
}

/** How many top-level rows each chip would show. One pass, so the numbers on
 *  the chips can never disagree with what clicking them reveals. */
export function countsOf(
  todos: TodoDto[] | null,
  today: string,
  week: { start: string; end: string },
  allow: Set<string> | null = null,
): Record<Filter, number> {
  const tops = (todos ?? []).filter(
    (row) => row.parentId === null && (allow === null || allow.has(row.id)),
  );
  const live = tops.filter((row) => !row.archived);
  return {
    all: live.length,
    open: live.filter((row) => !isDone(row)).length,
    today: live.filter((row) => row.dueDate === today).length,
    week: live.filter(
      (row) => row.dueDate !== null && row.dueDate >= week.start && row.dueDate <= week.end,
    ).length,
    high: live.filter((row) => row.priority === "high").length,
    archived: tops.length - live.length,
  };
}

const PRIORITY_RANK: Record<TodoDto["priority"], number> = { high: 0, normal: 1, low: 2 };

/** The order the 排序 menu asks for. `manual` is the stored order. */
export function sortRows(rows: TodoDto[], sort: Sort): TodoDto[] {
  if (sort === "manual") return rows;
  const sorted = [...rows];
  if (sort === "due") {
    // Dated rows first and soonest first; undated ones keep to the end in
    // their stored order rather than being scattered among the dated ones.
    sorted.sort((a, b) => {
      if (a.dueDate === null && b.dueDate === null) return byManual(a, b);
      if (a.dueDate === null) return 1;
      if (b.dueDate === null) return -1;
      return a.dueDate.localeCompare(b.dueDate) || byManual(a, b);
    });
    return sorted;
  }
  if (sort === "priority") {
    sorted.sort(
      (a, b) => PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority] || byManual(a, b),
    );
    return sorted;
  }
  sorted.sort((a, b) => b.updatedAt - a.updatedAt || byManual(a, b));
  return sorted;
}

export type RowGroup = {
  /** The status id or priority key the rows share. `null` for 无分组. */
  key: string | null;
  rows: TodoDto[];
};

/**
 * The list's grouping, in catalog order for 状态 and high-to-low for 优先级.
 *
 * Labels stay with the caller: this module knows nothing about the catalog,
 * and a grouping helper that had to be handed a catalog would no longer be
 * the same kind of thing as the rest of this file.
 */
export function groupRows(
  rows: TodoDto[],
  group: Group,
  statusOrder: string[],
  priorityOrder: string[],
): RowGroup[] {
  if (group === "none") return [{ key: null, rows }];
  const keys = group === "status" ? statusOrder : priorityOrder;
  const buckets = new Map<string, TodoDto[]>();
  for (const row of rows) {
    const key = group === "status" ? row.status : row.priority;
    const bucket = buckets.get(key);
    if (bucket) bucket.push(row);
    else buckets.set(key, [row]);
  }
  // Catalog order first, then anything the catalog does not know about — a
  // row left pointing at a status the user deleted still has to be rendered.
  const ordered = [...keys.filter((key) => buckets.has(key))];
  for (const key of buckets.keys()) {
    if (!ordered.includes(key)) ordered.push(key);
  }
  return ordered.map((key) => ({ key, rows: buckets.get(key) ?? [] }));
}

/** Live children of one todo, in board order. */
export function childrenOf(rows: TodoRows, id: string): TodoDto[] {
  return rows.children.get(id) ?? [];
}

/** How many sub-items hang off one todo. */
export function subCountOf(rows: TodoRows, id: string): number {
  return rows.children.get(id)?.length ?? 0;
}

/** Top-level rows sitting in one column. */
export function columnRows(rows: TodoRows, status: string): TodoDto[] {
  return rows.tops.filter((row) => row.status === status);
}

/** A due date already behind `today` on a row that is not done yet. */
export function isLate(todo: TodoDto, today: string): boolean {
  return todo.dueDate !== null && todo.dueDate < today && !isDone(todo);
}

/**
 * How a stored timestamp reads against `today`: whole days back, and the
 * clock time it carries.
 *
 * Pure, and wordless — the copy is the caller's, and `today` is passed in so
 * a demo route's frozen date produces the same line every time.
 */
export function stampOf(epochMs: number, today: string): { days: number; time: string } {
  const stamp = new Date(epochMs);
  const midnight = new Date(`${today}T00:00:00`);
  const own = new Date(stamp.getFullYear(), stamp.getMonth(), stamp.getDate());
  const pad = (n: number): string => String(n).padStart(2, "0");
  return {
    days: Math.round((midnight.getTime() - own.getTime()) / 86_400_000),
    time: `${pad(stamp.getHours())}:${pad(stamp.getMinutes())}`,
  };
}

/** A due date that is today — the one the chip reads as urgent rather than
 *  merely scheduled. */
export function isDueToday(todo: TodoDto, today: string): boolean {
  return todo.dueDate === today && !isDone(todo);
}

export type Progress = {
  done: number;
  total: number;
  /** 0…1, and 0 when there is nothing to finish. */
  ratio: number;
};

/** Sub-item progress, counted from the same children the list renders. */
export function progressOf(rows: TodoRows, id: string): Progress {
  const children = rows.children.get(id) ?? [];
  const done = children.filter((child) => isDone(child)).length;
  return { done, total: children.length, ratio: children.length === 0 ? 0 : done / children.length };
}

export type Stats = {
  /** Top-level rows of every kind — the number the bar's first cell names. */
  total: number;
  open: number;
  done: number;
  archived: number;
};

/**
 * Counts for the sticky footer, over **top-level rows only**.
 *
 * Sub-items roll up into their parent's progress bar and are never counted
 * here: a parent with five sub-items is one item on the board, and the four
 * cells always add up to `total`.
 */
export function statsOf(todos: TodoDto[] | null): Stats {
  const tops = (todos ?? []).filter((row) => row.parentId === null);
  const archived = tops.filter((row) => row.archived).length;
  const done = tops.filter((row) => !row.archived && isDone(row)).length;
  return { total: tops.length, open: tops.length - done - archived, done, archived };
}
