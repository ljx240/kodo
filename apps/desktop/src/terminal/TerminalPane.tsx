import { useCallback, useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { Terminal as XTerm, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  closeTerminal,
  isDesktop,
  onTerminalData,
  onTerminalExit,
  openTerminal,
  resizeTerminal,
  writeTerminal,
  type TerminalDataEvent,
} from "../api";
import { T } from "../i18n";
import type { PaneRect } from "./paneTree";

function cssVar(style: CSSStyleDeclaration, name: string, fallback: string): string {
  return style.getPropertyValue(name).trim() || fallback;
}

const LIGHT_ANSI = {
  black: "#242424",
  red: "#b91c2a",
  green: "#116329",
  yellow: "#984308",
  blue: "#0550ae",
  magenta: "#7542b4",
  cyan: "#0f6f75",
  white: "#606060",
  brightBlack: "#8c959f",
  brightRed: "#cf222e",
  brightGreen: "#1a7f37",
  brightYellow: "#9a6700",
  brightBlue: "#218bff",
  brightMagenta: "#a475f9",
  brightCyan: "#3192aa",
  brightWhite: "#242424",
};

const DARK_ANSI = {
  black: "#484848",
  red: "#ff7b72",
  green: "#3fb950",
  yellow: "#d29922",
  blue: "#79c0ff",
  magenta: "#d2a8ff",
  cyan: "#56d4dd",
  white: "#b9b9b9",
  brightBlack: "#828282",
  brightRed: "#ffa198",
  brightGreen: "#56d364",
  brightYellow: "#e3b341",
  brightBlue: "#a5d6ff",
  brightMagenta: "#e2c5ff",
  brightCyan: "#76e3ea",
  brightWhite: "#eeeeee",
};

/** xterm paints on a canvas, so it cannot read CSS variables itself. */
function terminalTheme(): ITheme {
  const root = document.documentElement;
  const style = getComputedStyle(root);
  const dark = root.dataset.theme === "dark";
  return {
    background: cssVar(style, "--kodo-canvas", dark ? "#1c1c1c" : "#ffffff"),
    foreground: cssVar(style, "--kodo-text", dark ? "#eeeeee" : "#242424"),
    cursor: cssVar(style, "--kodo-accent", "#176b60"),
    cursorAccent: cssVar(style, "--kodo-canvas", "#ffffff"),
    selectionBackground: cssVar(style, "--kodo-selected", "#e7e7e7"),
    ...(dark ? DARK_ANSI : LIGHT_ANSI),
  };
}

type Status = "starting" | "ready" | "exited" | "error";

export type TerminalPaneProps = {
  cwd: string;
  rect: PaneRect;
  visible: boolean;
  active: boolean;
  closable: boolean;
  onFocus: () => void;
  onClose: () => void;
};

export function TerminalPane({
  cwd,
  rect,
  visible,
  active,
  closable,
  onFocus,
  onClose,
}: TerminalPaneProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionRef = useRef<string | null>(null);
  const visibleRef = useRef(visible);
  /** The async open path must not focus a pane the user has since left. */
  const activePaneRef = useRef(active);
  const [status, setStatus] = useState<Status>("starting");
  const [error, setError] = useState<string | null>(null);
  const desktop = isDesktop();
  visibleRef.current = visible;
  activePaneRef.current = active;

  const refit = useCallback(() => {
    const host = hostRef.current;
    const terminal = terminalRef.current;
    const fit = fitRef.current;
    if (!host || !terminal || !fit) return;
    if (!visibleRef.current) return;
    if (host.clientWidth === 0 || host.clientHeight === 0) return;
    try {
      fit.fit();
    } catch {
      // The pane can be mid-layout; the next observer tick retries.
      return;
    }
    const sessionId = sessionRef.current;
    if (sessionId) {
      void resizeTerminal(sessionId, terminal.cols, terminal.rows).catch(() => {});
    }
  }, []);

  useEffect(() => {
    if (!desktop) return;
    const host = hostRef.current;
    if (!host) return;

    let alive = true;
    let pending: TerminalDataEvent[] = [];
    let frame: number | null = null;
    const unlisteners: Array<() => void> = [];

    const terminal = new XTerm({
      cursorBlink: true,
      fontFamily:
        getComputedStyle(document.documentElement).getPropertyValue("--kodo-font-mono").trim() ||
        "monospace",
      fontSize: 13,
      lineHeight: 1.25,
      scrollback: 5000,
      theme: terminalTheme(),
    });
    terminalRef.current = terminal;
    const fit = new FitAddon();
    fitRef.current = fit;
    terminal.loadAddon(fit);
    terminal.open(host);
    refit();

    const scheduleSize = () => {
      if (frame !== null) cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        frame = null;
        refit();
      });
    };

    const observer = new ResizeObserver(scheduleSize);
    observer.observe(host);
    const input = terminal.onData((data) => {
      const sessionId = sessionRef.current;
      if (!sessionId) return;
      void writeTerminal(sessionId, data).catch((failure) => {
        if (alive) setError(String(failure));
      });
    });

    void (async () => {
      const offData = await onTerminalData((event) => {
        if (sessionRef.current === null) pending.push(event);
        else if (event.id === sessionRef.current) terminal.write(event.data);
      });
      if (!alive) {
        offData();
        return;
      }
      unlisteners.push(offData);

      const offExit = await onTerminalExit((event) => {
        if (event.id !== sessionRef.current) return;
        setStatus("exited");
        terminal.options.disableStdin = true;
        const code = event.code === null ? "" : ` · ${event.code}`;
        terminal.write(`\r\n\x1b[2m[${T.terminal.exited}${code}]\x1b[0m\r\n`);
      });
      if (!alive) {
        offExit();
        return;
      }
      unlisteners.push(offExit);

      try {
        const id = await openTerminal(cwd || null, terminal.cols, terminal.rows);
        if (!alive) {
          if (id) void closeTerminal(id);
          return;
        }
        if (!id) {
          setStatus("error");
          setError(T.terminal.desktopOnly);
          terminal.options.disableStdin = true;
          return;
        }
        sessionRef.current = id;
        for (const event of pending) {
          if (event.id === id) terminal.write(event.data);
        }
        pending = [];
        setStatus("ready");
        refit();
        if (activePaneRef.current) terminal.focus();
      } catch (failure) {
        if (alive) {
          setStatus("error");
          setError(String(failure));
          terminal.options.disableStdin = true;
        }
      }
    })();

    return () => {
      alive = false;
      observer.disconnect();
      if (frame !== null) cancelAnimationFrame(frame);
      input.dispose();
      for (const off of unlisteners) off();
      terminal.dispose();
      terminalRef.current = null;
      fitRef.current = null;
      const sessionId = sessionRef.current;
      sessionRef.current = null;
      if (sessionId) void closeTerminal(sessionId);
    };
  }, [desktop, cwd, refit]);

  // Deferred so a pane that just appeared (tab switch / split) fits its box.
  useEffect(() => {
    if (!desktop || !visible) return;
    const frame = requestAnimationFrame(() => {
      refit();
      if (active) terminalRef.current?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [active, desktop, refit, visible]);

  // Theme switches repaint the whole app; the canvas needs the new palette.
  useEffect(() => {
    if (!desktop) return;
    const observer = new MutationObserver(() => {
      if (terminalRef.current) terminalRef.current.options.theme = terminalTheme();
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    return () => observer.disconnect();
  }, [desktop]);

  const style = {
    left: `${rect.left}%`,
    top: `${rect.top}%`,
    width: `${rect.width}%`,
    height: `${rect.height}%`,
  };

  return (
    <div
      className={`terminal-pane${active ? " terminal-pane--active" : ""}`}
      style={style}
      data-testid="terminal-pane"
      data-active={active ? "true" : undefined}
      onPointerDown={onFocus}
      onFocusCapture={onFocus}
    >
      {desktop ? (
        <>
          <div className="terminal-host" ref={hostRef} data-testid="terminal-host" />
          {status === "error" ? (
            <div className="terminal-pane-status terminal-pane-status--error">{error}</div>
          ) : status === "starting" ? (
            <div className="terminal-pane-status">{T.terminal.starting}</div>
          ) : null}
        </>
      ) : (
        <div className="terminal-unavailable">{T.terminal.desktopOnly}</div>
      )}
      {closable && (
        <button
          type="button"
          className="icon-btn icon-btn--sm terminal-pane-close"
          aria-label={T.terminal.closePane}
          title={T.terminal.closePane}
          data-testid="terminal-pane-close"
          onPointerDown={(event) => event.stopPropagation()}
          onClick={(event) => {
            event.stopPropagation();
            onClose();
          }}
        >
          <X size={12} strokeWidth={2} />
        </button>
      )}
    </div>
  );
}
