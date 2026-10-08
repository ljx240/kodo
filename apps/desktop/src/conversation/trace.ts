import type {
  ChangeDto,
  DeliveryStatus,
  InterimNote,
  ItemDto,
  LifecycleStatus,
  OutcomeStatus,
  TurnDto,
  TurnStatusDto,
  VerificationStatus,
} from "../api";
import type { TraceStep } from "../data/types";
import { T, failureCopy } from "../i18n";

/**
 * Step labels come from the central copy table; the core stores what happened,
 * not how to phrase it.
 */
const LABELS: Record<ItemDto["kind"], string> = {
  reasoning: T.step.thinking,
  search: T.step.search,
  fileRead: T.step.read,
  commandExecution: T.step.run,
  modelCall: T.step.model,
  fileChange: T.step.edit,
  agentMessage: T.step.finalize,
};

const PHASE_CODES = ["prepare", "analyze", "execute", "verify", "summarize"] as const;
export type PhaseCode = (typeof PHASE_CODES)[number];

function isPhase(code: string | undefined): code is PhaseCode {
  return Boolean(code) && (PHASE_CODES as readonly string[]).includes(code as string);
}

/**
 * ANSI control sequences (colors, weights) that terminals emit and browsers
 * show as garbage like `[1m[92m`. Display text is cleaned; the session log
 * keeps the raw output. The ESC literal is built from its code point so this
 * source stays plain ASCII.
 */
export function stripAnsi(text: string): string {
  const esc = String.fromCharCode(27);
  return text
    .replace(new RegExp(esc + "\\[[0-9;?]*[ -/]*[@-~]", "g"), "")
    .replace(new RegExp(esc + "\\][^" + esc + "]*(?:\\x07|" + esc + "\\\\)?", "g"), "")
    .replace(new RegExp(esc + "[@-Z\\\\-_]", "g"), "");
}

/** One stored item, as the trace renders it. */
export function toStep(item: ItemDto): TraceStep {
  const base = {
    label: LABELS[item.kind],
    duration: formatDuration(item.duration),
    durationMs: item.duration,
    status: item.status,
    stepId: item.id,
    phase: item.kind === "reasoning" && isPhase(item.phase) ? item.phase : undefined,
  };

  switch (item.kind) {
    case "reasoning":
      return {
        ...base,
        type: "thinking",
        detail: stripAnsi(item.summary),
        diagnostics: item.diagnostics ?? null,
      };
    case "search":
      return { ...base, type: "search", chip: stripAnsi(item.query), detail: stripAnsi(item.detail) };
    case "fileRead":
      return { ...base, type: "read", chip: stripAnsi(item.path), detail: stripAnsi(item.detail) };
    case "commandExecution":
      return {
        ...base,
        type: "run",
        chip: stripAnsi(item.command),
        detail: "",
        output: item.output ? stripAnsi(item.output) : undefined,
        cwd: item.cwd,
        exitCode: item.exitCode,
        denied: item.denied ?? false,
        failureClass: item.failureClass ?? null,
        failureTool: item.failureTool ?? null,
        status: item.denied || (item.exitCode != null && item.exitCode !== 0) ? "failed" : item.status,
      };
    case "modelCall":
      return {
        ...base,
        type: "model",
        detail: "",
        model: item.model,
        input_tokens: formatTokens(item.inputTokens, item.status),
        output_tokens: formatTokens(item.outputTokens, item.status),
        llmIoRef: item.llmIoRef ?? null,
      };
    case "fileChange":
      return {
        ...base,
        type: "edit",
        detail: describe(item.changes),
        files: item.changes.map((change) => change.path),
        fileSpanRef: item.fileSpanRef ?? null,
      };
    case "agentMessage":
      return { ...base, type: "finalize", detail: "", phase: "summarize" };
  }
}

/** The three-axis run state. Never collapsed into one ambiguous flag. */
export type RunStatus = {
  lifecycle: LifecycleStatus;
  delivery: DeliveryStatus;
  verification: VerificationStatus;
  outcome: OutcomeStatus;
};

