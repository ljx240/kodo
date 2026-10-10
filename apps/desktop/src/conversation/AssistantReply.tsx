import { BarChart3, Check, CircleAlert, Copy, RefreshCw, Square } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { DeliveryStatus, VerificationStatus } from "../api";
import { T } from "../i18n";
import { AgentTrace } from "./AgentTrace";
import { ChangedFilesSummary } from "./ChangedFilesSummary";
import { Markdown } from "./Markdown";
import type { FailureGroup, Reply } from "./trace";
import { formatCompactToast, isDocumentPath, isWaitingForReply } from "./trace";

type Props = {
  reply: Reply;
  /** True while this turn's reply is the active ⌘F find hit. */
  findHit?: boolean;
  /** Position in the turn list — the TurnRail's addressing hook. */
  turnIndex?: number;
  /** True while this turn is the one being generated. */
  running: boolean;
  /** Structured progress phase from the agent (not chain-of-thought). */
  progress?: { phase: string; detail: string } | null;
  /** Seconds since the running turn started, or null when idle. */
  elapsed?: number | null;
  /** True while the runner is waiting for an approval decision. */
  waitingApproval?: boolean;
  /** One-shot toast for a compaction pass; auto-clears after the call. */
  compactToast?: { dropped: number; freedPct: number } | null;
  /** One-shot toast for an adaptive budget lift (mid-turn cap raise).
   *  Cleared by the agent after the lift event has been shown. */
  budgetLiftToast?: { detail: string } | null;
  /** One line per provider switch observed on the streaming path. */
  failovers?: string[];
  onViewFiles: () => void;
  /** Re-send the last ask (regenerate). Omitted for demo or incomplete turns. */
  onRegenerate?: (() => void) | null;
  /** Re-run after fixing the environment (missing tools, permissions). */
  onRetryEnvironment?: (() => void) | null;
  /** Start a recovery run with extended budget and the same ask, retaining
   *  existing files and conversation history after budget exhaustion. */
  onContinueTurn?: (() => void) | null;
  /** Open the Response Trace page for this run (查看响应轨迹). */
  onOpenLogs?: (() => void) | null;
  /** Unified diffs for file-change steps. */
  fileDiffs?: Record<string, string> | null;
  /** Undo this turn's Kodo edits. Omitted in demo / while a run is live. */
  onUndoChanges?: (() => void) | null;
  undoingChanges?: boolean;
  /** Result of the last undo attempt. */
  undoMessage?: string | null;
  /** Open one changed file in the file viewer. Omitted in demo. */
  onOpenFile?: ((path: string) => void) | null;
  /** Open a path mentioned in the answer's markdown (inline code) in the
   *  right sidebar. Heuristic only — the click probes before opening. */
  onOpenPath?: ((path: string) => void) | null;
  /** Paths the user had already modified before this turn. */
  preExisting?: string[];
  /** Paths whose working tree diverged from Kodo's after-hash. */
  conflicts?: string[];
};

/**
 * One answer, read conclusion-first: what happened, what was delivered, whether
 * verification held, what changed, and what to do next. The trace above is the
 * story of how; this block is the result. Answer state and verification state
 * are separate — a usable answer with a blocked check is not a failed answer.
 */
