/**
 * Multi-provider AI configuration.
 *
 * Metadata lives in `settings.log`; API keys live only in `credentials.log`.
 * The webview sees a mask (`••••abcd`), never the raw secret — send a new
 * key only when changing it; leave the mask/blank to keep the stored one.
 */

import { isDesktop, loadProvidersCommand, saveProvidersCommand, setSetting, setting } from "../api";

export type ProviderTemplate = {
  id: string;
  name: string;
  models: string[];
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
  model: string;
  hasKey?: boolean;
};

export const PROVIDER_TEMPLATES: ProviderTemplate[] = [
  {
    id: "anthropic",
    name: "Anthropic",
    models: ["Claude 3.5 Sonnet", "Claude Sonnet 5", "Claude Opus 5"],
    keyPlaceholder: "sk-ant-...",
    customEndpoint: false,
  },
  {
    id: "openai",
    name: "OpenAI",
    models: ["GPT-4o", "GPT-4o mini", "o1", "o1-mini"],
    keyPlaceholder: "sk-...",
    customEndpoint: true,
  },
  {
    id: "deepseek",
    name: "DeepSeek",
    models: ["DeepSeek-V3", "DeepSeek-R1"],
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

export async function loadProviders(): Promise<ProviderConfig[]> {
  if (!isDesktop()) return [];
  const list = await loadProvidersCommand();
  return list ?? [];
}

export async function saveProviders(providers: ProviderConfig[]): Promise<void> {
  if (!isDesktop()) return;
  await saveProvidersCommand(providers);
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
  return {
    id: uid(),
    name: t.name,
    template: t.id,
    apiKey: "",
    endpoint: "",
    model: t.models[0] ?? "",
  };
}

/** Display label for a (possibly masked) key. */
export function maskApiKey(key: string, hasKey?: boolean): string {
  if (hasKey === false && !key) return "未设置";
  if (!key) return "未设置";
  if (key.includes("•")) return `已保存 · ${key}`;
  return `已保存 · …${key.slice(-4)}`;
}
