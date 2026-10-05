/**
 * Attachment card (Codex-style, 2026-09-23): images render a thumbnail;
 * PDF/Office/text carry a type badge on a gray card. Paths only — file bodies
 * never enter the page. One component for both render sites (COMPONENTS.md §13):
 * the composer's row above the draft (removable) and the sent message bubble
 * (static). Without a probe the extension still yields the right badge/icon.
 */
import { X } from "lucide-react";
import type { ContextProbe } from "../api";
import { fileLook, kindFromPath, type FileLook } from "../fileKind";
import { T } from "../i18n";

const fileName = (path: string) => path.split("/").pop() || path;

/** Badge/color/icon — probe kind first, extension guess otherwise. */
function cardLook(probe: ContextProbe | undefined, path: string): FileLook {
  return fileLook(probe?.name ?? path, probe?.kind ?? kindFromPath(path));
}

type Props = {
  /** Project-relative or absolute attachment path (shown in full via title). */
  path: string;
  /** Probe result when available — enables the image thumbnail and exact name. */
  probe?: ContextProbe;
  /** Composer passes this to show the remove control; bubbles omit it. */
  onRemove?: (path: string) => void;
  /** Sent bubbles pass this to open the file in the sidebar; the composer
   *  omits it (its card manages removal instead — the two never combine). */
  onOpenPath?: ((path: string) => void) | null;
};

export function ContextCard({ path, probe, onRemove, onOpenPath = null }: Props) {
  const kind = probe?.kind ?? kindFromPath(path);
  const look = cardLook(probe, path);
  const preview = (
    <div className="ctx-card-preview">
      {kind === "image" && probe?.preview ? (
        <img src={probe.preview} alt={probe.name} />
      ) : (
        <look.Icon size={28} strokeWidth={1.6} />
      )}
    </div>
  );
  const name = <span className="ctx-card-name">{probe?.name ?? fileName(path)}</span>;

  // Openable card (sent bubble): the whole card is one button. The remove
  // control never rides along — call sites are mutually exclusive.
  if (onOpenPath && !onRemove) {
    return (
      <button
        type="button"
        className={`ctx-card ctx-card--open ctx-card--${kind}`}
        data-context-path={path}
        title={path}
        onClick={() => onOpenPath(path)}
      >
        {preview}
        <span className="ctx-card-foot">
          <span className="ctx-badge" style={{ background: look.color }}>
            {look.badge}
          </span>
          {name}
        </span>
      </button>
    );
  }

  return (
    <div className={`ctx-card ctx-card--${kind}`} data-context-path={path} title={path}>
      {preview}
      <div className="ctx-card-foot">
        <span className="ctx-badge" style={{ background: look.color }}>
          {look.badge}
        </span>
        {name}
        {onRemove && (
          <button
            type="button"
            className="chip-remove"
            aria-label={T.composer.removeContext(path)}
            onClick={() => onRemove(path)}
          >
            <X size={12} strokeWidth={2} />
          </button>
        )}
      </div>
    </div>
  );
}
