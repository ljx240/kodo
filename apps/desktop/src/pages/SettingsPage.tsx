import {
  Archive,
  Box,
  Brain,
  Cloud,
  Database,
  Folder,
  Palette,
  Pencil,
  Plug,
  Plus,
  RotateCw,
  Scale,
  Search,
  Settings,
  ShieldCheck,
  SlidersHorizontal,
  SquareTerminal,
  Trash2,
  Wrench,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  listProviderModels,
  memoryDoctor,
  memoryStatus,
  setSetting,
  setting,
  type MemoryStatusDto,
} from "../api";
import { DEFAULT_PERMISSION, MODELS } from "../data/models";
import { PERMISSIONS, PERMISSION_SETTING } from "../data/models";
import { SANDBOXES, SANDBOX_SETTING, DEFAULT_SANDBOX } from "../data/models";
import { sandboxStatus, type SandboxStatus } from "../api";
import { T } from "../i18n";
import { Field, Switch } from "../shell/controls";
import {
  PROVIDER_TEMPLATES,
  type ModelOption,
  type ProviderConfig,
  templateById,
  dedupeModels,
  isSuitableForLong,
  loadProviders,
  saveProviders,
  uid,
  maskApiKey,
  migrateProviderConfig,
  providerModelLabel,
} from "../data/providers";
import { useSetting, useSettingBool } from "../data/useSetting";
import { DecisionSettingsBody } from "./DecisionSettings";

/** The shared switch markup: `Toggle` binds it to a settings key; `Field` and
 *  `Switch` live in `shell/controls` because the MCP page's editor uses the
 *  same settings-row geometry. */
