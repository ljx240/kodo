import { archiveFooter, archivedConversations, llmCalls, responseMeta, timeline, type ArchivedConversation, type TimelineRow } from "./demo";
import { changedFiles, conversation, project, projects, summary } from "./fixture";
import type { ChangedFile, Project } from "./types";

/**
 * The only module outside the raw `demo.ts` / `fixture.ts` files that may
 * import deterministic demo data.
 *
 * Production routes (Conversation / Trace / Settings / Inspector) must take
 * this state as props from the shell — never by importing fixture/demo —
 * so a live build cannot accidentally paint sample content.
 */
export type DemoState = {
  conversation: typeof conversation;
  project: typeof project;
  projects: Project[];
  summary: typeof summary;
  changedFiles: ChangedFile[];
  timeline: TimelineRow[];
  responseMeta: typeof responseMeta;
  llmCalls: typeof llmCalls;
  archivedConversations: ArchivedConversation[];
  archiveFooter: typeof archiveFooter;
};

export function loadDemoState(): DemoState {
  return {
    conversation,
    project,
    projects,
    summary,
    changedFiles,
    timeline,
    responseMeta,
    llmCalls,
    archivedConversations,
    archiveFooter,
  };
}
