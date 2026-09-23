import { Check, ChevronRight, Minus, Square, X } from "lucide-react";
import { useState } from "react";
import { splitDetail, type TraceStep } from "../data/types";
import { T, failureCopy } from "../i18n";
import { groupPhases, type PhaseGroup } from "./trace";

function StatusMark({ status }: { status: TraceStep["status"] }) {
  if (status === "done") {
    return (
      <span className="trace-mark trace-mark--done">
        <Check size={11} strokeWidth={3} />
      </span>
    );
  }
  if (status === "failed") {
    return (
      <span className="trace-mark trace-mark--failed">
        <X size={11} strokeWidth={3} />
      </span>
    );
  }
  if (status === "interrupted") {
    return (
      <span className="trace-mark trace-mark--interrupted">
        <Minus size={11} strokeWidth={3} />
      </span>
    );
  }
  if (status === "stopped") {
    return (
      <span className="trace-mark trace-mark--stopped">
        <Square size={8} strokeWidth={2.5} />
      </span>
    );
  }
  return <span className="trace-mark trace-mark--running" />;
}

/** A run step's failure, in words — never color alone. */
function failureLine(step: TraceStep): string | null {
  if (step.status !== "failed" && !step.denied) return null;
  if (step.denied) return failureCopy("denied").reason;
  const cls = step.failureClass ?? (step.exitCode === 127 ? "command_not_found" : "non_zero_exit");
  return failureCopy(cls).reason;
}

/**
 * What a row hides until it is opened.
 *
 * Each kind is hiding something different, so each answers its own question —
 * a command its cwd/exit code/output, a file edit the paths and unified diffs,
 * a model its token counts. `detail` is the fallback for the rest. Internal
 * scheduling diagnostics are intentionally not here; they live in 运行详情.
 */
function Expanded({
  step,
  fileDiffs,
  subSteps,
  panelId,
}: {
  step: TraceStep;
  fileDiffs?: Record<string, string> | null;
  subSteps?: TraceStep[];
  /** Target of the row button's aria-controls — every branch carries it. */
  panelId: string;
}) {
  if (subSteps && subSteps.length > 0) {
    return (
      <div className="trace-expand" id={panelId}>
        <ul className="trace-thinking-list" data-testid="trace-thinking-list">
          {subSteps.map((sub, index) => (
            <li key={index} className="trace-thinking-item">
              {sub.detail}
            </li>
          ))}
        </ul>
      </div>
    );
  }
  switch (step.type) {
    case "run": {
      const meta = [
        step.cwd ? `cwd ${step.cwd}` : null,
        step.exitCode === undefined || step.exitCode === null
          ? step.status === "running"
            ? "exit —"
            : null
          : T.reply.exitCode(step.exitCode),
      ]
        .filter(Boolean)
        .join(" · ");
      return (
        <div className="trace-expand" id={panelId}>
          {meta && (
            <div className="trace-meta" data-testid="trace-run-meta">
              {meta}
            </div>
          )}
          <pre className="trace-output" data-testid="trace-run-output">
            {step.output ?? (step.status === "running" ? T.reply.inProgress : "尚无输出")}
          </pre>
        </div>
      );
    }
    case "edit": {
      const paths = step.files ?? [];
      return (
        <div className="trace-expand" id={panelId}>
          {paths.length > 0 ? (
            <ul className="trace-file-list" data-testid="trace-file-list">
              {paths.map((path) => {
                const diff = fileDiffs?.[path];
                return (
                  <li key={path}>
                    <code className="code-chip">{path}</code>
                    {diff ? (
                      <pre className="trace-output trace-diff" data-testid={`trace-diff-${path}`}>
                        {diff}
                      </pre>
                    ) : null}
                  </li>
                );
              })}
            </ul>
          ) : (
            <pre className="trace-output">{step.detail}</pre>
          )}
        </div>
      );
    }
    case "read":
    case "search":
      return (
        <div className="trace-expand" id={panelId}>
          <pre className="trace-output">{step.chip || step.detail}</pre>
        </div>
      );
    case "model":
      return (
        <div className="trace-expand" id={panelId}>
          <pre className="trace-output">
            {`${step.model}\n输入 ${step.input_tokens} tokens · 输出 ${step.output_tokens} tokens`}
          </pre>
        </div>
      );
    default:
      return (
        <div className="trace-expand" id={panelId}>
          <pre className="trace-output">{step.detail}</pre>
        </div>
      );
  }
}

type RowData = {
  /** The row shown; for merged thinking rows this is the first sub-step. */
  step: TraceStep;
  /** Consecutive thinking steps folded into one row. */
  subSteps?: TraceStep[];
};

