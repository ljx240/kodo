/**
 * 待办 — the shared leaves.
 *
 * The chips render the same markup on a board card, a list row and (some of
 * them) in the editor, so they live once here rather than three times inline.
 * Their class names and `data-testid`s are part of the contract the
 * Playwright suite pins.
 *
 * Colour never lives in here: a chip carries the *palette key* the catalog
 * gave it (`data-color="blue"`), and `todo.css` owns what that looks like.
 * A user-created status or tag therefore needs no new CSS at all.
 */

import { ArrowUp, Calendar, Check, GripVertical, ListChecks, Pencil, Undo2 } from "lucide-react";
import type { TodoDto, TodoPersonDto } from "../../../api";
import { T } from "../../../i18n";
import { isDone, isDueToday, isLate, stampOf } from "./todoModel";
import { smartDate, smartDateLabel } from "./todoDate";
import {
  doneStatus,
  openStatus,
  PRIORITY_LABEL,
  statusOf,
  tagOf,
  type Catalogs,
} from "./todoTypes";

const W = T.workbench.todo;

/** The one status write path: drag, select and 完成 / 恢复 all land here. */
export type ApplyStatus = (id: string, version: number, status: string) => void;

/** An initial in a tinted circle — the account-free stand-in for a photo. */
export function Avatar({ person }: { person: TodoPersonDto }) {
  const initial = Array.from(person.name)[0] ?? "?";
  return (
    <span
      className="todo-avatar"
      data-color={person.color}
      title={person.name}
      aria-label={person.name}
    >
      {initial}
    </span>
  );
}

/** 高 / 中 / 低 with the arrow the design uses for all three. */
export function PriorityChip({ priority }: { priority: TodoDto["priority"] }) {
  return (
    <span className={`todo-priority todo-priority--${priority}`}>
      <ArrowUp size={12} strokeWidth={2.4} />
      {PRIORITY_LABEL[priority]}
    </span>
  );
}

export function TagChip({ name, color }: { name: string; color: string }) {
  return (
    <span className="todo-tag" data-color={color}>
      {name}
    </span>
  );
}

/**
 * The due date: 今天 12:00 while it is still today, `YYYY/MM/DD` otherwise.
 *
 * It reads as urgent — red — when the date is today or already behind us on
 * a row that is not done.
 */
export function DueChip({ todo, today }: { todo: TodoDto; today: string }) {
  const urgent = isDueToday(todo, today) || isLate(todo, today);
  // Smart relative date: 今天 / 明天 / 昨天 / 已逾期 N 天 / N 天后 /
  // absolute date. The chip text stays short so it fits the card's meta row
  // without pushing other chips off; today's time is folded into 今天.
  let text: string;
  if (!todo.dueDate) {
    text = W.noDue;
  } else {
    const lookup = {
      "workbench.todo.dueToday": W.dueToday,
      "workbench.todo.dueTomorrow": W.dueTomorrow,
      "workbench.todo.dueYesterday": W.dueYesterday,
      "workbench.todo.overdue": W.overdue,
      "workbench.todo.dueDaysSuffix": W.dueDaysSuffix,
    };
    const smart = smartDate(todo.dueDate, today) ?? {
      kind: "absolute" as const,
      yyyyMmDd: todo.dueDate.replace(/-/g, "/"),
    };
    text = smartDateLabel(smart, lookup, todo.dueTime ?? "");
  }
  return (
    <span className={`todo-due${urgent ? " todo-due--late" : ""}${todo.dueDate ? "" : " todo-due--none"}`}>
      <Calendar size={12} strokeWidth={1.9} />
      {text}
    </span>
  );
}

/** The sub-item bar. The empty track shows too — `0/2` is a real state the
 *  design prints, and a missing bar would read as "no sub-items at all". */
export function ProgressBar({ done, total }: { done: number; total: number }) {
  return (
    <span
      className={`todo-progress${done === total ? " todo-progress--full" : ""}`}
      role="img"
      aria-label={W.childrenProgress(done, total)}
    >
      <span className="todo-progress-fill" style={{ width: `${(done / total) * 100}%` }} />
    </span>
  );
}

/**
 * The status of one row, as a pill the catalog names and colours.
 *
 * A native `<select>` keeps the keyboard path and the `todo-status-*` hook; the
 * dot beside it is a sibling rather than part of the control, because a
 * `<select>` cannot carry markup — and the colour is the one thing that tells
 * a user-created status apart from the built-ins.
 */
