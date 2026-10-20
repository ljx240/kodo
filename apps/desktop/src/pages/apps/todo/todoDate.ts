/**
 * Smart date formatting for the board card.
 *
 * Linear / Things style. Relative labels: 今天 / 明天 / 昨天, `N 天后` for
 * the future, and `已逾期 N 天` once the date is behind us.
 */

const MS_PER_DAY = 24 * 60 * 60 * 1000;

/** Local midnight for a `YYYY-MM-DD` string. `null` for invalid input. */
function localMidnight(yyyyMmDd: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(yyyyMmDd);
  if (!match) return null;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(year, month - 1, day);
  if (
    date.getFullYear() !== year ||
    date.getMonth() !== month - 1 ||
    date.getDate() !== day
  ) {
    return null;
  }
  return date.getTime();
}

/** Local midnight for "today" so date arithmetic stays in the user's day. */
function todayLocalMidnight(today: string): number {
  return localMidnight(today) ?? new Date().setHours(0, 0, 0, 0);
}

export type SmartDate =
  | { kind: "today" }
  | { kind: "tomorrow" }
  | { kind: "yesterday" }
  | { kind: "overdue"; days: number }
  | { kind: "in-days"; days: number }
  | { kind: "absolute"; yyyyMmDd: string };

export function smartDate(yyyyMmDd: string, today: string): SmartDate | null {
  const target = localMidnight(yyyyMmDd);
  if (target === null) return null;
  const base = todayLocalMidnight(today);
  const days = Math.round((target - base) / MS_PER_DAY);
  if (days === 0) return { kind: "today" };
  if (days === 1) return { kind: "tomorrow" };
  if (days === -1) return { kind: "yesterday" };
  if (days < -1) return { kind: "overdue", days: -days };
  if (days > 1) return { kind: "in-days", days };
  return { kind: "absolute", yyyyMmDd };
}

/** Lookup values may be plain strings or functions of the due time — 今天
 *  folds the time in (`今天 12:00`) instead of appending it a second time. */
type SmartDateLabels = Readonly<Record<string, string | ((time: string) => string)>>;

export function smartDateLabel(s: SmartDate, lookup: SmartDateLabels, time = ""): string {
  const pick = (key: string): string => {
    const value = lookup[key];
    if (typeof value === "function") return value(time);
    return typeof value === "string" ? value : key;
  };
  const label = (() => {
    switch (s.kind) {
      case "today":
        return pick("workbench.todo.dueToday");
      case "tomorrow":
        return pick("workbench.todo.dueTomorrow");
      case "yesterday":
        return pick("workbench.todo.dueYesterday");
      case "overdue":
        return `${pick("workbench.todo.overdue")} ${s.days} ${pick("workbench.todo.dueDaysUnit")}`;
      case "in-days":
        return `${s.days} ${pick("workbench.todo.dueDaysSuffix")}`;
      case "absolute":
        return s.yyyyMmDd;
    }
  })();
  return s.kind === "today" || !time ? label : `${label} ${time}`;
}