import { useEffect, useRef } from "react";
import { ShieldAlert } from "lucide-react";
import type { DecisionPreviewDto } from "../decisionApi";
import { T } from "../i18n";
import "./decision.css";

type Props = {
  preview: DecisionPreviewDto | null;
  onApprove: () => void;
  onSkip: () => void;
};

/**
 * The send preview.
 *
 * It shows the *actual* bytes that would leave the machine, the
 * destination, the model version and what the request is for. The default
 * focus is "skip": approving is the deliberate action, skipping is the
 * safe one.
 */
export function DecisionPreviewDialog({ preview, onApprove, onSkip }: Props) {
  const skipRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (preview) skipRef.current?.focus();
  }, [preview]);

  if (!preview) return null;

  return (
    <div className="decision-preview-backdrop" data-testid="decision-preview-backdrop">
      <section
        className="decision-preview-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="decision-preview-title"
        aria-describedby="decision-preview-copy"
        data-testid="decision-preview"
      >
        <header className="decision-preview-head">
          <div>
            <p className="decision-eyebrow">{T.decision.previewEyebrow}</p>
            <h2 id="decision-preview-title">{T.decision.previewTitle}</h2>
          </div>
          <span className="decision-preview-badge">{T.decision.modes[preview.mode as "off" | "observe" | "assist"] ?? preview.mode}</span>
        </header>

        <p id="decision-preview-copy" className="decision-preview-copy">
          {preview.purpose}
        </p>

        <dl className="decision-preview-facts">
          <div>
            <dt>{T.decision.destinationLabel}</dt>
            <dd>
              <code data-testid="decision-preview-destination">
                {preview.origin}
                {preview.path}
              </code>
            </dd>
          </div>
          <div>
            <dt>{T.decision.modelLabel}</dt>
            <dd>
              <code data-testid="decision-preview-model">{preview.model}</code>
            </dd>
          </div>
          <div>
            <dt>{T.decision.sizeLabel}</dt>
            <dd data-testid="decision-preview-size">
              {preview.requestBytes} B · {T.decision.stateSizeLabel} {preview.stateBytes} B
            </dd>
          </div>
          <div>
            <dt>{T.decision.expiryLabel}</dt>
            <dd>{preview.expiresInSeconds}s</dd>
          </div>
        </dl>

        <div className="decision-preview-body">
          <section className="decision-preview-block">
            <h3>{T.decision.bodyLabel}</h3>
            <pre data-testid="decision-preview-body">{preview.requestBody}</pre>
          </section>
          <section className="decision-preview-block">
            <h3>{T.decision.questionsLabel}</h3>
            <pre data-testid="decision-preview-questions">{preview.questions}</pre>
          </section>
        </div>

        <p className="decision-preview-warning">
          <ShieldAlert size={13} strokeWidth={1.8} aria-hidden="true" />
          <span>{T.decision.previewWarning}</span>
        </p>

        <footer className="decision-preview-actions">
          <button
            ref={skipRef}
            type="button"
            className="btn"
            data-testid="decision-preview-skip"
            onClick={onSkip}
          >
            {T.decision.previewSkip}
          </button>
          <button
            type="button"
            className="btn btn--primary"
            data-testid="decision-preview-approve"
            onClick={onApprove}
          >
            {T.decision.previewApprove}
          </button>
        </footer>
      </section>
    </div>
  );
}