function TraceItem({
  row,
  fileDiffs,
  rowKey,
}: {
  row: RowData;
  fileDiffs?: Record<string, string> | null;
  rowKey: string;
}) {
  const { step, subSteps } = row;
  // `null` until the reader opens the row themselves. The default is a function
  // of the step rather than a value captured once: only the step still running,
  // a failed step, or one awaiting an approval opens on its own — completed
  // output, thinking, model calls and diffs stay closed until asked for.
  const [toggled, setToggled] = useState<boolean | null>(null);
  const autoOpen =
    step.status === "running" || step.status === "failed" || Boolean(step.denied);
  const shown = toggled ?? autoOpen;
  const panelId = `trace-step-${rowKey}`;
  const reason = failureLine(step);

  // `splitDetail` is a guess about where a fixture's chip ends. A step that
  // knows its own chip says so, and is never re-split.
  const { chip, text } =
    step.chip === undefined ? splitDetail(step.detail) : { chip: step.chip || null, text: step.detail };

  const toggle = () => setToggled(!shown);

  const durationText =
    step.status === "running" ? T.reply.inProgress : step.duration || T.reply.unknownDuration;

  const isRun = step.type === "run";
  const affordance = isRun
    ? shown
      ? T.action.hideOutput
      : T.action.viewOutput
    : null;

  return (
    <li className={`trace-row trace-row--${step.status}`}>
      {/* The whole row line is one native button — keyboard, focus ring and
          Enter/Space come free, and the expanded panel is a sibling so no
          interactive content sits inside the control. */}
      <button
        type="button"
        className="trace-row-inner"
        aria-expanded={shown}
        aria-controls={panelId}
        onClick={toggle}
      >
        <span className="trace-toggle">
          <ChevronRight size={14} strokeWidth={1.9} className={shown ? "rot-90" : undefined} />
        </span>

        <span className="trace-line">
          <StatusMark status={step.status} />

          <span className="trace-label">{step.label}</span>

          <span className="trace-body">
            {chip && <code className="code-chip">{chip}</code>}
            {subSteps && subSteps.length > 1 ? (
              <code className="code-chip">{T.reply.subSteps(subSteps.length)}</code>
            ) : (
              text && <span className="trace-detail">{text}</span>
            )}

            {step.model && <code className="code-chip code-chip--model">{step.model}</code>}

            {step.input_tokens && (
              <span className="trace-tokens">
                输入 {step.input_tokens} tokens · 输出 {step.output_tokens} tokens
              </span>
            )}

            {isRun && step.exitCode != null && step.exitCode !== 0 && (
              <code className="code-chip code-chip--fail" data-testid="trace-exit-code">
                {T.reply.exitCode(step.exitCode)}
              </code>
            )}

            {reason && (
              <span className="trace-fail-reason" data-testid="trace-fail-reason">
                {T.reply.reason(reason)}
              </span>
            )}

            {affordance && <span className="trace-affordance">[{affordance}]</span>}
          </span>

          <span className="trace-duration">{durationText}</span>
        </span>
      </button>

      {shown && <Expanded step={step} fileDiffs={fileDiffs} subSteps={subSteps} panelId={panelId} />}
    </li>
  );
}

function foldThinking(steps: TraceStep[]): RowData[] {
  const rows: RowData[] = [];
  for (const step of steps) {
    const last = rows[rows.length - 1];
    if (step.type === "thinking" && last?.subSteps) {
      last.subSteps.push(step);
      continue;
    }
    if (step.type === "thinking") {
      rows.push({ step, subSteps: [step] });
      continue;
    }
    rows.push({ step });
  }
  return rows;
}

function PhaseBlock({
  group,
  fileDiffs,
  phaseIndex,
}: {
  group: PhaseGroup;
  fileDiffs?: Record<string, string> | null;
  phaseIndex: number;
}) {
  // Phases start open so the steps stay one glance away; the header line is
  // what collapses them. Step rows inside follow their own expand rules.
  const [open, setOpen] = useState<boolean | null>(null);
  const shown = open ?? true;
  const panelId = `trace-phase-${phaseIndex}`;
  const blocked = group.steps.some(
    (step) =>
      step.failureClass === "command_not_found" ||
      step.failureClass === "permission_denied" ||
      step.failureClass === "timeout",
  );
  const statusWord = group.running
    ? T.reply.inProgress
    : group.failed
      ? blocked
        ? T.verification.blocked
        : T.verification.failed
      : group.durationLabel;
  const meta = [T.reply.stepsCount(group.count), statusWord].filter(Boolean).join(" · ");
  const rows = foldThinking(group.steps);

  return (
    <section className="trace-phase" data-testid={`trace-phase-${group.phase}`}>
      <button
        type="button"
        className="trace-phase-head"
        aria-expanded={shown}
        aria-controls={panelId}
        onClick={() => setOpen(!shown)}
      >
        <ChevronRight size={14} strokeWidth={1.9} className={shown ? "rot-90" : undefined} />
        <span className="trace-phase-label">{group.label}</span>
        <span className="trace-phase-meta">{meta}</span>
      </button>
      {shown && (
        <ol className="trace" id={panelId}>
          {rows.map((row, index) => (
            <TraceItem
              key={`${row.step.type}-${row.step.chip ?? "n"}-${index}`}
              row={row}
              fileDiffs={fileDiffs}
              rowKey={`${phaseIndex}-${index}`}
            />
          ))}
        </ol>
      )}
    </section>
  );
}

/**
 * The run's story, grouped by high-level phase. Internal scheduling rounds and
 * budget counters never appear here — the phase line is 准备项目上下文 ·
 * N 个步骤 · duration, not `rounds 0/6 · tools 0/32`.
 */
export function AgentTrace({
  steps,
  fileDiffs = null,
}: {
  steps: TraceStep[];
  /** Unified diffs keyed by project-relative path (`turn_changes`). */
  fileDiffs?: Record<string, string> | null;
}) {
  const phases = groupPhases(steps);
  return (
    <div className="trace-groups" data-testid="trace-groups">
      {phases.map((group, index) => (
        <PhaseBlock key={`${group.phase}-${index}`} group={group} fileDiffs={fileDiffs} phaseIndex={index} />
      ))}
    </div>
  );
}