function Toggle({ settingKey, label, fallback = true }: { settingKey: string; label: string; fallback?: boolean }) {
  const [on, setOn] = useSettingBool(settingKey, fallback);
  return <Switch checked={on} label={label} onChange={() => setOn(!on)} />;
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
        <Toggle settingKey={settingKey} label={label} />
      </div>
    </div>
  ) : (
    <div className="setting-row">
      <Toggle settingKey={settingKey} label={label} />
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
    id: "memory",
    title: T.memory.section,
    subtitle: T.memory.description,
    icon: <Brain size={17} strokeWidth={1.7} />,
  },
  {
    id: "appearance",
    title: T.page.settings.categories.appearance,
    subtitle: T.page.settings.categoryHints.appearance,
    icon: <Palette size={17} strokeWidth={1.7} />,
  },
  {
    id: "decision",
    title: T.decision.section,
    subtitle: T.decision.sectionHint,
    icon: <Scale size={17} strokeWidth={1.7} />,
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
          <button type="button" className="icon-btn" aria-label={T.page.settings.rows.closeAria} onClick={onClose}>
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
              <section className="settings-detail" data-category={active}>
                <p className="empty-note">{T.page.settings.noMatchesHint(query.trim())}</p>
              </section>
            ) : (
              <section className="settings-detail" data-category={active}>
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
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  /** Row-level connectivity verdict — which row it belongs to, and the text. */
  const [probe, setProbe] = useState<{
    id: string;
    status: "loading" | "ok" | "fail";
    text: string;
  } | null>(null);

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

  // The editor's own `GET /models` probe, fired straight from the row — a
  // credentials check that never opens the editor. Masked keys resolve to the
  // stored secret Rust-side (same as the editor's refresh).
  const runTest = (p: ProviderConfig) => {
    const hasUsableKey = Boolean(p.hasKey || (p.apiKey && !p.apiKey.includes("•")));
    if (!hasUsableKey) {
      setProbe({ id: p.id, status: "fail", text: T.page.settings.provider.noKey });
      return;
    }
    if (p.template === "custom" && !(p.endpoint ?? "").trim()) {
      setProbe({ id: p.id, status: "fail", text: T.page.settings.provider.endpointNeeded });
      return;
    }
    const id = p.id;
    setProbe({ id, status: "loading", text: "" });
    listProviderModels(p.template, p.endpoint ?? "", p.apiKey ?? "", p.id)
      .then((list) => {
        setProbe({ id, status: "ok", text: T.page.settings.provider.testOk(list.length) });
      })
      .catch((error: unknown) => {
        setProbe({ id, status: "fail", text: T.page.settings.provider.testFail(String(error)) });
      });
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
          {T.page.settings.provider.emptyList}
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
              {probe?.id === p.id && probe.status !== "loading" && (
                <span
                  className="provider-row-model"
                  data-testid="provider-test-result"
                  data-status={probe.status}
                >
                  {probe.text}
                </span>
              )}
            </div>
            <div className="provider-row-actions">
              <button
                type="button"
                className="icon-btn icon-btn--sm"
                aria-label={`${p.name} ${T.page.settings.rows.edit}`}
                title={`${p.name} ${T.page.settings.rows.edit}`}
                onClick={() => {
                  setConfirmDelete(null);
                  setEdit({ mode: "edit", index });
                }}
              >
                <Pencil size={14} strokeWidth={1.8} />
              </button>
              <button
                type="button"
                className={`icon-btn icon-btn--sm icon-btn--danger${confirmDelete === p.id ? " icon-btn--armed" : ""}`}
                aria-label={`${p.name} ${confirmDelete === p.id ? T.skill.deleteConfirm : T.page.settings.rows.delete}`}
                title={`${p.name} ${confirmDelete === p.id ? T.skill.deleteConfirm : T.page.settings.rows.delete}`}
                onClick={() => {
                  if (confirmDelete !== p.id) {
                    setConfirmDelete(p.id);
                    return;
                  }
                  setConfirmDelete(null);
                  void update(providers.filter((_, i) => i !== index));
                }}
              >
                {confirmDelete === p.id ? T.skill.deleteConfirm : <Trash2 size={14} strokeWidth={1.8} />}
              </button>
              <button
                type="button"
                className="icon-btn icon-btn--sm"
                aria-label={`${p.name} ${T.page.settings.provider.test}`}
                title={`${p.name} ${T.page.settings.provider.test}`}
                data-testid="provider-test"
                disabled={probe?.id === p.id && probe.status === "loading"}
                onClick={() => runTest(p)}
              >
                <Plug size={14} strokeWidth={1.8} />
              </button>
            </div>
          </div>
        );
      })}

      <button type="button" className="btn provider-add" onClick={() => { setConfirmDelete(null); setEdit({ mode: "add" }); }}>
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
  const [models, setModels] = useState<ModelOption[]>(
    () => initial?.models ?? [...templateById(initial?.template ?? PROVIDER_TEMPLATES[0].id).models],
  );
  const [fetched, setFetched] = useState<ModelOption[]>([]);
  const [fetchState, setFetchState] = useState<"idle" | "loading" | "error">("idle");
  const [fetchError, setFetchError] = useState("");
  const [pending, setPending] = useState("");
  const fetchSeq = useRef(0);

  const tmpl = templateById(templateId);
  // Raw typed key (no mask) or an already-stored secret can fetch; a fresh
  // provider without a key never leaves the editor for a network call.
  const hasUsableKey = Boolean(
    (apiKey && !apiKey.includes("•")) || initial?.hasKey,
  );
  const canFetch = hasUsableKey && (templateId !== "custom" || endpoint.trim() !== "");

  const runFetch = (seq: number) => {
    setFetchState("loading");
    listProviderModels(templateId, endpoint, apiKey, initial?.id)
      .then((list) => {
        if (seq !== fetchSeq.current) return;
        const mapped: ModelOption[] = list.map((m) => ({
          display_name: m.displayName,
          model_id: m.modelId,
        }));
        setFetched(mapped);
        // Replace the list with the live one; the selected model always stays
        // so the pointer can never dangle after a refresh.
        setModels((prev) => {
          const selected = prev.find(
            (m) => m.model_id.toLowerCase() === modelId.toLowerCase(),
          );
          return dedupeModels([...mapped, ...(selected ? [selected] : [])]);
        });
        setFetchState("idle");
        setFetchError("");
      })
      .catch((error: unknown) => {
        if (seq !== fetchSeq.current) return;
        setFetchState("error");
        setFetchError(T.page.settings.provider.fetchFailed(String(error)));
      });
  };

  // Auto-fetch: debounced on editor inputs, dropped when the key/endpoint is
  // not ready, and late responses are ignored after a template switch.
  useEffect(() => {
    if (!canFetch) return;
    const seq = ++fetchSeq.current;
    const timer = window.setTimeout(() => runFetch(seq), 400);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [templateId, endpoint, apiKey, initial?.hasKey]);

  const refresh = () => {
    if (!hasUsableKey) {
      setFetchState("error");
      setFetchError(T.page.settings.provider.noKey);
      return;
    }
    if (templateId === "custom" && !endpoint.trim()) {
      setFetchState("error");
      setFetchError(T.page.settings.provider.endpointNeeded);
      return;
    }
    const seq = ++fetchSeq.current;
    runFetch(seq);
  };

  const selectModel = (option: ModelOption) => {
    setModelId(option.model_id);
    setDisplayName(option.display_name);
  };

  const removeModel = (option: ModelOption) => {
    setModels((prev) => {
      const next = prev.filter((m) => m.model_id !== option.model_id);
      if (option.model_id.toLowerCase() === modelId.toLowerCase()) {
        const successor = next[0];
        setModelId(successor?.model_id ?? "");
        setDisplayName(successor?.display_name ?? "");
      }
      return next;
    });
  };

  const addModel = (option: ModelOption) => {
    const id = option.model_id.trim();
    if (!id) return;
    // Pick a sensible default `vision` based on the current template so a
    // freshly added model doesn't regress to "unknown". Modern OpenAI-
    // compatible endpoints (custom / openai_compatible / openai) accept
    // image inputs by default; deepseek / text_only stay text-only;
    // unknown templates opt in (undefined) — the provider editor still
    // lets the user toggle this after the fact.
    const template = (initial?.template ?? PROVIDER_TEMPLATES[0].id).toLowerCase();
    const defaultVision: boolean | undefined = (() => {
      switch (template) {
        case "openai":
        case "openai_compatible":
        case "custom":
          return true;
        case "deepseek":
        case "text_only":
        case "conservative":
          return false;
        default:
          return undefined;
      }
    })();
    const incomingVision = option.vision;
    const entry: ModelOption = {
      display_name: option.display_name.trim() || id,
      model_id: id,
      vision: incomingVision ?? defaultVision,
    };
    setModels((prev) => dedupeModels([...prev, entry]));
    selectModel(entry);
  };

  const available = dedupeModels([...tmpl.models, ...fetched]).filter(
    (m) => !models.some((have) => have.model_id.toLowerCase() === m.model_id.toLowerCase()),
  );

  const onTemplateChange = (nextId: string) => {
    setTemplateId(nextId);
    const nextTmpl = templateById(nextId);
    const selected = models.find((m) => m.model_id.toLowerCase() === modelId.toLowerCase());
    setModels(dedupeModels([...nextTmpl.models, ...(selected ? [selected] : [])]));
    setFetched([]);
    setFetchState("idle");
    setFetchError("");
  };

  const save = () => {
    const selected = models.find((m) => m.model_id.toLowerCase() === modelId.toLowerCase());
    const migrated = migrateProviderConfig({
      id: initial?.id ?? uid(),
      name: name || tmpl.name,
      template: templateId,
      apiKey,
      endpoint,
      model: modelId || models[0]?.model_id || "",
      modelId: modelId || models[0]?.model_id || "",
      displayName: displayName || selected?.display_name || models[0]?.display_name || "",
      models,
    });
    onSave(migrated);
  };

  return (
    <div className="provider-editor">
      <Field label={T.page.settings.rows.template} hintBelow wide>
        <select className="select" aria-label={T.page.settings.rows.template} value={templateId} onChange={(e) => onTemplateChange(e.target.value)}>
          {PROVIDER_TEMPLATES.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
      </Field>
      <Field label={T.page.settings.rows.displayName} wide>
        <input className="input" aria-label={T.page.settings.rows.displayName} placeholder={tmpl.name} value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field
        label={T.page.settings.rows.apiKey}
        hint={initial?.hasKey ? T.page.settings.provider.keyHintSaved : T.page.settings.provider.keyHintNew}
        hintBelow
        wide
      >
        <input
          className="input"
          type="password"
          aria-label={T.page.settings.rows.apiKey}
          autoComplete="new-password"
          placeholder={initial?.apiKey || tmpl.keyPlaceholder}
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
        />
      </Field>
      {tmpl.customEndpoint && (
        <Field label={T.page.settings.rows.baseUrl} wide>
          <input className="input" aria-label={T.page.settings.rows.baseUrl} spellCheck={false} placeholder="https://api.example.com/v1" value={endpoint} onChange={(e) => setEndpoint(e.target.value)} />
        </Field>
      )}
      {/* Stacked like the storage rows: a fetched list needs the full pane
          width, not the 215px control column. */}
      <Field label={T.page.settings.rows.model} block>
        <div className="model-editor">
          <div className="model-chips">
            {models.map((m) => (
              <span
                key={m.model_id}
                className={`model-chip${m.model_id.toLowerCase() === modelId.toLowerCase() ? " model-chip--active" : ""}`}
              >
                <button type="button" className="model-chip-label" aria-pressed={m.model_id.toLowerCase() === modelId.toLowerCase()} title={`${m.display_name} · ${m.model_id}`} onClick={() => selectModel(m)}>
                  {/* Fetched lists usually repeat the id as the name — render one span. */}
                  {m.display_name.toLowerCase() === m.model_id.toLowerCase() ? (
                    <span className="model-chip-id">{m.model_id}</span>
                  ) : (
                    <>
                      <span>{m.display_name}</span>
                      <span className="model-chip-id">{m.model_id}</span>
                    </>
                  )}
                </button>
                <button
                  type="button"
                  className="model-chip-remove"
                  aria-label={T.page.settings.provider.remove(m.display_name)}
                  onClick={() => removeModel(m)}
                >
                  ×
                </button>
              </span>
            ))}
          </div>
          <div className="model-toolbar">
            {/* Only render the picker when something is actually addable —
                an empty select is a dead control. */}
            {available.length > 0 && (
              <select className="select" value="" onChange={(e) => {
                const id = e.target.value;
                if (!id) return;
                const option = available.find((m) => m.model_id === id);
                if (option) addModel(option);
              }}>
                <option value="">{T.page.settings.provider.chooseFromList}</option>
                {available.map((m) => (
                  <option key={m.model_id} value={m.model_id}>
                    {m.display_name} — {m.model_id}
                  </option>
                ))}
              </select>
            )}
            <input
              className="input"
              placeholder={T.page.settings.provider.addPlaceholder}
              aria-label={T.page.settings.provider.addPlaceholder}
              value={pending}
              onChange={(e) => setPending(e.target.value)}
              onKeyDown={(e) => {
                if (e.key !== "Enter") return;
                e.preventDefault();
                addModel({ display_name: pending, model_id: pending });
                setPending("");
              }}
            />
            <button
              type="button"
              className="btn"
              disabled={!pending.trim()}
              onClick={() => {
                addModel({ display_name: pending, model_id: pending });
                setPending("");
              }}
            >
              {T.page.settings.provider.add}
            </button>
            <button type="button" className="icon-btn" aria-label={T.page.settings.provider.refresh} onClick={refresh}>
              <RotateCw size={14} strokeWidth={1.8} />
            </button>
          </div>
          {/* Long-final task warning (2026-10-13): preview / flash / latest
              channels are known to land with an empty `text` block on
              tasks that need a long final report. The warning is rendered
              for the currently selected model only — switching the active
              chip makes it disappear without disturbing the rest of the
              model list. The renderer reads `suitableForLong` straight off
              the chip entry (Tauri pre-fills it from the agent-side
              suffix sniffer) and falls back to the local sniffer in
              `data/providers.ts` so catalog rows that have not been
              refreshed still get the warning. */}
          {(() => {
            const selected = models.find((m) => m.model_id.toLowerCase() === modelId.toLowerCase());
            if (!selected) return null;
            if (isSuitableForLong(selected)) return null;
            return (
              <p className="model-status model-status--warning" role="note">
                {T.page.settings.rows.previewChannelWarning}
              </p>
            );
          })()}
          <p
            className={`model-status${fetchState === "error" ? " model-status--error" : ""}`}
            role={fetchState === "error" ? "alert" : undefined}
          >
            {fetchState === "loading"
              ? T.page.settings.provider.loading
              : fetchState === "error"
                ? fetchError
                : models.length > 0
                  ? T.page.settings.provider.modelHintCatalog
                  : T.page.settings.provider.modelHintFree}
          </p>
        </div>
      </Field>
      {/* No extra margin: the editor's column gap already spaces this row. */}
      <div className="provider-editor-actions">
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
  // Sandbox mode is orthogonal to the permission mode: Secure by Default is
  // `workspace-write`, the same default the CLI resolves.
  const [sandbox, setSandbox] = useSetting(SANDBOX_SETTING, DEFAULT_SANDBOX);
  const [status, setStatus] = useState<SandboxStatus | null>(null);

  useEffect(() => {
    let live = true;
    sandboxStatus()
      .then((next) => {
        if (live) setStatus(next);
      })
      .catch(() => {
        if (live) setStatus(null);
      });
    return () => {
      live = false;
    };
  }, []);

  return (
    <>
      <Field
        icon={<SquareTerminal size={20} strokeWidth={1.6} />}
        label={T.page.settings.rows.permissionMode}
        hint={T.page.settings.rows.permissionHint}
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
      <Field
        icon={<ShieldCheck size={20} strokeWidth={1.6} />}
        label={T.page.settings.rows.sandboxMode}
        hint={T.page.settings.rows.sandboxHint}
        hintBelow
        wide
      >
        <select
          className="select"
          value={sandbox}
          onChange={(e) => setSandbox(e.target.value)}
          aria-label={T.page.settings.rows.sandboxMode}
        >
          {SANDBOXES.map((item) => (
            <option key={item.value} value={item.value}>
              {item.label} — {item.description}
            </option>
          ))}
        </select>
      </Field>
      {status && <SandboxStatusRow status={status} />}
    </>
  );
}

/**
 * The measured enforcement of this host.
 *
 * Every level comes from the backend's functional probe, so the row cannot
 * claim an isolation the machine does not actually provide.
 */
function SandboxStatusRow({ status }: { status: SandboxStatus }) {
  const level = (raw: string) => {
    if (raw === "full") return T.page.settings.rows.sandboxLevelFull;
    if (raw === "partial") return T.page.settings.rows.sandboxLevelPartial;
    return T.page.settings.rows.sandboxLevelUnsupported;
  };
  return (
    <Field
      label={T.page.settings.rows.sandboxStatusLabel}
      hint={`${status.backend}`}
      hintBelow
      wide
    >
      {!status.available && (
        <p className="settings-hint" role="note">
          {T.page.settings.rows.sandboxUnavailable}
        </p>
      )}
      <p className="settings-hint">
        {T.page.settings.rows.sandboxReadonly}：{level(status.fileRead)} ·{" "}
        {T.page.settings.rows.sandboxWrite}：{level(status.fileWrite)} ·{" "}
        {T.page.settings.rows.sandboxNetwork}：{level(status.networkEgress)}
      </p>
      {status.notes.map((note) => (
        <p key={note} className="settings-hint" role="note">
          {note}
        </p>
      ))}
    </Field>
  );
}

function StorageBody({ onOpenArchive }: { onOpenArchive?: () => void }) {
  return (
    <>
      <Field
        label={T.page.settings.rows.localState}
        hint={T.page.settings.rows.localStateHint}
        hintBelow
        block
      >
        <p className="settings-hint">
          {T.page.settings.rows.storageDirLabel}
          <code>~/Library/Application Support/Kodo</code>
          {T.page.settings.rows.storageNoteTail}
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
        hint={T.page.settings.rows.themeHint}
        hintBelow
        wide
      >
        <select className="select" value={theme} onChange={(e) => setTheme(e.target.value)} aria-label={T.page.settings.rows.theme}>
          <option value="light">{T.page.settings.options.light}</option>
          <option value="dark">深色</option>
          <option value="system">{T.page.settings.options.system}</option>
        </select>
      </Field>
      <Field label={T.page.settings.rows.density} hint={T.page.settings.rows.densityHint} hintBelow wide>
        <select className="select" value={density} onChange={(e) => setDensity(e.target.value)} aria-label={T.page.settings.rows.density}>
          <option value="compact">{T.page.settings.options.compact}</option>
          <option value="comfortable">{T.page.settings.options.comfortable}</option>
        </select>
      </Field>
      <SwitchRow label={T.page.settings.rows.lineNumbers} hint={T.page.settings.rows.lineNumbersHint} trailing settingKey="show-line-numbers" />
      <SwitchRow label={T.page.settings.rows.systemFont} hint={T.page.settings.rows.systemFontHint} trailing settingKey="use-system-font" />
    </>
  );
}

/** Default-model control fed by the configured providers' model lists so the
 *  control and the composer chip always name a model the backend can send.
 *  With no providers yet (fresh install / demo route) it keeps the built-in
 *  defaults — the control still writes `default-model`, a live runtime key. */
function DefaultModelSelect({
  model,
  onSelectModel,
}: {
  model: string;
  onSelectModel: (m: string) => void;
}) {
  const [providers, setProviders] = useState<ProviderConfig[]>([]);

  useEffect(() => {
    void loadProviders().then(setProviders);
  }, []);

  const known =
    providers.length === 0
      ? MODELS.some((name) => name.toLowerCase() === model.toLowerCase())
      : providers.some((p) =>
          p.models?.some(
            (m) =>
              m.model_id.toLowerCase() === model.toLowerCase() ||
              m.display_name.toLowerCase() === model.toLowerCase(),
          ),
        );

  return (
    <select
      className="select"
      value={model}
      aria-label={T.page.settings.rows.defaultModel}
      onChange={(event) => onSelectModel(event.target.value)}
    >
      {providers.length === 0
        ? MODELS.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))
        : providers.map((p) => (
            <optgroup key={p.id} label={p.name}>
              {(p.models ?? []).map((m) => (
                <option key={m.model_id} value={m.model_id}>
                  {m.display_name} — {m.model_id}
                </option>
              ))}
            </optgroup>
          ))}
      {!known && model && <option value={model}>{model}</option>}
    </select>
  );
}

