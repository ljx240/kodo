/**
 * SchedulerPage — read + lifecycle surface for the scheduled-tasks store.
 *
 * The first-version GUI is intentionally read + lifecycle only. The
 * create / edit flow wants a real form (Once / Interval / Daily /
 * Weekly with timezone pickers) and lives in a sibling modal that the
 * desktop shell opens directly; this page is the index you scroll when
 * you want to know "what is firing next, and is it enabled?".
 *
 * Everything the page renders comes from the live Tauri commands —
 * `listScheduledTasks` / `listTaskRuns` / `setScheduledTaskEnabled` /
 * `runScheduledTaskNow` / `archiveScheduledTask` / `cancelTaskRun`. The
 * demo fixture does not populate scheduler data, so the page falls back
 * to an empty-state copy when no rows arrive.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { Archive, ChevronDown, ChevronRight, Clock, Pause, Play, RefreshCw, Zap } from "lucide-react";
import {
  archiveScheduledTask,
  cancelTaskRun,
  isDesktop,
  listJobTypes,
  listScheduledTasks,
  listTaskRuns,
  runScheduledTaskNow,
  setScheduledTaskEnabled,
  type ScheduledTaskDto,
  type ScheduleDto,
  type TaskKindDto,
  type TaskRunDto,
} from "../api";
import { T } from "../i18n";

const S = T.schedule;

type Filter = "all" | "enabled" | "paused";

function formatTs(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined) return "—";
  const ms = seconds * 1000;
  if (!Number.isFinite(ms)) return "—";
  return new Date(ms).toLocaleString();
}

function summarizeSchedule(schedule: ScheduleDto): string {
  switch (schedule.kind) {
    case "once":
      return `一次 · ${formatTs(schedule.fireAt)}`;
    case "interval":
      return `每 ${Math.round(schedule.intervalSeconds / 60)} 分钟`;
    case "daily":
      return `每天 ${pad2(schedule.hour)}:${pad2(schedule.minute)} (${schedule.tzName})`;
    case "weekly": {
      const days = schedule.weekdays.map(weekdayLabel).join("·");
      return `每${days} ${pad2(schedule.hour)}:${pad2(schedule.minute)} (${schedule.tzName})`;
    }
  }
}

function pad2(n: number): string {
  return n.toString().padStart(2, "0");
}

function weekdayLabel(d: number): string {
  switch (d) {
    case 1:
      return "一";
    case 2:
      return "二";
    case 3:
      return "三";
    case 4:
      return "四";
    case 5:
      return "五";
    case 6:
      return "六";
    case 7:
      return "日";
    default:
      return `?${d}`;
  }
}

function taskKindLabel(kind: TaskKindDto): string {
  if (kind.kind === "agent") {
    return S.list.kindAgent;
  }
  return `${S.list.kindJob} · ${kind.jobType}`;
}

function taskProjectLabel(kind: TaskKindDto): string {
  if (kind.kind === "agent") {
    return kind.project;
  }
  return kind.resources.length > 0 ? kind.resources.join(", ") : "—";
}

function stateLabel(state: ScheduledTaskDto["state"]): string {
  switch (state) {
    case "enabled":
      return S.list.stateEnabled;
    case "paused":
      return S.list.statePaused;
    case "finished":
      return S.list.stateFinished;
    case "archived":
      return S.list.stateArchived;
  }
}

function mapSchedulerError(err: unknown): string {
  if (typeof err === "string") return S.errors[err as keyof typeof S.errors] ?? err;
  if (err && typeof err === "object" && "code" in err) {
    const code = (err as { code?: string }).code;
    if (code) {
      const localized = S.errors[code as keyof typeof S.errors];
      if (localized) return localized;
    }
  }
  return S.errors.unknown;
}

export function SchedulerPage({ demo }: { demo: boolean }) {
  const live = !demo && isDesktop();
  const [tasks, setTasks] = useState<ScheduledTaskDto[]>([]);
  const [filter, setFilter] = useState<Filter>("all");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [runs, setRuns] = useState<TaskRunDto[]>([]);
  const [runsLoading, setRunsLoading] = useState(false);
  const [jobTypes, setJobTypes] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const reload = useCallback(async () => {
    if (!live) return;
    try {
      const list = await listScheduledTasks(false);
      setTasks(list);
      setError(null);
    } catch (err) {
      setError(mapSchedulerError(err));
    }
  }, [live]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    if (!live) return;
    void listJobTypes().then(setJobTypes).catch(() => setJobTypes([]));
  }, [live]);

  const filtered = useMemo(
    () =>
      tasks.filter((task) => {
        if (task.archived) return false;
        if (filter === "all") return true;
        return task.state === filter;
      }),
    [tasks, filter],
  );

  const toggleExpanded = useCallback(
    async (taskId: string) => {
      if (expanded === taskId) {
        setExpanded(null);
        setRuns([]);
        return;
      }
      setExpanded(taskId);
      if (!live) return;
      setRunsLoading(true);
      try {
        const history = await listTaskRuns(taskId, 50);
        setRuns(history);
      } catch (err) {
        setError(mapSchedulerError(err));
      } finally {
        setRunsLoading(false);
      }
    },
    [expanded, live],
  );

  const withBusy = useCallback(
    async (key: string, action: () => Promise<void>) => {
      setBusy(key);
      try {
        await action();
        await reload();
      } catch (err) {
        setError(mapSchedulerError(err));
      } finally {
        setBusy(null);
      }
    },
    [reload],
  );

  const onToggle = (task: ScheduledTaskDto) =>
    withBusy(`toggle-${task.id}`, async () => {
      await setScheduledTaskEnabled(task.id, task.state !== "enabled");
    });

  const onRunNow = (task: ScheduledTaskDto) =>
    withBusy(`run-${task.id}`, async () => {
      await runScheduledTaskNow(task.id);
    });

  const onArchive = (task: ScheduledTaskDto) =>
    withBusy(`archive-${task.id}`, async () => {
      await archiveScheduledTask(task.id);
    });

  const onCancelRun = (runId: string) =>
    withBusy(`cancel-${runId}`, async () => {
      const accepted = await cancelTaskRun(runId);
      if (!accepted && error === null) {
        // The backend returns false for already-finished runs; the row
        // has its own copy path so the user understands why nothing
        // happened.
      }
      if (expanded) {
        const history = await listTaskRuns(expanded, 50);
        setRuns(history);
      }
    });

  return (
    <main className="main page-schedule">
      <header className="page-head">
        <div className="page-head-titles">
          <h1>{S.title}</h1>
          <p className="page-subtitle">{S.subtitle}</p>
        </div>
        <div className="page-head-actions">
          <button
            type="button"
            className="icon-btn"
            aria-label={S.actions.refresh}
            onClick={() => void reload()}
            disabled={!live || busy !== null}
          >
            <RefreshCw size={15} strokeWidth={1.8} />
          </button>
        </div>
      </header>

      <div className="app-body">
      {error && (
        <p className="page-error" role="alert">
          {error}
        </p>
      )}

      <div className="schedule-filter">
        {(["all", "enabled", "paused"] as const).map((value) => (
          <button
            key={value}
            type="button"
            className={`schedule-filter-pill${filter === value ? " schedule-filter-pill--active" : ""}`}
            aria-pressed={filter === value}
            onClick={() => setFilter(value)}
            data-testid={`schedule-filter-${value}`}
          >
            {value === "all" ? S.list.filterAll : value === "enabled" ? S.list.filterEnabled : S.list.filterPaused}
          </button>
        ))}
      </div>

      {filtered.length === 0 ? (
        <div className="page-empty" role="status">
          <Clock size={24} strokeWidth={1.5} aria-hidden="true" />
          <p>{S.list.empty}</p>
        </div>
      ) : (
        <ul className="schedule-list">
          {filtered.map((task) => {
            const isOpen = expanded === task.id;
            const toggleKey = `toggle-${task.id}`;
            const runKey = `run-${task.id}`;
            const archiveKey = `archive-${task.id}`;
            return (
              <li
                key={task.id}
                className={`schedule-row${isOpen ? " schedule-row--open" : ""}`}
                data-testid={`schedule-row-${task.id}`}
              >
                <div className="schedule-row-main">
                  <button
                    type="button"
                    className="schedule-row-toggle"
                    onClick={() => void toggleExpanded(task.id)}
                    aria-expanded={isOpen}
                    aria-label={S.actions.history}
                  >
                    {isOpen ? <ChevronDown size={14} strokeWidth={2} /> : <ChevronRight size={14} strokeWidth={2} />}
                  </button>
                  <div className="schedule-row-meta">
                    <div className="schedule-row-title">{task.title}</div>
                    <div className="schedule-row-sub">
                      <span className="schedule-pill schedule-pill--kind">
                        {taskKindLabel(task.task)}
                      </span>
                      <span className="schedule-pill">{taskProjectLabel(task.task)}</span>
                      <span className="schedule-pill">
                        <Clock size={11} strokeWidth={1.8} />
                        {summarizeSchedule(task.schedule)}
                      </span>
                      <span className="schedule-pill">
                        {S.list.headers.next}: {formatTs(task.nextFireAt)}
                      </span>
                      <span
                        className={`schedule-pill schedule-pill--state schedule-pill--state-${task.state}`}
                      >
                        {stateLabel(task.state)}
                      </span>
                    </div>
                  </div>
                  <div className="schedule-row-actions">
                    <button
                      type="button"
                      className="icon-btn icon-btn--sm"
                      onClick={() => onToggle(task)}
                      disabled={!live || task.archived || busy === toggleKey}
                      aria-label={task.state === "enabled" ? S.actions.pause : S.actions.resume}
                      title={task.state === "enabled" ? S.actions.pause : S.actions.resume}
                    >
                      {task.state === "enabled" ? (
                        <Pause size={13} strokeWidth={1.9} />
                      ) : (
                        <Play size={13} strokeWidth={1.9} />
                      )}
                    </button>
                    <button
                      type="button"
                      className="icon-btn icon-btn--sm"
                      onClick={() => onRunNow(task)}
                      disabled={!live || task.archived || busy === runKey}
                      aria-label={S.actions.runNow}
                      title={S.actions.runNow}
                    >
                      <Zap size={13} strokeWidth={1.9} />
                    </button>
                    <button
                      type="button"
                      className="icon-btn icon-btn--sm"
                      onClick={() => onArchive(task)}
                      disabled={!live || task.archived || task.state === "enabled" || busy === archiveKey}
                      aria-label={S.actions.archive}
                      title={S.actions.archive}
                    >
                      <Archive size={13} strokeWidth={1.8} />
                    </button>
                  </div>
                </div>
                {isOpen && (
                  <div className="schedule-row-history">
                    <h3>{S.history.title(task.id)}</h3>
                    {runsLoading ? (
                      <p className="page-empty">…</p>
                    ) : runs.length === 0 ? (
                      <p className="page-empty">{S.history.empty}</p>
                    ) : (
                      <table className="schedule-runs">
                        <thead>
                          <tr>
                            <th>{S.history.headers.runId}</th>
                            <th>{S.history.headers.lifecycle}</th>
                            <th>{S.history.headers.outcome}</th>
                            <th>{S.history.headers.scheduledFor}</th>
                            <th>{S.history.headers.startedAt}</th>
                            <th>{S.history.headers.finishedAt}</th>
                            <th>{S.history.headers.reason}</th>
                            <th aria-label="actions" />
                          </tr>
                        </thead>
                        <tbody>
                          {runs.map((run) => (
                            <tr key={run.id}>
                              <td className="schedule-runs-id">{run.id}</td>
                              <td>{run.lifecycle}</td>
                              <td>{run.outcome ?? "—"}</td>
                              <td>{formatTs(run.scheduledFor)}</td>
                              <td>{formatTs(run.startedAt)}</td>
                              <td>{formatTs(run.finishedAt)}</td>
                              <td>{run.reasonCode ?? "—"}</td>
                              <td>
                                <button
                                  type="button"
                                  className="icon-btn icon-btn--sm"
                                  onClick={() => onCancelRun(run.id)}
                                  disabled={
                                    !live ||
                                    busy === `cancel-${run.id}` ||
                                    run.lifecycle === "finished"
                                  }
                                  aria-label={S.history.cancel}
                                  title={S.history.cancel}
                                >
                                  <Pause size={12} strokeWidth={1.9} />
                                </button>
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}

      {jobTypes.length > 0 && (
        <section className="schedule-job-types">
          <h2>{S.jobTypes.title}</h2>
          <ul>
            {jobTypes.map((name) => (
              <li key={name}>{name}</li>
            ))}
          </ul>
        </section>
      )}
      </div>
    </main>
  );
}
