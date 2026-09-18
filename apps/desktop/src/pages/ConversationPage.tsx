import { Archive, Folder, MoreVertical, Pencil } from "lucide-react";
import { Fragment, useEffect, useMemo, useState } from "react";
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
import { changedFiles, conversation, projects, summary } from "../data/fixture";
import { snapshotFromTurn, type LiveSnapshot } from "../data/liveContext";
import { type ProviderConfig } from "../data/providers";
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
};

type Approval = { step: number; kind: string; detail: string };

const demoReply: Reply = {
  steps: conversation.assistant.trace,
  final: conversation.assistant.final,
  checks: conversation.assistant.checks,
  changes: changedFiles,
  files: summary.files_changed,
  added: summary.added,
  removed: summary.removed,
  interrupted: false,
};

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
}: Props) {
  const isFixture = conversationId === conversation.id;
  const [turns, setTurns] = useState<TurnDto[]>([]);
  const [title, setTitle] = useState("");
  const [running, setRunning] = useState(false);
  const [approval, setApproval] = useState<Approval | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [draftTitle, setDraftTitle] = useState("");

  useEffect(() => {
    setApproval(null);
    if (!conversationId || isFixture) {
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
  }, [conversationId, isFixture]);

  useEffect(() => {
    if (!conversationId || isFixture) return;

    let alive = true;
    let stop: (() => void) | null = null;

    void onRunEvent((event) => {
      if (event.session !== conversationId) return;
      if (event.type === "approvalRequest") {
        setApproval({ step: event.step, kind: event.kind, detail: event.detail ?? "" });
        return;
      }
      setTurns((current) => reduce(current, event));
      if (event.type === "turnComplete" || event.type === "stopped" || event.type === "error") {
        setRunning(false);
        setApproval(null);
      }
    }).then((off) => {
      if (alive) stop = off;
      else off();
    });

    return () => {
      alive = false;
      stop?.();
    };
  }, [conversationId, isFixture]);

  const liveSnapshot = useMemo(() => {
    if (isFixture || !conversationId) return null;
    const last = turns[turns.length - 1] ?? null;
    return snapshotFromTurn(conversationId, title || "新对话", projectName, projectPath, last, running);
  }, [isFixture, conversationId, turns, title, running, projectName, projectPath]);

  useEffect(() => {
    onSnapshot?.(isFixture || !conversationId ? null : liveSnapshot);
  }, [onSnapshot, liveSnapshot, isFixture, conversationId]);

  const send = async (text: string) => {
    if (!conversationId) return;
    setTurns((current) => [...current, blank(text)]);
    setRunning(true);
    try {
      await sendMessage(conversationId, text);
    } catch (failure) {
      setRunning(false);
      setTurns((current) => updateLast(current, (turn) => ({ ...turn, error: String(failure) })));
    }
  };

  const stop = () => {
    if (!conversationId) return;
    setApproval(null);
    void stopRun(conversationId);
  };

  const decide = (approved: boolean) => {
    if (!conversationId || !approval) return;
    void respondApproval(conversationId, approval.step, approved);
    setApproval(null);
  };

  const heading = isFixture ? titleOf(conversationId) : title || "新对话";
  const liveSession = !isFixture && conversationId !== null;

  return (
    <main className="main">
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
                    const next = draftTitle.trim();
                    setTitle(next);
                    setRenaming(false);
                    void Promise.resolve(onRetitle(conversationId, next)).catch(() => {
                      /* workspace.retitle reports via tree refresh failure */
                    });
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
                        if (conversationId) void onArchive(conversationId);
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

          {isFixture ? (
            <>
              <UserMessage time={conversation.user.time} text={conversation.user.content} />
              <AssistantReply
                time={conversation.assistant.time}
                reply={demoReply}
                running={false}
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
                  <UserMessage text={turn.ask} />
                  <AssistantReply
                    time=""
                    reply={toReply(turn, running && last)}
                    running={running && last}
                    onViewFiles={onViewFiles}
                  />
                </Fragment>
              );
            })
          )}

          {/* Approval sits outside the turn list so it also shows on an empty turn. */}
          {liveSession && approval && (
            <div className="approval-bar" role="alertdialog" aria-label="审批工具步骤">
              <div className="approval-text">
                <strong>需要批准 · {approval.kind}</strong>
                <code>{approval.detail || "继续执行该步骤"}</code>
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
        </div>
      </div>

      <Composer
        provider={provider}
        providers={providers}
        onSelectProvider={onSelectProvider}
        ready={!isFixture && conversationId !== null}
        running={running}
        onSend={(text) => void send(text)}
        onStop={stop}
      />
    </main>
  );
}

function UserMessage({ time, text }: { time?: string; text: string }) {
  return (
    <div className="msg-user">
      <div className="msg-head">
        <span className="avatar avatar--user">LI</span>
        <span className="msg-author">You</span>
        {time && <span className="msg-time">{time}</span>}
      </div>
      <p className="msg-bubble">{text}</p>
    </div>
  );
}

function blank(ask: string): TurnDto {
  return { ask, items: [], done: false, stopped: false, error: null };
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

function titleOf(conversationId: string | null): string {
  if (!conversationId) return "新对话";
  for (const project of projects) {
    const match = project.conversations.find((item) => item.id === conversationId);
    if (match) return match.title;
  }
  return conversation.title;
}
