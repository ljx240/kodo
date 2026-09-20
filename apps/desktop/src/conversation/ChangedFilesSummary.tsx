import { AlertTriangle, ArrowRight, Files, User } from "lucide-react";

type Props = {
  files: number;
  added: number;
  removed: number;
  /** Files dirty before the turn (pre-existing user changes). */
  preExisting?: number;
  /** Files whose working tree diverged from Kodo's after-hash (undo risk). */
  conflicts?: number;
  onView: () => void;
};

export function ChangedFilesSummary({
  files,
  added,
  removed,
  preExisting = 0,
  conflicts = 0,
  onView,
}: Props) {
  return (
    <div className="changed-summary">
      <Files size={15} strokeWidth={1.7} />
      <span className="changed-count">{files} files changed</span>
      <span className="delta-add">+{added}</span>
      <span className="delta-del">-{removed}</span>
      {preExisting > 0 && (
        <span className="changed-pre" data-testid="pre-existing-count" title="Pre-existing user change">
          <User size={13} strokeWidth={1.8} />
          {preExisting} pre-existing
        </span>
      )}
      {conflicts > 0 && (
        <span className="changed-conflict" data-testid="undo-conflict-count" title="Undo conflict">
          <AlertTriangle size={13} strokeWidth={1.8} />
          {conflicts} conflict
        </span>
      )}
      <button type="button" className="btn btn--ghost" onClick={onView}>
        <span>View files</span>
        <ArrowRight size={14} strokeWidth={1.9} />
      </button>
    </div>
  );
}
