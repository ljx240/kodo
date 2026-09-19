import { BarChart3, Copy, Sparkles } from "lucide-react";
import { useEffect, useState } from "react";
import { formatDuration } from "../conversation/trace";
import { llmCalls, responseMeta } from "../data/demo";
import type { LiveSnapshot } from "../data/liveContext";
import { llmFromTurn } from "../data/liveContext";
import { MetaRow, Section } from "./Section";
import { ChangedFilesSection } from "./ResponseInspector";

export function LlmCallsDetailed({ demo, live }: { demo: boolean; live: LiveSnapshot | null }) {
  const calls = !demo
    ? llmFromTurn(live?.turn ?? null)
    : llmCalls.map((call) => ({
        model: call.model,
        input_tokens: call.inputTokens,
        output_tokens: call.outputTokens,
        duration: call.duration,
      }));
  return (
    <Section icon={<Sparkles size={14} strokeWidth={1.7} />} title="LLM calls" count={calls.length}>
      <div className="ins-body">
        {calls.length === 0 ? (
          <p className="ins-note">暂无模型调用。</p>
        ) : (
          calls.map((call, index) => (
            <div key={`${call.model}-${index}`} className="llm-call">
              <div className="llm-head">
                <span className="llm-index">{index + 1}</span>
                <Sparkles size={14} strokeWidth={1.7} className="llm-icon" />
                <span className="llm-model">{call.model}</span>
                <span className="llm-time">{call.duration}</span>
              </div>
              <MetaRow label="Input tokens">{call.input_tokens}</MetaRow>
              <MetaRow label="Output tokens">{call.output_tokens}</MetaRow>
              <MetaRow label="Duration">{call.duration}</MetaRow>
            </div>
          ))
        )}
      </div>
    </Section>
  );
}

function MetadataSection({ demo, live }: { demo: boolean; live: LiveSnapshot | null }) {
  return (
    <Section icon={<BarChart3 size={14} strokeWidth={1.7} />} title="Metadata">
      <div className="ins-body">
        <MetaRow label="Model">{!demo ? live?.reply?.steps.length ? "见 LLM 卡片" : "—" : responseMeta.model}</MetaRow>
        <MetaRow label="Total steps">{!demo ? (live?.reply?.steps.length ?? 0) : responseMeta.totalSteps}</MetaRow>
        <MetaRow label="Workspace">{!demo ? live?.projectPath || "—" : responseMeta.workspace}</MetaRow>
      </div>
    </Section>
  );
}

export function TraceOverview({ demo, live }: { demo: boolean; live: LiveSnapshot | null }) {
  return (
    <>
      <ChangedFilesSection demo={demo} live={live} />
      <LlmCallsDetailed demo={demo} live={live} />
      <MetadataSection demo={demo} live={live} />
    </>
  );
}

/** Left-hand metadata block of the Response Trace header. */
export function ReplyMeta({
  demo,
  live,
}: {
  demo: boolean;
  live: LiveSnapshot | null;
}) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1200);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const turn = live?.turn ?? null;
  const modelCalls = turn ? llmFromTurn(turn) : [];
  const lastModel = modelCalls[modelCalls.length - 1];
  const firstAt = turn?.items[0]?.at ?? null;
  const lastAt = turn?.items[turn.items.length - 1]?.at ?? null;
  const durationMs =
    firstAt != null && lastAt != null && lastAt >= firstAt ? (lastAt - firstAt) * 1000 : null;

  const replyId = demo ? responseMeta.replyId : live?.conversationId ?? "—";
  const startedAt = demo
    ? responseMeta.startedAt
    : firstAt != null
      ? new Date(firstAt * 1000).toLocaleString()
      : "—";
  const duration = demo
    ? responseMeta.duration
    : live?.running
      ? "进行中"
      : durationMs != null
        ? formatDuration(durationMs)
        : "—";
  const model = demo ? responseMeta.model : lastModel?.model ?? "—";

  return (
    <dl className="reply-meta">
      <div className="reply-meta-row">
        <dt>Reply ID</dt>
        <dd>
          <code className="code-chip">{replyId}</code>
          <button
            type="button"
            className="icon-btn icon-btn--sm"
            aria-label="Copy reply id"
            onClick={() => {
              const value = replyId;
              if (navigator.clipboard?.writeText) {
                void navigator.clipboard.writeText(value);
              }
              setCopied(true);
            }}
          >
            <Copy size={13} strokeWidth={1.8} />
          </button>
          {copied && <span className="copied-hint">已复制</span>}
        </dd>
      </div>
      <div className="reply-meta-row">
        <dt>Started at</dt>
        <dd>{startedAt}</dd>
      </div>
      <div className="reply-meta-row">
        <dt>Duration</dt>
        <dd>{duration}</dd>
      </div>
      <div className="reply-meta-row">
        <dt>Model</dt>
        <dd>{model}</dd>
      </div>
    </dl>
  );
}
