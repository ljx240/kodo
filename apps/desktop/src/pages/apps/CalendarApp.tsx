/**
 * 日历 — the workbench's second app.
 *
 * Week and month views over one dataset (`listCalendarEvents`). An event
 * stores a local date plus `HH:MM` wall-clock times in its **own** IANA zone,
 * so a 16:00 meeting keeps reading 16:00 wherever the laptop is; the form
 * defaults to the machine's zone and shows the event's zone whenever it
 * differs. Editing an event never touches a todo's due date and never starts
 * a run — 待办截止 is a read-only overlay on top of its own records.
 */

import { CalendarDays, ChevronLeft, ChevronRight, Plus, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  createCalendarEvent,
  deleteCalendarEvent,
  listCalendarEvents,
  listTodos,
  updateCalendarEvent,
  type AppManifestDto,
  type CalendarEventDto,
  type TodoDto,
} from "../../api";
import { T } from "../../i18n";
import { DEMO_TIMEZONE, DEMO_TODAY } from "../../data/workbench";
import { workbenchErrorText } from "./errors";
import { systemTimezone, today as todayIso } from "./time";

const W = T.workbench.calendar;

/** The window the week grid draws; 09:00–18:00 inside it is working time. */
const DAY_START = 8;
const DAY_END = 20;
const WORK_START = 9;
const WORK_END = 18;
const HOUR_PX = 44;

/**
 * Room a week block has for text, in whole lines. A block is a flex column of
 * 11px lines (`.cal-event` in pages.css: 1px gaps, 1px borders, 2px padding), so
 * a 30- or 45-minute block fits one line, a 1-hour block the time as well, and
 * the timezone needs a third, which is what the 1.5h threshold buys.
 */
const EVENT_LINE_PX = 16;
const EVENT_CHROME_PX = 6;
const linesIn = (height: number): number =>
  Math.min(3, Math.max(1, Math.floor((height - EVENT_CHROME_PX + 1) / EVENT_LINE_PX)));

const ZONES = [
  "Asia/Shanghai",
  "Asia/Tokyo",
  "Asia/Singapore",
  "Europe/London",
  "Europe/Berlin",
  "America/New_York",
  "America/Los_Angeles",
  "UTC",
];

const pad = (n: number): string => String(n).padStart(2, "0");

/** Local midnight of a `YYYY-MM-DD`, so arithmetic never crosses a zone. */
function atMidnight(iso: string): Date {
  const [year, month, day] = iso.split("-").map(Number);
  return new Date(year, month - 1, day);
}