export function TodoStatusSelect({
  todo,
  catalogs,
  onApply,
}: {
  todo: TodoDto;
  catalogs: Catalogs;
  onApply: ApplyStatus;
}) {
  return (
    <span className="todo-status-field" data-color={statusOf(catalogs.statuses, todo.status)?.color}>
      <span className="todo-status-dot" aria-hidden="true" />
      <select
        className="select todo-status"
        aria-label={W.status}
        value={todo.status}
        data-testid={`todo-status-${todo.id}`}
        onChange={(event) => {
          const next = event.target.value;
          if (next !== todo.status) onApply(todo.id, todo.version, next);
        }}
      >
        {catalogs.statuses.map((status) => (
          <option key={status.id} value={status.id}>
            {status.name}
          </option>
        ))}
      </select>
    </span>
  );
}

export function TodoCompleteButton({
  todo,
  catalogs,
  onApply,
}: {
  todo: TodoDto;
  catalogs: Catalogs;
  onApply: ApplyStatus;
}) {
  const done = isDone(todo);
  const label = done ? W.restore : W.complete;
  // 完成 → the column flagged `isDone`; 恢复 → the first one that is not.
  // Both are the catalog's to name, and both go through the one status path.
  const target = done ? openStatus(catalogs.statuses) : doneStatus(catalogs.statuses);
  return (
    <button
      type="button"
      className="todo-check"
      title={label}
      aria-label={label}
      data-testid={`todo-toggle-${todo.id}`}
      disabled={!target}
      onClick={() => {
        if (target) onApply(todo.id, todo.version, target.id);
      }}
    >
      {done ? <Undo2 size={13} strokeWidth={2.2} /> : <Check size={13} strokeWidth={2.2} />}
    </button>
  );
}

export function TodoEditButton({
  todo,
  onEdit,
}: {
  todo: TodoDto;
  onEdit: (todo: TodoDto) => void;
}) {
  return (
    <button
      type="button"
      className="todo-more"
      title={W.edit}
      aria-label={W.edit}
      data-testid={`todo-edit-${todo.id}`}
      onClick={() => onEdit(todo)}
    >
      <Pencil size={13} strokeWidth={1.9} />
    </button>
  );
}

export function TodoDragHandle({
  todo,
  onDragStart,
}: {
  todo: TodoDto;
  onDragStart: (event: React.PointerEvent<HTMLElement>, todo: TodoDto) => void;
}) {
  return (
    <button
      type="button"
      className="todo-handle"
      aria-label={W.dragHandle}
      title={W.dragHandle}
      data-testid={`todo-handle-${todo.id}`}
      onPointerDown={(event) => onDragStart(event, todo)}
    >
      <GripVertical size={14} strokeWidth={1.9} />
    </button>
  );
}

/**
 * The sub-item progress line, and nothing at all when there are no sub-items.
 *
 * The card foot spells it out (`2/5 子项`) because it sits among unrelated
 * chips; the list's 子项进度 cell is already under its own header, so it says
 * just `2/5` there and saves the width.
 */
export function TodoProgress({
  done,
  total,
  compact = false,
}: {
  done: number;
  total: number;
  compact?: boolean;
}) {
  if (total === 0) return null;
  return (
    <span className="todo-subcount">
      {!compact && <ListChecks size={13} strokeWidth={1.8} />}
      {compact ? W.childrenRatio(done, total) : W.childrenProgress(done, total)}
      <ProgressBar done={done} total={total} />
    </span>
  );
}

/** The tag chips of one row, in catalog order, skipping ids that no longer
 *  resolve (a tag deleted while a row still names it). */
export function TagChips({ tagIds, catalogs }: { tagIds: string[]; catalogs: Catalogs }) {
  return (
    <>
      {tagIds.map((id) => {
        const tag = tagOf(catalogs.tags, id);
        return tag ? <TagChip key={id} name={tag.name} color={tag.color} /> : null;
      })}
    </>
  );
}

/** A short, calendar-relative label: "刚刚", "N 分钟前", "N 小时前",
 *  "HH:MM" for the same day, "昨天", "N 天前", or "YYYY/MM/DD". `today` is
 *  the screen's frozen clock. */
export function relativeTime(epochMs: number, today: string): string {
  const now = new Date(`${today}T00:00:00`).getTime();
  const minutes = Math.floor((now - epochMs) / 60_000);
  if (minutes < 1) return W.updatedJustNow;
  if (minutes < 60) return W.updatedMinutes(minutes);
  const { days, time } = stampOf(epochMs, today);
  if (days === 0) return time;
  if (days === 1) return W.updatedYesterday;
  if (days < 7) return W.updatedDays(days);
  return W.updatedOn(epochMs);
}
