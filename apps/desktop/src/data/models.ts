/**
 * What the model pickers offer.
 *
 * Display labels for the reference UI. Backend always uses model_id via
 * the provider catalog — these chips are not sent as API model ids.
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
  {
    value: "full",
    label: "完全访问",
    description: "项目内操作自动执行；破坏性 git / 提权仍需确认（不推荐）",
  },
];

export const PERMISSION_SETTING = "permission";
export const DEFAULT_PERMISSION: Permission = "ask";
