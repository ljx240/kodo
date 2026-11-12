import { X } from "lucide-react";
import { Fragment, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { TurnDto } from "../api";
import { T } from "../i18n";
import { contextHistory } from "./contextInsights";
import { formatDuration } from "./trace";
import "./context-insights.css";

const copy = T.contextInsights;
const count = new Intl.NumberFormat("zh-CN");
const categories = Object.keys(copy.categories) as (keyof typeof copy.categories)[];
const injectionCategories = Object.keys(copy.injections) as (keyof typeof copy.injections)[];

function signedTokens(value: number) {
  if (value === 0) return "0";
  return `${value > 0 ? "+" : "−"}${count.format(Math.abs(value))}`;
}

function CompositionRow({ label, tokens, previous, share }: {
  label: string;
  tokens: number;
  previous?: number;
  share?: number;
}) {
  return (
    <div className={share === undefined ? "context-insights-injected" : undefined}>
      <dt>{label}</dt>
      <dd>
        <span aria-hidden="true" className={share === undefined ? undefined : "context-insights-track"}>
          {share !== undefined && <span style={{ width: `${share}%` }} />}
        </span>
        <span>{count.format(tokens)}</span>
        {previous !== undefined && <span className="context-insights-muted">{signedTokens(tokens - previous)}</span>}
      </dd>
    </div>
  );
}

export function ContextInsightsDialog({ turns, onClose }: {
  turns: TurnDto[];
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const trend = useRef<HTMLDivElement>(null);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [mode, setMode] = useState<"occupancy" | "delta">("occupancy");
  const { requests, adjustments } = useMemo(() => contextHistory(turns), [turns]);
  const latest = requests[requests.length - 1];
  const selected = requests.find((request) => request.key === selectedKey) ?? latest;
  const composition = selected?.usage.composition;
  const previous = selected?.previousComposition;
  const injections = composition?.injections;
  const deltaMode = mode === "delta";
  const deltaMax = deltaMode
    ? requests.reduce((max, request) => Math.max(max, Math.abs(request.inputDelta ?? 0)), 0)
    : 0;
  const elapsed = formatDuration(selected?.call?.duration);
  const output = selected?.call?.outputTokens ?? 0;

  useLayoutEffect(() => {
    const element = dialog.current;
    element?.showModal();
    return () => element?.close();
  }, []);

  useLayoutEffect(() => {
    if (!selectedKey && trend.current) trend.current.scrollLeft = trend.current.scrollWidth;
  }, [latest?.key, selectedKey, mode]);

  const selectRequest = (key: string) => setSelectedKey(key === latest?.key ? null : key);

  return (
    <dialog
      ref={dialog}
      className="context-insights"
      aria-labelledby="context-insights-title"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <header className="context-insights-head">
        <h2 id="context-insights-title">{copy.title}</h2>
        <button type="button" className="context-insights-close" aria-label={copy.close} onClick={onClose}>
          <X size={16} aria-hidden="true" />
        </button>
      </header>
      <div className="context-insights-body">
        {selected && (
          <>
            <section className="context-insights-overview" aria-label={copy.occupancy}>
              <div>
                <div className="context-insights-muted">{copy.occupancy}</div>
                <div className="context-insights-reading">
                  <strong>{selected.usage.percent === null ? "—" : `${selected.usage.percent}%`}</strong>
                  <span>
                    {selected.usage.used === null
                      ? copy.usageUnavailable
                      : `${count.format(selected.usage.used)} / ${count.format(selected.budget.window)} tokens`}
                  </span>
                </div>
              </div>
              <div className="context-insights-model">
                <span>{selected.budget.lastModel}</span>
                {selected.usage.source && (
                  <span className="context-insights-muted">
                    {copy[selected.usage.source]}
                  </span>
                )}
              </div>
              <div className="context-insights-request-details">
                <span>{copy.requestAt(selected.turn, selected.request)}</span>
                {selected.inputDelta !== null && <span>{copy.inputChange} {signedTokens(selected.inputDelta)} tokens</span>}
                {output > 0 && <span>{copy.output} {count.format(output)} tokens</span>}
                {elapsed && <span>{copy.elapsed} {elapsed}</span>}
              </div>
            </section>
            <section className="context-insights-section" aria-labelledby="context-insights-trend">
              <div className="context-insights-section-head">
                <h3 id="context-insights-trend">{copy.trend}</h3>
                <span>{copy.requestCount(requests.length)}</span>
              </div>
              <div className="context-insights-modes" role="group" aria-label={copy.trendMode}>
                <button type="button" aria-pressed={!deltaMode} onClick={() => setMode("occupancy")}>
                  {copy.occupancy}
                </button>
                <button type="button" aria-pressed={deltaMode} onClick={() => setMode("delta")}>
                  {copy.inputChange}
                </button>
              </div>
              <div className="context-insights-plot" data-mode={mode}>
                <div className="context-insights-axis" aria-hidden="true">
                  <span>{deltaMode ? deltaMax > 0 ? signedTokens(deltaMax) : "" : "100%"}</span>
                  <span>{deltaMode ? "0" : "50%"}</span>
                  <span>{deltaMode ? deltaMax > 0 ? signedTokens(-deltaMax) : "" : "0%"}</span>
                </div>
                <div className="context-insights-requests" ref={trend}>
                  {requests.map((request) => {
                    const value = deltaMode ? request.inputDelta : request.usage.percent;
                    const reading = deltaMode
                      ? value === null ? copy.changeUnavailable : `${signedTokens(value)} tokens`
                      : value === null ? copy.usageUnavailable : `${value}%`;
                    const label = `${copy.requestAt(request.turn, request.request)}，${reading}`;
                    const height = deltaMode ? Math.abs(value ?? 0) / Math.max(1, deltaMax) * 50 : value ?? 0;
                    return (
                      <button
                        type="button"
                        key={request.key}
                        className="context-insights-request"
                        data-request-key={request.key}
                        aria-label={label}
                        aria-description={request.adjusted ? copy.adjusted : undefined}
                        aria-pressed={request.key === selected.key}
                        title={request.adjusted ? `${label}，${copy.adjusted}` : label}
                        onClick={() => selectRequest(request.key)}
                      >
                        {request.adjusted && <span className="context-insights-marker" aria-hidden="true" />}
                        <span className="context-insights-bar-area" aria-hidden="true">
                          {value === null
                            ? <span className="context-insights-missing">—</span>
                            : <span className="context-insights-bar" data-negative={value < 0} style={{ height: `${height}%` }} />}
                        </span>
                      </button>
                    );
                  })}
                </div>
              </div>
              {deltaMode && <p className="context-insights-muted">{copy.comparisonRule}</p>}
              {requests.some((request) => request.adjusted) && (
                <p className="context-insights-legend context-insights-muted">
                  <span className="context-insights-marker" aria-hidden="true" />{copy.adjusted}
                </p>
              )}
            </section>
            <section className="context-insights-section" aria-labelledby="context-insights-composition">
              <div className="context-insights-section-head">
                <h3 id="context-insights-composition">{copy.composition}</h3>
                {composition && <span>{copy.estimated} {count.format(composition.total)} tokens</span>}
              </div>
              {composition ? (
                <>
                  {previous && <p className="context-insights-delta-heading context-insights-muted">{copy.compositionChange}</p>}
                  <dl className="context-insights-composition" data-comparing={previous !== null && previous !== undefined}>
                    {categories.map((category) => {
                      const tokens = composition.parts[category];
                      return (
                        <Fragment key={category}>
                          <CompositionRow
                            label={copy.categories[category]}
                            tokens={tokens}
                            previous={previous?.parts[category]}
                            share={composition.total > 0 ? tokens / composition.total * 100 : 0}
                          />
                          {category === "userContext" && injections && injectionCategories.map((key) => {
                            const previousTokens = previous?.injections?.[key];
                            if (injections[key] === 0 && !previousTokens) return null;
                            return (
                              <CompositionRow
                                key={key}
                                label={copy.injections[key]}
                                tokens={injections[key]}
                                previous={previousTokens}
                              />
                            );
                          })}
                        </Fragment>
                      );
                    })}
                  </dl>
                </>
              ) : (
                <p className="context-insights-muted">{copy.compositionUnavailable}</p>
              )}
            </section>
          </>
        )}
        {adjustments.length > 0 && (
          <section className="context-insights-section" aria-labelledby="context-insights-adjustments">
            <div className="context-insights-section-head">
              <h3 id="context-insights-adjustments">{copy.adjustments}</h3>
            </div>
            <ul className="context-insights-adjustments">
              {adjustments.slice().reverse().map((event) => {
                const next = event.nextRequest;
                return (
                  <li key={event.key}>
                    <div>
                      <span>{event.kind === "compaction" ? copy.compaction : copy.trimCause[event.trim.cause]}</span>
                      <span className="context-insights-muted">
                        {event.kind === "compaction"
                          ? copy.compacted(event.dropped, event.summarizedInto)
                          : copy.trimmed(event.trim.dropped)}
                      </span>
                    </div>
                    {next ? (
                      <button type="button" onClick={() => {
                        selectRequest(next.key);
                        trend.current?.querySelector<HTMLButtonElement>(`[data-request-key="${next.key}"]`)?.focus();
                      }}>
                        {copy.requestAt(event.turn, next.number)}
                      </button>
                    ) : (
                      <span className="context-insights-muted">{copy.turn(event.turn)}</span>
                    )}
                  </li>
                );
              })}
            </ul>
          </section>
        )}
        {!selected && adjustments.length === 0 && <p className="context-insights-empty">{copy.empty}</p>}
      </div>
    </dialog>
  );
}
