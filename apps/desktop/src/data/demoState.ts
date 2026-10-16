import {
  archiveFooter,
  archivedConversations,
  demoArtifactRefs,
  demoItems,
  demoLlmIo,
  demoMemoryEntries,
  demoSpans,
  llmCalls,
  responseMeta,
  timeline,
  type ArchivedConversation,
  type TimelineRow,
} from "./demo";
import { changedFiles, conversation, project, projects, summary } from "./fixture";
import type {
  ArtifactRefDto,
  FileSpanDto,
  ItemDto,
  LlmIoDto,
  MemoryEntryDto,
} from "../api";
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
  /** Item-level fixture that backs the Response Trace "地图" tab on
   *  /ui-demo/trace when no live session is loaded. */
  items?: ItemDto[];
  /** Captured LLM I/O bodies for the demo Model calls. */
  llmIoCaptures?: LlmIoDto[];
  /** Captured file-span (before/after) bodies for the demo Edit calls. */
  spans?: FileSpanDto[];
  /** Artifact refs (matches what `list_artifacts` returns live). */
  artifactRefs?: ArtifactRefDto[];
  /** Memory entries (matches what `memory_list` returns live). The
   *  Memory inspector on /ui-demo/* reads these via the browser stub. */
  memoryEntries?: MemoryEntryDto[];
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
    items: demoItems,
    llmIoCaptures: demoLlmIo,
    spans: demoSpans,
    artifactRefs: demoArtifactRefs,
    memoryEntries: demoMemoryEntries,
    responseMeta,
    llmCalls,
    archivedConversations,
    archiveFooter,
  };
}
