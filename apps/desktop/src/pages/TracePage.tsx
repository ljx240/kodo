import {
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
import { useEffect, useState } from "react";
import { loadSession, type SessionDto } from "../api";
import { formatDuration, formatTokens, mergeChanges, totals } from "../conversation/trace";
import { type DemoState, type DemoState as Demo } from "../data/demoState";
import type { ChangedFile } from "../data/types";
import { ReplyMeta } from "../inspector/TraceInspector";
import { FileRow } from "../inspector/Section";
import { T, toolAlias } from "../i18n";

const TABS = [
  { id: "timeline", label: T.page.traceTabs.timeline },
  { id: "logs", label: T.page.traceTabs.logs },
  { id: "artifacts", label: T.page.traceTabs.artifacts },
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

  if (!demoMode && !conversationId) {
    return (
      <main className="main">
        <div className="scroll">
          <p className="empty-note">{T.page.selectConversation}</p>
        </div>
      </main>
    );
  }

  if (!demoMode && !session) {
    return (
      <main className="main">
        <div className="scroll">
          <p className="empty-note">{T.page.loadingTrace}</p>
        </div>
      </main>
    );
  }

  const data = demoMode && demo
    ? {
        rows: demo.timeline,
        final: demo.conversation.assistant.final,
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
      <header className="topbar topbar--plain">
        <nav className="crumbs">
          <Folder size={14} strokeWidth={1.7} />
          <span>{data.crumbProject}</span>
          <span className="crumb-sep">/</span>
          <span>{data.crumbTitle}</span>
          <span className="crumb-sep">/</span>
          <span className="crumb-current">{T.page.responseTrace}</span>
        </nav>
        <span className="spacer" />
      </header>

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

          {tab === "timeline" && (
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
          )}
        </div>
      </div>
    </main>
  );
}