export type VerifyCounts = { passed: number; failed: number; blocked: number; notRun: number };

export type FailureGroup = {
  cls: string;
  count: number;
  /** Localized cause line, e.g. 找不到命令. */
  reason: string;
  /** Localized recovery suggestion. */
  recovery: string;
  /** Missing binary for command_not_found groups. */
  tool: string | null;
  /** Merged summary: 3 个验证步骤因缺少 cargo 而阻塞. */
  label: string;
  /** Raw commands, preserved for 查看失败命令. */
  commands: string[];
  stepIds: number[];
};

export type PhaseGroup = {
  phase: PhaseCode;
  label: string;
  steps: TraceStep[];
  /** Sum of known durations; null when nothing measurable ran. */
  durationMs: number | null;
  durationLabel: string;
  failed: boolean;
  running: boolean;
  /** How many of the group's steps are thinking steps (sub-step count). */
  count: number;
};

/** What one turn renders as, once its items are read. */
export type Reply = {
  steps: TraceStep[];
  /** Conversational answer (你是谁/你好): the sentence alone, no report card. */
  plain: boolean;
  /** Consecutive same-phase runs — the default trace grouping. */
  phases: PhaseGroup[];
  status: RunStatus;
  /** The answer, or null while there is not one yet. */
  final: string | null;
  checks: string[];
  changes: ChangeDto[];
  /** The change totals after path-merge. */
  files: number;
  added: number;
  removed: number;
  /** An item that was started and never finished, and is not running now. */
  interrupted: boolean;
  /** The user stopped this run. */
  stopped: boolean;
  /** Terminal error recorded on the turn, if any. */
  error: string | null;
  /** Model chips from completed model calls in this turn. */
  models: string[];
  /** `12.4k → 2.1k` rollup for the reply footer, or null when unknown. */
  tokens: string | null;
  /** Real token counts; nulls mean "not counted yet", never 0. */
  tokensPending: boolean;
  /** Live context-window telemetry from the most recent model call. `null`
   *  until the first `ContextBudget` event arrives this turn. */
  contextBudget: ContextBudget | null;
  verify: VerifyCounts;
  /** Consecutive same-root-cause failures merged into one card. */
  failureGroups: FailureGroup[];
};

/**
 * Folds a turn into the pieces the reply renders.
 *
 * The agent's message is pulled out of the step list rather than drawn as a
 * step: it is the answer, and the trace is what led to it. Status prefers the
 * backend's structured axes and only derives them from item facts when a turn
 * predates the outcome fields (live/demo turns) — never from display strings.
 */
export function toReply(turn: TurnDto, running: boolean, awaitingApproval = false): Reply {
  const answer = [...turn.items].reverse().find((item) => item.kind === "agentMessage");
  const status = runStatus(turn, running, awaitingApproval);
  const hasRunningItem = turn.items.some((item) => item.status === "running");
  // A step still marked running after the run left its live phases is a killed
  // step: paint it as the turn's terminal lifecycle, never as still spinning.
  const remapRunningTo =
    !running &&
    (status.lifecycle === "interrupted" || status.lifecycle === "stopped" || status.lifecycle === "failed")
      ? status.lifecycle
      : null;
  const steps = mergeNotes(
    turn.items
      .filter((item) => item.kind !== "agentMessage")
      .map(toStep)
      .map((step) => (remapRunningTo && step.status === "running" ? { ...step, status: remapRunningTo } : step)),
    turn.notes ?? [],
  );
  const rawChanges = turn.items.flatMap((item) => (item.kind === "fileChange" ? item.changes : []));
  const changes = mergeChanges(rawChanges);
  const modelCalls = turn.items.filter((item) => item.kind === "modelCall");
  const models = [...new Set(modelCalls.map((item) => (item.kind === "modelCall" ? item.model : "")))].filter(
    Boolean,
  );
  const input = modelCalls.reduce(
    (sum, item) => sum + (item.kind === "modelCall" ? item.inputTokens : 0),
    0,
  );
  const output = modelCalls.reduce(
    (sum, item) => sum + (item.kind === "modelCall" ? item.outputTokens : 0),
    0,
  );
  const tokensPending = running || (modelCalls.length > 0 && input + output === 0);
  const verify = verifyCounts(steps);
  const failureGroups = mergeFailures(steps);

  return {
    steps,
    plain: answer?.kind === "agentMessage" ? Boolean(answer.plain) : false,
    phases: groupPhases(steps),
    status,
    final: answer?.kind === "agentMessage" ? sanitizeAssistantText(answer.text) : null,
    checks: answer?.kind === "agentMessage" ? answer.checks : [],
    ...totals(changes),
    changes,
    interrupted:
      !running &&
      !turn.done &&
      !turn.stopped &&
      !turn.error &&
      (Boolean(turn.interrupted) || hasRunningItem),
    stopped: Boolean(turn.stopped),
    error: turn.error ?? null,
    models,
    tokens: modelCalls.length > 0 ? `${formatTokens(input, "done")} → ${formatTokens(output, "done")}` : null,
    tokensPending,
    contextBudget: null,
    verify,
    failureGroups,
  };
}

