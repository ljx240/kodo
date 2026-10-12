import { useEffect, useRef } from "react";
import { T } from "../i18n";

export const WIDTH_MIN = 320;
export const CONVERSATION_WIDTH_MIN = 400;

export function clampWidth(value: number, max: number = Infinity): number {
  return Math.min(max, Math.max(WIDTH_MIN, Math.round(value)));
}

type Props = {
  /** Width the panel currently shows — the active mode's value, drag included. */
  value: number;
  /** Live drag update; App applies it transiently and commits on pointerup. */
  onDrag: (width: number | null) => void;
  /** Persist the width into the active key (base / wide). */
  onCommit: (width: number) => void;
  /** Available panel width, after the sidebar, rail and borders. */
  max: number;
  /** Largest panel width that leaves room for the conversation. */
  restoreWidth: number;
  /** Toggle focus mode (double-click on the separator). App owns persistence. */
  onToggleFocus: () => void;
};

/**
 * The handle on the conversation ↔ inspector boundary. Wide windows only —
 * narrow ones overlay the conversation and keep their own clamp, so there is
 * nothing to drag there. Drag reports moves and commits; the width state and
 * the persisted keys live in App (one source of truth for panel geometry).
 */
export function PanelResizer({ value, onDrag, onCommit, max, restoreWidth, onToggleFocus }: Props) {
  const dragRef = useRef({ x: 0, w: 0, moved: false });

  useEffect(() => () => onDrag(null), [onDrag]);

  const resizedWidth = (width: number, delta: number) => {
    // Reverse motion skips the gap left by collapsing the conversation.
    const next = clampWidth((width > restoreWidth && delta < 0 ? restoreWidth : width) + delta, max);
    return next > restoreWidth ? max : next;
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
        dragRef.current = { x: e.clientX, w: value, moved: false };
        onDrag(value);
      }}
      onPointerMove={(e) => {
        if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
        const previous = dragRef.current;
        const next = resizedWidth(previous.w, previous.x - e.clientX);
        dragRef.current = { x: e.clientX, w: next, moved: previous.moved || next !== previous.w };
        onDrag(next);
      }}
      onPointerUp={(e) => {
        if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
        e.currentTarget.releasePointerCapture(e.pointerId);
        onDrag(null);
        // A click without drag (e.g. the second click of a dblclick gesture)
        // would re-commit the current value, bleeding it into the active
        // persistence key. Only commit when the pointer actually moved.
        const previous = dragRef.current;
        const next = resizedWidth(previous.w, previous.x - e.clientX);
        if (previous.moved || next !== previous.w) onCommit(next);
      }}
      onPointerCancel={(e) => {
        if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
        onDrag(null);
      }}
      onLostPointerCapture={() => onDrag(null)}
      onDoubleClick={() => {
        onToggleFocus();
      }}
      onKeyDown={(e) => {
        // ← widens (the boundary slides left over the conversation), → narrows.
        const step = e.shiftKey ? 64 : 16;
        if (e.key === "ArrowLeft") {
          e.preventDefault();
          onCommit(resizedWidth(value, step));
        } else if (e.key === "ArrowRight") {
          e.preventDefault();
          onCommit(resizedWidth(value, -step));
        }
      }}
    />
  );
}
