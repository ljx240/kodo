/**
 * 待办 — local types and the module-level constants derived from i18n.
 *
 * Kept apart from the components so the model layer (`todoModel.ts`) can be
 * read as pure functions over these types, with no JSX and no React import.
 */

import type {
  AppManifestDto,
  TodoActivityDto,
  TodoAttachmentDto,
  TodoDto,
  TodoPersonDto,
  TodoPriorityDto,
  TodoReminderDto,
  TodoStatusDefDto,
  TodoTagDto,
} from "../../../api";
import { T } from "../../../i18n";

const W = T.workbench.todo;

export type View = "board" | "list";

/**
 * The chips above both views. `archived` is a filter rather than a toggle —
 * it is one more thing you can ask to see, and it is the only one whose rows
 * the board has no column for.
 */
export type Filter = "all" | "open" | "today" | "week" | "high" | "archived";
export type Sort = "manual" | "due" | "priority" | "updated";
/** The list's grouping. The board is always grouped by status — it is columns. */
export type Group = "none" | "status" | "priority";

/** A status id, as it comes off a row. The catalog names and colours it. */
export type StatusId = string;
/** The board's columns, in order — `list_todo_statuses`, never a literal. */
export type StatusCatalog = TodoStatusDefDto[];
export type PeopleCatalog = TodoPersonDto[];
export type TagCatalog = TodoTagDto[];

/**
 * The three reference lists one screen needs.
 *
 * They travel as one prop because a card, a row and the editor all need the
 * same three, and threading three separate props through four components
 * would be the only thing they disagree about.
 */
export type Catalogs = {
  statuses: StatusCatalog;
  people: PeopleCatalog;
  tags: TagCatalog;
};

/** The column a column id names, or `undefined` if the catalog lost it. */
export function statusOf(catalog: StatusCatalog, id: StatusId): TodoStatusDefDto | undefined {
  return catalog.find((status) => status.id === id);
}

/** Its display name; an id with no row left falls back to the id itself, so a
 *  stale row shows something nameable instead of a blank pill. */
export function statusName(catalog: StatusCatalog, id: StatusId): string {
  return statusOf(catalog, id)?.name ?? id;
}

/** The column 完成 moves a todo to — the first one flagged `isDone`. */
export function doneStatus(catalog: StatusCatalog): TodoStatusDefDto | undefined {
  return catalog.find((status) => status.isDone);
}

/** `kodo_core::apps::DEFAULT_TODO_STATUS` — where a new item lands. */
export const DEFAULT_TODO_STATUS = "todo";

/** The person a row is assigned to, or `undefined` when it has none. */
export function personOf(catalog: PeopleCatalog, id: string | null): TodoPersonDto | undefined {
  return id === null ? undefined : catalog.find((person) => person.id === id);
}

/** One tag by id; a tag deleted while a row still names it renders nothing. */
export function tagOf(catalog: TagCatalog, id: string): TodoTagDto | undefined {
  return catalog.find((tag) => tag.id === id);
}

/** The default column, or the catalog's first if the user removed it. */
export function defaultStatus(catalog: StatusCatalog): StatusId {
  return statusOf(catalog, DEFAULT_TODO_STATUS)?.id ?? catalog[0]?.id ?? DEFAULT_TODO_STATUS;
}

/** The column 恢复 moves it back to — the first one that is not `isDone`. */
export function openStatus(catalog: StatusCatalog): TodoStatusDefDto | undefined {
  return catalog.find((status) => !status.isDone);
}

export const FILTERS: Filter[] = ["all", "open", "today", "week", "high", "archived"];
export const FILTER_LABEL: Record<Filter, string> = {
  all: W.filters.all,
  open: W.filters.open,
  today: W.filters.today,
  week: W.filters.week,
  high: W.filters.high,
  archived: W.filters.archived,
};

export const SORTS: Sort[] = ["manual", "due", "priority", "updated"];
export const SORT_LABEL: Record<Sort, string> = {
  manual: W.sortOptions.manual,
  due: W.sortOptions.due,
  priority: W.sortOptions.priority,
  updated: W.sortOptions.updated,
};

export const GROUPS: Group[] = ["none", "status", "priority"];
export const GROUP_LABEL: Record<Group, string> = {
  none: W.groupOptions.none,
  status: W.groupOptions.status,
  priority: W.groupOptions.priority,
};

