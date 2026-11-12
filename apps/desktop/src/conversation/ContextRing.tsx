import { useId, type CSSProperties } from "react";
import type { ContextBudget } from "../api";
import { T } from "../i18n";
import { contextUsage } from "./contextInsights";
import { formatTokenPair } from "./trace";

/**
 * The most recent request's window usage and entry to the session dashboard.
 */
export function ContextRing({
  budget,
  onOpen,
  size = 14,
}: {
  budget: ContextBudget | null;
  onOpen: () => void;
  size?: number;
}) {
  const tooltipId = useId();
  if (!budget) return null;
  const usage = contextUsage(budget);
  const percent = usage.percent ?? 0;
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
  const tooltipLines: string[] = [T.budget.ringTitle];
  if (usage.used === null) {
    tooltipLines.push(T.contextInsights.usageUnavailable);
  } else {
    tooltipLines.push(
      T.budget.ringPercent(percent, 100 - percent),
      T.budget.ringTokens(formatTokenPair(usage.used), formatTokenPair(budget.window)),
    );
    if (budget.used === 0) tooltipLines.push(T.contextInsights.estimated);
  }
  return (
    <button
      type="button"
      className="composer-context-ring"
      data-testid="composer-context-ring"
      data-percent={percent}
      data-tone={tone}
      style={containerStyle}
      aria-label={T.contextInsights.title}
      aria-haspopup="dialog"
      aria-describedby={tooltipId}
      onClick={onOpen}
    >
      <svg
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        aria-hidden="true"
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
          strokeLinecap="round"
          fill="none"
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />
      </svg>
      <span
        className="composer-context-ring-tooltip"
        id={tooltipId}
        role="tooltip"
        data-testid="composer-context-ring-tooltip"
      >
        {tooltipLines.map((line) => (
          <span key={line} className="composer-context-ring-tooltip-line">
            {line}
          </span>
        ))}
      </span>
    </button>
  );
}
