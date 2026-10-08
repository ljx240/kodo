import {
  memo,
  useEffect,
  useId,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent,
  type PointerEvent,
} from "react";
import { T } from "../i18n";

/** One rail mark: a loaded turn, addressed by index in the turn list. */
export type TurnRailItem = {
  index: number;
  prompt: string;
  response: string;
};

type Props = {
  items: readonly TurnRailItem[];
  /** The turn currently in the reading band — tracked by ConversationPage. */
  activeIndex: number | null;
  onNavigate: (index: number) => void;
};

/** Fixed pitch between neighbouring marks; overflow scrolls inside the frame. */
const TURN_SPACING_PX = 10;
/** Rail padding above the first mark and below the last one, per end. */
const RAIL_INSET_PX = 6;
/** Fade band the mask reserves at a scrollable end. */
const FADE_PX = 24;

type TurnPositionStyle = CSSProperties & {
  readonly "--turn-natural-position": string;
};

type TurnFrameStyle = CSSProperties & {
  readonly "--turn-natural-height": string;
  readonly "--turn-rail-inset": string;
  readonly "--turn-scroll-top": string;
};

function itemPosition(index: number): TurnPositionStyle {
  return { "--turn-natural-position": `${String(index * TURN_SPACING_PX)}px` };
}

function frameStyle(count: number, scrollTop: number): TurnFrameStyle {
  return {
    "--turn-natural-height": `${String((count - 1) * TURN_SPACING_PX + 2 * RAIL_INSET_PX)}px`,
    "--turn-rail-inset": `${String(RAIL_INSET_PX)}px`,
    "--turn-scroll-top": `${String(scrollTop)}px`,
  };
}

function itemAtPointer(
  items: readonly TurnRailItem[],
  frame: HTMLElement,
  scrollTop: number,
  clientY: number,
): TurnRailItem | undefined {
  const rect = frame.getBoundingClientRect();
  const offset = clientY - rect.top + scrollTop - RAIL_INSET_PX;
  const index = Math.max(0, Math.min(items.length - 1, Math.round(offset / TURN_SPACING_PX)));
  return items[index];
}

/** Scroll state the mask fades and follow logic read together. */
interface RailScrollState {
  readonly top: number;
  readonly canScrollUp: boolean;
  readonly canScrollDown: boolean;
}

const RAIL_AT_REST: RailScrollState = { top: 0, canScrollUp: false, canScrollDown: false };

function railScrollState(scroller: HTMLElement): RailScrollState {
  const top = scroller.scrollTop;
  return {
    top,
    canScrollUp: top > 1,
    canScrollDown: top < scroller.scrollHeight - scroller.clientHeight - 1,
  };
}

function sameRailScrollState(left: RailScrollState, right: RailScrollState): boolean {
  return (
    left.top === right.top &&
    left.canScrollUp === right.canScrollUp &&
    left.canScrollDown === right.canScrollDown
  );
}

