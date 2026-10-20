import { useMemo } from "react";
import { Brain } from "lucide-react";
import { parseDecisionRecord, type DecisionRecordDto } from "../decisionApi";
import { T } from "../i18n";
import { Section } from "./Section";
import type { ItemDto } from "../api";

/** Read every decision record out of a turn's reasoning items. A diagnostic
 *  that is not a decision record is ignored — an unreadable schema must not
 *  break the Inspector. */
export function decisionRecordsFromItems(items: ItemDto[] | undefined): DecisionRecordDto[] {
  if (!items) return [];
  const records: DecisionRecordDto[] = [];
  for (const item of items) {
    if (item.kind !== "reasoning") continue;
    const record = parseDecisionRecord(item.diagnostics);
    if (record) records.push(record);
  }
  return records;
}

function statusLabel(record: DecisionRecordDto): string {
  if (record.status === "applied") return T.decision.statusApplied;
  if (record.status === "observed") return T.decision.statusObserved;
  if (record.status === "fallback") return T.decision.statusFallback;
  if (record.status === "cancelled") return T.decision.statusCancelled;
  return T.decision.statusSkipped;
}

/** The Inspector's decision section. It shows what the decision component
 *  said, whether it was used, the probability spread, the timings and the
 *  reason for a fallback — and nothing about the request content. */
export function DecisionRecordsSection({ items }: { items: ItemDto[] | undefined }) {
  const records = useMemo(() => decisionRecordsFromItems(items), [items]);
  if (records.length === 0) return null;
  return (
    <Section icon={<Brain size={14} strokeWidth={1.7} />} title={T.decision.inspectorTitle} count={records.length}>
      <div className="ins-body" data-testid="decision-records">
        {records.map((record) => (
          <article className="decision-record" key={`${record.requestId}-${record.status}`}>
            <div className="decision-record-head">
              <strong>{statusLabel(record)}</strong>
              <span>{T.decision.providers[record.provider as "typeSafe" | "openRouter"] ?? record.provider}</span>
              <span>{record.model}</span>
              <span>
                {T.decision.scenarioLabel} {record.suggestion ?? record.baseline ?? "—"}
              </span>
              <span>
                {T.decision.durationLabel} {record.durationMs} ms
                {record.consentWaitMs > 0 ? ` · ${T.decision.consentWaitLabel} ${record.consentWaitMs} ms` : ""}
              </span>
            </div>
            {record.probabilities && Object.keys(record.probabilities).length > 0 && (
              <div className="decision-record-probs">
                {Object.entries(record.probabilities).map(([key, value]) => (
                  <code key={key}>
                    {key} {value.toFixed(2)}
                  </code>
                ))}
              </div>
            )}
            {record.rejectedReason && (
              <div className="decision-record-head">
                <span>
                  {T.decision.rejectedLabel}{" "}
                  {T.decision.rejectedReasons[
                    record.rejectedReason as keyof typeof T.decision.rejectedReasons
                  ] ?? record.rejectedReason}
                </span>
              </div>
            )}
            {record.skipCode && (
              <div className="decision-record-head">
                <span>{T.decision.skipLabel} {record.skipCode}</span>
              </div>
            )}
            {record.fallbackCode && (
              <div className="decision-record-head">
                <span>{T.decision.fallbackLabel} {record.fallbackCode}</span>
              </div>
            )}
          </article>
        ))}
      </div>
    </Section>
  );
}
