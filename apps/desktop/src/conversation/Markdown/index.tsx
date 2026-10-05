import { Check, Copy, Info, Lightbulb, Maximize2, OctagonAlert, TriangleAlert } from "lucide-react";
import {
  Fragment,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react";
import { openUrl } from "../../api";
import { looksLikePath } from "../../fileKind";
import { highlightLines, resolveLanguage } from "../../highlight";
import { T } from "../../i18n";
import {
  parseBlocks,
  type Block,
  type CalloutKind,
  type ColAlign,
  type ListItem,
  type QuoteLine,
} from "./parser";

/**
 * Compact conversation markdown.
 *
 * COMPONENTS.md asks FinalResponse to render markdown with mono code blocks.
 * The conversation column is IDE-dense, so this stays a small local parser —
 * see parser.ts for the block grammar. Long code folds behind 显示完整代码,
 * many headings get a small TOC, formulas (KaTeX) and diagrams (mermaid) lazy
 * load so the answer never waits on them, and anything that fails to render
 * falls back to its literal source.
 */
/** "Open this referenced file in the sidebar" — threaded to inline code. */
export type OpenPath = (path: string) => void;

export function Markdown({ text, onOpenPath = null }: { text: string; onOpenPath?: OpenPath | null }) {
  const blocks = useMemo(() => parseBlocks(text), [text]);
  const headings = useMemo(
    () =>
      blocks.flatMap((block, index) =>
        block.kind === "heading" ? [{ index, text: block.text, id: headingId(index) }] : [],
      ),
    [blocks],
  );
  // Footnote refs resolve against this instance's definitions; `fnIds` is
  // threaded through rendering so a ref without a definition stays literal.
  const footnotes = useMemo(
    () =>
      blocks.flatMap((block) =>
        block.kind === "footnote" ? [{ id: block.id, text: block.text }] : [],
      ),
    [blocks],
  );
  const fnIds = useMemo(() => new Set(footnotes.map((footnote) => footnote.id)), [footnotes]);
  const showToc = headings.length >= 4;
  // Scroll-follow: track which headings sit in the reading band and light up
  // the first one in document order. IO with the viewport root works even
  // though the conversation scrolls in an inner container.
  const [activeHeading, setActiveHeading] = useState<string | null>(null);
  useEffect(() => {
    if (!showToc || typeof IntersectionObserver === "undefined") return;
    const visible = new Set<string>();
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) visible.add(entry.target.id);
          else visible.delete(entry.target.id);
        }
        const active = headings.find((heading) => visible.has(heading.id));
        setActiveHeading(active ? active.id : null);
      },
      // Band = top 30% of the viewport: the container's top edge sits below
      // the topbar (~60px), so a heading jumped to block:"start" must land
      // inside the band, or the entry it came from never lights up.
      { root: null, rootMargin: "0px 0px -70% 0px" },
    );
    for (const heading of headings) {
      const element = document.getElementById(heading.id);
      if (element) observer.observe(element);
    }
    return () => observer.disconnect();
  }, [showToc, headings]);
  // Anchors can't hash-jump: the conversation scrolls in an inner container,
  // so the click scrolls the target into view itself.
  const jump = (event: MouseEvent, id: string) => {
    event.preventDefault();
    document.getElementById(id)?.scrollIntoView({ block: "start" });
  };

  return (
    <div className="md">
      {showToc && (
        <nav className="md-toc" aria-label={T.reply.toc} data-testid="md-toc">
          <span className="md-toc-title">{T.reply.toc}</span>
          <ol className="md-toc-list">
            {headings.map((heading) => (
              <li key={heading.id}>
                <a
                  href={`#${heading.id}`}
                  className={heading.id === activeHeading ? "md-toc-active" : undefined}
                  onClick={(event) => jump(event, heading.id)}
                >
                  {heading.text}
                </a>
              </li>
            ))}
          </ol>
        </nav>
      )}
      {blocks.map((block, index) => renderBlock(block, index, headingId(index), fnIds, onOpenPath))}
      {footnotes.length > 0 && (
        <div className="md-footnotes" data-testid="md-footnotes">
          <div className="md-footnotes-title">{T.reply.footnotes}</div>
          {footnotes.map((footnote) => (
            <div className="md-footnote" key={footnote.id} id={`md-fn-${footnote.id}`}>
              <span className="md-footnote-marker">[{footnote.id}]</span>{" "}
              {inline(footnote.text, true, fnIds, onOpenPath)}{" "}
              <a
                className="md-footnote-back"
                href={`#md-fnr-${footnote.id}`}
                aria-label={T.reply.footnoteBack}
                onClick={(event) => jump(event, `md-fnr-${footnote.id}`)}
              >
                ↩
              </a>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function headingId(index: number): string {
  return `md-h-${index}`;
}

const CODE_FOLD_LINES = 10;
const CODE_SHOWN_LINES = 6;

function CodeBlock({ body, lang }: { body: string; lang: string }) {
  const preId = useId();
  const lines = body.split("\n");
  const long = lines.length > CODE_FOLD_LINES;
  // Long code reads as a summary first: the head of the block, then the rest
  // only when asked for.
  const [expanded, setExpanded] = useState(false);
  // Copy always hands over the full block, never the folded summary.
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<number | undefined>(undefined);
  const shown = !long || expanded ? body : lines.slice(0, CODE_SHOWN_LINES).join("\n");
  // Syntax color per fence header; null → plain (unknown lang, oversized, hljs
  // failure). Each highlighted line is standalone-balanced, so joining with
  // newlines reproduces the original layout exactly.
  const html = useMemo(
    () => highlightLines(shown, resolveLanguage(lang))?.join("\n") ?? null,
    [shown, lang],
  );

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(body);
      setCopied(true);
      window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard denied — the code stays selectable */
    }
  };

  return (
    <div className="md-code-wrap">
      <div className="md-code-head">
        {lang && <span className="md-code-lang">{lang}</span>}
        <span className="spacer" />
        <button
          type="button"
          className="md-code-copy"
          aria-label={copied ? T.action.copied : T.action.copyCode}
          title={copied ? T.action.copied : T.action.copyCode}
          data-testid="md-code-copy"
          onClick={() => void copy()}
        >
          {copied ? <Check size={12} strokeWidth={2.2} /> : <Copy size={12} strokeWidth={1.8} />}
          <span>{copied ? T.action.copied : T.action.copyCode}</span>
        </button>
      </div>
      <pre className="md-code" id={preId} data-lang={lang || undefined}>
        {html !== null ? (
          <code dangerouslySetInnerHTML={{ __html: html }} />
        ) : (
          <code>{shown}</code>
        )}
      </pre>
      {long && (
        <button
          type="button"
          className="md-code-toggle"
          aria-expanded={expanded}
          aria-controls={preId}
          data-testid="md-code-toggle"
          onClick={() => setExpanded((value) => !value)}
        >
          {expanded ? T.reply.hideCode : T.reply.showCode}
        </button>
      )}
    </div>
  );
}

let mermaidPromise: Promise<typeof import("mermaid").default> | null = null;

/** One shared, initialized mermaid — loaded only when an answer has a diagram. */
function loadMermaid(): Promise<typeof import("mermaid").default> {
  if (!mermaidPromise) {
    mermaidPromise = import("mermaid")
      .then((module) => {
        const mermaid = module.default;
        mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: "neutral" });
        return mermaid;
      })
      .catch((error) => {
        mermaidPromise = null;
        throw error;
      });
  }
  return mermaidPromise;
}

/** Zoom steps for the fullscreen diagram lightbox. */
const ZOOM_STEPS = [0.5, 0.75, 1, 1.5, 2, 3];

/** ```mermaid fence → SVG. Loading shows a quiet note; any failure (bad
 *  diagram, chunk load) drops back to the plain code block — zero regression.
 *  A hover toolbar offers source view, copy and a zoomable fullscreen lightbox. */
function MermaidBlock({ body }: { body: string }) {
  const [state, setState] = useState<"loading" | "svg" | "failed">("loading");
  const [svg, setSvg] = useState("");
  const [showSource, setShowSource] = useState(false);
  const [copied, setCopied] = useState(false);
  const [lightbox, setLightbox] = useState(false);
  const [zoomIndex, setZoomIndex] = useState(2);
  const reactId = useId().replace(/:/g, "");
  const attempt = useRef(0);
  const copyTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const mermaid = await loadMermaid();
        if (!alive) return;
        // Parse first: a bad diagram then rejects without mermaid injecting
        // its error SVG into the page.
        await mermaid.parse(body);
        if (!alive) return;
        attempt.current += 1;
        const { svg: rendered } = await mermaid.render(`md-mermaid-${reactId}-${attempt.current}`, body);
        if (!alive) return;
        setSvg(rendered);
        setState("svg");
      } catch {
        if (alive) setState("failed");
      }
    })();
    return () => {
      alive = false;
    };
  }, [body, reactId]);

  // Escape closes the lightbox while it is open (same mechanics as the
  // FileViewer dialog): stop there so the rest of the page stays untouched.
  useEffect(() => {
    if (!lightbox) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setLightbox(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [lightbox]);

  if (state === "failed") return <CodeBlock body={body} lang="mermaid" />;
  if (state === "loading") {
    return <div className="md-mermaid-loading">{T.reply.mermaidLoading}</div>;
  }

  const copySource = async () => {
    try {
      await navigator.clipboard.writeText(body);
      setCopied(true);
      window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard denied — source stays selectable */
    }
  };

  const diagram = <div className="md-mermaid" dangerouslySetInnerHTML={{ __html: svg }} />;
  const zoom = ZOOM_STEPS[zoomIndex];

  return (
    <div className="md-mermaid-block">
      <div className="md-mermaid-toolbar">
        <button
          type="button"
          className="md-mermaid-btn"
          aria-pressed={showSource}
          data-testid="md-mermaid-source"
          onClick={() => setShowSource((value) => !value)}
        >
          {showSource ? T.reply.mermaidDiagram : T.reply.mermaidSource}
        </button>
        <button
          type="button"
          className="md-mermaid-btn"
          data-testid="md-mermaid-copy"
          onClick={() => void copySource()}
        >
          {copied ? T.action.copied : T.action.copyCode}
        </button>
        <button
          type="button"
          className="md-mermaid-btn"
          data-testid="md-mermaid-fullscreen"
          onClick={() => setLightbox(true)}
        >
          <Maximize2 size={12} strokeWidth={1.8} />
          <span>{T.reply.fullscreen}</span>
        </button>
      </div>
      {showSource ? <CodeBlock body={body} lang="mermaid" /> : diagram}
      {lightbox && (
        <div
          className="md-lightbox"
          role="dialog"
          aria-modal="true"
          aria-label={T.reply.fullscreen}
          data-testid="md-lightbox"
          onClick={(event) => {
            if (event.target === event.currentTarget) setLightbox(false);
          }}
        >
          <div className="md-lightbox-bar">
            <button
              type="button"
              className="md-mermaid-btn"
              aria-label={T.reply.zoomOut}
              data-testid="md-lightbox-out"
              disabled={zoomIndex === 0}
              onClick={() => setZoomIndex((value) => Math.max(0, value - 1))}
            >
              −
            </button>
            <span className="md-lightbox-zoom" data-zoom={zoom}>
              {Math.round(zoom * 100)}%
            </span>
            <button
              type="button"
              className="md-mermaid-btn"
              aria-label={T.reply.zoomIn}
              data-testid="md-lightbox-in"
              disabled={zoomIndex === ZOOM_STEPS.length - 1}
              onClick={() => setZoomIndex((value) => Math.min(ZOOM_STEPS.length - 1, value + 1))}
            >
              +
            </button>
            <span className="spacer" />
            <button
              type="button"
              className="md-mermaid-btn"
              data-testid="md-lightbox-close"
              onClick={() => setLightbox(false)}
            >
              {T.action.close}
            </button>
          </div>
          {/* CSS zoom (not transform) keeps the scroll area in sync at 3×. */}
          <div
            className="md-lightbox-stage"
            style={{ zoom }}
            dangerouslySetInnerHTML={{ __html: svg }}
          />
        </div>
      )}
    </div>
  );
}

let katexPromise: Promise<typeof import("katex")> | null = null;

/** KaTeX module + stylesheet, loaded only once a formula appears. */
function loadKatex(): Promise<typeof import("katex")> {
  if (!katexPromise) {
    katexPromise = (async () => {
      await import("katex/dist/katex.min.css");
      return import("katex");
    })().catch((error) => {
      katexPromise = null;
      throw error;
    });
  }
  return katexPromise;
}

/** `$tex$` / `$$tex$$` → KaTeX. While loading — or when the TeX is invalid —
 *  the literal source stays on screen, so a broken formula is still readable. */
function Formula({ tex, display }: { tex: string; display: boolean }) {
  const [html, setHtml] = useState<string | null>(null);
  // Copy sits on block formulas only — a button inside flowing inline text
  // would break line geometry.
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    loadKatex()
      .then((katex) => {
        const rendered = katex.renderToString(tex, { displayMode: display, throwOnError: true });
        if (alive) setHtml(rendered);
      })
      .catch(() => {
        /* literal fallback stays */
      });
    return () => {
      alive = false;
    };
  }, [tex, display]);

  if (html !== null) {
    if (!display) return <span dangerouslySetInnerHTML={{ __html: html }} />;
    const copy = async () => {
      try {
        await navigator.clipboard.writeText(tex);
        setCopied(true);
        window.clearTimeout(copyTimer.current);
        copyTimer.current = window.setTimeout(() => setCopied(false), 1200);
      } catch {
        /* clipboard denied — the source stays selectable */
      }
    };
    return (
      <div className="md-formula-block md-formula-wrap">
        <span dangerouslySetInnerHTML={{ __html: html }} />
        <button
          type="button"
          className="md-formula-copy"
          aria-label={copied ? T.action.copied : T.action.copyLatex}
          title={copied ? T.action.copied : T.action.copyLatex}
          data-testid="md-formula-copy"
          onClick={() => void copy()}
        >
          {copied ? T.action.copied : T.action.copyLatex}
        </button>
      </div>
    );
  }
  return <>{display ? `$$${tex}$$` : `$${tex}$`}</>;
}

