/**
 * Persona preview / editor modal (settings-modal house style, same as
 * SkillDetailModal). Three structured fields — 名称 / 描述 / 说明词 — because
 * that is the whole data shape (COMPONENTS §13d); the frontend composes the
 * three-section markdown and the Rust parser validates it authoritatively.
 * No enable/disable toggle, no model/permission/tools fields (2026-09-30).
 */
import { X } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { deleteAgent, readAgent, saveAgent, type AgentView } from "../api";
import { T } from "../i18n";

type Props = {
  /** view = read-only preview · edit = editor · new = create. */
  mode: "view" | "edit" | "new";
  /** Agent id the row opened; absent when creating. */
  name?: string;
  /** Full view when the caller already has one (rows from listAgents). */
  initial?: AgentView | null;
  /** Active project — the backend resolves project-scope personas first. */
  projectPath?: string;
  onClose: () => void;
  onChanged: () => void;
};

/** Mirrors the backend's name charset (`agents.rs` store functions). */
const NAME_RE = /^[A-Za-z0-9_-]{1,64}$/;

/** The three-section markdown the Rust parser expects. */
function composeMarkdown(name: string, description: string, instructions: string): string {
  return `# Agent: ${name}\n\n## description\n${description}\n\n## instructions\n${instructions}\n`;
}