function ModelsBody({ model, onSelectModel }: { model: string; onSelectModel: (m: string) => void }) {
  return (
    <>
      <Field label={T.page.settings.rows.fallback} hint={T.page.settings.rows.fallbackHint} wide>
        <FallbackSelect />
      </Field>
      <Field label={T.page.settings.rows.defaultModel} hint={T.page.settings.models.defaultModelHint} wide>
        <DefaultModelSelect model={model} onSelectModel={onSelectModel} />
      </Field>
      <SwitchRow
        label={T.page.settings.rows.extendedThinking}
        hint={T.page.settings.rows.extendedThinkingHint}
        trailing
        settingKey="extended-thinking"
      />
      <SwitchRow
        label="本地详细轨迹"
        hint="默认关闭。启用后，提示词、工具参数及文件正文经脱敏仅保存到本机；不保存私有思考正文。每轮上限 64 MiB / 200 个文件。"
        trailing
        settingKey="local-detailed-traces"
      />
      <Field label={T.page.settings.rows.maxOutputTokens} hint={T.page.settings.rows.maxTokensHint} wide>
        <MaxTokens />
      </Field>
      <Field label={T.page.settings.rows.contextWindow} hint={T.page.settings.rows.contextWindowHint} wide>
        <ContextWindow />
      </Field>
    </>
  );
}

