/**
 * 插件 · 技能 tab (SkillsTab) under PluginsPage: the builtin list for the
 * current agent space plus the project's and the user's own skills, each row
 * opening the preview modal. The class split is a hard invariant — builtin
 * rows carry `.skill-row`, store rows `.user-skill-row` (project rows add
 * `.project-skill-row` + a distinct data hook) — so acceptance tests can count
 * the builtin list exactly (7 in code space). The page head (mark, h1, 新建
 * button) lives on PluginsPage; `openNew` is its nonce-driven action.
 */
import { Pencil, Plus, Puzzle, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { deleteSkill, listSkills, type SkillSource, type SkillView } from "../api";
import { skillSpace, skillsFor } from "../data/skills";
import { T } from "../i18n";
import { SkillDetailModal } from "./SkillDetailModal";

type StoreSource = "user" | "project";

type ModalState =
  | {
      mode: "view";
      name: string;
      source: SkillSource;
      label?: string;
      detail?: string;
      initial?: SkillView | null;
    }
  | { mode: "edit"; name: string; source: StoreSource; initial: SkillView }
  | { mode: "new"; source: StoreSource }
  | null;

/** Human label for a skill source. `plugin:<tool>` gets the tool name. */
function sourceLabel(source: SkillSource): string {
  if (source.startsWith("plugin:")) {
    return T.skill.pluginSource(source.slice("plugin:".length));
  }
  return T.skill.sources[source] ?? source;
}

export function SkillsTab({
  agentMode,
  projectPath,
  openNew,
}: {
  agentMode: string;
  projectPath?: string | null;
  /** Head-action nonce: each increment opens the new-skill modal once. */
  openNew: number;
}) {
  const [userSkills, setUserSkills] = useState<SkillView[]>([]);
  const [projectSkills, setProjectSkills] = useState<SkillView[]>([]);
  /** Read-only skills found in other agents' global/project directories. */
  const [externalSkills, setExternalSkills] = useState<SkillView[]>([]);
  const [modal, setModal] = useState<ModalState>(null);
  /** Armed two-step delete on a row: `"<scope>:<name>"` awaiting confirmation. */
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  // The head button belongs to PluginsPage, which unmounts with the tab. Skip
  // the first run so a nonce left over from an earlier mount never reopens the
  // modal the moment this tab comes back.
  const firstOpen = useRef(true);
  useEffect(() => {
    if (firstOpen.current) {
      firstOpen.current = false;
      return;
    }
    if (openNew > 0) setModal({ mode: "new", source: "user" });
  }, [openNew]);

  const refresh = useCallback(() => {
    void listSkills(projectPath).then((list) => {
      const inSpace = (skill: SkillView) => skillSpace(skill.taskTypes) === agentMode;
      setUserSkills(list.filter((skill) => skill.source === "user" && inSpace(skill)));
      setProjectSkills(list.filter((skill) => skill.source === "project" && inSpace(skill)));
      setExternalSkills(
        list.filter(
          (skill) =>
            !["builtin", "user", "project"].includes(skill.source) && inSpace(skill),
        ),
      );
    });
  }, [agentMode, projectPath]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const openRow = (skill: SkillView) => {
    setConfirmDelete(null);
    setModal({
      mode: "view",
      name: skill.name,
      source: skill.source,
      initial: skill,
    });
  };

  /** One editable store row (user or project) — same geometry, scoped CRUD. */
  const storeRow = (skill: SkillView, source: StoreSource) => {
    const key = `${source}:${skill.name}`;
    const rowClass =
      source === "project" ? "user-skill-row project-skill-row" : "user-skill-row";
    const dataHook =
      source === "project"
        ? { "data-project-skill-id": skill.name }
        : { "data-user-skill-id": skill.name };
    return (
      <div className={rowClass} key={key} {...dataHook}>
        <button
          type="button"
          className="user-skill-open"
          onClick={() => openRow(skill)}
        >
          <Puzzle size={17} strokeWidth={1.7} />
          <span className="skill-row-label">{skill.name}</span>
          <code>/{skill.name}</code>
        </button>
        <span className="skill-row-actions">
          <button
            type="button"
            className="icon-btn icon-btn--sm"
            aria-label={`${skill.name} ${T.skill.edit}`}
            data-testid={`skill-edit-${skill.name}`}
            onClick={() =>
              setModal({ mode: "edit", name: skill.name, source, initial: skill })
            }
          >
            <Pencil size={14} strokeWidth={1.8} />
          </button>
          <button
            type="button"
            className={`icon-btn icon-btn--sm icon-btn--danger${
              confirmDelete === key ? " icon-btn--armed" : ""
            }`}
            aria-label={
              confirmDelete === key
                ? `${skill.name} ${T.skill.deleteConfirm}`
                : `${skill.name} ${T.skill.delete}`
            }
            data-testid={`skill-delete-${skill.name}`}
            onClick={() => {
              if (confirmDelete !== key) {
                setConfirmDelete(key);
                return;
              }
              setConfirmDelete(null);
              void deleteSkill(skill.name, { scope: source, project: projectPath })
                .then(refresh)
                .catch(() => setConfirmDelete(null));
            }}
          >
            {confirmDelete === key ? (
              <span className="skill-row-confirm">{T.skill.deleteConfirm}</span>
            ) : (
              <Trash2 size={14} strokeWidth={1.8} />
            )}
          </button>
        </span>
      </div>
    );
  };

  /** One read-only external row — preview only, no edit/delete controls. */
  const externalRow = (skill: SkillView) => (
    <div
      className="external-skill-row"
      key={`${skill.source}:${skill.name}`}
      data-external-skill-id={skill.name}
      data-source={skill.source}
    >
      <button
        type="button"
        className="external-skill-open"
        onClick={() => openRow(skill)}
      >
        <Puzzle size={17} strokeWidth={1.7} />
        <span className="skill-row-label">{skill.name}</span>
        <span className="skill-source-badge">{sourceLabel(skill.source)}</span>
        <span className="skill-row-readonly">{T.skill.externalReadOnly}</span>
        <code>/{skill.name}</code>
      </button>
    </div>
  );

  return (
    <>
      <section className="skills-card" aria-label={T.page.skillsCard}>
        {skillsFor(agentMode).map((skill) => (
          <button
            type="button"
            className="skill-row"
            key={skill.id}
            data-skill-id={skill.id}
            onClick={() =>
              setModal({
                mode: "view",
                name: skill.id,
                source: "builtin",
                label: skill.label,
                detail: skill.detail,
              })
            }
          >
            <Puzzle size={17} strokeWidth={1.7} />
            <span className="skill-row-label">{skill.label}</span>
            <code>/{skill.id}</code>
          </button>
        ))}
      </section>

      {projectPath && (
        <section className="skills-card" aria-label={T.page.skillsProjectCard}>
          <div className="skills-card-head">
            {T.page.skillsProjectCard}
            <span className="spacer" />
            <button
              type="button"
              className="icon-btn icon-btn--sm"
              aria-label={T.page.skillsNewInProject}
              data-testid="skill-new-project"
              onClick={() => setModal({ mode: "new", source: "project" })}
            >
              <Plus size={14} strokeWidth={1.8} />
            </button>
          </div>
          {projectSkills.length === 0 ? (
            <div className="skills-empty" data-testid="skills-project-empty">
              {T.page.skillsProjectEmpty}
            </div>
          ) : (
            projectSkills.map((skill) => storeRow(skill, "project"))
          )}
        </section>
      )}

      <section className="skills-card" aria-label={T.page.skillsUserCard}>
        <div className="skills-card-head">{T.page.skillsUserCard}</div>
        {userSkills.length === 0 ? (
          <div className="skills-empty" data-testid="skills-empty">
            {T.page.skillsEmpty}
          </div>
        ) : (
          userSkills.map((skill) => storeRow(skill, "user"))
        )}
      </section>

      <section
        className="skills-card"
        aria-label={T.page.skillsLocalCard}
        data-testid="skills-local-card"
      >
        <div className="skills-card-head">
          {T.page.skillsLocalCard}
          <span className="skills-card-hint">{T.page.skillsLocalHint}</span>
        </div>
        {externalSkills.length === 0 ? (
          <div className="skills-empty" data-testid="skills-local-empty">
            {T.page.skillsLocalEmpty}
          </div>
        ) : (
          externalSkills.map(externalRow)
        )}
      </section>

      {modal && (
        <SkillDetailModal
          {...modal}
          agentMode={agentMode}
          projectPath={projectPath}
          onClose={() => setModal(null)}
          onChanged={refresh}
        />
      )}
    </>
  );
}