function validate(name: string, description: string, instructions: string): string | null {
  if (!name) return T.agent.nameRequired;
  if (!NAME_RE.test(name)) return T.agent.nameInvalid;
  if (!description.trim()) return T.agent.descriptionRequired;
  if (!instructions.trim()) return T.agent.instructionsRequired;
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

export function AgentDetailModal({ mode, name, initial, projectPath, onClose, onChanged }: Props) {
  const [view, setView] = useState<AgentView | null>(initial ?? null);
  const [editing, setEditing] = useState(mode !== "view");
  const [nameField, setNameField] = useState(mode === "new" ? "" : (initial?.name ?? name ?? ""));
  const [descriptionField, setDescriptionField] = useState(initial?.description ?? "");
  const [instructionsField, setInstructionsField] = useState(initial?.instructions ?? "");
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  // Builtins are read-only by definition — the row already passes
  // `mode: "view"`, but `startEdit` is also reachable, so gate it on the
  // loaded source as well.
  const isBuiltin = (view?.source ?? initial?.source) === "builtin";

  // Upgrade the preview with the backend view when one exists; static row
  // info keeps the modal honest in the raw browser.
  useEffect(() => {
    if (!name || mode === "new") return;
    let alive = true;
    void readAgent(name, projectPath).then((full) => {
      if (alive && full) {
        setView(full);
        // Backfill edit fields only before the user starts typing.
        if (!editing) {
          setDescriptionField(full.description);
          setInstructionsField(full.instructions);
        }
      }
    });
    return () => {
      alive = false;
    };
    // `editing` intentionally omitted: a save-triggered re-read must not
    // clobber in-progress edits.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [name, mode, projectPath]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const title = editing
    ? mode === "new"
      ? T.agent.newTitle
      : T.agent.editTitle
    : (view?.name ?? name ?? T.agent.viewTitle);

  const startEdit = () => {
    setNameField(view?.name ?? name ?? "");
    setDescriptionField(view?.description ?? "");
    setInstructionsField(view?.instructions ?? "");
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
    const description = descriptionField.trim();
    const instructions = instructionsField.trim();
    const problem = validate(finalName, description, instructions);
    if (problem) {
      setError(problem);
      return;
    }
    setError(null);
    try {
      await saveAgent(finalName, composeMarkdown(finalName, description, instructions));
      const oldName = view?.name ?? name;
      if (mode === "edit" && oldName && oldName !== finalName) {
        // Renames store the new file first, then best-effort remove the old
        // one — worst case a recoverable duplicate, never data loss.
        await deleteAgent(oldName).catch(() => undefined);
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
      await deleteAgent(target);
      onChanged();
      onClose();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="settings-modal" role="dialog" aria-modal="true" aria-label={title} data-testid="agent-detail">
      <button
        type="button"
        className="settings-modal-backdrop"
        aria-label={T.action.close}
        onClick={onClose}
      />
      <div className="settings-modal-panel skill-detail-panel">
        <header className="fv-head">
          <span className="fv-title">{title}</span>
          <span className="spacer" />
          <button
            type="button"
            className="icon-btn icon-btn--sm"
            data-testid="agent-detail-close"
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
              data-testid="agent-name"
              aria-label={T.agent.namePlaceholder}
              placeholder={T.agent.namePlaceholder}
              value={nameField}
              spellCheck={false}
              onChange={(event) => setNameField(event.target.value)}
            />
            <input
              className="input"
              data-testid="agent-description"
              aria-label={T.agent.descriptionPlaceholder}
              placeholder={T.agent.descriptionPlaceholder}
              value={descriptionField}
              onChange={(event) => setDescriptionField(event.target.value)}
            />
            <textarea
              className="skill-editor agent-editor-instructions"
              data-testid="agent-instructions"
              aria-label={T.agent.instructionsPlaceholder}
              placeholder={T.agent.instructionsPlaceholder}
              value={instructionsField}
              spellCheck={false}
              onChange={(event) => setInstructionsField(event.target.value)}
            />
            <p className="settings-hint">{T.agent.instructionsHint}</p>
            {error && (
              <p className="skill-editor-error" role="alert" data-testid="agent-detail-error">
                {error}
              </p>
            )}
          </div>
        ) : (
          <div className="skill-detail-body">
            {view?.description && <p className="skill-detail-detail">{view.description}</p>}
            {view?.instructions && (
              <Section title={T.agent.instructionsLabel}>
                <pre className="skill-detail-md">{view.instructions}</pre>
              </Section>
            )}
            {view && (view.defaults.model || view.defaults.provider || view.defaults.permission || view.defaults.skills.length > 0) && (
              <Section title={T.agent.defaultsLabel}>
                <ul className="skill-detail-list" data-testid="agent-detail-defaults">
                  {view.defaults.provider && (
                    <li><strong>服务商</strong> · <code>{view.defaults.provider}</code></li>
                  )}
                  {view.defaults.model && (
                    <li><strong>模型</strong> · <code>{view.defaults.model}</code></li>
                  )}
                  {view.defaults.permission && (
                    <li><strong>权限</strong> · <code>{view.defaults.permission}</code></li>
                  )}
                  {view.defaults.skills.length > 0 && (
                    <li>
                      <strong>技能</strong> ·
                      {" "}
                      {view.defaults.skills.map((s) => <code key={s}>{s}</code>).reduce<ReactNode[]>(
                        (acc, node, i) => (i === 0 ? [node] : [...acc, " · ", node]),
                        [],
                      )}
                    </li>
                  )}
                </ul>
              </Section>
            )}
            {view && view.variables.length > 0 && (
              <Section title={T.agent.variablesLabel}>
                <ul className="skill-detail-list" data-testid="agent-detail-variables">
                  {view.variables.map((v) => (
                    <li key={v}><code>{`{{${v}}}`}</code></li>
                  ))}
                </ul>
                <p className="settings-hint">{T.agent.variablesHint}</p>
              </Section>
            )}
            {view && view.examples.length > 0 && (
              <Section title={T.agent.examplesLabel}>
                <ol className="skill-detail-examples" data-testid="agent-detail-examples">
                  {view.examples.map((ex, i) => (
                    <li key={i}>
                      <p className="skill-detail-example-label">输入</p>
                      <pre className="skill-detail-md">{ex.input}</pre>
                      <p className="skill-detail-example-label">输出</p>
                      <pre className="skill-detail-md">{ex.output}</pre>
                    </li>
                  ))}
                </ol>
              </Section>
            )}
            {!view && <p className="skill-detail-detail">{T.agent.loadFailed}</p>}
            {error && (
              <p className="skill-editor-error" role="alert" data-testid="agent-detail-error">
                {error}
              </p>
            )}
          </div>
        )}

        <footer className="skill-detail-foot">
          {editing ? (
            <>
              <span className="spacer" />
              <button type="button" className="btn" data-testid="agent-detail-cancel" onClick={cancel}>
                {T.agent.cancel}
              </button>
              <button
                type="button"
                className="btn btn--primary"
                data-testid="agent-detail-save"
                onClick={() => void save()}
              >
                {T.agent.save}
              </button>
            </>
          ) : (
            <>
              <span className="spacer" />
              {!isBuiltin && (
                <>
                  <button type="button" className="btn" data-testid="agent-detail-edit" onClick={startEdit}>
                    {T.agent.edit}
                  </button>
                  <button
                    type="button"
                    className={`btn${confirmDelete ? " btn--danger-armed" : ""}`}
                    data-testid="agent-detail-delete"
                    onClick={() => void remove()}
                  >
                    {confirmDelete ? T.skill.deleteConfirm : T.agent.delete}
                  </button>
                </>
              )}
              <button type="button" className="btn" data-testid="agent-detail-close" onClick={onClose}>
                {T.agent.close}
              </button>
            </>
          )}
        </footer>
      </div>
    </div>
  );
}
