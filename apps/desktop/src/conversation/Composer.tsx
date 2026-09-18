import { ArrowUp, ChevronDown, Plus, Sparkles, Square, Shield, ShieldCheck, ShieldAlert, Mic, Settings } from "lucide-react";
import { useLayoutEffect, useRef, useState, useEffect } from "react";
import { isDesktop, setSetting, setting } from "../api";
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
  onSend: (text: string) => void;
  onStop: () => void;
};

const PERM_ICONS: Record<Permission, typeof Shield> = {
  ask: ShieldAlert,
  auto: ShieldCheck,
  full: Shield,
};

const PERM_CONFIG: Record<Permission, { label: string; color: string }> = {
  ask: { label: "请求批准", color: "var(--kodo-accent)" },
  auto: { label: "自动批准安全操作", color: "var(--kodo-success)" },
  full: { label: "完全访问", color: "var(--kodo-warning, #d97706)" },
};

const openSettings = () => navigate(isDesktop() ? "/settings" : "/ui-demo/settings");

export function Composer({ provider, providers, onSelectProvider, ready, running, onSend, onStop }: Props) {
  const [draft, setDraft] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);
  const [permission, setPermission] = useState<Permission>(DEFAULT_PERMISSION);

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

  const send = () => {
    const text = draft.trim();
    if (!text || !ready || running) return;
    setDraft("");
    onSend(text);
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

        {/* Row 2: controls */}
        <div className="composer-controls">
          {/* Left side */}
          <div className="composer-left">
            <button type="button" className="icon-btn" aria-label="Add context">
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
                          hint={p.model || t.models[0] || ""}
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

            {/* Mic button (decorative) */}
            <button type="button" className="icon-btn" aria-label="Voice input">
              <Mic size={16} strokeWidth={1.7} />
            </button>

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
      </div>
    </div>
  );
}