function FallbackSelect() {
  const [value, setValue] = useSetting("fallback-behavior", "next");
  return (
    <select className="select" value={value} onChange={(e) => setValue(e.target.value)} aria-label={T.page.settings.rows.fallback}>
      <option value="next">{T.page.settings.options.tryNext}</option>
      <option value="fail">{T.page.settings.options.stopError}</option>
    </select>
  );
}

function MaxTokens() {
  const [value, setValue] = useSetting("max-output-tokens", "4096");
  return <input className="input" inputMode="numeric" aria-label={T.page.settings.rows.maxOutputTokens} value={value} onChange={(e) => setValue(e.target.value)} />;
}

function ContextWindow() {
  const [value, setValue] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const edited = useRef(false);

  useEffect(() => {
    let alive = true;
    void setting("context-window-tokens").then((stored) => {
      if (!alive) return;
      setValue(stored ?? "");
      setLoaded(true);
    });
    return () => { alive = false; };
  }, []);

  const save = async () => {
    if (!edited.current) return;
    const raw = value.trim();
    if (raw && (!/^\d+$/.test(raw) || Number(raw) < 1 || Number(raw) > 4_294_967_295)) {
      setError(T.page.settings.rows.contextWindowInvalid);
      return;
    }
    setSaving(true);
    try {
      await setSetting("context-window-tokens", raw);
      edited.current = false;
      setValue(raw);
      setError("");
    } catch {
      setError(T.page.settings.rows.contextWindowSaveFailed);
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <input
        className="input"
        inputMode="numeric"
        aria-label={T.page.settings.rows.contextWindow}
        aria-invalid={Boolean(error)}
        aria-describedby={error ? "context-window-error" : undefined}
        placeholder={T.page.settings.rows.contextWindowAuto}
        value={value}
        disabled={!loaded || saving}
        onChange={(event) => { edited.current = true; setValue(event.target.value); setError(""); }}
        onBlur={() => { void save(); }}
        onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }}
      />
      {error && <span id="context-window-error" className="setting-hint" role="alert">{error}</span>}
    </>
  );
}

