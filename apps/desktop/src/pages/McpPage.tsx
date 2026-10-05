import {
  BookOpen,
  Boxes,
  Brain,
  Check,
  ChevronDown,
  ChevronRight,
  Clock,
  Database,
  FileText,
  Folder,
  GitBranch,
  GitMerge,
  Globe,
  HardDrive,
  Layers,
  ListTree,
  MonitorPlay,
  MessageSquare,
  PenTool,
  Play,
  Plug,
  Plus,
  Pencil,
  Shield,
  SquareKanban,
  Table2,
  Trash2,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useState } from "react";
import {
  listMcpTools,
  loadMcpServers,
  saveMcpServers,
  type McpServerDto,
  type McpToolDto,
} from "../api";
import { MCP_TEMPLATES, mcpTemplateById, type McpTemplate } from "../data/mcp";
import { uid } from "../data/providers";
import { T } from "../i18n";
import { Field, Switch } from "../shell/controls";

/**
 * The MCP destination page (sidebar, under 技能): a connector catalog of
 * compact tiles — the one approved tile grid (DESIGN §9) — that prefills the
 * add editor, plus configured servers with enable toggles, inline editing and
 * each server's tool list. Settings → 工具与权限 keeps only the permission mode.
 */

/** Repeatable key/value rows for MCP env and headers. Values arrive masked and
 *  a masked or blank value is sent back untouched (mask writeback — the backend
 *  keeps the stored secret); deleting a row is how one is removed. */
type KeyValueRow = { key: string; value: string };

function KeyValueRows({
  rows,
  onChange,
}: {
  rows: KeyValueRow[];
  onChange: (rows: KeyValueRow[]) => void;
}) {
  return (
    <div className="model-editor">
      {rows.map((row, index) => (
        <div key={index} className="kv-row">
          <input
            className="input"
            placeholder={T.page.mcp.keyPlaceholder}
            value={row.key}
            onChange={(e) => {
              const next = [...rows];
              next[index] = { ...row, key: e.target.value };
              onChange(next);
            }}
          />
          {/* Password type keeps the masked value out of plain sight while still
              round-tripping it on save; typing replaces it. */}
          <input
            className="input"
            type="password"
            placeholder={T.page.mcp.valuePlaceholder}
            value={row.value}
            onChange={(e) => {
              const next = [...rows];
              next[index] = { ...row, value: e.target.value };
              onChange(next);
            }}
          />
          <button
            type="button"
            className="icon-btn icon-btn--sm icon-btn--danger"
            aria-label={T.page.mcp.removeRow(row.key || T.page.mcp.keyPlaceholder)}
            onClick={() => onChange(rows.filter((_, i) => i !== index))}
          >
            <Trash2 size={14} strokeWidth={1.8} />
          </button>
        </div>
      ))}
      <div>
        <button type="button" className="btn" onClick={() => onChange([...rows, { key: "", value: "" }])}>
          <Plus size={15} strokeWidth={1.9} />
          <span>{T.page.mcp.addRow}</span>
        </button>
      </div>
    </div>
  );
}

/** Editor seed from a catalog template; secret rows come up blank. Returns a
 *  blank custom seed when no template id is given. */
function templateFormSeed(templateId?: string) {
  const template: McpTemplate | undefined = templateId ? mcpTemplateById(templateId) : undefined;
  return {
    name: template?.name ?? "",
    transport: (template?.transport ?? "stdio") as "stdio" | "http",
    command: template?.command ?? "",
    args: (template?.args ?? []).join(" "),
    env: (template?.envKeys ?? []).map((key) => ({ key, value: "" })),
    url: template?.url ?? "",
    headers: (template?.headerKeys ?? []).map((key) => ({ key, value: "" })),
  };
}

/** First non-flag argv entry — the package a stdio server launches. */
function stdioPackage(args: string[]): string {
  return args.find((arg) => !arg.startsWith("-")) ?? "";
}

/** A template is "already configured" when a server carries its name or its
 *  launch identity — the stdio package (first non-flag argv) or the http URL.
 *  The catalog offers one instance per connector and marks the tile 已添加;
 *  a deliberate second instance (a second filesystem root, say) goes through
 *  添加 MCP 服务器, which stays unlimited. */
