import { ChevronRight } from "lucide-react";
import type { TraceStep } from "../data/types";
import { T } from "../i18n";
import { AgentTrace } from "./AgentTrace";
import { Markdown } from "./Markdown";

/** Public updates stay in arrival order; each intervening tool batch folds independently. */
export function ConversationProcess({ steps, fileDiffs, onOpenFile, onOpenPath }: {
  steps: TraceStep[];
  fileDiffs?: Record<string, string> | null;
  onOpenFile?: ((path: string) => void) | null;
  onOpenPath?: ((path: string) => void) | null;
}) {
  const groups: TraceStep[][] = [];
  for (const step of steps) {
    const last = groups.at(-1);
    if (step.type === "note" || !last || last[0].type === "note") groups.push([step]);
    else last.push(step);
  }

  return <div className="conversation-process" data-testid="conversation-process">
    {groups.map((group, index) => {
      const first = group[0];
      if (first.type === "note") return first.detail.trim() ? (
        <div key={`note-${first.stepId ?? index}`} className="conversation-note" data-testid="trace-note" aria-live="polite" aria-label={T.reply.progressNotes}>
          <Markdown text={first.detail} onOpenPath={onOpenPath} />
        </div>
      ) : null;
      const active = [...group].reverse().find((step) => step.status === "running");
      const failed = group.some((step) => step.status === "failed" || step.denied);
      const actions = group.filter((step) => ["search", "read", "run", "edit"].includes(step.type)).length;
      const label = failed ? T.reply.processFailed : active
        ? active.type === "model" ? T.reply.generating : active.type === "thinking" ? T.reply.working : `正在${active.label}`
        : T.reply.process;
      return <details key={`tools-${first.stepId ?? index}`} className="conversation-tools" data-testid="process-tools" open={failed ? true : undefined}>
        <summary>
          <ChevronRight size={14} aria-hidden />
          <span>{label}</span>
          {actions > 0 && <span className="conversation-tools-count">{T.reply.stepsCount(actions)}</span>}
        </summary>
        <AgentTrace steps={group} fileDiffs={fileDiffs} onOpenFile={onOpenFile} />
      </details>;
    })}
  </div>;
}
