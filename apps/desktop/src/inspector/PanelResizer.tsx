import { useRef } from "react";
import { T } from "../i18n";

/** House clamp for the right-sidebar column.
 *  Normal drag stops at the focus threshold (900); past it the conversation
 *  column collapses to a narrow state strip and the Inspector grows further
 *  (1100). The threshold is also the lower bound of the focus-mode range, so
 *  the two modes' domains touch at exactly one width. */
export const WIDTH_MIN = 320;
export const WIDTH_MAX = 900;
export const WIDTH_FOCUS_THRESHOLD = 900;
export const WIDTH_FOCUS_MAX = 1100;

export function clampWidth(value: number, max: number = WIDTH_MAX): number {
  return Math.min(max, Math.max(WIDTH_MIN, Math.round(value)));
}

type Props = {
  /** Width the panel currently shows — the active mode's value, drag included. */
  value: number;
  /** Live drag update; App applies it transiently and commits on pointerup. */
  onDrag: (width: number | null) => void;
  /** Persist the width into the active key (base / wide / focus). */
  onCommit: (width: number) => void;
  /** Focus mode is the deliberate exception to "conversation is the largest
   *  visual region": conversation drops to a 300px column so the Inspector
   *  can grow up to {@link WIDTH_FOCUS_MAX}. */
  focusMode: boolean;
  /** Toggle focus mode (double-click on the separator). App owns persistence. */
  onToggleFocus: () => void;
};

/**
 * The handle on the conversation ↔ inspector boundary. Wide windows only —
 * narrow ones overlay the conversation and keep their own clamp, so there is
 * nothing to drag there. Drag reports moves and commits; the width state and
 * the persisted keys live in App (one source of truth for panel geometry).
 */
export function PanelResizer({ value, onDrag, onCommit, focusMode, onToggleFocus }: Props) {
  const startRef = useRef({ x: 0, w: 0 });
  const max = focusMode ? WIDTH_FOCUS_MAX : WIDTH_MAX;

  const setResizing = (on: boolean) => {
    const app = document.querySelector(".app");
    app?.classList.toggle("app--resizing", on);
  };

  return (
    <div
      className="ins-resizer"
      data-testid="inspector-resizer"
      role="separator"
      aria-orientation="vertical"
      aria-label={T.inspector.resize}
      aria-valuenow={value}
      aria-valuemin={WIDTH_MIN}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        startRef.current = { x: e.clientX, w: value };
        setResizing(true);
      }}
      onPointerMove={(e) => {
        if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
        onDrag(clampWidth(startRef.current.w + (startRef.current.x - e.clientX), max));
      }}
      onPointerUp={(e) => {
        if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
        e.currentTarget.releasePointerCapture(e.pointerId);
        setResizing(false);
        onDrag(null);
        // A click without drag (e.g. the second click of a dblclick gesture)
        // would re-commit the current value, bleeding it into the active
        // persistence key. Only commit when the pointer actually moved.
        const next = clampWidth(startRef.current.w + (startRef.current.x - e.clientX), max);
        if (next !== startRef.current.w) onCommit(next);
      }}
      onPointerCancel={() => {
        setResizing(false);
        onDrag(null);
      }}
      onDoubleClick={() => {
        onToggleFocus();
      }}
      onKeyDown={(e) => {
        // ← widens (the boundary slides left over the conversation), → narrows.
        const step = e.shiftKey ? 64 : 16;
        if (e.key === "ArrowLeft") {
          e.preventDefault();
          onCommit(clampWidth(value + step, max));
        } else if (e.key === "ArrowRight") {
          e.preventDefault();
          onCommit(clampWidth(value - step, max));
        }
      }}
    />
  );
}
