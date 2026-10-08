import { AlertTriangle, FilePlus2, Undo2, User } from "lucide-react";
import { useState } from "react";
import type { ChangeDto } from "../api";
import { T } from "../i18n";

type Props = {
  files: number;
  added: number;
  removed: number;
  /** Unique paths for this turn, already merged by `mergeChanges`. */
  changes: ChangeDto[];
  /** Paths the user had already modified before Kodo ran. */
  preExisting?: string[];
  /** Paths whose working tree diverged from Kodo's after-hash (undo risk). */
  conflicts?: string[];
  onView: () => void;
  /** Undo this turn's Kodo edits. Omitted in demo / while a run is live. */
  onUndo?: (() => void) | null;
  undoing?: boolean;
};

const FOLD_AT = 3;

/**
 * End-of-reply card. Four audiences are kept apart: what Kodo changed this
 * turn, what the user had already changed, where undo would conflict, and what
 * the working tree finally looks like. Counts come from the caller after
 * path-merge, so a path edited twice is one row with its edit count.
 */
export function ChangedFilesSummary({
  files,
  added,
  removed,
  changes,
  preExisting = [],
  conflicts = [],
  onView,
  onUndo = null,
  undoing = false,
}: Props) {
  const [showAll, setShowAll] = useState(false);
  if (files <= 0 || changes.length === 0) return null;

  const folded = changes.length > FOLD_AT && !showAll;
  const shown = folded ? changes.slice(0, FOLD_AT) : changes;
  const kodoCount = changes.length;
  const netNote = [
    `${kodoCount} 个文件由本轮 Kodo 修改`,
    preExisting.length > 0 ? `${preExisting.length} 个用户原有` : null,
    conflicts.length > 0 ? `${conflicts.length} 个冲突` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <section className="changed-summary" data-testid="changed-summary" aria-label={T.files.kodoEdits}>
      <header className="changed-summary-head">
        <span className="changed-summary-icon" aria-hidden>
          <FilePlus2 size={16} strokeWidth={1.8} />
        </span>
        <div className="changed-summary-titles">
          <span className="changed-summary-title">{T.files.title(files)}</span>
          <span className="changed-summary-delta">
            <span className="delta-add">+{added}</span>{" "}
            <span className="delta-del">-{removed}</span>
          </span>
        </div>
        {preExisting.length > 0 && (
          <span className="changed-pre" data-testid="pre-existing-count" title={T.files.preExisting}>
            <User size={13} strokeWidth={1.8} />
            {T.files.preExistingCount(preExisting.length)}
          </span>
        )}
        {conflicts.length > 0 && (
          <span className="changed-conflict" data-testid="undo-conflict-count" title={T.files.conflicts}>
            <AlertTriangle size={13} strokeWidth={1.8} />
            {T.files.conflictCount(conflicts.length)}
          </span>
        )}
        <span className="spacer" />
        {onUndo && (
          <button
            type="button"
            className="changed-summary-action"
            data-testid="undo-turn"
            disabled={undoing}
            title={undoing ? T.action.undoing : T.files.undoSafe}
            onClick={onUndo}
          >
            <span>{undoing ? T.action.undoing : T.action.undo}</span>
            <Undo2 size={14} strokeWidth={1.8} />
          </button>
        )}
        <button type="button" className="btn btn--sm" data-testid="review-files" onClick={onView}>
          {T.action.review}
        </button>
      </header>

      <div className="changed-group" data-testid="changed-kodo">
        <h4 className="changed-group-head">{T.files.kodoEdits}</h4>
        <ul className="changed-file-list">
          {shown.map((change) => (
            <li key={change.path} className="changed-file-row">
              <span className="changed-file-path" title={change.path}>
                {change.path}
              </span>
              <span className="changed-file-edits">{T.files.edits(change.edits ?? 1)}</span>
              <span className="changed-file-delta">
                <span className="delta-add">+{change.added}</span>{" "}
                <span className="delta-del">-{change.removed}</span>
              </span>
            </li>
          ))}
        </ul>
        {folded && (
          <button
            type="button"
            className="changed-fold"
            data-testid="changed-show-all"
            onClick={() => setShowAll(true)}
          >
            {T.action.showAll}
          </button>
        )}
      </div>

      {preExisting.length > 0 && (
        <div className="changed-group" data-testid="changed-pre-existing">
          <h4 className="changed-group-head">{T.files.preExisting}</h4>
          <ul className="changed-file-list">
            {preExisting.map((path) => (
              <li key={path} className="changed-file-row">
                <span className="changed-file-path" title={path}>
                  {path}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {conflicts.length > 0 && (
        <div className="changed-group" data-testid="changed-conflicts">
          <h4 className="changed-group-head">{T.files.conflicts}</h4>
          <ul className="changed-file-list">
            {conflicts.map((path) => (
              <li key={path} className="changed-file-row">
                <span className="changed-file-path" title={path}>
                  {path}
                </span>
              </li>
            ))}
          </ul>
          <p className="changed-note" data-testid="undo-conflict-note" role="note">
            {T.files.undoConflict}
          </p>
        </div>
      )}

      <div className="changed-group" data-testid="changed-net">
        <h4 className="changed-group-head">{T.files.net}</h4>
        <p className="changed-note">{netNote}</p>
        <p className="changed-note">{T.files.cumulativeNote}</p>
      </div>
    </section>
  );
}
