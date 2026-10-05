import { useEffect, useRef, useState } from "react";
import { isDesktop, setSetting, setting } from "../api";
import { T } from "../i18n";

const ONBOARDING_KEY = "onboarding-done";
/** Hold per stage before auto-advancing; the final stage waits for the CTA. */
const STAGE_MS = 3200;

/**
 * First-run auto onboarding (Dia-style, 2026-09-23): a full-viewport sequence
 * of typographic stages that plays itself — fade/rise entrances, auto-advance,
 * a slim per-stage timing bar. One-shot: 跳过 and 开始使用 both stamp
 * `onboarding-done`, so it never returns. Demo routes and the plain browser
 * (visual fixtures) never show it.
 */
export function AutoOnboarding({ demo }: { demo: boolean }) {
  // "waiting" until the setting resolves — never flash the overlay over a
  // shell that has already been onboarded.
  const [state, setState] = useState<"waiting" | "open" | "closed">(
    isDesktop() && !demo ? "waiting" : "closed",
  );
  const [stage, setStage] = useState(0);
  const skipRef = useRef<HTMLButtonElement | null>(null);
  const stages = T.onboarding.stages;
  const last = stage === stages.length - 1;

  useEffect(() => {
    if (!isDesktop() || demo) return;
    let alive = true;
    void setting(ONBOARDING_KEY).then((stored) => {
      if (!alive) return;
      setState(stored === "true" ? "closed" : "open");
    });
    return () => {
      alive = false;
    };
  }, [demo]);

  // Auto play. The cadence is also the timing bar's fill duration (CSS var).
  useEffect(() => {
    if (state !== "open" || last) return;
    const timer = window.setTimeout(() => setStage((index) => index + 1), STAGE_MS);
    return () => window.clearTimeout(timer);
  }, [state, stage, last]);

  useEffect(() => {
    if (state !== "open") return;
    skipRef.current?.focus();
  }, [state]);

  const finish = () => {
    setState("closed");
    if (isDesktop()) void setSetting(ONBOARDING_KEY, "true");
  };

  useEffect(() => {
    if (state !== "open") return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") finish();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // finish only closes over stable setters
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state]);

  if (state !== "open") return null;

  const step = stages[stage];
  return (
    <div
      className="onb"
      role="dialog"
      aria-modal="true"
      aria-label={T.onboarding.label}
      data-testid="onboarding"
      style={{ "--onb-stage-ms": `${STAGE_MS}ms` } as React.CSSProperties}
    >
      <button
        type="button"
        className="onb-skip"
        data-testid="onboarding-skip"
        ref={skipRef}
        onClick={finish}
      >
        {T.onboarding.skip}
      </button>

      {/* key: each stage remounts so its entrance animation plays. */}
      <div className="onb-stage" key={stage} data-testid="onboarding-stage" aria-live="polite">
        {step.kicker ? <p className="onb-kicker">{step.kicker}</p> : null}
        <h1 className="onb-title">{step.title}</h1>
        <p className="onb-body">{step.body}</p>
        {last && (
          <button
            type="button"
            className="btn btn--primary onb-cta"
            data-testid="onboarding-start"
            onClick={finish}
          >
            {T.onboarding.start}
          </button>
        )}
      </div>

      <div className="onb-dots" aria-label={T.onboarding.stepsLabel}>
        {stages.map((_, index) => (
          <button
            key={index}
            type="button"
            className={`onb-dot${index === stage ? " onb-dot--active" : ""}${
              index < stage ? " onb-dot--done" : ""
            }`}
            data-testid={`onboarding-dot-${index}`}
            aria-label={T.onboarding.stepOf(index + 1)}
            aria-current={index === stage ? "step" : undefined}
            onClick={() => setStage(index)}
          >
            <span className="onb-dot-bar">
              <span className="onb-dot-fill" />
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
