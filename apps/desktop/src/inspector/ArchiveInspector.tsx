import { CircleCheckBig, Folder, RotateCcw, SquareArrowOutUpRight } from "lucide-react";
import { conversation, project } from "../data/fixture";

export type ArchiveSelection = {
  id: string;
  title: string;
  projectName: string;
  model: string;
  at: string;
  summary?: string;
} | null;

export function ArchiveOverview({
  demo,
  selection,
  onRestore,
  onOpenTrace,
}: {
  demo: boolean;
  selection: ArchiveSelection;
  onRestore: () => void;
  onOpenTrace: () => void;
}) {
  // Live routes never paint fixture conversation facts.
  if (!demo && !selection) {
    return (
      <div className="ins-body">
        <p className="ins-note">在左侧选择一条归档会话以查看详情；恢复操作只在这里提供。</p>
      </div>
    );
  }

  const title = selection?.title ?? (demo ? conversation.title : "");
  const projectName = selection?.projectName ?? (demo ? project.name : "");
  const model = selection?.model && selection.model !== "—" ? selection.model : demo ? "Claude 3.5 Sonnet" : "—";
  const archivedAt = selection?.at || (demo ? "Apr 29, 2024 10:25 AM" : "");
  const summary = selection?.summary ?? (demo ? conversation.assistant.final : "—");

  return (
    <>
      <SectionLite title="Conversation details">
        <div className="ins-body">
          <div className="ins-title-row">
            <span className="ins-title">{title}</span>
            <span className="archived-pill">Archived</span>
          </div>
          <div className="ins-title-row">
            <span className="meta-project">
              <Folder size={13} strokeWidth={1.7} />
              {projectName}
            </span>
          </div>
          <p className="ins-note">归档时间：{archivedAt}</p>
          <p className="ins-note">模型：{model}</p>
          <p className="ins-note">{summary}</p>
        </div>
      </SectionLite>

      <div className="ins-actions">
        <button
          type="button"
          className="btn btn--primary btn--block"
          disabled={!selection || !selection.id}
          onClick={onRestore}
        >
          <RotateCcw size={15} strokeWidth={1.9} />
          <span>Restore conversation</span>
        </button>
        <button type="button" className="btn btn--block" onClick={onOpenTrace}>
          <SquareArrowOutUpRight size={15} strokeWidth={1.9} />
          <span>Open trace</span>
        </button>
      </div>
    </>
  );
}

function SectionLite({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="ins-section">
      <header className="ins-section-head">
        <CircleCheckBig size={14} strokeWidth={1.7} className="ins-section-icon" />
        <h3>{title}</h3>
      </header>
      {children}
    </section>
  );
}
