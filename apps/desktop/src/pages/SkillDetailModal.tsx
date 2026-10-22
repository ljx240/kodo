/**
 * Skill viewer / editor modal, laid out like the GPT skill page: a file tree
 * on the left (SKILL.md plus whatever the folder ships), a rendered preview on
 * the right, and edit/new switching that pane to a raw editor.
 *
 * Read-only semantics are unchanged: builtins and skills discovered in other
 * agents' directories never expose edit/delete, and the backend refuses file
 * writes outside Kodo's own stores. Auxiliary files edit in place; SKILL.md
 * goes through the skill editor because saving it must re-parse the spec.
 */
import { FileText, MessageSquare, Pencil, Sparkles, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import {
  deleteSkill,
  listSkillFiles,
  readSkill,
  readSkillFile,
  saveSkill,
  saveSkillFile,
  type SkillFileContent,
  type SkillFileEntry,
  type SkillScope,
  type SkillSource,
  type SkillView,
} from "../api";
import { Markdown } from "../conversation/Markdown";
import { PENDING_SKILL_KEY } from "../data/skills";
import { T } from "../i18n";
import { navigate } from "../routes";
import { SkillFileTree } from "./SkillFileTree";

type Props = {
  /** view = read-only preview · edit = store skill editor · new = create. */
  mode: "view" | "edit" | "new";
  /** Skill id the row opened; absent when creating. */
  name?: string;
  /** Source known from the row (also the create target for `new` mode). */
  source?: SkillSource;
  /** Static display label from the list (builtin rows: 缺陷修复 …). */
  label?: string;
  /** Static one-line description from the display list (builtin rows). */
  detail?: string;
  /** Full view when the caller already has one (rows from listSkills). */
  initial?: SkillView | null;
  /** Current agent space — a new skill's template follows it. */
  agentMode: string;
  /** Selected project path — required for project-scope CRUD. */
  projectPath?: string | null;
  onClose: () => void;
  onChanged: () => void;
};

/** Mirrors the backend's name charset (`skill.rs` store functions). */
const NAME_RE = /^[A-Za-z0-9_-]{1,64}$/;

/**
 * Names the editor refuses for a user skill. The 9 builtin ids are anchored by
 * the backend (`SkillRegistry::with_user` drops a same-named overlay), so this
 * check only saves the user a wasted save; `doc` / `summary` / `qa` are the
 * display-only ids in `data/skills.ts` that have no backend file but would
 * still collide as chip ids. Keep in sync with `skill.rs` `SOURCES`.
 */
const RESERVED = new Set([
  "bug-fix",
  "feature",
  "test",
  "refactor",
  "code-review",
  "docs",
  "work",
  "workflow",
  "kodo-ui",
  "doc",
  "summary",
  "qa",
]);

/** A valid starter SKILL.md for the space the editor was opened in. */
function templateFor(agentMode: string): string {
  if (agentMode === "work") {
    return [
      "# Skill: my-workflow",
      "",
      "## description",
      "One line: when this skill applies and what a good result looks like",
      "",
      "## applicable_task_types",
      "- work",
      "",
      "## context_strategy",
      "work-focused",
      "",
      "## allowed_tools",
      "- search",
      "- read_file",
      "- write_file",
      "- list_files",
      "- run_command",
      "",
      "## verification_policy",
      "none",
      "",
      "## workflow",
      "- s1 | read | Gather the inputs",
      "- s2 | command | Run the chained commands",
      "- s3 | edit | Apply the file operations",
      "- s4 | read | Review and report the outcome",
      "",
      "## completion_criteria",
      "- Every step's outcome is reported in order",
      "- Files stay inside the selected folder",
      "",
      "## guidance",
      "Describe when to use this skill and what a good result looks like.",
      "",
    ].join("\n");
  }
  return [
    "# Skill: my-skill",
    "",
    "## description",
    "One line: when this skill applies and what a good result looks like",
    "",
    "## applicable_task_types",
    "- docs",
    "",
    "## context_strategy",
    "docs-focused",
    "",
    "## allowed_tools",
    "- search",
    "- read_file",
    "- list_files",
    "- write_file",
    "",
    "## verification_policy",
    "none",
    "",
    "## workflow",
    "- s1 | read | Read the input files",
    "- s2 | edit | Apply the requested change",
    "- s3 | read | Review the result",
    "",
    "## completion_criteria",
    "- The requested change is complete",
    "- The result was reviewed",
    "",
    "## guidance",
    "Describe when to use this skill and what a good result looks like.",
    "",
  ].join("\n");
}

/** Keep the `# Skill:` title line equal to the filename the skill saves as. */
function withTitle(markdown: string, name: string): string {
  const lines = markdown.split("\n");
  const index = lines.findIndex((line) => line.trim().startsWith("# Skill:"));
  if (index === -1) return `# Skill: ${name}\n\n${markdown.replace(/^\n+/, "")}`;
  lines[index] = `# Skill: ${name}`;
  return lines.join("\n");
}

function validate(name: string, markdown: string): string | null {
  if (!name) return T.skill.nameRequired;
  if (!NAME_RE.test(name)) return T.skill.nameInvalid;
  if (RESERVED.has(name)) return T.skill.nameReserved;
  if (!markdown.trim()) return T.skill.markdownRequired;
  return null;
}

/** Human label for an external source (`claude`, `plugin:codex`, …). */
function sourceLabel(source: string): string {
  if (source.startsWith("plugin:")) {
    return T.skill.pluginSource(source.slice("plugin:".length));
  }
  return T.skill.sources[source] ?? source;
}

/** `SKILL.md` is the spec (title stripped, metadata shown above); everything
 *  else is shown as its own file. */
function isMarkdownFile(path: string): boolean {
  const ext = path.split(".").pop()?.toLowerCase();
  return ext === "md" || ext === "mdx";
}

/** Frontmatter is metadata, not body — the meta rows above the preview carry
 *  it. The Kodo title is hidden for the same reason. */
function previewBody(markdown: string, skillFile: boolean): string {
  let body = markdown.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n?/, "");
  if (skillFile) body = body.replace(/^#\s+Skill:\s*\S+\s*\r?\n?/i, "");
  return body.trim();
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function modifiedLabel(at: number | null | undefined): string | null {
  if (!at) return null;
  const date = new Date(at * 1000);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "long",
    day: "numeric",
  }).format(date);
}