/**
 * The three axes. Live lifecycle values (working / awaiting approval) come from
 * the event stream; everything else is structured data.
 */
export function runStatus(
  turn: TurnDto,
  running: boolean,
  awaitingApproval = false,
): RunStatus {
  if (running) {
    return {
      lifecycle: awaitingApproval ? "awaiting_approval" : "working",
      delivery: "partial",
      verification: "running",
      outcome: awaitingApproval ? "awaiting_approval" : "working",
    };
  }
  if (turn.status) return turn.status;

  // Legacy / demo turns: derive from structured flags and item facts only.
  const items = turn.items;
  const wroteFiles = items.some((item) => item.kind === "fileChange");
  let lifecycle: LifecycleStatus;
  if (turn.error) lifecycle = "failed";
  else if (turn.stopped) lifecycle = "stopped";
  else if (turn.interrupted || items.some((item) => item.status === "running")) lifecycle = "interrupted";
  else if (turn.done) lifecycle = "completed";
  else lifecycle = "interrupted";

  let delivery: DeliveryStatus | null = null;
  let verification: VerificationStatus = "not_run";
  for (const item of items) {
    if (item.kind === "agentMessage") {
      if (item.delivery) delivery = item.delivery as DeliveryStatus;
      if (item.verification) verification = item.verification as VerificationStatus;
      // A legacy answer carries no delivery code; its existence is the delivery.
      if (!delivery && !turn.error) delivery = "ready";
    }
  }
  if (!delivery) delivery = turn.error ? "failed" : wroteFiles ? "partial" : "failed";

  const counts = verifyCounts(items.map(toStep));
  if (verification === "not_run") {
    if (counts.blocked > 0) verification = "blocked";
    else if (counts.failed > 0) verification = "failed";
    else if (counts.passed > 0) verification = "passed";
  }

  return { lifecycle, delivery, verification, outcome: outcomeFor(lifecycle, delivery, verification, wroteFiles) };
}

export function outcomeFor(
  lifecycle: LifecycleStatus,
  delivery: DeliveryStatus,
  verification: VerificationStatus,
  wroteFiles: boolean,
): OutcomeStatus {
  if (lifecycle === "stopped") return "stopped";
  if (lifecycle === "interrupted") return "interrupted";
  if (lifecycle === "failed") return "failed";
  if (lifecycle === "working") return "working";
  if (lifecycle === "awaiting_approval") return "awaiting_approval";
  if (lifecycle === "queued") return "queued";
  if (verification === "blocked") return "blocked_by_environment";
  if (delivery === "failed") return "failed";
  if (delivery === "ready" && verification === "passed") return "completed";
  if (delivery === "ready" && verification === "not_run" && !wroteFiles) return "completed";
  return "partially_completed";
}

