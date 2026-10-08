import type { CSSProperties } from "react";
import { T } from "../i18n";
import type { ContextBudget } from "./trace";

/**
 * Codex-style circular progress ring for the live context window.
 *
 * Rendered next to the model picker in the composer so the user sees the
 * window fill up while a turn is in flight. Three stacked layers:
 *
 * - background arc (`--kodo-hover`): full circle, fixed
 * - foreground arc: length proportional to `percent`, color band:
 *   muted < 60% < warn < 80% < danger (matches `reply-context-bar` palette)
 * - hover popover (role=tooltip): three lines, see `T.budget.ringTooltipBody`.
 *   Follows the same popover recipe as `.turn-rail-preview` (DESIGN §12):
 *   `pointer-events: none`, `--kodo-canvas` on `--kodo-border`, popover
 *   radius, find-bar shadow.
 *
 * `null` when `budget` is missing so the ring doesn't hold stale state
 * across turns.
 */
export function ContextRing({
  budget,
  size = 14,
}: {
  budget: ContextBudget | null;
  size?: number;
}) {
  if (!budget) return null;
  const percent = Math.max(0, Math.min(100, budget.percent));
  const stroke = 1.6;
  const r = (size - stroke) / 2;
  const circumference = 2 * Math.PI * r;
  const dash = (percent / 100) * circumference;
  const tone =
    percent >= 80 ? "danger" : percent >= 60 ? "warn" : "muted";
  const containerStyle: CSSProperties = {
    width: size,
    height: size,
    flex: "0 0 auto",
    display: "inline-block",
    color: `var(--kodo-context-ring-${tone})`,
  };
  const usedLabel = formatTokenLabel(budget.used);
  const windowLabel = formatTokenLabel(budget.window);
  const tooltipBody = T.budget.ringTooltipBody(percent, usedLabel, windowLabel);
  return (
    <span
      className="composer-context-ring"
      data-testid="composer-context-ring"
      data-percent={percent}
      data-tone={tone}
      style={containerStyle}
      tabIndex={0}
      aria-label={tooltipBody.replace(/\n/g, " · ")}
    >
      <svg
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        role="presentation"
      >
        <circle
          className="composer-context-ring-bg"
          cx={size / 2}
          cy={size / 2}
          r={r}
          strokeWidth={stroke}
          fill="none"
        />
        <circle
          className="composer-context-ring-fg"
          cx={size / 2}
          cy={size / 2}
          r={r}
          strokeWidth={stroke}
          strokeDasharray={`${dash} ${circumference - dash}`}
          strokeDashoffset={circumference / 4}
          strokeLinecap="round"
          fill="none"
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />
      </svg>
      <span
        className="composer-context-ring-tooltip"
        role="tooltip"
        data-testid="composer-context-ring-tooltip"
      >
        {tooltipBody.split("\n").map((line, i) => (
          <span key={i} className="composer-context-ring-tooltip-line">
            {line}
          </span>
        ))}
      </span>
    </span>
  );
}

/** Mirror of `formatTokenPair` in `./trace.ts` so the tooltip numbers read
 *  identically to the in-reply bar (e.g. 12480 → "12.5k", 258000 → "258.0k"). */
function formatTokenLabel(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}m`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}