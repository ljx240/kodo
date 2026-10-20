import { ArrowRight, BookOpen, Bug, Check, Copy, Pencil, Sparkles, Wrench } from "lucide-react";
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
// Welcome-identity marks — same family as the sidebar brand mark, one per
// agent space (DESIGN §11). Vite resolves these to hashed asset URLs.
import kodoCodeMark from "../assets/kodo-code-mark.png";
import kodoWorkMark from "../assets/kodo-work-mark.png";
import {
  loadSession,
  onRunEvent,
  probeContextFile,
  respondApproval,
  runState,
  sendMessage,
  stopRun,
  taskFeedback,
  turnChanges,
  undoTurn,
  validateContextPath,
  type ChangeDto,
  type ContextProbe,
  type InterimNote,
  type ItemDto,
  type RunEventDto,
  type TurnDto,
} from "../api";
import { AssistantReply } from "../conversation/AssistantReply";
import { Composer } from "../conversation/Composer";
import { ContextCard } from "../conversation/ContextCard";
import { FindBar } from "../conversation/FindBar";
import { KnowledgeBinding } from "../conversation/KnowledgeBinding";
import { KnowledgeSources } from "../conversation/KnowledgeSources";
import { TurnRail, type TurnRailItem } from "../conversation/TurnRail";
import type { FilePanelRequest } from "../inspector/Inspector";
import {
  durationMsOf,
  groupPhases,
  mergeChanges,
  mergeFailures,
  outcomeFor,
  titleFromAsk,
  toReply,
  totals,
  verifyCounts,
  type ContextBudget,
  type Reply,
  type RunStatus,
} from "../conversation/trace";
import { type DemoState } from "../data/demoState";
import { kindFromPath } from "../fileKind";
import {
  knowledgeGetBindings,
  knowledgeListBases,
  knowledgePrepareTurn,
  knowledgeSetBindings,
  type KnowledgeBase,
  type KnowledgeSelection,
  type PreparedTurn,
} from "../knowledgeApi";
import { snapshotFromTurn, type LiveSnapshot } from "../data/liveContext";
import { type ProviderConfig } from "../data/providers";
import type { Project } from "../data/types";
import { T } from "../i18n";
import { navigate } from "../routes";
import type { ConvActions } from "../shell/TopBar";

type Props = {
  conversationId: string | null;
  /** Which agent space is showing (owned by App — this page only reads it). */
  agentMode: string;
  provider: ProviderConfig | null;
  providers: ProviderConfig[];
  /** Default-model value shared with Settings and the top-bar chip (§5b). */
  defaultModel?: string;
  onSelectProvider: (index: number) => void;
  onSelectProviderModel: (providerIndex: number, modelId: string, displayName: string) => void;
  onViewFiles: () => void;
  onRetitle: (id: string, title: string) => Promise<void> | void;
  onArchive: (id: string) => Promise<void> | void;
  onOpenTrace: () => void;
  /**
   * Publishes stable rename/archive entry points for the top bar (the content
   * area lost its title header on 2026-09-30 — App hosts the buttons, this
   * page keeps the handlers and their failure banner). Null while no live
   * conversation is open.
   */
  onConvActions?: (actions: ConvActions | null) => void;
  /** Pushed by the run-event stream: the count of memories the agent
   *  actually injected into the user-message observation block this turn.
   *  App uses this to render the live count on the TopBar Memory chip.
   *  `null` = no event this turn (engine off). */
  onMemoryRecalled?: (count: number) => void;
  /** Opens App's right-sidebar 文件 panel (demo / no project → onViewFiles). */
  onOpenFilePanel?: (request: FilePanelRequest) => void;
  /** Publishes the per-file diff map up so the panel can render 变更 tabs. */
  onFileDiffs?: (diffs: Record<string, string>) => void;
  onSnapshot?: (snapshot: LiveSnapshot | null) => void;
  onConversationCreated?: (id: string) => void;
  onConversationCommitted?: () => Promise<void> | void;
  onOpenSettings?: () => void;
  /** Same path the sidebar/palette take — the welcome 「继续上次对话」 CTA. */
  onOpenConversation?: (id: string) => void;
  projects: Project[];
  projectName?: string;
  projectPath?: string;
  /**
   * Hidden but still mounted — the Response Trace route keeps a live run's
   * page alive so its event stream never tears down mid-answer (2026-09-30).
   */
  stowed?: boolean;
  /**
   * Deterministic fixture for `/ui-demo` only. Live routes pass null and must
   * never import data/fixture or data/demo themselves.
   */
  demo?: DemoState | null;
  /**
   * Composer persona chain, keyed by session (App-owned, in-memory, lost
   * on refresh). The chain is applied in array order — last entry wins
   * for any conflicting session-state binding the persona carries.
   */
  personaListBySession: Record<string, string[]>;
  onSelectAgents: (sessionKey: string, names: string[] | null) => void;
  /** Persona list version — bumped after AgentsTab CRUD so the Composer
   *  dropdown refetches. */
  agentsVersion?: number;
  /** Favorited persona names — the composer's 常用 section reads them. */
  favoriteAgents: string[];
  /** Recently used personas (MRU, capped at 5) — the 最近用 section. */
  recentAgents: Array<{ name: string; at: number }>;
  /** Record one persona as just-used; fires after a successful send. */
  recordRecentAgent: (name: string) => void;
};

type Approval = {
  runId?: number;
  step: number;
  kind: string;
  detail: string;
  cwd?: string;
  riskCategory?: string;
  reason?: string;
};

/** Recoverable failure: message + optional retry / recovery action. */
type ActionErrorState = {
  message: string;
  retry?: () => void;
  retryLabel?: string;
  action?: { label: string; run: () => void };
};

type PendingSend = { text: string; context: string[] };

type FailoverNotice = {
  fromProvider: string;
  fromModel: string;
  errorClass: string;
  error: string;
  toProvider: string;
  toModel: string;
};

function demoReplyFor(demo: DemoState, index: number): Reply {
  const turn = demo.conversation.turns[index];
  // Fixture list is the source of truth; totals re-derive so the card never
  // claims a file count the list does not show. Fixture steps predate the
  // structured fields: labels come from the central copy table and durations
  // are parsed back into milliseconds for the phase headers.
  const steps = turn.assistant.trace.map((step) => ({
    ...step,
    label: T.step[step.type],
    durationMs: step.durationMs ?? durationMsOf(step.duration),
  }));
  // The recorded file changes belong to turn 1's task; later demo turns are
  // analysis/writing and stay file-free so the shared summary stays truthful.
  const changes =
    index === 0 ? mergeChanges(demo.changedFiles.map((file) => ({ ...file, edits: 1 }))) : [];
  const verify = verifyCounts(steps);
  const status: RunStatus = {
    lifecycle: "completed",
    delivery: "ready",
    // The fixture records its acceptance checks as passing.
    verification: verify.failed > 0 ? "failed" : "passed",
    outcome: "completed",
  };
  status.outcome = outcomeFor(status.lifecycle, status.delivery, status.verification, changes.length > 0);
  return {
    steps,
    // Demo turns are task reports; conversational `plain` answers are live-only.
    plain: false,
    phases: groupPhases(steps),
    status,
    final: turn.assistant.final,
    checks: turn.assistant.checks,
    changes,
    // The legacy fixture has no post-write read events to attest a readback.
    documentReadbacks: [],
    ...totals(changes),
    interrupted: false,
    stopped: false,
    error: null,
    models: demo.summary.llm_calls.map((call) => call.model),
    tokens: null,
    tokensPending: false,
    contextBudget: null,
    verify,
    failureGroups: mergeFailures(steps),
    failureReason: null,
  };
}