/**
 * How many acceptance checks passed / failed / were blocked by the environment.
 *
 * A turn with structured reasoning phases counts only the commands inside
 * the verify phase — the exploratory runs in execute are the story of how,
 * not the verdict, and counting them produced the contradiction
 * 「未通过 · 通过 15 · 失败 0」 when verification never ran (2026-09-30).
 * Fixtures without reasoning phases (the answer step's lone `summarize`
 * marker is not enough) keep the flat run count.
 */
export function verifyCounts(steps: TraceStep[]): VerifyCounts {
  const counts: VerifyCounts = { passed: 0, failed: 0, blocked: 0, notRun: 0 };
  const phased = steps.some((step) => step.type === "thinking" && isPhase(step.phase));
  let current: PhaseCode = "prepare";
  for (const step of steps) {
    if (isPhase(step.phase)) current = step.phase;
    if (step.type !== "run") continue;
    if (phased && current !== "verify") continue;
    const cls = step.failureClass;
    const failed = step.status === "failed" || step.denied || (step.exitCode != null && step.exitCode !== 0);
    if (!failed) {
      if (step.status === "done") counts.passed += 1;
      continue;
    }
    if (cls === "command_not_found" || cls === "permission_denied" || cls === "timeout") {
      counts.blocked += 1;
    } else {
      counts.failed += 1;
    }
  }
  return counts;
}

/**
 * Merge consecutive failures that share a root cause into one group, so three
 * `cargo` commands that cannot start read as one environment blockage.
 */
export function mergeFailures(steps: TraceStep[]): FailureGroup[] {
  const groups: FailureGroup[] = [];
  for (const step of steps) {
    const failed = step.status === "failed" || step.denied || (step.exitCode != null && step.exitCode !== 0);
    if (!failed || step.type !== "run") continue;
    const cls = step.failureClass ?? "unknown";
    const tool = step.failureTool ?? null;
    const last = groups[groups.length - 1];
    if (last && last.cls === cls && last.tool === tool) {
      last.count += 1;
      if (step.chip) last.commands.push(step.chip);
      if (step.stepId != null) last.stepIds.push(step.stepId);
      continue;
    }
    groups.push({
      cls,
      count: 1,
      reason: failureCopy(cls).reason,
      recovery: failureCopy(cls).recovery,
      tool,
      label: "",
      commands: step.chip ? [step.chip] : [],
      stepIds: step.stepId != null ? [step.stepId] : [],
    });
  }
  for (const group of groups) {
    const blocked = group.cls === "command_not_found" || group.cls === "permission_denied" || group.cls === "timeout";
    group.label = blocked
      ? T.verify.mergedBlocked(group.count, group.tool ?? "")
      : T.verify.mergedFailed(group.count, group.reason);
  }
  return groups;
}

/** Fallback phase for fixture/legacy steps that predate structured phases. */
function typePhase(type: TraceStep["type"]): PhaseCode {
  switch (type) {
    case "thinking":
      return "prepare";
    case "search":
    case "read":
      return "analyze";
    case "finalize":
      return "summarize";
    default:
      return "execute";
  }
}

/**
 * Interim stream prose, folded into the step list at the item boundary that
 * flushed it (`beforeId`) so the trace keeps arrival order. Anchors that never
 * matched a visible step (the item is the answer itself) still land last rather
 * than being dropped silently.
 */
function mergeNotes(steps: TraceStep[], notes: InterimNote[]): TraceStep[] {
  if (notes.length === 0) return steps;
  const noteStep = (note: InterimNote): TraceStep => ({
    type: "note",
    label: "",
    detail: sanitizeAssistantText(stripAnsi(note.text)),
    duration: "",
    durationMs: null,
    status: "done",
    stepId: note.id,
  });
  const byAnchor = new Map<number, InterimNote[]>();
  for (const note of notes) {
    const list = byAnchor.get(note.beforeId);
    if (list) list.push(note);
    else byAnchor.set(note.beforeId, [note]);
  }
  const merged: TraceStep[] = [];
  for (const step of steps) {
    const anchored = step.stepId != null ? byAnchor.get(step.stepId) : undefined;
    if (anchored) {
      merged.push(...anchored.map(noteStep));
      byAnchor.delete(step.stepId!);
    }
    merged.push(step);
  }
  for (const leftover of byAnchor.values()) merged.push(...leftover.map(noteStep));
  return merged;
}

