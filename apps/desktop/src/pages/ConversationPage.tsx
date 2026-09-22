import { Archive, Folder, MoreVertical, Pencil } from "lucide-react";
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import {
  loadSession,
  onRunEvent,
  respondApproval,
  sendMessage,
  stopRun,
  turnChanges,
  type ItemDto,
  type RunEventDto,
  type TurnDto,
} from "../api";
import { AssistantReply } from "../conversation/AssistantReply";
import { Composer } from "../conversation/Composer";
import { titleFromAsk, toReply, type Reply } from "../conversation/trace";
import { type DemoState } from "../data/demoState";
import { snapshotFromTurn, type LiveSnapshot } from "../data/liveContext";
import { type ProviderConfig } from "../data/providers";
import { navigate } from "../routes";
import { Menu, MenuItem } from "../shell/Menu";

type Props = {
  conversationId: string | null;
  provider: ProviderConfig | null;
  providers: ProviderConfig[];
  onSelectProvider: (index: number) => void;
  onSelectProviderModel: (providerIndex: number, modelId: string, displayName: string) => void;
  onViewFiles: () => void;
  onRetitle: (id: string, title: string) => Promise<void> | void;
  onArchive: (id: string) => Promise<void> | void;
  onOpenTrace: () => void;
  onSnapshot?: (snapshot: LiveSnapshot | null) => void;
  onConversationCreated?: (id: string) => void;
  onConversationCommitted?: () => Promise<void> | void;
  onOpenSettings?: () => void;
  projectName?: string;
  projectPath?: string;
  /**
   * Deterministic fixture for `/ui-demo` only. Live routes pass null and must
   * never import data/fixture or data/demo themselves.
   */
  demo?: DemoState | null;
};

type Approval = {
  step: number;
  kind: string;
  detail: string;
  cwd?: string;
  riskCategory?: string;
  reason?: string;
};

/** Recoverable failure: message + optional retry / recovery action. */
type ActionErrorState = {
  message: string;
  retry?: () => void;
  retryLabel?: string;
  action?: { label: string; run: () => void };
};

type PendingSend = { text: string; context: string[] };

type FailoverNotice = {
  fromProvider: string;
  fromModel: string;
  errorClass: string;
  error: string;
  toProvider: string;
  toModel: string;
};

function demoReplyFrom(demo: DemoState): Reply {
  return {
    steps: demo.conversation.assistant.trace,
    status: "completed",
    final: demo.conversation.assistant.final,
    checks: demo.conversation.assistant.checks,
    changes: demo.changedFiles,
    files: demo.summary.files_changed,
    added: demo.summary.added,
    removed: demo.summary.removed,
    interrupted: false,
    stopped: false,
    error: null,
    models: demo.summary.llm_calls.map((call) => call.model),
    tokens: null,
  };
}

