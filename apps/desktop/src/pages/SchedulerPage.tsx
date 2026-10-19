/** Task configuration and run history share the existing persistent scheduler. */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Archive,
  ChevronDown,
  ChevronRight,
  Clock,
  Pencil,
  Pause,
  Play,
  Plus,
  RefreshCw,
  Zap,
} from "lucide-react";
import {
  archiveScheduledTask,
  cancelTaskRun,
  isDesktop,
  listJobTypes,
  listScheduledTasks,
  listTaskRuns,
  runScheduledTaskNow,
  setScheduledTaskEnabled,
  type ProjectDto,
  type ScheduleDto,
  type ScheduledTaskDto,
  type TaskRunDto,
} from "../api";
import { T } from "../i18n";
import { SchedulerEditor, TASK_TEMPLATES, type TaskTemplate } from "./SchedulerEditor";
import {
  formatScheduleTime,
  JOB_NAMES,
  OUTCOMES,
  RUN_STATES,
  schedulerError,
  WEEKDAYS,
} from "./schedulerModel";
import "./scheduler.css";

const S = T.schedule;
type Filter = "all" | ScheduledTaskDto["state"];
const FILTERS: Record<Filter, string> = {
  all: "全部",
  enabled: "启用",
  paused: "暂停",
  finished: "已结束",
  archived: "已归档",
};

function summarizeSchedule(task: ScheduledTaskDto): string {
  const schedule: ScheduleDto = task.schedule;
  const clock = (hour: number, minute: number) =>
    `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`;
  switch (schedule.kind) {
    case "once":
      return `一次 · ${formatScheduleTime(schedule.fireAt, task.timezone)}`;
    case "interval":
      return `每 ${schedule.intervalSeconds / 60} 分钟`;
    case "daily":
      return `每天 ${clock(schedule.hour, schedule.minute)}`;
    case "weekly":
      return `每周${schedule.weekdays.map((d) => WEEKDAYS[d - 1]).join("、")} ${clock(schedule.hour, schedule.minute)}`;
  }
}