/** Folder skills (path ends in `SKILL.md`) can be edited in place; a flat
 *  store skill (`<name>.md`) still goes through `save_skill`. */
function isFolderSkill(view: SkillView | null): boolean {
  return Boolean(view?.path && /(^|[\\/])SKILL\.md$/.test(view.path));
}

export function SkillDetailModal({
  mode,
  name,
  source,
  label,
  detail,
  initial,
  agentMode,
  projectPath,
  onClose,
  onChanged,
}: Props) {
  const [view, setView] = useState<SkillView | null>(initial ?? null);
  const [editing, setEditing] = useState(mode !== "view");
  const [nameField, setNameField] = useState(mode === "new" ? "" : (initial?.name ?? name ?? ""));
  const [markdown, setMarkdown] = useState(mode === "new" ? templateFor(agentMode) : (initial?.markdown ?? ""));
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const [files, setFiles] = useState<SkillFileEntry[]>([]);
  const [filesLoading, setFilesLoading] = useState(false);
  const [currentFile, setCurrentFile] = useState("SKILL.md");
  const [file, setFile] = useState<SkillFileContent | null>(null);
  const [fileLoading, setFileLoading] = useState(false);
  const [fileError, setFileError] = useState<string | null>(null);
  /** Non-null while the auxiliary-file editor holds a draft for this path. */
  const [draftPath, setDraftPath] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [savingFile, setSavingFile] = useState(false);

  // Upgrade the preview with the parsed backend view when one exists; static
  // row info (raw browser, builtin before resolve) keeps the modal honest.
  useEffect(() => {
    if (!name || mode === "new") return;
    let alive = true;
    const scope = source === "project" || source === "user" ? source : undefined;
    void readSkill(name, { scope, project: projectPath, source }).then((full) => {
      if (alive && full) setView(full);
    });
    return () => {
      alive = false;
    };
  }, [name, mode, source, projectPath]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // Escape backs out one step at a time so a draft is never lost by a
      // reflexive key press.
      if (draftPath !== null) {
        setDraftPath(null);
        setFileError(null);
      } else if (editing && mode !== "new") {
        setError(null);
        setConfirmDelete(false);
        setEditing(false);
      } else {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, draftPath, editing, mode]);

  const resolved = view?.source ?? source;
  /** Where writes land: the row's own store (user is the default). */
  const scope: SkillScope = resolved === "project" || source === "project" ? "project" : "user";
  /** External and builtin rows never expose edit/delete. */
  const editable = view ? !view.readOnly : resolved === "user" || resolved === "project";
  const targetName = view?.name ?? name ?? "";
  const targetSource = view?.source ?? source;
  const fileScope: SkillScope | undefined =
    targetSource === "project" || targetSource === "user" ? targetSource : undefined;
  const title = editing
    ? mode === "new"
      ? T.skill.newTitle
      : T.skill.editTitle
    : (label ?? view?.name ?? name ?? T.skill.newTitle);
  const modified = modifiedLabel(view?.modifiedAt);
  const skillFileSelected = currentFile === "SKILL.md";
  const treeLocked = editing || draftPath !== null;

  const loadFiles = useCallback(async () => {
    if (!targetName || mode === "new") return;
    setFilesLoading(true);
    const list = await listSkillFiles(targetName, {
      scope: fileScope,
      project: projectPath,
      source: targetSource,
    });
    setFiles(list);
    setCurrentFile((current) =>
      list.some((entry) => entry.path === current) ? current : (list[0]?.path ?? "SKILL.md"),
    );
    setFilesLoading(false);
  }, [targetName, fileScope, projectPath, targetSource, mode]);

  useEffect(() => {
    void loadFiles();
  }, [loadFiles]);

  useEffect(() => {
    if (!targetName || mode === "new") return;
    let alive = true;
    setFileLoading(true);
    setFileError(null);
    setDraftPath(null);
    void readSkillFile(targetName, currentFile, {
      scope: fileScope,
      project: projectPath,
      source: targetSource,
    }).then((content) => {
      if (!alive) return;
      setFile(content);
      setFileLoading(false);
      if (!content) setFileError(T.skill.loadFailed);
    });
    return () => {
      alive = false;
    };
  }, [targetName, currentFile, fileScope, projectPath, targetSource, mode]);

  const startEdit = () => {
    setNameField(view?.name ?? name ?? "");
    setMarkdown(view?.markdown ?? "");
    setError(null);
    setConfirmDelete(false);
    setDraftPath(null);
    setEditing(true);
  };

  const cancel = () => {
    setError(null);
    setConfirmDelete(false);
    setDraftPath(null);
    if (mode === "new") onClose();
    else setEditing(false);
  };

  const save = async () => {
    const finalName = nameField.trim();
    const problem = validate(finalName, markdown);
    if (problem) {
      setError(problem);
      return;
    }
    setError(null);
    try {
      const body = withTitle(markdown, finalName);
      const oldName = view?.name ?? name;
      const renamed = mode === "edit" && Boolean(oldName) && oldName !== finalName;
      if (mode === "edit" && !renamed && isFolderSkill(view) && oldName) {
        // Folder skills save in place so their auxiliary files stay attached.
        await saveSkillFile(oldName, "SKILL.md", body, {
          scope,
          project: projectPath,
          source: view?.source,
        });
      } else {
        await saveSkill(finalName, body, { scope, project: projectPath });
        if (renamed && oldName) {
          // Renames store the new file first, then best-effort remove the old
          // one — worst case a recoverable duplicate, never data loss.
          await deleteSkill(oldName, { scope, project: projectPath }).catch(() => undefined);
        }
      }
      onChanged();
      onClose();
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async () => {
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    const target = view?.name ?? name;
    if (!target) return;
    try {
      await deleteSkill(target, { scope, project: projectPath });
      onChanged();
      onClose();
    } catch (e) {
      setError(String(e));
    }
  };

  const startFileEdit = () => {
    if (!file || file.binary) return;
    setDraft(file.content ?? "");
    setDraftPath(file.path);
    setFileError(null);
  };

  const cancelFileEdit = () => {
    setDraftPath(null);
    setFileError(null);
  };

  const saveDraftFile = async () => {
    if (!targetName || !draftPath) return;
    setSavingFile(true);
    try {
      const saved = await saveSkillFile(targetName, draftPath, draft, {
        scope: fileScope,
        project: projectPath,
        source: targetSource,
      });
      setFile(saved);
      setDraftPath(null);
      setFileError(null);
      await loadFiles();
    } catch (e) {
      setFileError(String(e));
    } finally {
      setSavingFile(false);
    }
  };

  const tryInChat = () => {
    if (!targetName) return;
    try {
      window.sessionStorage.setItem(PENDING_SKILL_KEY, targetName);
    } catch {
      /* storage denied — the conversation still opens, just without the chip */
    }
    navigate(
      window.location.pathname.startsWith("/ui-demo") ? "/ui-demo/conversation" : "/conversation",
    );
  };

  const preview = file && !file.binary ? file.content ?? "" : "";

  return (
    <div className="settings-modal" role="dialog" aria-modal="true" aria-label={title} data-testid="skill-detail">
      <button
        type="button"
        className="settings-modal-backdrop"
        aria-label={T.action.close}
        onClick={onClose}
      />
      <div className="settings-modal-panel skill-detail-panel skill-detail-panel--wide">
        <header className="modal-head skill-viewer-head">
          <span className="skill-viewer-icon" aria-hidden="true">
            <Sparkles size={18} strokeWidth={1.8} />
          </span>
          <div className="skill-viewer-heading">
            <span className="modal-title">{title}</span>
            {!editing && resolved && (
              <span className="skill-viewer-subtitle">
                <span className="skill-detail-source">
                  {resolved === "user"
                    ? T.skill.user
                    : resolved === "project"
                      ? T.skill.project
                      : resolved === "builtin"
                        ? T.skill.builtin
                        : sourceLabel(resolved)}
                </span>
                {modified && (
                  <span className="skill-viewer-date" title={T.skill.modifiedOn(modified)}>
                    {modified}
                  </span>
                )}
              </span>
            )}
          </div>
          <span className="spacer" />
          <button
            type="button"
            className="icon-btn icon-btn--sm"
            data-testid="skill-detail-close"
            aria-label={T.action.close}
            title={T.action.close}
            onClick={onClose}
          >
            <X size={14} strokeWidth={1.8} />
          </button>
        </header>

        <div
          className={`skill-viewer-body${mode === "new" ? " skill-viewer-body--new" : ""}${
            editing || mode === "new" ? " skill-viewer-body--edit" : ""
          }`}
        >
          {mode !== "new" && (
            <SkillFileTree
              files={files}
              active={currentFile}
              loading={filesLoading}
              disabled={treeLocked}
              onSelect={(path) => {
                if (treeLocked) return;
                setCurrentFile(path);
              }}
            />
          )}

          <section className="skill-viewer-main">
            {editing || mode === "new" ? (
              <div className="skill-viewer-editor">
                <input
                  className="skill-editor-name"
                  data-testid="skill-name"
                  aria-label={T.skill.namePlaceholder}
                  placeholder={T.skill.namePlaceholder}
                  value={nameField}
                  spellCheck={false}
                  onChange={(event) => setNameField(event.target.value)}
                />
                <textarea
                  className="skill-editor"
                  data-testid="skill-markdown"
                  aria-label={T.skill.markdown}
                  value={markdown}
                  spellCheck={false}
                  onChange={(event) => setMarkdown(event.target.value)}
                />
                {error && (
                  <p className="skill-editor-error" role="alert" data-testid="skill-detail-error">
                    {error}
                  </p>
                )}
              </div>
            ) : (
              <>
                <div className="skill-viewer-main-head">
                  <FileText size={14} strokeWidth={1.8} aria-hidden="true" />
                  <span className="skill-viewer-file">{currentFile}</span>
                  {file && <span className="skill-viewer-size">{formatSize(file.size)}</span>}
                  <span className="spacer" />
                  {draftPath === null &&
                    !skillFileSelected &&
                    file &&
                    !file.binary &&
                    editable && (
                      <button
                        type="button"
                        className="btn btn--sm"
                        data-testid="skill-file-edit"
                        onClick={startFileEdit}
                      >
                        <Pencil size={12} strokeWidth={1.8} />
                        {T.skill.edit}
                      </button>
                    )}
                  {draftPath !== null && (
                    <>
                      <button
                        type="button"
                        className="btn"
                        data-testid="skill-file-cancel"
                        onClick={cancelFileEdit}
                      >
                        {T.skill.cancel}
                      </button>
                      <button
                        type="button"
                        className="btn btn--primary"
                        data-testid="skill-file-save"
                        disabled={savingFile}
                        onClick={() => void saveDraftFile()}
                      >
                        {T.skill.save}
                      </button>
                    </>
                  )}
                </div>

                <div className="skill-viewer-content" data-testid="skill-file-preview">
                  {fileLoading ? (
                    <p className="skill-viewer-note">{T.skill.loadingFile}</p>
                  ) : draftPath !== null ? (
                    <textarea
                      className="skill-viewer-editor-area"
                      data-testid="skill-file-text"
                      aria-label={currentFile}
                      value={draft}
                      spellCheck={false}
                      onChange={(event) => setDraft(event.target.value)}
                    />
                  ) : fileError ? (
                    <p className="skill-editor-error" role="alert" data-testid="skill-file-error">
                      {fileError}
                    </p>
                  ) : file?.binary ? (
                    <p className="skill-viewer-note">{T.skill.binaryFile}</p>
                  ) : file ? (
                    <div className="skill-viewer-preview">
                      {skillFileSelected && view && (
                        <dl className="skill-detail-meta">
                          <div className="skill-detail-meta-row">
                            <dt>{T.skill.metaName}</dt>
                            <dd>
                              <code>{view.name}</code>
                            </dd>
                          </div>
                          <div className="skill-detail-meta-row">
                            <dt>{T.skill.metaDescription}</dt>
                            <dd>{view.description || detail || T.skill.noDescription}</dd>
                          </div>
                        </dl>
                      )}
                      {skillFileSelected && view?.guidanceOnly && (
                        <p className="skill-detail-note">{T.skill.guidanceOnly}</p>
                      )}
                      {file.truncated && (
                        <p className="skill-viewer-note">{T.skill.fileTruncated}</p>
                      )}
                      {isMarkdownFile(currentFile) ? (
                        <div className="skill-viewer-markdown" data-testid="skill-detail-md">
                          <Markdown text={previewBody(preview, skillFileSelected)} toc={false} />
                        </div>
                      ) : (
                        <pre className="skill-viewer-text" data-testid="skill-detail-md">
                          {preview}
                        </pre>
                      )}
                    </div>
                  ) : (
                    <p className="skill-viewer-note">{T.skill.loadFailed}</p>
                  )}
                </div>
              </>
            )}
          </section>
        </div>

        <footer className="skill-detail-foot skill-viewer-foot">
          <code className="skill-detail-path" data-testid="skill-detail-path">
            {view?.path || (resolved === "builtin" ? T.skill.builtinPath : "")}
          </code>
          <span className="spacer" />
          {editing ? (
            <>
              <button type="button" className="btn" data-testid="skill-detail-cancel" onClick={cancel}>
                {T.skill.cancel}
              </button>
              <button
                type="button"
                className="btn btn--primary"
                data-testid="skill-detail-save"
                onClick={() => void save()}
              >
                {T.skill.save}
              </button>
            </>
          ) : (
            <>
              {error && (
                <span className="skill-viewer-foot-error" role="alert" data-testid="skill-detail-error">
                  {error}
                </span>
              )}
              {draftPath === null && (
                <>
                  {editable && (
                    <button
                      type="button"
                      className={`btn${confirmDelete ? " btn--danger-armed" : ""}`}
                      data-testid="skill-detail-delete"
                      onClick={() => void remove()}
                    >
                      {confirmDelete ? T.skill.deleteConfirm : T.skill.delete}
                    </button>
                  )}
                  {editable && (
                    <button
                      type="button"
                      className="btn"
                      data-testid="skill-detail-edit"
                      onClick={startEdit}
                    >
                      <Pencil size={13} strokeWidth={1.8} />
                      {T.skill.edit}
                    </button>
                  )}
                  <button
                    type="button"
                    className="btn btn--primary"
                    data-testid="skill-try-in-chat"
                    disabled={!targetName}
                    onClick={tryInChat}
                  >
                    <MessageSquare size={13} strokeWidth={1.8} />
                    {T.skill.tryInChat}
                  </button>
                </>
              )}
            </>
          )}
        </footer>
      </div>
    </div>
  );
}
