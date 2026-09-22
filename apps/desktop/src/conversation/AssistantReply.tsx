import { Check, CircleAlert, CircleCheck, Copy, RefreshCw, Square } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { AgentTrace } from "./AgentTrace";
import { ChangedFilesSummary } from "./ChangedFilesSummary";
import { Markdown } from "./Markdown";
import { sanitizeAssistantText, type Reply } from "./trace";

type Props = {
  time: string;
  reply: Reply;
  /** True while this turn is the one being generated. */
  running: boolean;
  /** Live assistant text accumulated from provider stream deltas. */
  streamText?: string;
  /** Structured progress phase from the agent (not chain-of-thought). */
  progress?: { phase: string; detail: string } | null;
  /** Seconds since the running turn started, or null when idle. */
  elapsed?: number | null;
  /** True while the runner is waiting for an approval decision. */
  waitingApproval?: boolean;
  /** One line per provider switch observed on the streaming path. */
  failovers?: string[];
  onViewFiles: () => void;
  /** Re-send the last ask (regenerate). Omitted for demo or incomplete turns. */
  onRegenerate?: (() => void) | null;
};

export function AssistantReply({
  time,
  reply,
  running,
  streamText = "",
  progress = null,
  elapsed = null,
  waitingApproval = false,
  failovers = [],
  onViewFiles,
  onRegenerate = null,
}: Props) {
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
    };
  }, []);

  const copyFinal = async () => {
    if (!reply.final || copied) return;
    try {
      await navigator.clipboard.writeText(reply.final);
      setCopied(true);
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard denied — silent; the answer stays selectable */
    }
  };

  const statusLine = waitingApproval
    ? "等待你的批准后继续执行"
    : progress
    ? `${progress.phase}${progress.detail ? ` · ${progress.detail}` : ""}`
    : running
      ? elapsed != null
        ? `正在处理您的请求… ${formatElapsed(elapsed)}`
        : "正在处理您的请求..."
      : null;

  return (
    <article className="reply">
      <div className="msg-head">
        <span className="avatar avatar--kodo">K</span>
        <span className="msg-author">Kodo</span>
        {time && <span className="msg-time">{time}</span>}
        {reply.models.length > 0 && (
          <code className="code-chip code-chip--model" data-testid="reply-model">
            {reply.models[reply.models.length - 1]}
          </code>
        )}
        <span className="spacer" />
        {!running && reply.final && (
          <button
            type="button"
            className="icon-btn icon-btn--sm reply-action"
            aria-label={copied ? "已复制" : "复制回复"}
            data-testid="reply-copy"
            title={copied ? "已复制" : "复制回复"}
            onClick={() => void copyFinal()}
          >
            {copied ? <Check size={13} strokeWidth={2.2} /> : <Copy size={13} strokeWidth={1.8} />}
          </button>
        )}
        {!running && onRegenerate && (reply.final || reply.stopped || reply.error || reply.interrupted) && (
          <button
            type="button"
            className="icon-btn icon-btn--sm reply-action"
            aria-label="重新生成"
            data-testid="reply-regenerate"
            title="重新生成"
            onClick={onRegenerate}
          >
            <RefreshCw size={13} strokeWidth={1.8} />
          </button>
        )}
      </div>

      {statusLine && (
        <p className="reply-working" data-testid="agent-progress">
          <span className="reply-working-dot" aria-hidden />
          {statusLine}
          {running && streamText ? <span className="stream-caret" aria-hidden /> : null}
        </p>
      )}

      {failovers.map((line) => (
        <p key={line} className="reply-failover" data-testid="failover-note">
          {line}
        </p>
      ))}

      {running && streamText && (
        <div className="final stream-preview" data-testid="stream-preview">
          <p className="final-text">{sanitizeAssistantText(streamText)}</p>
        </div>
      )}

      {reply.stopped && (
        <p className="reply-stopped" data-testid="reply-stopped">
          <Square size={13} strokeWidth={2.2} />
          这次运行已停止。
        </p>
      )}
      {reply.error && (
        <div className="reply-error reply-failed" role="alert" data-testid="reply-error">
          <CircleAlert size={14} strokeWidth={1.9} />
          <span>{reply.error}</span>
        </div>
      )}
      {reply.interrupted && (
        <p className="reply-interrupted" data-testid="reply-interrupted">
          这次运行中断了，最后一步没有完成。
        </p>
      )}

      <AgentTrace steps={reply.steps} />

      {reply.final && (
        <div className="final">
          <Markdown text={reply.final} />

          {reply.checks.length > 0 && (
            <section className="checks">
              <h3 className="checks-head">
                <CircleCheck size={15} strokeWidth={1.9} />
                <span>检查结果</span>
              </h3>
              <ul className="checks-list">
                {reply.checks.map((check) => (
                  <li key={check}>{check}</li>
                ))}
              </ul>
            </section>
          )}
        </div>
      )}

      {!running && (reply.tokens || reply.steps.length > 0) && (
        <div className="reply-footer" data-testid="reply-footer">
          <span>
            {reply.steps.length} 个步骤
            {reply.tokens ? ` · tokens ${reply.tokens}` : ""}
          </span>
        </div>
      )}

      {reply.files > 0 && (
        <ChangedFilesSummary
          files={reply.files}
          added={reply.added}
          removed={reply.removed}
          onView={onViewFiles}
        />
      )}
    </article>
  );
}

function formatElapsed(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}
