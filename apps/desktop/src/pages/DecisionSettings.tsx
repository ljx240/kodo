import { useCallback, useEffect, useState } from "react";
import {
  decisionClearCredential,
  decisionGetSettings,
  decisionSaveSettings,
  decisionSetCredential,
  type DecisionSettingsDto,
} from "../decisionApi";
import { Field, Switch } from "../shell/controls";
import { T } from "../i18n";

type Mode = "off" | "observe" | "assist";
type Provider = "typeSafe" | "openRouter";

/** The settings form's own state. It starts from the backend's defaults so
 *  the numeric limits live in exactly one place. */
type Form = {
  provider: Provider;
  mode: Mode;
  model: string;
  taskRoute: boolean;
  failureTriage: boolean;
  requestTimeoutMs: number;
  maxCallsPerTurn: number;
  maxRequestBytes: number;
  maxStateBytes: number;
  maxResponseBytes: number;
  maxDecisionTimeMsPerTurn: number;
};

function toForm(dto: DecisionSettingsDto): Form {
  let base: Partial<Form> = {};
  try {
    base = JSON.parse(dto.defaultsJson) as Partial<Form>;
  } catch {
    base = {};
  }
  const mode = (["off", "observe", "assist"] as const).includes(dto.mode as Mode)
    ? (dto.mode as Mode)
    : "off";
  const provider = (["typeSafe", "openRouter"] as const).includes(dto.provider as Provider)
    ? (dto.provider as Provider)
    : "typeSafe";
  return {
    provider,
    mode,
    model: dto.model || base.model || "jev-1.13.0",
    taskRoute: dto.taskRouteEnabled,
    failureTriage: dto.failureTriageEnabled,
    requestTimeoutMs: base.requestTimeoutMs ?? 1500,
    maxCallsPerTurn: base.maxCallsPerTurn ?? 3,
    maxRequestBytes: base.maxRequestBytes ?? 32768,
    maxStateBytes: base.maxStateBytes ?? 16384,
    maxResponseBytes: base.maxResponseBytes ?? 65536,
    maxDecisionTimeMsPerTurn: base.maxDecisionTimeMsPerTurn ?? 4500,
  };
}

function toJson(form: Form): string {
  return JSON.stringify({
    schemaVersion: 1,
    provider: form.provider,
    mode: form.mode,
    model: form.model,
    scenarios: { taskRoute: form.taskRoute, failureTriage: form.failureTriage },
    requestTimeoutMs: form.requestTimeoutMs,
    maxCallsPerTurn: form.maxCallsPerTurn,
    maxRequestBytes: form.maxRequestBytes,
    maxStateBytes: form.maxStateBytes,
    maxResponseBytes: form.maxResponseBytes,
    maxDecisionTimeMsPerTurn: form.maxDecisionTimeMsPerTurn,
    policyVersion: "kodo-jev-v1",
  });
}

/** The model each provider starts from. */
const PROVIDER_DEFAULT_MODEL: Record<Provider, string> = {
  typeSafe: "jev-1.13.0",
  openRouter: "typesafe/jev-1.14",
};

