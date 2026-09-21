import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ArrowUp, ChevronDown, FileText, Paperclip, Plus, Puzzle, Sparkles, Square, Shield, ShieldCheck, ShieldAlert, Settings, X } from "lucide-react";
import { useLayoutEffect, useRef, useState, useEffect, useCallback } from "react";
import { isDesktop, pickFiles, setSetting, setting, validateContextPath } from "../api";
import { type ProviderConfig, templateById } from "../data/providers";
import { type Permission, PERMISSIONS, PERMISSION_SETTING, DEFAULT_PERMISSION } from "../data/models";
import { Menu, MenuItem } from "../shell/Menu";
import { navigate } from "../routes";

/** The draft stops growing here and starts scrolling instead (LAYOUT.md:150). */
const MAX_HEIGHT = 160;

type Props = {
  /** Active provider config. */
  provider: ProviderConfig | null;
  /** All configured providers. */
  providers: ProviderConfig[];
  /** Switch to a different provider. */
  onSelectProvider: (index: number) => void;
  /** False when there is nowhere to send to — a demo route, or no session. */
  ready: boolean;
  running: boolean;
  /** Project root for Add context; empty disables the picker. */
  projectPath: string;
  /** Project-relative paths currently pinned for the next send. */
  contexts: string[];
  onAddContext: (path: string) => void;
  onRemoveContext: (path: string) => void;
  /** Recoverable failure from adding context (illegal path, read failure). */
  onContextError: (message: string) => void;
  onSend: (text: string, context: string[]) => void;
  onStop: () => void;
  /** Provider is missing / unavailable — shown with a Settings recovery action. */
  providerWarning: string | null;
  onOpenProviderSettings: () => void;
};

const PERM_ICONS: Record<Permission, typeof Shield> = {
  ask: ShieldAlert,
  auto: ShieldCheck,
  full: Shield,
};

const PERM_CONFIG: Record<Permission, { label: string; color: string }> = {
  ask: { label: "请求批准", color: "var(--kodo-accent)" },
  auto: { label: "自动批准安全操作", color: "var(--kodo-success-text)" },
  full: { label: "完全访问", color: "var(--kodo-warning-text)" },
};

const openSettings = () => navigate(isDesktop() ? "/settings" : "/ui-demo/settings");

/** Builtin agent skills mirrored from skills/<id>/SKILL.md (runtime SkillRegistry). */
const BUILTIN_SKILLS: { id: string; label: string }[] = [
  { id: "bug-fix", label: "缺陷修复" },
  { id: "feature", label: "功能开发" },
  { id: "test", label: "测试" },
  { id: "refactor", label: "重构" },
  { id: "code-review", label: "代码评审" },
  { id: "docs", label: "文档" },
  { id: "kodo-ui", label: "Kodo UI" },
];

