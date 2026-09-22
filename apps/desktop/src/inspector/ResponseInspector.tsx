import { BarChart3, Check, Folder, History, Sparkles, Terminal, Undo2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { turnChanges, undoTurn, type TurnChangeDto } from "../api";
import type { DemoState } from "../data/demoState";
import type { LiveSnapshot } from "../data/liveContext";
import { changesFromReply, commandsFromTurn, llmFromTurn, toolsFromTurn } from "../data/liveContext";
import { formatDuration } from "../conversation/trace";
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
          setMessage("撤销失败：未得到撤销报告");
          return;
        }
        const parts: string[] = [];
        if (report.restored.length) {
          parts.push(`已撤销 ${report.restored.length} 个 Kodo 修改`);
        }
        if (report.conflicts.length) {
          const details = report.conflicts
            .map((c) => `${c.path}（${c.reason}）`)
            .join("、");
          parts.push(`撤销冲突 ${report.conflicts.length} 个：${details} — 已保留您的修改`);
        }
        if (!parts.length) {
          parts.push("没有可撤销的 Kodo 修改");
        }
        setMessage(parts.join(" · "));
        reload();
      })
      .catch((error: unknown) => setMessage(`撤销失败：${String(error)}`))
      .finally(() => setUndoing(false));
  };

  if (!projectPath || !sessionId) return null;
  if (loadError) {
    return (
      <div className="ins-body">
        <p className="ins-note" data-testid="diff-error" role="alert">
          加载 diff 失败：{loadError}
        </p>
        <button type="button" className="btn btn--sm" data-testid="diff-retry" onClick={reload}>
          重试
        </button>
      </div>
    );
  }
  if (!changes) {
    return (
      <div className="ins-body">
        <p className="ins-note" data-testid="diff-loading">
          加载本轮 diff…
        </p>
      </div>
    );
  }
  if (changes.length === 0) {
    return (
      <div className="ins-body">
        <p className="ins-note" data-testid="diff-empty">
          本轮没有 Kodo 文件修改。
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
              {change.userPreexisting ? " · (pre-existing user change)" : ""}
              {change.conflict ? " · (undo conflict)" : ""}
              {!change.conflict && !change.userPreexisting ? " · (Kodo change)" : ""}
            </button>
            {change.conflict && (
              <p className="ins-note" data-testid={`conflict-${change.path}`}>
                Undo conflict — working tree diverged from Kodo&apos;s after-hash;您的后续修改会被保留
              </p>
            )}
            {open === change.path && (
              <pre className="terminal-block diff-block" data-testid={`diff-body-${change.path}`}>
                {change.diff || "(no diff)"}
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
          title={undoing ? "正在撤销…" : "撤销本轮 Kodo 修改（不会覆盖您的后续编辑）"}
          onClick={undo}
        >
          <Undo2 size={13} strokeWidth={1.8} />
          <span>{undoing ? "撤销中…" : "撤销 Kodo 修改"}</span>
        </button>
      </div>
    </>
  );
}

export function ThisResponse({ demo, demoState, live }: InspectorData) {
  const fixture = demo ? demoState?.conversation : undefined;
  if (!demo && live) {
    const reply = live.reply;
    const steps = reply?.steps.length ?? 0;
    return (
      <Section
        icon={<History size={14} strokeWidth={1.7} />}
        title="This response"
        meta={<span className="ins-time">{live.running ? "进行中" : ""}</span>}
      >
        <div className="ins-body">
          <MetaRow label="Status">
            <span className="status-pill">
              <span className="status-mark">{live.running ? "…" : <Check size={9} strokeWidth={4} />}</span>
              <span>{live.running ? "Working" : live.turn?.stopped ? "Stopped" : live.turn?.done ? "Completed" : "—"}</span>
            </span>
          </MetaRow>
          <MetaRow label="Conversation">{live.title}</MetaRow>
          <MetaRow label="Total steps">{steps}</MetaRow>
        </div>
      </Section>
    );
  }
  if (!demo && !live) {
    return (
      <Section icon={<History size={14} strokeWidth={1.7} />} title="This response">
        <EmptyBody note="尚未打开会话，或当前会话还没有回复。" />
      </Section>
    );
  }
  if (!fixture) {
    return (
      <Section icon={<History size={14} strokeWidth={1.7} />} title="This response">
        <EmptyBody note="演示数据未加载。" />
      </Section>
    );
  }
  return (
    <Section
      icon={<History size={14} strokeWidth={1.7} />}
      title="This response"
      meta={<span className="ins-time">{fixture.assistant.time}</span>}
    >
      <div className="ins-body">
        <MetaRow label="Status">
          <span className="status-pill">
            <span className="status-mark">
              <Check size={9} strokeWidth={4} />
            </span>
            <span>Completed</span>
          </span>
        </MetaRow>
        <MetaRow label="Duration">{fixture.assistant.duration}</MetaRow>
        <MetaRow label="Total steps">{fixture.assistant.trace.length}</MetaRow>
        <MetaRow label="Input tokens">14.5k</MetaRow>
        <MetaRow label="Output tokens">2.1k</MetaRow>
        <MetaRow label="Model">Claude 3.5 Sonnet</MetaRow>
      </div>
    </Section>
  );
}

export function ChangedFilesSection({ demo, demoState, live, limit, onShowAll }: InspectorData & {
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
    <Section icon={<BarChart3 size={14} strokeWidth={1.7} />} title="Changed files" count={total}>
      {!demo && live?.conversationId && live.projectPath ? (
        <LiveDiffPanel sessionId={live.conversationId} projectPath={live.projectPath} />
      ) : (
        <div className="ins-body">
          {!demo && files.length === 0 ? (
            <p className="ins-note">本轮没有记录到文件变更。</p>
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
          Show all {total} files →
        </button>
      )}
    </Section>
  );
}

export function ToolsSection({ demo, demoState, live }: InspectorData) {
  const tools = !demo ? toolsFromTurn(live?.turn ?? null) : demoState?.summary.tools_used ?? {};
  const entries = Object.entries(tools);
  return (
    <Section icon={<Terminal size={14} strokeWidth={1.7} />} title="Tools used" count={entries.length}>
      <div className="ins-body">
        {entries.length === 0 ? (
          <p className="ins-note">暂无工具调用。</p>
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
    <Section icon={<Sparkles size={14} strokeWidth={1.7} />} title="LLM calls" count={calls.length}>
      <div className="ins-body">
        {calls.length === 0 ? (
          <p className="ins-note">暂无模型调用记录。</p>
        ) : (
          calls.map((call) => (
            <div key={`${call.model}-${call.input_tokens}`} className="llm-call">
              <div className="llm-head">
                <Sparkles size={14} strokeWidth={1.7} className="llm-icon" />
                <span className="llm-model">{call.model}</span>
                {detailed && <span className="llm-time">{call.duration}</span>}
                {!detailed && <span className="llm-duration">{call.duration}</span>}
              </div>
              <MetaRow label="Input tokens">{call.input_tokens}</MetaRow>
              <MetaRow label="Output tokens">{call.output_tokens}</MetaRow>
              {detailed && <MetaRow label="Duration">{call.duration}</MetaRow>}
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
    <Section icon={<Folder size={14} strokeWidth={1.7} />} title="Current project">
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
    <Section icon={<Terminal size={14} strokeWidth={1.7} />} title="Terminal output" count={runs.length}>
      <div className="ins-body">
        {runs.length === 0 ? (
          <p className="ins-note">暂无命令输出。</p>
        ) : (
          runs.map((run) => (
            <pre key={run.command} className="terminal-block">
              {run.output || "(无输出)"}
            </pre>
          ))
        )}
      </div>
    </Section>
  );
}

export function ResponseOverview(props: InspectorData & { onShowAllFiles?: () => void }) {
  const { onShowAllFiles, ...data } = props;
  return (
    <>
      <ThisResponse {...data} />
      <ChangedFilesSection {...data} limit={5} onShowAll={onShowAllFiles} />
      <ToolsSection {...data} />
      <LlmSection {...data} />
      <CurrentProjectSection {...data} />
    </>
  );
}

/** Kept for Trace page formatting helpers that imported formatDuration from here historically. */
export { formatDuration };
