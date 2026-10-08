import { useEffect, useRef, useState, type FormEvent } from "react";
import { ExternalLink } from "lucide-react";
import { browseClose, browseOpen, browseSetPosition, browseSetUrl, openUrl } from "../api";
import { useSetting } from "../data/useSetting";
import { T } from "../i18n";

/** http(s) only — Rust side applies the same allowlist in
 *  `parse_safe_external_url` (main.rs); the JS check is a fast first line so
 *  the panel never round-trips with `javascript:` / `data:` / `file:`. */
const SAFE_URL = /^https?:\/\/[^\s]+$/i;
/** A native webview that never finishes loading within this window is treated
 *  as refused (network unreachable / server error / CAPTCHA). The webview
 *  itself does not fire a "load failed" event we can observe cross-platform,
 *  so the timeout is the only signal we have. */
const LOAD_TIMEOUT_MS = 10000;
/** Drag debounce — resize the child webview once the pointer lands. */
const RESIZE_DEBOUNCE_MS = 50;

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
  /** The DOM node the embedded webview is sized to track. `.ins-content--flush`
   *  lives behind a `<div class="browse">` here, but since flex layout puts
   *  them at the same rect, observing `.browse` is equivalent — and it stays
   *  mounted across the URL bar changes that re-key the inner iframe slot. */
  const containerRef = useRef<HTMLDivElement | null>(null);

  // Adopt the remembered URL once, on first open; later loads are explicit.
  useEffect(() => {
    if (src != null || !SAFE_URL.test(stored.trim())) return;
    const url = stored.trim();
    setDraft(url);
    setSrc(url);
    setPhase("loading");
  }, [stored, src]);

  // Create or destroy the embedded webview alongside `src`. Tauri parents the
  // window to the main webview; we then size it via ResizeObserver below.
  useEffect(() => {
    if (!src) return;
    let cancelled = false;
    void (async () => {
      try {
        await browseOpen(src);
      } catch (error) {
        if (!cancelled) setError(String(error));
      }
    })();
    return () => {
      cancelled = true;
      // The webview stays parented to main; closing on teardown avoids stale
      // OS surfaces when the inspector unmounts entirely (panel switch away).
      void browseClose();
    };
  }, [src]);

  // Same demotion contract as the 文件 panel: Escape steps back to 概览.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Keep the embedded webview glued to the panel content rect. The observer
  // fires every layout tick (panel drag, focus-mode toggle, window resize);
  // debouncing keeps the IPC off the hot path.
  useEffect(() => {
    if (!src) return;
    const node = containerRef.current;
    if (!node) return;
    let pending: number | null = null;
    const measure = () => {
      pending = null;
      const rect = node.getBoundingClientRect();
      // Coords are relative to the main window's content area, which matches
      // DOMRect x/y when the OS scale factor is 1 (Tauri reports logical px
      // by default). At 2x the webview ends up a touch small; the visible
      // gap is masked by the panel's own background and resizes cleanly on
      // the next drag tick.
      void browseSetPosition(rect.left, rect.top, rect.width, rect.height);
    };
    const observer = new ResizeObserver(() => {
      if (pending != null) window.clearTimeout(pending);
      pending = window.setTimeout(measure, RESIZE_DEBOUNCE_MS);
    });
    observer.observe(node);
    // First measure is synchronous so the initial layout lands without a flash.
    measure();
    return () => {
      observer.disconnect();
      if (pending != null) window.clearTimeout(pending);
    };
  }, [src, reloadKey]);

  // No successful load within the window → offer the system-browser fallback.
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
    // The new src will re-fire the create/destroy effect above, but on the
    // *same* URL after a reload we want to force-navigate — explicit set here
    // keeps the native webview honest when only reloadKey changed.
    if (src === url) void browseSetUrl(url);
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    load(draft);
  };

  return (
    <div className="browse" data-testid="browse-panel" ref={containerRef}>
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
          <button type="button" className="btn" data-testid="browse-retry" onClick={() => load(src)}>
            {T.inspector.browseRetry}
          </button>
        </div>
      )}

      {!src && (
        <p className="browse-empty" data-testid="browse-empty">
          {T.inspector.browseEmpty}
        </p>
      )}

      {/* Native webview fills the same slot the iframe used to occupy; the
          OS-level browser is sized to match this rect by the ResizeObserver
          above. Cross-origin X-Frame-Options never reach the desktop shell. */}
      {src && (
        <div
          key={`${src}#${reloadKey}`}
          className="browse-frame"
          data-testid="browse-frame"
          aria-label={T.inspector.browse}
          role="presentation"
        />
      )}

      {/* The OS webview is opaque to X-Frame-Options, so the system-browser
          escape hatch is always available — including sites that fail to
          load (network / TLS / CAPTCHA). */}
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
