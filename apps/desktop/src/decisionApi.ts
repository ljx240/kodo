import { invoke } from "@tauri-apps/api/core";

/** Mirrors `kodo_shell::decision::DecisionSettingsView`. The credential never
 *  comes back as plaintext — only a mask and a boolean. */
export type DecisionSettingsDto = {
  /** `typeSafe`, `openRouter` or `tencent`. */
  provider: string;
  mode: string;
  assistAvailable: boolean;
  model: string;
  taskRouteEnabled: boolean;
  failureTriageEnabled: boolean;
  hasCredential: boolean;
  credentialMask: string;
  /** The credential state of every provider. */
  credentials: DecisionCredentialDto[];
  configError: string | null;
  /** True when the stored record was recovered from a truncated write. */
  recovered: boolean;
  configGeneration: number;
  /** The default configuration as JSON. The form starts from this so the
   *  numeric limits live in exactly one place. */
  defaultsJson: string;
};

/** The credential state of one provider. */
export type DecisionCredentialDto = {
  provider: string;
  id: string;
  hasCredential: boolean;
  credentialMask: string;
};

/** The one outbound request the user is asked about. The body is the exact
 *  bytes that would be sent — a summary cannot be approved, only read. */
export type DecisionPreviewDto = {
  session: string;
  requestId: string;
  runGeneration: number;
  scenario: string;
  mode: string;
  origin: string;
  path: string;
  model: string;
  purpose: string;
  requestBody: string;
  questions: string;
  requestBytes: number;
  stateBytes: number;
  expiresInSeconds: number;
};

/** Mirrors `kodo_agent::decision::DecisionRecord` — the bounded metadata the
 *  Inspector shows. It never carries the request body. */
export type DecisionRecordDto = {
  schema: string;
  requestId: string;
  mode: string;
  /** `typeSafe`, `openRouter` or `tencent` — the service that answered. */
  provider: string;
  model: string;
  policyVersion: string;
  status: string;
  baseline?: string;
  suggestion?: string;
  applied: boolean;
  probabilities?: Record<string, number>;
  confidence?: number;
  durationMs: number;
  consentWaitMs: number;
  inputTokens: number | null;
  rejectedReason?: string;
  skipCode?: string;
  fallbackCode?: string;
};

