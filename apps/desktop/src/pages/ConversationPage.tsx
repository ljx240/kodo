import { Archive, Folder, MoreVertical, Pencil } from "lucide-react";
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import {
  loadSession,
  onRunEvent,
  respondApproval,
  sendMessage,
  stopRun,
  type ItemDto,
  type RunEventDto,
  type TurnDto,
} from "../api";
import { AssistantReply } from "../conversation/AssistantReply";
import { Composer } from "../conversation/Composer";
import { toReply, type Reply } from "../conversation/trace";
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
  onViewFiles: () => void;
  onRetitle: (id: string, title: string) => Promise<void> | void;
  onArchive: (id: string) => Promise<void> | void;
  onOpenTrace: () => void;
  onSnapshot?: (snapshot: LiveSnapshot | null) => void;
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

function demoReplyFrom(demo: DemoState): Reply {
  return {
    steps: demo.conversation.assistant.trace,
    status: "completed",
    error: null,
    final: demo.conversation.assistant.final,
    checks: demo.conversation.assistant.checks,
    changes: demo.changedFiles,
    files: demo.summary.files_changed,
    added: demo.summary.added,
    removed: demo.summary.removed,
  };
}

export function ConversationPage({
  conversationId,
  provider,
  providers,
  onSelectProvider,
  onViewFiles,
  onRetitle,
  onArchive,
  onOpenTrace,
  onSnapshot,
  projectName = "",
  projectPath = "",
  demo = null,
}: Props) {
  const demoMode = Boolean(demo);
  const [turns, setTurns] = useState<TurnDto[]>([]);
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
  /** False after Stop — late TextDelta races must not repaint the preview. */
  const acceptStreamRef = useRef(true);

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
    acceptStreamRef.current = true;
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
      // Reopened sessions start "not running". The reply lifecycle derives
      // interrupted/stopped state from the persisted turn envelope.
      setRunning(false);
    });
    return () => {
      alive = false;
    };
  }, [conversationId, demoMode, onSnapshot]);

  useEffect(() => {
    if (!conversationId || demoMode) return;

    let alive = true;
    let stop: (() => void) | null = null;

    void onRunEvent((event) => {
      if (event.session !== conversationId) return;
      if (event.type === "approvalRequest") {
        setApproval({
          step: event.step,
          kind: event.kind,
          detail: event.detail ?? "",
          cwd: event.cwd,
          riskCategory: event.riskCategory,
          reason: event.reason,
        });
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
      if (event.type === "itemCompleted" && event.item.kind === "agentMessage") {
        setStreamText("");
      }
      if (event.type === "turnStarted") {
        acceptStreamRef.current = true;
        setStreamText("");
        setProgress(null);
      }
      setTurns((current) => reduce(current, event));
      if (event.type === "turnComplete" || event.type === "stopped" || event.type === "error") {
        acceptStreamRef.current = false;
        setRunning(false);
        setApproval(null);
        setStreamText("");
        setProgress(null);
      }
    }).then((off) => {
      if (alive) stop = off;
      else off();
    });

    return () => {
      alive = false;
      stop?.();
    };
  }, [conversationId, demoMode]);

  const liveSnapshot = useMemo(() => {
    if (demoMode || !conversationId) return null;
    const last = turns[turns.length - 1] ?? null;
    return snapshotFromTurn(conversationId, title || "新对话", projectName, projectPath, last, running);
  }, [demoMode, conversationId, turns, title, running, projectName, projectPath]);

  useEffect(() => {
    onSnapshot?.(demoMode || !conversationId ? null : liveSnapshot);
  }, [onSnapshot, liveSnapshot, demoMode, conversationId]);

  const send = async (text: string, context: string[]) => {
    if (!conversationId) return;
    const payload = { text, context };
    setPageError(null);
    setTurns((current) => [...current, blank(text, context)]);
    setRunning(true);
    acceptStreamRef.current = true;
    setStreamText("");
    try {
      await sendMessage(conversationId, text, context);
      // Context is consumed for this turn only; the user re-pins if needed.
      setContexts([]);
    } catch (failure) {
      setRunning(false);
      const message = errorMessage(failure);
      setTurns((current) => updateLast(current, (turn) => ({ ...turn, error: message })));
      setPageError({
        message: `发送失败：${message}`,
        retryLabel: "重试发送",
        retry: () => {
          setPageError(null);
          void send(payload.text, payload.context);
        },
      });
    }
  };

  const stop = () => {
    if (!conversationId) return;
    setApproval(null);
    // Stop 后不再产生用户可见 TextDelta。
    acceptStreamRef.current = false;
    setStreamText("");
    setProgress(null);
    void stopRun(conversationId).catch((failure) => {
      setApprovalError(`停止失败：${errorMessage(failure)}`);
    });
  };

  const decide = (approved: boolean) => {
    if (!conversationId || !approval) return;
    const pending = approval;
    setApproval(null);
    setApprovalError(null);
    void respondApproval(conversationId, pending.step, approved).catch((failure) => {
      setApprovalError(
        `审批响应失败：${errorMessage(failure)}（${approved ? "允许" : "拒绝"} step ${pending.step}）`,
      );
    });
  };

  const addContext = (path: string) => {
    setContexts((current) => (current.includes(path) ? current : [...current, path]));
  };

  const rename = (next: string) => {
    if (!conversationId) return;
    const previous = title;
    setTitle(next);
    setRenaming(false);
    setPageError(null);
    void Promise.resolve(onRetitle(conversationId, next)).catch((failure) => {
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

  const heading = demoMode && demo
    ? demo.conversation.title
    : title || "新对话";
  const liveSession = !demoMode && conversationId !== null;
  const demoReply = demo ? demoReplyFrom(demo) : null;
  // Centered first-run stage: empty live conversation, no demo payload, no approval bar.
  const showWelcome = liveSession && turns.length === 0 && !approval && !pageError;

  const composer = (
    <Composer
      provider={provider}
      providers={providers}
      onSelectProvider={onSelectProvider}
      ready={!demoMode && conversationId !== null}
      running={running}
      projectPath={projectPath}
      contexts={contexts}
      onAddContext={addContext}
      onRemoveContext={(path) => setContexts((current) => current.filter((item) => item !== path))}
      onContextError={(message) =>
        setPageError({
          message,
          action: {
            label: "打开设置",
            run: () => navigateSettings(),
          },
        })
      }
      onSend={(text, context) => void send(text, context)}
      onStop={stop}
      providerWarning={providerWarning}
      onOpenProviderSettings={() => navigateSettings()}
    />
  );

  return (
    <main className={showWelcome ? "main main--welcome" : "main"}>
      <div className="scroll">
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
                  if (event.key === "Enter" && draftTitle.trim() && conversationId) {
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
                        if (conversationId) archive(conversationId);
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

          {showWelcome ? (
            <div className="welcome-stage" data-testid="welcome">
              <div className="welcome-copy">
                <h2 className="welcome-title">今天想做什么？</h2>
                <p className="welcome-sub empty-note">
                  描述任务或粘贴报错，我会在当前项目里读代码、改代码。
                </p>
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
                    onViewFiles={onViewFiles}
                  />
                </>
              ) : turns.length === 0 ? (
                <p className="empty-note">发一条消息开始这次对话。</p>
              ) : (
                turns.map((turn, index) => {
                  const last = index === turns.length - 1;
                  return (
                    <Fragment key={index}>
                      <UserMessage text={turn.ask} context={turn.context} />
                      <AssistantReply
                        time=""
                        reply={toReply(turn, running && last)}
                        streamText={running && last ? streamText : ""}
                        progress={running && last ? progress : null}
                        onViewFiles={onViewFiles}
                      />
                    </Fragment>
                  );
                })
              )}
            </>
          )}

          {/* Approval sits outside the turn list so it also shows on an empty turn. */}
          {liveSession && approval && (
            <div className="approval-bar" role="alertdialog" aria-label="审批工具步骤" data-testid="approval-bar">
              <div className="approval-text">
                <strong>
                  需要批准 · {approval.kind}
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
                    risk: {approval.riskCategory}
                  </span>
                )}
              </div>
              <div className="approval-actions">
                <button type="button" className="btn" onClick={() => decide(false)}>
                  拒绝
                </button>
                <button type="button" className="btn btn--primary" onClick={() => decide(true)}>
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
  return { ask, context, items: [], done: false, stopped: false, error: null };
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
  if (index === -1) return [...items, item];
  const next = [...items];
  next[index] = item;
  return next;
}
