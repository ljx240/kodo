/** Shapes mirroring demo/conversation.fixture.json. */

export type ConversationRef = {
  id: string;
  title: string;
  time: string;
  active?: boolean;
};

export type Project = {
  id: string;
  /**
   * The directory on disk. Set only for projects registered in the core; the
   * fixture's projects stand in for a workspace and have no real path.
   */
  path?: string;
  name: string;
  expanded: boolean;
  conversations: ConversationRef[];
};

export type StepStatus = "done" | "running" | "failed" | "interrupted" | "stopped";

export type TraceStep = {
  type: "thinking" | "search" | "read" | "run" | "model" | "edit" | "finalize";
  label: string;
  detail: string;
  /**
   * The mono chip, when the caller knows it outright. The fixture leaves this
   * unset and relies on [`splitDetail`], which is a guess that only works
   * because its chip is pure ASCII and its prose pure CJK; real steps carry the
   * two fields separately, so they say so instead of being re-split.
   */
  chip?: string;
  duration: string;
  /** Raw milliseconds when known; 0 means "so fast it is hidden". */
  durationMs?: number | null;
  status: StepStatus;
  output?: string;
  model?: string;
  input_tokens?: string;
  output_tokens?: string;
  /** Command working directory, when the runner recorded one. */
  cwd?: string;
  /** Command exit code; null while still running. */
  exitCode?: number | null;
  /** Paths touched by a file-change step. */
  files?: string[];
  /** Public phase code (prepare/analyze/execute/verify/summarize). */
  phase?: string;
  /** Structured failure taxonomy code from the backend. */
  failureClass?: string | null;
  /** Missing binary when failureClass is command_not_found. */
  failureTool?: string | null;
  /** The user refused this command. */
  denied?: boolean;
  /** Internal scheduling diagnostics — Debug surfaces only. */
  diagnostics?: string | null;
  /** Stable id for expand/collapse + aria wiring. */
  stepId?: number;
};

export type ChangedFile = {
  path: string;
  added: number;
  removed: number;
};

export type LlmCall = {
  model: string;
  input_tokens: string;
  output_tokens: string;
  duration: string;
};

export type Fixture = {
  project: { id: string; name: string; path: string; branch: string };
  projects: Project[];
  conversation: {
    id: string;
    title: string;
    user: { time: string; content: string };
    assistant: {
      time: string;
      status: StepStatus;
      duration: string;
      trace: TraceStep[];
      final: string;
      checks: string[];
    };
  };
  changed_files: ChangedFile[];
  summary: {
    files_changed: number;
    added: number;
    removed: number;
    tools_used: Record<string, number>;
    llm_calls: LlmCall[];
  };
};

/**
 * Splits a fixture detail such as `cargo check 检查项目编译状态` into the
 * command chip the references render in mono and the trailing prose.
 * The fixture itself stays untouched.
 */
export function splitDetail(detail: string): { chip: string | null; text: string } {
  const index = detail.search(/[一-鿿]/);
  if (index <= 0) return { chip: null, text: detail };
  return { chip: detail.slice(0, index).trim(), text: detail.slice(index) };
}
