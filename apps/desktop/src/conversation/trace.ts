import type { ChangeDto, ItemDto, TurnDto } from "../api";
import type { TraceStep } from "../data/types";

/**
 * The labels the reference draws for each kind of step. They live here rather
 * than on the item because the core stores what happened, not how to phrase it.
 */
const LABELS: Record<ItemDto["kind"], string> = {
  reasoning: "Thinking",
  search: "Search codebase",
  fileRead: "Read file",
  commandExecution: "Run command",
  modelCall: "Call model",
  fileChange: "Edit files",
  agentMessage: "Draft answer",
};

/** One stored item, as the trace renders it. */
export function toStep(item: ItemDto): TraceStep {
  const base = {
    label: LABELS[item.kind],
    duration: formatDuration(item.duration),
    status: item.status,
  };

  switch (item.kind) {
    case "reasoning":
      return { ...base, type: "thinking", detail: item.summary };
    case "search":
      return { ...base, type: "search", chip: item.query, detail: item.detail };
    case "fileRead":
      return { ...base, type: "read", chip: item.path, detail: item.detail };
    case "commandExecution":
      return { ...base, type: "run", chip: item.command, detail: "", output: item.output || undefined };
    case "modelCall":
      return {
        ...base,
        type: "model",
        detail: "",
        model: item.model,
        input_tokens: formatTokens(item.inputTokens),
        output_tokens: formatTokens(item.outputTokens),
      };
    case "fileChange":
      return { ...base, type: "edit", detail: describe(item.changes) };
    case "agentMessage":
      return { ...base, type: "finalize", detail: "" };
  }
}

/** What one turn renders as, once its items are read. */
export type Reply = {
  steps: TraceStep[];
  status: ReplyStatus;
  error: string | null;
  /** The answer, or null while there is not one yet. */
  final: string | null;
  checks: string[];
  changes: ChangeDto[];
  /** The change totals, which the fixture states outright rather than deriving. */
  files: number;
  added: number;
  removed: number;
};

export type ReplyStatus = "empty" | "working" | "completed" | "stopped" | "interrupted" | "failed";

/**
 * Folds a turn into the pieces the reply renders.
 *
 * The agent's message is pulled out of the step list rather than drawn as a
 * step: it is the answer, and the trace is what led to it. The lifecycle is
 * derived once here so the reply, trace, and Inspector cannot disagree.
 */
export function toReply(turn: TurnDto, running: boolean): Reply {
  const answer = [...turn.items].reverse().find((item) => item.kind === "agentMessage");
  const status = replyStatus(turn, running);
  const steps = turn.items
    .filter((item) => item.kind !== "agentMessage")
    .map(toStep)
    .map((step) =>
      step.status === "running" &&
      (status === "interrupted" || status === "stopped" || status === "failed")
        ? { ...step, status }
        : step,
    );
  const changes = turn.items.flatMap((item) => (item.kind === "fileChange" ? item.changes : []));

  return {
    steps,
    status,
    error: turn.error,
    final: answer?.kind === "agentMessage" ? answer.text : null,
    checks: answer?.kind === "agentMessage" ? answer.checks : [],
    ...totals(changes),
    changes,
  };
}

export function replyStatus(turn: TurnDto, running: boolean): ReplyStatus {
  if (running) return "working";
  if (turn.error) return "failed";
  if (turn.done) return "completed";
  if (turn.stopped) return "stopped";
  if (turn.items.some((item) => item.status === "running")) return "interrupted";
  return "empty";
}

export function totals(changes: ChangeDto[]): { files: number; added: number; removed: number } {
  return {
    files: changes.length,
    added: changes.reduce((total, change) => total + change.added, 0),
    removed: changes.reduce((total, change) => total + change.removed, 0),
  };
}

function describe(changes: ChangeDto[]): string {
  const added = changes.reduce((total, change) => total + change.added, 0);
  const removed = changes.reduce((total, change) => total + change.removed, 0);
  return `${changes.length} 个文件 · +${added} −${removed}`;
}

/** Milliseconds, in the shape the reference uses: `820ms`, `1.5s`, `1m 12s`. */
export function formatDuration(ms: number | null): string {
  if (ms === null) return "—";
  if (ms < 1000) return `${ms}ms`;

  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;

  const whole = Math.round(seconds);
  return `${Math.floor(whole / 60)}m ${whole % 60}s`;
}

/** `12480` → `12.5k`, so a token count stays one short chip. */
export function formatTokens(count: number): string {
  return count < 1000 ? String(count) : `${(count / 1000).toFixed(1)}k`;
}
