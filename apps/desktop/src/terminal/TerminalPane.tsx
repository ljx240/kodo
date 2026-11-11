import { useEffect, useRef, useState } from "react";
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

/** xterm's theme takes resolved colors. */
function terminalTheme(): ITheme {
  const style = getComputedStyle(document.documentElement);
  const color = (name: string) => style.getPropertyValue(`--kodo-terminal-${name}`).trim();
  return {
    background: color("canvas"),
    foreground: color("text"),
    cursor: color("focus"),
    cursorAccent: color("canvas"),
    selectionBackground: color("selected"),
    black: color("ansi-black"),
    red: color("error"),
    green: color("ansi-green"),
    yellow: color("ansi-yellow"),
    blue: color("ansi-blue"),
    magenta: color("ansi-magenta"),
    cyan: color("ansi-cyan"),
    white: color("secondary"),
    brightBlack: color("ansi-bright-black"),
    brightRed: color("ansi-bright-red"),
    brightGreen: color("ansi-bright-green"),
    brightYellow: color("ansi-bright-yellow"),
    brightBlue: color("ansi-bright-blue"),
    brightMagenta: color("ansi-bright-magenta"),
    brightCyan: color("ansi-bright-cyan"),
    brightWhite: color("text"),
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
  const visibleRef = useRef(visible);
  const [status, setStatus] = useState<Status>("starting");
  const [error, setError] = useState<string | null>(null);
  const desktop = isDesktop();
  visibleRef.current = visible;

  useEffect(() => {
    if (!desktop) return;
    const host = hostRef.current;
    if (!host) return;

    let alive = true;
    let sessionId: string | null = null;
    let opening: TerminalDataEvent[] | null = null;
    let frame: number | null = null;
    const unlisteners: Array<() => void> = [];

    const terminal = new XTerm({
      fontFamily: 'Menlo, Consolas, "Liberation Mono", monospace',
      fontSize: 13,
      lineHeight: 1.25,
      scrollback: 5000,
      theme: terminalTheme(),
    });
    terminalRef.current = terminal;
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(host);
    const refit = () => {
      if (visibleRef.current && host.clientWidth > 0 && host.clientHeight > 0) fit.fit();
    };
    refit();

    const observer = new ResizeObserver(() => {
      if (!visibleRef.current || frame !== null) return;
      frame = requestAnimationFrame(() => {
        frame = null;
        refit();
      });
    });
    observer.observe(host);
    const resizeSession = ({ cols, rows }: { cols: number; rows: number }) => {
      if (sessionId && !terminal.options.disableStdin) {
        void resizeTerminal(sessionId, cols, rows).catch(() => {});
      }
    };
    const resize = terminal.onResize(resizeSession);
    const input = terminal.onData((data) => {
      if (!sessionId) return;
      void writeTerminal(sessionId, data).catch((failure) => {
        if (alive) setError(String(failure));
      });
    });

    void (async () => {
      const offData = await onTerminalData((event) => {
        if (event.id === sessionId) terminal.write(event.data);
        else opening?.push(event);
      });
      if (!alive) {
        offData();
        return;
      }
      unlisteners.push(offData);

      const offExit = await onTerminalExit((event) => {
        if (event.id !== sessionId) return;
        setStatus("exited");
        terminal.options.disableStdin = true;
        const code = event.code === null ? "" : `（${event.code}）`;
        terminal.write(`\r\n\x1b[2m[${T.terminal.exited}${code}]\x1b[0m\r\n`);
      });
      if (!alive) {
        offExit();
        return;
      }
      unlisteners.push(offExit);

      const { cols, rows } = terminal;
      const buffered: TerminalDataEvent[] = [];
      opening = buffered;
      try {
        const id = await openTerminal(cwd || null, cols, rows);
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
        sessionId = id;
        for (const event of buffered) {
          if (event.id === id) terminal.write(event.data);
        }
        setStatus("ready");
        if (cols !== terminal.cols || rows !== terminal.rows) resizeSession(terminal);
      } catch (failure) {
        if (alive) {
          setStatus("error");
          setError(String(failure));
          terminal.options.disableStdin = true;
        }
      } finally {
        opening = null;
      }
    })();

    return () => {
      alive = false;
      observer.disconnect();
      if (frame !== null) cancelAnimationFrame(frame);
      input.dispose();
      resize.dispose();
      for (const off of unlisteners) off();
      terminal.dispose();
      terminalRef.current = null;
      if (sessionId) void closeTerminal(sessionId);
      sessionId = null;
      opening = null;
    };
  }, [desktop, cwd]);

  useEffect(() => {
    if (active) terminalRef.current?.focus();
  }, [active, desktop]);

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
      className={`terminal-pane${active ? " terminal-pane--active" : ""}${closable ? " terminal-pane--split" : ""}`}
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
            <div className="terminal-pane-status terminal-pane-status--error" role="alert">
              {error}
            </div>
          ) : status === "starting" ? (
            <div className="terminal-pane-status" role="status">{T.terminal.starting}</div>
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
