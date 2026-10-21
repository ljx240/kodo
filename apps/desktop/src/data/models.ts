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
  { value: "ask", label: "请求批准", description: "运行命令、编辑文件和外部调用均需批准" },
  { value: "auto", label: "自动批准安全操作", description: "已识别的只读命令自动执行；其他操作需确认" },
  {
    value: "full",
    label: "项目内自动执行",
    description: "项目文件编辑、已识别的只读命令与项目构建/测试命令自动执行；其他命令和外部调用需确认",
  },
];

export const PERMISSION_SETTING = "permission";
export const DEFAULT_PERMISSION: Permission = "ask";

/**
 * Sandbox modes for untrusted project code.
 *
 * Orthogonal to the permission mode: the permission mode decides whether a
 * human is asked, the sandbox decides what the process may reach. Default is
 * `workspace-write` (Secure by Default).
 */
export type SandboxMode = "read-only" | "workspace-write" | "unrestricted";

export const SANDBOXES: { value: SandboxMode; label: string; description: string }[] = [
  {
    value: "read-only",
    label: "工作区只读",
    description: "源码不可修改，仅允许在私有临时目录写入",
  },
  {
    value: "workspace-write",
    label: "工作区写入",
    description: "可写工作区，项目外写入被操作系统阻止，禁止出网",
  },
  {
    value: "unrestricted",
    label: "不受限",
    description: "关闭沙箱，命令以当前用户权限直接执行（风险自负）",
  },
];

export const SANDBOX_SETTING = "sandbox-mode";
export const DEFAULT_SANDBOX: SandboxMode = "workspace-write";