const SAFE_URL = /^(https?:\/\/|mailto:)/i;

function ExternalLink({ href, label }: { href: string; label: ReactNode }) {
  // Only the webview-safe schemes become links; anything else stays text.
  if (!SAFE_URL.test(href)) return <>{label}</>;
  return (
    <a
      className="md-link"
      href={href}
      onClick={(event) => {
        // Never let the webview navigate: the system browser opens it instead.
        event.preventDefault();
        void openUrl(href);
      }}
    >
      {label}
    </a>
  );
}

function Table({
  header,
  align,
  rows,
  fnIds,
  onOpenPath = null,
}: {
  header: string[];
  align: ColAlign[];
  rows: string[][];
  fnIds: ReadonlySet<string>;
  onOpenPath?: OpenPath | null;
}) {
  const style = (column: number) =>
    align[column] ? { textAlign: align[column] as "left" | "center" | "right" } : undefined;
  return (
    <div className="md-table-wrap">
      <table className="md-table">
        <thead>
          <tr>
            {header.map((cell, column) => (
              <th key={column} style={style(column)}>
                {inline(cell, true, fnIds, onOpenPath)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, rowIndex) => (
            <tr key={rowIndex}>
              {header.map((_, column) => (
                <td key={column} style={style(column)}>
                  {inline(row[column] ?? "", true, fnIds, onOpenPath)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Flat depth-tagged items → nested list DOM. Depths are pre-clamped so a
 *  child sits exactly one level below its parent. */
function buildList(
  items: ListItem[],
  cursor: { i: number },
  depth: number,
  fnIds: ReadonlySet<string>,
  onOpenPath: OpenPath | null = null,
): ReactNode {
  const first = items[cursor.i];
  const ListTag = first.ordered ? "ol" : "ul";
  // `5. 6.` keeps its original numbering; `<ol start>` only when it differs.
  const start =
    first.ordered && first.start !== null && first.start !== 1 ? first.start : undefined;
  const nodes: ReactNode[] = [];
  while (cursor.i < items.length && items[cursor.i].depth === depth) {
    const item = items[cursor.i];
    cursor.i += 1;
    const hasChildren = cursor.i < items.length && items[cursor.i].depth > depth;
    const children = hasChildren ? buildList(items, cursor, depth + 1, fnIds, onOpenPath) : null;
    nodes.push(
      <li key={nodes.length} className={item.task ? "md-task" : undefined}>
        {item.task ? (
          <input type="checkbox" checked={item.task === "done"} disabled readOnly />
        ) : null}
        {inline(item.text, true, fnIds, onOpenPath)}
        {children}
      </li>,
    );
  }
  return (
    <ListTag className="md-list" start={start}>
      {nodes}
    </ListTag>
  );
}

/** Indent jumps (0 → 4) collapse to one level at a time. */
function clampDepths(items: ListItem[]): ListItem[] {
  let prev = -1;
  return items.map((item) => {
    prev = Math.min(item.depth, prev + 1);
    return { ...item, depth: prev };
  });
}

const CALLOUT_ICON: Record<CalloutKind, typeof Info> = {
  note: Info,
  tip: Lightbulb,
  warning: TriangleAlert,
  important: OctagonAlert,
};

/** Quote lines at `depth` become paragraphs; deeper runs nest as child
 *  blockquotes. The cursor advances in place so siblings share the walk. */
function quoteChildren(
  lines: QuoteLine[],
  cursor: { i: number },
  depth: number,
  fnIds: ReadonlySet<string>,
  onOpenPath: OpenPath | null = null,
): ReactNode[] {
  const nodes: ReactNode[] = [];
  let paragraph: string[] = [];
  const flush = () => {
    if (paragraph.length) {
      nodes.push(
        <p key={nodes.length}>{inline(paragraph.join("\n"), true, fnIds, onOpenPath)}</p>,
      );
      paragraph = [];
    }
  };
  while (cursor.i < lines.length) {
    const line = lines[cursor.i];
    if (line.depth < depth) break;
    if (line.depth > depth) {
      flush();
      nodes.push(
        <blockquote key={nodes.length} className="md-quote">
          {quoteChildren(lines, cursor, line.depth, fnIds, onOpenPath)}
        </blockquote>,
      );
      continue;
    }
    cursor.i += 1;
    if (line.text === null) flush();
    else paragraph.push(line.text);
  }
  flush();
  return nodes;
}

function renderBlock(
  block: Block,
  index: number,
  id: string,
  fnIds: ReadonlySet<string>,
  onOpenPath: OpenPath | null = null,
): ReactNode {
  switch (block.kind) {
    case "code":
      if (block.lang.toLowerCase() === "mermaid") {
        return <MermaidBlock key={index} body={block.body} />;
      }
      return <CodeBlock key={index} body={block.body} lang={block.lang} />;
    case "heading": {
      // h1–h6 input → three rendered tiers (h3/h4/h5): clearly above the 14px
      // body, still inside the compact IDE type scale.
      const tier = block.level <= 2 ? 1 : block.level <= 4 ? 2 : 3;
      const Tag = (["h3", "h4", "h5"][tier - 1] as "h3" | "h4" | "h5");
      return (
        <Tag key={index} className={`md-heading md-heading--${tier}`} id={id}>
          {inline(block.text, true, fnIds, onOpenPath)}
        </Tag>
      );
    }
    case "list":
      return (
        <Fragment key={index}>
          {buildList(clampDepths(block.items), { i: 0 }, 0, fnIds, onOpenPath)}
        </Fragment>
      );
    case "quote": {
      const children =
        block.lines.length > 0
          ? quoteChildren(block.lines, { i: 0 }, block.lines[0].depth, fnIds, onOpenPath)
          : [];
      if (block.callout) {
        const Icon = CALLOUT_ICON[block.callout];
        return (
          <blockquote
            key={index}
            className="md-quote md-callout"
            data-callout={block.callout}
          >
            <div className="md-callout-head">
              <Icon size={13} strokeWidth={1.8} />
              <span className="md-callout-label">{T.reply.callout[block.callout]}</span>
            </div>
            {children}
          </blockquote>
        );
      }
      return (
        <blockquote key={index} className="md-quote">
          {children}
        </blockquote>
      );
    }
    case "footnote":
      // Definitions are hoisted to the end of the answer by `Markdown`.
      return null;
    case "formula":
      return <Formula key={index} tex={block.tex} display />;
    case "table":
      return (
        <Table
          key={index}
          header={block.header}
          align={block.align}
          rows={block.rows}
          fnIds={fnIds}
          onOpenPath={onOpenPath}
        />
      );
    case "hr":
      return <hr key={index} className="md-hr" />;
    default:
      return (
        <p key={index} className="md-para">
          {inline(block.text, true, fnIds, onOpenPath)}
        </p>
      );
  }
}

// Named groups keep the two variants (with/without link rules) aligned; the
// math guard refuses spaces at the delimiters and a digit after `$`, so
// currency like `$5 and $10` stays literal text. Order matters: escapes win
// first so `\*not em\*` stays literal; underscore italics require word
// boundaries so snake_case identifiers never turn into `<em>`.
const INLINE_BASE =
  "(?<escape>\\\\[\\\\`*_{}\\[\\]()#+\\-.!>~=$|])" +
  "|(?<code>`[^`]+`)" +
  "|(?<del>~~[^~]+~~)" +
  "|(?<mark>==[^=]+==)" +
  "|(?<strong>\\*\\*[^*]+\\*\\*)" +
  "|(?<strongU>__[^_]+__)" +
  "|(?<em>\\*[^*]+\\*)" +
  "|(?<emU>(?<!\\w)_[^_\\s](?:[^_]*[^_\\s])?_(?!\\w))";
// Footnote refs ride the links-gated channel so a label can never nest an
// `<a>` inside an `<a>`.
const INLINE_LINK =
  "|(?<link>\\[[^\\]]+\\]\\([^\\s)]+\\))" +
  "|(?<footnote>\\[\\^[^\\]]+\\])";
const INLINE_TAIL =
  "|(?<url>https?:\\/\\/[^\\s<]+)" +
  "|(?<www>www\\.[^\\s<]+\\.[^\\s<]+)" +
  "|(?<email>[\\w.+-]+@[\\w-]+\\.[A-Za-z]{2,})" +
  "|(?<!\\\\)\\$(?<math>[^$\\s](?:[^$]*[^$\\s])?)\\$(?!\\d)";

/** Trailing sentence punctuation is text, not part of a bare URL —
 *  ASCII and the common CJK stops (`。` etc.) alike. */
function trimUrlPunctuation(token: string): string {
  return token.replace(/[.,;:!?'")\]。，、；：！？…]+$/u, "");
}

/** Code / bold / italic / strikethrough / highlight / links / footnotes /
 *  bare URLs / `$math$`. `links: false` runs over link labels so a label can
 *  never nest an `<a>` inside an `<a>`. A `[^id]` ref only becomes a link
 *  when `fnIds` holds its definition; otherwise it stays literal text.
 *  `onOpenPath` turns path-shaped code spans into sidebar-open buttons. */
function inline(
  text: string,
  links = true,
  fnIds: ReadonlySet<string> = new Set(),
  onOpenPath: OpenPath | null = null,
): ReactNode[] {
  const parts: ReactNode[] = [];
  const pattern = new RegExp(
    INLINE_BASE + (links ? INLINE_LINK : "") + INLINE_TAIL,
    "g",
  );
  let last = 0;
  let match: RegExpExecArray | null;
  let key = 0;

  while ((match = pattern.exec(text)) !== null) {
    if (match.index > last) parts.push(text.slice(last, match.index));
    const token = match[0];
    const groups = match.groups ?? {};
    if (groups.escape) {
      // Backslash escape → the literal character it shields.
      parts.push(token.slice(1));
    } else if (groups.code) {
      const value = token.slice(1, -1);
      // Path-shaped code opens the sidebar file panel; everything else
      // stays plain code. The click still probes before the panel opens.
      parts.push(
        onOpenPath && looksLikePath(value) ? (
          <button
            key={key++}
            type="button"
            className="md-inline-code md-code-path"
            data-testid="md-code-path"
            onClick={() => onOpenPath(value)}
          >
            {value}
          </button>
        ) : (
          <code key={key++} className="md-inline-code">
            {value}
          </code>
        ),
      );
    } else if (groups.del) {
      parts.push(<del key={key++}>{token.slice(2, -2)}</del>);
    } else if (groups.mark) {
      parts.push(<mark key={key++}>{token.slice(2, -2)}</mark>);
    } else if (groups.strong || groups.strongU) {
      parts.push(<strong key={key++}>{token.slice(2, -2)}</strong>);
    } else if (groups.em || groups.emU) {
      parts.push(<em key={key++}>{token.slice(1, -1)}</em>);
    } else if (groups.link) {
      const parsed = /^\[([^\]]+)\]\(([^\s)]+)\)$/.exec(token);
      if (parsed) {
        parts.push(
          <ExternalLink
            key={key++}
            href={parsed[2]}
            label={inline(parsed[1], false, fnIds, onOpenPath)}
          />,
        );
        last = match.index + token.length;
        continue;
      }
      parts.push(token);
    } else if (groups.footnote) {
      const id = token.slice(2, -1);
      if (!fnIds.has(id)) {
        parts.push(token);
      } else {
        parts.push(
          <sup key={key++} className="md-fnref">
            <a
              className="md-link"
              id={`md-fnr-${id}`}
              href={`#md-fn-${id}`}
              onClick={(event) => {
                event.preventDefault();
                document.getElementById(`md-fn-${id}`)?.scrollIntoView({ block: "center" });
              }}
            >
              {token}
            </a>
          </sup>,
        );
      }
    } else if (groups.url || groups.www) {
      const label = trimUrlPunctuation(token);
      const href = groups.www ? `https://${label}` : label;
      parts.push(<ExternalLink key={key++} href={href} label={label} />);
      if (label.length < token.length) {
        parts.push(token.slice(label.length));
      }
      last = match.index + label.length;
      continue;
    } else if (groups.email) {
      parts.push(<ExternalLink key={key++} href={`mailto:${token}`} label={token} />);
    } else if (groups.math) {
      parts.push(<Formula key={key++} tex={groups.math} display={false} />);
    }
    last = match.index + token.length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts.map((node, nodeIndex) => <Fragment key={nodeIndex}>{node}</Fragment>);
}
