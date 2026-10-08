import {
  Archive,
  Box,
  Cloud,
  Database,
  FileText,
  Folder,
  GitBranch,
  Palette,
  Pencil,
  Plus,
  Search,
  Settings,
  SlidersHorizontal,
  SquareTerminal,
  Trash2,
  Wrench,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { DEFAULT_PERMISSION, MODELS } from "../data/models";
import { PERMISSIONS, PERMISSION_SETTING } from "../data/models";
import { T } from "../i18n";
import {
  PROVIDER_TEMPLATES,
  type ProviderConfig,
  templateById,
  loadProviders,
  saveProviders,
  uid,
  maskApiKey,
  migrateProviderConfig,
  providerModelLabel,
} from "../data/providers";
import { useSetting, useSettingBool } from "../data/useSetting";

function Toggle({ settingKey, fallback = true }: { settingKey: string; fallback?: boolean }) {
  const [on, setOn] = useSettingBool(settingKey, fallback);
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      className={`toggle${on ? " toggle--on" : ""}`}
      onClick={() => setOn(!on)}
    >
      <span className="toggle-knob" />
    </button>
  );
}

function Field({
  icon,
  label,
  hint,
  hintBelow,
  wide,
  block,
  children,
}: {
  icon?: ReactNode;
  label: string;
  hint?: string;
  hintBelow?: boolean;
  wide?: boolean;
  block?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={`setting-row${block ? " setting-row--stacked" : ""}`}>
      {icon && <span className="setting-icon">{icon}</span>}
      <div className="setting-text">
        <span className="setting-label">{label}</span>
        {hint && !hintBelow && !block && <span className="setting-hint">{hint}</span>}
      </div>
      <div className={`setting-control${wide ? " setting-control--wide" : ""}`}>
        {children}
        {hint && hintBelow && <span className="setting-hint">{hint}</span>}
      </div>
      {hint && block && <span className="setting-hint">{hint}</span>}
    </div>
  );
}

function SwitchRow({ label, hint, trailing, settingKey }: { label: string; hint: string; trailing?: boolean; settingKey: string }) {
  const text = (
    <div className="setting-text">
      <span className="setting-label">{label}</span>
      <span className="setting-hint">{hint}</span>
    </div>
  );
  return trailing ? (
    <div className="setting-row">
      {text}
      <div className="setting-control">
        <Toggle settingKey={settingKey} />
      </div>
    </div>
  ) : (
    <div className="setting-row">
      <Toggle settingKey={settingKey} />
      {text}
    </div>
  );
}

/** Seven categories; "AI Provider" stays English as a technical proper noun. */
const SECTIONS: { id: string; title: string; subtitle: string; icon: ReactNode }[] = [
  {
    id: "general",
    title: T.page.settings.categories.general,
    subtitle: T.page.settings.categoryHints.general,
    icon: <SlidersHorizontal size={17} strokeWidth={1.7} />,
  },
  {
    id: "models",
    title: T.page.settings.categories.models,
    subtitle: T.page.settings.categoryHints.models,
    icon: <Box size={17} strokeWidth={1.7} />,
  },
  {
    id: "provider",
    title: T.page.settings.categories.provider,
    subtitle: T.page.settings.categoryHints.provider,
    icon: <Cloud size={17} strokeWidth={1.7} />,
  },
  {
    id: "tools",
    title: T.page.settings.categories.tools,
    subtitle: T.page.settings.categoryHints.tools,
    icon: <Wrench size={17} strokeWidth={1.7} />,
  },
  {
    id: "projects",
    title: T.page.settings.categories.projects,
    subtitle: T.page.settings.categoryHints.projects,
    icon: <Folder size={17} strokeWidth={1.7} />,
  },
  {
    id: "storage",
    title: T.page.settings.categories.storage,
    subtitle: T.page.settings.categoryHints.storage,
    icon: <Database size={17} strokeWidth={1.7} />,
  },
  {
    id: "appearance",
    title: T.page.settings.categories.appearance,
    subtitle: T.page.settings.categoryHints.appearance,
    icon: <Palette size={17} strokeWidth={1.7} />,
  },
];

