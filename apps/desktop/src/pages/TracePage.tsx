import {
  ArrowLeft,
  BarChart3,
  Check,
  CheckCircle2,
  Circle,
  Folder,
  Pencil,
  Play,
  Search,
  Sparkles,
} from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import {
  listArtifacts,
  loadFileSpan,
  loadLlmIo,
  loadSession,
  type ArtifactRefDto,
  type FileSpanDto,
  type LlmIoDto,
  type SessionDto,
} from "../api";
import { LlmIoViewer } from "../conversation/LlmIoViewer";
import { ThinkingView } from "../conversation/ThinkingView";
import { FileSpanModal } from "../conversation/FileSpanModal";
import { formatDuration, formatTokens, mergeChanges, totals } from "../conversation/trace";
import { type DemoState, type DemoState as Demo } from "../data/demoState";
import type { ChangedFile } from "../data/types";
import { ReplyMeta } from "../inspector/TraceInspector";
import { FileRow } from "../inspector/Section";
import { T, toolAlias } from "../i18n";
import { navigate } from "../routes";

const TABS = [
  { id: "timeline", label: T.page.traceTabs.timeline },
  { id: "logs", label: T.page.traceTabs.logs },
  { id: "artifacts", label: T.page.traceTabs.artifacts },
  { id: "llmIo", label: T.page.traceTabs.llmIo },
  { id: "thinking", label: T.page.traceTabs.thinking },
  { id: "spans", label: T.page.traceTabs.spans },
] as const;
type Tab = (typeof TABS)[number]["id"];

type TimelineRow = Demo["timeline"][number];

const TYPE_ICONS: Record<TimelineRow["type"], typeof Circle> = {
  Thinking: Circle,
  Search: Search,
  Read: BarChart3,
  Run: Play,
  Output: CheckCircle2,
  Model: Sparkles,
  Edit: Pencil,
  Finalize: CheckCircle2,
};

function TypePill({ type }: { type: TimelineRow["type"] }) {
  const Icon = TYPE_ICONS[type];
  return (
    <span className={`type-pill type-pill--${type.toLowerCase()}`}>
      <Icon size={13} strokeWidth={1.9} />
      <span>{toolAlias(type)}</span>
    </span>
  );
}

function liveTimeline(session: SessionDto): {
  rows: TimelineRow[];
  final: string;
  checks: string[];
  files: ChangedFile[];
  done: boolean;
  stopped: boolean;
  interrupted: boolean;
} {
  const turn = session.turns[session.turns.length - 1];
  if (!turn) {
    return { rows: [], final: "", checks: [], files: [], done: false, stopped: false, interrupted: false };
  }
  let final = "";
  let checks: string[] = [];
  const files: ChangedFile[] = [];
  const rows: TimelineRow[] = [];

  turn.items.forEach((item, index) => {
    if (item.kind === "agentMessage") {
      final = item.text;
      checks = item.checks;
      rows.push({
        n: index + 1,
        time: new Date(item.at * 1000).toLocaleTimeString(),
        duration: formatDuration(item.duration),
        type: "Finalize",
        title: T.step.finalize,
        note: item.text.slice(0, 80),
      });
      return;
    }
    const base = {
      n: index + 1,
      time: new Date(item.at * 1000).toLocaleTimeString(),
      duration: formatDuration(item.duration),
    };
    switch (item.kind) {
      case "reasoning":
        rows.push({ ...base, type: "Thinking", title: T.step.thinking, note: item.summary });
        break;
      case "search":
        rows.push({ ...base, type: "Search", title: T.step.search, note: item.detail, chip: item.query });
        break;
      case "fileRead":
        rows.push({ ...base, type: "Read", title: T.step.read, note: item.detail, chip: item.path });
        break;
      case "commandExecution":
        rows.push({
          ...base,
          type: "Run",
          title: T.step.run,
          note: item.output.slice(0, 80) || "命令输出",
          chip: item.command,
          ok: item.exitCode === 0,
        });
        break;
      case "modelCall":
        rows.push({
          ...base,
          type: "Model",
          title: T.step.model,
          note: T.step.model,
          chips: [item.model, `${formatTokens(item.inputTokens)} → ${formatTokens(item.outputTokens)}`],
        });
        break;
      case "fileChange": {
        const merged = mergeChanges(item.changes);
        const stepTotals = totals(merged);
        files.push(...merged);
        rows.push({
          ...base,
          type: "Edit",
          title: T.step.edit,
          note: `${stepTotals.files} 个文件`,
          delta: {
            added: stepTotals.added,
            removed: stepTotals.removed,
          },
        });
        break;
      }
    }
  });

  return {
    rows,
    final,
    checks,
    files: mergeChanges(files),
    done: turn.done,
    stopped: turn.stopped,
    interrupted:
      !turn.done &&
      !turn.stopped &&
      !turn.error &&
      (Boolean(turn.interrupted) || turn.items.some((item) => item.status === "running")),
  };
}