export function Composer({
  provider,
  providers,
  onSelectProvider,
  ready,
  running,
  projectPath,
  contexts,
  onAddContext,
  onRemoveContext,
  onContextError,
  onSend,
  onStop,
  providerWarning,
  onOpenProviderSettings,
}: Props) {
  const [draft, setDraft] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const [permission, setPermission] = useState<Permission>(DEFAULT_PERMISSION);
  /** + opens a floating action menu anchored to the composer box. */
  const [plusOpen, setPlusOpen] = useState(false);
  /** Side panel next to the + menu (skills list). */
  const [plusPanel, setPlusPanel] = useState<"skills" | null>(null);
  /** Skills stay structured in the composer until send; they are not draft text. */
  const [selectedSkills, setSelectedSkills] = useState<string[]>([]);

  useEffect(() => {
    void setting(PERMISSION_SETTING).then((v) => {
      if (v === "ask" || v === "auto" || v === "full") setPermission(v);
    });
  }, []);

  const setPerm = (p: Permission) => {
    setPermission(p);
    void setSetting(PERMISSION_SETTING, p);
  };

  // Grow with the text up to the cap. Doing it in a layout effect keeps the
  // box from painting at its old height for a frame after every keystroke.
  useLayoutEffect(() => {
    const box = input.current;
    if (!box) return;
    box.style.height = "auto";
    box.style.height = `${Math.min(box.scrollHeight, MAX_HEIGHT)}px`;
  }, [draft]);

  const closePlus = useCallback(() => {
    setPlusOpen(false);
    setPlusPanel(null);
  }, []);

  const pickFile = async (path: string) => {
    if (contexts.includes(path)) return;
    const ok = await validateContextPath(projectPath, path).catch(() => false);
    if (!ok) {
      const isAbsolute = path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path);
      onContextError(
        isAbsolute ? `无法读取该文件（文件不存在或不是文本）：${path}` : `无法读取项目内路径：${path}`,
      );
      return;
    }
    onAddContext(path);
  };

  const applySkill = (skillId: string) => {
    setSelectedSkills((current) => (current.includes(skillId) ? current : [...current, skillId]));
    closePlus();
    requestAnimationFrame(() => input.current?.focus());
  };

  // Stable handle for async drop/paste/file-dialog handlers that must see the latest pickFile.
  const pickFileRef = useRef(pickFile);
  pickFileRef.current = pickFile;

  /** Absolute path → project-relative when inside the project; else null. */
  const toProjectRelative = (absolute: string): string | null => {
    if (!projectPath) return null;
    const root = projectPath.replace(/\/+$/, "");
    const path = absolute.replace(/\\/g, "/");
    if (path === root) return null;
    if (!path.startsWith(`${root}/`)) return null;
    return path.slice(root.length + 1);
  };

  const addExternalPaths = useCallback(
    async (paths: string[]) => {
      for (const raw of paths) {
        const absoluteOutside = raw.startsWith("/") || /^[A-Za-z]:[\\/]/.test(raw);
        const relative = toProjectRelative(raw);
        // Inside the project → project-relative; elsewhere keep the absolute path.
        const next = relative ?? (absoluteOutside ? raw.replace(/\\/g, "/") : raw);
        if (!next) {
          onContextError(`无法添加路径：${raw}`);
          continue;
        }
        await pickFileRef.current(next);
      }
    },
    [projectPath, onContextError],
  );

  /** "添加照片和文件" — OS dialog on desktop; hidden file input elsewhere. */
  const openAttachFiles = async () => {
    closePlus();
    if (isDesktop()) {
      const paths = await pickFiles();
      if (paths && paths.length > 0) await addExternalPaths(paths);
      return;
    }
    fileInput.current?.click();
  };

  const onFileInput = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(event.target.files ?? []);
    event.target.value = "";
    const paths = files
      .map((file) => (file as File & { path?: string }).path)
      .filter((path): path is string => Boolean(path));
    if (paths.length > 0) await addExternalPaths(paths);
  };

  // Native file drop (Tauri): full filesystem paths, independent of HTML5 DnD.
  useEffect(() => {
    if (!isDesktop() || !projectPath) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    // getCurrentWebview() can throw synchronously when __TAURI_INTERNALS__ is a stub
    // (browser tests) or window.webview is missing — only bind when it exists.
    void Promise.resolve()
      .then(() => getCurrentWebview())
      .then((webview) =>
        webview.onDragDropEvent((event) => {
          if (event.payload.type !== "drop") return;
          void addExternalPaths(event.payload.paths);
        }),
      )
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => {
        /* drag-drop unavailable — HTML5 handlers still cover the browser shell */
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [projectPath, addExternalPaths]);

  const addDataTransferPaths = async (transfer: DataTransfer | null) => {
    if (!transfer) return;
    const uris = transfer.getData("text/uri-list");
    const plain = transfer.getData("text/plain");
    const candidates = `${uris}\n${plain}`
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith("#"))
      .map((line) => line.replace(/^file:\/\//, ""));
    if (candidates.length > 0) await addExternalPaths(candidates);
  };

  const onComposerPaste = async (event: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(event.clipboardData?.files ?? []);
    if (files.length === 0) return;
    // Prefer filesystem paths when the shell exposed them on the File object.
    const withPath = files
      .map((file) => (file as File & { path?: string }).path)
      .filter((path): path is string => Boolean(path));
    if (withPath.length > 0) {
      event.preventDefault();
      await addExternalPaths(withPath);
      return;
    }
    const text = event.clipboardData.getData("text/plain").trim();
    if (text.startsWith("/") || text.startsWith("file://")) {
      event.preventDefault();
      await addExternalPaths([text.replace(/^file:\/\//, "")]);
    }
  };

  const onComposerDrop = async (event: React.DragEvent<HTMLDivElement>) => {
    if (!event.dataTransfer) return;
    const hasFiles = event.dataTransfer.types.includes("Files");
    const hasUri = event.dataTransfer.types.includes("text/uri-list");
    if (!hasFiles && !hasUri) return;
    event.preventDefault();
    event.stopPropagation();
    const withPath = Array.from(event.dataTransfer.files)
      .map((file) => (file as File & { path?: string }).path)
      .filter((path): path is string => Boolean(path));
    if (withPath.length > 0) {
      await addExternalPaths(withPath);
      return;
    }
    await addDataTransferPaths(event.dataTransfer);
  };

  const send = () => {
    const skillText = selectedSkills.map((skillId) => `【技能：${skillId}】`).join("\n");
    const text = [skillText, draft.trim()].filter(Boolean).join("\n");
    if (!text || !ready || running) return;
    setDraft("");
    setSelectedSkills([]);
    onSend(text, [...contexts]);
  };

  const permConfig = PERM_CONFIG[permission];
  const PermIcon = PERM_ICONS[permission];
  const modelLabel = provider?.model ?? "选择模型";

  return (
    <div className="composer">
      <div
        className="composer-box"
        onDragOver={(event) => {
          if (event.dataTransfer?.types.includes("Files") || event.dataTransfer?.types.includes("text/uri-list")) {
            event.preventDefault();
          }
        }}
        onDrop={(event) => void onComposerDrop(event)}
      >
        {selectedSkills.length > 0 && (
          <div className="composer-skills" data-testid="composer-skills" aria-label="已选择技能">
            {selectedSkills.map((skillId) => (
              <span key={skillId} className="chip composer-skill" data-skill-id={skillId}>
                <Puzzle size={13} strokeWidth={1.8} className="chip-icon" />
                <span className="composer-skill-label">技能</span>
                <code>{skillId}</code>
                <button
                  type="button"
                  className="chip-remove"
                  aria-label={`Remove skill ${skillId}`}
                  onClick={() => setSelectedSkills((current) => current.filter((id) => id !== skillId))}
                >
                  <X size={12} strokeWidth={2} />
                </button>
              </span>
            ))}
          </div>
        )}

        {/* Row 1: textarea */}
        <div className="composer-row">
          <textarea
            ref={input}
            className="composer-input"
            rows={1}
            placeholder="描述任务，输入/调用技能"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onPaste={(event) => void onComposerPaste(event)}
            onKeyDown={(event) => {
              // Enter sends, Shift+Enter breaks the line.
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                send();
              }
            }}
          />
        </div>

        {/* Pinned context chips — paths only, never file bodies. */}
        {contexts.length > 0 && (
          <div className="composer-contexts" data-testid="composer-contexts">
            {contexts.map((path) => (
              <span key={path} className="chip chip--context" data-context-path={path}>
                <FileText size={13} strokeWidth={1.8} className="chip-icon" />
                <span className="chip-label">{path}</span>
                <button
                  type="button"
                  className="chip-remove"
                  aria-label={`Remove context ${path}`}
                  onClick={() => onRemoveContext(path)}
                >
                  <X size={12} strokeWidth={2} />
                </button>
              </span>
            ))}
          </div>
        )}

        {/* Row 2: controls */}
        <div className="composer-controls">
          {/* Left side */}
          <div className="composer-left">
            <div
              className="menu-anchor composer-plus"
              onKeyDown={(event) => {
                if (plusOpen && event.key === "Escape") {
                  event.preventDefault();
                  event.stopPropagation();
                  closePlus();
                }
              }}
            >
              <button
                type="button"
                className="icon-btn"
                aria-label="Composer menu"
                aria-expanded={plusOpen}
                aria-haspopup="menu"
                aria-controls={plusOpen ? "composer-plus-menu" : undefined}
                title="添加照片和文件、技能或打开模型设置"
                onClick={() => {
                  if (plusOpen) closePlus();
                  else {
                    setPlusOpen(true);
                    setPlusPanel(null);
                  }
                }}
              >
                <Plus size={16} strokeWidth={1.7} />
              </button>

              {plusOpen && (
                <>
                  <div className="menu-backdrop" onClick={() => closePlus()} />
                  <div className="composer-plus-root">
                    <div id="composer-plus-menu" className="menu composer-plus-menu" role="menu">
                      <button
                        type="button"
                        role="menuitem"
                        className="menu-item"
                        data-testid="add-context-item"
                        title="从文件系统选择照片和文件"
                        onClick={() => void openAttachFiles()}
                      >
                        <Paperclip size={14} strokeWidth={1.8} />
                        <span>添加照片和文件</span>
                      </button>
                      <button
                        type="button"
                        role="menuitem"
                        className={`menu-item${plusPanel === "skills" ? " menu-item--active" : ""}`}
                        onClick={() => setPlusPanel((panel) => (panel === "skills" ? null : "skills"))}
                      >
                        <Puzzle size={14} strokeWidth={1.8} />
                        <span>技能</span>
                        <ChevronDown size={12} strokeWidth={2} className="menu-item-trail" />
                      </button>
                      <div className="menu-separator" />
                      <MenuItem
                        icon={<Settings size={14} strokeWidth={1.8} />}
                        label="配置自定义模型"
                        onSelect={() => {
                          closePlus();
                          openSettings();
                        }}
                      />
                    </div>

                    {plusPanel === "skills" && (
                      <div className="menu composer-plus-panel" role="listbox" aria-label="Skills">
                        {BUILTIN_SKILLS.map((skill) => (
                          <button
                            key={skill.id}
                            type="button"
                            role="option"
                            className="menu-item"
                            data-skill-id={skill.id}
                            onClick={() => applySkill(skill.id)}
                          >
                            <Puzzle size={14} strokeWidth={1.8} />
                            <span>{skill.label}</span>
                            <span className="menu-item-hint">{skill.id}</span>
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                </>
              )}
            </div>

            {/* Browser fallback when the Tauri dialog is unavailable. */}
            <input
              ref={fileInput}
              type="file"
              multiple
              data-testid="attach-file-input"
              className="composer-attach-input"
              aria-label="添加照片和文件"
              onChange={(event) => void onFileInput(event)}
            />

            {/* Permission dropdown */}
            <Menu
              trigger={({ open, toggle }) => (
                <button
                  type="button"
                  className="composer-perm-trigger"
                  aria-expanded={open}
                  onClick={toggle}
                  style={{ color: permConfig.color }}
                >
                  <PermIcon size={14} strokeWidth={2} />
                  <span>{permConfig.label}</span>
                  <ChevronDown size={12} strokeWidth={2} />
                </button>
              )}
            >
              {(close) =>
                PERMISSIONS.map(({ value, label, description }) => {
                  const cfg = PERM_CONFIG[value];
                  const Icon = PERM_ICONS[value];
                  return (
                    <MenuItem
                      key={value}
                      icon={<Icon size={14} strokeWidth={1.8} style={{ color: cfg.color }} />}
                      label={`${label} — ${description}`}
                      onSelect={() => {
                        close();
                        setPerm(value);
                      }}
                    />
                  );
                })
              }
            </Menu>
          </div>

          {/* Right side */}
          <div className="composer-right">
            {/* Model / provider picker */}
            <Menu
              trigger={({ open, toggle }) => (
                <button type="button" className="composer-model" aria-expanded={open} onClick={toggle}>
                  <span>{modelLabel}</span>
                  <ChevronDown size={13} strokeWidth={1.9} />
                </button>
              )}
            >
              {(close) => (
                <>
                  {providers.length === 0 ? (
                    <MenuItem
                      icon={<Plus size={14} strokeWidth={1.8} />}
                      label="添加 AI 服务商..."
                      onSelect={() => {
                        close();
                        openSettings();
                      }}
                    />
                  ) : (
                    providers.map((p, i) => {
                      const t = templateById(p.template);
                      return (
                        <MenuItem
                          key={p.id}
                          icon={<Sparkles size={14} strokeWidth={1.8} />}
                          label={p.name}
                          hint={
                            p.displayName ||
                            p.modelId ||
                            p.model ||
                            t.models[0]?.display_name ||
                            t.models[0]?.model_id ||
                            ""
                          }
                          onSelect={() => {
                            close();
                            onSelectProvider(i);
                          }}
                        />
                      );
                    })
                  )}
                  <div className="menu-separator" />
                  <MenuItem
                    icon={<Settings size={14} strokeWidth={1.8} />}
                    label="配置自定义模型"
                    onSelect={() => {
                      close();
                      openSettings();
                    }}
                  />
                </>
              )}
            </Menu>

            {running ? (
              <button type="button" className="composer-send composer-send--stop" aria-label="Stop" onClick={onStop}>
                <Square size={13} strokeWidth={2.4} />
              </button>
            ) : (
              <button
                type="button"
                className="composer-send"
                aria-label="Send"
                disabled={!ready || (draft.trim() === "" && selectedSkills.length === 0)}
                title={
                  !ready
                    ? "对话尚未就绪"
                    : draft.trim() === "" && selectedSkills.length === 0
                      ? "请输入消息或选择技能后再发送"
                      : "发送消息"
                }
                onClick={send}
              >
                <ArrowUp size={16} strokeWidth={2.2} />
              </button>
            )}
          </div>
        </div>
      </div>

      {providerWarning && (
        <div className="composer-alert" role="alert" data-testid="provider-warning">
          <span>{providerWarning}</span>
          <button type="button" className="btn btn--primary btn--sm" onClick={onOpenProviderSettings}>
            打开设置
          </button>
        </div>
      )}
    </div>
  );
}