type Props = {
  model: string;
  onSelectModel: (model: string) => void;
  onProvidersSaved?: (providers: ProviderConfig[]) => void;
  onOpenArchive?: () => void;
  onClose?: () => void;
  modal?: boolean;
};

export function SettingsPage({ model, onSelectModel, onProvidersSaved, onOpenArchive, onClose, modal = false }: Props) {
  const [active, setActive] = useState(SECTIONS[0].id);
  const [query, setQuery] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);
  const section = SECTIONS.find((item) => item.id === active) ?? SECTIONS[0];

  const needle = query.trim().toLowerCase();
  const matches = (item: (typeof SECTIONS)[number]) =>
    item.title.toLowerCase().includes(needle) || item.subtitle.toLowerCase().includes(needle);
  const visibleSections = needle ? SECTIONS.filter(matches) : SECTIONS;
  const noMatch = needle.length > 0 && visibleSections.length === 0;

  // Keep the open category while it still matches; otherwise jump to the first
  // category that does, so the detail pane never shows a hidden section.
  useEffect(() => {
    if (!needle) return;
    const current = SECTIONS.find((item) => item.id === active);
    if (current && matches(current)) return;
    const first = SECTIONS.find(matches);
    if (first) setActive(first.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [needle, active]);

  // ⌘F / Ctrl+F focuses the page-head search while Settings is on screen.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <main className={`main${modal ? " settings-modal-panel" : ""}`}>
      <header className="page-head">
        <span className="page-head-mark">
          <Settings size={18} strokeWidth={1.7} />
        </span>
        <div className="page-head-text">
          <h1>{T.page.settings.title}</h1>
          <p>{T.page.settings.subtitle}</p>
        </div>
        <span className="spacer" />
        <div className="search-field search-field--inline">
          <Search size={15} strokeWidth={1.7} />
          <input
            ref={searchRef}
            placeholder={T.page.settings.searchPlaceholder}
            value={query}
            aria-label={T.page.settings.searchAria}
            onChange={(event) => setQuery(event.target.value)}
          />
          <kbd>⌘F</kbd>
        </div>
        {modal && onClose && (
          <button type="button" className="icon-btn" aria-label="关闭设置" onClick={onClose}>
            ×
          </button>
        )}
      </header>

      <div className="settings-layout">
        <nav className="settings-nav">
          {visibleSections.map(({ id, title, icon }) => (
            <button
              key={id}
              type="button"
              className={`settings-nav-item${id === active ? " settings-nav-item--active" : ""}`}
              aria-current={id === active ? "page" : undefined}
              onClick={() => setActive(id)}
            >
              {icon}
              <span>{title}</span>
            </button>
          ))}
          {noMatch && <p className="settings-nav-empty">{T.page.settings.noMatches}</p>}
        </nav>

        <div className="scroll">
          <div className="page-inner page-inner--wide">
            {noMatch ? (
              <section className="settings-detail">
                <p className="empty-note">{T.page.settings.noMatchesHint(query.trim())}</p>
              </section>
            ) : (
              <section className="settings-detail">
                <header className="settings-section-head">
                  <span className="settings-card-icon">{section.icon}</span>
                  <div>
                    <h2>{section.title}</h2>
                    <p>{section.subtitle}</p>
                  </div>
                </header>

                <div className="settings-card-body">
                  {body(active, model, onSelectModel, onProvidersSaved, onOpenArchive)}
                </div>
              </section>
            )}
          </div>
        </div>
      </div>
    </main>
  );
}

type ProviderEditMode = null | { mode: "add" } | { mode: "edit"; index: number };

