/**
 * Skill preview / editor modal (FileViewer house style: settings-modal +
 * backdrop + panel). View mode shows the parsed sections a skill declares;
 * edit and new switch the body to a raw SKILL.md textarea — the parser is the
 * validator, so there is no second structured form to drift out of sync.
 * Builtins are read-only; user skills get 编辑/删除 (two-step, no dialog).
 */
import { X } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import {
  deleteSkill,
  readSkill,
  saveSkill,
  type SkillScope,
  type SkillView,
} from "../api";
import { T } from "../i18n";

type Props = {
  /** view = read-only preview · edit = store skill editor · new = create. */
  mode: "view" | "edit" | "new";
  /** Skill id the row opened; absent when creating. */
  name?: string;
  /** Source known from the row (also the create target for `new` mode). */
  source?: "builtin" | "user" | "project";
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
 * Display-only ids from `data/skills.ts` that never reach the backend store
 * but would collide as chip ids if a user skill reused them. The 8 builtin
 * names are rejected by the backend; these four are the frontend's own.
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

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="skill-detail-section">
      <h3>{title}</h3>
      {children}
    </section>
  );
}

const KIND_LABELS = T.skill.kinds as Record<string, string>;

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

  // Upgrade the preview with the parsed backend view when one exists; static
  // row info (raw browser, builtin before resolve) keeps the modal honest.
  useEffect(() => {
    if (!name || mode === "new") return;
    let alive = true;
    const scope = source === "project" || source === "user" ? source : undefined;
    void readSkill(name, { scope, project: projectPath }).then((full) => {
      if (alive && full) setView(full);
    });
    return () => {
      alive = false;
    };
  }, [name, mode, source, projectPath]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const resolved = view?.source ?? source;
  /** Where writes land: the row's own store (user is the default). */
  const scope: SkillScope = resolved === "project" || source === "project" ? "project" : "user";
  const title = editing
    ? mode === "new"
      ? T.skill.newTitle
      : T.skill.editTitle
    : (label ?? view?.name ?? name ?? T.skill.newTitle);

  const startEdit = () => {
    setNameField(view?.name ?? name ?? "");
    setMarkdown(view?.markdown ?? "");
    setError(null);
    setConfirmDelete(false);
    setEditing(true);
  };

  const cancel = () => {
    setError(null);
    setConfirmDelete(false);
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
      await saveSkill(finalName, body, { scope, project: projectPath });
      const oldName = view?.name ?? name;
      if (mode === "edit" && oldName && oldName !== finalName) {
        // Renames store the new file first, then best-effort remove the old
        // one — worst case a recoverable duplicate, never data loss.
        await deleteSkill(oldName, { scope, project: projectPath }).catch(() => undefined);
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

  return (
    <div className="settings-modal" role="dialog" aria-modal="true" aria-label={title} data-testid="skill-detail">
      <button
        type="button"
        className="settings-modal-backdrop"
        aria-label={T.action.close}
        onClick={onClose}
      />
      <div className="settings-modal-panel skill-detail-panel">
        <header className="fv-head">
          <span className="fv-title">{title}</span>
          {!editing && resolved && (
            <span className="skill-detail-source">
              {resolved === "user"
                ? T.skill.user
                : resolved === "project"
                  ? T.skill.project
                  : T.skill.builtin}
            </span>
          )}
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

        {editing ? (
          <div className="skill-detail-body skill-detail-body--edit">
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
          <div className="skill-detail-body">
            {(detail || view?.description) && (
              <p className="skill-detail-detail">{detail ?? view?.description}</p>
            )}
            {view && (
              <>
                <Section title={T.skill.taskTypes}>
                  <div className="skill-detail-tags">
                    {view.taskTypes.map((task) => (
                      <code key={task}>{task}</code>
                    ))}
                  </div>
                </Section>
                <Section title={T.skill.strategy}>
                  <div className="skill-detail-tags">
                    <code>{view.strategy}</code>
                  </div>
                </Section>
                <Section title={T.skill.policy}>
                  <div className="skill-detail-tags">
                    <code>{view.policy}</code>
                  </div>
                </Section>
                {view.workflow.length > 0 && (
                  <Section title={T.skill.workflow}>
                    <ol className="skill-detail-steps">
                      {view.workflow.map((step) => (
                        <li key={step.id}>
                          <code>{step.id}</code>
                          <span className="skill-detail-kind">{KIND_LABELS[step.kind] ?? step.kind}</span>
                          <span>{step.title}</span>
                        </li>
                      ))}
                    </ol>
                  </Section>
                )}
                {view.allowedTools.length > 0 && (
                  <Section title={T.skill.allowedTools}>
                    <div className="skill-detail-tags">
                      {view.allowedTools.map((tool) => (
                        <code key={tool}>{tool}</code>
                      ))}
                    </div>
                  </Section>
                )}
                {view.completionCriteria.length > 0 && (
                  <Section title={T.skill.completionCriteria}>
                    <ul className="skill-detail-list">
                      {view.completionCriteria.map((criterion) => (
                        <li key={criterion}>{criterion}</li>
                      ))}
                    </ul>
                  </Section>
                )}
                {view.guidance && (
                  <Section title={T.skill.guidance}>
                    <p className="skill-detail-detail">{view.guidance}</p>
                  </Section>
                )}
                {view.markdown && (
                  <Section title={T.skill.markdown}>
                    <pre className="skill-detail-md">{view.markdown}</pre>
                  </Section>
                )}
              </>
            )}
            {!view && !detail && (
              <p className="skill-detail-detail">{T.skill.loadFailed}</p>
            )}
            {error && (
              <p className="skill-editor-error" role="alert" data-testid="skill-detail-error">
                {error}
              </p>
            )}
          </div>
        )}

        {(editing || resolved === "user" || resolved === "project") && (
          <footer className="skill-detail-foot">
            {editing ? (
              <>
                <span className="spacer" />
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
                <span className="spacer" />
                <button type="button" className="btn" data-testid="skill-detail-edit" onClick={startEdit}>
                  {T.skill.edit}
                </button>
                <button
                  type="button"
                  className={`btn${confirmDelete ? " btn--danger-armed" : ""}`}
                  data-testid="skill-detail-delete"
                  onClick={() => void remove()}
                >
                  {confirmDelete ? T.skill.deleteConfirm : T.skill.delete}
                </button>
              </>
            )}
          </footer>
        )}
      </div>
    </div>
  );
}
