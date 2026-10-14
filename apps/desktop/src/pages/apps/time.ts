/**
 * Date helpers shared by the workbench apps.
 *
 * Everything works on **local** calendar dates (`YYYY-MM-DD`), never UTC:
 * `toISOString()` would shift the day for anyone east or west of Greenwich.
 * The demo routes pass a frozen date in instead of reading the clock
 * (DEMO_DATA.md), so the three callers take the date they should use.
 */

const pad = (n: number): string => String(n).padStart(2, "0");

/** Local calendar date of a moment, as `YYYY-MM-DD`. */
export function isoDate(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** Today, local. Not used on demo routes. */
export function today(): string {
  return isoDate(new Date());
}

/** The IANA zone this machine runs in, for a new event's default. */
export function systemTimezone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
}