/**
 * Group consecutive same-phase steps. Phase comes from the structured field on
 * reasoning steps and is inherited by the steps between them — never parsed
 * out of summary text. Fixture steps with no phase at all fall back to the
 * phase their kind implies.
 */
export function groupPhases(steps: TraceStep[]): PhaseGroup[] {
  const groups: PhaseGroup[] = [];
  let current: PhaseCode = "prepare";
  const structured = steps.some((step) => isPhase(step.phase));
  for (const step of steps) {
    let code: PhaseCode;
    if (isPhase(step.phase)) {
      current = step.phase;
      code = step.phase;
    } else {
      code = structured ? current : typePhase(step.type);
    }
    const last = groups[groups.length - 1];
    if (last && last.phase === code) {
      last.steps.push(step);
    } else {
      groups.push({
        phase: code,
        label: phaseLabel(code),
        steps: [step],
        durationMs: null,
        durationLabel: "",
        failed: false,
        running: false,
        count: 0,
      });
    }
  }
  for (const group of groups) {
    let sum = 0;
    let measured = false;
    for (const step of group.steps) {
      if (typeof step.durationMs === "number" && step.durationMs > 0) {
        sum += step.durationMs;
        measured = true;
      }
    }
    group.failed = group.steps.some((step) => step.status === "failed");
    group.running = group.steps.some((step) => step.status === "running");
    group.durationMs = measured ? sum : null;
    // Unmeasured groups omit the segment rather than claiming 0ms / —.
    group.durationLabel = group.running
      ? T.reply.inProgress
      : measured
        ? formatDuration(sum) || ""
        : "";
    // Interim prose rows ride along but are not steps.
    group.count = group.steps.filter((step) => step.type !== "note").length;
  }
  return groups;
}

export function phaseLabel(code: string): string {
  if (isPhase(code)) return T.phase[code];
  return T.phase.unknown;
}

/**
 * Removes provider protocol and internal workflow notes from user-facing text.
 * Tag-protocol spans (`<tool_call` / `<function=NAME` / `<parameter=KEY`)
 * go first — they are multi-line and several calls can share one line.
 * Matching is indexOf-based: a regex literal cannot hold the closers because
 * their slash would end the literal.
 */