function TurnRailFrame({ items, activeIndex, onNavigate }: Props) {
  const [previewIndex, setPreviewIndex] = useState<number | null>(null);
  const [scrollState, setScrollState] = useState<RailScrollState>(RAIL_AT_REST);
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  /** While the pointer works the rail, follow must not move it under the hand. */
  const pointerInsideRef = useRef(false);
  const previewId = useId();

  const syncScrollState = (): void => {
    const scroller = scrollerRef.current;
    if (scroller === null) return;
    const next = railScrollState(scroller);
    setScrollState((current) => (sameRailScrollState(current, next) ? current : next));
  };

  // Frame resizes (composer/inspector changes) move the overflow edges without
  // a scroll event; item count changes move the content height the same way.
  useEffect(() => {
    const scroller = scrollerRef.current;
    if (scroller === null || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(syncScrollState);
    observer.observe(scroller);
    return () => observer.disconnect();
  }, []);
  useEffect(syncScrollState, [items.length]);

  // Keep the active mark visible: centre it whenever it leaves the scrollport,
  // unless the reader's pointer is working the rail.
  useEffect(() => {
    const scroller = scrollerRef.current;
    const index = items.findIndex((item) => item.index === activeIndex);
    if (scroller === null || index < 0 || pointerInsideRef.current) return;
    const markTop = index * TURN_SPACING_PX + RAIL_INSET_PX;
    const viewTop = scroller.scrollTop;
    const viewHeight = scroller.clientHeight;
    if (
      viewHeight <= 0 ||
      (markTop >= viewTop + FADE_PX && markTop <= viewTop + viewHeight - FADE_PX)
    ) {
      return;
    }
    const target = Math.max(0, markTop - viewHeight / 2);
    const reduced =
      typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (typeof scroller.scrollTo === "function") {
      scroller.scrollTo({ top: target, behavior: reduced ? "auto" : "smooth" });
    } else {
      scroller.scrollTop = target;
    }
    syncScrollState();
  }, [activeIndex, items]);

  // Conditional render comes after every hook — React hook order stays stable.
  if (items.length < 2) return null;

  const previewItem = previewIndex === null ? undefined : items[previewIndex];
  const previewPosition = previewIndex === null ? undefined : itemPosition(previewIndex);
  const previewAtPointer = (event: PointerEvent<HTMLElement>): void => {
    const scrollTop = scrollerRef.current?.scrollTop ?? 0;
    setPreviewIndex(
      itemAtPointer(items, event.currentTarget, scrollTop, event.clientY)?.index ?? null,
    );
  };
  const navigateAtPointer = (event: MouseEvent<HTMLElement>): void => {
    const scrollTop = scrollerRef.current?.scrollTop ?? 0;
    const item = itemAtPointer(items, event.currentTarget, scrollTop, event.clientY);
    if (item !== undefined) onNavigate(item.index);
  };
  const fadeClasses = ["turn-rail-scroller"];
  if (scrollState.canScrollUp) fadeClasses.push("turn-rail-fade-top");
  if (scrollState.canScrollDown) fadeClasses.push("turn-rail-fade-bottom");

  return (
    <div className="turn-rail">
      <nav
        className="turn-rail-frame"
        style={frameStyle(items.length, scrollState.top)}
        aria-label={T.turnRail.label}
        data-testid="turn-rail"
        onClick={navigateAtPointer}
        onPointerMove={previewAtPointer}
        onPointerEnter={() => {
          pointerInsideRef.current = true;
        }}
        onPointerLeave={() => {
          pointerInsideRef.current = false;
          setPreviewIndex(null);
        }}
      >
        <div
          ref={scrollerRef}
          className={fadeClasses.join(" ")}
          onScroll={() => syncScrollState()}
        >
          <div className="turn-rail-marks">
            {items.map((item, position) => {
              const active = item.index === activeIndex;
              const showingPreview = item.index === previewIndex;
              const classes = ["turn-rail-mark"];
              if (active) classes.push("turn-rail-mark-active");
              else if (showingPreview) classes.push("turn-rail-mark-preview");
              return (
                <div
                  key={item.index}
                  className="turn-rail-mark-position"
                  style={itemPosition(position)}
                >
                  <button
                    type="button"
                    className={classes.join(" ")}
                    data-testid="turn-rail-mark"
                    aria-label={T.turnRail.jump(position + 1)}
                    aria-current={active ? "true" : undefined}
                    aria-describedby={showingPreview ? previewId : undefined}
                    onClick={(event) => {
                      event.stopPropagation();
                      onNavigate(item.index);
                    }}
                    onFocus={() => setPreviewIndex(item.index)}
                    onBlur={() => setPreviewIndex(null)}
                  />
                </div>
              );
            })}
          </div>
        </div>
        {previewItem !== undefined && previewPosition !== undefined && (
          <div
            id={previewId}
            role="tooltip"
            className="turn-rail-preview"
            data-testid="turn-rail-preview"
            style={previewPosition}
          >
            <div className="turn-rail-preview-prompt">
              {previewItem.prompt || T.turnRail.turn(previewItem.index + 1)}
            </div>
            {previewItem.response !== "" && (
              <div className="turn-rail-preview-response">{previewItem.response}</div>
            )}
          </div>
        )}
      </nav>
    </div>
  );
}

/**
 * Fixed-pitch rail of every turn — hover and focus previews, click to scroll
 * the turn into view, active mark tracks the reading band. An overlay in the
 * conversation's right gutter: it consumes no layout width and never reads as
 * a second column (COMPONENTS §16).
 *
 * Memoized because the enclosing view re-renders on every streaming delta:
 * without the guard a long session rebuilds every mark per commit for a rail
 * that only changes when a turn is added, removed, or becomes active. Props
 * must therefore stay referentially stable across those commits.
 */
export const TurnRail = memo(TurnRailFrame);
