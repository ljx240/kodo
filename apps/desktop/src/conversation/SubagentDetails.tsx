import type { SubagentTraceDto } from "../api";
import { formatDuration, stripAnsi } from "./trace";
import "../styles/subagent.css";

const STATUS: Record<string, string> = {
  running: "执行中", completed: "已完成", done: "已完成", failed: "失败",
  blocked: "等待处理", cancelled: "已停止", timed_out: "超时",
  interrupted: "已中断", stopped: "已停止",
};

export function subagentStatus(status: string): string {
  return STATUS[status] ?? "未知状态";
}

export function SubagentDetails({ trace, status }: {
  trace: SubagentTraceDto;
  /** Parent recovery/stop takes precedence over a stale running snapshot. */
  status?: string;
}) {
  const effective = trace.status === "running" && status && status !== "running" ? status : trace.status;
  return (
    <section className="subagent-details" aria-label="子 Agent 执行详情" data-testid="subagent-details">
      <p className="trace-meta">
        <strong>{trace.agent}</strong> · {subagentStatus(effective)} · 输入 {trace.inputTokens.toLocaleString()} / 输出 {trace.outputTokens.toLocaleString()} tokens
      </p>
      <p className="subagent-task">{stripAnsi(trace.task)}</p>
      <dl className="subagent-identity">
        <dt>子任务</dt><dd><code>{trace.id}</code></dd>
        <dt>父任务</dt><dd><code>{trace.parentId}</code></dd>
      </dl>
      <ol className="subagent-steps" aria-label="子任务执行步骤">
        {trace.steps.map((step, index) => {
          const state = step.status === "running" && effective !== "running"
            ? effective === "cancelled" || effective === "stopped" ? "stopped" : "interrupted"
            : step.status;
          return (
            <li key={index}>
              <details open={state === "running" || state === "failed"}>
                <summary>
                  <span className="subagent-step-label">{stripAnsi(step.label)}</span>
                  <span className="trace-meta">{subagentStatus(state)} {formatDuration(step.durationMs)}</span>
                </summary>
                {step.output && <pre className="trace-output">{stripAnsi(step.output)}</pre>}
              </details>
            </li>
          );
        })}
      </ol>
      {trace.omittedSteps > 0 && <p className="trace-meta">另有 {trace.omittedSteps} 个步骤未保留，token 为全部调用合计。</p>}
      {trace.output && <pre className="trace-output" data-testid="subagent-result">{stripAnsi(trace.output)}</pre>}
    </section>
  );
}