export function sanitizeAssistantText(text: string): string {
  const withoutTags = stripTagSpans(text);
  return withoutTags
    .split("\n")
    .filter((line) => {
      const trimmed = line.trimStart();
      return !trimmed.includes('"tool_calls"') &&
        !trimmed.includes("<function=") &&
        !trimmed.includes("<parameter=") &&
        !/^[-*]?\s*\*\*(工具协议|技能系统|工作原则|Verification status)\*\*/.test(trimmed) &&
        !/^[-*]?\s*\*\*Verification status:\*\*/.test(trimmed);
    })
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

/** Drops every <tool_call…</tool_call> / <function=…</function> / <parameter=…</parameter> span (debris included). */
function stripTagSpans(text: string): string {
  const pairs: Array<[string, string]> = [
    ["<tool_call", "</tool_call>"],
    ["<function=", "</function>"],
    ["<parameter=", "</parameter>"],
  ];
  let out = text;
  for (const [open, close] of pairs) {
    let result = "";
    let rest = out;
    for (;;) {
      const start = rest.indexOf(open);
      if (start < 0) {
        result += rest;
        break;
      }
      result += rest.slice(0, start);
      const end = rest.indexOf(close, start + open.length);
      if (end < 0) break;
      rest = rest.slice(end + close.length);
    }
    out = result;
  }
  return out;
}

/**
 * One path → one entry. A turn that edits the same file twice must not count
 * it twice; line deltas for that path are summed (per-step deltas, not a net
 * working-tree diff), and the edit count records how many times it was touched.
 */
export function mergeChanges(changes: ChangeDto[]): ChangeDto[] {
  const byPath = new Map<string, ChangeDto>();
  for (const change of changes) {
    const existing = byPath.get(change.path);
    if (existing) {
      existing.added += change.added;
      existing.removed += change.removed;
      existing.edits = (existing.edits ?? 1) + (change.edits ?? 1);
    } else {
      byPath.set(change.path, { ...change, edits: change.edits ?? 1 });
    }
  }
  return [...byPath.values()];
}

export function totals(changes: ChangeDto[]): { files: number; added: number; removed: number } {
  const unique = mergeChanges(changes);
  return {
    files: unique.length,
    added: unique.reduce((total, change) => total + change.added, 0),
    removed: unique.reduce((total, change) => total + change.removed, 0),
  };
}

function describe(changes: ChangeDto[]): string {
  const { files, added, removed } = totals(changes);
  return `${files} 个文件 · +${added} −${removed}`;
}

/**
 * Milliseconds as `820ms`, `1.5s`, `1m 12s`. Unknown and sub-readings of 0 ms
 * both return "" — renderers omit the placeholder rather than showing `—`.
 */
export function formatDuration(ms: number | null | undefined): string {
  if (ms == null || ms <= 0) return "";

  if (ms < 1000) return `${ms}ms`;

  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;

  const whole = Math.round(seconds);
  return `${Math.floor(whole / 60)}m ${whole % 60}s`;
}

/**
 * `12480` → `12.5k`. While a model call runs (or nothing has been counted yet)
 * this reads 统计中… — never a bare 0.
 */
export function formatTokens(count: number, status?: string): string {
  if (status === "running") return T.tokens.counting;
  if (!count) return T.tokens.pending;
  return count < 1000 ? String(count) : `${(count / 1000).toFixed(1)}k`;
}

/** Live context-window telemetry (mirrors Rust `SinkEvent::ContextBudget`). */
export type ContextBudget = {
  used: number;
  window: number;
  percent: number;
  cumulativeChars: number;
  compactedCount: number;
  lastModel: string;
};

/** Compact "12.4k / 200k · 6%" string for the Codex-style bar label. */
export function formatContextBudget(b: ContextBudget): string {
  return `${formatTokenPair(b.used)} / ${formatTokenPair(b.window)} · ${b.percent}%`;
}

/** Single number shrinker (e.g. 12480 → "12.5k"); mirror of Rust's
 *  `shrink_tokens` in `state.rs` so numbers read the same end-to-end. */
function formatTokenPair(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}m`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

/** Toast copy for `RunEvent::Compacted` — "已压缩 N 条 · 释放 X%"。 */
export function formatCompactToast(dropped: number, freedPct: number): string {
  return `${T.budget.compacted(dropped)} · ${T.budget.freed(freedPct)}`;
}

/** Session titles default to 新对话 until the first ask supplies a real one. */
export function titleFromAsk(ask: string): string | null {
  const compact = ask.replace(/\s+/g, " ").trim();
  if (!compact) return null;
  return compact.length <= 24 ? compact : `${compact.slice(0, 24)}…`;
}

/**
 * Reads `820ms` / `1.5s` / `1m 12s` back into milliseconds. Fixture steps
 * carry only the display string; live steps carry `duration` (ms) outright.
 */
export function durationMsOf(text: string | null | undefined): number | null {
  if (!text) return null;
  let total = 0;
  let matched = false;
  for (const part of text.trim().split(/\s+/)) {
    const match = /^(\d+(?:\.\d+)?)(ms|s|m)$/.exec(part);
    if (!match) continue;
    matched = true;
    const value = parseFloat(match[1]);
    total += match[2] === "ms" ? value : match[2] === "s" ? value * 1000 : value * 60_000;
  }
  return matched ? total : null;
}

export type { DeliveryStatus, LifecycleStatus, OutcomeStatus, TurnStatusDto, VerificationStatus };
