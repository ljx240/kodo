import type { ScheduleDto, ScheduledTaskDto, TaskRunDto } from "../api";
import { T } from "../i18n";

export const WEEKDAYS = ["一", "二", "三", "四", "五", "六", "日"];
export const JOB_NAMES: Record<string, string> = {
  workspace_refresh: "刷新工作区",
  memory_outbox_replay: "更新记忆索引",
};
export const RUN_STATES: Record<TaskRunDto["lifecycle"], string> = {
  queued: "排队中",
  running: "执行中",
  cancelling: "正在取消",
  finished: "已结束",
};
export const OUTCOMES: Record<NonNullable<TaskRunDto["outcome"]>, string> = {
  succeeded: "成功",
  partial: "部分完成",
  blocked: "需要处理",
  failed: "失败",
  cancelled: "已取消",
  timedOut: "已超时",
  interrupted: "已中断",
  skipped: "已跳过",
};

export function schedulerError(error: unknown): string {
  const code =
    typeof error === "string"
      ? error
      : error instanceof Error
        ? error.message
        : error && typeof error === "object" && "code" in error
          ? String(error.code)
          : "unknown";
  return T.schedule.errors[code as keyof typeof T.schedule.errors] ?? code;
}

export function formatScheduleTime(seconds: number | null | undefined, timezone?: string): string {
  if (seconds == null || !Number.isFinite(seconds)) return "—";
  try {
    return new Date(seconds * 1000).toLocaleString("zh-CN", {
      timeZone: timezone,
      hour12: false,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  } catch {
    return new Date(seconds * 1000).toLocaleString("zh-CN");
  }
}

/** An input's wall time belongs to the task timezone, never the browser timezone. */
export function wallTime(seconds: number, timezone: string): string {
  const parts = new Intl.DateTimeFormat("en-CA", {
    timeZone: timezone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  }).formatToParts(new Date(seconds * 1000));
  const get = (key: string) => parts.find((p) => p.type === key)?.value ?? "00";
  return `${get("year")}-${get("month")}-${get("day")}T${get("hour")}:${get("minute")}:${get("second")}`;
}

export function wallTimeToUnix(value: string, timezone: string): number {
  const normalized = value.length === 16 ? `${value}:00` : value;
  const nominal = Date.parse(`${normalized}Z`) / 1000;
  if (
    !Number.isFinite(nominal) ||
    new Date(nominal * 1000).toISOString().slice(0, 19) !== normalized
  ) {
    throw new Error("请选择有效的执行日期和时间。");
  }
  // Sample both sides of a timezone transition. A gap has no matching UTC
  // instant; an overlap has two and takes the earlier, as the Rust calculator does.
  const matches = [-86400, 0, 86400]
    .map((delta) => {
      const sample = nominal + delta;
      const offset = Date.parse(`${wallTime(sample, timezone)}Z`) / 1000 - sample;
      return nominal - offset;
    })
    .filter((candidate) => wallTime(candidate, timezone) === normalized);
  if (matches.length === 0)
    throw new Error("该时区不存在这个本地时刻（夏令时切换），请选择其他时间。");
  return Math.min(...matches);
}

export type TaskDraft = {
  title: string;
  type: "agent" | "job";
  project: string;
  message: string;
  jobType: string;
  params: string;
  kind: ScheduleDto["kind"];
  timezone: string;
  onceAt: string;
  time: string;
  weekdays: number[];
  intervalMinutes: string;
  enabled: boolean;
  misfirePolicy: ScheduledTaskDto["misfirePolicy"];
  graceSeconds: string;
  overlapPolicy: ScheduledTaskDto["overlapPolicy"];
  timeoutSeconds: string;
};

export function taskDraft(task: ScheduledTaskDto | null, project = "", jobType = ""): TaskDraft {
  const schedule = task?.schedule;
  const timezone =
    schedule?.kind === "daily" || schedule?.kind === "weekly"
      ? schedule.tzName
      : task?.timezone || "Asia/Shanghai";
  return {
    title: task?.title ?? "",
    type: task?.task.kind ?? "agent",
    project: task?.task.kind === "agent" ? task.task.project : project,
    message: task?.task.kind === "agent" ? task.task.message : "",
    jobType: task?.task.kind === "job" ? task.task.jobType : jobType,
    params: task?.task.kind === "job" ? JSON.stringify(task.task.params, null, 2) : "{}",
    kind: schedule?.kind ?? "daily",
    timezone,
    onceAt: wallTime(
      schedule?.kind === "once" ? schedule.fireAt : Math.floor(Date.now() / 1000) + 3600,
      timezone,
    ),
    time:
      schedule?.kind === "daily" || schedule?.kind === "weekly"
        ? `${String(schedule.hour).padStart(2, "0")}:${String(schedule.minute).padStart(2, "0")}`
        : "09:00",
    weekdays: schedule?.kind === "weekly" ? schedule.weekdays : [1, 2, 3, 4, 5],
    intervalMinutes: schedule?.kind === "interval" ? String(schedule.intervalSeconds / 60) : "60",
    enabled: task ? task.state === "enabled" : true,
    misfirePolicy: task?.misfirePolicy ?? "latestOnce",
    graceSeconds: String(task?.graceSeconds ?? 3600),
    overlapPolicy: task?.overlapPolicy ?? "skip",
    timeoutSeconds: task?.timeoutSeconds?.toString() ?? "",
  };
}

function secondsInput(value: string, label: string, minimum: number): number {
  const number = Number(value);
  if (!value.trim() || !Number.isSafeInteger(number) || number < minimum || number > 31536000) {
    throw new Error(`${label}需在 ${minimum} 到 31536000 秒之间。`);
  }
  return number;
}

export function draftSchedule(draft: TaskDraft, original: ScheduledTaskDto | null): ScheduleDto {
  try {
    new Intl.DateTimeFormat("zh-CN", { timeZone: draft.timezone }).format();
  } catch {
    throw new Error("请输入有效的 IANA 时区，例如 Asia/Shanghai。");
  }
  if (!draft.timezone.trim()) throw new Error("请指定时区。");
  if (draft.kind === "once") {
    const fireAt = wallTimeToUnix(draft.onceAt, draft.timezone);
    if (fireAt <= Date.now() / 1000) throw new Error("一次执行的时间必须晚于现在。");
    return { kind: "once", fireAt };
  }
  if (draft.kind === "interval") {
    const intervalSeconds = secondsInput(
      String(Number(draft.intervalMinutes) * 60),
      "执行间隔",
      60,
    );
    return {
      kind: "interval",
      intervalSeconds,
      anchor:
        original?.schedule.kind === "interval" &&
        original.schedule.intervalSeconds === intervalSeconds
          ? original.schedule.anchor
          : Math.floor(Date.now() / 1000) + intervalSeconds,
    };
  }
  if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(draft.time)) throw new Error("请选择有效的执行时刻。");
  const [hour, minute] = draft.time.split(":").map(Number);
  if (draft.kind === "daily") return { kind: "daily", hour, minute, tzName: draft.timezone };
  if (!draft.weekdays.length) throw new Error("请至少选择一个执行日。");
  return {
    kind: "weekly",
    weekdays: [...draft.weekdays].sort((a, b) => a - b),
    hour,
    minute,
    tzName: draft.timezone,
  };
}

export function buildTask(
  draft: TaskDraft,
  original: ScheduledTaskDto | null,
  schedule: ScheduleDto,
  creationId: string,
): ScheduledTaskDto {
  if (!draft.title.trim()) throw new Error("请输入任务名称。");
  if (draft.type === "agent" && (!draft.project.trim() || !draft.message.trim())) {
    throw new Error("请选择项目并填写执行内容。");
  }
  if (draft.type === "job" && !draft.jobType) throw new Error("请选择已注册的后台作业。");
  let params: unknown;
  try {
    params = JSON.parse(draft.params);
  } catch {
    if (draft.type === "job") throw new Error("作业参数必须是合法 JSON。");
  }
  const previous = original?.task;
  const now = Math.floor(Date.now() / 1000);
  return {
    ...(original ?? {}),
    id: original?.id ?? creationId,
    title: draft.title.trim(),
    revision: original?.revision ?? 0,
    scheduleGeneration: original?.scheduleGeneration ?? 0,
    state: draft.enabled ? "enabled" : "paused",
    schedule,
    timezone: draft.timezone,
    task:
      draft.type === "agent"
        ? {
            kind: "agent",
            project: draft.project,
            message: draft.message.trim(),
            personas: previous?.kind === "agent" ? previous.personas : [],
            budgetFamily: previous?.kind === "agent" ? previous.budgetFamily : null,
            resourceKey:
              previous?.kind === "agent" && previous.project === draft.project
                ? previous.resourceKey
                : draft.project,
          }
        : {
            kind: "job",
            jobType: draft.jobType,
            params,
            resources:
              previous?.kind === "job" && previous.jobType === draft.jobType
                ? previous.resources
                : [draft.jobType === "memory_outbox_replay" ? "memory:v1" : "workspace"],
          },
    misfirePolicy: draft.misfirePolicy,
    overlapPolicy: draft.overlapPolicy,
    graceSeconds: secondsInput(draft.graceSeconds, "补跑宽限", 0),
    timeoutSeconds: draft.timeoutSeconds.trim()
      ? secondsInput(draft.timeoutSeconds, "运行超时", 1)
      : null,
    createdAt: original?.createdAt ?? now,
    updatedAt: now,
    nextFireAt: original?.nextFireAt ?? null,
    archived: false,
  };
}
