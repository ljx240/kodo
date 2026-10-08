import { ChevronDown, ChevronUp, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { T } from "../i18n";

type Props = {
  query: string;
  onQuery: (value: string) => void;
  /** 0-based position of the active hit; -1 while there is none. */
  index: number;
  total: number;
  onPrev: () => void;
  onNext: () => void;
  onClose: () => void;
};

/**
 * The conversation's in-page find widget (⌘F). A compact overlay pinned to the
 * conversation column — matches cover each turn's ask and final reply; the
 * active hit scrolls into view and outlines (`.find-hit` on the turn itself).
 */
export function FindBar({ query, onQuery, index, total, onPrev, onNext, onClose }: Props) {
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);

  return (
    <div className="find-bar" role="search" data-testid="find-bar">
      <input
        ref={input}
        className="find-input"
        data-testid="find-input"
        value={query}
        placeholder={T.find.placeholder}
        aria-label={T.find.label}
        onChange={(event) => onQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onClose();
          }
          if (event.key === "Enter") {
            event.preventDefault();
            if (event.shiftKey) onPrev();
            else onNext();
          }
        }}
      />
      <span className="find-count" data-testid="find-count">
        {query.trim() === ""
          ? ""
          : total === 0
            ? T.find.none
            : T.find.matches(index + 1, total)}
      </span>
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        aria-label={T.find.prev}
        data-testid="find-prev"
        disabled={total === 0}
        onClick={onPrev}
      >
        <ChevronUp size={13} strokeWidth={1.9} />
      </button>
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        aria-label={T.find.next}
        data-testid="find-next"
        disabled={total === 0}
        onClick={onNext}
      >
        <ChevronDown size={13} strokeWidth={1.9} />
      </button>
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        aria-label={T.action.close}
        data-testid="find-close"
        onClick={onClose}
      >
        <X size={13} strokeWidth={1.9} />
      </button>
    </div>
  );
}