/**
 * The list's columns, in the design's order.
 *
 * 标题 and 操作 are always there; everything between them is what the ⚙
 * 自定义列 menu adds and removes. All seven start on: the design shows the
 * whole table, and the menu is there to take away what a screen does not need.
 */
export type ColumnKey = "status" | "priority" | "due" | "tags" | "assignee" | "progress" | "updated";
export const COLUMNS: ColumnKey[] = [
  "status",
  "priority",
  "due",
  "tags",
  "assignee",
  "progress",
  "updated",
];
/** Where a fresh install starts, and where a pref written by another build
 *  falls back to. */
export const DEFAULT_COLUMNS: ColumnKey[] = [...COLUMNS];
export const COLUMN_LABEL: Record<ColumnKey, string> = {
  status: W.columns.status,
  priority: W.columns.priority,
  due: W.columns.due,
  tags: W.columns.tags,
  assignee: W.columns.assignee,
  progress: W.columns.progress,
  updated: W.columns.updated,
};

/** How wide each column is, in the list's one grid template. The two flexible
 *  tracks are the ones whose content is a sentence (标题) and a list of chips
 *  (标签), so a wider window gives them the room rather than the date. */
export const COLUMN_WIDTH: Record<ColumnKey, string> = {
  status: "112px",
  priority: "88px",
  due: "124px",
  tags: "minmax(110px, 1fr)",
  assignee: "76px",
  progress: "96px",
  updated: "92px",
};

/** The list's grid: the checkbox, the title, whichever columns are on, 操作. */
export function listTemplate(columns: ColumnKey[]): string {
  return ["26px", "minmax(220px, 2fr)", ...columns.map((key) => COLUMN_WIDTH[key]), "60px"].join(
    " ",
  );
}

/** The workbench pref the column set is stored under. */
export const COLUMNS_PREF = "todo.columns";

/** A persisted column set, as the pref stores it. Unknown keys are dropped
 *  rather than trusted — a pref written by an older build must not break the
 *  table, and one written by a newer build must not invent a column. */
export function readColumns(raw: string | null): ColumnKey[] {
  if (!raw) return DEFAULT_COLUMNS;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return DEFAULT_COLUMNS;
    return COLUMNS.filter((key) => parsed.includes(key));
  } catch {
    return DEFAULT_COLUMNS;
  }
}

export const PRIORITIES: TodoPriorityDto[] = ["low", "normal", "high"];
export const PRIORITY_LABEL: Record<TodoPriorityDto, string> = {
  low: W.priorities.low,
  normal: W.priorities.normal,
  high: W.priorities.high,
};

/** `MM-DD`, the form a board card can show without wrapping. */
export function shortDate(date: string): string {
  return date.slice(5);
}

/** Read the drop column under the pointer. The ghost is `pointer-events:none`,
 *  so this sees the column beneath it. Any status id counts — the catalog is
 *  the user's, so the drop target cannot be a closed set. */
export function columnAt(x: number, y: number): StatusId | null {
  const zone = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-todo-column]");
  return zone?.dataset.todoColumn ?? null;
}

/** The card the pointer is over, or `null` if the pointer is on padding or
 *  on a card in a different column from the one we started dragging. */
export function cardAt(x: number, y: number): string | null {
  const card = document
    .elementFromPoint(x, y)
    ?.closest<HTMLElement>("[data-todo-card]");
  return card?.dataset.todoCard ?? null;
}

export type Drag = {
  id: string;
  version: number;
  status: StatusId;
  title: string;
  x: number;
  y: number;
  over: StatusId | null;
  /** The id of the card the pointer is hovering over, so an in-column drop
   *  can land *before* it rather than at the tail. `null` means "at the end". */
  beforeId: string | null;
};

export type EditorState = {
  draft: TodoDto;
  /** Validation or backend failure shown inside the modal. */
  error: string | null;
  /** New sub-task title being typed. */
  child: string;
  /** New comment being typed. */
  comment: string;
  /** Sub-resources loaded on editor open. */
  attachments: TodoAttachmentDto[];
  reminders: TodoReminderDto[];
  activity: TodoActivityDto[];
  /** Reminder being composed (`kind` + the minutes / message). */
  reminderDraft: { kind: TodoReminderDto["kind"]; offsetMinutes: number; message: string } | null;
};

export type TodoAppProps = {
  manifest: AppManifestDto;
  /** Demo routes freeze "today" so screenshots are reproducible. */
  demo: boolean;
  onBack: () => void;
};
