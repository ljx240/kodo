import { ArrowUp, ChevronDown, FileText, Plus, Search, Sparkles, Square, Shield, ShieldCheck, ShieldAlert, Settings, X } from "lucide-react";
import { useLayoutEffect, useRef, useState, useEffect, useCallback } from "react";
import { isDesktop, listProjectFiles, setSetting, setting, validateContextPath } from "../api";
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
  const [permission, setPermission] = useState<Permission>(DEFAULT_PERMISSION);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [fileQuery, setFileQuery] = useState("");
  const [fileList, setFileList] = useState<string[]>([]);
  const [listLoading, setListLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);

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

  const refreshFiles = useCallback(async () => {
    if (!projectPath) {
      setFileList([]);
      return;
    }
    setListLoading(true);
    setListError(null);
    try {
      const paths = await listProjectFiles(projectPath, fileQuery);
      setFileList(paths);
    } catch (error) {
      setFileList([]);
      const message = error instanceof Error ? error.message : String(error);
      setListError(message);
      onContextError(`无法列出项目文件：${message}`);
    } finally {
      setListLoading(false);
    }
  }, [projectPath, fileQuery, onContextError]);

  useEffect(() => {
    if (!pickerOpen) return;
    void refreshFiles();
  }, [pickerOpen, refreshFiles]);

  const pickFile = async (path: string) => {
    if (contexts.includes(path)) {
      setPickerOpen(false);
      return;
    }
    const ok = await validateContextPath(projectPath, path).catch(() => false);
    if (!ok) {
      onContextError(`无法读取项目外或无效路径：${path}`);
      setListError(`无效路径：${path}`);
      return;
    }
    onAddContext(path);
    setPickerOpen(false);
    setFileQuery("");
  };

  const send = () => {
    const text = draft.trim();
    if (!text || !ready || running) return;
    setDraft("");
    onSend(text, [...contexts]);
  };

  const permConfig = PERM_CONFIG[permission];
  const PermIcon = PERM_ICONS[permission];
  const modelLabel = provider?.model ?? "选择模型";

  return (
    <div className="composer">
      <div className="composer-box">
        {/* Row 1: textarea */}
        <div className="composer-row">
          <textarea
            ref={input}
            className="composer-input"
            rows={1}
            placeholder="描述任务，输入/调用技能"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
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
            <button
              type="button"
              className="icon-btn"
              aria-label="Add context"
              aria-expanded={pickerOpen}
              aria-haspopup="dialog"
              aria-controls={pickerOpen ? "context-picker" : undefined}
              disabled={!ready || !projectPath}
              onClick={() => setPickerOpen((open) => !open)}
            >
              <Plus size={16} strokeWidth={1.7} />
            </button>

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
                disabled={!ready || draft.trim() === ""}
                onClick={send}
              >
                <ArrowUp size={16} strokeWidth={2.2} />
              </button>
            )}
          </div>
        </div>

        {/* File picker for Add context */}
        {pickerOpen && (
          <div className="context-picker" id="context-picker" role="dialog" aria-label="Add context files">
            <div className="context-picker-search">
              <Search size={14} strokeWidth={1.8} />
              <input
                className="context-picker-input"
                placeholder="在项目内搜索文件…"
                value={fileQuery}
                autoFocus
                onChange={(event) => setFileQuery(event.target.value)}
              />
              <button type="button" className="icon-btn icon-btn--sm" aria-label="Close context picker" onClick={() => setPickerOpen(false)}>
                <X size={14} strokeWidth={2} />
              </button>
            </div>
            {listError && <p className="context-picker-error">{listError}</p>}
            <ul className="context-picker-list" data-testid="context-file-list">
              {listLoading && <li className="context-picker-empty">加载中…</li>}
              {!listLoading && fileList.length === 0 && !listError && (
                <li className="context-picker-empty">没有匹配的文件</li>
              )}
              {!listLoading &&
                fileList.slice(0, 40).map((path) => (
                  <li key={path}>
                    <button
                      type="button"
                      className="context-picker-item"
                      data-file-path={path}
                      onClick={() => void pickFile(path)}
                    >
                      <FileText size={13} strokeWidth={1.8} />
                      <span>{path}</span>
                    </button>
                  </li>
                ))}
            </ul>
          </div>
        )}
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