export function SchedulerPage({
  demo,
  projects = [],
  onOpenConversation,
}: {
  demo: boolean;
  projects?: ProjectDto[];
  onOpenConversation?: (id: string) => Promise<void>;
}) {
  const live = !demo && isDesktop();
  const [tasks, setTasks] = useState<ScheduledTaskDto[]>([]);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState("all");
  const [project, setProject] = useState("all");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [history, setHistory] = useState<{ taskId: string; rows: TaskRunDto[] } | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [runsLoading, setRunsLoading] = useState(false);
  const [jobTypes, setJobTypes] = useState<string[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [loading, setLoading] = useState(live);
  const [busy, setBusy] = useState<string | null>(null);
  const busyRef = useRef(false);
  const mounted = useRef(false);
  const listRequest = useRef(0);
  const [refreshToken, setRefreshToken] = useState(0);
  const [eventsUnavailable, setEventsUnavailable] = useState(false);
  const [editor, setEditor] = useState<{
    task: ScheduledTaskDto | null;
    template?: TaskTemplate;
  } | null>(null);

  const reload = useCallback(async () => {
    if (!live) return;
    const request = ++listRequest.current;
    try {
      const rows = await listScheduledTasks(true);
      if (!mounted.current || request !== listRequest.current) return;
      setTasks(rows);
      setLoadError(null);
      setRefreshToken((value) => value + 1);
    } catch (failure) {
      if (mounted.current && request === listRequest.current) setLoadError(schedulerError(failure));
    } finally {
      if (mounted.current && request === listRequest.current) setLoading(false);
    }
  }, [live]);

  useEffect(() => {
    mounted.current = true;
    if (!live)
      return () => {
        mounted.current = false;
      };
    let timer: number | undefined;
    const scheduleReload = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => void reload(), 80);
    };
    const refreshVisible = () => {
      if (!document.hidden) scheduleReload();
    };
    const subscriptions = ["scheduler:task_changed", "scheduler:run_changed"].map((event) =>
      listen(event, scheduleReload)
        .then((stop) => {
          if (!mounted.current) stop();
          return stop;
        })
        .catch(() => {
          if (mounted.current) setEventsUnavailable(true);
          return () => {};
        }),
    );
    // Snapshot after subscribing also covers an event during startup. Focus and
    // visible-page polling recover missed events and changes from the CLI.
    void Promise.all(subscriptions).then(() => {
      if (mounted.current) void reload();
    });
    void reload();
    void listJobTypes()
      .then((rows) => {
        if (mounted.current) setJobTypes(rows);
      })
      .catch((failure) => {
        if (mounted.current) setError(schedulerError(failure));
      });
    const interval = window.setInterval(refreshVisible, 5000);
    window.addEventListener("focus", refreshVisible);
    document.addEventListener("visibilitychange", refreshVisible);
    return () => {
      mounted.current = false;
      listRequest.current += 1;
      window.clearTimeout(timer);
      window.clearInterval(interval);
      window.removeEventListener("focus", refreshVisible);
      document.removeEventListener("visibilitychange", refreshVisible);
      void Promise.all(subscriptions).then((stops) => stops.forEach((stop) => stop()));
    };
  }, [live, reload]);

  useEffect(() => {
    if (!expanded || !live) return;
    let active = true;
    setRunsLoading(true);
    setHistoryError(null);
    void listTaskRuns(expanded, 50)
      .then((rows) => {
        if (active) setHistory({ taskId: expanded, rows });
      })
      .catch((failure) => {
        if (active) setHistoryError(schedulerError(failure));
      })
      .finally(() => {
        if (active) setRunsLoading(false);
      });
    return () => {
      active = false;
    };
  }, [expanded, live, refreshToken]);

  const filtered = useMemo(
    () =>
      tasks.filter((task) => {
        if (filter === "archived" ? !task.archived : task.archived) return false;
        if (filter !== "all" && filter !== "archived" && task.state !== filter) return false;
        if (kind !== "all" && task.task.kind !== kind) return false;
        if (project !== "all" && (task.task.kind !== "agent" || task.task.project !== project))
          return false;
        const target =
          task.task.kind === "agent"
            ? `${task.task.project} ${task.task.message}`
            : (JOB_NAMES[task.task.jobType] ?? task.task.jobType);
        return `${task.title} ${target}`.toLowerCase().includes(query.trim().toLowerCase());
      }),
    [tasks, filter, query, kind, project],
  );
  const projectPaths = [
    ...new Set(tasks.flatMap((task) => (task.task.kind === "agent" ? [task.task.project] : []))),
  ];

  const withBusy = async (key: string, action: () => Promise<void>) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(key);
    setError(null);
    setNotice(null);
    try {
      await action();
      await reload();
    } catch (failure) {
      if (mounted.current) setError(schedulerError(failure));
    } finally {
      busyRef.current = false;
      if (mounted.current) setBusy(null);
    }
  };

  const onSaved = (saved: ScheduledTaskDto) => {
    setTasks((rows) => [saved, ...rows.filter((row) => row.id !== saved.id)]);
    setEditor(null);
    setFilter("all");
    setKind("all");
    setProject("all");
    setQuery("");
    setNotice("任务已保存。");
    setError(null);
    void reload();
  };
  const emptyWorkspace =
    tasks.length === 0 &&
    filter === "all" &&
    !query &&
    kind === "all" &&
    project === "all" &&
    !loadError;

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
            disabled={!live || busy !== null}
            onClick={() => void reload()}
          >
            <RefreshCw size={15} />
          </button>
          <button
            type="button"
            className="btn btn--primary"
            disabled={!live || busy !== null}
            onClick={() => setEditor({ task: null })}
          >
            <Plus size={15} />
            新建任务
          </button>
        </div>
      </header>
      <div className="app-body">
        <p className="schedule-hint">
          仅在 Kodo 运行期间执行。先用「立即执行」试跑，再检查运行结果。
        </p>
        {!live && <p className="schedule-hint">请在桌面端 Kodo 中创建和管理定时任务。</p>}
        {(error || loadError) && (
          <p className="page-error" role="alert">
            {error || loadError}
          </p>
        )}
        {notice && (
          <p className="schedule-notice" role="status">
            {notice}
          </p>
        )}
        {eventsUnavailable && (
          <p className="schedule-hint">实时通知暂不可用，页面会定期同步，也可手动刷新。</p>
        )}
        <div className="schedule-filter" aria-label="任务状态">
          {(Object.keys(FILTERS) as Filter[]).map((value) => (
            <button
              type="button"
              key={value}
              className={`schedule-filter-pill${filter === value ? " schedule-filter-pill--active" : ""}`}
              aria-pressed={filter === value}
              data-testid={`schedule-filter-${value}`}
              onClick={() => setFilter(value)}
            >
              {FILTERS[value]}
            </button>
          ))}
        </div>
        <div className="schedule-toolbar">
          <input
            className="input"
            aria-label="搜索定时任务"
            placeholder="搜索名称、项目或执行内容"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <select
            className="input"
            aria-label="筛选执行类型"
            value={kind}
            onChange={(e) => setKind(e.target.value)}
          >
            <option value="all">所有类型</option>
            <option value="agent">Agent 任务</option>
            <option value="job">后台作业</option>
          </select>
          <select
            className="input"
            aria-label="筛选项目"
            value={project}
            onChange={(e) => setProject(e.target.value)}
          >
            <option value="all">所有项目</option>
            {projectPaths.map((path) => (
              <option key={path} value={path}>
                {projects.find((p) => p.path === path)?.name ?? path}
              </option>
            ))}
          </select>
        </div>
        {loading ? (
          <p className="page-empty" role="status">
            正在加载定时任务…
          </p>
        ) : filtered.length === 0 ? (
          <div className="page-empty" role="status">
            <Clock size={24} aria-hidden="true" />
            <p>{emptyWorkspace ? S.list.empty : "没有符合条件的任务。"}</p>
            {live && emptyWorkspace && (
              <button type="button" className="btn" onClick={() => setEditor({ task: null })}>
                创建第一个任务
              </button>
            )}
          </div>
        ) : (
          <ul className="schedule-list">
            {filtered.map((task) => {
              const open = expanded === task.id;
              const rows = history?.taskId === task.id ? history.rows : [];
              const taskProject = task.task.kind === "agent" ? task.task.project : null;
              const timezone =
                task.schedule.kind === "daily" || task.schedule.kind === "weekly"
                  ? task.schedule.tzName
                  : task.timezone;
              return (
                <li
                  key={task.id}
                  className={`schedule-row${open ? " schedule-row--open" : ""}`}
                  data-testid={`schedule-row-${task.id}`}
                >
                  <div className="schedule-row-main">
                    <button
                      type="button"
                      className="schedule-row-toggle"
                      aria-expanded={open}
                      aria-label={`${S.actions.history} · ${task.title}`}
                      onClick={() => {
                        setExpanded(open ? null : task.id);
                        setHistoryError(null);
                      }}
                    >
                      {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                    </button>
                    <div className="schedule-row-meta">
                      <div className="schedule-row-title">{task.title}</div>
                      <div className="schedule-row-sub">
                        <span className="schedule-pill schedule-pill--kind">
                          {task.task.kind === "agent"
                            ? S.list.kindAgent
                            : (JOB_NAMES[task.task.jobType] ?? task.task.jobType)}
                        </span>
                        {taskProject && (
                          <span className="schedule-pill" title={taskProject}>
                            {projects.find((p) => p.path === taskProject)?.name ?? taskProject}
                          </span>
                        )}
                        <span className="schedule-pill">
                          <Clock size={11} />
                          {summarizeSchedule(task)} · {timezone}
                        </span>
                        <span className="schedule-pill">
                          {S.list.headers.next}: {formatScheduleTime(task.nextFireAt, timezone)}
                        </span>
                        <span
                          className={`schedule-pill schedule-pill--state schedule-pill--state-${task.state}`}
                        >
                          {FILTERS[task.state]}
                        </span>
                      </div>
                    </div>
                    <div className="schedule-row-actions">
                      <button
                        type="button"
                        className="icon-btn icon-btn--sm"
                        aria-label={`编辑 · ${task.title}`}
                        title="编辑"
                        disabled={!live || task.archived || busy !== null}
                        onClick={() => setEditor({ task })}
                      >
                        <Pencil size={13} />
                      </button>
                      <button
                        type="button"
                        className="icon-btn icon-btn--sm"
                        aria-label={task.state === "enabled" ? S.actions.pause : S.actions.resume}
                        title={
                          task.state === "finished"
                            ? "一次任务已结束，请编辑执行时间后再启用"
                            : task.state === "enabled"
                              ? S.actions.pause
                              : S.actions.resume
                        }
                        disabled={
                          !live || task.archived || task.state === "finished" || busy !== null
                        }
                        onClick={() =>
                          void withBusy(`toggle-${task.id}`, async () => {
                            await setScheduledTaskEnabled(task.id, task.state !== "enabled");
                          })
                        }
                      >
                        {task.state === "enabled" ? <Pause size={13} /> : <Play size={13} />}
                      </button>
                      <button
                        type="button"
                        className="icon-btn icon-btn--sm"
                        aria-label={S.actions.runNow}
                        title={S.actions.runNow}
                        disabled={!live || task.archived || busy !== null}
                        onClick={() =>
                          void withBusy(`run-${task.id}`, async () => {
                            await runScheduledTaskNow(task.id);
                            if (mounted.current) {
                              setExpanded(task.id);
                              setNotice("已提交本次执行，请在运行历史查看进度。");
                            }
                          })
                        }
                      >
                        <Zap size={13} />
                      </button>
                      <button
                        type="button"
                        className="icon-btn icon-btn--sm"
                        aria-label={S.actions.archive}
                        title={task.state === "enabled" ? "请先暂停任务再归档" : S.actions.archive}
                        disabled={
                          !live || task.archived || task.state === "enabled" || busy !== null
                        }
                        onClick={() =>
                          void withBusy(`archive-${task.id}`, async () => {
                            await archiveScheduledTask(task.id);
                            if (mounted.current)
                              setNotice("任务已归档，配置和历史可在「已归档」中查看。");
                          })
                        }
                      >
                        <Archive size={13} />
                      </button>
                    </div>
                  </div>
                  {open && (
                    <div className="schedule-row-history">
                      <h3>运行历史 · {task.title}</h3>
                      {task.task.kind === "agent" && (
                        <p className="schedule-task-message">{task.task.message}</p>
                      )}
                      {historyError && (
                        <p className="page-error" role="alert">
                          {historyError}
                        </p>
                      )}
                      {runsLoading && rows.length === 0 ? (
                        <p className="page-empty">正在加载运行记录…</p>
                      ) : rows.length === 0 ? (
                        <p className="page-empty">{S.history.empty}</p>
                      ) : (
                        <table className="schedule-runs">
                          <thead>
                            <tr>
                              <th>状态 / 结果</th>
                              <th>原计划 / 来源</th>
                              <th>实际开始 / 结束</th>
                              <th>结果与原因</th>
                              <th>操作</th>
                            </tr>
                          </thead>
                          <tbody>
                            {rows.map((run) => (
                              <tr key={run.id} data-testid={`schedule-run-${run.id}`}>
                                <td>
                                  <span
                                    className={`schedule-pill schedule-outcome-${run.outcome ?? run.lifecycle}`}
                                  >
                                    {run.outcome
                                      ? OUTCOMES[run.outcome]
                                      : RUN_STATES[run.lifecycle]}
                                  </span>
                                  {run.outcome && run.lifecycle !== "finished" && (
                                    <small>{RUN_STATES[run.lifecycle]}</small>
                                  )}
                                </td>
                                <td>
                                  {formatScheduleTime(run.scheduledFor, timezone)}
                                  <small>
                                    {run.triggerSource === "manual" ? "手动执行" : "定时执行"}
                                    {run.skippedCount > 0
                                      ? ` · 合并/跳过 ${run.skippedCount} 次`
                                      : ""}
                                  </small>
                                </td>
                                <td>
                                  {formatScheduleTime(run.startedAt, timezone)}
                                  <small>{formatScheduleTime(run.finishedAt, timezone)}</small>
                                </td>
                                <td className="schedule-run-summary">
                                  {run.errorSummary ||
                                    (run.reasonCode
                                      ? (S.reasons[run.reasonCode as keyof typeof S.reasons] ??
                                        run.reasonCode)
                                      : "—")}
                                  {run.reportRef && (
                                    <small title={run.reportRef}>报告引用：{run.reportRef}</small>
                                  )}
                                  {run.delivery && (
                                    <small>
                                      交付：
                                      {S.delivery[run.delivery as keyof typeof S.delivery] ??
                                        run.delivery}
                                    </small>
                                  )}
                                  {run.verification && (
                                    <small>
                                      验证：
                                      {S.verification[
                                        run.verification as keyof typeof S.verification
                                      ] ?? run.verification}
                                    </small>
                                  )}
                                </td>
                                <td>
                                  {run.sessionId && onOpenConversation && (
                                    <button
                                      type="button"
                                      className="btn"
                                      disabled={busy !== null}
                                      onClick={() =>
                                        void withBusy(`open-${run.id}`, async () => {
                                          await onOpenConversation(run.sessionId!);
                                        })
                                      }
                                    >
                                      查看会话
                                    </button>
                                  )}
                                  {run.lifecycle !== "finished" && (
                                    <button
                                      type="button"
                                      className="btn"
                                      disabled={
                                        !live || busy !== null || run.lifecycle === "cancelling"
                                      }
                                      onClick={() =>
                                        void withBusy(`cancel-${run.id}`, async () => {
                                          const accepted = await cancelTaskRun(run.id);
                                          if (mounted.current)
                                            setNotice(
                                              accepted
                                                ? S.history.cancelled
                                                : S.history.cancelFinished,
                                            );
                                        })
                                      }
                                    >
                                      {run.lifecycle === "cancelling" ? "正在取消" : "取消本次"}
                                    </button>
                                  )}
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
        {filter === "all" && !query && (
          <section className="schedule-templates" aria-label="任务模板">
            <h2>从常用任务开始</h2>
            <p className="schedule-hint">选择模板后，补充项目和时间再保存。</p>
            <div>
              {TASK_TEMPLATES.map((template) => (
                <button
                  type="button"
                  className="btn"
                  disabled={!live || busy !== null}
                  key={template.title}
                  onClick={() => setEditor({ task: null, template })}
                >
                  <Plus size={13} />
                  {template.title}
                </button>
              ))}
            </div>
          </section>
        )}
      </div>
      {editor && (
        <SchedulerEditor
          task={editor.task}
          template={editor.template}
          projects={projects}
          jobTypes={jobTypes}
          onClose={() => setEditor(null)}
          onSaved={onSaved}
        />
      )}
    </main>
  );
}
