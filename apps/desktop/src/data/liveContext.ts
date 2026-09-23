import type { ItemDto, TurnDto } from "../api";
import { T } from "../i18n";
import { formatDuration, formatTokens, toReply, type Reply } from "../conversation/trace";

/** Live conversation facts the Inspector paints on non-demo routes. */
export type LiveSnapshot = {
  conversationId: string;
  title: string;
  projectName: string;
  projectPath: string;
  turn: TurnDto | null;
  reply: Reply | null;
  running: boolean;
  /** Undo this turn's Kodo edits (Inspector summary action). */
  onUndo?: (() => void) | null;
  undoing?: boolean;
  /** Re-run after fixing the environment / regenerate. */
  onRetry?: (() => void) | null;
};

export function snapshotFromTurn(
  conversationId: string,
  title: string,
  projectName: string,
  projectPath: string,
  turn: TurnDto | null,
  running: boolean,
  actions?: { onUndo?: (() => void) | null; undoing?: boolean; onRetry?: (() => void) | null },
): LiveSnapshot {
  return {
    conversationId,
    title,
    projectName,
    projectPath,
    turn,
    reply: turn ? toReply(turn, running) : null,
    running,
    onUndo: actions?.onUndo ?? null,
    undoing: actions?.undoing ?? false,
    onRetry: actions?.onRetry ?? null,
  };
}

/** Tool counts keyed by the zh labels the UI shows. */
export function toolsFromTurn(turn: TurnDto | null): Record<string, number> {
  if (!turn) return {};
  const counts: Record<string, number> = {};
  for (const item of turn.items as ItemDto[]) {
    if (item.kind === "commandExecution") counts[T.step.run] = (counts[T.step.run] ?? 0) + 1;
    if (item.kind === "search") counts[T.step.search] = (counts[T.step.search] ?? 0) + 1;
    if (item.kind === "fileRead") counts[T.step.read] = (counts[T.step.read] ?? 0) + 1;
    if (item.kind === "modelCall") counts[T.step.model] = (counts[T.step.model] ?? 0) + 1;
    if (item.kind === "fileChange") counts[T.step.edit] = (counts[T.step.edit] ?? 0) + 1;
  }
  return counts;
}

export function llmFromTurn(turn: TurnDto | null) {
  if (!turn) return [];
  return turn.items
    .filter((item) => item.kind === "modelCall")
    .map((item) => {
      if (item.kind !== "modelCall") throw new Error("unreachable");
      return {
        model: item.model,
        input_tokens: formatTokens(item.inputTokens, item.status),
        output_tokens: formatTokens(item.outputTokens, item.status),
        duration: item.status === "running" ? T.reply.inProgress : formatDuration(item.duration),
      };
    });
}

export function commandsFromTurn(turn: TurnDto | null): { command: string; output: string }[] {
  if (!turn) return [];
  return turn.items
    .filter((item) => item.kind === "commandExecution")
    .map((item) => {
      if (item.kind !== "commandExecution") throw new Error("unreachable");
      return { command: item.command, output: item.output };
    });
}

export function changesFromReply(reply: Reply | null) {
  return reply?.changes ?? [];
}
