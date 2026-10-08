import { Check, CircleAlert, Copy, RefreshCw, Square } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { loadFileSpan, loadLlmIo, type FileSpanDto, type LlmIoDto } from "../api";
import type { DeliveryStatus, VerificationStatus } from "../api";
import { T } from "../i18n";
import { AgentTrace } from "./AgentTrace";
import { ChangedFilesSummary } from "./ChangedFilesSummary";
import { FileSpanModal } from "./FileSpanModal";
import { LlmIoViewer } from "./LlmIoViewer";
import { Markdown } from "./Markdown";
import { sanitizeAssistantText, type FailureGroup, type Reply } from "./trace";

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
  /** Re-run after fixing the environment (missing tools, permissions). */
  onRetryEnvironment?: (() => void) | null;
  /** Open the full run log (运行详情). */
  onOpenLogs?: (() => void) | null;
  /** Unified diffs for file-change steps. */
  fileDiffs?: Record<string, string> | null;
  /** Undo this turn's Kodo edits. Omitted in demo / while a run is live. */
  onUndoChanges?: (() => void) | null;
  undoingChanges?: boolean;
  /** Paths the user had already modified before this turn. */
  preExisting?: string[];
  /** Paths whose working tree diverged from Kodo's after-hash. */
  conflicts?: string[];
  /** Session id + current turn index so the trace-modal viewers can call the
   *  trace commands. Omitted on demo / pre-capture turns — AgentTrace then
   *  hides its "查看 I/O / patch 前文件" affordances. */
  sessionId?: string | null;
  turnSeq?: number | null;
};