function ProviderListSection({ onSaved }: { onSaved?: (providers: ProviderConfig[]) => void }) {
  const [providers, setProviders] = useState<ProviderConfig[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [edit, setEdit] = useState<ProviderEditMode>(null);

  useEffect(() => {
    void loadProviders().then((list) => {
      setProviders(list);
      setLoaded(true);
    });
  }, []);

  const update = async (next: ProviderConfig[]) => {
    setProviders(next);
    await saveProviders(next);
    const fresh = await loadProviders();
    setProviders(fresh);
    onSaved?.(fresh);
  };

  if (!loaded) return null;

  if (edit) {
    const initial = edit.mode === "edit" ? providers[edit.index] : undefined;
    return (
      <ProviderEditor
        initial={initial}
        onSave={(config) => {
          if (edit.mode === "add") {
            void update([...providers, config]);
          } else {
            const next = [...providers];
            next[edit.index] = config;
            void update(next);
          }
          setEdit(null);
        }}
        onCancel={() => setEdit(null)}
      />
    );
  }

  return (
    <>
      {providers.length === 0 && (
        <p className="settings-hint" style={{ marginBottom: 12 }}>
          还没有添加任何 AI 服务商。点击下方按钮添加第一个。
        </p>
      )}

      {providers.map((p, index) => {
        const t = templateById(p.template);
        return (
          <div key={p.id} className="provider-row">
            <div className="provider-row-info">
              <span className="provider-row-name">{p.name}</span>
              <span className="code-chip">{t.name}</span>
              {p.model && <span className="provider-row-model">{providerModelLabel(p) || p.model}</span>}
              <span className="provider-row-model">{maskApiKey(p.apiKey, p.hasKey)}</span>
            </div>
            <div className="provider-row-actions">
              <button type="button" className="icon-btn icon-btn--sm" aria-label={T.page.settings.rows.edit} onClick={() => setEdit({ mode: "edit", index })}>
                <Pencil size={14} strokeWidth={1.8} />
              </button>
              <button
                type="button"
                className="icon-btn icon-btn--sm icon-btn--danger"
                aria-label={T.page.settings.rows.delete}
                onClick={() => void update(providers.filter((_, i) => i !== index))}
              >
                <Trash2 size={14} strokeWidth={1.8} />
              </button>
            </div>
          </div>
        );
      })}

      <button type="button" className="btn" style={{ marginTop: 8 }} onClick={() => setEdit({ mode: "add" })}>
        <Plus size={15} strokeWidth={1.9} />
        <span>{T.page.settings.rows.addProvider}</span>
      </button>
    </>
  );
}

function ProviderEditor({
  initial,
  onSave,
  onCancel,
}: {
  initial?: ProviderConfig;
  onSave: (config: ProviderConfig) => void;
  onCancel: () => void;
}) {
  const [templateId, setTemplateId] = useState(initial?.template ?? PROVIDER_TEMPLATES[0].id);
  const [name, setName] = useState(initial?.name ?? "");
  const [apiKey, setApiKey] = useState(initial?.apiKey ?? "");
  const [endpoint, setEndpoint] = useState(initial?.endpoint ?? "");
  const [modelId, setModelId] = useState(initial?.modelId || initial?.model || "");
  const [displayName, setDisplayName] = useState(initial?.displayName || "");

  const tmpl = templateById(templateId);

  const save = () => {
    const selected = tmpl.models.find((m) => m.model_id === modelId);
    const migrated = migrateProviderConfig({
      id: initial?.id ?? uid(),
      name: name || tmpl.name,
      template: templateId,
      apiKey,
      endpoint,
      model: modelId || tmpl.models[0]?.model_id || "",
      modelId: modelId || tmpl.models[0]?.model_id || "",
      displayName: displayName || selected?.display_name || tmpl.models[0]?.display_name || "",
    });
    onSave(migrated);
  };

  return (
    <div className="provider-editor">
      <Field label={T.page.settings.rows.template} hintBelow wide>
        <select className="select" value={templateId} onChange={(e) => setTemplateId(e.target.value)}>
          {PROVIDER_TEMPLATES.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
      </Field>
      <Field label={T.page.settings.rows.displayName} wide>
        <input className="input" placeholder={tmpl.name} value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field
        label={T.page.settings.rows.apiKey}
        hint={
          initial?.hasKey
            ? "已保存密钥（界面只显示掩码）。留空或保持掩码表示不修改；输入新值则覆盖。"
            : "密钥仅写入 credentials.log，不会进入 settings.log，也不会回传明文。"
        }
        hintBelow
        wide
      >
        <input
          className="input"
          type="password"
          placeholder={initial?.apiKey || tmpl.keyPlaceholder}
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
        />
      </Field>
      {tmpl.customEndpoint && (
        <Field label={T.page.settings.rows.baseUrl} wide>
          <input className="input" placeholder="https://api.example.com/v1" value={endpoint} onChange={(e) => setEndpoint(e.target.value)} />
        </Field>
      )}
      <Field
        label={T.page.settings.rows.model}
        hint={tmpl.models.length > 0 ? "界面显示名称；后端发送 model_id" : "请直接填写 API model id，例如 gpt-4o"}
        hintBelow
        wide
      >
        {tmpl.models.length > 0 ? (
          <select
            className="select"
            value={modelId || tmpl.models[0].model_id}
            onChange={(e) => {
              const next = e.target.value;
              setModelId(next);
              const opt = tmpl.models.find((m) => m.model_id === next);
              if (opt) setDisplayName(opt.display_name);
            }}
          >
            {tmpl.models.map((m) => (
              <option key={m.model_id} value={m.model_id}>
                {m.display_name} — {m.model_id}
              </option>
            ))}
          </select>
        ) : (
          <input
            className="input"
            placeholder="model-id"
            value={modelId}
            onChange={(e) => {
              setModelId(e.target.value);
              setDisplayName(e.target.value);
            }}
          />
        )}
      </Field>
      <div className="provider-row-actions" style={{ marginTop: 8 }}>
        <button type="button" className="btn" onClick={onCancel}>
          {T.page.settings.rows.cancel}
        </button>
        <button type="button" className="btn btn--primary" onClick={save}>
          {T.page.settings.rows.save}
        </button>
      </div>
    </div>
  );
}

function ToolsBody() {
  // Same default as DEFAULT_PERMISSION / the composer trigger ("ask", Secure by Default).
  const [permission, setPermission] = useSetting(PERMISSION_SETTING, DEFAULT_PERMISSION);
  return (
    <>
      <Field
        icon={<SquareTerminal size={20} strokeWidth={1.6} />}
        label={T.page.settings.rows.permissionMode}
        hint="Shell、文件与 git 步骤共用此策略（写入 permission，由 run driver 读取）"
        hintBelow
        wide
      >
        <select
          className="select"
          value={permission}
          onChange={(e) => setPermission(e.target.value)}
          aria-label={T.page.settings.rows.permissionMode}
        >
          {PERMISSIONS.map((item) => (
            <option key={item.value} value={item.value}>
              {item.label} — {item.description}
            </option>
          ))}
        </select>
      </Field>
      <Field icon={<FileText size={20} strokeWidth={1.6} />} label={T.page.settings.rows.fileWrite} hint="由 Permission mode 统一约束（同一 runtime key）" hintBelow wide>
        <select
          className="select"
          value={permission}
          onChange={(e) => setPermission(e.target.value)}
          aria-label={T.page.settings.rows.fileWrite}
        >
          {PERMISSIONS.map((item) => (
            <option key={item.value} value={item.value}>
              {item.label}
            </option>
          ))}
        </select>
      </Field>
      <Field icon={<GitBranch size={20} strokeWidth={1.6} />} label={T.page.settings.rows.gitAccess} hint="由 Permission mode 统一约束（同一 runtime key）" hintBelow wide>
        <select
          className="select"
          value={permission}
          onChange={(e) => setPermission(e.target.value)}
          aria-label={T.page.settings.rows.gitAccess}
        >
          {PERMISSIONS.map((item) => (
            <option key={item.value} value={item.value}>
              {item.label}
            </option>
          ))}
        </select>
      </Field>
    </>
  );
}

function StorageBody({ onOpenArchive }: { onOpenArchive?: () => void }) {
  return (
    <>
      <Field
        label={T.page.settings.rows.localState}
        hint="项目、会话与设置写入 settings.log（append-only）；API Key 在 credentials.log"
        hintBelow
        block
      >
        <p className="settings-hint">
          macOS 目录：<code>~/Library/Application Support/Kodo</code>。不使用 SQLite；自动归档天数与导出将在后续版本接入。
        </p>
      </Field>
      {onOpenArchive && (
        <button type="button" className="btn" onClick={onOpenArchive}>
          <Archive size={15} strokeWidth={1.8} />
          <span>{T.page.settings.rows.viewArchive}</span>
        </button>
      )}
    </>
  );
}

function AppearanceBody() {
  const [theme, setTheme] = useSetting("theme", "light");
  const [density, setDensity] = useSetting("density", "compact");
  return (
    <>
      <Field
        label={T.page.settings.rows.theme}
        hint="写入 settings.log 并应用到 <html data-theme>；System 跟随系统深色偏好"
        hintBelow
        wide
      >
        <select className="select" value={theme} onChange={(e) => setTheme(e.target.value)} aria-label={T.page.settings.rows.theme}>
          <option value="light">{T.page.settings.options.light}</option>
          <option value="system">{T.page.settings.options.system}</option>
        </select>
      </Field>
      <Field label={T.page.settings.rows.density} hint="应用到 app 的 data-density" hintBelow wide>
        <select className="select" value={density} onChange={(e) => setDensity(e.target.value)}>
          <option value="compact">{T.page.settings.options.compact}</option>
          <option value="comfortable">{T.page.settings.options.comfortable}</option>
        </select>
      </Field>
      <SwitchRow label={T.page.settings.rows.lineNumbers} hint="应用到 data-line-numbers 属性" trailing settingKey="show-line-numbers" />
      <SwitchRow label={T.page.settings.rows.systemFont} hint="应用 app--system-font 类" trailing settingKey="use-system-font" />
    </>
  );
}

function ModelsBody({ model, onSelectModel }: { model: string; onSelectModel: (m: string) => void }) {
  return (
    <>
      <Field label={T.page.settings.rows.fallback} hint="所选模型不可用时" hintBelow wide>
        <FallbackSelect />
      </Field>
      <Field label={T.page.settings.rows.defaultModel} hint="用于新会话" hintBelow wide>
        <select className="select" value={model} onChange={(event) => onSelectModel(event.target.value)}>
          {MODELS.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
      </Field>
      <SwitchRow label={T.page.settings.rows.extendedThinking} hint="允许模型逐步思考" trailing settingKey="extended-thinking" />
      <Field label={T.page.settings.rows.maxOutputTokens} hint="新会话的默认值" hintBelow wide>
        <MaxTokens />
      </Field>
    </>
  );
}

function FallbackSelect() {
  const [value, setValue] = useSetting("fallback-behavior", "next");
  return (
    <select className="select" value={value} onChange={(e) => setValue(e.target.value)}>
      <option value="next">{T.page.settings.options.tryNext}</option>
      <option value="fail">{T.page.settings.options.stopError}</option>
    </select>
  );
}

function MaxTokens() {
  const [value, setValue] = useSetting("max-output-tokens", "4096");
  return <input className="input" value={value} onChange={(e) => setValue(e.target.value)} />;
}

function GeneralBody({ model, onSelectModel }: { model: string; onSelectModel: (m: string) => void }) {
  return (
    <>
      <Field label={T.page.settings.rows.defaultModel} hint="用于新会话" hintBelow wide>
        <select className="select" value={model} onChange={(event) => onSelectModel(event.target.value)}>
          {MODELS.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
      </Field>
      <Field label={T.page.settings.rows.fallback} hint="所选模型不可用时" hintBelow wide>
        <FallbackSelect />
      </Field>
    </>
  );
}

function body(
  active: string,
  model: string,
  onSelectModel: (m: string) => void,
  onProvidersSaved?: (providers: ProviderConfig[]) => void,
  onOpenArchive?: () => void,
) {
  switch (active) {
    case "models":
      return <ModelsBody model={model} onSelectModel={onSelectModel} />;
    case "provider":
      return <ProviderListSection onSaved={onProvidersSaved} />;
    case "tools":
      return <ToolsBody />;
    case "projects":
      return (
        <SwitchRow
          label={T.page.settings.rows.autoGitBranch}
          hint="在上下文中包含当前分支"
          trailing
          settingKey="auto-detect-git-branch"
        />
      );
    case "storage":
      return <StorageBody onOpenArchive={onOpenArchive} />;
    case "appearance":
      return <AppearanceBody />;
    default:
      return <GeneralBody model={model} onSelectModel={onSelectModel} />;
  }
}