export function DecisionSettingsBody() {
  const [dto, setDto] = useState<DecisionSettingsDto | null>(null);
  const [form, setForm] = useState<Form | null>(null);
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    const next = await decisionGetSettings();
    setDto(next);
    setForm(next ? toForm(next) : null);
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const save = async () => {
    if (!form) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await decisionSaveSettings(toJson(form));
      setNotice(T.decision.saved);
      await reload();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const storeSecret = async () => {
    if (!secret.trim()) return;
    const provider = form?.provider ?? "typeSafe";
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await decisionSetCredential(provider, secret);
      setSecret("");
      setNotice(T.decision.credentialSaved);
      await reload();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const clearSecret = async () => {
    const provider = form?.provider ?? "typeSafe";
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await decisionClearCredential(provider);
      setNotice(T.decision.credentialCleared);
      await reload();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  if (!form) {
    return <p className="empty-note">{T.decision.loading}</p>;
  }

  // The key belongs to the selected provider: TypeSafe and OpenRouter keys
  // live under their own ids.
  const credentialState =
    dto?.credentials.find((entry) => entry.provider === form.provider) ?? null;

  const set = <K extends keyof Form>(key: K, value: Form[K]) =>
    setForm((current) => (current ? { ...current, [key]: value } : current));

  return (
    <div className="decision-settings" data-testid="decision-settings">
      <p className="decision-settings-notice">{T.decision.outboundNotice}</p>

      {dto?.configError && (
        <p className="decision-settings-error" role="alert" data-testid="decision-config-error">
          {dto.configError}
        </p>
      )}

      {dto?.recovered && (
        <p className="decision-settings-recovered" role="status" data-testid="decision-recovered">
          {T.decision.recovered}
        </p>
      )}

      <Field label={T.decision.providerLabel} hint={T.decision.providerHint} wide>
        <div className="decision-mode" role="radiogroup" aria-label={T.decision.providerLabel}>
          {(["typeSafe", "openRouter"] as const).map((provider) => (
            <button
              key={provider}
              type="button"
              role="radio"
              aria-checked={form.provider === provider}
              className={`decision-mode-option${form.provider === provider ? " decision-mode-option--on" : ""}`}
              data-testid={`decision-provider-${provider}`}
              disabled={busy}
              onClick={() => {
                set("provider", provider);
                // The model id means different things per provider, so the
                // default follows the provider.
                set("model", PROVIDER_DEFAULT_MODEL[provider]);
              }}
            >
              {T.decision.providers[provider]}
            </button>
          ))}
        </div>
      </Field>

      <Field label={T.decision.modeLabel} hint={T.decision.modeHint} wide>
        <div className="decision-mode" role="radiogroup" aria-label={T.decision.modeLabel}>
          {(["off", "observe", "assist"] as const).map((mode) => (
            <button
              key={mode}
              type="button"
              role="radio"
              aria-checked={form.mode === mode}
              className={`decision-mode-option${form.mode === mode ? " decision-mode-option--on" : ""}`}
              data-testid={`decision-mode-${mode}`}
              disabled={busy}
              onClick={() => set("mode", mode)}
            >
              {T.decision.modes[mode]}
            </button>
          ))}
        </div>
      </Field>

      <Field label={T.decision.modelLabel} hint={T.decision.modelHint}>
        <input
          className="input"
          value={form.model}
          data-testid="decision-model"
          disabled={busy}
          onChange={(event) => set("model", event.target.value)}
        />
      </Field>

      <Field label={T.decision.taskRouteLabel} hint={T.decision.taskRouteHint}>
        <Switch
          checked={form.taskRoute}
          label={T.decision.taskRouteLabel}
          onChange={() => set("taskRoute", !form.taskRoute)}
        />
      </Field>

      <Field label={T.decision.failureTriageLabel} hint={T.decision.failureTriageHint}>
        <Switch
          checked={form.failureTriage}
          label={T.decision.failureTriageLabel}
          onChange={() => set("failureTriage", !form.failureTriage)}
        />
      </Field>

      <Field label={T.decision.credentialLabel} hint={T.decision.credentialHint} block>
        <div className="decision-credential">
          <input
            className="input"
            type="password"
            value={secret}
            placeholder={
              credentialState?.hasCredential
                ? credentialState.credentialMask
                : T.decision.credentialPlaceholder
            }
            data-testid="decision-credential"
            disabled={busy}
            onChange={(event) => setSecret(event.target.value)}
          />
          <button
            type="button"
            className="btn"
            data-testid="decision-credential-save"
            disabled={busy || !secret.trim()}
            onClick={() => void storeSecret()}
          >
            {T.decision.credentialSave}
          </button>
          <button
            type="button"
            className="btn"
            data-testid="decision-credential-clear"
            disabled={busy || !credentialState?.hasCredential}
            onClick={() => void clearSecret()}
          >
            {T.decision.credentialClear}
          </button>
        </div>
      </Field>

      {notice && (
        <p className="decision-settings-ok" role="status" data-testid="decision-notice">
          {notice}
        </p>
      )}
      {error && (
        <p className="decision-settings-error" role="alert" data-testid="decision-error">
          {error}
        </p>
      )}

      <div className="decision-settings-actions">
        <button
          type="button"
          className="btn btn--primary"
          data-testid="decision-save"
          disabled={busy}
          onClick={() => void save()}
        >
          {T.decision.save}
        </button>
      </div>
    </div>
  );
}
