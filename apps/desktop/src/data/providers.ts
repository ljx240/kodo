/**
 * Multi-provider AI configuration.
 *
 * Metadata lives in `settings.log`; API keys live only in `credentials.log`.
 * UI uses `display_name`; backend always sends `model_id`.
 */

import { isDesktop, loadProvidersCommand, saveProvidersCommand, setSetting, setting } from "../api";

export type ModelOption = {
  display_name: string;
  model_id: string;
  /** Whether the model accepts image attachments as multimodal content
   *  blocks. Wired from `ProviderCapabilities::vision`; landing on this
   *  flag with `false` triggers the Composer's first-time image-skip
   *  notice (ConversationPage). Defaults to `undefined` for legacy
   *  seeded catalog entries — treat undefined as opt-in (the template
   *  default kicks in). */
  vision?: boolean;
  /** Whether this channel is suitable for tasks that need a long final
   *  assistant message. Preview / flash / latest channels set this to
   *  `false` via the suffix sniffer in
   *  `kodo_agent::resolve_model_identity`; the desktop picker reads it
   *  to render a "预览通道" warning on the active model. Undefined on
   *  legacy seeded catalog entries — treated as `true` (the safe
   *  default) so older data does not regress into a warning storm. */
  suitableForLong?: boolean;
};

export type ProviderTemplate = {
  id: string;
  name: string;
  models: ModelOption[];
  keyPlaceholder: string;
  customEndpoint: boolean;
};

export type ProviderConfig = {
  id: string;
  name: string;
  template: string;
  /** Masked or empty when loaded from the shell. */
  apiKey: string;
  endpoint: string;
  /** Legacy field — may hold display name or model id. Prefer modelId. */
  model: string;
  /** Backend API model id. */
  modelId?: string;
  /** UI label. */
  displayName?: string;
  /** Configured models. `undefined` = legacy record (seed from template);
   *  `[]` = user deliberately cleared the list. */
  models?: ModelOption[];
  hasKey?: boolean;
};

export const PROVIDER_TEMPLATES: ProviderTemplate[] = [
  {
    id: "anthropic",
    name: "Anthropic",
    models: [
      { display_name: "Claude 3.5 Sonnet", model_id: "claude-3-5-sonnet-latest", vision: true },
      { display_name: "Claude Sonnet 5", model_id: "claude-sonnet-4-5", vision: true },
      { display_name: "Claude Opus 5", model_id: "claude-opus-4-1", vision: true },
    ],
    keyPlaceholder: "sk-ant-...",
    customEndpoint: false,
  },
  {
    id: "openai",
    name: "OpenAI",
    models: [
      // gpt-4o / gpt-4o-mini accept image inputs; the o1 reasoning
      // family is text-only as of 2026-10 — explicit `vision: false`
      // wins over the openai_compatible default.
      { display_name: "GPT-4o", model_id: "gpt-4o", vision: true },
      { display_name: "GPT-4o mini", model_id: "gpt-4o-mini", vision: true },
      { display_name: "o1", model_id: "o1", vision: false },
      { display_name: "o1-mini", model_id: "o1-mini", vision: false },
    ],
    keyPlaceholder: "sk-...",
    customEndpoint: true,
  },
  {
    id: "deepseek",
    name: "DeepSeek",
    models: [
      // DeepSeek chat / reasoner endpoints do not accept image inputs
      // (verified against the public API surface, 2026-10). Catalog
      // entries pin `vision: false` so the Composer's first-time
      // image-skip notice fires the same turn the user attaches a PNG.
      { display_name: "DeepSeek-V3", model_id: "deepseek-chat", vision: false },
      { display_name: "DeepSeek-R1", model_id: "deepseek-reasoner", vision: false },
    ],
    keyPlaceholder: "sk-...",
    customEndpoint: true,
  },
  {
    id: "custom",
    name: "自定义 (OpenAI 兼容)",
    models: [],
    keyPlaceholder: "API Key",
    customEndpoint: true,
  },
];

const ACTIVE_PROVIDER_KEY = "active-provider";

export function templateById(id: string): ProviderTemplate {
  return PROVIDER_TEMPLATES.find((t) => t.id === id) ?? PROVIDER_TEMPLATES[0];
}

/** Dedupe by lowercase id; first entry wins. Preserves `vision` so a
 *  fetched-from-API model that the catalog knows is text-only (e.g. o1)
 *  keeps its false flag across a round-trip through the user's saved
 *  provider config. */
export function dedupeModels(list: ModelOption[]): ModelOption[] {
  const seen = new Set<string>();
  const out: ModelOption[] = [];
  for (const m of list) {
    const key = m.model_id.trim().toLowerCase();
    if (!key || seen.has(key)) continue;
    seen.add(key);
    out.push({
      display_name: m.display_name.trim() || m.model_id,
      model_id: m.model_id.trim(),
      vision: m.vision,
    });
  }
  return out;
}