function isDesktop(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

/** The default configuration, as the backend hands it to the form. Kept in
 *  sync with `kodo_agent::decision::DecisionSettings::default()`. */
const DEFAULT_SETTINGS_JSON = JSON.stringify({
  schemaVersion: 1,
  provider: "typeSafe",
  mode: "off",
  model: "jev-1.13.0",
  scenarios: { taskRoute: false, failureTriage: false },
  requestTimeoutMs: 1500,
  maxCallsPerTurn: 3,
  maxRequestBytes: 32768,
  maxStateBytes: 16384,
  maxResponseBytes: 65536,
  maxDecisionTimeMsPerTurn: 4500,
  policyVersion: "kodo-jev-v1",
});

/** What the settings form sees in a plain browser (the visual suite and
 *  `vite dev`). The feature is off and no credential is stored, so nothing
 *  can be sent. */
const BROWSER_SETTINGS: DecisionSettingsDto = {
  provider: "typeSafe",
  mode: "off",
  assistAvailable: true,
  model: "jev-1.13.0",
  taskRouteEnabled: false,
  failureTriageEnabled: false,
  hasCredential: false,
  credentialMask: "",
  credentials: [
    { provider: "typeSafe", id: "jev", hasCredential: false, credentialMask: "" },
    { provider: "openRouter", id: "openrouter", hasCredential: false, credentialMask: "" },
    { provider: "tencent", id: "tencent", hasCredential: false, credentialMask: "" },
  ],
  configError: null,
  recovered: false,
  configGeneration: 1,
  defaultsJson: DEFAULT_SETTINGS_JSON,
};

/** The decision settings. `null` outside the desktop shell. */
export function decisionGetSettings(): Promise<DecisionSettingsDto | null> {
  if (!isDesktop()) return Promise.resolve(BROWSER_SETTINGS);
  return invoke<DecisionSettingsDto>("decision_get_settings").catch((error: unknown) => {
    console.warn("kodo: decision_get_settings failed:", error);
    return null;
  });
}

/** Store the decision settings. The backend validates before it writes, so
 *  an illegal mode or an out-of-range quota is refused, not stored. */
export function decisionSaveSettings(settings: string): Promise<number | null> {
  if (!isDesktop()) return Promise.resolve(BROWSER_SETTINGS.configGeneration);
  return invoke<number>("decision_save_settings", { settings });
}

/** Store a provider's decision credential. The plaintext never comes back. */
export function decisionSetCredential(
  provider: string,
  secret: string,
): Promise<number | null> {
  if (!isDesktop()) return Promise.resolve(BROWSER_SETTINGS.configGeneration);
  return invoke<number>("decision_set_credential", { provider, secret });
}

/** Remove one provider's decision credential. */
export function decisionClearCredential(provider: string): Promise<number | null> {
  if (!isDesktop()) return Promise.resolve(BROWSER_SETTINGS.configGeneration);
  return invoke<number>("decision_clear_credential", { provider });
}

/** Answer one pending send preview. The request id is the only thing the
 *  frontend supplies: the session, the run and the bytes come from the
 *  backend's own record. */
export function decisionConfirmPreview(
  requestId: string,
  approved: boolean,
): Promise<void | null> {
  if (!isDesktop()) return Promise.resolve(null);
  return invoke<void>("decision_confirm_preview", { requestId, approved });
}

/** The preview a live conversation is still waiting on.
 *
 *  The run event reaches only the conversation that was open when it was
 *  emitted. A user who switched away gets the dialog back from this call
 *  when they return; the backend refuses a run that no longer owns the
 *  session. */
export function decisionPendingPreview(
  session: string,
): Promise<DecisionPreviewDto | null> {
  if (!isDesktop()) return Promise.resolve(null);
  return invoke<DecisionPreviewDto | null>("decision_pending_preview", {
    session,
  }).catch((error: unknown) => {
    console.warn("kodo: decision_pending_preview failed:", error);
    return null;
  });
}

/**
 * The demo send preview.
 *
 * The visual suite drives the preview from the address bar, the same way
 * the knowledge fixture does. The content is fictional: no real request,
 * no real credential, no live clock.
 */
export function demoDecisionPreview(): DecisionPreviewDto | null {
  if (isDesktop()) return null;
  if (new URLSearchParams(window.location.search).get("decision-preview") !== "demo") {
    return null;
  }
  const requestBody = JSON.stringify({
    model: "jev-1.13.0",
    state: { message: "把重复的校验逻辑合并一下", mode: "code" },
    questions: {
      task_type: {
        type: "choice",
        instructions: "Classify the primary requested coding task in state.message.",
        criteria: {
          "bug-fix": "Repair an existing incorrect behavior",
          feature: "Add new product behavior",
          refactor: "Restructure code while preserving behavior",
          unknown: "Insufficient information or no clear primary task",
        },
      },
    },
  });
  return {
    session: "demo-session",
    requestId: "jev-demo",
    runGeneration: 1,
    scenario: "taskRoute",
    mode: "observe",
    origin: "https://api.typesafe.ai",
    path: "/v1/systemone",
    model: "jev-1.13.0",
    purpose: "判断这次任务应归入哪一类工作流程（只影响任务分类，不改变权限与预算）",
    requestBody,
    questions: JSON.stringify(
      {
        task_type: {
          type: "choice",
          criteria: {
            "bug-fix": "Repair an existing incorrect behavior",
            feature: "Add new product behavior",
            refactor: "Restructure code while preserving behavior",
            unknown: "Insufficient information or no clear primary task",
          },
        },
      },
      null,
      2,
    ),
    requestBytes: requestBody.length,
    stateBytes: 48,
    expiresInSeconds: 300,
  };
}

/** Read a decision record out of a reasoning diagnostic. Returns `null` when
 *  the diagnostic is something else — an unreadable schema must not break
 *  the trace. */
export function parseDecisionRecord(diagnostics: string | null | undefined): DecisionRecordDto | null {
  if (!diagnostics) return null;
  const text = diagnostics.trim();
  if (!text.startsWith("{")) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== "object") return null;
  const record = parsed as Partial<DecisionRecordDto>;
  if (record.schema !== "decision.v1") return null;
  return {
    schema: record.schema,
    requestId: record.requestId ?? "",
    mode: record.mode ?? "off",
    provider: record.provider ?? "typeSafe",
    model: record.model ?? "",
    policyVersion: record.policyVersion ?? "",
    status: record.status ?? "skipped",
    baseline: record.baseline,
    suggestion: record.suggestion,
    applied: Boolean(record.applied),
    probabilities: record.probabilities,
    confidence: record.confidence,
    durationMs: record.durationMs ?? 0,
    consentWaitMs: record.consentWaitMs ?? 0,
    inputTokens: record.inputTokens ?? null,
    rejectedReason: record.rejectedReason,
    skipCode: record.skipCode,
    fallbackCode: record.fallbackCode,
  };
}
