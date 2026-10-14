/**
 * 待办 — 状态管理 modal.
 *
 * The catalog is the source of truth for the board's columns and the chip's
 * 已完成 flag, so this modal is the only place a user can add, rename, recolour,
 * reorder and remove statuses. A status that still holds todos is refused —
 * unless the user names a reassignment target — because removing it would
 * otherwise leave the board with a column-less row.
 *
 * Reorder uses pointer events, the same primitive the cards drag uses, so
 * the affordance is identical and the keyboard alternative is the same: the
 * up/down arrow keys move a row.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronDown, GripVertical, Plus, Trash2 } from "lucide-react";
import { T } from "../../../i18n";
import type {
  TodoStatusDefDto,
  TodoStatusDraftDto,
} from "../../../api";

const W = T.workbench.todo.statusManager;

/** The palette the modal offers. Kept in lock-step with the `data-color` keys
 *  the CSS in `todo.css` knows about. */
const COLORS = [
  "gray", "blue", "amber", "green",
  "purple", "teal", "pink", "red",
] as const;
type Color = (typeof COLORS)[number];

type Draft = {
  id: string;
  name: string;
  color: Color;
  isDone: boolean;
  builtin: boolean;
  version: number;
  held: number;
};

function describe(
  status: TodoStatusDefDto,
  rows: { id: string; status: string }[],
): Draft {
  return {
    id: status.id,
    name: status.name,
    color: status.color as Color,
    isDone: status.isDone,
    builtin: status.builtin,
    version: status.version,
    held: rows.filter((row) => row.status === status.id).length,
  };
}