export function ConversationPage({
  conversationId,
  agentMode,
  provider,
  providers,
  defaultModel,
  onSelectProvider,
  onSelectProviderModel,
  onViewFiles,
  onRetitle,
  onArchive,
  onOpenTrace,
  onConvActions,
  onMemoryRecalled,
  onOpenFilePanel,
  onFileDiffs,
  onSnapshot,
  onConversationCreated,
  onConversationCommitted,
  onOpenSettings,
  onOpenConversation,
  projects,
  projectName = "",
  projectPath = "",
  stowed = false,
  demo = null,
  personaListBySession,
  onSelectAgents,
  favoriteAgents,
  recentAgents,
  recordRecentAgent,
  agentsVersion = 0,
}: Props) {
  const demoMode = Boolean(demo);
  const openSettings = onOpenSettings ?? navigateSettings;
  const [turns, setTurns] = useState<TurnDto[]>([]);
  const [createdConversationId, setCreatedConversationId] = useState<string | null>(null);
  const [title, setTitle] = useState("");
  const [running, setRunning] = useState(false);
  const [approval, setApproval] = useState<Approval | null>(null);
  /** 编辑并发送: a user turn sent back to the composer (nonce forces refocus). */
  const [editSeed, setEditSeed] = useState<{ text: string; nonce: number } | null>(null);
  /** ⌘F in-conversation find — hits cover each turn's ask and final reply. */
  const [find, setFind] = useState({ open: false, query: "", index: 0 });
  const [contexts, setContexts] = useState<string[]>([]);
  /** Attachment probes by path — shared by composer cards and sent bubbles. */
  const [probes, setProbes] = useState<Record<string, ContextProbe>>({});
  const [pageError, setPageError] = useState<ActionErrorState | null>(null);
  const [approvalError, setApprovalError] = useState<string | null>(null);
  // Provider stream text is never shown as a draft up top. It accumulates here
  // and flushes as in-order interim notes at step boundaries; whatever remains
  // when the final answer lands is the draft of that answer and is discarded.
  const textBufRef = useRef("");
  const noteSeqRef = useRef(0);
  const [progress, setProgress] = useState<{ phase: string; detail: string } | null>(null);
  const [failovers, setFailovers] = useState<string[]>([]);
  const [failover, setFailover] = useState<FailoverNotice | null>(null);
  /** Latest context-window telemetry (Codex-style progress bar fuel). */
  const [budget, setBudget] = useState<ContextBudget | null>(null);
  /** One-shot toast for the most recent compaction pass; auto-cleared. */
  const [compactToast, setCompactToast] = useState<{ dropped: number; freedPct: number } | null>(null);
  const compactToastTimerRef = useRef<number | null>(null);
  /** One-shot toast for an adaptive mid-turn budget lift. Cleared on the
   *  next turnStarted / stop, or after 1500 ms if no follow-up arrives. */
  const [budgetLiftToast, setBudgetLiftToast] = useState<{ detail: string } | null>(null);
  const budgetLiftTimerRef = useRef<number | null>(null);
  const [queue, setQueueState] = useState<PendingSend[]>([]);
  const [runStartedAt, setRunStartedAt] = useState<number | null>(null);
  const [elapsed, setElapsed] = useState<number | null>(null);
  /** Vision-incapable + image-attached: first-time per session banner. The
   *  user dismisses with `知道了`; subsequent send-with-image calls skip
   *  the gate. Persisted only for the page session, not across reloads —
   *  a hard reload reminds the user once (matches the plan: "首次提示，
   *  之后静默降级"). */
  const [visionSkipAcked, setVisionSkipAcked] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    return window.sessionStorage.getItem("kodo:vision-img-skip-acked") === "1";
  });
  /** The send we are about to dispatch once the user acknowledges the
   *  vision-skip banner. Held in state so re-renders don't lose the
   *  payload. `null` means the banner is closed. */
  const [pendingVisionSend, setPendingVisionSend] = useState<PendingSend | null>(null);
  const [fileDiffs, setFileDiffs] = useState<Record<string, string> | null>(null);
  /** Paths the user had already dirtied / where undo would conflict. */
  const [turnMeta, setTurnMeta] = useState<{ preExisting: string[]; conflicts: string[] }>({
    preExisting: [],
    conflicts: [],
  });
  const [undoingChanges, setUndoingChanges] = useState(false);
  /** Honest undo feedback for the reply card (已撤销 / 无可撤销 / 冲突明细). */
  const [undoMessage, setUndoMessage] = useState<string | null>(null);
  /** Every "show this file" ask funnels here and lands in App's file panel;
   * demo / no project falls back to Inspector Files (no file bodies there). */
  const openFilePanel = (path: string | null, tab: "full" | "diff", changes: ChangeDto[]) => {
    if (demoMode || !projectPath || !onOpenFilePanel) {
      onViewFiles();
      return;
    }
    onOpenFilePanel({ path, tab, changes, nonce: Date.now() });
  };
  /** Click-through from answer markdown / context cards: two-tier validation —
   *  shape already gated at render time, now the probe decides. Project-relative
   *  paths pass straight through; absolute paths must live under the project
   *  root (the prefix is stripped). Anything else is a no-op; demo falls back
   *  to Inspector's 改动 section like every other file entry. */
  const openReferencedFile = async (raw: string, changes: ChangeDto[]) => {
    const trimmed = raw.trim();
    if (!trimmed) return;
    if (demoMode || !projectPath) {
      onViewFiles();
      return;
    }
    if (trimmed.includes("://") || trimmed.startsWith("..")) return;
    const root = projectPath.replace(/\/+$/, "");
    let rel = trimmed;
    if (trimmed.startsWith("/")) {
      if (!trimmed.startsWith(`${root}/`)) return;
      rel = trimmed.slice(root.length + 1);
    }
    if (await validateContextPath(projectPath, rel)) {
      openFilePanel(rel, "full", changes);
    }
  };
  /** False after Stop — late TextDelta races must not repaint the preview. */
  const acceptStreamRef = useRef(true);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const stickToBottomRef = useRef(true);
  /** Last reader-set scroll offset — restored when a stow (trace detour) ends. */
  const lastScrollTopRef = useRef(0);
  /** Queue lives in a ref so drain never double-fires under StrictMode. */
  const queueRef = useRef<PendingSend[]>([]);
  const drainingRef = useRef(false);

  const applyQueue = (next: PendingSend[]) => {
    queueRef.current = next;
    setQueueState(next);
  };

  const providerWarning = useMemo(() => {
    if (demoMode) return null;
    if (providers.length === 0) return "尚未配置 AI Provider";
    const active = provider;
    if (!active) return "当前 Provider 不可用";
    if (!active.hasKey && !(active.apiKey && active.apiKey.length > 0)) {
      return "当前 Provider 缺少 API Key";
    }
    return null;
  }, [demoMode, providers, provider]);

  /** Does the currently-selected model accept image attachments? `undefined`
   *  means the catalog doesn't know (legacy / user-added id) — treat as
   *  opt-in and don't gate. `false` means definitely text-only and triggers
   *  the first-time skip notice. */
  const activeModelVision = useMemo<boolean | undefined>(() => {
    if (!provider) return undefined;
    const target = provider.modelId || provider.model;
    if (!target) return undefined;
    const match = provider.models?.find((m) => m.model_id === target);
    return match?.vision;
  }, [provider]);

  useEffect(() => {
    setApproval(null);
    setPageError(null);
    setContexts([]);
    setProbes({});
    setApprovalError(null);
    textBufRef.current = "";
    setProgress(null);
    setFailovers([]);
    setFailover(null);
    setBudget(null);
    setCompactToast(null);
    setBudgetLiftToast(null);
    if (compactToastTimerRef.current !== null) {
      window.clearTimeout(compactToastTimerRef.current);
      compactToastTimerRef.current = null;
    }
    if (budgetLiftTimerRef.current !== null) {
      window.clearTimeout(budgetLiftTimerRef.current);
      budgetLiftTimerRef.current = null;
    }
    applyQueue([]);
    setRunStartedAt(null);
    setElapsed(null);
    setUndoMessage(null);
    // The diff map belongs to one conversation — clear it here and upstream so
    // the sidebar panel never shows the previous thread's 变更.
    setFileDiffs(null);
    onFileDiffs?.({});
    acceptStreamRef.current = true;
    stickToBottomRef.current = true;
    if (createdConversationId && conversationId === createdConversationId) return;
    setCreatedConversationId(null);
    if (!conversationId || demoMode) {
      setTurns([]);
      setTitle("");
      setRunning(false);
      onSnapshot?.(null);
      return;
    }

    // Do not keep the previous conversation's live flag while this one loads.
    setRunning(false);
    setRunStartedAt(null);
    setApproval(null);
    setProgress(null);
    // A run can be live while the user is elsewhere. The log carries every
    // finished step; `run_state` adds the facts the log cannot: still running,
    // elapsed clock, latest phase, and a pending approval. Events arriving
    // during this window are buffered and replayed after the log lands so a
    // step finishing in the gap is never lost.
    const loading = { session: conversationId, events: [] as RunEventDto[] };
    loadingEventsRef.current = loading;
    let alive = true;
    void (async () => {
      const live = await runState(conversationId);
      const loaded = await loadSession(conversationId);
      if (!alive || !loaded) {
        if (loadingEventsRef.current === loading) loadingEventsRef.current = null;
        return;
      }
      setTurns(loaded.turns);
      setTitle(loaded.title);
      const last = loaded.turns.length > 0 ? loaded.turns[loaded.turns.length - 1] : null;
      const logFinished = Boolean(last && (last.done || last.stopped || last.error));
      // A run that already wrote its terminal envelope may still be registered
      // for a few milliseconds; the approval branch is the one case where a
      // finished-looking log still needs the live UI.
      if (live?.running && (!logFinished || live.approval)) {
        setRunning(true);
        setRunStartedAt(live.startedAt ?? Date.now());
        setApproval(
          live.approval
            ? {
                runId: live.approval.runId,
                step: live.approval.step,
                kind: live.approval.kind,
                detail: live.approval.detail,
                cwd: live.approval.cwd,
                riskCategory: live.approval.riskCategory,
                reason: live.approval.reason,
              }
            : null,
        );
        setProgress(
          live.approval ? { phase: "等待审批", detail: "请确认后继续执行" } : live.progress,
        );
      } else {
        setRunning(false);
        setRunStartedAt(null);
        setApproval(null);
      }
      if (loadingEventsRef.current === loading) {
        loadingEventsRef.current = null;
        for (const event of loading.events) handleEventRef.current(event);
      }
    })();
    return () => {
      alive = false;
      if (loadingEventsRef.current === loading) loadingEventsRef.current = null;
    };
  }, [conversationId, demoMode, onSnapshot, createdConversationId]);

  // Sent-image thumbnails: probes are not persisted with the turn, so a
  // reloaded session re-probes image attachments once (shared map; the
  // in-flight set dedupes concurrent messages). A failed probe degrades to
  // the extension guess and is stored so the effect never retries it.
  useEffect(() => {
    for (const turn of turns) {
      for (const path of turn.context ?? []) {
        if (probes[path] || probeInflight.has(path)) continue;
        if (kindFromPath(path) !== "image") continue;
        probeInflight.add(path);
        void probeContextFile(projectPath, path)
          .then((probe) => setProbes((current) => ({ ...current, [path]: probe })))
          .catch(() =>
            setProbes((current) => ({
              ...current,
              [path]: {
                kind: "image",
                name: path.split("/").pop() || path,
                size: 0,
                preview: null,
              },
            })),
          )
          .finally(() => probeInflight.delete(path));
      }
    }
  }, [turns, probes, projectPath]);

  const sessionId = conversationId ?? createdConversationId;
  /** Persona selector key: same slots as draftKey, so "new" pre-creation. */
  const sessionKey = sessionId ?? "new";
  const personaChain = personaListBySession[sessionKey] ?? [];
  // Send reads the chain at dispatch time (retry/queue included), so a
  // mid-session switch applies to the very next message.
  const personaChainRef = useRef<string[]>([]);
  personaChainRef.current = personaChain;

  // The first send creates the session id — move the draft-slot chain
  // over, otherwise the choice evaporates the moment the conversation lands.
  // The draft slot is cleared so the next 新建任务 starts at 无智能体.
  const migratedAgentsRef = useRef(false);
  useEffect(() => {
    if (!createdConversationId || migratedAgentsRef.current) return;
    migratedAgentsRef.current = true;
    const draftChoice = personaListBySession["new"];
    if (draftChoice && draftChoice.length > 0) {
      onSelectAgents(createdConversationId, draftChoice);
      onSelectAgents("new", null);
    }
  }, [createdConversationId, personaListBySession, onSelectAgents]);
  useEffect(() => {
    if (!createdConversationId) migratedAgentsRef.current = false;
  }, [createdConversationId]);

  /** Live sessions get conversation controls (rename/archive in the top bar). */
  const liveSession = !demoMode && sessionId !== null;

  const knowledgeProjectPath = projectPath || null;
  const knowledgeScopeKey = `${sessionId ?? "new"}\u0000${knowledgeProjectPath ?? "personal"}`;
  const [knowledgeBases, setKnowledgeBases] = useState<KnowledgeBase[]>([]);
  const [knowledgeSelection, setKnowledgeSelection] = useState<KnowledgeSelection>({
    baseIds: [],
    strict: true,
  });
  const [knowledgeLoading, setKnowledgeLoading] = useState(!demoMode);
  const [knowledgeSaving, setKnowledgeSaving] = useState(false);
  const [knowledgePreparing, setKnowledgePreparing] = useState(false);
  const [knowledgeError, setKnowledgeError] = useState<string | null>(null);
  const [knowledgeFallbackAvailable, setKnowledgeFallbackAvailable] = useState(false);
  const [knowledgeApproval, setKnowledgeApproval] = useState<PreparedTurn | null>(null);
  const [knowledgeReload, setKnowledgeReload] = useState(0);
  const knowledgeSelectionRef = useRef(knowledgeSelection);
  knowledgeSelectionRef.current = knowledgeSelection;
  const knowledgeScopeKeyRef = useRef(knowledgeScopeKey);
  knowledgeScopeKeyRef.current = knowledgeScopeKey;
  const knowledgeLoadedScopeRef = useRef("");
  const knowledgeGenerationRef = useRef(0);
  const knowledgeSendLockRef = useRef(false);
  const knowledgeSendLockGenerationRef = useRef(0);
  const knowledgeSavingRef = useRef(false);
  knowledgeSavingRef.current = knowledgeSaving;
  const pendingKnowledgeApprovalRef = useRef<{ resolve: (approved: boolean) => void } | null>(null);
  const applyKnowledgeSelection = (next: KnowledgeSelection) => {
    knowledgeSelectionRef.current = next;
    setKnowledgeSelection(next);
  };
  const finishKnowledgeApproval = (approved: boolean) => {
    const pending = pendingKnowledgeApprovalRef.current;
    if (!pending) return;
    pendingKnowledgeApprovalRef.current = null;
    setKnowledgeApproval(null);
    pending.resolve(approved);
  };

  useEffect(() => {
    const generation = ++knowledgeGenerationRef.current;
    let alive = true;
    knowledgeLoadedScopeRef.current = "";
    finishKnowledgeApproval(false);
    knowledgeSavingRef.current = false;
    applyKnowledgeSelection({ baseIds: [], strict: true });
    setKnowledgeBases([]);
    setKnowledgeError(null);
    setKnowledgeFallbackAvailable(false);
    setKnowledgeSaving(false);
    setKnowledgeLoading(!demoMode);

    if (demoMode) {
      setKnowledgeLoading(false);
      knowledgeLoadedScopeRef.current = knowledgeScopeKey;
      return () => {
        alive = false;
        knowledgeGenerationRef.current += 1;
        if (knowledgeSendLockGenerationRef.current === generation) {
          knowledgeSendLockRef.current = false;
          setKnowledgePreparing(false);
        }
        finishKnowledgeApproval(false);
      };
    }

    void (async () => {
      try {
        const listedBases = await knowledgeListBases({ projectPath: knowledgeProjectPath });
        const bases = Array.from(new Map(listedBases.map((base) => [base.id, base])).values());
        const persisted = sessionId
          ? await knowledgeGetBindings({ sessionId, projectPath: knowledgeProjectPath })
          : { baseIds: [], strict: true };
        if (!alive || knowledgeGenerationRef.current !== generation) return;
        const allowed = new Set(bases.map((base) => base.id));
        const cleanSelection: KnowledgeSelection = {
          baseIds: persisted.baseIds.filter((baseId) => allowed.has(baseId)),
          strict: persisted.strict ?? true,
        };
        setKnowledgeBases(bases);
        setKnowledgeFallbackAvailable(false);
        applyKnowledgeSelection(cleanSelection);
        if (cleanSelection.baseIds.length !== persisted.baseIds.length) {
          await knowledgeSetBindings({
            sessionId: sessionId!,
            projectPath: knowledgeProjectPath,
            selection: cleanSelection,
          });
          if (!alive || knowledgeGenerationRef.current !== generation) return;
          setKnowledgeError("部分已绑定知识库在当前范围不可用，已从本轮选择中清除。");
        }
        knowledgeLoadedScopeRef.current = knowledgeScopeKey;
      } catch (failure) {
        if (!alive || knowledgeGenerationRef.current !== generation) return;
        const code = errorMessage(failure);
        if (code === "app_disabled") {
          setKnowledgeError("知识库已停用，本轮不使用资料。");
          knowledgeLoadedScopeRef.current = knowledgeScopeKey;
          setKnowledgeFallbackAvailable(false);
        } else {
          setKnowledgeError(`知识库绑定读取失败：${knowledgeErrorMessage(failure)}`);
          setKnowledgeFallbackAvailable(true);
        }
        setKnowledgeBases([]);
        applyKnowledgeSelection({ baseIds: [], strict: true });
      } finally {
        if (alive && knowledgeGenerationRef.current === generation) setKnowledgeLoading(false);
      }
    })();

    return () => {
      alive = false;
      if (knowledgeGenerationRef.current === generation) knowledgeGenerationRef.current += 1;
      if (knowledgeSendLockGenerationRef.current === generation) {
        knowledgeSendLockRef.current = false;
        setKnowledgePreparing(false);
      }
      finishKnowledgeApproval(false);
    };
  }, [knowledgeScopeKey, demoMode, knowledgeReload]);

  useEffect(() => {
    if (!knowledgeApproval) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        finishKnowledgeApproval(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [knowledgeApproval]);

  // A create-send races its own run: the first answer can finish before
  // send_message returns the new session id. Keep one long-lived subscription,
  // match events through this ref, and buffer the race window for replay
  // (2026-09-23 — the first turn of a new conversation was lost before).
  const sessionIdRef = useRef<string | null>(null);
  sessionIdRef.current = sessionId;
  const createInFlightRef = useRef(false);
  const runGenerationsRef = useRef(new Map<string, number>());
  const racedEventsRef = useRef<RunEventDto[]>([]);
  /** Run events for the conversation currently being loaded. Replayed after
   *  its log snapshot is applied, so a step that finishes mid-load survives. */
  const loadingEventsRef = useRef<{ session: string; events: RunEventDto[] } | null>(null);
  const handleEventRef = useRef<(event: RunEventDto) => void>(() => {});
  const stopRequestedRef = useRef(false);

  // Elapsed clock for the live turn status line.
  useEffect(() => {
    if (runStartedAt === null || !running) {
      setElapsed(null);
      return;
    }
    const tick = () => setElapsed(Math.max(0, Math.floor((Date.now() - runStartedAt) / 1000)));
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [runStartedAt, running]);

  const adoptAskTitle = (ask: string, id: string | null) => {
    if (!id || demoMode) return;
    if (title && title !== "新对话") return;
    const next = titleFromAsk(ask);
    if (!next) return;
    setTitle(next);
    void Promise.resolve(onRetitle(id, next)).catch(() => {
      /* title is cosmetic; keep the turn usable if retitle fails */
    });
  };

  const persistKnowledgeSelection = async (next: KnowledgeSelection) => {
    const scopeKey = knowledgeScopeKeyRef.current;
    if (
      demoMode ||
      knowledgeSendLockRef.current ||
      knowledgeSavingRef.current ||
      knowledgeLoadedScopeRef.current !== scopeKey
    ) return;
    const previous = knowledgeSelectionRef.current;
    if (sessionId) {
      knowledgeSavingRef.current = true;
      setKnowledgeSaving(true);
      setKnowledgeError(null);
      try {
        await knowledgeSetBindings({
          sessionId,
          projectPath: knowledgeProjectPath,
          selection: next,
        });
        if (knowledgeScopeKeyRef.current !== scopeKey) return;
        applyKnowledgeSelection(next);
      } catch (failure) {
        if (knowledgeScopeKeyRef.current === scopeKey) {
          setKnowledgeError(`知识库绑定保存失败：${knowledgeErrorMessage(failure)}`);
          applyKnowledgeSelection(previous);
        }
      } finally {
        if (knowledgeScopeKeyRef.current === scopeKey) {
          knowledgeSavingRef.current = false;
          setKnowledgeSaving(false);
        }
      }
      return;
    }
    applyKnowledgeSelection(next);
    setKnowledgeError(null);
  };

  const toggleKnowledgeBase = (baseId: string, checked: boolean) => {
    const current = knowledgeSelectionRef.current;
    const selected = new Set(current.baseIds);
    if (checked) selected.add(baseId);
    else selected.delete(baseId);
    void persistKnowledgeSelection({ ...current, baseIds: Array.from(selected) });
  };

  const changeKnowledgeStrict = (strict: boolean) => {
    void persistKnowledgeSelection({ ...knowledgeSelectionRef.current, strict });
  };

  const continueWithoutKnowledge = () => {
    const scopeKey = knowledgeScopeKeyRef.current;
    applyKnowledgeSelection({ baseIds: [], strict: true });
    setKnowledgeBases([]);
    setKnowledgeError(null);
    setKnowledgeFallbackAvailable(false);
    setKnowledgeLoading(false);
    knowledgeLoadedScopeRef.current = scopeKey;
  };

  const dispatchSend = async (text: string, context: string[], opts?: { skipTitle?: boolean; budgetFamily?: string | null }) => {
    if (knowledgeSendLockRef.current) return;
    knowledgeSendLockRef.current = true;
    const payload = { text, context };
    const targetId = sessionId ?? "";
    const scopeKey = knowledgeScopeKeyRef.current;
    const generation = knowledgeGenerationRef.current;
    knowledgeSendLockGenerationRef.current = generation;
    let prepared: PreparedTurn | null = null;
    let remoteApproved = false;
    const restoreDraft = () => setEditSeed({ text, nonce: Date.now() });

    try {
      try {
        if (!demoMode) {
          if (
            knowledgeLoading ||
            knowledgeSavingRef.current ||
            knowledgeLoadedScopeRef.current !== scopeKey
          ) {
            throw new Error("knowledge_binding_unavailable");
          }
          const selection = knowledgeSelectionRef.current;
          if (selection.baseIds.length > 0) {
            setKnowledgePreparing(true);
            prepared = await knowledgePrepareTurn({
              sessionId: targetId,
              projectPath: knowledgeProjectPath,
              selection,
              text,
            });
            if (
              knowledgeGenerationRef.current !== generation ||
              knowledgeScopeKeyRef.current !== scopeKey
            ) return;
            if (prepared.remote) {
              const approved = await new Promise<boolean>((resolve) => {
                pendingKnowledgeApprovalRef.current = { resolve };
                setKnowledgeApproval(prepared);
              });
              if (
                knowledgeGenerationRef.current !== generation ||
                knowledgeScopeKeyRef.current !== scopeKey
              ) return;
              if (!approved) {
                restoreDraft();
                return;
              }
              remoteApproved = true;
            }
          }
        }
      } catch (failure) {
        if (
          knowledgeGenerationRef.current === generation &&
          knowledgeScopeKeyRef.current === scopeKey
        ) {
          restoreDraft();
          setPageError({
            message: `资料准备失败：${knowledgeErrorMessage(failure)}`,
            retryLabel: "重新准备",
            retry: () => {
              setPageError(null);
              void dispatchSend(text, context, { ...opts, skipTitle: true });
            },
          });
        }
        return;
      } finally {
        if (knowledgeGenerationRef.current === generation) setKnowledgePreparing(false);
      }

      if (
        knowledgeGenerationRef.current !== generation ||
        knowledgeScopeKeyRef.current !== scopeKey
      ) return;

    setPageError(null);
    setUndoMessage(null);
    setTurns((current) => [...current, blank(text, context, Boolean(prepared?.token))]);
    setRunning(true);
    setRunStartedAt(Date.now());
    acceptStreamRef.current = true;
    textBufRef.current = "";
    setProgress(null);
    setBudget(null);
    setCompactToast(null);
    setBudgetLiftToast(null);
    if (compactToastTimerRef.current !== null) {
      window.clearTimeout(compactToastTimerRef.current);
      compactToastTimerRef.current = null;
    }
    if (budgetLiftTimerRef.current !== null) {
      window.clearTimeout(budgetLiftTimerRef.current);
      budgetLiftTimerRef.current = null;
    }
    stickToBottomRef.current = true;
    stopRequestedRef.current = false;
    // Existing sessions adopt the title right away; a create-send adopts once
    // its id exists.
    if (!opts?.skipTitle && targetId) adoptAskTitle(text, targetId);
    if (!targetId) {
      // send_message will create the session — hold events that race the id.
      createInFlightRef.current = true;
      racedEventsRef.current = [];
    }
    try {
      const personaChain = personaChainRef.current;
      const createdId = await sendMessage(
        targetId,
        text,
        context,
        projectPath,
        personaChain,
        opts?.budgetFamily ?? null,
        prepared?.token,
        prepared ? remoteApproved : undefined,
      );
      // Stamp the chain's first entry as just-used; recents are an MRU,
      // so the next send will move it back to the top automatically.
      if (personaChain.length > 0) recordRecentAgent(personaChain[0]);
      if (!targetId && createdId) {
        sessionIdRef.current = createdId;
        createInFlightRef.current = false;
        setCreatedConversationId(createdId);
        onConversationCreated?.(createdId);
        if (!opts?.skipTitle) adoptAskTitle(text, createdId);
        // Replay the run events that arrived before the id was known — for a
        // fast conversational answer this is the entire turn.
        const raced = racedEventsRef.current;
        racedEventsRef.current = [];
        for (const event of raced) {
          if (event.session === createdId) handleEventRef.current(event);
        }
        if (stopRequestedRef.current) {
          stopRequestedRef.current = false;
          void stopRun(createdId).catch((failure) => {
            setApprovalError(`停止失败：${errorMessage(failure)}`);
          });
        }
      }
      if (!createdId && !targetId) throw new Error("创建任务失败");
      await onConversationCommitted?.();
      // Context is consumed for this turn only; the user re-pins if needed.
      setContexts([]);
    } catch (failure) {
      createInFlightRef.current = false;
      setRunning(false);
      setRunStartedAt(null);
      const message = errorMessage(failure);
      setTurns((current) => updateLast(current, (turn) => ({ ...turn, error: message })));
      setPageError({
        message: `发送失败：${message}`,
        retryLabel: "重试发送",
        retry: () => {
          setPageError(null);
          void dispatchSend(payload.text, payload.context, { skipTitle: true });
        },
      });
    }
    } finally {
      if (knowledgeGenerationRef.current === generation) setKnowledgePreparing(false);
      if (knowledgeSendLockGenerationRef.current === generation) {
        knowledgeSendLockRef.current = false;
      }
    }
  };

  const drainQueue = () => {
    if (drainingRef.current) return;
    const pending = queueRef.current;
    if (pending.length === 0) return;
    drainingRef.current = true;
    const [next, ...rest] = pending;
    applyQueue(rest);
    void dispatchSend(next.text, next.context, { skipTitle: true }).finally(() => {
      drainingRef.current = false;
    });
  };

  const send = (text: string, context: string[]) => {
    if (knowledgeSendLockRef.current || knowledgeSavingRef.current) return;
    // Running: queue the follow-up instead of dropping the draft (codex/DSH).
    if (running) {
      applyQueue([...queueRef.current, { text, context }]);
      return;
    }
    // Vision-incapable model + image attached: surface a one-time notice
    // before sending. The message is parked in `pendingVisionSend` and
    // dispatched only after the user acks. `vision === undefined` means
    // the catalog doesn't know (legacy entry) — treat as opt-in (no gate).
    if (
      !visionSkipAcked &&
      activeModelVision === false &&
      context.some((path) => kindFromPath(path) === "image")
    ) {
      setPendingVisionSend({ text, context });
      return;
    }
    void dispatchSend(text, context);
  };

  /** Acknowledge the vision-skip notice: persist the flag, dismiss the
   *  banner, and dispatch the held message if there is one. Idempotent —
   *  calling with no pending send still flips the session flag. */
  const ackVisionSkip = () => {
    if (typeof window !== "undefined") {
      window.sessionStorage.setItem("kodo:vision-img-skip-acked", "1");
    }
    setVisionSkipAcked(true);
    const held = pendingVisionSend;
    setPendingVisionSend(null);
    if (held) void dispatchSend(held.text, held.context);
  };

  /** Cancel the pending send without acking; the user keeps the draft and
   *  can re-trigger send (e.g. by switching model). The Composer cleared
   *  its draft on submit, so re-seed it via `editSeed` to restore the
   *  typed text — the attachment chip already stays put (page-level
   *  state). */
  const dismissVisionSkip = () => {
    const held = pendingVisionSend;
    setPendingVisionSend(null);
    if (held) setEditSeed({ text: held.text, nonce: Date.now() });
  };

  const regenerate = () => {
    if (!sessionId || running) return;
    const last = [...turns].reverse().find((turn) => turn.ask.trim());
    if (!last) return;
    void dispatchSend(last.ask, last.context ?? [], { skipTitle: true });
  };

  /** Retry unfinished work with extended headroom, preserving task policy,
   *  existing files and public conversation history. This starts a new run;
   *  the backend must re-inspect files rather than trusting a failed claim. */
  const continueTurn = () => {
    if (!sessionId || running) return;
    const last = [...turns].reverse().find((turn) => turn.ask.trim());
    if (!last) return;
    void dispatchSend(last.ask, last.context ?? [], { skipTitle: true, budgetFamily: "extended" });
  };

  const stop = () => {
    setApproval(null);
    // Stop 后不再产生用户可见 TextDelta。
    acceptStreamRef.current = false;
    textBufRef.current = "";
    setProgress(null);
    setBudget(null);
    setCompactToast(null);
    if (compactToastTimerRef.current !== null) {
      window.clearTimeout(compactToastTimerRef.current);
      compactToastTimerRef.current = null;
    }
    setRunStartedAt(null);
    const current = sessionIdRef.current;
    if (!current) {
      // First send still creating the session — apply stop once the id lands.
      if (createInFlightRef.current) stopRequestedRef.current = true;
      return;
    }
    void stopRun(current).catch((failure) => {
      setApprovalError(`停止失败：${errorMessage(failure)}`);
    });
  };

  /** Undo only this turn's Kodo edits; conflicts keep the user's later work. */
  const undoChanges = () => {
    if (!sessionId || !projectPath || undoingChanges || running) return;
    setPageError(null);
    setUndoingChanges(true);
    void undoTurn(projectPath, sessionId)
      .then((report) => {
        if (!report) throw new Error("未得到撤销报告");
        // The report is the truth: say what was restored, what conflicted, or
        // that there was nothing — never a blanket "已撤销".
        const parts: string[] = [];
        if (report.restored.length > 0) parts.push(T.files.undoDone);
        if (report.conflicts.length > 0) {
          parts.push(
            `${T.files.conflictCount(report.conflicts.length)}：${report.conflicts
              .map((conflict) => conflict.path)
              .join("、")}`,
          );
        }
        setUndoMessage(parts.length > 0 ? parts.join(" ") : T.files.undoNothing);
        return loadSession(sessionId).then((loaded) => {
          if (loaded) setTurns(loaded.turns);
        });
      })
      .catch((failure: unknown) => {
        setPageError({ message: `撤销失败：${errorMessage(failure)}` });
      })
      .finally(() => setUndoingChanges(false));
  };

  const decide = (approved: boolean, sessionWide = false) => {
    if (!sessionId || !approval) return;
    const pending = approval;
    setApproval(null);
    setApprovalError(null);
    void respondApproval(sessionId, pending.step, approved, sessionWide, pending.runId).catch((failure) => {
      const label = !approved ? "拒绝" : sessionWide ? "本会话允许" : "允许";
      setApprovalError(
        `审批响应失败：${errorMessage(failure)}（${label} step ${pending.step}）`,
      );
    });
  };

  const addContext = (path: string) => {
    setContexts((current) => (current.includes(path) ? current : [...current, path]));
  };


  const rename = (next: string) => {
    if (!sessionId) return;
    const previous = title;
    setTitle(next);
    setPageError(null);
    void Promise.resolve(onRetitle(sessionId, next)).catch((failure) => {
      setTitle(previous);
      setPageError({
        message: `重命名失败：${errorMessage(failure)}`,
        retryLabel: "重试重命名",
        retry: () => rename(next),
      });
    });
  };

  const archive = (id: string) => {
    setPageError(null);
    void Promise.resolve(onArchive(id)).catch((failure) => {
      setPageError({
        message: `归档失败：${errorMessage(failure)}`,
        retryLabel: "重试归档",
        retry: () => archive(id),
      });
    });
  };

  // The top bar hosts rename/archive since the content title header is gone
  // (2026-09-30): this page keeps the handlers — and their failure banners —
  // and publishes stable entry points for the top bar to call.
  const renameRef = useRef(rename);
  const archiveRef = useRef(archive);
  useEffect(() => {
    renameRef.current = rename;
    archiveRef.current = archive;
  });
  useEffect(() => {
    if (!liveSession || !sessionId) {
      onConvActions?.(null);
      return;
    }
    onConvActions?.({
      rename: (next) => renameRef.current(next),
      archive: () => archiveRef.current(sessionId),
    });
    return () => onConvActions?.(null);
  }, [liveSession, sessionId, onConvActions]);

  // Legacy streams flush only at tool boundaries; bookkeeping and retries are not public updates.
  const flushInterim = (item: ItemDto) => {
    if (item.kind === "modelCall" || item.kind === "agentMessage") return;
    if (item.kind === "reasoning") {
      if (item.phase === "commentary") textBufRef.current = "";
      return;
    }
    const text = textBufRef.current.trim();
    textBufRef.current = "";
    if (!text) return;
    const note: InterimNote = { id: (noteSeqRef.current += 1), text, beforeId: item.id };
    setTurns((current) =>
      updateLast(current, (turn) => ({ ...turn, notes: [...(turn.notes ?? []), note] })),
    );
  };

  const handleEvent = (event: RunEventDto) => {
    if (event.runId !== undefined) {
      const current = runGenerationsRef.current.get(event.session);
      if (event.type === "turnStarted") {
        if (current !== undefined && event.runId < current) return;
        runGenerationsRef.current.set(event.session, event.runId);
      } else if (current !== undefined && event.runId !== current) return;
      else runGenerationsRef.current.set(event.session, event.runId);
    }
    if (event.type === "memoryRecalled") {
      onMemoryRecalled?.(event.count);
      return;
    }
    if (event.type === "approvalRequest") {
      setApproval({
        runId: event.runId,
        step: event.step,
        kind: event.kind,
        detail: event.detail ?? "",
        cwd: event.cwd,
        riskCategory: event.riskCategory,
        reason: event.reason,
      });
      setProgress({ phase: "等待审批", detail: "请确认后继续执行" });
      return;
    }
    if (event.type === "textDelta") {
      if (acceptStreamRef.current) {
        textBufRef.current += event.text;
      }
      return;
    }
    if (event.type === "progress") {
      setProgress({ phase: event.phase, detail: event.detail });
      // Adaptive budget lift rides the same `progress` channel; surface it
      // as a dedicated toast (auto-clears) so the user can see the cap
      // moved, not just the running-status line.
      if (event.phase === "budget-lift") {
        setBudgetLiftToast({ detail: event.detail });
        if (budgetLiftTimerRef.current !== null) {
          window.clearTimeout(budgetLiftTimerRef.current);
        }
        budgetLiftTimerRef.current = window.setTimeout(() => {
          setBudgetLiftToast(null);
          budgetLiftTimerRef.current = null;
        }, 1500);
      }
      return;
    }
    if (event.type === "contextBudget") {
      setBudget({
        used: event.used,
        window: event.window,
        percent: event.percent,
        cumulativeChars: event.cumulativeChars,
        compactedCount: event.compactedCount,
        lastModel: event.lastModel,
      });
      return;
    }
    if (event.type === "compacted") {
      setCompactToast({ dropped: event.dropped, freedPct: event.freedPct });
      if (compactToastTimerRef.current !== null) {
        window.clearTimeout(compactToastTimerRef.current);
      }
      compactToastTimerRef.current = window.setTimeout(() => {
        setCompactToast(null);
        compactToastTimerRef.current = null;
      }, 1500);
      return;
    }
    if (event.type === "failover" || event.type === "providerSwitch") {
      // A failed candidate's partial stream is discarded — the next one replays.
      textBufRef.current = "";
      const notice = {
        fromProvider: event.fromProvider,
        fromModel: event.fromModel,
        errorClass: event.errorClass,
        error: event.error,
        toProvider: event.toProvider,
        toModel: event.toModel,
      };
      setFailover(notice);
      setFailovers((current) => [
        ...current,
        `已从 ${event.fromProvider}（${event.fromModel}）切换到 ${event.toProvider}（${event.toModel}）· ${event.errorClass}`,
      ]);
      return;
    }
    if (event.type === "itemStarted" || event.type === "itemCompleted") {
      if (event.type === "itemStarted" && event.item.kind === "modelCall") textBufRef.current = "";
      flushInterim(event.item);
    }
    if (event.type === "itemCompleted" && event.item.kind === "agentMessage") {
      // The trailing buffer is the draft of this answer — the answer replaces it.
      textBufRef.current = "";
    }
    if (event.type === "turnStarted") {
      acceptStreamRef.current = true;
      textBufRef.current = "";
      setProgress(null);
      setFailovers([]);
      setFailover(null);
      setBudget(null);
      setCompactToast(null);
      // A new turn starts: until the next memoryRecalled event, the chip
      // should show "engine on, nothing yet" rather than the prior turn's
      // count. `null` = "no event yet this turn".
      onMemoryRecalled?.(0);
      setBudgetLiftToast(null);
      if (compactToastTimerRef.current !== null) {
        window.clearTimeout(compactToastTimerRef.current);
        compactToastTimerRef.current = null;
      }
      if (budgetLiftTimerRef.current !== null) {
        window.clearTimeout(budgetLiftTimerRef.current);
        budgetLiftTimerRef.current = null;
      }
      setRunStartedAt((current) => current ?? Date.now());
    }
    setTurns((current) => reduce(current, event));
    if (event.type === "turnComplete" || event.type === "stopped" || event.type === "error") {
      acceptStreamRef.current = false;
      setRunning(false);
      setApproval(null);
      textBufRef.current = "";
      setProgress(null);
      setBudget(null);
      setCompactToast(null);
      setBudgetLiftToast(null);
      if (compactToastTimerRef.current !== null) {
        window.clearTimeout(compactToastTimerRef.current);
        compactToastTimerRef.current = null;
      }
      if (budgetLiftTimerRef.current !== null) {
        window.clearTimeout(budgetLiftTimerRef.current);
        budgetLiftTimerRef.current = null;
      }
      setRunStartedAt(null);
      if (event.type === "turnComplete") {
        drainQueue();
        // Completion ping: Rust gates the OS attention request on focus and
        // owns the native title marker (webview titles never sync back).
        void taskFeedback();
      }
      if (event.type === "error") void taskFeedback();
    }
  };
  // The subscription captures this ref, so a single long-lived listener always
  // applies the latest handler without being torn down on every id change.
  handleEventRef.current = handleEvent;

  useEffect(() => {
    if (demoMode) return;

    let alive = true;
    let stop: (() => void) | null = null;

    void onRunEvent((event) => {
      const loading = loadingEventsRef.current;
      if (loading && event.session === loading.session) {
        loading.events.push(event);
        if (loading.events.length > 500) loading.events.shift();
        return;
      }
      const current = sessionIdRef.current;
      if (current && event.session === current) {
        handleEventRef.current(event);
        return;
      }
      // Create-send in flight: the answer can land before its session id is
      // known. Hold those events until the id resolves, then replay them.
      if (createInFlightRef.current) {
        racedEventsRef.current.push(event);
        if (racedEventsRef.current.length > 500) racedEventsRef.current.shift();
      }
    }).then((off) => {
      if (alive) stop = off;
      else off();
    });

    return () => {
      alive = false;
      void Promise.resolve(stop?.()).catch(() => {
        // The Tauri event bridge can finish registering after the view has
        // already moved to the newly-created session. Cleanup is best effort.
      });
    };
  }, [demoMode]);

  // Follow the live turn: stick to bottom only while the reader is already there.
  useEffect(() => {
    if (demoMode) return;
    const el = scrollRef.current;
    if (!el || !stickToBottomRef.current) return;
    el.scrollTop = el.scrollHeight;
  }, [demoMode, turns, progress, queue, running]);

  // Stowing (Response Trace over a live run) hides this pane via display:none,
  // which drops the layout box — on return, put the reader back where they
  // were: the bottom if they were following the run, their own position if not.
  useEffect(() => {
    if (stowed) return;
    const el = scrollRef.current;
    if (!el) return;
    // The demo is a frozen recording presented from turn 1 (DEMO_DATA: the
    // Inspector and Trace pages describe turns[0]), so it opens at the top.
    el.scrollTop = demoMode
      ? lastScrollTopRef.current
      : stickToBottomRef.current
        ? el.scrollHeight
        : lastScrollTopRef.current;
  }, [stowed, demoMode]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    stickToBottomRef.current = distance < 48;
    lastScrollTopRef.current = el.scrollTop;
  };

  // Load per-file unified diffs for Edit steps once a turn recorded changes.
  // The same payload carries which paths the user had already modified and
  // where undo would conflict — the reply card must not blur those together.
  useEffect(() => {
    if (demoMode || !conversationId || !projectPath || running) return;
    const hasEdit = turns.some((turn) => turn.items.some((item) => item.kind === "fileChange"));
    if (!hasEdit) return;
    let alive = true;
    void turnChanges(projectPath, conversationId)
      .then((list) => {
        if (!alive || !list) return;
        const map: Record<string, string> = {};
        const preExisting: string[] = [];
        const conflicts: string[] = [];
        for (const change of list) {
          if (change.diff) map[change.path] = change.diff;
          if (change.userPreexisting) preExisting.push(change.path);
          if (change.conflict) conflicts.push(change.path);
        }
        setFileDiffs(map);
        onFileDiffs?.(map);
        setTurnMeta({ preExisting, conflicts });
      })
      .catch(() => {
        /* diffs are progressive disclosure; the path list still stands */
      });
    return () => {
      alive = false;
    };
  }, [demoMode, conversationId, projectPath, turns, running]);

  const liveSnapshot = useMemo(() => {
    if (demoMode || !sessionId) return null;
    const last = turns[turns.length - 1] ?? null;
    return snapshotFromTurn(sessionId, title || "新对话", projectName, projectPath, last, running, {
      onUndo: undoChanges,
      undoing: undoingChanges,
      onRetry: regenerate,
    });
  }, [demoMode, sessionId, turns, title, running, projectName, projectPath, undoingChanges]);

  useEffect(() => {
    onSnapshot?.(demoMode || !sessionId ? null : liveSnapshot);
  }, [onSnapshot, liveSnapshot, demoMode, sessionId]);

  // ⌘F find: bounded scan over the in-memory turns (ask + final reply text).
  const findHits = useMemo(() => {
    const query = find.query.trim().toLowerCase();
    if (!query) return [];
    const hits: Array<{ turn: number; part: "ask" | "reply" }> = [];
    turns.forEach((turn, index) => {
      if (turn.ask.toLowerCase().includes(query)) hits.push({ turn: index, part: "ask" });
      const reply = toReply(turn, false, false);
      if (reply.final?.toLowerCase().includes(query)) hits.push({ turn: index, part: "reply" });
    });
    return hits;
  }, [turns, find.query]);

  const findIndex = findHits.length
    ? Math.min(Math.max(find.index, 0), findHits.length - 1)
    : -1;
  const activeFindHit = find.open && findIndex >= 0 ? findHits[findIndex] : null;
  // Keyed by turn+part so streaming `turns` updates don't re-center the scroll.
  const activeFindKey = activeFindHit ? `${activeFindHit.turn}:${activeFindHit.part}` : null;

  useEffect(() => {
    if (activeFindKey === null) return;
    const [turnText, part] = activeFindKey.split(":");
    const root = scrollRef.current;
    if (!root) return;
    const nodes = root.querySelectorAll(part === "ask" ? ".msg-user" : ".reply");
    const node = nodes[Number(turnText)];
    node?.scrollIntoView({ block: "center" });
  }, [activeFindKey]);

  // ⌘F opens the widget — the browser's own find never fits a conversation.
  // Stowed (on the trace route) the keys belong to that page, not us.
  useEffect(() => {
    if (stowed) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        setFind((current) => ({ ...current, open: true }));
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [stowed]);

  // A different conversation starts with a closed, empty find.
  useEffect(() => {
    setFind({ open: false, query: "", index: 0 });
  }, [conversationId]);

  const demoTurns = useMemo(() => (demo ? demo.conversation.turns : []), [demo]);

  // TurnRail data: prompts + response excerpts, one per turn, both routes.
  // Memoized on the raw sources — the rail is memoized and must not re-render
  // on every streaming delta that leaves `turns` structurally unchanged.
  const railItems = useMemo<TurnRailItem[]>(() => {
    if (demoMode) {
      return demoTurns.map((turn, index) => ({
        index,
        prompt: turn.user.content,
        response: turn.assistant.final,
      }));
    }
    return turns.map((turn, index) => ({
      index,
      prompt: turn.ask,
      response: toReply(turn, false, false).final ?? "",
    }));
  }, [demoMode, demoTurns, turns]);

  // Active tick tracks the reading band (Markdown TOC precedent: light the
  // topmost turn whose top has crossed the 70% line). Between turns the last
  // one stays lit; rows without data-turn-index (queue/approval) never win.
  const [activeTurn, setActiveTurn] = useState<number | null>(0);
  useEffect(() => {
    const root = scrollRef.current;
    if (!root || typeof IntersectionObserver === "undefined") return;
    const visible = new Set<number>();
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const index = Number((entry.target as HTMLElement).dataset.turnIndex);
          if (entry.isIntersecting) visible.add(index);
          else visible.delete(index);
        }
        if (visible.size > 0) setActiveTurn(Math.min(...visible));
      },
      { root, rootMargin: "0px 0px -70% 0px" },
    );
    root.querySelectorAll<HTMLElement>("[data-turn-index]").forEach((el) => observer.observe(el));
    return () => observer.disconnect();
  }, [railItems]);

  // Tick click: center the turn. The first scroll event flips the 48px
  // stick-to-bottom flag, so jumping up releases follow for free; jumping to
  // the last tick keeps it. During a live run the follow effect may win one
  // frame — benign, the flag settles it.
  const navigateToTurn = (index: number) => {
    const node = scrollRef.current?.querySelector<HTMLElement>(
      `[data-turn-index="${String(index)}"]`,
    );
    const reduced =
      typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
    node?.scrollIntoView({ block: "center", behavior: reduced ? "auto" : "smooth" });
  };
  // Centered first-run stage: empty live conversation, no demo payload, no approval bar.
  const showWelcome = !demoMode && turns.length === 0 && !approval && !pageError && queue.length === 0;
  // The newest thread in this agent space — the welcome CTA back into it.
  // `projects` is already mode-filtered and non-archived (useWorkspace).
  const continueLast = useMemo(() => {
    let best: { id: string; at: number } | null = null;
    for (const project of projects) {
      for (const conversation of project.conversations) {
        if (conversation.id === conversationId) continue;
        const at = conversation.at ?? 0;
        if (!best || at > best.at) best = { id: conversation.id, at };
      }
    }
    return best;
  }, [projects, conversationId]);

  const knowledgeScopeReady = knowledgeLoadedScopeRef.current === knowledgeScopeKey;
  const knowledgeUiBusy =
    knowledgeLoading ||
    knowledgeSaving ||
    knowledgePreparing ||
    Boolean(knowledgeApproval) ||
    (!demoMode && !knowledgeScopeReady);
  const composer = (
    <>
      {!demoMode && (
        <KnowledgeBinding
          bases={knowledgeBases}
          selection={knowledgeSelection}
          loading={knowledgeLoading}
          busy={knowledgeSaving || knowledgePreparing || Boolean(knowledgeApproval)}
          error={knowledgeError}
          onContinueWithoutKnowledge={knowledgeFallbackAvailable ? continueWithoutKnowledge : undefined}
          projectName={projectName}
          approval={knowledgeApproval}
          onToggleBase={toggleKnowledgeBase}
          onStrictChange={changeKnowledgeStrict}
          onRetry={() => setKnowledgeReload((value) => value + 1)}
          onApprove={() => finishKnowledgeApproval(true)}
          onCancelApproval={() => finishKnowledgeApproval(false)}
        />
      )}
      <Composer
        provider={provider}
        providers={providers}
        defaultModel={defaultModel}
        onSelectProvider={onSelectProvider}
        onSelectProviderModel={onSelectProviderModel}
        agentMode={agentMode}
        ready={!demoMode && !knowledgeUiBusy}
        running={running}
        projectPath={projectPath}
        contexts={contexts}
        probes={probes}
        onProbed={(path, probe) => setProbes((current) => ({ ...current, [path]: probe }))}
        queueCount={queue.length}
        onAddContext={addContext}
        onRemoveContext={(path) => setContexts((current) => current.filter((item) => item !== path))}
        onContextError={(message) =>
          setPageError({
            message,
            action: {
              label: "打开设置",
              run: () => openSettings(),
            },
          })
        }
        onSend={(text, context) => send(text, context)}
        onStop={stop}
        providerWarning={providerWarning}
        onOpenProviderSettings={openSettings}
        draftKey={`kodo:draft:${sessionId ?? "new"}:${projectPath ?? "-"}`}
        editSeed={editSeed}
        personaChain={personaChain}
        onSelectAgents={(names) => onSelectAgents(sessionKey, names)}
        favoriteAgents={favoriteAgents}
        recentAgents={recentAgents}
        contextBudget={running ? budget : null}
        agentsVersion={agentsVersion}
      />
    </>
  );

  return (
    <main
      className={`${showWelcome ? "main main--welcome" : "main main--conversation"}${
        stowed ? " main--stowed" : ""
      }`}
    >
      {find.open && (
        <FindBar
          query={find.query}
          onQuery={(value) => setFind((current) => ({ ...current, query: value, index: 0 }))}
          index={findIndex}
          total={findHits.length}
          onPrev={() =>
            setFind((current) => ({
              ...current,
              index: findHits.length
                ? (current.index - 1 + findHits.length) % findHits.length
                : 0,
            }))
          }
          onNext={() =>
            setFind((current) => ({
              ...current,
              index: findHits.length ? (current.index + 1) % findHits.length : 0,
            }))
          }
          onClose={() => setFind((current) => ({ ...current, open: false }))}
        />
      )}
      <div className="conv-stage">
        <div className="scroll" ref={scrollRef} onScroll={onScroll} data-testid="conversation-scroll">
          <div className="page-inner">
            {pageError && (
              <div className="action-error" role="alert" data-testid="action-error">
                <span className="action-error-msg">{pageError.message}</span>
                <div className="action-error-actions">
                  {pageError.retry && (
                    <button
                      type="button"
                      className="btn btn--primary btn--sm"
                      data-testid="action-error-retry"
                      onClick={pageError.retry}
                    >
                      {pageError.retryLabel ?? "重试"}
                    </button>
                  )}
                  <button
                    type="button"
                    className="btn btn--sm"
                    data-testid="action-error-dismiss"
                    onClick={() => setPageError(null)}
                  >
                    关闭
                  </button>
                </div>
              </div>
            )}

            {failover && (
              <div className="action-error" role="status" data-testid="failover-notice">
                <span className="action-error-msg">
                  Failover · {failover.fromProvider}/{failover.fromModel} → {failover.toProvider}/
                  {failover.toModel} · {failover.errorClass}
                  {failover.error ? ` — ${failover.error.slice(0, 160)}` : ""}
                </span>
                <div className="action-error-actions">
                  <button
                    type="button"
                    className="btn btn--sm"
                    data-testid="failover-dismiss"
                    onClick={() => setFailover(null)}
                  >
                    关闭
                  </button>
                </div>
              </div>
            )}

            {showWelcome ? (
              <div className="welcome-stage" data-testid="welcome">
                <div className="welcome-copy">
                  {/* Space identity (DESIGN §11): the mode logo + the
                     `Kodo Code` / `Kodo Work` wordmark sits above the title
                     so a fresh 新建任务 is unambiguous before the first
                     message. `data-mode` powers the test and any future
                     per-space chrome tweaks. */}
                  <div
                    className="welcome-identity"
                    data-testid="welcome-identity"
                    data-mode={agentMode}
                  >
                    <img
                      className="welcome-identity-mark"
                      src={agentMode === "work" ? kodoWorkMark : kodoCodeMark}
                      alt=""
                      width={22}
                      height={22}
                    />
                    <span className="welcome-identity-name">
                      {agentMode === "work" ? T.nav.brandWork : T.nav.brandCode}
                    </span>
                  </div>
                  <h2 className="welcome-title">{T.welcome.title}</h2>
                  <p className="welcome-sub empty-note" data-testid="welcome-sub">
                    {agentMode === "work"
                      ? T.welcome.subtitleWork
                      : projectPath
                        ? T.welcome.subtitleCode
                        : T.welcome.subtitleCodeNoProject}
                  </p>
                  <div className="welcome-tasks" data-testid="welcome-tasks">
                    <button
                      type="button"
                      className="welcome-task"
                      data-testid="welcome-task-understand"
                      onClick={() =>
                        window.dispatchEvent(
                          new CustomEvent("kodo:focus-composer", {
                            detail: T.welcome.taskUnderstandPrompt,
                          }),
                        )
                      }
                    >
                      <BookOpen size={14} strokeWidth={1.8} className="welcome-task-icon" />
                      <span className="welcome-task-label">{T.welcome.taskUnderstand}</span>
                    </button>
                    <button
                      type="button"
                      className="welcome-task"
                      data-testid="welcome-task-implement"
                      onClick={() =>
                        window.dispatchEvent(
                          new CustomEvent("kodo:focus-composer", {
                            detail: T.welcome.taskImplementPrompt,
                          }),
                        )
                      }
                    >
                      <Sparkles size={14} strokeWidth={1.8} className="welcome-task-icon" />
                      <span className="welcome-task-label">{T.welcome.taskImplement}</span>
                    </button>
                    <button
                      type="button"
                      className="welcome-task"
                      data-testid="welcome-task-fix"
                      onClick={() =>
                        window.dispatchEvent(
                          new CustomEvent("kodo:focus-composer", {
                            detail: T.welcome.taskFixPrompt,
                          }),
                        )
                      }
                    >
                      <Wrench size={14} strokeWidth={1.8} className="welcome-task-icon" />
                      <span className="welcome-task-label">{T.welcome.taskFix}</span>
                    </button>
                    <button
                      type="button"
                      className="welcome-task"
                      data-testid="welcome-task-review"
                      onClick={() =>
                        window.dispatchEvent(
                          new CustomEvent("kodo:focus-composer", {
                            detail: T.welcome.taskReviewPrompt,
                          }),
                        )
                      }
                    >
                      <Bug size={14} strokeWidth={1.8} className="welcome-task-icon" />
                      <span className="welcome-task-label">{T.welcome.taskReview}</span>
                    </button>
                  </div>
                  <div className="welcome-hints" data-testid="welcome-hints">
                    <button
                      type="button"
                      className="welcome-hint welcome-hint--btn"
                      data-testid="welcome-hint-skills"
                      onClick={() =>
                        window.dispatchEvent(
                          new CustomEvent("kodo:focus-composer", { detail: "/" }),
                        )
                      }
                    >
                      {T.welcome.hintSkills}
                    </button>
                    <button
                      type="button"
                      className="welcome-hint welcome-hint--btn"
                      data-testid="welcome-hint-attach"
                      onClick={() =>
                        window.dispatchEvent(new CustomEvent("kodo:attach-files"))
                      }
                    >
                      {T.welcome.hintAttach}
                    </button>
                    <span className="welcome-hint welcome-hint--static">
                      {T.welcome.hintQueue}
                    </span>
                    {continueLast && onOpenConversation && (
                      <button
                        type="button"
                        className="btn btn--sm welcome-continue"
                        data-testid="welcome-continue"
                        onClick={() => onOpenConversation(continueLast.id)}
                      >
                        <ArrowRight size={12} strokeWidth={2} />
                        <span>{T.welcome.continueLast}</span>
                      </button>
                    )}
                  </div>
                </div>
                {pendingVisionSend && (
                  <div
                    className="action-error"
                    role="alert"
                    data-testid="vision-skip-notice"
                  >
                    <span className="action-error-msg">
                      {T.composer.visionSkippedNotice}
                    </span>
                    <div className="action-error-actions">
                      <button
                        type="button"
                        className="btn btn--primary btn--sm"
                        data-testid="vision-skip-ack"
                        onClick={ackVisionSkip}
                      >
                        {T.composer.visionSkippedNoticeAck}
                      </button>
                      <button
                        type="button"
                        className="btn btn--sm"
                        data-testid="vision-skip-cancel"
                        onClick={dismissVisionSkip}
                      >
                        取消
                      </button>
                    </div>
                  </div>
                )}
                {composer}
              </div>
            ) : (
              <>
                {demoMode && demoTurns.length > 0 ? (
                  demoTurns.map((turn, index) => {
                    const reply = demoReplyFor(demo!, index);
                    return (
                      <Fragment key={index}>
                        <UserMessage text={turn.user.content} turnIndex={index} />
                        <AssistantReply
                          reply={reply}
                          turnIndex={index}
                          running={false}
                          failovers={[]}
                          onViewFiles={onViewFiles}
                          onOpenLogs={onOpenTrace}
                          fileDiffs={fileDiffs}
                          sessionId={demo!.conversation.id}
                          // Demo has no project — openReferencedFile falls back to 改动.
                          onOpenPath={(raw) => void openReferencedFile(raw, reply.changes)}
                        />
                        <KnowledgeSources
                          sessionId={demo!.conversation.id}
                          projectPath={null}
                          turnSeq={index + 1}
                          finished
                          knowledgeUsed={new URLSearchParams(window.location.search).get("knowledge-fixture") === "citations"}
                        />
                      </Fragment>
                    );
                  })
                ) : turns.length === 0 && queue.length === 0 ? (
                  <p className="empty-note">发一条消息开始这次对话。</p>
                ) : (
                  turns.map((turn, index) => {
                    const last = index === turns.length - 1;
                    const reply = toReply(turn, running && last, running && last && Boolean(approval));
                    // Review opens the sidebar file panel at this turn's changes
                    // (demo has no file bodies → Inspector fallback).
                    const openViewer = (path: string | null, tab: "full" | "diff") =>
                      openFilePanel(path, tab, reply.changes);
                    return (
                      <Fragment key={index}>
                        <UserMessage
                          text={turn.ask}
                          context={turn.context}
                          probes={probes}
                          // Only project-internal paths render as openable cards;
                          // external absolute attachments stay static (no-op).
                          openContext={
                            projectPath
                              ? (path) => {
                                  const root = projectPath.replace(/\/+$/, "");
                                  if (path.startsWith("/") && !path.startsWith(`${root}/`)) {
                                    return null;
                                  }
                                  return (target) => void openReferencedFile(target, reply.changes);
                                }
                              : undefined
                          }
                          findHit={Boolean(activeFindHit && activeFindHit.part === "ask" && activeFindHit.turn === index)}
                          turnIndex={index}
                          // Session logs are append-only: 编辑预填后作为新轮发送，
                          // 不截断历史（见 Composer 的 editSeed 注释）。
                          onEdit={
                            liveSession
                              ? (text) => setEditSeed({ text, nonce: Date.now() })
                              : undefined
                          }
                        />
                        <AssistantReply
                          reply={reply}
                          findHit={Boolean(activeFindHit && activeFindHit.part === "reply" && activeFindHit.turn === index)}
                          turnIndex={index}
                          running={running && last}
                          progress={running && last ? progress : null}
                          elapsed={running && last ? elapsed : null}
                          waitingApproval={running && last ? Boolean(approval) : false}
                          compactToast={running && last ? compactToast : null}
                          budgetLiftToast={running && last ? budgetLiftToast : null}
                          failovers={last ? failovers : []}
                          onViewFiles={() => {
                            const first = reply.changes[0]?.path ?? null;
                            openViewer(first, "diff");
                          }}
                          onRegenerate={liveSession && last && !running ? regenerate : null}
                          onRetryEnvironment={liveSession && last && !running ? regenerate : null}
                          onContinueTurn={
                            liveSession && last && !running && reply.failureReason === "budget_exhausted"
                              ? continueTurn
                              : null
                          }
                          onOpenLogs={onOpenTrace}
                          fileDiffs={fileDiffs}
                          onUndoChanges={liveSession && last && !running && Boolean(projectPath) ? undoChanges : null}
                          undoingChanges={undoingChanges}
                          undoMessage={last ? undoMessage : null}
                          onOpenFile={
                            liveSession && Boolean(projectPath)
                              ? (path) => openViewer(path, "diff")
                              : null
                          }
                          // Inline code paths: probed at click, opened full-tab.
                          onOpenPath={(raw) => void openReferencedFile(raw, reply.changes)}
                          sessionId={sessionId}
                          preExisting={last ? turnMeta.preExisting : []}
                          conflicts={last ? turnMeta.conflicts : []}
                        />
                        <KnowledgeSources
                          sessionId={sessionId}
                          projectPath={knowledgeProjectPath}
                          turnSeq={index + 1}
                          finished={turn.done || turn.stopped || Boolean(turn.error) || !running}
                          knowledgeUsed={turn.knowledgeUsed === true}
                        />
                      </Fragment>
                    );
                  })
                )}
              </>
            )}

            {queue.length > 0 && (
              <div className="send-queue" data-testid="send-queue">
                <div className="send-queue-head">
                  <strong>待发送 · {queue.length}</strong>
                  <button type="button" className="btn btn--sm" data-testid="queue-clear" onClick={() => applyQueue([])}>
                    清空
                  </button>
                </div>
                <ul className="send-queue-list">
                  {queue.map((item, index) => (
                    <li key={`${item.text}-${index}`}>
                      <span className="send-queue-text">{item.text}</span>
                      <button
                        type="button"
                        className="icon-btn icon-btn--sm"
                        aria-label={`移出队列 ${index + 1}`}
                        onClick={() =>
                          applyQueue(queueRef.current.filter((_, position) => position !== index))
                        }
                      >
                        ×
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            )}

            {/* Approval sits outside the turn list so it also shows on an empty turn. */}
            {liveSession && approval && (
              <div className="approval-bar" role="alertdialog" aria-label="审批工具步骤" data-testid="approval-bar">
                <div className="approval-text">
                  <strong>
                    需要批准 · {approval.kind === "FileChange" ? "修改文件" : approval.kind}
                    {approval.riskCategory ? ` · ${approval.riskCategory}` : ""}
                  </strong>
                  <code data-testid="approval-command">{approval.detail || "继续执行该步骤"}</code>
                  {approval.reason && (
                    <span className="approval-reason" data-testid="approval-reason">
                      原因：{approval.reason}
                    </span>
                  )}
                  {approval.cwd && (
                    <span className="approval-cwd" data-testid="approval-cwd">
                      cwd: {approval.cwd}
                    </span>
                  )}
                  {approval.riskCategory && (
                    <span className="approval-risk" data-testid="approval-risk">
                      风险：{approval.riskCategory}
                    </span>
                  )}
                </div>
                <div className="approval-actions">
                  <button type="button" className="btn" data-testid="approval-deny" onClick={() => decide(false)}>
                    拒绝
                  </button>
                  <button
                    type="button"
                    className="btn"
                    data-testid="approval-allow-session"
                    title="本会话内相同命令不再询问"
                    onClick={() => decide(true, true)}
                  >
                    本会话允许
                  </button>
                  <button type="button" className="btn btn--primary" data-testid="approval-allow" onClick={() => decide(true)}>
                    允许
                  </button>
                </div>
              </div>
            )}
            {approvalError && (
              <div className="action-error" role="alert" data-testid="approval-error">
                <span className="action-error-msg">{approvalError}</span>
                <div className="action-error-actions">
                  <button type="button" className="btn btn--sm" onClick={() => setApprovalError(null)}>
                    关闭
                  </button>
                </div>
              </div>
            )}
          </div>
        </div>
        <TurnRail items={railItems} activeIndex={activeTurn} onNavigate={navigateToTurn} />
      </div>

      {!showWelcome && pendingVisionSend && (
        <div
          className="action-error"
          role="alert"
          data-testid="vision-skip-notice"
        >
          <span className="action-error-msg">{T.composer.visionSkippedNotice}</span>
          <div className="action-error-actions">
            <button
              type="button"
              className="btn btn--primary btn--sm"
              data-testid="vision-skip-ack"
              onClick={ackVisionSkip}
            >
              {T.composer.visionSkippedNoticeAck}
            </button>
            <button
              type="button"
              className="btn btn--sm"
              data-testid="vision-skip-cancel"
              onClick={dismissVisionSkip}
            >
              取消
            </button>
          </div>
        </div>
      )}
      {!showWelcome && composer}
    </main>
  );
}

function errorMessage(failure: unknown): string {
  if (failure instanceof Error) return failure.message;
  return String(failure);
}

function knowledgeErrorMessage(failure: unknown): string {
  const code = errorMessage(failure);
  const copy: Record<string, string> = {
    knowledge_binding_unavailable: "知识库绑定尚未准备好，请重新读取后再发送。",
    knowledge_remote_approval_required: "资料外发尚未获得批准，本轮未发送。",
    knowledge_unavailable: "知识库暂时不可用，本轮未发送。",
    app_disabled: "知识库功能当前已停用，本轮未发送。",
    not_found: "所选知识库或资料已不存在，请刷新绑定后重试。",
    scope_denied: "当前会话范围无法访问所选知识库。",
    storage_unavailable: "知识库暂时不可用，请稍后重试。",
    revision_conflict: "资料版本已变化，请重新检索后再试。",
    preview_expired: "发送预览已过期，请重新准备本轮。",
    source_changed: "资料来源已变化，请重新检索后再试。",
    invalid_input: "知识库选择无效，请检查绑定后重试。",
  };
  return copy[code] ?? "资料准备失败，请检查知识库绑定后重试。";
}

function navigateSettings() {
  navigate(window.location.pathname.startsWith("/ui-demo") ? "/ui-demo/settings" : "/settings");
}

/** Paths currently being re-probed — shared across every mounted bubble. */
const probeInflight = new Set<string>();

function UserMessage({
  text,
  context,
  probes,
  onEdit,
  openContext,
  findHit = false,
  turnIndex,
}: {
  text: string;
  context?: string[];
  probes?: Record<string, ContextProbe>;
  /** Live-only: prefill the composer with this turn (demo has no session to append to). */
  onEdit?: (text: string) => void;
  /** Per-path openability gate: return a handler to make the card clickable
   *  (opens in the sidebar file panel), or null to keep it a static card. */
  openContext?: (path: string) => ((path: string) => void) | null;
  /** True while this turn's ask is the active ⌘F find hit. */
  findHit?: boolean;
  /** Position in the turn list — the TurnRail's addressing hook. */
  turnIndex?: number;
}) {
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
    };
  }, []);

  const copyText = async () => {
    if (copied) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard denied — silent; the bubble stays selectable */
    }
  };

  return (
    <div className={findHit ? "msg-user find-hit" : "msg-user"} data-turn-index={turnIndex}>
      <div className="msg-actions">
        <button
          type="button"
          className="icon-btn icon-btn--sm reply-action"
          aria-label={copied ? T.action.copied : T.action.copyMessage}
          data-testid="msg-copy"
          title={copied ? T.action.copied : T.action.copyMessage}
          onClick={() => void copyText()}
        >
          {copied ? <Check size={13} strokeWidth={2.2} /> : <Copy size={13} strokeWidth={1.8} />}
        </button>
        {onEdit && (
          <button
            type="button"
            className="icon-btn icon-btn--sm reply-action"
            aria-label={T.action.editResend}
            data-testid="msg-edit"
            title={T.action.editResend}
            onClick={() => onEdit(text)}
          >
            <Pencil size={13} strokeWidth={1.8} />
          </button>
        )}
      </div>
      {context && context.length > 0 && (
        <div className="msg-contexts" data-testid="msg-contexts">
          {context.map((path) => (
            <ContextCard
              key={path}
              path={path}
              probe={probes?.[path]}
              onOpenPath={openContext?.(path) ?? null}
            />
          ))}
        </div>
      )}
      <p className="msg-bubble">{text}</p>
    </div>
  );
}

function blank(ask: string, context: string[] = [], knowledgeUsed = false): TurnDto {
  return {
    ask,
    context,
    items: [],
    done: false,
    stopped: false,
    interrupted: false,
    error: null,
    ...(knowledgeUsed ? { knowledgeUsed: true } : {}),
  };
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
  const next = index === -1 ? [...items, item] : items.map((existing, position) => (position === index ? item : existing));
  return next.sort((left, right) => left.id - right.id);
}