function templateConfigured(template: McpTemplate, servers: McpServerDto[]): boolean {
  return servers.some((server) => {
    if (server.name === template.name) return true;
    if (server.transport !== template.transport) return false;
    if (template.transport === "http") return template.url !== "" && server.url === template.url;
    const pkg = stdioPackage(template.args);
    return pkg !== "" && stdioPackage(server.args) === pkg;
  });
}

function McpServerEditor({
  initial,
  templateId,
  onSave,
  onCancel,
}: {
  initial?: McpServerDto;
  templateId?: string;
  onSave: (config: McpServerDto) => void;
  onCancel: () => void;
}) {
  // Editing an existing server never seeds from a template — a catalog pick
  // can only prefill at creation time.
  const [seed] = useState(() =>
    initial
      ? {
          name: initial.name,
          transport: initial.transport,
          command: initial.command,
          args: initial.args.join(" "),
          env: Object.entries(initial.env).map(([key, value]) => ({ key, value })),
          url: initial.url,
          headers: Object.entries(initial.headers).map(([key, value]) => ({ key, value })),
        }
      : templateFormSeed(templateId),
  );
  const [name, setName] = useState(seed.name);
  const [transport, setTransport] = useState<"stdio" | "http">(seed.transport);
  const [command, setCommand] = useState(seed.command);
  const [args, setArgs] = useState(seed.args);
  const [env, setEnv] = useState<KeyValueRow[]>(seed.env);
  const [url, setUrl] = useState(seed.url);
  const [headers, setHeaders] = useState<KeyValueRow[]>(seed.headers);

  const template = templateId ? mcpTemplateById(templateId) : undefined;

  // Blank keys are dropped; a duplicate key keeps the last row.
  const toMap = (rows: KeyValueRow[]): Record<string, string> =>
    Object.fromEntries(
      rows.filter((row) => row.key.trim() !== "").map((row) => [row.key.trim(), row.value]),
    );

  const save = () => {
    onSave({
      id: initial?.id ?? uid(),
      name: name.trim() || (transport === "stdio" ? command.trim() : url.trim()) || "mcp",
      // Creating the row is the opt-in act; the row toggle can pause it later.
      enabled: initial?.enabled ?? true,
      transport,
      // Only the active transport's fields persist — switching clears the other half.
      command: transport === "stdio" ? command : "",
      args: transport === "stdio" ? args.split(/\s+/).filter(Boolean) : [],
      env: transport === "stdio" ? toMap(env) : {},
      url: transport === "http" ? url : "",
      headers: transport === "http" ? toMap(headers) : {},
    });
  };

  return (
    <div className="provider-editor">
      {!initial && template && <p className="mcp-template-hint">{T.page.mcp.catalogApplied(template.name)}</p>}
      <Field label={T.page.settings.rows.displayName} wide>
        <input className="input" placeholder="filesystem" value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field label={T.page.mcp.transport} hintBelow wide>
        <select
          className="select"
          value={transport}
          onChange={(e) => setTransport(e.target.value === "http" ? "http" : "stdio")}
          aria-label={T.page.mcp.transport}
        >
          <option value="stdio">stdio</option>
          <option value="http">http</option>
        </select>
      </Field>
      {transport === "stdio" ? (
        <>
          <Field label={T.page.mcp.command} wide>
            <input className="input" placeholder="npx" value={command} onChange={(e) => setCommand(e.target.value)} />
          </Field>
          <Field label={T.page.mcp.args} hint={T.page.mcp.argsHint} hintBelow wide>
            <input className="input" value={args} onChange={(e) => setArgs(e.target.value)} />
          </Field>
          <Field label={T.page.mcp.env} hint={T.page.mcp.secretHint} hintBelow block>
            <KeyValueRows rows={env} onChange={setEnv} />
          </Field>
        </>
      ) : (
        <>
          <Field label={T.page.mcp.url} wide>
            <input
              className="input"
              placeholder="https://mcp.example.com/mcp"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
            />
          </Field>
          <Field label={T.page.mcp.headers} hint={T.page.mcp.secretHint} hintBelow block>
            <KeyValueRows rows={headers} onChange={setHeaders} />
          </Field>
        </>
      )}
      {/* No extra margin: the editor's column gap already spaces this row. */}
      <div className="provider-row-actions">
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

/** One server's tool list, fetched on expand through `mcp_list_tools` (the
 *  shell connects with the stored secrets; names and descriptions only ever
 *  reach the webview). Concurrent asks for the same server share one call —
 *  StrictMode double-mounts the effect in dev, and each call really spawns
 *  the server (`npx -y …` can download for seconds even async). */
const toolsInflight = new Map<string, Promise<McpToolDto[]>>();

function listToolsOnce(id: string): Promise<McpToolDto[]> {
  const inflight = toolsInflight.get(id);
  if (inflight) return inflight;
  const call = listMcpTools(id);
  toolsInflight.set(id, call);
  void call.then(
    () => toolsInflight.delete(id),
    () => toolsInflight.delete(id),
  );
  return call;
}

function McpTools({ id }: { id: string }) {
  const [state, setState] = useState<"loading" | "ready" | "error">("loading");
  const [tools, setTools] = useState<McpToolDto[]>([]);

  useEffect(() => {
    let alive = true;
    setState("loading");
    listToolsOnce(id)
      .then((list) => {
        if (!alive) return;
        setTools(list);
        setState("ready");
      })
      .catch(() => {
        if (alive) setState("error");
      });
    return () => {
      alive = false;
    };
  }, [id]);

  return (
    <div className="mcp-tools">
      {state === "loading" && <p className="settings-hint">{T.page.mcp.toolsLoading}</p>}
      {state === "error" && <p className="settings-hint">{T.page.mcp.toolsError}</p>}
      {state === "ready" && tools.length === 0 && <p className="settings-hint">{T.page.mcp.toolsEmpty}</p>}
      {state === "ready" &&
        tools.map((tool) => (
          <div key={tool.name} className="mcp-tool-row">
            <code className="code-chip">{tool.name}</code>
            <span className="mcp-tool-desc">{tool.description}</span>
          </div>
        ))}
    </div>
  );
}

const MCP_ICONS: Record<string, LucideIcon> = {
  folder: Folder,
  "git-branch": GitBranch,
  "git-merge": GitMerge,
  database: Database,
  table: Table2,
  brain: Brain,
  "monitor-play": MonitorPlay,
  shield: Shield,
  "message-square": MessageSquare,
  "file-text": FileText,
  "list-tree": ListTree,
  "book-open": BookOpen,
  play: Play,
  "hard-drive": HardDrive,
  layers: Layers,
  boxes: Boxes,
  "pen-tool": PenTool,
  "square-kanban": SquareKanban,
  globe: Globe,
  clock: Clock,
};

/** The connector catalog: one compact tile per template. A tile for a template
 *  that is already configured shows 已添加 and refuses a second instance; the
 *  custom add button stays unlimited for deliberate variants. */
function McpCatalog({
  servers,
  onPick,
}: {
  servers: McpServerDto[];
  onPick: (templateId: string) => void;
}) {
  return (
    <section>
      <header className="mcp-section-head">
        <h2 className="mcp-section-title">{T.page.mcp.catalogTitle}</h2>
        <p className="mcp-section-hint">{T.page.mcp.catalogHint}</p>
      </header>
      <div className="mcp-catalog">
        {MCP_TEMPLATES.map((template) => {
          const Icon = MCP_ICONS[template.icon] ?? Plug;
          const added = templateConfigured(template, servers);
          return (
            <button
              key={template.id}
              type="button"
              className={`mcp-card${added ? " mcp-card--added" : ""}`}
              aria-label={
                added ? T.page.mcp.catalogAdded(template.name) : T.page.mcp.catalogAdd(template.name)
              }
              disabled={added}
              onClick={() => onPick(template.id)}
            >
              <span className="mcp-card-top">
                <span className="mcp-card-icon">
                  <Icon size={16} strokeWidth={1.7} />
                </span>
                <span className="mcp-card-name">{template.name}</span>
              </span>
              <span className="mcp-card-desc">{T.page.mcp.catalogDesc[template.id]}</span>
              <span className="mcp-card-foot">
                <code className="code-chip">{template.transport}</code>
                {added ? (
                  <span className="mcp-card-state">
                    <Check size={12} strokeWidth={2} />
                    {T.page.mcp.catalogAddedLabel}
                  </span>
                ) : (
                  <span className="mcp-card-add">
                    <Plus size={12} strokeWidth={2} />
                    {T.page.mcp.catalogAddLabel}
                  </span>
                )}
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}

type McpEditMode = null | { mode: "add"; templateId?: string } | { mode: "edit"; index: number };

function McpServerListSection({
  servers,
  loaded,
  update,
  edit,
  setEdit,
}: {
  servers: McpServerDto[];
  loaded: boolean;
  update: (next: McpServerDto[]) => Promise<void>;
  edit: McpEditMode;
  setEdit: (edit: McpEditMode) => void;
}) {
  const [expanded, setExpanded] = useState<string | null>(null);

  return (
    <section>
      <header className="mcp-section-head">
        <h2 className="mcp-section-title">{T.page.mcp.serversTitle}</h2>
        <button type="button" className="btn" onClick={() => setEdit({ mode: "add" })}>
          <Plus size={15} strokeWidth={1.9} />
          <span>{T.page.mcp.add}</span>
        </button>
      </header>

      {!loaded && null}

      {loaded && edit && (
        <McpServerEditor
          initial={edit.mode === "edit" ? servers[edit.index] : undefined}
          templateId={edit.mode === "add" ? edit.templateId : undefined}
          onSave={(config) => {
            if (edit.mode === "add") {
              void update([...servers, config]);
            } else {
              const next = [...servers];
              next[edit.index] = config;
              void update(next);
            }
            setEdit(null);
          }}
          onCancel={() => setEdit(null)}
        />
      )}

      {loaded && !edit && (
        <>
          {servers.length === 0 && (
            <p className="settings-hint" style={{ marginBottom: 12 }}>
              {T.page.mcp.emptyList}
            </p>
          )}

          {servers.map((server, index) => (
            <div key={server.id}>
              <div className="provider-row">
                <div className="provider-row-info">
                  <button
                    type="button"
                    className="icon-btn icon-btn--sm mcp-server-chevron"
                    aria-expanded={expanded === server.id}
                    aria-label={T.page.mcp.toolsToggle(server.name)}
                    onClick={() => setExpanded(expanded === server.id ? null : server.id)}
                  >
                    {expanded === server.id ? (
                      <ChevronDown size={14} strokeWidth={1.9} />
                    ) : (
                      <ChevronRight size={14} strokeWidth={1.9} />
                    )}
                  </button>
                  <span className="provider-row-name">{server.name}</span>
                  <span className="code-chip">{server.transport}</span>
                  <span className="provider-row-model">
                    {server.transport === "stdio" ? server.command : server.url}
                  </span>
                </div>
                <div className="provider-row-actions">
                  <Switch
                    checked={server.enabled}
                    label={T.page.mcp.enabled(server.name)}
                    onChange={() => {
                      const next = [...servers];
                      next[index] = { ...server, enabled: !server.enabled };
                      void update(next);
                    }}
                  />
                  <button
                    type="button"
                    className="icon-btn icon-btn--sm"
                    aria-label={T.page.settings.rows.edit}
                    onClick={() => setEdit({ mode: "edit", index })}
                  >
                    <Pencil size={14} strokeWidth={1.8} />
                  </button>
                  <button
                    type="button"
                    className="icon-btn icon-btn--sm icon-btn--danger"
                    aria-label={T.page.settings.rows.delete}
                    onClick={() => {
                      if (expanded === server.id) setExpanded(null);
                      void update(servers.filter((_, i) => i !== index));
                    }}
                  >
                    <Trash2 size={14} strokeWidth={1.8} />
                  </button>
                </div>
              </div>
              {expanded === server.id && <McpTools id={server.id} />}
            </div>
          ))}
        </>
      )}
    </section>
  );
}

/** 插件 · MCP tab (McpTab) under PluginsPage — the page head lives there. */
export function McpTab() {
  // One source of truth for the server list: the catalog derives its 已添加
  // tiles from it and the list section edits it. `edit` also lives here so the
  // catalog can open the editor without prop-yanking through effects.
  const [edit, setEdit] = useState<McpEditMode>(null);
  const [servers, setServers] = useState<McpServerDto[]>([]);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    void loadMcpServers().then((list) => {
      setServers(list);
      setLoaded(true);
    });
  }, []);

  // Full-list save; the backend answers with re-masked secrets, which become
  // the new local state (same mask-writeback contract as providers).
  const update = async (next: McpServerDto[]) => {
    setServers(await saveMcpServers(next));
  };

  return (
    <>
      <McpCatalog
        servers={servers}
        onPick={(templateId) => setEdit({ mode: "add", templateId })}
      />
      <McpServerListSection
        servers={servers}
        loaded={loaded}
        update={update}
        edit={edit}
        setEdit={setEdit}
      />
    </>
  );
}
