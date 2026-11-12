import type { ContextBudget, ContextTrim, ItemDto, TurnDto } from "../api";

type Adjustment =
  | { kind: "compaction"; dropped: number; summarizedInto: number }
  | { kind: "trim"; trim: ContextTrim };

type HistoryAdjustment = Adjustment & {
  key: string;
  turn: number;
  nextRequest?: { key: string; number: number };
};

type Usage = ReturnType<typeof contextUsage>;
type ContextRequest = {
  key: string;
  turn: number;
  request: number;
  budget: ContextBudget;
  usage: Usage;
  inputDelta: number | null;
  previousComposition: Usage["composition"];
  call?: { outputTokens: number; duration: number | null };
  adjusted: boolean;
};

function requestBudget(item: ItemDto): ContextBudget | undefined {
  if (item.kind === "contextBudget") return item;
  if (item.kind !== "reasoning" || item.phase !== "budget") return;
  // The previous logger encoded these numeric fields in a fixed-format summary.
  const match = /^context (\d+)% \((\d+)\/(\d+)\) after (\d+) chars, (\d+) compactions, model=(.*)$/s.exec(item.summary);
  if (!match) return;
  const [, percent, used, window, cumulativeChars, compactedCount, lastModel] = match;
  return {
    used: Number(used),
    window: Number(window),
    percent: Number(percent),
    cumulativeChars: Number(cumulativeChars),
    compactedCount: Number(compactedCount),
    lastModel,
  };
}

export function latestContextBudget(turns: TurnDto[]): ContextBudget | null {
  for (let turn = turns.length - 1; turn >= 0; turn--) {
    const items = turns[turn].items;
    for (let index = items.length - 1; index >= 0; index--) {
      const budget = requestBudget(items[index]);
      if (budget) return budget;
    }
  }
  return null;
}

function contextAdjustment(item: ItemDto): Adjustment | undefined {
  if (item.kind === "compaction") {
    return { kind: "compaction", dropped: item.dropped, summarizedInto: item.summarizedInto };
  }
  if (item.kind === "contextTrim") return { kind: "trim", trim: item.trim };
  if (item.kind !== "reasoning" || item.phase !== "compact") return;
  const match = /^compacted \d+ messages into (\d+) \((\d+) dropped, \d+% freed\)$/.exec(item.summary);
  if (!match) return;
  return { kind: "compaction", summarizedInto: Number(match[1]), dropped: Number(match[2]) };
}

export function contextHistory(turns: TurnDto[]) {
  const requests: ContextRequest[] = [];
  const adjustments: HistoryAdjustment[] = [];
  for (const [index, turn] of turns.entries()) {
    let requestNumber = 0;
    let previous: ContextRequest | undefined;
    const pending: HistoryAdjustment[] = [];
    for (const [position, item] of turn.items.entries()) {
      const budget = requestBudget(item);
      const key = `${index}:${item.id}`;
      if (!budget) {
        if (item.kind === "reasoning" && item.phase === "failover") previous = undefined;
        const event = contextAdjustment(item);
        if (event) {
          const adjustment = { ...event, key, turn: index + 1 };
          adjustments.push(adjustment);
          pending.push(adjustment);
        }
        continue;
      }

      const usage = contextUsage(budget);
      const comparable = previous &&
        previous.budget.lastModel === budget.lastModel &&
        previous.budget.window === budget.window &&
        previous.usage.source === usage.source ? previous.usage : undefined;
      const priorComposition = comparable?.composition;
      const previousComposition = usage.composition && priorComposition &&
        Boolean(usage.composition.injections) === Boolean(priorComposition.injections) ? priorComposition : null;
      const inputDelta = comparable?.used != null && usage.used !== null &&
        (usage.source === "reported" || previousComposition) ? usage.used - comparable.used : null;

      // ModelCall completion immediately precedes its snapshot in the turn log.
      const call = turn.items[position - 1];
      const matchingCall = call?.kind === "modelCall" && call.status === "done" &&
        call.model === budget.lastModel && call.inputTokens === budget.used;
      requestNumber += 1;
      const reading: ContextRequest = {
        key,
        turn: index + 1,
        request: requestNumber,
        budget,
        usage,
        inputDelta,
        previousComposition,
        call: matchingCall ? { outputTokens: call.outputTokens, duration: call.duration } : undefined,
        adjusted: pending.length > 0,
      };
      for (const adjustment of pending) adjustment.nextRequest = { key, number: requestNumber };
      pending.length = 0;
      requests.push(reading);
      previous = reading;
    }
  }
  return { requests, adjustments };
}

export function contextUsage(budget: ContextBudget) {
  let composition = null;
  if (budget.composition) {
    const { injections, ...parts } = budget.composition;
    composition = { parts, injections, total: Object.values(parts).reduce((sum, tokens) => sum + tokens, 0) };
  }
  const source: "reported" | "estimated" | null = budget.used > 0 ? "reported" : composition ? "estimated" : null;
  const used = budget.used > 0 ? budget.used : composition?.total ?? null;
  const percent = used === null || budget.window === 0
    ? null
    : budget.used > 0 ? budget.percent : Math.min(100, Math.round(used / budget.window * 100));
  return { composition, used, percent, source };
}
