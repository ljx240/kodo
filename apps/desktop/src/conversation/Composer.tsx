import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  ArrowUp,
  Bot,
  Check,
  ChevronDown,
  FolderSearch,
  Paperclip,
  Plus,
  Puzzle,
  Search,
  Settings,
  Sparkles,
  Square,
  Shield,
  ShieldCheck,
  ShieldAlert,
  X,
} from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState, useEffect, useCallback } from "react";
import {
  type AgentView,
  type ContextProbe,
  isDesktop,
  listAgents,
  listProjectFiles,
  listSkills,
  pickFiles,
  probeContextFile,
  setSetting,
  setting,
  type SkillView,
} from "../api";
import {
  type ProviderConfig,
  providerModelLabel,
  providerModelId,
  templateById,
} from "../data/providers";
import { skillSpace, skillsFor } from "../data/skills";
import { type Permission, PERMISSIONS, PERMISSION_SETTING, DEFAULT_PERMISSION, DEFAULT_MODEL } from "../data/models";
import { fileLook, kindFromPath } from "../fileKind";
import { T } from "../i18n";
import { ContextCard } from "./ContextCard";
import { ContextRing } from "./ContextRing";
import type { ContextBudget } from "./trace";
import { Menu, MenuItem } from "../shell/Menu";
import { navigate } from "../routes";

/** The draft stops growing here and starts scrolling instead (LAYOUT.md:150). */
const MAX_HEIGHT = 160;

type Props = {
  /** Active provider config. */
  provider: ProviderConfig | null;
  /** All configured providers. */
  providers: ProviderConfig[];
  /** Settings' Default model — shown when no provider is active (§5b). */
  defaultModel?: string;
  /** Which agent space is showing — picks the skill list. */
  agentMode: string;
  /** Switch to a different provider. */
  onSelectProvider: (index: number) => void;
  /** Switch model inside a provider without leaving the composer. */
  onSelectProviderModel: (providerIndex: number, modelId: string, displayName: string) => void;
  /** False on demo routes; live sends validate the selected project in the page. */
  ready: boolean;
  running: boolean;
  /** Project root for Add context; empty disables the picker. */
  projectPath: string;
  /** Project-relative paths currently pinned for the next send. */
  contexts: string[];
  /** Probe results by path — owned by the page so sent bubbles can reuse them. */
  probes: Record<string, ContextProbe>;
  /** A probe finished for a freshly added attachment. */
  onProbed: (path: string, probe: ContextProbe) => void;
  /** Follow-ups queued while a run is in flight. */
  queueCount?: number;
  onAddContext: (path: string) => void;
  onRemoveContext: (path: string) => void;
  /** Recoverable failure from adding context (illegal path, read failure). */
  onContextError: (message: string) => void;
  onSend: (text: string, context: string[]) => void;
  onStop: () => void;
  /** Provider is missing / unavailable — shown with a Settings recovery action. */
  providerWarning: string | null;
  onOpenProviderSettings: () => void;
  /** localStorage key for the per-conversation draft (loads on change, clears on send). */
  draftKey: string;
  /** 编辑并发送: seeds the draft with a previous ask (nonce forces refocus on repeat clicks). */
  editSeed?: { text: string; nonce: number } | null;
  /** Persona chain selected for this session — empty = 无智能体 (the default).
   *  Order matters: last entry wins for any conflicting session-state binding
   *  the persona carries (model / permission / skills). */
  personaChain: string[];
  /** Session-scoped chain setter; `null` clears it. Owned one level up. */
  onSelectAgents: (names: string[] | null) => void;
  /** Favorited persona names — the dropdown's 常用 section reads them. */
  favoriteAgents: string[];
  /** Recently used personas (MRU, capped at 5) — the 最近用 section. */
  recentAgents: Array<{ name: string; at: number }>;
  /** Live context-window telemetry; null when no turn is in flight. */
  contextBudget: ContextBudget | null;
  /** Persona list version — bumped after AgentsTab CRUD so the dropdown
   *  re-pulls `listAgents()` without a stale cache. */
  agentsVersion?: number;
};

const PERM_ICONS: Record<Permission, typeof Shield> = {
  ask: ShieldAlert,
  auto: ShieldCheck,
  full: Shield,
};

