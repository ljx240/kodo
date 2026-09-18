import type { ItemDto, TurnDto } from "../api";
import type { Reply } from "../conversation/trace";
import { toReply } from "../conversation/trace";

/** Live conversation facts the Inspector paints on non-demo routes. */
export type LiveSnapshot = {
  conversationId: string;
  title: string;
  projectName: string;
  projectPath: string;
  turn: TurnDto | null;
  reply: Reply | null;
  running: boolean;
};

export function snapshotFromTurn(
  conversationId: string,
  title: string,
  projectName: string,
  projectPath: string,
  turn: TurnDto | null,
  running: boolean,
): LiveSnapshot {
  return {
    conversationId,
    title,
    projectName,
    projectPath,
    turn,
    reply: turn ? toReply(turn, running) : null,
    running,
  };
}

export function toolsFromTurn(turn: TurnDto | null): Record<string, number> {
  if (!turn) return {};
  const counts: Record<string, number> = {};
  for (const item of turn.items as ItemDto[]) {
    if (item.kind === "commandExecution") counts.Run = (counts.Run ?? 0) + 1;
    if (item.kind === "search") counts.Search = (counts.Search ?? 0) + 1;
    if (item.kind === "fileRead") counts.Read = (counts.Read ?? 0) + 1;
    if (item.kind === "modelCall") counts.Model = (counts.Model ?? 0) + 1;
    if (item.kind === "fileChange") counts.Edit = (counts.Edit ?? 0) + 1;
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
        input_tokens: String(item.inputTokens),
        output_tokens: String(item.outputTokens),
        duration: item.duration != null ? `${item.duration}ms` : "—",
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
