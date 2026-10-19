import { useEffect, useLayoutEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { Clock, X } from "lucide-react";
import {
  createScheduledTask,
  previewSchedule,
  updateScheduledTask,
  type ProjectDto,
  type ScheduledTaskDto,
} from "../api";
import {
  buildTask,
  draftSchedule,
  formatScheduleTime,
  JOB_NAMES,
  schedulerError,
  taskDraft,
  WEEKDAYS,
  type TaskDraft,
} from "./schedulerModel";

export type TaskTemplate = { title: string; message: string; weekdays?: number[] };
export const TASK_TEMPLATES: TaskTemplate[] = [
  {
    title: "每日项目简报",
    message: "阅读项目最近的本地改动，整理进展、风险和下一步建议。只做分析，给出简洁的中文报告。",
  },
  {
    title: "工作日代码巡检",
    message:
      "检查项目近期改动，优先识别可复现的缺陷和风险，列出文件位置和修复建议。只做审查，保留所有现有修改。",
    weekdays: [1, 2, 3, 4, 5],
  },
  {
    title: "每周工作总结",
    message:
      "根据项目最近一周的本地提交和文档，整理完成事项、未解决问题和下周计划。输出可直接使用的中文周报。",
    weekdays: [5],
  },
];

export function SchedulerEditor({
  task,
  projects,
  jobTypes,
  template,
  onClose,
  onSaved,
}: {
  task: ScheduledTaskDto | null;
  projects: ProjectDto[];
  jobTypes: string[];
  template?: TaskTemplate;
  onClose: () => void;
  onSaved: (task: ScheduledTaskDto) => void;
}) {
  const [draft, setDraft] = useState<TaskDraft>(() => ({
    ...taskDraft(task, projects[0]?.path, jobTypes[0]),
    ...(template
      ? {
          title: template.title,
          message: template.message,
          ...(template.weekdays ? { kind: "weekly" as const, weekdays: template.weekdays } : {}),
        }
      : {}),
  }));
  const [error, setError] = useState<string | null>(null);
  const [creationId] = useState(() => crypto.randomUUID());
  const [busy, setBusy] = useState(false);
  const saving = useRef(false);
  const [preview, setPreview] = useState<{ key: string; times: number[]; error?: string } | null>(
    null,
  );
  const dialog = useRef<HTMLDialogElement>(null);
  const title = task ? "编辑定时任务" : "新建定时任务";
  const patch = (value: Partial<TaskDraft>) => setDraft((previous) => ({ ...previous, ...value }));

  useLayoutEffect(() => {
    const element = dialog.current;
    element?.showModal();
    element?.querySelector<HTMLInputElement>("input")?.focus();
    return () => element?.close();
  }, []);

  const calculation = useMemo(() => {
    try {
      return { schedule: draftSchedule(draft, task), error: null };
    } catch (failure) {
      return { schedule: null, error: schedulerError(failure) };
    }
    // Execution content does not change the schedule or restart interval anchors.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    draft.kind,
    draft.timezone,
    draft.onceAt,
    draft.time,
    draft.weekdays,
    draft.intervalMinutes,
    task,
  ]);
  const scheduleKey = JSON.stringify(calculation.schedule);

  useEffect(() => {
    if (!calculation.schedule) return;
    let active = true;
    const schedule = calculation.schedule;
    const timer = window.setTimeout(() => {
      void previewSchedule(schedule, Math.floor(Date.now() / 1000)).then(
        (times) => active && setPreview({ key: scheduleKey, times }),
        (failure) =>
          active && setPreview({ key: scheduleKey, times: [], error: schedulerError(failure) }),
      );
    }, 200);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [calculation.schedule, scheduleKey]);

  const previewReady = !!calculation.schedule && preview?.key === scheduleKey;
  const previewError = calculation.error ?? (previewReady ? preview?.error : null);
  const canSave = previewReady && !previewError && !!preview?.times.length;

  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (saving.current || !canSave || !calculation.schedule) return;
    saving.current = true;
    setError(null);
    try {
      if (
        calculation.schedule.kind === "once" &&
        calculation.schedule.fireAt <= Date.now() / 1000
      ) {
        throw new Error("执行时间已过，请重新选择未来时间。");
      }
      if (draft.type === "job" && !jobTypes.includes(draft.jobType))
        throw new Error("该后台作业尚未注册，请选择其他作业。");
      const payload = buildTask(draft, task, calculation.schedule, creationId);
      setBusy(true);
      const saved = task
        ? await updateScheduledTask(task.revision, payload)
        : await createScheduledTask(payload);
      onSaved(saved);
    } catch (failure) {
      setError(schedulerError(failure));
    } finally {
      saving.current = false;
      setBusy(false);
    }
  };

  const projectOptions =
    projects.some((p) => p.path === draft.project) || !draft.project
      ? projects
      : [{ path: draft.project, name: draft.project }, ...projects];

  return (
    <dialog
      ref={dialog}
      className="modal schedule-editor"
      aria-labelledby="schedule-editor-title"
      onCancel={(event) => {
        event.preventDefault();
        if (!saving.current) onClose();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget && !saving.current) onClose();
      }}
    >
      <header className="modal-head">
        <h2 id="schedule-editor-title" className="modal-title">
          {title}
        </h2>
        <button
          type="button"
          className="icon-btn"
          aria-label="关闭编辑器"
          disabled={busy}
          onClick={onClose}
        >
          <X size={16} />
        </button>
      </header>
      <form onSubmit={(event) => void save(event)} className="schedule-editor-form">
        <div className="schedule-editor-body">
          <fieldset disabled={busy} className="schedule-editor-fields">
            {error && (
              <p className="page-error" role="alert">
                {error}
              </p>
            )}
            <label className="field">
              任务名称
              <input
                className="input"
                required
                maxLength={200}
                value={draft.title}
                onChange={(e) => patch({ title: e.target.value })}
                placeholder="例如：每日项目简报"
              />
            </label>
            <label className="field">
              执行类型
              <select
                className="input"
                value={draft.type}
                onChange={(e) => patch({ type: e.target.value as TaskDraft["type"] })}
              >
                <option value="agent">Agent 任务</option>
                <option value="job">后台作业</option>
              </select>
            </label>
            {draft.type === "agent" ? (
              <>
                <label className="field">
                  执行项目
                  <select
                    className="input"
                    required
                    value={draft.project}
                    onChange={(e) => patch({ project: e.target.value })}
                  >
                    <option value="">请选择项目</option>
                    {projectOptions.map((p) => (
                      <option key={p.path} value={p.path}>
                        {p.name} · {p.path}
                      </option>
                    ))}
                  </select>
                </label>
                {projects.length === 0 && (
                  <p className="schedule-hint">请先从侧栏添加项目，再创建 Agent 任务。</p>
                )}
                <label className="field">
                  执行内容
                  <textarea
                    className="input"
                    required
                    rows={5}
                    value={draft.message}
                    onChange={(e) => patch({ message: e.target.value })}
                    placeholder="说明每次执行的目标、约束和预期结果。"
                  />
                </label>
                <p className="schedule-hint">
                  每次运行创建独立会话，使用现有 Agent 配置；需要审批时按现有权限处理。
                </p>
              </>
            ) : (
              <>
                <label className="field">
                  后台作业
                  <select
                    aria-label="后台作业"
                    className="input"
                    required
                    value={draft.jobType}
                    onChange={(e) => patch({ jobType: e.target.value })}
                  >
                    <option value="">请选择作业</option>
                    {!jobTypes.includes(draft.jobType) && draft.jobType && (
                      <option value={draft.jobType}>{draft.jobType}（未注册）</option>
                    )}
                    {jobTypes.map((name) => (
                      <option key={name} value={name}>
                        {JOB_NAMES[name] ?? name}
                      </option>
                    ))}
                  </select>
                </label>
                {jobTypes.length === 0 && <p className="schedule-hint">当前没有可用的后台作业。</p>}
                <label className="field">
                  作业参数（JSON）
                  <textarea
                    className="input schedule-json"
                    rows={4}
                    value={draft.params}
                    onChange={(e) => patch({ params: e.target.value })}
                  />
                </label>
              </>
            )}
            <div className="schedule-form-row">
              <label className="field">
                重复规则
                <select
                  className="input"
                  value={draft.kind}
                  onChange={(e) => patch({ kind: e.target.value as TaskDraft["kind"] })}
                >
                  <option value="once">一次执行</option>
                  <option value="interval">固定间隔</option>
                  <option value="daily">每天</option>
                  <option value="weekly">每周</option>
                </select>
              </label>
              <label className="field">
                时区
                <input
                  className="input"
                  required
                  list="schedule-timezones"
                  value={draft.timezone}
                  onChange={(e) => patch({ timezone: e.target.value })}
                />
              </label>
              <datalist id="schedule-timezones">
                {[
                  "Asia/Shanghai",
                  "Asia/Tokyo",
                  "Asia/Singapore",
                  "Europe/London",
                  "America/New_York",
                  "America/Los_Angeles",
                  "UTC",
                ].map((tz) => (
                  <option key={tz} value={tz} />
                ))}
              </datalist>
            </div>
            {draft.kind === "once" && (
              <label className="field">
                执行日期和时间
                <input
                  className="input"
                  type="datetime-local"
                  required
                  step={1}
                  value={draft.onceAt}
                  onChange={(e) => patch({ onceAt: e.target.value })}
                />
              </label>
            )}
            {draft.kind === "interval" && (
              <label className="field">
                间隔（分钟）
                <input
                  className="input"
                  type="number"
                  min={1}
                  max={525600}
                  step="any"
                  required
                  value={draft.intervalMinutes}
                  onChange={(e) => patch({ intervalMinutes: e.target.value })}
                />
              </label>
            )}
            {(draft.kind === "daily" || draft.kind === "weekly") && (
              <label className="field">
                执行时刻
                <input
                  className="input"
                  type="time"
                  required
                  value={draft.time}
                  onChange={(e) => patch({ time: e.target.value })}
                />
              </label>
            )}
            {draft.kind === "weekly" && (
              <fieldset className="schedule-weekdays">
                <legend>执行日</legend>
                {WEEKDAYS.map((day, index) => (
                  <label key={day}>
                    <input
                      type="checkbox"
                      checked={draft.weekdays.includes(index + 1)}
                      onChange={(e) =>
                        patch({
                          weekdays: e.target.checked
                            ? [...draft.weekdays, index + 1]
                            : draft.weekdays.filter((d) => d !== index + 1),
                        })
                      }
                    />
                    周{day}
                  </label>
                ))}
              </fieldset>
            )}
            <label className="schedule-check">
              <input
                type="checkbox"
                checked={draft.enabled}
                onChange={(e) => patch({ enabled: e.target.checked })}
              />
              保存后启用
            </label>
            <details className="schedule-advanced">
              <summary>高级设置</summary>
              <label className="field">
                错过执行时
                <select
                  className="input"
                  value={draft.misfirePolicy}
                  onChange={(e) =>
                    patch({ misfirePolicy: e.target.value as TaskDraft["misfirePolicy"] })
                  }
                >
                  <option value="latestOnce">宽限期内补跑最近一次</option>
                  <option value="skip">跳过错过的执行</option>
                  <option value="coalesce">合并为一次补跑</option>
                </select>
              </label>
              <label className="field">
                补跑宽限（秒，0 使用默认一小时）
                <input
                  className="input"
                  type="number"
                  min={0}
                  max={31536000}
                  value={draft.graceSeconds}
                  onChange={(e) => patch({ graceSeconds: e.target.value })}
                />
              </label>
              <p className="schedule-hint">
                运行仍受项目资源限制；资源被占用时会记录跳过原因。编辑现有任务会保留原有重叠策略。
              </p>
              <label className="field">
                单次超时（秒，可留空）
                <input
                  className="input"
                  type="number"
                  min={1}
                  max={31536000}
                  value={draft.timeoutSeconds}
                  onChange={(e) => patch({ timeoutSeconds: e.target.value })}
                />
              </label>
            </details>
          </fieldset>
          <aside className="schedule-preview" aria-label="执行时间预览" aria-live="polite">
            <h3>
              <Clock size={15} />
              接下来的执行
            </h3>
            <p className="schedule-hint">{draft.timezone}</p>
            {previewError ? (
              <p className="page-error" role="alert">
                {previewError}
              </p>
            ) : !previewReady ? (
              <p className="schedule-hint">正在计算…</p>
            ) : preview?.times.length ? (
              <ol>
                {preview.times.map((time) => (
                  <li key={time}>{formatScheduleTime(time, draft.timezone)}</li>
                ))}
              </ol>
            ) : (
              <p className="schedule-hint">没有未来执行时间，请调整规则。</p>
            )}
            <p className="schedule-hint">
              请保持电脑开机、Kodo 运行，且项目可访问。应用重启后按高级设置处理错过的执行。
            </p>
            {!draft.enabled && (
              <p className="schedule-hint">任务将以暂停状态保存，恢复后才会定时执行。</p>
            )}
          </aside>
        </div>
        <footer className="schedule-editor-foot">
          <button type="button" className="btn" disabled={busy} onClick={onClose}>
            取消
          </button>
          <button type="submit" className="btn btn--primary" disabled={busy || !canSave}>
            {busy ? "保存中…" : task ? "保存修改" : "创建任务"}
          </button>
        </footer>
      </form>
    </dialog>
  );
}
