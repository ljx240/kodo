import { Check, ChevronRight, Minus, Square } from "lucide-react";
import { useState } from "react";
import { splitDetail, type TraceStep } from "../data/types";

function StatusMark({ status }: { status: TraceStep["status"] }) {
  if (status === "done") {
    return (
      <span className="trace-mark trace-mark--done">
        <Check size={11} strokeWidth={3} />
      </span>
    );
  }
  if (status === "failed") {
    return <span className="trace-mark trace-mark--failed" />;
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

/**
 * What a row hides until it is opened.
 *
 * Each kind is hiding something different, so each answers its own question —
 * a command its output, a read and a search the exact string it looked at, a
 * model its token counts. `detail` is the fallback for the rest.
 */
function Expanded({ step }: { step: TraceStep }) {
  switch (step.type) {
    case "run":
      return <pre className="trace-output">{step.output ?? "尚无输出"}</pre>;
    case "read":
    case "search":
      return <pre className="trace-output">{step.chip || step.detail}</pre>;
    case "model":
      return (
        <pre className="trace-output">
          {`${step.model}\n输入 ${step.input_tokens} tokens · 输出 ${step.output_tokens} tokens`}
        </pre>
      );
    default:
      return <pre className="trace-output">{step.detail}</pre>;
  }
}

function TraceItem({ step }: { step: TraceStep }) {
  // `null` until the reader works the row themselves. The default is a function
  // of the step rather than a value captured once, because a step that is still
  // running has no output yet: seeded from `Boolean(step.output)` it would open
  // when it started and snap shut the moment it finished, which is the one
  // moment its output matters.
  const [toggled, setToggled] = useState<boolean | null>(null);
  const shown = toggled ?? (Boolean(step.output) || step.status === "running");

  // `splitDetail` is a guess about where a fixture's chip ends. A step that
  // knows its own chip says so, and is never re-split.
  const { chip, text } =
    step.chip === undefined ? splitDetail(step.detail) : { chip: step.chip || null, text: step.detail };

  const toggle = () => setToggled((value) => !(value ?? shown));

  return (
    <li className={`trace-row trace-row--${step.status}`}>
      {/* The whole row is the target, not just the chevron: seven rows of which
          five did nothing when clicked was the defect this replaces. */}
      <div
        className="trace-row-inner"
        role="button"
        tabIndex={0}
        aria-expanded={shown}
        aria-label={`${step.label}${shown ? "：收起详情" : "：展开详情"}`}
        onClick={toggle}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            toggle();
          }
        }}
      >
        <span className="trace-toggle">
          <ChevronRight size={14} strokeWidth={1.9} className={shown ? "rot-90" : undefined} />
        </span>

        <div className="trace-line">
          <StatusMark status={step.status} />

          <span className="trace-label">{step.label}</span>

          <span className="trace-body">
            {chip && <code className="code-chip">{chip}</code>}
            <span className="trace-detail">{text}</span>

            {step.model && <code className="code-chip code-chip--model">{step.model}</code>}

            {step.input_tokens && (
              <span className="trace-tokens">
                输入 {step.input_tokens} tokens · 输出 {step.output_tokens} tokens
              </span>
            )}
          </span>

          {shown && <Expanded step={step} />}

          <span className="trace-duration">{step.duration || "—"}</span>

          <span className="trace-disclosure">
            <ChevronRight size={14} strokeWidth={1.9} className={shown ? "rot-90" : undefined} />
          </span>
        </div>
      </div>
    </li>
  );
}

export function AgentTrace({ steps }: { steps: TraceStep[] }) {
  return (
    <ol className="trace">
      {steps.map((step, index) => (
        <TraceItem key={`${step.type}-${index}`} step={step} />
      ))}
    </ol>
  );
}