/**
 * Semantic color only on the icon — the trigger label stays neutral so the
 * composer's left cluster reads as quiet chrome (DESIGN §4).
 */
const PERM_CONFIG: Record<Permission, { label: string; color: string }> = {
  ask: { label: "请求批准", color: "var(--kodo-accent)" },
  auto: { label: "自动批准安全操作", color: "var(--kodo-success-text)" },
  full: { label: "项目内自动执行", color: "var(--kodo-warning-text)" },
};

const openSettings = () => navigate(isDesktop() ? "/settings" : "/ui-demo/settings");

export function Composer({
  provider,
  providers,
  defaultModel,
  agentMode,
  onSelectProvider,
  onSelectProviderModel,
  ready,
  running,
  projectPath,
  contexts,
  probes,
  onProbed,
  queueCount = 0,
  onAddContext,
  onRemoveContext,
  onContextError,
  onSend,
  onStop,
  providerWarning,
  onOpenProviderSettings,
  draftKey,
  editSeed = null,
  personaChain,
  onSelectAgents,
  favoriteAgents,
  recentAgents,
  contextBudget,
  agentsVersion = 0,
}: Props) {
  const [draft, setDraft] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);
  /** True while an IME composition is open — the confirming Enter must not send. */
  const composingRef = useRef(false);

  // Draft persistence: load whenever the key changes; write-through on edit.
  // The guard keeps the load from racing the very first render's empty state.
  const loadedKey = useRef<string | null>(null);
  useEffect(() => {
    if (loadedKey.current === draftKey) return;
    loadedKey.current = draftKey;
    setDraft(localStorage.getItem(draftKey) ?? "");
  }, [draftKey]);

  const updateDraft = (value: string) => {
    setDraft(value);
    if (value) localStorage.setItem(draftKey, value);
    else localStorage.removeItem(draftKey);
  };

  // 编辑并发送 pre-fills the draft — the session log is append-only, so an
  // edit resends as a new turn instead of truncating history. The nonce guard
  // keeps a later draftKey change (first send creates the session) from
  // re-applying the same seed and clobbering what the user typed after it.
  const appliedEdit = useRef<number | null>(null);
  useEffect(() => {
    if (!editSeed || appliedEdit.current === editSeed.nonce) return;
    appliedEdit.current = editSeed.nonce;
    setDraft(editSeed.text);
    if (editSeed.text) localStorage.setItem(draftKey, editSeed.text);
    input.current?.focus();
  }, [editSeed, draftKey]);
  const fileInput = useRef<HTMLInputElement>(null);

  /** Welcome-stage hint buttons (and any future external trigger) ask the
   *  composer to take focus and optionally seed the draft. We use a CustomEvent
   *  rather than a forwarded ref / context so callers outside the composer
   *  tree (e.g. the welcome-stage JSX) do not need to know the composer's
   *  internals. */
  useEffect(() => {
    const onFocus = (event: Event) => {
      const detail = (event as CustomEvent<string | undefined>).detail;
      if (typeof detail === "string" && detail.length > 0) {
        setDraft((current) => (current.startsWith(detail) ? current : detail));
      }
      input.current?.focus();
    };
    window.addEventListener("kodo:focus-composer", onFocus);
    return () => window.removeEventListener("kodo:focus-composer", onFocus);
  }, []);

  const [permission, setPermission] = useState<Permission>(DEFAULT_PERMISSION);
  /** + opens a floating action menu anchored to the composer box. */
  const [plusOpen, setPlusOpen] = useState(false);
  /** Side panel next to the + menu (skills list / project files). */
  const [plusPanel, setPlusPanel] = useState<"skills" | "files" | null>(null);
  /** Skills selected from the + menu stay structured until send. */
  const [selectedSkills, setSelectedSkills] = useState<string[]>([]);
  const [fileQuery, setFileQuery] = useState("");
  const [projectFiles, setProjectFiles] = useState<string[]>([]);
  const [filesLoading, setFilesLoading] = useState(false);

  /** Project + user skills; joined into the picker after their space filter. */
  const [userSkills, setUserSkills] = useState<SkillView[]>([]);
  useEffect(() => {
    let alive = true;
    void listSkills(projectPath).then((list) => {
      if (alive) setUserSkills(list);
    });
    return () => {
      alive = false;
    };
  }, [projectPath]);

  /** Persona records for the agent selector — merges builtin + project
   *  + user. Refetches when `agentsVersion` bumps (after AgentsTab CRUD)
   *  or when the active project changes (project scope lives behind
   *  `.kodo/agents/` and is only enumerable from the backend). */
  const [agents, setAgents] = useState<AgentView[]>([]);
  useEffect(() => {
    let alive = true;
    void listAgents(projectPath || undefined).then((list) => {
      if (alive) setAgents(list);
    });
    return () => {
      alive = false;
    };
  }, [agentsVersion, projectPath]);

  /** Typeahead filter in the agent dropdown — empty string = show all. */
  const [agentQuery, setAgentQuery] = useState("");

  const slashQuery = draft.startsWith("/") && !draft.includes("\n") ? draft.slice(1).toLowerCase() : null;
  // The skill list follows the agent space (work vs code); project and user
  // skills join after their own space filter. Selection itself happens in the
  // backend (`run()` shadows a user skill with the project one on a name
  // collision), so the picker shows each name once — project copy first.
  const seen = new Set<string>();
  const skills = [
    ...skillsFor(agentMode),
    ...userSkills
      .filter(
        (skill) =>
          (skill.source === "user" || skill.source === "project") &&
          skillSpace(skill.taskTypes) === agentMode,
      )
      .filter((skill) => {
        if (seen.has(skill.name)) return false;
        seen.add(skill.name);
        return true;
      })
      .map((skill) => ({ id: skill.name, label: skill.name })),
  ];
  const slashMatches =
    slashQuery === null
      ? []
      : skills.filter(
          (skill) =>
            slashQuery === "" ||
            skill.id.includes(slashQuery) ||
            skill.label.includes(slashQuery),
        );
  const slashOpen = slashQuery !== null && slashMatches.length > 0;

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
    setFileQuery("");
  }, []);

  // Project file search for the + panel. Browser/demo without a project path
  // stays empty rather than inventing a catalog.
  useEffect(() => {
    if (!plusOpen || plusPanel !== "files" || !projectPath) {
      setProjectFiles([]);
      return;
    }
    let alive = true;
    setFilesLoading(true);
    const timer = window.setTimeout(() => {
      void listProjectFiles(projectPath, fileQuery || undefined)
        .then((files) => {
          if (!alive) return;
          setProjectFiles(files.slice(0, 80));
        })
        .catch(() => {
          if (alive) setProjectFiles([]);
        })
        .finally(() => {
          if (alive) setFilesLoading(false);
        });
    }, 120);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [plusOpen, plusPanel, projectPath, fileQuery]);

  const pickFile = async (path: string) => {
    if (contexts.includes(path)) return;
    if (!path || path.includes("..")) {
      onContextError(T.composer.attachInvalid(path));
      return;
    }
    // Probe accepts every attachment type (image/text/pdf/office/…) — only a
    // missing or invalid path is refused.
    const probe = await probeContextFile(projectPath, path).catch(() => null);
    if (!probe) {
      onContextError(T.composer.attachFailed(path));
      return;
    }
    onProbed(path, probe);
    onAddContext(path);
  };

  /** Skills selected from + are rendered as removable chips and serialized on send. */
  const applySkill = (skillId: string) => {
    setSelectedSkills((current) => (current.includes(skillId) ? current : [...current, skillId]));
    closePlus();
    requestAnimationFrame(() => input.current?.focus());
  };

  /** Slash selection uses the same structured chip as the + menu. */
  const applySkillFromSlash = (skillId: string) => {
    setDraft("");
    applySkill(skillId);
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
          onContextError(T.composer.attachPathFailed(raw));
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

  /** Persona default skills are merged with the user's manual selection at
   *  send time, so the chip row stays the single source of truth (the persona
   *  never silently strips a user-picked skill). Persona defaults that the
   *  user did not pick show up as chips anyway — they're seeded into the
   *  `selectedSkills` view so the user can see and remove them. */
  const personaDefaultSkills = useMemo(() => {
    const set = new Set<string>();
    for (const name of personaChain) {
      const record = agents.find((entry) => entry.name === name);
      for (const skill of record?.defaults?.skills ?? []) set.add(skill);
    }
    return set;
  }, [personaChain, agents]);
  const effectiveSkills = useMemo(() => {
    const merged = new Set(selectedSkills);
    for (const skill of personaDefaultSkills) merged.add(skill);
    return Array.from(merged);
  }, [selectedSkills, personaDefaultSkills]);

  const send = () => {
    // Slash menu owns Enter while it is open.
    if (slashOpen) return;
    const skillText = effectiveSkills.map((skillId) => `【技能：${skillId}】`).join("\n");
    const text = [skillText, draft.trim()].filter(Boolean).join("\n");
    if (!text || !ready) return;
    // Running still queues: the draft is intentional work, not a mis-click.
    if (running && text.startsWith("/")) return;
    setDraft("");
    setSelectedSkills([]);
    // Sent drafts are gone — don't resurrect them on the next visit.
    localStorage.removeItem(draftKey);
    onSend(text, [...contexts]);
  };

  const permConfig = PERM_CONFIG[permission];
  const PermIcon = PERM_ICONS[permission];
  // No provider yet — show the default model rather than a placeholder so the
  // composer chip and Settings' Default model agree (UI_ACCEPTANCE §5b).
  const modelLabel = provider
    ? providerModelLabel(provider) || provider.model || T.composer.selectModel
    : defaultModel || DEFAULT_MODEL;
  const providerName = provider?.name ?? "";
  // The picker is two panels (2026-09-30): providers on the left, the active
  // provider's models on the right — one source of truth (the active provider),
  // so clicking a provider both highlights it and swaps the right panel.
  const panelProvider =
    (provider && providers.find((p) => p.id === provider.id)) ?? providers[0] ?? null;
  const panelIndex = panelProvider ? providers.indexOf(panelProvider) : -1;
  const panelModels = panelProvider
    ? panelProvider.models ?? templateById(panelProvider.template).models
    : [];
  const panelCurrent = panelProvider ? providerModelId(panelProvider) || panelProvider.model : null;

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
        {/* Attachment cards on top (Codex-style, 2026-09-23) — shared with the
            sent-message bubble: images render a thumbnail; PDF/Office/text
            carry a type badge on a gray card. Paths only — file bodies never
            enter the page. */}
        {contexts.length > 0 && (
          <div className="composer-contexts" data-testid="composer-contexts">
            {contexts.map((path) => (
              <ContextCard
                key={path}
                path={path}
                probe={probes[path]}
                onRemove={onRemoveContext}
              />
            ))}
          </div>
        )}

        {effectiveSkills.length > 0 && (
          <div className="composer-skills" data-testid="composer-skills" aria-label="已选择技能">
            {effectiveSkills.map((skillId) => {
              const fromPersona = personaDefaultSkills.has(skillId);
              return (
                <span
                  key={skillId}
                  className={`chip composer-skill${fromPersona ? " composer-skill--persona" : ""}`}
                  data-skill-id={skillId}
                  data-from-persona={fromPersona ? "true" : "false"}
                >
                  <Puzzle size={13} strokeWidth={1.8} className="chip-icon" />
                  <span className="composer-skill-label">
                    {fromPersona ? T.composer.personaSkillLabel : "技能"}
                  </span>
                  <code>{skillId}</code>
                  <button
                    type="button"
                    className="chip-remove"
                    aria-label={T.composer.removeSkill(skillId)}
                    onClick={() => {
                      // Stripping a persona-default skill also clears it
                      // from `selectedSkills` so it does not come back via
                      // the union on the next render.
                      setSelectedSkills((current) => current.filter((id) => id !== skillId));
                    }}
                  >
                    <X size={12} strokeWidth={2} />
                  </button>
                </span>
              );
            })}
          </div>
        )}

        {/* Draft first, then the bottom control line: attach/permission left,
            model/send right (2026-09-23: controls sink to the bottom of the
            input box). */}
        <div className="composer-row">
          {/* Left: attachment menu + permission */}
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
                aria-label={T.composer.menu}
                aria-expanded={plusOpen}
                aria-haspopup="menu"
                aria-controls={plusOpen ? "composer-plus-menu" : undefined}
                title={T.composer.menuHint}
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
                  <div className={`composer-plus-root${plusPanel ? " composer-plus-root--panel-open" : ""}`}>
                    <div id="composer-plus-menu" className="menu composer-plus-menu" role="menu">
                      <button
                        type="button"
                        role="menuitem"
                        className="menu-item"
                        data-testid="add-context-item"
                        title={T.composer.attachTitle}
                        onClick={() => void openAttachFiles()}
                      >
                        <Paperclip size={14} strokeWidth={1.8} />
                        <span>{T.composer.attach}</span>
                      </button>
                      <button
                        type="button"
                        role="menuitem"
                        className={`menu-item${plusPanel === "files" ? " menu-item--active" : ""}`}
                        data-testid="project-files-item"
                        title={T.composer.projectFilesTitle}
                        onClick={() => setPlusPanel((panel) => (panel === "files" ? null : "files"))}
                      >
                        <FolderSearch size={14} strokeWidth={1.8} />
                        <span>{T.composer.projectFiles}</span>
                        <ChevronDown size={12} strokeWidth={2} className="menu-item-trail" />
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
                        label={T.composer.configureModel}
                        onSelect={() => {
                          closePlus();
                          openSettings();
                        }}
                      />
                    </div>

                    {plusPanel === "files" && (
                      <div className="menu composer-plus-panel composer-files-panel" role="listbox" aria-label={T.composer.projectFiles}>
                        <div className="composer-files-search">
                          <Search size={13} strokeWidth={1.8} />
                          <input
                            className="tree-input"
                            placeholder={T.composer.searchProjectFiles}
                            value={fileQuery}
                            autoFocus
                            onChange={(event) => setFileQuery(event.target.value)}
                          />
                        </div>
                        <div className="composer-files-list" data-testid="context-file-list">
                          {filesLoading && (
                            <div className="menu-item menu-item--static">{T.composer.searching}</div>
                          )}
                          {!filesLoading && projectFiles.length === 0 && (
                            <div className="menu-item menu-item--static">{T.composer.noFiles}</div>
                          )}
                          {!filesLoading &&
                            projectFiles.map((path) => {
                              const { Icon } = fileLook(path, kindFromPath(path));
                              return (
                                <button
                                  key={path}
                                  type="button"
                                  role="option"
                                  className="menu-item"
                                  data-file-path={path}
                                  onClick={() => {
                                    void pickFile(path);
                                    closePlus();
                                  }}
                                >
                                  <Icon size={14} strokeWidth={1.8} />
                                  <span className="menu-item-file">{path}</span>
                                </button>
                              );
                            })}
                        </div>
                      </div>
                    )}

                    {plusPanel === "skills" && (
                      <div className="menu composer-plus-panel" role="listbox" aria-label={T.composer.skills}>
                        {skills.map((skill) => (
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
              aria-label={T.composer.attach}
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
                >
                  <PermIcon size={14} strokeWidth={2} style={{ color: permConfig.color }} />
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

            {/* Agent (persona) dropdown — session-scoped, switchable mid-run
                of the session; the send resolves the name at dispatch time.
                12 builtins first (with badge), user-defined below; both
                filtered by a typeahead above the list. */}
            <Menu
              trigger={({ open, toggle }) => (
                <button
                  type="button"
                  className={
                    "composer-agent-trigger" +
                    (personaChain.length === 0 ? " composer-agent-trigger--placeholder" : "")
                  }
                  data-testid="composer-agent-trigger"
                  aria-label={T.composer.agentAria}
                  aria-expanded={open}
                  onClick={toggle}
                >
                  <Bot size={14} strokeWidth={2} />
                  <span>
                    {personaChain.length === 0
                      ? T.composer.agentNone
                      : personaChain.length === 1
                        ? personaChain[0]
                        : personaChain.join(" → ")}
                  </span>
                  <ChevronDown size={12} strokeWidth={2} />
                </button>
              )}
            >
              {(close) => {
                const q = agentQuery.trim().toLowerCase();
                const matches = (record: AgentView) =>
                  q === "" ||
                  record.name.toLowerCase().includes(q) ||
                  record.description.toLowerCase().includes(q);
                // Group ordering: 常用 → 最近用 → 项目 → 内置 → 自定义. The
                // favorites section is a flat cross-cut (any source), recents
                // skip names that no longer exist, project / builtin / user
                // stay in source order so the dropdown reads as a sorted
                // catalog with shortcut sections on top.
                const agentsByName = new Map(agents.map((a) => [a.name, a]));
                const visible = (name: string) => agentsByName.has(name);
                const favorites = favoriteAgents.filter(visible).filter((name) => matches(agentsByName.get(name)!));
                const recent = recentAgents
                  .filter((entry) => visible(entry.name))
                  .filter((entry) => matches(agentsByName.get(entry.name)!));
                const projectAgents = agents.filter(
                  (record) => record.source === "project" && matches(record),
                );
                const builtinAgents = agents.filter(
                  (record) => record.source === "builtin" && matches(record),
                );
                const userDefinedAgents = agents.filter(
                  (record) => record.source === "user" && matches(record),
                );
                /** Single-click replaces the chain; shift-click appends so the
                 *  user can build a chain one pick at a time. The "无智能体"
                 *  row is a single-click-only reset — shift+click on it is a
                 *  no-op. The handler captures the trigger modifier via the
                 *  native MouseEvent that MenuItem forwards. */
                const handlePick = (record: AgentView, event: React.MouseEvent) => {
                  close();
                  if (event.shiftKey) {
                    if (personaChain.includes(record.name)) {
                      // Shift-click on an already-picked persona removes it
                      // from the chain — the inverse of append.
                      onSelectAgents(personaChain.filter((n) => n !== record.name));
                    } else {
                      onSelectAgents([...personaChain, record.name]);
                    }
                  } else if (
                    personaChain.length === 1 &&
                    personaChain[0] === record.name
                  ) {
                    // Toggle off: clicking the active single persona clears
                    // the chain (the equivalent of picking 无智能体).
                    onSelectAgents(null);
                  } else {
                    onSelectAgents([record.name]);
                  }
                };
                const isSelected = (name: string) => personaChain.includes(name);
                const isChainTail = (name: string) =>
                  personaChain.length > 0 && personaChain[personaChain.length - 1] === name;
                const renderItem = (
                  record: AgentView,
                  hint: string,
                  testId: string,
                  sourceLabel?: string,
                ) => (
                  <button
                    type="button"
                    role="menuitem"
                    aria-checked={isSelected(record.name)}
                    className={`menu-item${isSelected(record.name) ? " menu-item--selected" : ""}`}
                    data-testid={testId}
                    onClick={(event) => handlePick(record, event)}
                  >
                    {isChainTail(record.name) ? (
                      <Check size={14} strokeWidth={2} />
                    ) : isSelected(record.name) ? (
                      <span className="composer-chain-marker" aria-hidden>•</span>
                    ) : (
                      <span className="menu-item-spacer" aria-hidden />
                    )}
                    <span>{record.name}</span>
                    <span className="menu-item-hint">
                      {sourceLabel ? `${sourceLabel} · ` : ""}
                      {hint || record.description}
                    </span>
                  </button>
                );
                return (
                  <>
                    <input
                      type="text"
                      className="composer-agent-filter"
                      data-testid="composer-agent-filter"
                      placeholder={T.agent.filterPlaceholder}
                      value={agentQuery}
                      onChange={(event) => setAgentQuery(event.target.value)}
                      onClick={(event) => event.stopPropagation()}
                    />
                    {personaChain.length > 0 && (
                      <div className="composer-agent-chain-summary" data-testid="composer-chain-summary">
                        {personaChain.join(" → ")}
                      </div>
                    )}
                    <MenuItem
                      icon={
                        personaChain.length === 0 ? (
                          <Check size={14} strokeWidth={2} />
                        ) : (
                          <span className="menu-item-spacer" aria-hidden />
                        )
                      }
                      label={T.composer.agentNone}
                      testId="composer-agent-none"
                      selected={personaChain.length === 0}
                      onSelect={() => {
                        close();
                        onSelectAgents(null);
                      }}
                    />
                    {favorites.length > 0 && (
                      <>
                        <div
                          className="composer-agent-section"
                          data-testid="composer-section-favorites"
                        >
                          {T.agent.groupFavorite}
                        </div>
                        {favorites.map((name) => {
                          const record = agentsByName.get(name)!;
                          return renderItem(
                            record,
                            T.agent.hintFavorite,
                            `composer-agent-fav-${name}`,
                          );
                        })}
                      </>
                    )}
                    {recent.length > 0 && (
                      <>
                        <div
                          className="composer-agent-section"
                          data-testid="composer-section-recent"
                        >
                          {T.agent.groupRecent}
                        </div>
                        {recent.map((entry) => {
                          const record = agentsByName.get(entry.name)!;
                          return renderItem(
                            record,
                            T.agent.hintRecent,
                            `composer-agent-recent-${entry.name}`,
                          );
                        })}
                      </>
                    )}
                    {projectAgents.length > 0 && (
                      <>
                        <div
                          className="composer-agent-section"
                          data-testid="composer-section-project"
                        >
                          {T.agent.groupProject}
                        </div>
                        {projectAgents.map((record) =>
                          renderItem(
                            record,
                            record.description,
                            `composer-agent-${record.name}`,
                            T.agent.badgeProject,
                          ),
                        )}
                      </>
                    )}
                    {builtinAgents.length > 0 && (
                      <>
                        <div
                          className="composer-agent-section"
                          data-testid="composer-section-builtin"
                        >
                          {T.agent.groupBuiltin}
                        </div>
                        {builtinAgents.map((record) =>
                          renderItem(
                            record,
                            record.description,
                            `composer-agent-${record.name}`,
                            T.agent.badgeBuiltin,
                          ),
                        )}
                      </>
                    )}
                    {userDefinedAgents.length > 0 && (
                      <>
                        <div
                          className="composer-agent-section"
                          data-testid="composer-section-user"
                        >
                          {T.agent.groupUser}
                        </div>
                        {userDefinedAgents.map((record) =>
                          renderItem(
                            record,
                            record.description,
                            `composer-agent-${record.name}`,
                          ),
                        )}
                      </>
                    )}
                  </>
                );
              }}
            </Menu>
          </div>

          <textarea
            ref={input}
            className="composer-input"
            rows={1}
            placeholder={T.composer.inputPlaceholder}
            value={draft}
            data-testid="composer-input"
            onCompositionStart={() => {
              composingRef.current = true;
            }}
            onCompositionEnd={() => {
              composingRef.current = false;
            }}
            onChange={(event) => updateDraft(event.target.value)}
            onPaste={(event) => void onComposerPaste(event)}
            onKeyDown={(event) => {
              // IME guard: the Enter that confirms a zh/ja composition must not
              // send. WebKit reports it as keyCode 229 (isComposing can read
              // false), Chrome as isComposing true — check both, plus the ref
              // for engines that end composition before the keydown lands.
              const native = event.nativeEvent as KeyboardEvent;
              const composing =
                native.isComposing || native.keyCode === 229 || composingRef.current;
              if (slashOpen && (event.key === "Enter" || event.key === "Tab") && !composing) {
                event.preventDefault();
                applySkillFromSlash(slashMatches[0].id);
                return;
              }
              if (slashOpen && event.key === "Escape") {
                event.preventDefault();
                setDraft("");
                return;
              }
              // Enter sends, Shift+Enter breaks the line.
              if (event.key === "Enter" && !event.shiftKey && !composing) {
                event.preventDefault();
                send();
              }
            }}
          />

          {/* Right: model picker + send */}
          <div className="composer-right">
            {/* Codex-style context ring — sits next to the model picker as a
                standalone element (the model button is a separate sibling
                so the ring never visually merges with the picker chrome). */}
            <ContextRing budget={contextBudget} />
            {/* Model / provider picker — providers left, their models right. */}
            <div className="menu-anchor composer-model-anchor">
              <Menu
                align="right"
                trigger={({ open, toggle }) => (
                  <button
                    type="button"
                    className="composer-model"
                    aria-expanded={open}
                    data-testid="composer-model"
                    onClick={toggle}
                  >
                    <span title={providerName ? `${providerName} / ${modelLabel}` : modelLabel}>
                      {modelLabel}
                    </span>
                    <ChevronDown size={13} strokeWidth={1.9} />
                  </button>
                )}
              >
                {(close) => (
                  <>
                    {providers.length === 0 ? (
                      <MenuItem
                        icon={<Plus size={14} strokeWidth={1.8} />}
                        label={T.composer.addProvider}
                        onSelect={() => {
                          close();
                          openSettings();
                        }}
                      />
                    ) : (
                      <div className="menu-model-split">
                        <div className="menu-model-providers">
                          {providers.map((p, i) => (
                            <button
                              key={p.id}
                              type="button"
                              role="menuitem"
                              aria-checked={i === panelIndex}
                              className={`menu-item menu-item--group${i === panelIndex ? " menu-item--active" : ""}`}
                              data-provider-index={i}
                              onClick={() => onSelectProvider(i)}
                            >
                              <Sparkles size={14} strokeWidth={1.8} />
                              <span>{p.name}</span>
                            </button>
                          ))}
                        </div>
                        <div
                          className="menu-model-list"
                          role="group"
                          aria-label={panelProvider ? T.composer.modelsAria(panelProvider.name) : ""}
                        >
                          {panelModels.map((m) => (
                            <button
                              key={m.model_id}
                              type="button"
                              role="menuitem"
                              aria-checked={panelCurrent === m.model_id}
                              className={`menu-item${panelCurrent === m.model_id ? " menu-item--selected" : ""}`}
                              data-model-id={m.model_id}
                              onClick={() => {
                                close();
                                onSelectProviderModel(panelIndex, m.model_id, m.display_name);
                              }}
                            >
                              <span>{m.display_name}</span>
                              <span className="menu-item-hint">{m.model_id}</span>
                            </button>
                          ))}
                          {panelModels.length === 0 && (
                            <button
                              type="button"
                              role="menuitem"
                              className="menu-item"
                              onClick={() => {
                                close();
                                openSettings();
                              }}
                            >
                              <span>{T.composer.configureModels}</span>
                            </button>
                          )}
                        </div>
                      </div>
                    )}
                    <div className="menu-separator" />
                    <MenuItem
                      icon={<Settings size={14} strokeWidth={1.8} />}
                      label={T.composer.configureModel}
                      onSelect={() => {
                        close();
                        openSettings();
                      }}
                    />
                  </>
                )}
              </Menu>
            </div>

            {running ? (
              <button type="button" className="composer-send composer-send--stop" aria-label={T.composer.stop} onClick={onStop}>
                <Square size={13} strokeWidth={2.4} />
              </button>
            ) : (
              <button
                type="button"
                className="composer-send"
                aria-label={T.composer.send}
                data-testid="composer-send"
                disabled={!ready || draft.trim() === ""}
                title={
                  !ready
                    ? "对话尚未就绪"
                    : draft.trim() === ""
                      ? "请输入消息后再发送"
                      : "发送消息"
                }
                onClick={send}
              >
                <ArrowUp size={16} strokeWidth={2.2} />
              </button>
            )}
          </div>
        </div>

        {/* Slash skill suggestions — progressive disclosure, not a second sidebar. */}
        {slashOpen && (
          <div className="composer-slash" role="listbox" aria-label="技能" data-testid="composer-slash">
            {slashMatches.map((skill) => (
              <button
                key={skill.id}
                type="button"
                role="option"
                className="menu-item"
                data-slash-id={skill.id}
                onClick={() => applySkillFromSlash(skill.id)}
              >
                <Puzzle size={14} strokeWidth={1.8} />
                <span>{skill.label}</span>
                <span className="menu-item-hint">/{skill.id}</span>
              </button>
            ))}
          </div>
        )}

        {/* Pinned context cards render on top of the draft (above). */}

      </div>

      {(queueCount > 0 || running) && (
        <div className="composer-status" data-testid="composer-status">
          {running ? <span>Agent 执行中 · Enter 可排队下一条</span> : null}
          {queueCount > 0 ? <span>队列 {queueCount}</span> : null}
        </div>
      )}

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