/** Memory Engine surface: status snapshot, run doctor / reindex. Read-only
 *  for now — toggles live in their own settings keys, not in this pane. */
function MemoryBody() {
  const [status, setStatus] = useState<MemoryStatusDto | null>(null);
  const [busy, setBusy] = useState<null | "doctor" | "reindex">(null);
  const [report, setReport] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    memoryStatus()
      .then((s) => { if (!cancelled) setStatus(s); })
      .catch(() => { if (!cancelled) setStatus(null); });
    return () => { cancelled = true; };
  }, []);

  const runDoctor = async (apply: boolean) => {
    setBusy(apply ? "reindex" : "doctor");
    try {
      const result = await memoryDoctor(apply);
      setReport(
        `${T.memory.doctorDone} · ok=${result.ok} · ${T.memory.outbox(result.outboxPending)} · ${T.memory.tombstones(result.tombstones)}`,
      );
      const fresh = await memoryStatus();
      setStatus(fresh);
    } catch (error) {
      setReport(String(error));
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <div className="setting-row">
        <span className="setting-label">{T.memory.storage}</span>
        <span className="setting-hint" style={{ fontFamily: "var(--font-mono, monospace)" }}>
          {status?.path ?? "—"}
        </span>
      </div>
      <div className="setting-row">
        <span className="setting-label">{T.memory.entries(
          Object.values(status?.rowCounts ?? {}).reduce((a, b) => a + b, 0),
        )}</span>
        <span className="setting-hint">
          {T.memory.outbox(status?.outboxPending ?? 0)} · {T.memory.size(status?.dbSizeBytes ?? 0)}
        </span>
      </div>
      <div className="setting-row">
        <button
          className="btn"
          onClick={() => void runDoctor(false)}
          disabled={busy !== null}
        >
          {busy === "doctor" ? "…" : T.memory.runDoctor}
        </button>
        <button
          className="btn"
          onClick={() => void runDoctor(true)}
          disabled={busy !== null}
        >
          {busy === "reindex" ? "…" : T.memory.reindex}
        </button>
      </div>
      {report && <p className="setting-hint">{report}</p>}
      <p className="setting-hint">{T.memory.description}</p>
    </>
  );
}

function GeneralBody({ model, onSelectModel }: { model: string; onSelectModel: (m: string) => void }) {
  return (
    <>
      <Field label={T.page.settings.rows.defaultModel} hint={T.page.settings.models.defaultModelHint} wide>
        <DefaultModelSelect model={model} onSelectModel={onSelectModel} />
      </Field>
      <Field label={T.page.settings.rows.fallback} hint={T.page.settings.rows.fallbackHint} wide>
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
          hint={T.page.settings.rows.projectsHint}
          trailing
          settingKey="auto-detect-git-branch"
        />
      );
    case "storage":
      return <StorageBody onOpenArchive={onOpenArchive} />;
    case "memory":
      return <MemoryBody />;
    case "appearance":
      return <AppearanceBody />;
    case "decision":
      return <DecisionSettingsBody />;
    default:
      return <GeneralBody model={model} onSelectModel={onSelectModel} />;
  }
}
