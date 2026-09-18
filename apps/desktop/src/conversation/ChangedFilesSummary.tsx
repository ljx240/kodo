import { ArrowRight, Files } from "lucide-react";

type Props = {
  files: number;
  added: number;
  removed: number;
  onView: () => void;
};

export function ChangedFilesSummary({ files, added, removed, onView }: Props) {
  return (
    <div className="changed-summary">
      <Files size={15} strokeWidth={1.7} />
      <span className="changed-count">{files} files changed</span>
      <span className="delta-add">+{added}</span>
      <span className="delta-del">-{removed}</span>
      <button type="button" className="btn btn--ghost" onClick={onView}>
        <span>View files</span>
        <ArrowRight size={14} strokeWidth={1.9} />
      </button>
    </div>
  );
}
