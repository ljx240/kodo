import { BarChart3, Check, Folder, History, Minus, RefreshCw, Sparkles, Square, Terminal, Undo2, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { turnChanges, undoTurn, type TurnChangeDto } from "../api";
import type { DemoState } from "../data/demoState";
import type { LiveSnapshot } from "../data/liveContext";
import { changesFromReply, commandsFromTurn, llmFromTurn, toolsFromTurn } from "../data/liveContext";
import { formatDuration, type OutcomeStatus, type Reply, type RunStatus } from "../conversation/trace";
import { T } from "../i18n";
import { FileRow, MetaRow, Section, ToolRow } from "./Section";

export type InspectorData = {
  /** Demo routes pass the fixture bundle; live routes pass null. */
  demo: boolean;
  demoState?: DemoState | null;
  live: LiveSnapshot | null;
};

function EmptyBody({ note }: { note: string }) {
  return (
    <div className="ins-body">
      <p className="ins-note">{note}</p>
    </div>
  );
}

/**
 * The single high-level status. The word carries the state — color is only a
 * reinforcement, never the signal itself.
 */
function StatusPill({ outcome }: { outcome: OutcomeStatus }) {
  const mark =
    outcome === "completed" ? (
      <Check size={9} strokeWidth={4} />
    ) : outcome === "working" || outcome === "queued" || outcome === "awaiting_approval" ? (
      "…"
    ) : outcome === "stopped" ? (
      <Square size={8} strokeWidth={2.5} />
    ) : outcome === "interrupted" ? (
      <Minus size={10} strokeWidth={3} />
    ) : outcome === "failed" || outcome === "blocked_by_environment" || outcome === "partially_completed" ? (
      <X size={9} strokeWidth={3} />
    ) : null;

  return (
    <span className={`status-pill status-pill--${outcome}`} data-testid="ins-status-pill">
      <span className="status-mark">{mark}</span>
      <span>{T.outcome[outcome]}</span>
    </span>
  );
}

/** Answer / verification words for the summary rows. */
function axisText(status: RunStatus): { answer: string; verify: string } {
  return {
    answer: T.delivery[status.delivery],
    verify: T.verification[status.verification],
  };
}

function filesText(reply: Reply | null): string {
  const files = reply?.files ?? 0;
  if (files > 0) return T.inspector.filesModified(files);
  return T.inspector.filesNone;
}

function verifyText(reply: Reply | null): string {
  if (!reply) return T.inspector.verifyWaiting;
  const { verify, status } = reply;
  if (status.verification === "running") return T.verification.running;
  if (verify.blocked > 0) return T.inspector.verifyBlocked(verify.blocked);
  if (verify.failed > 0) return T.inspector.verifyFailed(verify.failed);
  if (verify.passed > 0) return T.inspector.verifyPassed(verify.passed);
  return T.inspector.verifyNotRun;
}

function stepsText(reply: Reply | null, running: boolean): string {
  const steps = reply?.steps.length ?? 0;
  if (steps > 0) return T.reply.stepsCount(steps);
  return running ? T.inspector.stepsPending : T.reply.stepsCount(0);
}

function tokensText(reply: Reply | null): string {
  if (!reply || reply.tokensPending) return T.reply.statsPending;
  return reply.tokens ?? T.tokens.pending;
}

/** Real per-file diffs + safe undo for the open session's Kodo changes. */
function LiveDiffPanel({ sessionId, projectPath }: { sessionId: string; projectPath: string }) {
  const [changes, setChanges] = useState<TurnChangeDto[] | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [undoing, setUndoing] = useState(false);

  const reload = useCallback(() => {
    if (!projectPath || !sessionId) return;
    setLoadError(null);
    turnChanges(projectPath, sessionId)
      .then((list) => {
        setChanges(list ?? []);
      })
      .catch((error: unknown) => {
        setChanges(null);
        setLoadError(error instanceof Error ? error.message : String(error));
      });
  }, [projectPath, sessionId]);

  useEffect(() => {
    setChanges(null);
    setOpen(null);
    setMessage(null);
    setLoadError(null);
    setUndoing(false);
    reload();
  }, [reload]);

  const undo = () => {
    if (!projectPath || !sessionId || undoing) return;
    setMessage(null);
    setUndoing(true);
    undoTurn(projectPath, sessionId)
      .then((report) => {
        if (!report) {
          setMessage(T.files.undoConflict);
          return;
        }
        const parts: string[] = [];
        if (report.restored.length) {
          parts.push(T.files.undoDone);
        }
        if (report.conflicts.length) {
          const details = report.conflicts
            .map((c) => `${c.path}（${c.reason}）`)
            .join("、");
          parts.push(`${T.files.conflictCount(report.conflicts.length)}：${details} — ${T.files.undoSafe}`);
        }
        if (!parts.length) {
          parts.push(T.files.undoDone);
        }
        setMessage(parts.join(" · "));
        reload();
      })
      .catch(() => setMessage(T.files.undoConflict))
      .finally(() => setUndoing(false));
  };

  if (!projectPath || !sessionId) return null;
  if (loadError) {
    return (
      <div className="ins-body">
        <p className="ins-note" data-testid="diff-error" role="alert">
          {T.reply.reason(loadError)}
        </p>
        <button type="button" className="btn btn--sm" data-testid="diff-retry" onClick={reload}>
          {T.action.regenerate}
        </button>
      </div>
    );
  }
  if (!changes) {
    return (
      <div className="ins-body">
        <p className="ins-note" data-testid="diff-loading">
          {T.reply.statsPending}
        </p>
      </div>
    );
  }
  if (changes.length === 0) {
    return (
      <div className="ins-body">
        <p className="ins-note" data-testid="diff-empty">
          {T.inspector.filesNone}
        </p>
      </div>
    );
  }
  return (
    <>
      <div className="ins-body" data-testid="diff-list">
        {changes.map((change) => (
          <div key={change.path} className="turn-change">
            <button
              type="button"
              className="ins-link turn-change-head"
              aria-expanded={open === change.path}
              data-testid={`diff-toggle-${change.path}`}
              onClick={() => setOpen((cur) => (cur === change.path ? null : change.path))}
            >
              {change.path}
              {change.userPreexisting ? ` · ${T.files.preExisting}` : ""}
              {change.conflict ? ` · ${T.files.conflicts}` : ""}
              {!change.conflict && !change.userPreexisting ? ` · ${T.files.kodoEdits}` : ""}
            </button>
            {change.conflict && (
              <p className="ins-note" data-testid={`conflict-${change.path}`}>
                {T.files.undoConflict}
              </p>
            )}
            {open === change.path && (
              <pre className="terminal-block diff-block" data-testid={`diff-body-${change.path}`}>
                {change.diff || T.tokens.pending}
              </pre>
            )}
          </div>
        ))}
        {message && (
          <p className="ins-note" data-testid="undo-message" role="status">
            {message}
          </p>
        )}
      </div>
      <div className="ins-body">
        <button
          type="button"
          className="btn btn--sm"
          data-testid="undo-kodo"
          disabled={undoing}
          title={undoing ? T.action.undoing : T.files.undoSafe}
          onClick={undo}
        >
          <Undo2 size={13} strokeWidth={1.8} />
          <span>{undoing ? T.action.undoing : T.inspector.undoSafe}</span>
        </button>
      </div>
    </>
  );
}

/**
 * Summary first: 当前结果 / 回答 / 文件 / 验证, then the recovery actions.
 * While a run is live the rows say 统计中… / 尚未产生文件修改 / 等待步骤完成 —
 * never a bare 0.
 */
export function SummarySection({
  demo,
  demoState,
  live,
  onShowAllFiles,
}: InspectorData & { onShowAllFiles?: () => void }) {
  const [showFailures, setShowFailures] = useState(false);
  const fixture = demo ? demoState?.conversation : undefined;

  if (!demo && live) {
    const reply = live.reply;
    if (!live.turn && !live.running) {
      return (
        <Section icon={<History size={14} strokeWidth={1.7} />} title={T.inspector.thisResponse}>
          <EmptyBody note={T.inspector.notStarted} />
        </Section>
      );
    }
    const status: RunStatus = reply?.status ?? {
      lifecycle: live.running ? "working" : "interrupted",
      delivery: live.running ? "partial" : "failed",
      verification: "running",
      outcome: live.running ? "working" : "interrupted",
    };
    const axes = axisText(status);
    const failures = reply?.failureGroups ?? [];
    return (
      <Section
        icon={<History size={14} strokeWidth={1.7} />}
        title={T.inspector.thisResponse}
        meta={<span className="ins-time">{live.running ? T.reply.inProgress : ""}</span>}
      >
        <div className="ins-body">
          <MetaRow label={T.inspector.currentResult}>
            <StatusPill outcome={status.outcome} />
          </MetaRow>
          <MetaRow label={T.inspector.answerState}>{axes.answer}</MetaRow>
          <MetaRow label={T.inspector.filesState}>{filesText(reply)}</MetaRow>
          <MetaRow label={T.inspector.verificationState}>{verifyText(reply)}</MetaRow>
          <MetaRow label={T.inspector.conversation}>{live.title}</MetaRow>
          <MetaRow label={T.inspector.totalSteps}>{stepsText(reply, live.running)}</MetaRow>
          <MetaRow label={T.inspector.totalTokens}>{tokensText(reply)}</MetaRow>
        </div>
        <div className="ins-body ins-actions" data-testid="ins-summary-actions">
          {failures.length > 0 && (
            <button
              type="button"
              className="btn btn--sm"
              data-testid="ins-view-failures"
              aria-expanded={showFailures}
              onClick={() => setShowFailures((value) => !value)}
            >
              {T.action.viewFailures}
            </button>
          )}
          {onShowAllFiles && (reply?.files ?? 0) > 0 && (
            <button
              type="button"
              className="btn btn--sm"
              data-testid="ins-review-files"
              onClick={onShowAllFiles}
            >
              {T.action.review}
            </button>
          )}
          {live.onUndo && !live.running && (
            <button
              type="button"
              className="btn btn--sm"
              data-testid="ins-undo"
              disabled={live.undoing}
              title={live.undoing ? T.action.undoing : T.files.undoSafe}
              onClick={live.onUndo}
            >
              <Undo2 size={13} strokeWidth={1.8} />
              <span>{live.undoing ? T.action.undoing : T.action.undo}</span>
            </button>
          )}
          {live.onRetry && !live.running && (
            <button
              type="button"
              className="btn btn--sm"
              data-testid="ins-regenerate"
              onClick={live.onRetry}
            >
              <RefreshCw size={13} strokeWidth={1.8} />
              <span>{T.action.regenerate}</span>
            </button>
          )}
        </div>
        {showFailures && failures.length > 0 && (
          <div className="ins-body" data-testid="ins-failures">
            {failures.map((group) => (
              <div key={`${group.cls}-${group.tool ?? "n"}`} className="failure-group">
                <p className="failure-label">{group.label}</p>
                <p className="failure-reason">{T.reply.reason(group.reason)}</p>
                <p className="failure-advice">{T.reply.advice(group.recovery)}</p>
              </div>
            ))}
          </div>
        )}
      </Section>
    );
  }
  if (!demo && !live) {
    return (
      <Section icon={<History size={14} strokeWidth={1.7} />} title={T.inspector.thisResponse}>
        <EmptyBody note={T.inspector.noSession} />
      </Section>
    );
  }
  if (!fixture) {
    return (
      <Section icon={<History size={14} strokeWidth={1.7} />} title={T.inspector.thisResponse}>
        <EmptyBody note={T.inspector.notStarted} />
      </Section>
    );
  }
  return (
    <Section
      icon={<History size={14} strokeWidth={1.7} />}
      title={T.inspector.thisResponse}
      meta={<span className="ins-time">{fixture.assistant.time}</span>}
    >
      <div className="ins-body">
        <MetaRow label={T.inspector.currentResult}>
          <StatusPill outcome="completed" />
        </MetaRow>
        <MetaRow label={T.inspector.answerState}>{T.delivery.ready}</MetaRow>
        <MetaRow label={T.inspector.filesState}>
          {T.inspector.filesModified(demoState?.summary.files_changed ?? 0)}
        </MetaRow>
        <MetaRow label={T.inspector.verificationState}>
          {T.inspector.verifyPassed(fixture.assistant.checks.length)}
        </MetaRow>
        <MetaRow label={T.inspector.totalSteps}>{T.reply.stepsCount(fixture.assistant.trace.length)}</MetaRow>
        <MetaRow label={T.inspector.duration}>{fixture.assistant.duration}</MetaRow>
        <MetaRow label={T.inspector.inputTokens}>{demoState?.summary.llm_calls[0]?.input_tokens ?? "—"}</MetaRow>
        <MetaRow label={T.inspector.outputTokens}>{demoState?.summary.llm_calls[0]?.output_tokens ?? "—"}</MetaRow>
        <MetaRow label={T.inspector.model}>{demoState?.summary.llm_calls[0]?.model ?? "—"}</MetaRow>
      </div>
    </Section>
  );
}

export function ChangedFilesSection({
  demo,
  demoState,
  live,
  limit,
  onShowAll,
}: InspectorData & {
  limit?: number;
  /** Opens the Files tab — demo overview "Show all" must navigate, not no-op. */
  onShowAll?: () => void;
}) {
  const demoFiles = demoState?.changedFiles ?? [];
  const demoSummary = demoState?.summary;
  const files = !demo
    ? changesFromReply(live?.reply ?? null)
    : limit
      ? demoFiles.slice(0, limit)
      : demoFiles;
  const total = !demo ? files.length : demoSummary?.files_changed ?? files.length;
  const shown = limit && demo ? files.slice(0, limit) : files;

  return (
    <Section icon={<BarChart3 size={14} strokeWidth={1.7} />} title={T.inspector.changedFiles} count={total}>
      {!demo && live?.conversationId && live.projectPath ? (
        <LiveDiffPanel sessionId={live.conversationId} projectPath={live.projectPath} />
      ) : (
        <div className="ins-body">
          {!demo && files.length === 0 ? (
            <p className="ins-note">{T.inspector.filesNone}</p>
          ) : (
            shown.map((file) => <FileRow key={file.path} file={file} />)
          )}
        </div>
      )}
      {limit && demo && onShowAll && total > shown.length && (
        <button
          type="button"
          className="ins-link"
          data-testid="show-all-files"
          onClick={onShowAll}
        >
          {T.action.showAll} {total} →
        </button>
      )}
    </Section>
  );
}

export function ToolsSection({ demo, demoState, live }: InspectorData) {
  const tools = !demo ? toolsFromTurn(live?.turn ?? null) : demoState?.summary.tools_used ?? {};
  const entries = Object.entries(tools);
  return (
    <Section icon={<Terminal size={14} strokeWidth={1.7} />} title={T.inspector.toolsUsed} count={entries.length}>
      <div className="ins-body">
        {entries.length === 0 ? (
          <p className="ins-note">{T.inspector.stepsPending}</p>
        ) : (
          entries.map(([name, count]) => <ToolRow key={name} name={name} count={count} />)
        )}
      </div>
    </Section>
  );
}

export function LlmSection({ demo, demoState, live, detailed = false }: InspectorData & { detailed?: boolean }) {
  const calls = !demo ? llmFromTurn(live?.turn ?? null) : demoState?.summary.llm_calls ?? [];
  return (
    <Section icon={<Sparkles size={14} strokeWidth={1.7} />} title={T.inspector.llmCalls} count={calls.length}>
      <div className="ins-body">
        {calls.length === 0 ? (
          <p className="ins-note">{T.reply.statsPending}</p>
        ) : (
          calls.map((call) => (
            <div key={`${call.model}-${call.input_tokens}`} className="llm-call">
              <div className="llm-head">
                <Sparkles size={14} strokeWidth={1.7} className="llm-icon" />
                <span className="llm-model">{call.model}</span>
                {detailed && <span className="llm-time">{call.duration}</span>}
                {!detailed && <span className="llm-duration">{call.duration}</span>}
              </div>
              <MetaRow label={T.inspector.inputTokens}>{call.input_tokens}</MetaRow>
              <MetaRow label={T.inspector.outputTokens}>{call.output_tokens}</MetaRow>
              {detailed && <MetaRow label={T.inspector.duration}>{call.duration}</MetaRow>}
            </div>
          ))
        )}
      </div>
    </Section>
  );
}

export function CurrentProjectSection({ demo, demoState, live }: InspectorData) {
  const name = !demo ? live?.projectName || "未选择项目" : demoState?.project.name ?? "未选择项目";
  const path = !demo ? live?.projectPath || "—" : demoState?.project.path ?? "—";
  return (
    <Section icon={<Folder size={14} strokeWidth={1.7} />} title={T.inspector.currentProject}>
      <div className="ins-body">
        <div className="project-name">{name}</div>
        <div className="project-path">{path}</div>
      </div>
    </Section>
  );
}

export function TerminalSection({ demo, demoState, live }: InspectorData) {
  const runs = !demo
    ? commandsFromTurn(live?.turn ?? null)
    : (demoState?.conversation.assistant.trace ?? [])
        .filter((step) => step.output)
        .map((step) => ({ command: step.chip || step.label, output: step.output || "" }));
  return (
    <Section icon={<Terminal size={14} strokeWidth={1.7} />} title={T.inspector.terminalOutput} count={runs.length}>
      <div className="ins-body">
        {runs.length === 0 ? (
          <p className="ins-note">{T.inspector.stepsPending}</p>
        ) : (
          runs.map((run) => (
            <div key={run.command} className="terminal-run">
              <div className="terminal-cmd">
                <code className="code-chip">{run.command}</code>
              </div>
              <pre className="terminal-block">{run.output || T.tokens.pending}</pre>
            </div>
          ))
        )}
      </div>
    </Section>
  );
}

export function ResponseOverview(props: InspectorData & { onShowAllFiles?: () => void }) {
  const { onShowAllFiles, ...data } = props;
  const emptyLive = !data.demo && !data.live?.turn && !data.live?.running;
  return (
    <>
      <SummarySection {...data} onShowAllFiles={onShowAllFiles} />
      {!emptyLive && <ChangedFilesSection {...data} limit={5} onShowAll={onShowAllFiles} />}
      {!emptyLive && <ToolsSection {...data} />}
      {!emptyLive && <LlmSection {...data} />}
      <CurrentProjectSection {...data} />
    </>
  );
}

/** Kept for Trace page formatting helpers that imported formatDuration from here historically. */
export { formatDuration };