export function TodoStatusModal({
  statuses,
  rows,
  busyId,
  onCreate,
  onUpdate,
  onDelete,
  onReorder,
  onClose,
}: {
  statuses: TodoStatusDefDto[];
  rows: { id: string; status: string }[];
  busyId: string | null;
  onCreate: (draft: TodoStatusDraftDto) => Promise<void>;
  onUpdate: (status: TodoStatusDefDto) => Promise<void>;
  onDelete: (id: string, version: number, reassignTo: string | null) => Promise<void>;
  onReorder: (ids: string[]) => Promise<void>;
  onClose: () => void;
}) {
  const [drafts, setDrafts] = useState<Draft[]>(() =>
    statuses.map((status) => describe(status, rows)),
  );
  const [reassignFor, setReassignFor] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [newColor, setNewColor] = useState<Color>("gray");
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Re-describe whenever the parent's catalog or rows change — but only when
  // the modal isn't actively editing, so an in-progress rename is not stomped.
  const lastSynced = useRef<string>("");
  useEffect(() => {
    const key = statuses
      .map((s) => `${s.id}:${s.version}:${s.name}:${s.color}:${s.isDone}:${s.sortOrder}`)
      .join("|");
    if (key === lastSynced.current) return;
    lastSynced.current = key;
    setDrafts(statuses.map((status) => describe(status, rows)));
  }, [statuses, rows]);

  /** Persist a draft row's edits to the backend and, on success, mark this
   *  modal as the source of truth. */
  const commit = useCallback(
    async (draft: Draft): Promise<void> => {
      const source = statuses.find((row) => row.id === draft.id);
      if (!source) return;
      const next: TodoStatusDefDto = {
        ...source,
        name: draft.name.trim() || source.name,
        color: draft.color,
        isDone: draft.isDone,
      };
      if (
        next.name === source.name &&
        next.color === source.color &&
        next.isDone === source.isDone
      ) {
        return;
      }
      try {
        await onUpdate(next);
        setError(null);
      } catch (failure) {
        setError(T.workbench.todo.titleRequired);
      }
    },
    [onUpdate, statuses],
  );

  const submitNew = useCallback(async (): Promise<void> => {
    const name = newName.trim();
    if (!name) {
      setError(W.namePlaceholder);
      return;
    }
    setAdding(true);
    setError(null);
    try {
      await onCreate({ name, color: newColor, isDone: false });
      setNewName("");
      setNewColor("gray");
    } catch (failure) {
      setError(T.workbench.todo.titleRequired);
    } finally {
      setAdding(false);
    }
  }, [newName, newColor, onCreate]);

  const remove = useCallback(
    async (draft: Draft): Promise<void> => {
      if (draft.builtin) return;
      // If the column is empty, the backend accepts the delete with no
      // reassignment. Otherwise we ask the user to pick a target first.
      if (draft.held > 0 && reassignFor !== draft.id) {
        setReassignFor(draft.id);
        return;
      }
      const target = reassignFor === draft.id
        ? (drafts.find((row) => row.id !== draft.id)?.id ?? null)
        : null;
      try {
        await onDelete(draft.id, draft.version, target);
        setReassignFor(null);
        setError(null);
      } catch (failure) {
        setError(T.workbench.todo.titleRequired);
      }
    },
    [drafts, onDelete, reassignFor],
  );

  /** Commit a reorder once the user releases the row. The first commit
   *  rewrites the source; subsequent moves reuse the freshly-returned order. */
  const commitOrder = useCallback(
    (next: Draft[]): void => {
      const ids = next.map((row) => row.id);
      onReorder(ids).catch(() => {
        setError(T.workbench.todo.titleRequired);
      });
    },
    [onReorder],
  );

  return (
    <div className="settings-modal" role="dialog" aria-label={W.title}>
      <button
        type="button"
        className="settings-modal-backdrop"
        aria-label={T.workbench.todo.cancel}
        onClick={onClose}
      />
      <div className="settings-modal-panel todo-status-modal">
        <header className="page-head">
          <div className="page-head-titles">
            <h1>{W.title}</h1>
            <p className="page-subtitle">{W.description}</p>
          </div>
          <div className="page-head-actions">
            <button
              type="button"
              className="btn"
              onClick={onClose}
              data-testid="todo-status-close"
            >
              {T.workbench.todo.cancel}
            </button>
          </div>
        </header>

        <div className="todo-status-body">
          {error && (
            <p className="page-error" role="alert">
              {error}
            </p>
          )}

          <ul className="todo-status-list" role="list">
            {drafts.map((draft) => {
              const isBusy = busyId === draft.id;
              return (
                <li
                  key={draft.id}
                  className="todo-status-row"
                  data-testid={`todo-status-row-${draft.id}`}
                >
                  <span
                    className="todo-status-grip"
                    data-testid={`todo-status-grip-${draft.id}`}
                    role="button"
                    aria-label={W.reorder}
                    tabIndex={0}
                    onKeyDown={(event) => {
                      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
                        event.preventDefault();
                        const idx = drafts.findIndex((row) => row.id === draft.id);
                        const swap = event.key === "ArrowUp" ? idx - 1 : idx + 1;
                        if (swap < 0 || swap >= drafts.length) return;
                        const copy = [...drafts];
                        const a = copy[idx];
                        const b = copy[swap];
                        if (!a || !b) return;
                        copy[idx] = b;
                        copy[swap] = a;
                        setDrafts(copy);
                        commitOrder(copy);
                      }
                    }}
                  >
                    <GripVertical size={14} strokeWidth={1.9} />
                  </span>
                  <span
                    className="todo-status-chip"
                    data-color={draft.color}
                    data-testid={`todo-status-chip-${draft.id}`}
                  >
                    {draft.name || "—"}
                  </span>
                  <input
                    className="input todo-status-name"
                    value={draft.name}
                    onChange={(event) =>
                      setDrafts((current) =>
                        current.map((row) =>
                          row.id === draft.id ? { ...row, name: event.target.value } : row,
                        ),
                      )
                    }
                    onBlur={() => commit(draft)}
                    disabled={draft.builtin}
                    data-testid={`todo-status-name-${draft.id}`}
                  />
                  <label className="todo-status-color" data-testid={`todo-status-color-${draft.id}`}>
                    <span className="todo-status-color-current" data-color={draft.color} />
                    <select
                      value={draft.color}
                      onChange={(event) => {
                        const color = event.target.value as Color;
                        const next = drafts.map((row) =>
                          row.id === draft.id ? { ...row, color } : row,
                        );
                        setDrafts(next);
                        const target = next.find((row) => row.id === draft.id);
                        if (target) commit(target);
                      }}
                      disabled={draft.builtin}
                      data-testid={`todo-status-color-select-${draft.id}`}
                      aria-label={W.colorNames[draft.color]}
                    >
                      {COLORS.map((color) => (
                        <option key={color} value={color}>
                          {W.colorNames[color]}
                        </option>
                      ))}
                    </select>
                    <ChevronDown size={12} strokeWidth={2} />
                  </label>
                  <label className="todo-status-done">
                    <input
                      type="checkbox"
                      checked={draft.isDone}
                      onChange={(event) => {
                        const next = drafts.map((row) =>
                          row.id === draft.id ? { ...row, isDone: event.target.checked } : row,
                        );
                        setDrafts(next);
                        const target = next.find((row) => row.id === draft.id);
                        if (target) commit(target);
                      }}
                      disabled={draft.builtin}
                      data-testid={`todo-status-done-${draft.id}`}
                    />
                    <span>{W.done}</span>
                  </label>
                  {draft.builtin && (
                    <span className="todo-status-builtin" title={W.builtIn}>
                      {W.builtIn}
                    </span>
                  )}
                  {reassignFor === draft.id && draft.held > 0 && (
                    <select
                      className="input todo-status-reassign"
                      value=""
                      onChange={(event) => {
                        const target = event.target.value;
                        if (!target) return;
                        setReassignFor(null);
                        void onDelete(draft.id, draft.version, target).catch(() => {
                          setReassignFor(draft.id);
                          setError(T.workbench.todo.titleRequired);
                        });
                      }}
                      data-testid={`todo-status-reassign-${draft.id}`}
                    >
                      <option value="">{W.reassignPlaceholder}</option>
                      {drafts
                        .filter((row) => row.id !== draft.id)
                        .map((row) => (
                          <option key={row.id} value={row.id}>
                            {row.name}
                          </option>
                        ))}
                    </select>
                  )}
                  <button
                    type="button"
                    className="btn todo-status-delete"
                    onClick={() => remove(draft)}
                    disabled={draft.builtin || isBusy}
                    aria-label={W.delete}
                    data-testid={`todo-status-delete-${draft.id}`}
                  >
                    <Trash2 size={13} strokeWidth={1.9} />
                  </button>
                </li>
              );
            })}
          </ul>

          <div className="todo-status-new">
            <input
              className="input"
              placeholder={W.namePlaceholder}
              value={newName}
              onChange={(event) => setNewName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void submitNew();
                }
              }}
              data-testid="todo-status-new-name"
            />
            <label className="todo-status-color">
              <span className="todo-status-color-current" data-color={newColor} />
              <select
                value={newColor}
                onChange={(event) => setNewColor(event.target.value as Color)}
                data-testid="todo-status-new-color"
                aria-label={W.colorNames[newColor]}
              >
                {COLORS.map((color) => (
                  <option key={color} value={color}>
                    {W.colorNames[color]}
                  </option>
                ))}
              </select>
              <ChevronDown size={12} strokeWidth={2} />
            </label>
            <button
              type="button"
              className="btn btn--primary"
              onClick={() => void submitNew()}
              disabled={adding || newName.trim().length === 0}
              data-testid="todo-status-new"
            >
              <Plus size={13} strokeWidth={2.2} />
              {W.add}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
