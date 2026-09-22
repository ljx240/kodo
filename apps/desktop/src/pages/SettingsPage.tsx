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
  Settings,
  SlidersHorizontal,
  SquareTerminal,
  Trash2,
  Wrench,
} from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { MODELS } from "../data/models";
import { PERMISSIONS, PERMISSION_SETTING } from "../data/models";
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

const SECTIONS: { id: string; title: string; subtitle: string; icon: ReactNode }[] = [
  {
    id: "general",
    title: "General",
    subtitle: "Basic behavior and defaults",
    icon: <SlidersHorizontal size={17} strokeWidth={1.7} />,
  },
  { id: "models", title: "Models", subtitle: "Model preferences and behavior", icon: <Box size={17} strokeWidth={1.7} /> },
  {
    id: "provider",
    title: "AI Provider",
    subtitle: "Configure API keys and endpoints",
    icon: <Cloud size={17} strokeWidth={1.7} />,
  },
  {
    id: "tools",
    title: "Tools & Permissions",
    subtitle: "Control what Kodo can do",
    icon: <Wrench size={17} strokeWidth={1.7} />,
  },
  {
    id: "projects",
    title: "Projects",
    subtitle: "Workspace and project behavior",
    icon: <Folder size={17} strokeWidth={1.7} />,
  },
  {
    id: "storage",
    title: "Archive & Storage",
    subtitle: "Conversation history and local data",
    icon: <Database size={17} strokeWidth={1.7} />,
  },
  { id: "appearance", title: "Appearance", subtitle: "Customize the look and feel", icon: <Palette size={17} strokeWidth={1.7} /> },
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
  const section = SECTIONS.find((item) => item.id === active) ?? SECTIONS[0];

  return (
    <main className={`main${modal ? " settings-modal-panel" : ""}`}>
      <header className="page-head">
        <span className="page-head-mark">
          <Settings size={18} strokeWidth={1.7} />
        </span>
        <div className="page-head-text">
          <h1>Settings</h1>
          <p>Customize Kodo to fit your workflow</p>
        </div>
        <span className="spacer" />
        {modal && onClose && (
          <button type="button" className="icon-btn" aria-label="关闭设置" onClick={onClose}>
            ×
          </button>
        )}
      </header>

      <div className="settings-layout">
        <nav className="settings-nav">
          {SECTIONS.map(({ id, title, icon }) => (
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
        </nav>

        <div className="scroll">
          <div className="page-inner page-inner--wide">
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
              <button type="button" className="icon-btn icon-btn--sm" aria-label="Edit" onClick={() => setEdit({ mode: "edit", index })}>
                <Pencil size={14} strokeWidth={1.8} />
              </button>
              <button
                type="button"
                className="icon-btn icon-btn--sm icon-btn--danger"
                aria-label="Delete"
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
        <span>Add provider</span>
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
      <Field label="Template" hintBelow wide>
        <select className="select" value={templateId} onChange={(e) => setTemplateId(e.target.value)}>
          {PROVIDER_TEMPLATES.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
      </Field>
      <Field label="Display name" wide>
        <input className="input" placeholder={tmpl.name} value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field
        label="API key"
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
        <Field label="Base URL" wide>
          <input className="input" placeholder="https://api.example.com/v1" value={endpoint} onChange={(e) => setEndpoint(e.target.value)} />
        </Field>
      )}
      <Field
        label="Model"
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
          Cancel
        </button>
        <button type="button" className="btn btn--primary" onClick={save}>
          Save
        </button>
      </div>
    </div>
  );
}

function ToolsBody() {
  const [permission, setPermission] = useSetting(PERMISSION_SETTING, "full");
  return (
    <>
      <Field
        icon={<SquareTerminal size={20} strokeWidth={1.6} />}
        label="Permission mode"
        hint="Shell、文件与 git 步骤共用此策略（写入 permission，由 run driver 读取）"
        hintBelow
        wide
      >
        <select
          className="select"
          value={permission}
          onChange={(e) => setPermission(e.target.value)}
          aria-label="Permission mode"
        >
          {PERMISSIONS.map((item) => (
            <option key={item.value} value={item.value}>
              {item.label} — {item.description}
            </option>
          ))}
        </select>
      </Field>
      <Field icon={<FileText size={20} strokeWidth={1.6} />} label="File write access" hint="由 Permission mode 统一约束（同一 runtime key）" hintBelow wide>
        <select
          className="select"
          value={permission}
          onChange={(e) => setPermission(e.target.value)}
          aria-label="File write access"
        >
          {PERMISSIONS.map((item) => (
            <option key={item.value} value={item.value}>
              {item.label}
            </option>
          ))}
        </select>
      </Field>
      <Field icon={<GitBranch size={20} strokeWidth={1.6} />} label="Git access" hint="由 Permission mode 统一约束（同一 runtime key）" hintBelow wide>
        <select
          className="select"
          value={permission}
          onChange={(e) => setPermission(e.target.value)}
          aria-label="Git access"
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
        label="Local state"
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
          <span>查看归档</span>
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
        label="Theme"
        hint="写入 settings.log 并应用到 <html data-theme>；System 跟随系统深色偏好"
        hintBelow
        wide
      >
        <select className="select" value={theme} onChange={(e) => setTheme(e.target.value)} aria-label="Theme">
          <option value="light">Light</option>
          <option value="system">System（跟随系统）</option>
        </select>
      </Field>
      <Field label="Interface density" hint="应用到 app 的 data-density" hintBelow wide>
        <select className="select" value={density} onChange={(e) => setDensity(e.target.value)}>
          <option value="compact">Compact</option>
          <option value="comfortable">Comfortable</option>
        </select>
      </Field>
      <SwitchRow label="Show line numbers" hint="应用到 data-line-numbers 属性" trailing settingKey="show-line-numbers" />
      <SwitchRow label="Use system font" hint="应用 app--system-font 类" trailing settingKey="use-system-font" />
    </>
  );
}

function ModelsBody({ model, onSelectModel }: { model: string; onSelectModel: (m: string) => void }) {
  return (
    <>
      <Field label="Fallback behavior" hint="所选模型不可用时" hintBelow wide>
        <FallbackSelect />
      </Field>
      <Field label="Default model" hint="Used for new conversations" hintBelow wide>
        <select className="select" value={model} onChange={(event) => onSelectModel(event.target.value)}>
          {MODELS.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
      </Field>
      <SwitchRow label="Extended thinking" hint="Allow models to think step by step" trailing settingKey="extended-thinking" />
      <Field label="Max output tokens" hint="Default for new conversations" hintBelow wide>
        <MaxTokens />
      </Field>
    </>
  );
}

function FallbackSelect() {
  const [value, setValue] = useSetting("fallback-behavior", "next");
  return (
    <select className="select" value={value} onChange={(e) => setValue(e.target.value)}>
      <option value="next">Try next best model</option>
      <option value="fail">Stop and show the error</option>
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
      <Field label="Default model" hint="Used for new conversations" hintBelow wide>
        <select className="select" value={model} onChange={(event) => onSelectModel(event.target.value)}>
          {MODELS.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
      </Field>
      <Field label="Fallback behavior" hint="If the selected model is unavailable" hintBelow wide>
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
          label="Auto-detect git branch"
          hint="Include current branch in context"
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