/**
 * One answer, read conclusion-first: what happened, what was delivered, whether
 * verification held, what changed, and what to do next. The trace above is the
 * story of how; this block is the result. Answer state and verification state
 * are separate — a usable answer with a blocked check is not a failed answer.
 */
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
  onRetryEnvironment = null,
  onOpenLogs = null,
  fileDiffs = null,
  onUndoChanges = null,
  undoingChanges = false,
  preExisting = [],
  conflicts = [],
  sessionId = null,
  turnSeq = null,
}: Props) {
  const [copied, setCopied] = useState(false);
  const [answerExpanded, setAnswerExpanded] = useState(false);
  const [ignoredVerification, setIgnoredVerification] = useState(false);
  const [openLlmIoRef, setOpenLlmIoRef] = useState<string | null>(null);
  const [llmIo, setLlmIo] = useState<LlmIoDto | null>(null);
  const [llmIoLoading, setLlmIoLoading] = useState(false);
  const [openSpanRef, setOpenSpanRef] = useState<string | null>(null);
  const [span, setSpan] = useState<FileSpanDto | null>(null);
  const [spanLoading, setSpanLoading] = useState(false);
  const copyTimer = useRef<number | null>(null);

  useEffect(() => {
    if (!openLlmIoRef || !sessionId || turnSeq === null) {
      setLlmIo(null);
      return;
    }
    let alive = true;
    setLlmIoLoading(true);
    void loadLlmIo(sessionId, turnSeq, openLlmIoRef).then((data) => {
      if (!alive) return;
      setLlmIo(data);
      setLlmIoLoading(false);
    });
    return () => {
      alive = false;
    };
  }, [openLlmIoRef, sessionId, turnSeq]);

  useEffect(() => {
    if (!openSpanRef || !sessionId || turnSeq === null) {
      setSpan(null);
      return;
    }
    let alive = true;
    setSpanLoading(true);
    void loadFileSpan(sessionId, turnSeq, openSpanRef).then((data) => {
      if (!alive) return;
      setSpan(data);
      setSpanLoading(false);
    });
    return () => {
      alive = false;
    };
  }, [openSpanRef, sessionId, turnSeq]);

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
    ? T.reply.waitingApproval
    : progress
      ? `${publicPhase(progress.phase)}${progress.detail ? ` · ${progress.detail}` : ""}`
      : running
        ? elapsed != null
          ? `${T.reply.working}… ${formatElapsed(elapsed)}`
          : `${T.reply.working}...`
        : null;

  // Streaming text is a draft; the final answer replaces it in place once the
  // turn completes, so the two never render together.
  const showDraft = running && streamText.length > 0;
  const showFinal = Boolean(reply.final) && !showDraft;
  const finalText = reply.final ?? "";
  const { summary, rest } = summarizeAnswer(finalText);
  const answerBody = answerExpanded ? finalText : summary;

  const retry = onRetryEnvironment ?? onRegenerate;
  const outcomeWord = T.outcome[reply.status.outcome];
  const axesLine = `${T.reply.answerState(T.delivery[reply.status.delivery])} · ${T.reply.verifyState(
    T.verification[reply.status.verification],
  )}`;

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
            aria-label={copied ? T.action.copied : T.action.copy}
            data-testid="reply-copy"
            title={copied ? T.action.copied : T.action.copy}
            onClick={() => void copyFinal()}
          >
            {copied ? <Check size={13} strokeWidth={2.2} /> : <Copy size={13} strokeWidth={1.8} />}
          </button>
        )}
        {!running && onRegenerate && (reply.final || reply.stopped || reply.error || reply.interrupted) && (
          <button
            type="button"
            className="icon-btn icon-btn--sm reply-action"
            aria-label={T.action.regenerate}
            data-testid="reply-regenerate"
            title={T.action.regenerate}
            onClick={onRegenerate}
          >
            <RefreshCw size={13} strokeWidth={1.8} />
          </button>
        )}
      </div>

      {statusLine && (
        <p className="reply-working" data-testid="agent-progress" aria-live="polite">
          <span className="reply-working-dot" aria-hidden />
          <span className={`status-pill status-pill--${reply.status.outcome}`} data-testid="outcome-pill">
            {outcomeWord}
          </span>
          {statusLine}
          {running && streamText ? <span className="stream-caret" aria-hidden /> : null}
        </p>
      )}

      {failovers.map((line) => (
        <p key={line} className="reply-failover" data-testid="failover-note">
          {line}
        </p>
      ))}

      {showDraft && (
        <div className="final stream-preview" data-testid="stream-preview">
          <span className="draft-badge" data-testid="draft-badge">
            {T.reply.draft}
          </span>
          <p className="final-text">{sanitizeAssistantText(streamText)}</p>
        </div>
      )}

      {reply.stopped && (
        <p className="reply-stopped" data-testid="reply-stopped">
          <Square size={13} strokeWidth={2.2} />
          {T.reply.stopped}
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
          {T.reply.interrupted}
        </p>
      )}

      <AgentTrace
        steps={reply.steps}
        fileDiffs={fileDiffs}
        onOpenLlmIo={
          sessionId && turnSeq !== null
            ? (ref) => setOpenLlmIoRef(ref)
            : null
        }
        onOpenSpan={
          sessionId && turnSeq !== null
            ? (ref) => setOpenSpanRef(ref)
            : null
        }
      />

      {showFinal && (
        <div className="final">
          {/* 1. 结论 — the high-level result and the two independent axes. */}
          <section className="answer-conclusion" data-testid="answer-conclusion">
            <h3 className="answer-section-head">{T.reply.conclusion}</h3>
            <div className="answer-conclusion-row">
              <span
                className={`status-pill status-pill--${reply.status.outcome}`}
                data-testid="final-outcome-pill"
              >
                {outcomeWord}
              </span>
              <span className="answer-axes" data-testid="answer-axes">
                {axesLine}
              </span>
            </div>
          </section>

          {/* 2. 本次完成 — long answers read as a summary first. */}
          <section className="answer-body" data-testid="answer-body">
            <h3 className="answer-section-head">{T.reply.doneThisTurn}</h3>
            <Markdown text={answerBody} />
            {rest && (
              <button
                type="button"
                className="btn btn--sm answer-expand"
                data-testid="answer-expand"
                aria-expanded={answerExpanded}
                onClick={() => setAnswerExpanded((value) => !value)}
              >
                {answerExpanded ? T.action.collapseAnswer : T.action.expandAnswer}
              </button>
            )}
          </section>

          {/* 3. 验证结果 — its own card; a blocked check never cancels the answer. */}
          <VerifyCard
            reply={reply}
            ignored={ignoredVerification}
            onIgnore={() => setIgnoredVerification(true)}
            onRetryEnvironment={retry}
            onRegenerate={onRegenerate}
          />

          {/* 4. 修改文件 */}
          {!running && reply.files > 0 && (
            <ChangedFilesSummary
              files={reply.files}
              added={reply.added}
              removed={reply.removed}
              changes={reply.changes}
              preExisting={preExisting}
              conflicts={conflicts}
              onView={onViewFiles}
              onUndo={onUndoChanges}
              undoing={undoingChanges}
            />
          )}

          {/* 5. 下一步 */}
          <section className="answer-next" data-testid="answer-next">
            <h3 className="answer-section-head">{T.reply.nextSteps}</h3>
            <div className="answer-next-actions">
              <button type="button" className="btn btn--sm" data-testid="next-review" onClick={onViewFiles}>
                {T.action.review}
              </button>
              {retry && (
                <button
                  type="button"
                  className="btn btn--sm"
                  data-testid="next-retry-env"
                  onClick={retry}
                >
                  {T.action.retryEnvironment}
                </button>
              )}
              {onOpenLogs && (
                <button type="button" className="btn btn--sm" data-testid="next-logs" onClick={onOpenLogs}>
                  {T.action.viewLogs}
                </button>
              )}
            </div>
          </section>
        </div>
      )}

      {!running && (reply.tokens || reply.steps.length > 0) && (
        <div className="reply-footer" data-testid="reply-footer">
          <span>
            {T.reply.stepsCount(reply.steps.length)}
            {reply.tokens || reply.tokensPending ? ` · tokens ${reply.tokensPending ? T.tokens.counting : reply.tokens}` : ""}
          </span>
        </div>
      )}

      {openLlmIoRef && (
        <LlmIoModal
          capture={llmIo}
          loading={llmIoLoading}
          onClose={() => setOpenLlmIoRef(null)}
        />
      )}

      {openSpanRef && (
        <FileSpanModal
          span={span}
          loading={spanLoading}
          onClose={() => setOpenSpanRef(null)}
        />
      )}
    </article>
  );
}

