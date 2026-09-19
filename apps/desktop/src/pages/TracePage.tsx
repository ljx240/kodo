import {
  BarChart3,
  Check,
  CheckCircle2,
  Circle,
  Folder,
  Pencil,
  Play,
  Search,
  Share2,
  Sparkles,
  MoreHorizontal,
} from "lucide-react";
import { useEffect, useState } from "react";
import { loadSession, type SessionDto } from "../api";
import { formatDuration, formatTokens } from "../conversation/trace";
import { changedFiles, conversation, project } from "../data/fixture";
import { timeline, type TimelineRow } from "../data/demo";
import { ReplyMeta } from "../inspector/TraceInspector";
import { FileRow } from "../inspector/Section";

const TABS = ["Timeline", "Logs", "Artifacts"] as const;
type Tab = (typeof TABS)[number];

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
      <span>{type}</span>
    </span>
  );
}

function liveTimeline(session: SessionDto): {
  rows: TimelineRow[];
  final: string;
  checks: string[];
  files: typeof changedFiles;
  done: boolean;
  stopped: boolean;
} {
  const turn = session.turns[session.turns.length - 1];
  if (!turn) {
    return { rows: [], final: "", checks: [], files: [], done: false, stopped: false };
  }
  let final = "";
  let checks: string[] = [];
  const files: typeof changedFiles = [];
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
        title: "Finalize answer",
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
        rows.push({ ...base, type: "Thinking", title: "Thinking", note: item.summary });
        break;
      case "search":
        rows.push({ ...base, type: "Search", title: "Search codebase", note: item.detail, chip: item.query });
        break;
      case "fileRead":
        rows.push({ ...base, type: "Read", title: "Read file", note: item.detail, chip: item.path });
        break;
      case "commandExecution":
        rows.push({
          ...base,
          type: "Run",
          title: "Run command",
          note: item.output.slice(0, 80) || "命令输出",
          chip: item.command,
          ok: item.exitCode === 0,
        });
        break;
      case "modelCall":
        rows.push({
          ...base,
          type: "Model",
          title: "Model call",
          note: "调用模型",
          chips: [item.model, `${formatTokens(item.inputTokens)} → ${formatTokens(item.outputTokens)}`],
        });
        break;
      case "fileChange":
        item.changes.forEach((change) => files.push(change));
        rows.push({
          ...base,
          type: "Edit",
          title: "Edit files",
          note: `${item.changes.length} 个文件`,
          delta: {
            added: item.changes.reduce((n, c) => n + c.added, 0),
            removed: item.changes.reduce((n, c) => n + c.removed, 0),
          },
        });
        break;
    }
  });

  return { rows, final, checks, files, done: turn.done, stopped: turn.stopped };
}

export function TracePage({
  demo,
  conversationId,
  live,
}: {
  demo: boolean;
  conversationId: string | null;
  live?: SessionDto | null;
}) {
  const [tab, setTab] = useState<Tab>("Timeline");
  const [session, setSession] = useState<SessionDto | null>(live ?? null);

  useEffect(() => {
    if (demo) {
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
  }, [demo, conversationId, live]);

  if (!demo && !conversationId) {
    return (
      <main className="main">
        <div className="scroll">
          <p className="empty-note">先在侧边栏选择一个会话，再查看 Response Trace。</p>
        </div>
      </main>
    );
  }

  if (!demo && !session) {
    return (
      <main className="main">
        <div className="scroll">
          <p className="empty-note">正在加载会话轨迹…</p>
        </div>
      </main>
    );
  }

  const data = demo
    ? {
        rows: timeline,
        final: conversation.assistant.final,
        checks: [] as string[],
        files: changedFiles,
        done: true,
        stopped: false,
        crumbProject: project.name,
        crumbTitle: conversation.title,
      }
    : (() => {
        const mapped = liveTimeline(session!);
        return {
          ...mapped,
          crumbProject: session!.project.split("/").pop() || session!.project,
          crumbTitle: session!.title,
        };
      })();

  if (!demo && data.rows.length === 0) {
    return (
      <main className="main">
        <div className="scroll">
          <p className="empty-note">当前会话没有可展示的执行记录。</p>
        </div>
      </main>
    );
  }

  const statusLabel = demo ? "Completed" : data.stopped ? "Stopped" : data.done ? "Completed" : "Interrupted";
  const logs = data.rows.map((row) => {
    const detail = row.chip ?? row.chips?.join(" ") ?? "";
    return `${row.time}  ${String(row.duration).padStart(4)}  ${row.type.padEnd(9)} ${row.title}${detail ? ` ${detail}` : ""}`;
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
          <span className="crumb-current">Response Trace</span>
        </nav>
        <span className="spacer" />
        <button type="button" className="btn" disabled title="Coming later">
          <Share2 size={15} strokeWidth={1.9} />
          <span>Share</span>
        </button>
        <button type="button" className="icon-btn" aria-label="More" disabled title="Coming later">
          <MoreHorizontal size={16} strokeWidth={1.7} />
        </button>
      </header>

      <div className="scroll">
        <div className="page-inner page-inner--wide">
          <section className="reply-card">
            <div className="reply-card-head">
              <span className="reply-mark">
                <Sparkles size={14} strokeWidth={1.9} />
              </span>
              <h1 className="reply-card-title">Assistant Reply</h1>
              <span className="status-pill">
                <span className="status-mark">
                  <Check size={9} strokeWidth={4} />
                </span>
                <span>{statusLabel}</span>
              </span>
            </div>

            <div className="reply-card-body">
              <ReplyMeta demo={demo} live={demo ? null : {
                conversationId: session!.id,
                title: session!.title,
                projectName: data.crumbProject,
                projectPath: session!.project,
                turn: session!.turns[session!.turns.length - 1] ?? null,
                reply: null,
                running: false,
              }} />
              <div className="reply-summary">
                <h4>Final response summary</h4>
                <p>{data.final || "（无最终回复）"}</p>
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

          <nav className="page-tabs" role="tablist" aria-label="Trace views">
            {TABS.map((name) => (
              <button
                key={name}
                type="button"
                role="tab"
                className={`page-tab${name === tab ? " page-tab--active" : ""}`}
                aria-selected={name === tab}
                onClick={() => setTab(name)}
              >
                {name}
              </button>
            ))}
          </nav>

          {tab === "Logs" && <pre className="terminal-block terminal-block--tall">{logs.join("\n") || "（暂无日志）"}</pre>}

          {tab === "Artifacts" && (
            <div className="artifact-list">
              {data.files.length === 0 ? (
                <p className="empty-note">本轮没有文件变更。</p>
              ) : (
                data.files.map((file) => <FileRow key={file.path} file={file} />)
              )}
            </div>
          )}

          {tab === "Timeline" && (
            <table className="timeline">
              <thead>
                <tr>
                  <th className="col-n">#</th>
                  <th className="col-time">Time</th>
                  <th className="col-duration">Duration</th>
                  <th className="col-type">Type</th>
                  <th>Details</th>
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
                        <span>{row.title}</span>
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