export function AssistantReply({
  reply,
  findHit = false,
  turnIndex,
  running,
  progress = null,
  elapsed = null,
  waitingApproval = false,
  compactToast = null,
  budgetLiftToast = null,
  failovers = [],
  onViewFiles,
  onRegenerate = null,
  onRetryEnvironment = null,
  onContinueTurn = null,
  onOpenLogs = null,
  fileDiffs = null,
  onUndoChanges = null,
  undoMessage = null,
  onOpenFile = null,
  onOpenPath = null,
  undoingChanges = false,
  preExisting = [],
  conflicts = [],
}: Props) {
  const [copied, setCopied] = useState(false);
  const [answerExpanded, setAnswerExpanded] = useState(false);
  const [ignoredVerification, setIgnoredVerification] = useState(false);
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
    ? T.reply.waitingApproval
    : progress
      ? `${publicPhase(progress.phase)}${progress.detail ? ` · ${progress.detail}` : ""}`
      : running
        ? elapsed != null
          ? `${T.reply.working}… ${formatElapsed(elapsed)}`
          : `${T.reply.working}...`
        : null;

  // No streaming draft up top: the answer surface appears only once the final
  // reply lands (2026-09-23). Interim prose rides in the trace in order.
  const showFinal = Boolean(reply.final);
  const finalText = reply.final ?? "";
  const { summary, rest } = summarizeAnswer(finalText);
  const answerBody = answerExpanded ? finalText : summary;

  const budgetFailure = reply.failureReason === "budget_exhausted";
  const environmentBlocked = reply.status.verification === "blocked" || reply.failureGroups.some(
    (group) => ["command_not_found", "permission_denied", "timeout"].includes(group.cls),
  );
  const retry = !budgetFailure && environmentBlocked ? (onRetryEnvironment ?? onRegenerate) : null;
  const documentsOnly = reply.changes.length > 0 && reply.changes.every((change) => isDocumentPath(change.path));
  const pendingQuestion = isWaitingForReply(reply);
  const outcomeWord = pendingQuestion ? T.reply.pendingQuestion : T.outcome[reply.status.outcome];
  const axesLine = `${documentsOnly ? T.reply.documentState[reply.status.delivery] : T.reply.answerState(T.delivery[reply.status.delivery])} · ${T.reply.verifyState(
    documentsOnly && reply.status.verification === "not_run" ? T.reply.commandVerificationNotRun : T.verification[reply.status.verification],
  )}`;
  // No speaker header (2026-09-30): left/right alignment carries who spoke.
  // Copy/regenerate/trace live in a quiet row at the reply's bottom instead.
  const canCopy = !running && Boolean(reply.final);
  const canRegenerate =
    !running &&
    !pendingQuestion &&
    Boolean(onRegenerate) &&
    Boolean(reply.final || reply.stopped || reply.error || reply.interrupted);

  return (
    <article className={findHit ? "reply find-hit" : "reply"} data-turn-index={turnIndex}>
      {statusLine && (
        <p className="reply-working" data-testid="agent-progress" aria-live="polite">
          <span className="reply-working-dot" aria-hidden />
          <span className={`status-pill status-pill--${reply.status.outcome}`} data-testid="outcome-pill">
            {outcomeWord}
          </span>
          {statusLine}
        </p>
      )}

      {/* The horizontal in-reply context bar was retired in favour of the
          Codex-style ring at the bottom of the composer (single source of
          truth). The compact toast below still surfaces compaction
          activity directly under the trace, so the user keeps visibility
          without the duplicated header bar. */}

      {compactToast ? (
        <p className="reply-compact-toast" data-testid="compact-toast" aria-live="polite">
          {formatCompactToast(compactToast.dropped, compactToast.freedPct)}
        </p>
      ) : null}

      {budgetLiftToast ? (
        <p className="reply-compact-toast reply-budget-lift-toast" data-testid="budget-lift-toast" aria-live="polite">
          {budgetLiftToast.detail}
        </p>
      ) : null}

      {failovers.map((line) => (
        <p key={line} className="reply-failover" data-testid="failover-note">
          {line}
        </p>
      ))}

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

      {/* The process stays on screen only while there is no final reply —
          running turns, and turns that ended without one. Once the answer
          lands the conversation shows the reply alone; the full process lives
          in the Response Trace page (2026-09-24). */}
      {!showFinal && (
        <AgentTrace steps={reply.steps} fileDiffs={fileDiffs} onOpenFile={onOpenFile} />
      )}

      {/* Conversational answers render as the sentence alone (2026-09-23):
          no conclusion card, no axes line, no next-step chrome. */}
      {showFinal && reply.plain && (
        <div className="final" data-testid="answer-plain">
          {!running && reply.status.outcome !== "completed" && (
            <div className="reply-outcome">
              <span className={`status-pill status-pill--${reply.status.outcome}`} data-testid="final-outcome-pill">
                {outcomeWord}
              </span>
            </div>
          )}
          <Markdown text={finalText} onOpenPath={onOpenPath} />
        </div>
      )}

      {showFinal && !reply.plain && (
        <div className="final">
          {/* 1. 结论 — the high-level result and the two independent axes. */}
          <section className="answer-conclusion" data-testid="answer-conclusion">
            {!(documentsOnly && reply.status.outcome === "completed") && <h3 className="answer-section-head">{T.reply.conclusion}</h3>}
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

          {/* 2. Result — failures and pending questions never claim completion. */}
          <section className="answer-body" data-testid="answer-body">
            <h3 className="answer-section-head">{documentsOnly && reply.status.outcome === "completed" ? T.reply.documentResult : reply.status.outcome === "completed" ? T.reply.doneThisTurn : T.reply.resultThisTurn}</h3>
            <Markdown text={answerBody} onOpenPath={onOpenPath} />
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
              readBackPaths={reply.documentReadbacks}
              preExisting={preExisting}
              conflicts={conflicts}
              onView={onViewFiles}
              onUndo={onUndoChanges}
              undoing={undoingChanges}
              undoMessage={undoMessage}
              onOpenFile={onOpenFile}
            />
          )}

          {/* 5. 下一步 */}
          {!pendingQuestion && (onContinueTurn || retry) && <section className="answer-next" data-testid="answer-next">
            <h3 className="answer-section-head">{T.reply.nextSteps}</h3>
            <div className="answer-next-actions">
              {onContinueTurn && (
                <button
                  type="button"
                  className="btn btn--sm btn--primary"
                  data-testid="next-continue"
                  title={T.action.continueTurnHint}
                  onClick={() => onContinueTurn?.()}
                >
                  {T.action.continueTurn}
                </button>
              )}
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
            </div>
          </section>}
        </div>
      )}

      {/* Step/token accounting is process metadata: it leaves with the trace
          once the final reply lands (只显示最终回复正文). */}
      {!showFinal && !running && (reply.tokens || reply.steps.length > 0) && (
        <div className="reply-footer" data-testid="reply-footer">
          <span>
            {T.reply.stepsCount(reply.steps.filter((step) => step.type !== "note").length)}
            {reply.tokens || reply.tokensPending ? ` · tokens ${reply.tokensPending ? T.tokens.counting : reply.tokens}` : ""}
          </span>
        </div>
      )}

      {/* A quiet utility row at the very bottom of the reply — always visible,
          so 复制回复 / 重新生成 / 查看响应轨迹 never hide behind hover
          (2026-09-30). Icon-only: the label moves to aria-label + title,
          same as the message copy button. It shows whichever the turn allows. */}
      {(canCopy || canRegenerate || onOpenLogs) && (
        <div className="reply-actions">
          {canCopy && (
            <button
              type="button"
              className="icon-btn icon-btn--sm"
              data-testid="reply-copy"
              aria-label={copied ? T.action.copied : T.action.copy}
              title={copied ? T.action.copied : T.action.copy}
              onClick={() => void copyFinal()}
            >
              {copied ? <Check size={13} strokeWidth={2.2} /> : <Copy size={13} strokeWidth={1.8} />}
            </button>
          )}
          {canRegenerate && (
            <button
              type="button"
              className="icon-btn icon-btn--sm"
              data-testid="reply-regenerate"
              aria-label={T.action.regenerate}
              title={T.action.regenerate}
              onClick={() => onRegenerate?.()}
            >
              <RefreshCw size={13} strokeWidth={1.8} />
            </button>
          )}
          {onOpenLogs && (
            <button
              type="button"
              className="icon-btn icon-btn--sm"
              data-testid="reply-trace"
              aria-label={T.action.viewTrace}
              title={T.action.viewTrace}
              onClick={onOpenLogs}
            >
              <BarChart3 size={13} strokeWidth={1.8} />
            </button>
          )}
        </div>
      )}
    </article>
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
  // Model checklist prose does not establish that verification ran.
  const hasContent = failureGroups.length > 0 || verify.passed > 0 || verify.failed > 0 || verify.blocked > 0;
  if (!hasContent) return null;
  const normalize = (text: string) => text.replace(/[`*_]/g, "").replace(/\s+/g, " ").trim();
  const answerLines = new Set((reply.final ?? "").split("\n").map((line) => normalize(line.replace(/^\s*[-*+]\s+(?:\[[xX ]\]\s*)?/, ""))));
  const visibleChecks = [...new Set(checks)].filter((check) => normalize(check) && !answerLines.has(normalize(check)));

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

      {visibleChecks.length > 0 && (
        <ul className="checks-list" data-testid="checks-list">
          {visibleChecks.map((check) => (
            <li key={check}><Markdown text={check} /></li>
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