/**
 * The page's only chrome: a back control into the conversation, plus the
 * response path. Returning is one click (or one conversation click in the
 * sidebar) — never a dead end.
 */
function TraceTopBar({ demoMode, children }: { demoMode: boolean; children?: ReactNode }) {
  return (
    <header className="topbar topbar--plain">
      <div className="topbar-left">
        <button
          type="button"
          className="icon-btn icon-btn--sm"
          aria-label={T.action.backToConversation}
          title={T.action.backToConversation}
          data-testid="trace-back"
          onClick={() => navigate(demoMode ? "/ui-demo/conversation" : "/conversation")}
        >
          <ArrowLeft size={15} strokeWidth={1.8} />
        </button>
        {children}
      </div>
      <span className="spacer" />
    </header>
  );
}

export function TracePage({
  demo,
  conversationId,
  live,
}: {
  /** Fixture bundle for `/ui-demo` only; live routes pass null. */
  demo: DemoState | null;
  conversationId: string | null;
  live?: SessionDto | null;
}) {
  const demoMode = Boolean(demo);
  const [tab, setTab] = useState<Tab>("timeline");
  const [session, setSession] = useState<SessionDto | null>(live ?? null);
  const [openSpanRef, setOpenSpanRef] = useState<ArtifactRefDto | null>(null);
  const [openSpan, setOpenSpan] = useState<FileSpanDto | null>(null);
  const [openSpanLoading, setOpenSpanLoading] = useState<boolean>(false);

  useEffect(() => {
    if (demoMode) {
      setSession(null);
      return;
    }
    if (live !== undefined) {
      setSession(live);
      return;
    }
    if (!conversationId) {
      setSession(null);
      return;
    }
    let alive = true;
    void loadSession(conversationId).then((loaded) => {
      if (alive) setSession(loaded);
    });
    return () => {
      alive = false;
    };
  }, [demoMode, conversationId, live]);

  useEffect(() => {
    if (!openSpanRef || demoMode || !session) {
      setOpenSpan(null);
      return;
    }
    const turnSeq = session.turns.length - 1;
    setOpenSpanLoading(true);
    let alive = true;
    void loadFileSpan(session.id, turnSeq, openSpanRef.label).then((data) => {
      if (!alive) return;
      setOpenSpan(data);
      setOpenSpanLoading(false);
    });
    return () => {
      alive = false;
    };
  }, [openSpanRef, demoMode, session]);

  if (!demoMode && !conversationId) {
    return (
      <main className="main">
        <TraceTopBar demoMode={demoMode} />
        <div className="scroll">
          <p className="empty-note">{T.page.selectConversation}</p>
        </div>
      </main>
    );
  }

  if (!demoMode && !session) {
    return (
      <main className="main">
        <TraceTopBar demoMode={demoMode} />
        <div className="scroll">
          <p className="empty-note">{T.page.loadingTrace}</p>
        </div>
      </main>
    );
  }

  const data = demoMode && demo
    ? {
        rows: demo.timeline,
        // The demo Response Trace records turn 1's run (pairs with demo.timeline).
        final: demo.conversation.turns[0].assistant.final,
        checks: [] as string[],
        files: demo.changedFiles,
        done: true,
        stopped: false,
        interrupted: false,
        crumbProject: demo.project.name,
        crumbTitle: demo.conversation.title,
      }
    : (() => {
        const mapped = liveTimeline(session!);
        return {
          ...mapped,
          crumbProject: session!.project.split("/").pop() || session!.project,
          crumbTitle: session!.title,
        };
      })();

  if (!demoMode && data.rows.length === 0) {
    return (
      <main className="main">
        <TraceTopBar demoMode={demoMode} />
        <div className="scroll">
          <p className="empty-note">{T.page.emptyTrace}</p>
        </div>
      </main>
    );
  }

  const statusLabel = demoMode
    ? T.lifecycle.completed
    : data.stopped
      ? T.lifecycle.stopped
      : data.done
        ? T.lifecycle.completed
        : T.lifecycle.interrupted;
  const logs = data.rows.map((row) => {
    const detail = row.chip ?? row.chips?.join(" ") ?? "";
    // Fixture rows name tools in English; logs show the same zh labels as the UI.
    const title = toolAlias(row.title);
    return `${row.time}  ${String(row.duration).padStart(4)}  ${row.type.padEnd(9)} ${title}${detail ? ` ${detail}` : ""}`;
  });

  return (
    <main className="main">
      <TraceTopBar demoMode={demoMode}>
        <nav className="crumbs">
          <Folder size={14} strokeWidth={1.7} />
          <span>{data.crumbProject}</span>
          <span className="crumb-sep">/</span>
          <span>{data.crumbTitle}</span>
          <span className="crumb-sep">/</span>
          <span className="crumb-current">{T.page.responseTrace}</span>
        </nav>
      </TraceTopBar>

      <div className="scroll">
        <div className="page-inner page-inner--wide">
          <section className="reply-card">
            <div className="reply-card-head">
              <span className="reply-mark">
                <Sparkles size={14} strokeWidth={1.9} />
              </span>
              <h1 className="reply-card-title">{T.page.assistantReply}</h1>
              <span className="status-pill">
                <span className="status-mark">
                  <Check size={9} strokeWidth={4} />
                </span>
                <span>{statusLabel}</span>
              </span>
            </div>

            <div className="reply-card-body">
              <ReplyMeta
                demo={demoMode}
                demoState={demo}
                live={
                  demoMode
                    ? null
                    : {
                        conversationId: session!.id,
                        title: session!.title,
                        projectName: data.crumbProject,
                        projectPath: session!.project,
                        turn: session!.turns[session!.turns.length - 1] ?? null,
                        reply: null,
                        running: false,
                      }
                }
              />
              <div className="reply-summary">
                <h4>{T.page.finalSummary}</h4>
                <p>{data.final || T.page.noFinalReply}</p>
                {data.checks.length > 0 && (
                  <ul className="trace-checks">
                    {data.checks.map((check) => (
                      <li key={check}>{check}</li>
                    ))}
                  </ul>
                )}
              </div>
            </div>
          </section>

          <nav className="page-tabs" role="tablist" aria-label={T.page.traceViews}>
            {TABS.map(({ id, label }) => (
              <button
                key={id}
                type="button"
                role="tab"
                className={`page-tab${id === tab ? " page-tab--active" : ""}`}
                aria-selected={id === tab}
                onClick={() => setTab(id)}
              >
                {label}
              </button>
            ))}
          </nav>

          {tab === "logs" && <pre className="terminal-block terminal-block--tall">{logs.join("\n") || T.page.noLogs}</pre>}

          {tab === "artifacts" && (
            <div className="artifact-list">
              {data.files.length === 0 ? (
                <p className="empty-note">{T.page.noFileChanges}</p>
              ) : (
                data.files.map((file) => <FileRow key={file.path} file={file} />)
              )}
            </div>
          )}

          {tab === "llmIo" && (
            <LlmIoTab
              demoMode={demoMode}
              sessionId={demoMode ? null : session!.id}
              turnSeq={demoMode ? null : session!.turns.length - 1}
            />
          )}

          {tab === "thinking" && (
            <ThinkingView
              demoMode={demoMode}
              rows={data.rows}
            />
          )}

          {tab === "spans" && (
            <SpansTab
              demoMode={demoMode}
              sessionId={demoMode ? null : session!.id}
              turnSeq={demoMode ? null : session!.turns.length - 1}
              onOpenSpan={(ref) => {
                if (!demoMode) setOpenSpanRef(ref);
              }}
            />
          )}

          {tab === "timeline" && (
            <div className="table-scroll" role="region" aria-label="执行时间线" tabIndex={0}>
              <table className="timeline">
                <thead>
                  <tr>
                    <th className="col-n">{T.page.timelineColumns.n}</th>
                    <th className="col-time">{T.page.timelineColumns.time}</th>
                    <th className="col-duration">{T.page.timelineColumns.duration}</th>
                    <th className="col-type">{T.page.timelineColumns.type}</th>
                    <th>{T.page.timelineColumns.details}</th>
                  </tr>
                </thead>
                <tbody>
                  {data.rows.map((row) => (
                    <tr key={`${row.n}-${row.title}`}>
                      <td className="col-n">{row.n}</td>
                      <td className="col-time mono">{row.time}</td>
                      <td className="col-duration mono">{row.duration}</td>
                      <td className="col-type">
                        <TypePill type={row.type} />
                      </td>
                      <td>
                        <div className="tl-title">
                          <span>{toolAlias(row.title)}</span>
                          {row.chip && <code className="code-chip">{row.chip}</code>}
                          {row.chips?.map((chip) => (
                            <code key={chip} className="code-chip">
                              {chip}
                            </code>
                          ))}
                          {row.delta && (
                            <>
                              <span className="delta-add">+{row.delta.added}</span>
                              <span className="delta-del">-{row.delta.removed}</span>
                            </>
                          )}
                          {row.ok && (
                            <span className="tl-ok">
                              <Check size={12} strokeWidth={3} />
                            </span>
                          )}
                        </div>
                        <div className="tl-note">{row.note}</div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </div>

      {openSpanRef && (
        <FileSpanModal
          span={openSpan}
          loading={openSpanLoading}
          onClose={() => {
            setOpenSpanRef(null);
            setOpenSpan(null);
          }}
        />
      )}
    </main>
  );
}

// ---------------------------------------------------------------------------
// Tab bodies (Phase E — transparency surface)
//
// Each tab is a small self-contained component so the page stays focused on
// layout. They all share the same lifecycle: while the user is on the tab
// they lazy-load the matching artifact list once.
// ---------------------------------------------------------------------------

function LlmIoTab({
  demoMode,
  sessionId,
  turnSeq,
}: {
  demoMode: boolean;
  sessionId: string | null;
  turnSeq: number | null;
}) {
  const [refs, setRefs] = useState<ArtifactRefDto[] | null>(null);
  const [active, setActive] = useState<number>(0);
  const [capture, setCapture] = useState<LlmIoDto | null>(null);
  const [loading, setLoading] = useState<boolean>(false);

  useEffect(() => {
    if (demoMode || sessionId === null || turnSeq === null) return;
    let alive = true;
    setRefs(null);
    setCapture(null);
    setActive(0);
    void listArtifacts(sessionId, turnSeq).then((list) => {
      if (!alive) return;
      setRefs((list ?? []).filter((a) => a.kind === "llm_io"));
    });
    return () => {
      alive = false;
    };
  }, [demoMode, sessionId, turnSeq]);

  useEffect(() => {
    if (!refs || refs.length === 0) {
      setCapture(null);
      return;
    }
    const sel = refs[Math.min(active, refs.length - 1)];
    if (!sel || !sessionId || turnSeq === null) return;
    let alive = true;
    setLoading(true);
    void loadLlmIo(sessionId, turnSeq, sel.label).then((data) => {
      if (!alive) return;
      setCapture(data);
      setLoading(false);
    });
    return () => {
      alive = false;
    };
  }, [refs, active, sessionId, turnSeq]);

  if (demoMode) {
    return <p className="empty-note">{T.page.llmIo.empty}</p>;
  }
  if (!refs) {
    return <p className="empty-note">{T.page.loadingTrace}</p>;
  }
  if (refs.length === 0) {
    return <p className="empty-note">{T.page.llmIo.empty}</p>;
  }
  return (
    <LlmIoViewer
      artifacts={refs}
      activeIndex={active}
      onSelect={setActive}
      capture={capture}
      loading={loading}
    />
  );
}

function SpansTab({
  demoMode,
  sessionId,
  turnSeq,
  onOpenSpan,
}: {
  demoMode: boolean;
  sessionId: string | null;
  turnSeq: number | null;
  onOpenSpan: (ref: ArtifactRefDto) => void;
}) {
  const [refs, setRefs] = useState<ArtifactRefDto[] | null>(null);

  useEffect(() => {
    if (demoMode || sessionId === null || turnSeq === null) return;
    let alive = true;
    void listArtifacts(sessionId, turnSeq).then((list) => {
      if (!alive) return;
      const spans = (list ?? []).filter((a) => a.kind === "file_span");
      setRefs(spans);
    });
    return () => {
      alive = false;
    };
  }, [demoMode, sessionId, turnSeq]);

  if (demoMode) {
    return <p className="empty-note">{T.page.spansView.empty}</p>;
  }
  if (!refs) {
    return <p className="empty-note">{T.page.loadingTrace}</p>;
  }
  if (refs.length === 0) {
    return <p className="empty-note">{T.page.spansView.empty}</p>;
  }
  return (
    <div className="artifact-list">
      {refs.map((ref) => {
        const pathHint = ref.label.split("/").pop() ?? ref.label;
        return (
          <button
            key={ref.label}
            type="button"
            className="artifact-row"
            onClick={() => onOpenSpan(ref)}
            title={ref.label}
          >
            <code className="code-chip">{pathHint}</code>
            <span className="muted">
              {(ref.sizeBytes / 1024).toFixed(1)} KiB
            </span>
          </button>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Hook the open-span modal state up to the page
// ---------------------------------------------------------------------------
