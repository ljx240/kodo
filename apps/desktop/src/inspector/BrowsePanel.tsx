import { useEffect, useRef, useState, type FormEvent } from "react";
import { ExternalLink, RotateCw } from "lucide-react";
import { openUrl } from "../api";
import { useSetting } from "../data/useSetting";
import { T } from "../i18n";

/** http(s) only — Rust side applies the same allowlist in
 *  `parse_safe_external_url` (main.rs); the JS check is a fast first line so
 *  the panel never round-trips with `javascript:` / `data:` / `file:`. */
const SAFE_URL = /^https?:\/\/[^\s]+$/i;
/** A native <iframe> that never finishes loading within this window is treated
 *  as refused (network unreachable / server error / X-Frame-Options block).
 *  The frame itself does not fire a "load failed" event we can observe
 *  cross-origin, so the timeout is the only signal we have. */
const LOAD_TIMEOUT_MS = 10000;

type Props = {
  /** Demote back to 概览 — same Escape/X contract as the 文件 panel. */
  onClose: () => void;
};

export function BrowsePanel({ onClose }: Props) {
  const [stored, setStored] = useSetting("browse-url", "");
  const [draft, setDraft] = useState("");
  const [src, setSrc] = useState<string | null>(null);
  const [phase, setPhase] = useState<"idle" | "loading" | "loaded" | "blocked">("idle");
  const [error, setError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  const frameRef = useRef<HTMLIFrameElement | null>(null);

  // Adopt the remembered URL once, on first open; later loads are explicit.
  useEffect(() => {
    if (src != null || !SAFE_URL.test(stored.trim())) return;
    const url = stored.trim();
    setDraft(url);
    setSrc(url);
    setPhase("loading");
  }, [stored, src]);

  // Same demotion contract as the 文件 panel: Escape steps back to 概览.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // No successful load within the window → offer the system-browser fallback.
  // X-Frame-Options / CSP frame-ancestors will silently fail to render the
  // page, so a load that never resolves is the user-visible signal.
  useEffect(() => {
    if (phase !== "loading") return;
    const id = window.setTimeout(() => setPhase("blocked"), LOAD_TIMEOUT_MS);
    return () => window.clearTimeout(id);
  }, [phase, src, reloadKey]);

  const load = (raw: string) => {
    const url = raw.trim();
    if (!SAFE_URL.test(url)) {
      setError(T.inspector.browseInvalid);
      return;
    }
    setError(null);
    setStored(url);
    setSrc(url);
    setPhase("loading");
    setReloadKey((n) => n + 1);
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    load(draft);
  };

  const reload = () => {
    if (!src) return;
    setError(null);
    setPhase("loading");
    setReloadKey((n) => n + 1);
  };

  return (
    <div className="browse" data-testid="browse-panel">
      <form className="browse-bar" onSubmit={submit}>
        <input
          className="browse-input"
          data-testid="browse-url"
          type="text"
          inputMode="url"
          value={draft}
          placeholder={T.inspector.browsePlaceholder}
          aria-label={T.inspector.browseUrl}
          onChange={(event) => setDraft(event.target.value)}
        />
        <button
          type="button"
          className="browse-icon-btn"
          data-testid="browse-reload"
          title={T.inspector.browseRetry}
          aria-label={T.inspector.browseRetry}
          onClick={reload}
          disabled={!src}
        >
          <RotateCw size={14} strokeWidth={1.8} />
        </button>
        <button type="submit" className="btn btn--primary browse-go" data-testid="browse-go">
          {T.inspector.browseGo}
        </button>
      </form>

      {error && (
        <p className="browse-error" role="alert" data-testid="browse-error">
          {error}
        </p>
      )}

      {phase === "blocked" && src && (
        <div className="browse-banner" data-testid="browse-blocked">
          <span>{T.inspector.browseBlocked}</span>
          <button type="button" className="btn" onClick={() => void openUrl(src)}>
            {T.inspector.browseOpenExternal}
          </button>
          <button type="button" className="btn" data-testid="browse-retry" onClick={reload}>
            {T.inspector.browseRetry}
          </button>
        </div>
      )}

      {!src && (
        <p className="browse-empty" data-testid="browse-empty">
          {T.inspector.browseEmpty}
        </p>
      )}

      {/* Embedded iframe lives in the DOM — the user asked for the browser
          to stay inside the inspector column, so we no longer spawn an OS
          child window. Sites that block framing (X-Frame-Options / CSP
          frame-ancestors) show the empty page; the load-timeout banner +
          system-browser button above cover that case. */}
      {src && (
        <iframe
          key={`${src}#${reloadKey}`}
          ref={frameRef}
          className="browse-frame"
          data-testid="browse-frame"
          src={src}
          title={T.inspector.browse}
          // Embedded sites can run their own scripts/forms; top navigation
          // and popups remain blocked by the sandbox.
          sandbox="allow-scripts allow-same-origin allow-forms"
          referrerPolicy="no-referrer"
          onLoad={() => setPhase("loaded")}
        />
      )}

      {src && (
        <footer className="browse-foot">
          <button
            type="button"
            className="btn"
            data-testid="browse-external"
            onClick={() => void openUrl(src)}
          >
            <ExternalLink size={14} strokeWidth={1.8} />
            {T.inspector.browseOpenExternal}
          </button>
        </footer>
      )}
    </div>
  );
}
