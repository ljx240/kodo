import { useState, type ReactNode } from "react";
import type { ArtifactRefDto, LlmIoDto } from "../api";
import { T } from "../i18n";

/**
 * Side-by-side viewer for a single LLM call. The left column is the request
 * the model received; the right column is what it sent. The component is
 * purely visual — the parent passes the loaded capture and the artifact list
 * to navigate between calls.
 */
export function LlmIoViewer({
  artifacts,
  activeIndex,
  onSelect,
  capture,
  loading,
}: {
  artifacts: ArtifactRefDto[];
  activeIndex: number;
  onSelect: (index: number) => void;
  capture: LlmIoDto | null;
  loading: boolean;
}) {
  return (
    <div className="llm-io">
      <CallBar
        artifacts={artifacts}
        activeIndex={activeIndex}
        onSelect={onSelect}
        capture={capture}
      />
      {capture ? (
        <CaptureBody capture={capture} loading={loading} />
      ) : (
        <p className="empty-note">{T.page.loadingTrace}</p>
      )}
    </div>
  );
}

function CallBar({
  artifacts,
  activeIndex,
  onSelect,
  capture,
}: {
  artifacts: ArtifactRefDto[];
  activeIndex: number;
  onSelect: (index: number) => void;
  capture: LlmIoDto | null;
}) {
  if (artifacts.length === 0) return null;
  const safeIndex = Math.max(0, Math.min(activeIndex, artifacts.length - 1));
  return (
    <div className="llm-io-bar">
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        disabled={safeIndex === 0}
        onClick={() => onSelect(safeIndex - 1)}
        aria-label={T.page.llmIo.prev}
      >
        ‹
      </button>
      <span className="llm-io-bar-meta">
        {T.page.llmIo.callLabel(
          artifacts[safeIndex].seq,
          capture?.model ?? "—",
        )}
        {capture && (
          <span className="muted">
            {" · "}
            {capture.protocol}
            {" · "}
            {capture.inputTokens} → {capture.outputTokens} tokens
            {" · "}
            {(capture.providerLatencyMs / 1000).toFixed(2)}s
          </span>
        )}
      </span>
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        disabled={safeIndex >= artifacts.length - 1}
        onClick={() => onSelect(safeIndex + 1)}
        aria-label={T.page.llmIo.next}
      >
        ›
      </button>
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        aria-label={T.page.llmIo.copyAll}
        onClick={() => {
          if (capture) void copyCapture(capture);
        }}
      >
        ⧉
      </button>
    </div>
  );
}

async function copyCapture(capture: LlmIoDto) {
  const text = JSON.stringify(capture, null, 2);
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    /* clipboard unavailable in tests — silently no-op */
  }
}

function CaptureBody({
  capture,
  loading,
}: {
  capture: LlmIoDto;
  loading: boolean;
}) {
  if (loading) {
    return <p className="empty-note">{T.page.loadingTrace}</p>;
  }
  return (
    <div className="llm-io-grid">
      <section className="llm-io-col">
        <h4>{T.page.llmIo.request}</h4>
        <SystemPrompt system={capture.request.system} />
        <Messages messages={capture.request.messages} />
      </section>
      <section className="llm-io-col">
        <h4>{T.page.llmIo.response}</h4>
        <ResponseText text={capture.response.text} />
        <Reasoning text={capture.response.reasoningContent} />
        <NativeCalls calls={capture.response.nativeToolCalls} />
      </section>
    </div>
  );
}

function SystemPrompt({ system }: { system: string }) {
  const [open, setOpen] = useState<boolean>(false);
  const long = system.length > 400;
  const display = !long || open ? system : `${system.slice(0, 400)}…`;
  return (
    <details className="llm-io-block" open={open || !long}>
      <summary>
        <span>{T.page.llmIo.systemPrompt}</span>
        {long && !open && (
          <button
            type="button"
            className="link-btn"
            onClick={(event) => {
              event.preventDefault();
              setOpen(true);
            }}
          >
            {T.page.llmIo.systemHidden}
          </button>
        )}
        <code className="muted mono">{system.length} chars</code>
      </summary>
      <pre className="llm-io-pre">{display}</pre>
    </details>
  );
}

function Messages({ messages }: { messages: unknown }) {
  return (
    <details className="llm-io-block">
      <summary>
        <span>{T.page.llmIo.messages}</span>
        <code className="muted mono">{summarizeMessages(messages)}</code>
      </summary>
      <pre className="llm-io-pre">{JSON.stringify(messages, null, 2)}</pre>
    </details>
  );
}

function ResponseText({ text }: { text: string }) {
  if (!text) return null;
  return (
    <details className="llm-io-block" open>
      <summary>
        <span>{T.page.llmIo.response}</span>
        <code className="muted mono">{text.length} chars</code>
      </summary>
      <pre className="llm-io-pre">{text}</pre>
    </details>
  );
}

function Reasoning({ text }: { text: string }) {
  if (!text) return null;
  return (
    <details className="llm-io-block">
      <summary>
        <span>{T.page.llmIo.reasoning}</span>
        <code className="muted mono">{text.length} chars</code>
      </summary>
      <pre className="llm-io-pre">{text}</pre>
    </details>
  );
}

function NativeCalls({ calls }: { calls: unknown }) {
  const arr = Array.isArray(calls) ? calls : [];
  if (arr.length === 0) return null;
  return (
    <details className="llm-io-block" open>
      <summary>
        <span>{T.page.llmIo.tools}</span>
        <code className="muted mono">{arr.length}</code>
      </summary>
      <pre className="llm-io-pre">{JSON.stringify(calls, null, 2)}</pre>
    </details>
  );
}

function summarizeMessages(messages: unknown): ReactNode {
  if (!Array.isArray(messages)) {
    return "—";
  }
  const count = messages.length;
  const roles = new Map<string, number>();
  for (const m of messages) {
    if (m && typeof m === "object" && "role" in (m as Record<string, unknown>)) {
      const role = String((m as Record<string, unknown>).role);
      roles.set(role, (roles.get(role) ?? 0) + 1);
    }
  }
  const parts: string[] = [];
  for (const [role, n] of roles) parts.push(`${role}×${n}`);
  return `${count} 条${parts.length ? `（${parts.join(" / ")}）` : ""}`;
}