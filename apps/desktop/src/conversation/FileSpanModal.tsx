import { X } from "lucide-react";
import type { FileSpanDto } from "../api";
import { T } from "../i18n";

/**
 * Side-by-side before/after viewer for one captured file write. Opens from
 * the AgentTrace row's "查看 patch 前文件" link and from the trace page's
 * spans tab. The parent owns loading + close; this component only renders.
 */
export function FileSpanModal({
  span,
  loading,
  onClose,
}: {
  span: FileSpanDto | null;
  loading: boolean;
  onClose: () => void;
}) {
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="modal modal--wide"
        role="dialog"
        aria-modal="true"
        aria-label={T.page.spansView.title}
      >
        <header className="modal-head">
          <h3 className="modal-title">
            {span ? span.path : T.page.spansView.title}
          </h3>
          <button
            type="button"
            className="icon-btn"
            aria-label={T.action.close}
            onClick={onClose}
          >
            <X size={14} strokeWidth={1.8} />
          </button>
        </header>
        {loading || !span ? (
          <p className="empty-note">{T.page.loadingTrace}</p>
        ) : (
          <div className="modal-body modal-body--scroll">
            <div className="modal-cols">
              <section>
                <h4>{T.page.spansView.before}</h4>
                <pre className="file-span-pre">{span.contentBefore || "(空)"}</pre>
              </section>
              <section>
                <h4>
                  {span.contentAfter === null
                    ? "文件已删除或新建"
                    : T.page.spansView.after}
                </h4>
                <pre className="file-span-pre">
                  {span.contentAfter === null
                    ? "(文件已删除或新建)"
                    : span.contentAfter}
                </pre>
              </section>
            </div>
            <footer className="modal-foot muted mono">
              {T.page.spansView.toolCall(span.toolCallId)}
            </footer>
          </div>
        )}
      </div>
    </div>
  );
}