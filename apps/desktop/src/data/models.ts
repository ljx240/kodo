/**
 * What the model pickers offer.
 *
 * Nothing calls a model API until a Provider key is configured. The first
 * entry is the default the reference screenshots show.
 */
export const MODELS = ["Claude 3.5 Sonnet", "Claude Sonnet 5", "Claude Opus 5"];

export const DEFAULT_MODEL = MODELS[0];

/** The settings key the default-model chip is stored under. */
export const MODEL_SETTING = "default-model";

/** Permission modes for the run driver. Default is ask (Secure by Default). */
export type Permission = "ask" | "auto" | "full";

export const PERMISSIONS: { value: Permission; label: string; description: string }[] = [
  { value: "ask", label: "请求批准", description: "每个 shell 命令都需要您手动批准" },
  { value: "auto", label: "自动批准安全操作", description: "普通命令自动执行，危险命令仍需确认" },
  { value: "full", label: "完全访问", description: "所有命令自动执行，无需确认（不推荐）" },
];

export const PERMISSION_SETTING = "permission";
export const DEFAULT_PERMISSION: Permission = "ask";
