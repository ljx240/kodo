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
  hasKey?: boolean;
};

export const PROVIDER_TEMPLATES: ProviderTemplate[] = [
  {
    id: "anthropic",
    name: "Anthropic",
    models: [
      { display_name: "Claude 3.5 Sonnet", model_id: "claude-3-5-sonnet-latest" },
      { display_name: "Claude Sonnet 5", model_id: "claude-sonnet-4-5" },
      { display_name: "Claude Opus 5", model_id: "claude-opus-4-1" },
    ],
    keyPlaceholder: "sk-ant-...",
    customEndpoint: false,
  },
  {
    id: "openai",
    name: "OpenAI",
    models: [
      { display_name: "GPT-4o", model_id: "gpt-4o" },
      { display_name: "GPT-4o mini", model_id: "gpt-4o-mini" },
      { display_name: "o1", model_id: "o1" },
      { display_name: "o1-mini", model_id: "o1-mini" },
    ],
    keyPlaceholder: "sk-...",
    customEndpoint: true,
  },
  {
    id: "deepseek",
    name: "DeepSeek",
    models: [
      { display_name: "DeepSeek-V3", model_id: "deepseek-chat" },
      { display_name: "DeepSeek-R1", model_id: "deepseek-reasoner" },
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
  return {
    ...config,
    template,
    model: modelId,
    modelId,
    displayName,
  };
}

export async function loadProviders(): Promise<ProviderConfig[]> {
  if (!isDesktop()) return [];
  const list = await loadProvidersCommand();
  return (list ?? []).map(migrateProviderConfig);
}

export async function saveProviders(providers: ProviderConfig[]): Promise<void> {
  if (!isDesktop()) return;
  const migrated = providers.map(migrateProviderConfig);
  await saveProvidersCommand(
    migrated.map((p) => ({
      ...p,
      model: p.modelId || p.model,
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
  };
}

/** Display label for a (possibly masked) key. */
export function maskApiKey(key: string, hasKey?: boolean): string {
  if (hasKey === false && !key) return "未设置";
  if (!key) return "未设置";
  if (key.includes("•")) return `已保存 · ${key}`;
  return `已保存 · …${key.slice(-4)}`;
}

/** What the provider picker shows. */
export function providerModelLabel(config: ProviderConfig | null | undefined): string {
  return config?.displayName || config?.modelId || config?.model || "";
}

/** What the backend will send. */
export function providerModelId(config: ProviderConfig): string {
  return config.modelId || config.model || "";
}