/**
 * Inline modal wrapper around [`LlmIoViewer`] — the viewer is built for the
 * trace page's two-column layout, so on the conversation we frame it in the
 * modal chrome from the rest of the transparency UI.
 */
function LlmIoModal({
  capture,
  loading,
  onClose,
}: {
  capture: LlmIoDto | null;
  loading: boolean;
  onClose: () => void;
}) {
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="modal modal--wide" role="dialog" aria-modal="true" aria-label={T.page.llmIo.title}>
        <header className="modal-head">
          <h3 className="modal-title">{T.page.llmIo.title}</h3>
          <button
            type="button"
            className="icon-btn"
            aria-label={T.action.close}
            onClick={onClose}
          >
            ×
          </button>
        </header>
        <div className="modal-body modal-body--scroll">
          {loading || !capture ? (
            <p className="empty-note">{T.page.loadingTrace}</p>
          ) : (
            <LlmIoViewer
              artifacts={[{ kind: "llm_io", seq: capture.seq, label: `llm_io/${String(capture.seq).padStart(3, "0")}.json`, sizeBytes: 0, at: capture.at }]}
              activeIndex={0}
              onSelect={() => undefined}
              capture={capture}
              loading={false}
            />
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * Verification as a card: counts first, then each merged root-cause group with
 * its raw commands behind 查看失败命令, then the recovery actions.
 */
function VerifyCard({
  reply,
  ignored,
  onIgnore,
  onRetryEnvironment,
  onRegenerate,
}: {
  reply: Reply;
  ignored: boolean;
  onIgnore: () => void;
  onRetryEnvironment: (() => void) | null;
  onRegenerate: (() => void) | null;
}) {
  const { verify, checks, failureGroups, status } = reply;
  const hasContent = checks.length > 0 || failureGroups.length > 0 || verify.passed > 0 || verify.failed > 0 || verify.blocked > 0;
  if (!hasContent) return null;

  return (
    <section className="verify-card" data-testid="verify-card" aria-label={T.reply.verificationCard}>
      <header className="verify-card-head">
        <h3 className="answer-section-head">{T.reply.verificationCard}</h3>
        <span
          className={`status-pill status-pill--verify-${status.verification}`}
          data-testid="verify-pill"
        >
          {T.verification[status.verification as VerificationStatus] ?? T.verification.not_run}
        </span>
      </header>

      <p className="verify-summary" data-testid="verify-summary">
        {T.verify.summary(verify.passed, verify.failed, verify.blocked)}
      </p>

      {checks.length > 0 && (
        <ul className="checks-list" data-testid="checks-list">
          {checks.map((check) => (
            <li key={check}>{check}</li>
          ))}
        </ul>
      )}

      {failureGroups.map((group) => (
        <FailureGroupCard key={`${group.cls}-${group.tool ?? "n"}`} group={group} />
      ))}

      {ignored ? (
        <p className="reply-note" data-testid="verification-ignored" role="status">
          {T.reply.verificationIgnored}
        </p>
      ) : (
        failureGroups.length > 0 && (
          <div className="failure-actions" data-testid="failure-actions">
            {onRetryEnvironment && (
              <button
                type="button"
                className="btn btn--primary btn--sm"
                data-testid="retry-environment"
                onClick={onRetryEnvironment}
              >
                {T.action.retryEnvironment}
              </button>
            )}
            <button
              type="button"
              className="btn btn--sm"
              data-testid="ignore-verification"
              onClick={onIgnore}
            >
              {T.action.ignoreVerification}
            </button>
            {onRegenerate && (
              <button
                type="button"
                className="btn btn--sm"
                data-testid="verify-regenerate"
                onClick={onRegenerate}
              >
                {T.action.regenerate}
              </button>
            )}
          </div>
        )
      )}
    </section>
  );
}

function FailureGroupCard({ group }: { group: FailureGroup }) {
  const [showCommands, setShowCommands] = useState(false);
  const commandsId = `failure-commands-${group.cls}`;
  return (
    <div className="failure-group" data-testid={`failure-group-${group.cls}`}>
      <p className="failure-label" data-testid="failure-label">
        {group.label}
      </p>
      <p className="failure-reason">{T.reply.reason(group.reason)}</p>
      <p className="failure-advice">{T.reply.advice(group.recovery)}</p>
      {group.commands.length > 0 && (
        <>
          <button
            type="button"
            className="failure-toggle"
            data-testid="failure-toggle-commands"
            aria-expanded={showCommands}
            aria-controls={commandsId}
            onClick={() => setShowCommands((value) => !value)}
          >
            {T.action.viewFailedCommands}
          </button>
          {showCommands && (
            <pre className="trace-output failure-commands" id={commandsId} data-testid="failure-commands">
              {group.commands.join("\n")}
            </pre>
          )}
        </>
      )}
    </div>
  );
}

/** Structured phase codes map to public phase labels; anything else shows through. */
function publicPhase(phase: string): string {
  const known = T.phase as Record<string, string>;
  return known[phase] ?? phase;
}

/**
 * Long answers read as a summary first: paragraphs up to a soft cap, with the
 * full text behind 展开完整回答. The cut is on a paragraph boundary so the
 * summary never ends mid-line.
 */
const ANSWER_SUMMARY_CHARS = 480;

function summarizeAnswer(text: string): { summary: string; rest: string | null } {
  if (!text || text.length <= ANSWER_SUMMARY_CHARS) return { summary: text, rest: null };
  const paragraphs = text.split(/\n{2,}/);
  let summary = "";
  for (const para of paragraphs) {
    if (summary && summary.length + para.length + 2 > ANSWER_SUMMARY_CHARS) break;
    summary = summary ? `${summary}\n\n${para}` : para;
  }
  if (!summary) summary = text.slice(0, ANSWER_SUMMARY_CHARS);
  return { summary, rest: text };
}

function formatElapsed(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

/** Kept for callers that only need the delivery word. */
export function deliveryLabel(delivery: DeliveryStatus): string {
  return T.delivery[delivery];
}