/** Migrate legacy config: display label → model_id when known. */
export function migrateProviderConfig(config: ProviderConfig): ProviderConfig {
  const template = config.template || "custom";
  const rawId = config.modelId?.trim() || config.model?.trim() || "";
  const tmpl = templateById(template);
  const known = tmpl.models.find(
    (m) =>
      m.model_id.toLowerCase() === rawId.toLowerCase() ||
      m.display_name.toLowerCase() === rawId.toLowerCase(),
  );
  const modelId = known?.model_id || (rawId && !rawId.includes(" ") ? rawId : rawId);
  const displayName =
    config.displayName?.trim() ||
    known?.display_name ||
    rawId ||
    tmpl.models[0]?.display_name ||
    "";
  // Seed legacy records (models === undefined) from template presets + the
  // selected pointer; an explicit [] means the user cleared the list.
  let models: ModelOption[];
  if (config.models === undefined) {
    models = dedupeModels([...tmpl.models]);
  } else {
    models = dedupeModels(config.models);
  }
  if (modelId && !models.some((m) => m.model_id.toLowerCase() === modelId.toLowerCase())) {
    models = [...models, { display_name: displayName || modelId, model_id: modelId }];
  }
  return {
    ...config,
    template,
    model: modelId,
    modelId,
    displayName,
    models,
  };
}

export async function loadProviders(): Promise<ProviderConfig[]> {
  if (!isDesktop()) return [];
  const list = await loadProvidersCommand();
  return (list ?? []).map((dto) =>
    migrateProviderConfig({
      ...dto,
      // wire format is camelCase; internal ModelOption is snake_case.
      // `vision` rides both ways (Tauri command returns it on the way
      // down; undefined entries fall back to the template default at
      // notice-render time). `suitableForLong` rides too so the picker
      // can render the preview-channel warning on the active model.
      models: dto.models
        ? dto.models.map((m) => ({
            display_name: m.displayName,
            model_id: m.modelId,
            vision: m.vision,
            suitableForLong: m.suitableForLong,
          }))
        : undefined,
    }),
  );
}

export async function saveProviders(providers: ProviderConfig[]): Promise<void> {
  if (!isDesktop()) return;
  const migrated = providers.map(migrateProviderConfig);
  await saveProvidersCommand(
    migrated.map((p) => ({
      ...p,
      model: p.modelId || p.model,
      // Pass through `vision` when known so the Tauri list_provider_models
      // recompute agrees with the frontend catalog. `undefined` entries
      // (e.g. user-added custom model ids) get the template default.
      // `suitableForLong` rides the wire too so the picker can render
      // the preview-channel warning without re-sniffing on every read.
      models: (p.models ?? []).map((m) => ({
        modelId: m.model_id,
        displayName: m.display_name,
        ...(m.vision === undefined ? {} : { vision: m.vision }),
        ...(m.suitableForLong === undefined
          ? {}
          : { suitableForLong: m.suitableForLong }),
      })),
    })),
  );
}

export async function loadActiveIndex(): Promise<number> {
  if (!isDesktop()) return 0;
  const raw = await setting(ACTIVE_PROVIDER_KEY);
  const n = Number(raw);
  return Number.isFinite(n) && n >= 0 ? n : 0;
}

export async function saveActiveIndex(index: number): Promise<void> {
  if (!isDesktop()) return;
  await setSetting(ACTIVE_PROVIDER_KEY, String(index));
}

export function uid(): string {
  return Math.random().toString(36).slice(2, 10);
}

export function createFromTemplate(templateId: string): ProviderConfig {
  const t = templateById(templateId);
  const first = t.models[0];
  return {
    id: uid(),
    name: t.name,
    template: t.id,
    apiKey: "",
    endpoint: "",
    model: first?.model_id ?? "",
    modelId: first?.model_id ?? "",
    displayName: first?.display_name ?? "",
    models: [...t.models],
  };
}

/** Display label for a (possibly masked) key. */
export function maskApiKey(key: string, hasKey?: boolean): string {
  if (hasKey === false && !key) return "未设置";
  if (!key) return "未设置";
  if (key.includes("•")) return `已保存 · ${key}`;
  return `已保存 · …${key.slice(-4)}`;
}

/** Strips trailing version/channel tags that leak from raw model ids. Cosmetic
 *  only — `providerModelId` (the wire value) is unchanged. */
const VERSION_SUFFIX = /-(preview|latest|exp|beta|alpha)(\b|$)/i;

function prettifyModelLabel(raw: string): string {
  const s = raw.trim();
  if (!s) return "";
  return s.replace(VERSION_SUFFIX, "") || raw;
}

/** Whether the given model id is a preview / flash / latest channel.
 *  Mirrors `kodo_agent::looks_like_preview_channel` — keep them in sync
 *  so the desktop picker and the agent loop agree on what counts as
 *  "preview". Used both for catalog migration and for the per-row
 *  warning rendered in the settings picker. */
export function looksLikePreviewChannel(modelId: string): boolean {
  const id = modelId.trim().toLowerCase();
  if (!id) return false;
  return (
    id.endsWith("-preview") ||
    id.endsWith("-flash-preview") ||
    id.endsWith("-flash") ||
    id.endsWith("-latest") ||
    id.includes("-flash-preview-")
  );
}

/** Whether the given model is suitable for tasks that need a long final
 *  assistant message. Undefined → safe default `true` so legacy catalog
 *  entries never render a false-positive warning. */
export function isSuitableForLong(model: { suitableForLong?: boolean; model_id: string }): boolean {
  if (model.suitableForLong !== undefined) return model.suitableForLong;
  // Fall back to the local suffix sniffer so a catalog row that hasn't
  // been refreshed through the Tauri list command still gets the
  // warning. Matches the agent-side sniffer exactly.
  return !looksLikePreviewChannel(model.model_id);
}

/** What the provider picker shows. */
export function providerModelLabel(config: ProviderConfig | null | undefined): string {
  return prettifyModelLabel(config?.displayName || config?.modelId || config?.model || "");
}

/** What the backend will send. */
export function providerModelId(config: ProviderConfig): string {
  return config.modelId || config.model || "";
}
