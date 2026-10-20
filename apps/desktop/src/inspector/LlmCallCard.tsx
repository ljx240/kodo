import { Loader2, Sparkles } from "lucide-react";
import { useEffect, useState } from "react";
import { loadLlmIo, type ItemDto, type LlmIoDto } from "../api";
import { LlmIoCaptureDetailStacked } from "../conversation/LlmIoViewer";
import { formatDuration, formatTokens } from "../conversation/trace";
import type { DemoState } from "../data/demoState";
import type { LiveSnapshot } from "../data/liveContext";
import { T } from "../i18n";
import { MetaRow } from "./Section";

/**
 * One expandable model-call record for the inspector's 模型调用 section —
 * the compact header (index, model, duration) plus token rows, with the
 * captured LLM request/response bodies lazily loaded under it.
 *
 * Shared by the conversation inspector (`ResponseInspector.LlmSection`) and
 * the response-trace inspector (`TraceInspector.LlmCallsDetailed`) so the two
 * surfaces never drift apart. Data wiring stays with the caller: this module
 * only knows how to render a summary and fetch its `llmIoRef` body.
 */
export type CallSummary = {
  model: string;
  /** Pre-formatted for display — `12.4k`, `进行中`, … — never a raw count. */
  input_tokens: string;
  output_tokens: string;
  duration: string;
  /** 1-based ordinal among this turn's model calls (the card's `#`). */
  index: number;
  /** Reference into `<state_dir>/traces/<session>/<turn>/llm_io/<seq>.json`
   *  when the agent captured the body. `null` means "no detail available". */
  llmIoRef?: string | null;
};

/**
 * Summarises every `modelCall` item of one turn. Called with the turn's
 * items (live or demo `demoState.items`) — token/duration strings are
 * formatted exactly as `llmFromTurn` formats them, so the summary panel and
 * any historical rendering agree.
 */
export function modelCallsFromItems(items: ItemDto[] | undefined): CallSummary[] {
  const summaries: CallSummary[] = [];
  if (!items) return summaries;
  let index = 0;
  for (const item of items) {
    if (item.kind !== "modelCall") continue;
    index += 1;
    summaries.push({
      model: item.model,
      input_tokens: formatTokens(item.inputTokens, item.status),
      output_tokens: formatTokens(item.outputTokens, item.status),
      duration: item.status === "running" ? T.reply.inProgress : formatDuration(item.duration),
      index,
      llmIoRef: item.llmIoRef ?? null,
    });
  }
  return summaries;
}

/** The list body shared by both sections: empty note, or one card per call. */
export function LlmCallList({ summaries, demo, demoState, live }: {
  summaries: CallSummary[];
  demo: boolean;
  demoState?: DemoState | null;
  live: LiveSnapshot | null;
}) {
  if (summaries.length === 0) {
    return <p className="ins-note">{T.inspector.noModelCalls}</p>;
  }
  return (
    <>
      {summaries.map((summary) => (
        <LlmCallCard
          key={`${summary.model}-${summary.index}`}
          summary={summary}
          demo={demo}
          demoState={demoState}
          live={live}
        />
      ))}
    </>
  );
}

/** One model-call record. The header stays compact (model + duration); the
 *  body lazily loads the captured request/response when expanded, falling
 *  back to the demo's in-memory captures or a "detail unavailable" message. */
function LlmCallCard({
  summary,
  demo,
  demoState,
  live,
}: {
  summary: CallSummary;
  demo: boolean;
  demoState?: DemoState | null;
  live: LiveSnapshot | null;
}) {
  // Uncontrolled `<details>`: the browser owns open/close so React can't
  // accidentally re-open the card on re-mount (HMR, StrictMode, or
  // a parent `key` bump). State here is only for the captured body.
  const [capture, setCapture] = useState<LlmIoDto | null>(null);
  const [loadAttempted, setLoadAttempted] = useState<boolean>(false);
  const loading = capture === null && loadAttempted === false;

  // Reset cached body when the underlying ref changes (e.g. live turn swap).
  useEffect(() => {
    setCapture(null);
    setLoadAttempted(false);
  }, [summary.llmIoRef ?? ""]);

  return (
    <details
      className="llm-call"
      data-testid={`llm-call-${summary.index}`}
      onToggle={(event) => {
        const isOpen = (event.target as HTMLDetailsElement).open;
        if (!isOpen || demo) return;
        if (loadAttempted || capture) return;
        if (!summary.llmIoRef || !live?.conversationId || live?.turnSeq == null) return;
        setLoadAttempted(true);
        void loadLlmIo(
          live.conversationId,
          live.turnSeq,
          summary.llmIoRef,
        ).then((body) => setCapture(body));
      }}
    >
      <summary className="llm-head">
        <span className="llm-index">{summary.index}</span>
        <Sparkles size={14} strokeWidth={1.7} className="llm-icon" />
        <span className="llm-model">{summary.model}</span>
        <span className="llm-time">{summary.duration}</span>
      </summary>
      <div className="llm-call-meta">
        <MetaRow label={T.inspector.inputTokens}>{summary.input_tokens}</MetaRow>
        <MetaRow label={T.inspector.outputTokens}>{summary.output_tokens}</MetaRow>
        <MetaRow label={T.inspector.duration}>{summary.duration}</MetaRow>
      </div>
      <div className="llm-call-detail" data-testid={`llm-call-detail-${summary.index}`}>
        <LlmCallDetailBody
          summary={summary}
          demo={demo}
          demoCaptures={demoState?.llmIoCaptures ?? null}
          capture={capture}
          loading={loading}
          loadAttempted={loadAttempted}
        />
      </div>
    </details>
  );
}

/** Render one model call's captured body. In demo mode the captures live in
 *  the demo state and pair by call index; in live mode the parent already
 *  loaded (or failed to load) the body for the active session/turn/ref. */
function LlmCallDetailBody({
  summary,
  demo,
  demoCaptures,
  capture,
  loading,
  loadAttempted,
}: {
  summary: CallSummary;
  demo: boolean;
  demoCaptures: LlmIoDto[] | null;
  capture: LlmIoDto | null;
  loading: boolean;
  loadAttempted: boolean;
}) {
  if (loading && !demo) {
    return (
      <p className="ins-note llm-call-status">
        <Loader2 size={12} strokeWidth={2} className="llm-call-spinner" aria-hidden />
        {T.page.loadingTrace}
      </p>
    );
  }
  const body: LlmIoDto | null = demo
    ? (() => {
        // No `llmIoRef` on demo items — pair by 1-based index against the
        // fixture list. Index 0 is for the first modelCall, etc.
        const list = demoCaptures ?? [];
        if (list.length === 0) return null;
        return list[Math.min(summary.index - 1, list.length - 1)] ?? null;
      })()
    : capture;
  if (!body) {
    if (demo || loadAttempted || !summary.llmIoRef) {
      return <p className="ins-note">{T.inspector.noModelDetail}</p>;
    }
    return <p className="ins-note">{T.page.loadingTrace}</p>;
  }
  return <LlmIoCaptureDetailStacked capture={body} />;
}
