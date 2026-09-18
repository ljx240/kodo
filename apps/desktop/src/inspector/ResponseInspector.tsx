import { BarChart3, Check, Folder, History, Sparkles, Terminal } from "lucide-react";
import { changedFiles, conversation, project, summary } from "../data/fixture";
import type { LiveSnapshot } from "../data/liveContext";
import { changesFromReply, commandsFromTurn, llmFromTurn, toolsFromTurn } from "../data/liveContext";
import { formatDuration } from "../conversation/trace";
import { FileRow, MetaRow, Section, ToolRow } from "./Section";

export type InspectorData = {
  /** Demo routes keep fixture; live routes pass the snapshot (possibly null). */
  demo: boolean;
  live: LiveSnapshot | null;
};

function EmptyBody({ note }: { note: string }) {
  return (
    <div className="ins-body">
      <p className="ins-note">{note}</p>
    </div>
  );
}

export function ThisResponse({ demo, live }: InspectorData) {
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
  return (
    <Section
      icon={<History size={14} strokeWidth={1.7} />}
      title="This response"
      meta={<span className="ins-time">{conversation.assistant.time}</span>}
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
        <MetaRow label="Duration">{conversation.assistant.duration}</MetaRow>
        <MetaRow label="Total steps">{conversation.assistant.trace.length}</MetaRow>
        <MetaRow label="Input tokens">14.5k</MetaRow>
        <MetaRow label="Output tokens">2.1k</MetaRow>
        <MetaRow label="Model">Claude 3.5 Sonnet</MetaRow>
      </div>
    </Section>
  );
}

export function ChangedFilesSection({ demo, live, limit }: InspectorData & { limit?: number }) {
  const files = !demo
    ? changesFromReply(live?.reply ?? null)
    : limit
      ? changedFiles.slice(0, limit)
      : changedFiles;
  const total = !demo ? files.length : summary.files_changed;
  const shown = limit && demo ? files.slice(0, limit) : files;

  return (
    <Section icon={<BarChart3 size={14} strokeWidth={1.7} />} title="Changed files" count={total}>
      <div className="ins-body">
        {!demo && files.length === 0 ? (
          <p className="ins-note">本轮没有记录到文件变更。</p>
        ) : (
          shown.map((file) => <FileRow key={file.path} file={file} />)
        )}
      </div>
      {limit && demo && (
        <button type="button" className="ins-link">
          Show all {summary.files_changed} files →
        </button>
      )}
    </Section>
  );
}

export function ToolsSection({ demo, live }: InspectorData) {
  const tools = !demo ? toolsFromTurn(live?.turn ?? null) : summary.tools_used;
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

export function LlmSection({ demo, live, detailed = false }: InspectorData & { detailed?: boolean }) {
  const calls = !demo ? llmFromTurn(live?.turn ?? null) : summary.llm_calls;
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

export function CurrentProjectSection({ demo, live }: InspectorData) {
  const name = !demo ? live?.projectName || "未选择项目" : project.name;
  const path = !demo ? live?.projectPath || "—" : project.path;
  return (
    <Section icon={<Folder size={14} strokeWidth={1.7} />} title="Current project">
      <div className="ins-body">
        <div className="project-name">{name}</div>
        <div className="project-path">{path}</div>
      </div>
    </Section>
  );
}

export function TerminalSection({ demo, live }: InspectorData) {
  const runs = !demo
    ? commandsFromTurn(live?.turn ?? null)
    : conversation.assistant.trace
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

export function ResponseOverview(props: InspectorData) {
  return (
    <>
      <ThisResponse {...props} />
      <ChangedFilesSection {...props} limit={5} />
      <ToolsSection {...props} />
      <LlmSection {...props} />
      <CurrentProjectSection {...props} />
    </>
  );
}

/** Kept for Trace page formatting helpers that imported formatDuration from here historically. */
export { formatDuration };