export function ConversationPage({
  conversationId,
  provider,
  providers,
  onSelectProvider,
  onSelectProviderModel,
  onViewFiles,
  onRetitle,
  onArchive,
  onOpenTrace,
  onSnapshot,
  onConversationCreated,
  onConversationCommitted,
  onOpenSettings,
  projectName = "",
  projectPath = "",
  demo = null,
}: Props) {
  const demoMode = Boolean(demo);
  const openSettings = onOpenSettings ?? navigateSettings;
  const [turns, setTurns] = useState<TurnDto[]>([]);
  const [createdConversationId, setCreatedConversationId] = useState<string | null>(null);
  const [title, setTitle] = useState("");
  const [running, setRunning] = useState(false);
  const [approval, setApproval] = useState<Approval | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [draftTitle, setDraftTitle] = useState("");
  const [contexts, setContexts] = useState<string[]>([]);
  const [pageError, setPageError] = useState<ActionErrorState | null>(null);
  const [approvalError, setApprovalError] = useState<string | null>(null);
  const [streamText, setStreamText] = useState("");
  const [progress, setProgress] = useState<{ phase: string; detail: string } | null>(null);
  const [failovers, setFailovers] = useState<string[]>([]);
  const [failover, setFailover] = useState<FailoverNotice | null>(null);
  const [queue, setQueueState] = useState<PendingSend[]>([]);
  const [runStartedAt, setRunStartedAt] = useState<number | null>(null);
  const [elapsed, setElapsed] = useState<number | null>(null);
  const [fileDiffs, setFileDiffs] = useState<Record<string, string> | null>(null);
  /** False after Stop — late TextDelta races must not repaint the preview. */
  const acceptStreamRef = useRef(true);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const stickToBottomRef = useRef(true);
  /** Queue lives in a ref so drain never double-fires under StrictMode. */
  const queueRef = useRef<PendingSend[]>([]);
  const drainingRef = useRef(false);

  const applyQueue = (next: PendingSend[]) => {
    queueRef.current = next;
    setQueueState(next);
  };

  const providerWarning = useMemo(() => {
    if (demoMode) return null;
    if (providers.length === 0) return "尚未配置 AI Provider";
    const active = provider;
    if (!active) return "当前 Provider 不可用";
    if (!active.hasKey && !(active.apiKey && active.apiKey.length > 0)) {
      return "当前 Provider 缺少 API Key";
    }
    return null;
  }, [demoMode, providers, provider]);

  useEffect(() => {
    setApproval(null);
    setPageError(null);
    setContexts([]);
    setApprovalError(null);
    setStreamText("");
    setProgress(null);
    setFailovers([]);
    setFailover(null);
    applyQueue([]);
    setRunStartedAt(null);
    setElapsed(null);
    acceptStreamRef.current = true;
    stickToBottomRef.current = true;
    if (createdConversationId && conversationId === createdConversationId) return;
    setCreatedConversationId(null);
    if (!conversationId || demoMode) {
      setTurns([]);
      setTitle("");
      setRunning(false);
      onSnapshot?.(null);
      return;
    }

    let alive = true;
    void loadSession(conversationId).then((loaded) => {
      if (!alive || !loaded) return;
      setTurns(loaded.turns);
      setTitle(loaded.title);
      // Reopened sessions start "not running". A step still marked running in
      // the log is a killed run — toReply paints that as interrupted. If the
      // driver is still live, run:event will set running again on the next step.
      setRunning(false);
    });
    return () => {
      alive = false;
    };
  }, [conversationId, demoMode, onSnapshot, createdConversationId]);

  const sessionId = conversationId ?? createdConversationId;

  // Elapsed clock for the live turn status line.
  useEffect(() => {
    if (runStartedAt === null || !running) {
      setElapsed(null);
      return;
    }
    const tick = () => setElapsed(Math.max(0, Math.floor((Date.now() - runStartedAt) / 1000)));
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [runStartedAt, running]);

  const adoptAskTitle = (ask: string) => {
    if (!sessionId || demoMode) return;
    if (title && title !== "新对话") return;
    const next = titleFromAsk(ask);
    if (!next) return;
    setTitle(next);
    void Promise.resolve(onRetitle(sessionId, next)).catch(() => {
      /* title is cosmetic; keep the turn usable if retitle fails */
    });
  };

  const dispatchSend = async (text: string, context: string[], opts?: { skipTitle?: boolean }) => {
    const payload = { text, context };
    const targetId = sessionId ?? "";
    setPageError(null);
    setTurns((current) => [...current, blank(text, context)]);
    setRunning(true);
    setRunStartedAt(Date.now());
    acceptStreamRef.current = true;
    setStreamText("");
    setProgress(null);
    stickToBottomRef.current = true;
    if (!opts?.skipTitle) adoptAskTitle(text);
    try {
      const createdId = await sendMessage(targetId, text, context, projectPath);
      if (!targetId && createdId) {
        setCreatedConversationId(createdId);
        onConversationCreated?.(createdId);
      }
      if (!createdId && !targetId) throw new Error("创建任务失败");
      await onConversationCommitted?.();
      // Context is consumed for this turn only; the user re-pins if needed.
      setContexts([]);
    } catch (failure) {
      setRunning(false);
      setRunStartedAt(null);
      const message = errorMessage(failure);
      setTurns((current) => updateLast(current, (turn) => ({ ...turn, error: message })));
      setPageError({
        message: `发送失败：${message}`,
        retryLabel: "重试发送",
        retry: () => {
          setPageError(null);
          void dispatchSend(payload.text, payload.context, { skipTitle: true });
        },
      });
    }
  };

  const drainQueue = () => {
    if (drainingRef.current) return;
    const pending = queueRef.current;
    if (pending.length === 0) return;
    drainingRef.current = true;
    const [next, ...rest] = pending;
    applyQueue(rest);
    void dispatchSend(next.text, next.context, { skipTitle: true }).finally(() => {
      drainingRef.current = false;
    });
  };

  const send = (text: string, context: string[]) => {
    if (!sessionId && !projectPath) return;
    // Running: queue the follow-up instead of dropping the draft (codex/DSH).
    if (running) {
      applyQueue([...queueRef.current, { text, context }]);
      return;
    }
    void dispatchSend(text, context);
  };

  const regenerate = () => {
    if (!sessionId || running) return;
    const last = [...turns].reverse().find((turn) => turn.ask.trim());
    if (!last) return;
    void dispatchSend(last.ask, last.context ?? [], { skipTitle: true });
  };

  const stop = () => {
    if (!sessionId) return;
    setApproval(null);
    // Stop 后不再产生用户可见 TextDelta。
    acceptStreamRef.current = false;
    setStreamText("");
    setProgress(null);
    setRunStartedAt(null);
    void stopRun(sessionId).catch((failure) => {
      setApprovalError(`停止失败：${errorMessage(failure)}`);
    });
  };

  const decide = (approved: boolean, sessionWide = false) => {
    if (!sessionId || !approval) return;
    const pending = approval;
    setApproval(null);
    setApprovalError(null);
    void respondApproval(sessionId, pending.step, approved, sessionWide).catch((failure) => {
      const label = !approved ? "拒绝" : sessionWide ? "本会话允许" : "允许";
      setApprovalError(
        `审批响应失败：${errorMessage(failure)}（${label} step ${pending.step}）`,
      );
    });
  };

  const addContext = (path: string) => {
    setContexts((current) => (current.includes(path) ? current : [...current, path]));
  };

  const rename = (next: string) => {
    if (!sessionId) return;
    const previous = title;
    setTitle(next);
    setRenaming(false);
    setPageError(null);
    void Promise.resolve(onRetitle(sessionId, next)).catch((failure) => {
      setTitle(previous);
      setPageError({
        message: `重命名失败：${errorMessage(failure)}`,
        retryLabel: "重试重命名",
        retry: () => rename(next),
      });
    });
  };

  const archive = (id: string) => {
    setPageError(null);
    void Promise.resolve(onArchive(id)).catch((failure) => {
      setPageError({
        message: `归档失败：${errorMessage(failure)}`,
        retryLabel: "重试归档",
        retry: () => archive(id),
      });
    });
  };

  useEffect(() => {
    if (!sessionId || demoMode) return;

    let alive = true;
    let stop: (() => void) | null = null;

    void onRunEvent((event) => {
      if (event.session !== sessionId) return;
      if (event.type === "approvalRequest") {
        setApproval({
          step: event.step,
          kind: event.kind,
          detail: event.detail ?? "",
          cwd: event.cwd,
          riskCategory: event.riskCategory,
          reason: event.reason,
        });
        setProgress({ phase: "等待审批", detail: "请确认后继续执行" });
        return;
      }
      if (event.type === "textDelta") {
        if (acceptStreamRef.current) {
          setStreamText((current) => current + event.text);
        }
        return;
      }
      if (event.type === "progress") {
        setProgress({ phase: event.phase, detail: event.detail });
        return;
      }
      if (event.type === "failover" || event.type === "providerSwitch") {
        const notice = {
          fromProvider: event.fromProvider,
          fromModel: event.fromModel,
          errorClass: event.errorClass,
          error: event.error,
          toProvider: event.toProvider,
          toModel: event.toModel,
        };
        setFailover(notice);
        setFailovers((current) => [
          ...current,
          `已从 ${event.fromProvider}（${event.fromModel}）切换到 ${event.toProvider}（${event.toModel}）· ${event.errorClass}`,
        ]);
        return;
      }
      if (event.type === "itemCompleted" && event.item.kind === "agentMessage") {
        setStreamText("");
      }
      if (event.type === "turnStarted") {
        acceptStreamRef.current = true;
        setStreamText("");
        setProgress(null);
        setFailovers([]);
        setFailover(null);
        setRunStartedAt((current) => current ?? Date.now());
      }
      setTurns((current) => reduce(current, event));
      if (event.type === "turnComplete" || event.type === "stopped" || event.type === "error") {
        acceptStreamRef.current = false;
        setRunning(false);
        setApproval(null);
        setStreamText("");
        setProgress(null);
        setRunStartedAt(null);
        if (event.type === "turnComplete") drainQueue();
      }
    }).then((off) => {
      if (alive) stop = off;
      else off();
    });

    return () => {
      alive = false;
      void Promise.resolve(stop?.()).catch(() => {
        // The Tauri event bridge can finish registering after the view has
        // already moved to the newly-created session. Cleanup is best effort.
      });
    };
  }, [sessionId, demoMode]);

  // Follow the live turn: stick to bottom only while the reader is already there.
  useEffect(() => {
    if (demoMode) return;
    const el = scrollRef.current;
    if (!el || !stickToBottomRef.current) return;
    el.scrollTop = el.scrollHeight;
  }, [demoMode, turns, streamText, progress, queue, running]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    stickToBottomRef.current = distance < 48;
  };

  // Load per-file unified diffs for Edit steps once a turn recorded changes.
  useEffect(() => {
    if (demoMode || !conversationId || !projectPath || running) return;
    const hasEdit = turns.some((turn) => turn.items.some((item) => item.kind === "fileChange"));
    if (!hasEdit) return;
    let alive = true;
    void turnChanges(projectPath, conversationId)
      .then((list) => {
        if (!alive || !list) return;
        const map: Record<string, string> = {};
        for (const change of list) {
          if (change.diff) map[change.path] = change.diff;
        }
        setFileDiffs(map);
      })
      .catch(() => {
        /* diffs are progressive disclosure; the path list still stands */
      });
    return () => {
      alive = false;
    };
  }, [demoMode, conversationId, projectPath, turns, running]);

  const liveSnapshot = useMemo(() => {
    if (demoMode || !sessionId) return null;
    const last = turns[turns.length - 1] ?? null;
    return snapshotFromTurn(sessionId, title || "新对话", projectName, projectPath, last, running);
  }, [demoMode, sessionId, turns, title, running, projectName, projectPath]);

  useEffect(() => {
    onSnapshot?.(demoMode || !sessionId ? null : liveSnapshot);
  }, [onSnapshot, liveSnapshot, demoMode, sessionId]);

  const heading = demoMode && demo
    ? demo.conversation.title
    : title || "新对话";
  const liveSession = !demoMode && sessionId !== null;
  const demoReply = demo ? demoReplyFrom(demo) : null;
  // Centered first-run stage: empty live conversation, no demo payload, no approval bar.
  const showWelcome = !demoMode && turns.length === 0 && !approval && !pageError && queue.length === 0 && Boolean(projectPath);

  const composer = (
    <Composer
      provider={provider}
      providers={providers}
      onSelectProvider={onSelectProvider}
      onSelectProviderModel={onSelectProviderModel}
      ready={!demoMode && (sessionId !== null || Boolean(projectPath))}
      running={running}
      projectPath={projectPath}
      contexts={contexts}
      queueCount={queue.length}
      onAddContext={addContext}
      onRemoveContext={(path) => setContexts((current) => current.filter((item) => item !== path))}
      onContextError={(message) =>
        setPageError({
          message,
          action: {
            label: "打开设置",
            run: () => openSettings(),
          },
        })
      }
      onSend={(text, context) => send(text, context)}
      onStop={stop}
      providerWarning={providerWarning}
      onOpenProviderSettings={openSettings}
    />
  );

  return (
    <main className={showWelcome ? "main main--welcome" : "main"}>
      <div className="scroll" ref={scrollRef} onScroll={onScroll} data-testid="conversation-scroll">
        <div className="page-inner">
          <div className="conv-head">
            <Folder size={16} strokeWidth={1.7} />
            {renaming && liveSession ? (
              <input
                className="conv-title-input"
                value={draftTitle}
                autoFocus
                onChange={(event) => setDraftTitle(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && draftTitle.trim() && sessionId) {
                    rename(draftTitle.trim());
                  }
                  if (event.key === "Escape") setRenaming(false);
                }}
                onBlur={() => setRenaming(false)}
              />
            ) : (
              <h1 className="conv-title">{heading}</h1>
            )}
            {liveSession && (
              <button
                type="button"
                className="icon-btn icon-btn--sm"
                aria-label="Rename conversation"
                onClick={() => {
                  setDraftTitle(heading);
                  setRenaming(true);
                }}
              >
                <Pencil size={13} strokeWidth={1.8} />
              </button>
            )}
            <span className="spacer" />
            {liveSession && (
              <Menu
                align="right"
                trigger={({ toggle }) => (
                  <button type="button" className="icon-btn" aria-label="Conversation actions" onClick={toggle}>
                    <MoreVertical size={16} strokeWidth={1.8} />
                  </button>
                )}
              >
                {(close) => (
                  <>
                    <MenuItem
                      icon={<Pencil size={14} strokeWidth={1.8} />}
                      label="Rename conversation"
                      onSelect={() => {
                        close();
                        setDraftTitle(heading);
                        setRenaming(true);
                      }}
                    />
                    <MenuItem
                      icon={<Archive size={14} strokeWidth={1.8} />}
                      label="Archive conversation"
                      danger
                      onSelect={() => {
                        close();
                        if (sessionId) archive(sessionId);
                      }}
                    />
                    <MenuItem
                      icon={<Folder size={14} strokeWidth={1.8} />}
                      label="Open response trace"
                      onSelect={() => {
                        close();
                        onOpenTrace();
                      }}
                    />
                  </>
                )}
              </Menu>
            )}
          </div>

          {pageError && (
            <div className="action-error" role="alert" data-testid="action-error">
              <span className="action-error-msg">{pageError.message}</span>
              <div className="action-error-actions">
                {pageError.retry && (
                  <button
                    type="button"
                    className="btn btn--primary btn--sm"
                    data-testid="action-error-retry"
                    onClick={pageError.retry}
                  >
                    {pageError.retryLabel ?? "重试"}
                  </button>
                )}
                <button
                  type="button"
                  className="btn btn--sm"
                  data-testid="action-error-dismiss"
                  onClick={() => setPageError(null)}
                >
                  关闭
                </button>
              </div>
            </div>
          )}

          {failover && (
            <div className="action-error" role="status" data-testid="failover-notice">
              <span className="action-error-msg">
                Failover · {failover.fromProvider}/{failover.fromModel} → {failover.toProvider}/
                {failover.toModel} · {failover.errorClass}
                {failover.error ? ` — ${failover.error.slice(0, 160)}` : ""}
              </span>
              <div className="action-error-actions">
                <button
                  type="button"
                  className="btn btn--sm"
                  data-testid="failover-dismiss"
                  onClick={() => setFailover(null)}
                >
                  关闭
                </button>
              </div>
            </div>
          )}

          {showWelcome ? (
            <div className="welcome-stage" data-testid="welcome">
              <div className="welcome-copy">
                <h2 className="welcome-title">今天想做什么？</h2>
                <p className="welcome-sub empty-note">
                  描述任务或粘贴报错，我会在当前项目里读代码、改代码。
                </p>
                <div className="welcome-hints" data-testid="welcome-hints">
                  <span className="welcome-hint">/ 调用技能</span>
                  <span className="welcome-hint">+ 附加文件</span>
                  <span className="welcome-hint">运行中可排队下一条</span>
                </div>
              </div>
              {composer}
            </div>
          ) : (
            <>
              {demoMode && demo && demoReply ? (
                <>
                  <UserMessage time={demo.conversation.user.time} text={demo.conversation.user.content} />
                  <AssistantReply
                    time={demo.conversation.assistant.time}
                    reply={demoReply}
                    running={false}
                    failovers={[]}
                    onViewFiles={onViewFiles}
                    fileDiffs={fileDiffs}
                  />
                </>
              ) : turns.length === 0 && queue.length === 0 ? (
                <p className="empty-note">发一条消息开始这次对话。</p>
              ) : (
                turns.map((turn, index) => {
                  const last = index === turns.length - 1;
                  return (
                    <Fragment key={index}>
                      <UserMessage
                        text={turn.ask}
                        context={turn.context}
                        time={formatTurnTime(turn)}
                      />
                      <AssistantReply
                        time=""
                        reply={toReply(turn, running && last)}
                        running={running && last}
                        streamText={running && last ? streamText : ""}
                        progress={running && last ? progress : null}
                        elapsed={running && last ? elapsed : null}
                        waitingApproval={running && last ? Boolean(approval) : false}
                        failovers={last ? failovers : []}
                        onViewFiles={onViewFiles}
                        onRegenerate={liveSession && last && !running ? regenerate : null}
                        fileDiffs={fileDiffs}
                      />
                    </Fragment>
                  );
                })
              )}
            </>
          )}

          {queue.length > 0 && (
            <div className="send-queue" data-testid="send-queue">
              <div className="send-queue-head">
                <strong>待发送 · {queue.length}</strong>
                <button type="button" className="btn btn--sm" data-testid="queue-clear" onClick={() => applyQueue([])}>
                  清空
                </button>
              </div>
              <ul className="send-queue-list">
                {queue.map((item, index) => (
                  <li key={`${item.text}-${index}`}>
                    <span className="send-queue-text">{item.text}</span>
                    <button
                      type="button"
                      className="icon-btn icon-btn--sm"
                      aria-label={`移出队列 ${index + 1}`}
                      onClick={() =>
                        applyQueue(queueRef.current.filter((_, position) => position !== index))
                      }
                    >
                      ×
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}

          {/* Approval sits outside the turn list so it also shows on an empty turn. */}
          {liveSession && approval && (
            <div className="approval-bar" role="alertdialog" aria-label="审批工具步骤" data-testid="approval-bar">
              <div className="approval-text">
                <strong>
                  需要批准 · {approval.kind === "FileChange" ? "修改文件" : approval.kind}
                  {approval.riskCategory ? ` · ${approval.riskCategory}` : ""}
                </strong>
                <code data-testid="approval-command">{approval.detail || "继续执行该步骤"}</code>
                {approval.reason && (
                  <span className="approval-reason" data-testid="approval-reason">
                    原因：{approval.reason}
                  </span>
                )}
                {approval.cwd && (
                  <span className="approval-cwd" data-testid="approval-cwd">
                    cwd: {approval.cwd}
                  </span>
                )}
                {approval.riskCategory && (
                  <span className="approval-risk" data-testid="approval-risk">
                    风险：{approval.riskCategory}
                  </span>
                )}
              </div>
              <div className="approval-actions">
                <button type="button" className="btn" data-testid="approval-deny" onClick={() => decide(false)}>
                  拒绝
                </button>
                <button
                  type="button"
                  className="btn"
                  data-testid="approval-allow-session"
                  title="本会话内相同命令不再询问"
                  onClick={() => decide(true, true)}
                >
                  本会话允许
                </button>
                <button type="button" className="btn btn--primary" data-testid="approval-allow" onClick={() => decide(true)}>
                  允许
                </button>
              </div>
            </div>
          )}
          {approvalError && (
            <div className="action-error" role="alert" data-testid="approval-error">
              <span className="action-error-msg">{approvalError}</span>
              <div className="action-error-actions">
                <button type="button" className="btn btn--sm" onClick={() => setApprovalError(null)}>
                  关闭
                </button>
              </div>
            </div>
          )}
        </div>
      </div>

      {!showWelcome && composer}
    </main>
  );
}

function errorMessage(failure: unknown): string {
  if (failure instanceof Error) return failure.message;
  return String(failure);
}

function navigateSettings() {
  navigate(window.location.pathname.startsWith("/ui-demo") ? "/ui-demo/settings" : "/settings");
}

function formatTurnTime(turn: TurnDto): string | undefined {
  const at = turn.items[0]?.at;
  if (!at) return undefined;
  // core `at` is unix seconds
  const date = new Date(at < 1e12 ? at * 1000 : at);
  if (Number.isNaN(date.getTime())) return undefined;
  return date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

function UserMessage({ time, text, context }: { time?: string; text: string; context?: string[] }) {
  return (
    <div className="msg-user">
      <div className="msg-head">
        <span className="avatar avatar--user">LI</span>
        <span className="msg-author">You</span>
        {time && <span className="msg-time">{time}</span>}
      </div>
      {context && context.length > 0 && (
        <div className="msg-contexts" data-testid="msg-contexts">
          {context.map((path) => (
            <span key={path} className="chip chip--context chip--static">
              <span className="chip-label">{path}</span>
            </span>
          ))}
        </div>
      )}
      <p className="msg-bubble">{text}</p>
    </div>
  );
}

function blank(ask: string, context: string[] = []): TurnDto {
  return { ask, context, items: [], done: false, stopped: false, interrupted: false, error: null };
}

function reduce(turns: TurnDto[], event: RunEventDto): TurnDto[] {
  switch (event.type) {
    case "itemStarted":
    case "itemCompleted":
      return updateLast(turns, (turn) => ({ ...turn, items: upsert(turn.items, event.item) }));
    case "turnComplete":
      return updateLast(turns, (turn) => ({ ...turn, done: true }));
    case "stopped":
      return updateLast(turns, (turn) => ({ ...turn, stopped: true }));
    case "error":
      return updateLast(turns, (turn) => ({ ...turn, error: event.message }));
    default:
      return turns;
  }
}

function updateLast(turns: TurnDto[], change: (turn: TurnDto) => TurnDto): TurnDto[] {
  if (turns.length === 0) return turns;
  const next = [...turns];
  next[next.length - 1] = change(next[next.length - 1]);
  return next;
}

function upsert(items: ItemDto[], item: ItemDto): ItemDto[] {
  const index = items.findIndex((existing) => existing.id === item.id);
  const next = index === -1 ? [...items, item] : items.map((existing, position) => (position === index ? item : existing));
  return next.sort((left, right) => left.id - right.id);
}