function isoOf(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

function shift(iso: string, days: number): string {
  const date = atMidnight(iso);
  date.setDate(date.getDate() + days);
  return isoOf(date);
}

/** Monday of the week `iso` falls in. */
function weekStart(iso: string): string {
  const date = atMidnight(iso);
  const weekday = (date.getDay() + 6) % 7;
  return shift(iso, -weekday);
}

function monthStart(iso: string): string {
  return `${iso.slice(0, 7)}-01`;
}

function shiftMonths(iso: string, months: number): string {
  const date = atMidnight(iso);
  date.setMonth(date.getMonth() + months, 1);
  return isoOf(date);
}

function minutes(time: string | null): number | null {
  if (!time) return null;
  const [hour, minute] = time.split(":").map(Number);
  return hour * 60 + minute;
}

type View = "week" | "month";

type EditorState = {
  /** Existing event, or a blank draft for a new one. */
  draft: CalendarEventDto;
  isNew: boolean;
  error: string | null;
  /** Second click on 删除 confirms. */
  confirmDelete: boolean;
};

type Props = {
  manifest: AppManifestDto;
  demo: boolean;
  onBack: () => void;
};

function blankEvent(date: string, timezone: string, start?: string): CalendarEventDto {
  const startTime = start ?? "09:00";
  return {
    id: "",
    title: "",
    notes: "",
    date,
    startTime,
    endTime: `${pad(Math.min(Number(startTime.slice(0, 2)) + 1, 23))}:${startTime.slice(3)}`,
    allDay: false,
    timezone,
    version: 0,
    createdAt: 0,
    updatedAt: 0,
  };
}

export function CalendarApp({ manifest, demo, onBack }: Props) {
  const today = demo ? DEMO_TODAY : todayIso();
  const zone = demo ? DEMO_TIMEZONE : systemTimezone();

  const [events, setEvents] = useState<CalendarEventDto[] | null>(null);
  const [view, setView] = useState<View>("week");
  const [anchor, setAnchor] = useState(today);
  const [error, setError] = useState<string | null>(null);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [showDue, setShowDue] = useState(false);
  const [dueTodos, setDueTodos] = useState<TodoDto[]>([]);

  const reload = useCallback(async () => {
    const rows = await listCalendarEvents();
    setEvents(rows);
    return rows;
  }, []);

  useEffect(() => {
    reload().then(
      () => setError(null),
      (failure: unknown) => setError(workbenchErrorText(failure)),
    );
  }, [reload]);

  // The due-date overlay is opt-in and reads the todo app's own records: no
  // copy is kept, so a due date can never drift from what 待办 shows.
  useEffect(() => {
    if (!showDue) return;
    void listTodos(true)
      .then((rows) => setDueTodos(rows.filter((row) => !row.archived && row.dueDate)))
      .catch(() => setDueTodos([]));
  }, [showDue]);

  const put = useCallback((next: CalendarEventDto) => {
    setEvents((rows) => {
      const list = rows ?? [];
      return list.some((row) => row.id === next.id)
        ? list.map((row) => (row.id === next.id ? next : row))
        : [...list, next];
    });
  }, []);

  const fail = useCallback(
    (failure: unknown) => {
      setError(workbenchErrorText(failure));
      void reload().catch(() => {});
    },
    [reload],
  );

  // --- layout ---------------------------------------------------------------

  const week = useMemo(
    () => Array.from({ length: 7 }, (_, index) => shift(weekStart(anchor), index)),
    [anchor],
  );

  const monthDays = useMemo(() => {
    const first = monthStart(anchor);
    const start = weekStart(first);
    return Array.from({ length: 42 }, (_, index) => shift(start, index));
  }, [anchor]);

  const byDate = useMemo(() => {
    const map = new Map<string, CalendarEventDto[]>();
    for (const event of events ?? []) {
      const list = map.get(event.date) ?? [];
      list.push(event);
      map.set(event.date, list);
    }
    for (const list of map.values()) {
      list.sort((a, b) => (minutes(a.startTime) ?? -1) - (minutes(b.startTime) ?? -1));
    }
    return map;
  }, [events]);

  const dueByDate = useMemo(() => {
    const map = new Map<string, TodoDto[]>();
    for (const todo of dueTodos) {
      if (!todo.dueDate) continue;
      const list = map.get(todo.dueDate) ?? [];
      list.push(todo);
      map.set(todo.dueDate, list);
    }
    return map;
  }, [dueTodos]);

  const month = Number(anchor.slice(5, 7));
  const heading =
    view === "week"
      ? W.weekRange(week[0].slice(5), week[6].slice(5))
      : W.monthTitle(Number(anchor.slice(0, 4)), month);

  // --- writes ---------------------------------------------------------------

  const save = useCallback(
    (state: EditorState) => {
      const draft = state.draft;
      if (!draft.title.trim()) {
        setEditor((current) => (current ? { ...current, error: W.titleRequired } : current));
        return;
      }
      const start = minutes(draft.startTime);
      const end = minutes(draft.endTime);
      if (!draft.allDay && start !== null && end !== null && end <= start) {
        setEditor((current) => (current ? { ...current, error: W.timeInvalid } : current));
        return;
      }
      // The draft's own zone is what gets stored — `blankEvent` already seeded a
      // new event with the calendar's, and an edit must not re-zone an event the
      // user filed against another one.
      const payload: CalendarEventDto = {
        ...draft,
        title: draft.title.trim(),
        startTime: draft.allDay ? null : draft.startTime,
        endTime: draft.allDay ? null : draft.endTime,
      };
      const request = state.isNew
        ? createCalendarEvent({
            title: payload.title,
            notes: payload.notes,
            date: payload.date,
            startTime: payload.startTime,
            endTime: payload.endTime,
            allDay: payload.allDay,
            timezone: payload.timezone,
          })
        : updateCalendarEvent(draft.version, payload);
      request
        .then((next) => {
          put(next);
          setError(null);
          setEditor(null);
        })
        .catch((failure: unknown) => {
          const code = workbenchErrorText(failure);
          setEditor((current) => (current ? { ...current, error: code } : current));
          if (!state.isNew) {
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
          }
        });
    },
    [put, reload],
  );

  const remove = useCallback(
    (event: CalendarEventDto) => {
      void deleteCalendarEvent(event.id, event.version)
        .then(() => {
          setEvents((rows) => (rows ?? []).filter((row) => row.id !== event.id));
          setError(null);
          setEditor(null);
        })
        .catch(fail);
    },
    [fail],
  );

  const openNew = (date: string, start?: string) =>
    setEditor({
      draft: blankEvent(date, zone, start),
      isNew: true,
      error: null,
      confirmDelete: false,
    });

  const openEdit = (event: CalendarEventDto) =>
    setEditor({ draft: { ...event }, isNew: false, error: null, confirmDelete: false });

  // --- pieces ---------------------------------------------------------------

  const dueLine = (date: string) => {
    const due = dueByDate.get(date) ?? [];
    if (!showDue || due.length === 0) return null;
    return (
      <ul className="cal-due" data-testid={`cal-due-${date}`}>
        {due.slice(0, 2).map((todo) => (
          <li
            key={todo.id}
            className={`cal-due-item${todo.isDone ? " cal-due-item--done" : ""}`}
          >
            {W.dueLabel} · {todo.title}
          </li>
        ))}
        {due.length > 2 && <li className="cal-due-item">{W.moreEvents(due.length - 2)}</li>}
      </ul>
    );
  };

  const eventBlock = (event: CalendarEventDto) => {
    const start = minutes(event.startTime) ?? DAY_START * 60;
    const end = minutes(event.endTime) ?? start + 60;
    const top = Math.max(0, ((start - DAY_START * 60) / 60) * HOUR_PX);
    const height = Math.max(
      // A short event still gets one whole line; below that the block would clip
      // the label it always shows.
      EVENT_CHROME_PX + EVENT_LINE_PX,
      Math.min(
        ((end - start) / 60) * HOUR_PX,
        (((DAY_END - DAY_START) * 60 - (start - DAY_START * 60)) / 60) * HOUR_PX,
      ),
    );
    const foreign = event.timezone !== zone;
    const lines = linesIn(height);
    return (
      <button
        key={event.id}
        type="button"
        className="cal-event"
        style={{ top, height }}
        title={
          foreign ? `${event.startTime} ${event.timezone}` : `${event.startTime} – ${event.endTime}`
        }
        data-testid={`cal-event-${event.id}`}
        onClick={() => openEdit(event)}
      >
        {/* The title is the one line a block always carries: the grid's rows
            already say when, and a 45-minute block has room for one line only,
            never for the time and the title together. */}
        {lines >= 2 && <span className="cal-event-time">{event.startTime}</span>}
        <span className="cal-event-title">{event.title}</span>
        {lines >= 3 && foreign && <span className="cal-event-zone">{event.timezone}</span>}
      </button>
    );
  };

  const allDayRow = (days: string[]) => {
    const rows: React.ReactNode[] = [];
    for (const day of days) {
      const allDay = (byDate.get(day) ?? []).filter((event) => event.allDay);
      if (allDay.length === 0) continue;
      rows.push(
        <div key={day} className="cal-allday-cell" data-testid={`cal-allday-${day}`}>
          {allDay.map((event) => (
            <button
              key={event.id}
              type="button"
              className="cal-allday-event"
              data-testid={`cal-event-${event.id}`}
              onClick={() => openEdit(event)}
            >
              {event.title}
            </button>
          ))}
        </div>,
      );
    }
    if (rows.length === 0) return null;
    return (
      <div className="cal-allday">
        <span className="cal-gutter-label">{W.allDay}</span>
        <div className="cal-allday-grid">{rows}</div>
      </div>
    );
  };

  const hours = Array.from({ length: DAY_END - DAY_START }, (_, index) => DAY_START + index);

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
            {(["week", "month"] as const).map((value) => (
              <button
                key={value}
                type="button"
                className={`app-seg-btn${view === value ? " app-seg-btn--active" : ""}`}
                aria-pressed={view === value}
                onClick={() => setView(value)}
                data-testid={`cal-view-${value}`}
              >
                {value === "week" ? W.week : W.month}
              </button>
            ))}
          </div>

          <div className="cal-nav">
            <button
              type="button"
              className="icon-btn icon-btn--sm"
              title={W.prev}
              aria-label={W.prev}
              onClick={() =>
                setAnchor((current) =>
                  view === "week" ? shift(current, -7) : shiftMonths(current, -1),
                )
              }
            >
              <ChevronLeft size={15} strokeWidth={1.8} />
            </button>
            <button
              type="button"
              className="btn"
              onClick={() => setAnchor(today)}
              data-testid="cal-today"
            >
              {W.today}
            </button>
            <button
              type="button"
              className="icon-btn icon-btn--sm"
              title={W.next}
              aria-label={W.next}
              onClick={() =>
                setAnchor((current) =>
                  view === "week" ? shift(current, 7) : shiftMonths(current, 1),
                )
              }
            >
              <ChevronRight size={15} strokeWidth={1.8} />
            </button>
            <span className="cal-heading" data-testid="cal-heading">
              {heading}
            </span>
          </div>

          <span className="spacer" />

          <button
            type="button"
            className={`app-filter${showDue ? " app-filter--active" : ""}`}
            aria-pressed={showDue}
            onClick={() => setShowDue((on) => !on)}
            data-testid="cal-due-toggle"
          >
            {W.showDueTodos}
          </button>

          <span className="cal-zone" title={W.timezoneLabel}>
            {W.timezoneLabel} {zone}
          </span>

          <button
            type="button"
            className="btn btn--primary"
            onClick={() => openNew(anchor)}
            data-testid="cal-new"
          >
            <Plus size={14} strokeWidth={1.8} />
            {W.newEvent}
          </button>
        </div>

        {view === "week" ? (
          <div className="cal-week" data-testid="cal-week">
            <div className="cal-week-head">
              <span className="cal-gutter-label" />
              {week.map((day, index) => (
                <div
                  key={day}
                  className={`cal-day-head${day === today ? " cal-day-head--today" : ""}`}
                  data-testid={`cal-day-head-${day}`}
                >
                  {W.weekdays[index]}
                  <span className="cal-day-num">{day.slice(5)}</span>
                </div>
              ))}
            </div>

            {allDayRow(week)}

            <div className="cal-grid">
              <div className="cal-gutter">
                {hours.map((hour) => (
                  <div key={hour} className="cal-hour-label">
                    {pad(hour)}:00
                  </div>
                ))}
              </div>
              {week.map((day) => (
                <div key={day} className="cal-day-col" data-testid={`cal-day-${day}`}>
                  {hours.map((hour) => (
                    <button
                      key={hour}
                      type="button"
                      className={`cal-slot${hour >= WORK_START && hour < WORK_END ? " cal-slot--work" : ""}`}
                      aria-label={`${day} ${pad(hour)}:00`}
                      onClick={() => openNew(day, `${pad(hour)}:00`)}
                    />
                  ))}
                  {(byDate.get(day) ?? []).filter((event) => !event.allDay).map(eventBlock)}
                  {dueLine(day)}
                </div>
              ))}
            </div>
          </div>
        ) : (
          <div className="cal-month" data-testid="cal-month">
            <div className="cal-month-head">
              {W.weekdays.map((label) => (
                <span key={label} className="cal-month-weekday">
                  {label}
                </span>
              ))}
            </div>
            <div className="cal-month-grid">
              {monthDays.map((day) => {
                const dayEvents = byDate.get(day) ?? [];
                return (
                  <div
                    key={day}
                    className={`cal-month-cell${day.slice(0, 7) === anchor.slice(0, 7) ? "" : " cal-month-cell--outside"}${
                      day === today ? " cal-month-cell--today" : ""
                    }`}
                    data-testid={`cal-cell-${day}`}
                  >
                    <button
                      type="button"
                      className="cal-month-day"
                      aria-label={W.dayTitle(Number(day.slice(5, 7)), Number(day.slice(8, 10)))}
                      onClick={() => openNew(day)}
                    >
                      {Number(day.slice(8, 10))}
                    </button>
                    <ul className="cal-month-events">
                      {dayEvents.slice(0, 3).map((event) => (
                        <li key={event.id}>
                          <button
                            type="button"
                            className={`cal-month-event${event.allDay ? " cal-month-event--allday" : ""}`}
                            data-testid={`cal-event-${event.id}`}
                            onClick={() => openEdit(event)}
                          >
                            {event.allDay ? W.allDay : event.startTime} {event.title}
                          </button>
                        </li>
                      ))}
                      {dayEvents.length > 3 && (
                        <li className="cal-month-more">{W.moreEvents(dayEvents.length - 3)}</li>
                      )}
                    </ul>
                    {showDue && (dueByDate.get(day) ?? []).length > 0 && (
                      <span className="cal-month-due" data-testid={`cal-month-due-${day}`}>
                        {W.dueLabel} {(dueByDate.get(day) ?? []).length}
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>

      {editor && (
        <div
          className="settings-modal"
          role="dialog"
          aria-modal="true"
          aria-label={editor.isNew ? W.newEvent : W.editEvent}
        >
          <div className="settings-modal-backdrop" onClick={() => setEditor(null)} />
          <div className="settings-modal-panel wb-modal-panel">
            <header className="modal-head">
              <CalendarDays size={15} strokeWidth={1.8} />
              <span className="modal-title">{editor.isNew ? W.newEvent : W.editEvent}</span>
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
                  autoFocus
                  value={editor.draft.title}
                  aria-label={W.titleLabel}
                  data-testid="cal-edit-title"
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
                  <span className="field-label">{W.date}</span>
                  <input
                    type="date"
                    className="input"
                    value={editor.draft.date}
                    aria-label={W.date}
                    data-testid="cal-edit-date"
                    onChange={(event) =>
                      setEditor((current) =>
                        current
                          ? { ...current, draft: { ...current.draft, date: event.target.value } }
                          : current,
                      )
                    }
                  />
                </label>

                <label className="field app-check">
                  <span className="field-label">{W.allDay}</span>
                  <input
                    type="checkbox"
                    checked={editor.draft.allDay}
                    aria-label={W.allDay}
                    data-testid="cal-edit-allday"
                    onChange={(event) =>
                      setEditor((current) =>
                        current
                          ? {
                              ...current,
                              draft: { ...current.draft, allDay: event.target.checked },
                            }
                          : current,
                      )
                    }
                  />
                </label>
              </div>

              {!editor.draft.allDay && (
                <div className="app-form-row">
                  <label className="field">
                    <span className="field-label">{W.start}</span>
                    <input
                      type="time"
                      className="input"
                      value={editor.draft.startTime ?? ""}
                      aria-label={W.start}
                      data-testid="cal-edit-start"
                      onChange={(event) =>
                        setEditor((current) =>
                          current
                            ? {
                                ...current,
                                draft: { ...current.draft, startTime: event.target.value },
                              }
                            : current,
                        )
                      }
                    />
                  </label>
                  <label className="field">
                    <span className="field-label">{W.end}</span>
                    <input
                      type="time"
                      className="input"
                      value={editor.draft.endTime ?? ""}
                      aria-label={W.end}
                      data-testid="cal-edit-end"
                      onChange={(event) =>
                        setEditor((current) =>
                          current
                            ? {
                                ...current,
                                draft: { ...current.draft, endTime: event.target.value },
                              }
                            : current,
                        )
                      }
                    />
                  </label>
                </div>
              )}

              <label className="field field--wide">
                <span className="field-label">{W.timezone}</span>
                <select
                  className="select"
                  value={editor.draft.timezone}
                  aria-label={W.timezone}
                  data-testid="cal-edit-timezone"
                  onChange={(event) =>
                    setEditor((current) =>
                      current
                        ? { ...current, draft: { ...current.draft, timezone: event.target.value } }
                        : current,
                    )
                  }
                >
                  {[...new Set([zone, editor.draft.timezone, ...ZONES])].map((value) => (
                    <option key={value} value={value}>
                      {value === zone ? `${value} (${W.timezoneSystem})` : value}
                    </option>
                  ))}
                </select>
              </label>

              <label className="field field--wide">
                <span className="field-label">{W.notes}</span>
                <textarea
                  className="input todo-notes"
                  rows={3}
                  value={editor.draft.notes}
                  aria-label={W.notes}
                  data-testid="cal-edit-notes"
                  onChange={(event) =>
                    setEditor((current) =>
                      current
                        ? { ...current, draft: { ...current.draft, notes: event.target.value } }
                        : current,
                    )
                  }
                />
              </label>

              <p className="app-hint">{W.todoWriteHint}</p>
            </div>

            <footer className="skill-detail-foot">
              {!editor.isNew && (
                <button
                  type="button"
                  className={`btn${editor.confirmDelete ? " btn--danger" : ""}`}
                  data-testid="cal-edit-delete"
                  onClick={() => {
                    if (!editor.confirmDelete) {
                      setEditor((current) =>
                        current ? { ...current, confirmDelete: true } : current,
                      );
                      return;
                    }
                    remove(editor.draft);
                  }}
                >
                  <Trash2 size={13} strokeWidth={1.8} />
                  {editor.confirmDelete ? W.deleteConfirm : W.delete}
                </button>
              )}
              <span className="spacer" />
              <button type="button" className="btn" onClick={() => setEditor(null)}>
                {W.cancel}
              </button>
              <button
                type="button"
                className="btn btn--primary"
                data-testid="cal-edit-save"
                onClick={() => save(editor)}
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
