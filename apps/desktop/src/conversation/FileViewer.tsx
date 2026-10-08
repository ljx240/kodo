import { Check, ChevronRight, Copy, ExternalLink, FolderTree } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  listProjectFiles,
  openProjectFile,
  probeContextFile,
  readProjectFile,
  revealFile,
  type ChangeDto,
  type ContextProbe,
} from "../api";
import { fileLook, formatSize, kindFromPath, previewRoute } from "../fileKind";
import { highlightLines, languageForPath } from "../highlight";
import { T } from "../i18n";
import { parseDiff, UnifiedLine, type DiffRow } from "./diff";
import { Markdown } from "./Markdown";

type Props = {
  projectPath: string;
  /** Unified diffs of this turn's changes, keyed by project-relative path. */
  diffs: Record<string, string>;
  /** This turn's merged changes — tree badges show +n -m from here. */
  changed: ChangeDto[];
  initialPath: string | null;
  initialTab: "full" | "diff";
  /** Bumps per open request so re-clicking the same path re-syncs selection. */
  nonce?: number;
  /** Narrow column / narrow window: the tree folds into a left drawer. */
  compact?: boolean;
  /** Demotes the panel to Inspector (the panel chrome owns real closing). */
  onClose: () => void;
};

type TreeNode = {
  name: string;
  path: string;
  /** Sub-nodes for a directory; null marks a file leaf. */
  children: TreeNode[] | null;
};

type Tab = "full" | "preview" | "diff";

/** Nested tree from flat project-relative paths; dirs first, then files, A→Z. */
function buildTree(paths: string[]): TreeNode[] {
  const root: TreeNode[] = [];
  for (const path of paths) {
    const parts = path.split("/").filter(Boolean);
    if (parts.length === 0) continue;
    let level = root;
    let acc = "";
    parts.forEach((part, index) => {
      acc = acc ? `${acc}/${part}` : part;
      const isFile = index === parts.length - 1;
      let node = level.find((item) => item.name === part && item.children !== null && !isFile)
        ?? level.find((item) => item.name === part && item.children === null && isFile);
      if (!node) {
        node = { name: part, path: acc, children: isFile ? null : [] };
        level.push(node);
      }
      if (!isFile && node.children) level = node.children;
    });
  }
  const sortLevel = (nodes: TreeNode[]) => {
    nodes.sort((a, b) => {
      const aDir = a.children !== null;
      const bDir = b.children !== null;
      if (aDir !== bDir) return aDir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    for (const node of nodes) if (node.children) sortLevel(node.children);
  };
  sortLevel(root);
  return root;
}

/**
 * Which pane a file opens on: its change content whenever the file has a
 * change to show, else the rendered preview (markdown/HTML for text, the
 * kind pane for image/PDF/Office/binary), else raw source.
 */
function pickTab(path: string | null, preferDiff: boolean, canDiff: (p: string) => boolean): Tab {
  if (!path) return "full";
  if (preferDiff && canDiff(path)) return "diff";
  if (previewRoute(path, kindFromPath(path))) return "preview";
  return "full";
}

/** A body with real +/− rows — `@@ no changes @@` stubs do not count. */
function diffHasBody(diff: string): boolean {
  for (const line of diff.split("\n")) {
    if (line.startsWith("+++ ") || line.startsWith("--- ")) continue;
    if (line.startsWith("+") || line.startsWith("-")) return true;
  }
  return false;
}

/**
 * Fallback body for a create whose diff was never recorded: the file's lines
 * are exactly what the turn added, so they render as additions.
 */
function addOnlyDiff(path: string, content: string): string {
  const lines = content.split("\n");
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  const body = lines.map((line) => `+${line}`).join("\n");
  return `--- /dev/null\n+++ b/${path}\n@@ -0,0 +${lines.length} @@\n${body}\n`;
}

/**
 * Codex-style file browser for one project: a tree on the left, and on the
 * right the full source, a rendered preview (markdown/HTML), or this turn's
 * diff. A right-sidebar panel body (no modal shell) — live sessions only; the
 * demo fixture carries no file bodies, so demo review keeps the Inspector.
 */
export function FileViewer({
  projectPath,
  diffs,
  changed,
  initialPath,
  initialTab,
  nonce,
  compact = false,
  onClose,
}: Props) {
  const [paths, setPaths] = useState<string[] | null>(null);
  const [selected, setSelected] = useState<string | null>(initialPath);
  const [tab, setTab] = useState<Tab>(() =>
    pickTab(initialPath, initialTab === "diff", (path) =>
      changed.some((item) => item.path === path) || Boolean(diffs[path]),
    ),
  );
  /** Compact drawer visibility — auto-closes on file select. */
  const [treeOpen, setTreeOpen] = useState(false);

  // The panel stays mounted while requests flow in (a changed-file row clicked
  // while already open): re-sync selection/tab per request identity — the
  // useState initializers above run only on mount. Same path re-clicked still
  // lands because the nonce bumps every ask.
  const lastRequest = useRef({ path: initialPath, tab: initialTab, nonce });
  useEffect(() => {
    if (
      lastRequest.current.path === initialPath &&
      lastRequest.current.tab === initialTab &&
      lastRequest.current.nonce === nonce
    ) {
      return;
    }
    lastRequest.current = { path: initialPath, tab: initialTab, nonce };
    setSelected(initialPath);
    setTab(
      pickTab(initialPath, initialTab === "diff", (path) =>
        changed.some((item) => item.path === path) || Boolean(diffs[path]),
      ),
    );
    // Reveal the requested file: seed its parent directories as expanded.
    if (initialPath) {
      setExpanded((current) => {
        const next = new Set(current);
        const parts = initialPath.split("/");
        for (let i = 1; i < parts.length; i += 1) next.add(parts.slice(0, i).join("/"));
        return next;
      });
    }
  }, [initialPath, initialTab, nonce]);
  const [content, setContent] = useState<string | null>(null);
  /** Authoritative kind/size/preview from the probe — set before any read. */
  const [probe, setProbe] = useState<ContextProbe | null>(null);
  const [contentError, setContentError] = useState(false);
  const [retryKey, setRetryKey] = useState(0);
  const [filter, setFilter] = useState("");
  const [onlyChanged, setOnlyChanged] = useState(false);
  const [diffMode, setDiffMode] = useState<"unified" | "split">("unified");
  /** Header copy-path flip — same 1.2s Check pattern as the reply actions. */
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
    };
  }, []);

  const absoluteOf = (path: string) => `${projectPath.replace(/\/+$/, "")}/${path}`;

  const copyPath = async () => {
    if (!selected || copied) return;
    try {
      await navigator.clipboard.writeText(absoluteOf(selected));
      setCopied(true);
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard denied — silent; the path stays selectable in the header */
    }
  };
  const [openFolds, setOpenFolds] = useState<Set<number>>(new Set());
  const [expanded, setExpanded] = useState<Set<string>>(() => {
    const init = new Set<string>();
    const seed = (path: string | null) => {
      if (!path) return;
      const parts = path.split("/");
      for (let i = 1; i < parts.length; i += 1) init.add(parts.slice(0, i).join("/"));
    };
    seed(initialPath);
    for (const change of changed) seed(change.path);
    return init;
  });

  useEffect(() => {
    let alive = true;
    void listProjectFiles(projectPath, undefined, 5000)
      .then((list) => {
        if (alive) setPaths(list);
      })
      .catch(() => {
        if (alive) setPaths([]);
      });
    return () => {
      alive = false;
    };
  }, [projectPath]);

  // Probe first: kind decides whether a read even happens — non-text files
  // hit the backend's binary refusal, so they route straight to their preview
  // pane instead of an error. A probe or read failure shares the retry path.
  useEffect(() => {
    if (!selected) {
      setContent(null);
      setProbe(null);
      setContentError(false);
      return;
    }
    let alive = true;
    setContent(null);
    setProbe(null);
    setContentError(false);
    void probeContextFile(projectPath, selected)
      .then((info) => {
        if (!alive) return;
        setProbe(info);
        if (info.kind !== "text") return;
        return readProjectFile(projectPath, selected).then((text) => {
          if (alive) setContent(text);
        });
      })
      .catch(() => {
        if (alive) setContentError(true);
      });
    return () => {
      alive = false;
    };
  }, [projectPath, selected, retryKey]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const turnPaths = useMemo(() => {
    const set = new Set<string>();
    for (const change of changed) set.add(change.path);
    return set;
  }, [changed]);

  const query = filter.trim().toLowerCase();
  const visiblePaths = (paths ?? []).filter((path) => {
    if (onlyChanged && !turnPaths.has(path)) return false;
    if (query && !path.toLowerCase().includes(query)) return false;
    return true;
  });
  const tree = useMemo(() => buildTree(visiblePaths), [visiblePaths]);
  // While filtering, every directory in the filtered tree stands open — the
  // filter's job is to surface matches, not to make the user hunt for them.
  const filtering = query !== "" || onlyChanged;

  const deltaOf = (path: string) => changed.find((item) => item.path === path) ?? null;
  const delta = selected ? deltaOf(selected) : null;
  const recorded = selected ? (diffs[selected] ?? null) : null;
  const recordedBody = recorded && diffHasBody(recorded) ? recorded : null;
  // A create the changeset never stored still shows its change — the file's
  // lines are exactly what the turn added. Edits without a body stay honest.
  const diffText =
    recordedBody ??
    (!recordedBody && selected && delta && delta.removed === 0 && content !== null
      ? addOnlyDiff(selected, content)
      : null);
  // Probe kind once known; until then the path-extension guess keeps md/html
  // routing stable with no flicker.
  const route = selected ? previewRoute(selected, probe?.kind ?? kindFromPath(selected)) : null;
  // 变更 is reachable for every changed file — never gated on a stored diff.
  const canDiff = (path: string) =>
    turnPaths.has(path) || diffHasBody(diffs[path] ?? "");
  const showDiffTab = selected !== null && canDiff(selected);
  // 完整文件 only means anything for text — non-text has no readable body.
  const showFullTab = !probe || probe.kind === "text";
  const effectiveTab: Tab =
    tab === "diff" && showDiffTab
      ? "diff"
      : tab === "full" && showFullTab
        ? "full"
        : route
          ? "preview"
          : "full";
  const diffItems = useMemo(() => (diffText ? foldContext(parseDiff(diffText)) : []), [diffText]);
  // One whole-file hljs pass per selection; null → plain-text lines below
  // (unknown language, oversized file, or hljs failure).
  const highlighted = useMemo(
    () => (content !== null ? highlightLines(content, languageForPath(selected ?? "")) : null),
    [content, selected],
  );

  // Fresh file → fresh folds; an open gap on one file means nothing on another.
  useEffect(() => {
    setOpenFolds(new Set());
    setDiffMode("unified");
  }, [selected]);

  const selectFile = (path: string) => {
    setSelected(path);
    setTab(pickTab(path, canDiff(path), canDiff));
    // The drawer overlays the content — pick a file, get out of the way.
    if (compact) setTreeOpen(false);
  };

  const toggleDir = (path: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const toggleFold = (id: number) => {
    setOpenFolds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const renderNodes = (nodes: TreeNode[], depth: number): ReactNode =>
    nodes.map((node) => {
      const delta = node.children === null ? deltaOf(node.path) : null;
      const isOpen = filtering || expanded.has(node.path);
      const { Icon } = fileLook(node.path, kindFromPath(node.path));
      return (
        <li key={node.path}>
          {node.children !== null ? (
            <button
              type="button"
              className="fv-tree-row fv-tree-dir"
              style={{ paddingLeft: 8 + depth * 14 }}
              data-testid={`tree-toggle-${node.path}`}
              aria-expanded={isOpen}
              onClick={() => toggleDir(node.path)}
            >
              <ChevronRight
                size={13}
                strokeWidth={1.8}
                className={`fv-caret${isOpen ? " fv-caret--open" : ""}`}
              />
              <span className="fv-tree-name">{node.name}</span>
            </button>
          ) : (
            <button
              type="button"
              className={`fv-tree-row fv-tree-file${selected === node.path ? " fv-tree-file--active" : ""}`}
              // The kind icon sits in the caret column, so file rows pad like
              // dir rows and names line up across both.
              style={{ paddingLeft: 8 + depth * 14 }}
              data-testid={`tree-file-${node.path}`}
              onClick={() => selectFile(node.path)}
            >
              <Icon size={13} strokeWidth={1.7} className="fv-tree-icon" aria-hidden />
              <span className="fv-tree-name">{node.name}</span>
              {delta && (
                <span className="fv-tree-delta">
                  <span className="delta-add">+{delta.added}</span>{" "}
                  <span className="delta-del">-{delta.removed}</span>
                </span>
              )}
            </button>
          )}
          {node.children !== null && isOpen && (
            <ul className="fv-tree-list">{renderNodes(node.children, depth + 1)}</ul>
          )}
        </li>
      );
    });

  return (
    <div
      className={`fv${compact ? " fv--compact" : ""}${compact && treeOpen ? " fv--tree-open" : ""}`}
      data-testid="file-viewer"
    >
      <header className="fv-head">
        {compact && (
          <button
            type="button"
            className="icon-btn icon-btn--sm"
            data-testid="tree-toggle"
            aria-label={T.viewer.tree}
            title={T.viewer.tree}
            aria-expanded={treeOpen}
            onClick={() => setTreeOpen((value) => !value)}
          >
            <FolderTree size={14} strokeWidth={1.8} />
          </button>
        )}
        <span className="fv-title">{T.viewer.title}</span>
        {selected && (
          <code className="fv-path" data-testid="viewer-path">
            {selected}
          </code>
        )}
        {selected && (
          <>
            <button
              type="button"
              className="icon-btn icon-btn--sm"
              data-testid="viewer-copy-path"
              aria-label={T.viewer.copyPath}
              title={copied ? T.action.copied : T.viewer.copyPath}
              onClick={() => void copyPath()}
            >
              {copied ? <Check size={14} strokeWidth={1.8} /> : <Copy size={14} strokeWidth={1.8} />}
            </button>
            <button
              type="button"
              className="icon-btn icon-btn--sm"
              data-testid="viewer-reveal"
              aria-label={T.viewer.reveal}
              title={T.viewer.reveal}
              onClick={() => {
                // A file removed since the tree rendered just fails silently —
                // the viewer stays open on the last good selection.
                void revealFile(projectPath, selected).catch(() => {});
              }}
            >
              <ExternalLink size={14} strokeWidth={1.8} />
            </button>
          </>
        )}
      </header>

      <div className="fv-body">
        <aside className="fv-tree" aria-label={T.viewer.tree}>
          <h3 className="fv-tree-head">{T.viewer.tree}</h3>
          <div className="fv-filter-bar">
            <input
              className="fv-filter"
              data-testid="tree-filter"
              value={filter}
              placeholder={T.viewer.filter}
              aria-label={T.viewer.filter}
              onChange={(event) => setFilter(event.target.value)}
            />
            <button
              type="button"
              className={`fv-only-changed${onlyChanged ? " fv-only-changed--on" : ""}`}
              data-testid="tree-only-changed"
              aria-pressed={onlyChanged}
              onClick={() => setOnlyChanged((value) => !value)}
            >
              {T.viewer.onlyChanged}
            </button>
          </div>
          {paths === null ? (
            <p className="fv-note">{T.viewer.loading}</p>
          ) : visiblePaths.length === 0 ? (
            <p className="fv-note" data-testid="tree-no-match">
              {paths.length === 0 ? T.viewer.empty : T.viewer.noMatch}
            </p>
          ) : (
            <ul className="fv-tree-list" data-testid="viewer-tree">
              {renderNodes(tree, 0)}
            </ul>
          )}
        </aside>

        <div className="fv-main">
          <div className="fv-tabs" role="tablist">
            {showFullTab && (
              <button
                type="button"
                role="tab"
                aria-selected={effectiveTab === "full"}
                className={`fv-tab${effectiveTab === "full" ? " fv-tab--active" : ""}`}
                data-testid="file-tab-full"
                onClick={() => setTab("full")}
              >
                {T.viewer.full}
              </button>
            )}
            {route && (
              <button
                type="button"
                role="tab"
                aria-selected={effectiveTab === "preview"}
                className={`fv-tab${effectiveTab === "preview" ? " fv-tab--active" : ""}`}
                data-testid="file-tab-preview"
                onClick={() => setTab("preview")}
              >
                {T.viewer.preview}
              </button>
            )}
            {showDiffTab && (
              <button
                type="button"
                role="tab"
                aria-selected={effectiveTab === "diff"}
                className={`fv-tab${effectiveTab === "diff" ? " fv-tab--active" : ""}`}
                data-testid="file-tab-diff"
                onClick={() => setTab("diff")}
              >
                {T.viewer.diff}
              </button>
            )}
          </div>

          <div className="fv-pane">
            {!selected ? (
              <p className="fv-note">{T.viewer.selectFile}</p>
            ) : probe === null && !contentError ? (
              // Probing — before content, so a non-text file never parks on
              // a read that the backend would refuse anyway.
              <p className="fv-note">{T.viewer.loading}</p>
            ) : contentError ? (
              <div className="fv-note fv-note--error" data-testid="viewer-error">
                <p>{T.viewer.error}</p>
                <button
                  type="button"
                  className="btn btn--sm"
                  data-testid="viewer-retry"
                  onClick={() => setRetryKey((key) => key + 1)}
                >
                  {T.viewer.retry}
                </button>
              </div>
            ) : probe !== null && probe.kind !== "text" && effectiveTab !== "diff" ? (
              route === "image" && probe.preview ? (
                <div className="fv-preview fv-preview--image">
                  <img
                    className="fv-preview-img"
                    data-testid="viewer-image"
                    src={probe.preview}
                    alt={probe.name}
                  />
                </div>
              ) : (
                <KindCard projectPath={projectPath} probe={probe} path={selected} />
              )
            ) : content === null ? (
              <p className="fv-note">{T.viewer.loading}</p>
            ) : effectiveTab === "diff" ? (
              <>
                <div className="fv-diff-bar" data-testid="diff-bar">
                  {delta && (
                    <span className="fv-diff-stats" data-testid="diff-stats">
                      <span className="delta-add">+{delta.added}</span>{" "}
                      <span className="delta-del">-{delta.removed}</span>
                    </span>
                  )}
                  <span className="spacer" />
                  {diffText && (
                    <div className="fv-mode" role="group" aria-label={T.viewer.diffMode}>
                      <button
                        type="button"
                        className={`fv-mode-btn${diffMode === "unified" ? " fv-mode-btn--active" : ""}`}
                        data-testid="diff-mode-unified"
                        aria-pressed={diffMode === "unified"}
                        onClick={() => setDiffMode("unified")}
                      >
                        {T.viewer.unified}
                      </button>
                      <button
                        type="button"
                        className={`fv-mode-btn${diffMode === "split" ? " fv-mode-btn--active" : ""}`}
                        data-testid="diff-mode-split"
                        aria-pressed={diffMode === "split"}
                        onClick={() => setDiffMode("split")}
                      >
                        {T.viewer.split}
                      </button>
                    </div>
                  )}
                </div>
                {!diffText ? (
                  <p className="fv-note" data-testid="viewer-diff-missing">
                    {T.viewer.diffUnrecorded}
                  </p>
                ) : diffMode === "unified" ? (
                  <pre className="fv-code" data-testid="viewer-diff">
                    {diffItems.map((item) =>
                      item.type === "fold" && !openFolds.has(item.id) ? (
                        <button
                          key={`fold-${item.id}`}
                          type="button"
                          className="fv-fold"
                          data-testid={`diff-fold-${item.id}`}
                          onClick={() => toggleFold(item.id)}
                        >
                          {T.viewer.expandContext(item.hidden.length)}
                        </button>
                      ) : item.type === "fold" ? (
                        item.hidden.map((row, index) => (
                          <UnifiedLine key={`fold-${item.id}-${index}`} row={row} />
                        ))
                      ) : (
                        <UnifiedLine key={item.key} row={item.row} />
                      ),
                    )}
                  </pre>
                ) : (
                  <pre className="fv-code" data-testid="viewer-diff-split">
                    {toSplit(diffItems).map((item, index) =>
                      item.type === "fold" && !openFolds.has(item.id) ? (
                        <button
                          key={`fold-${item.id}`}
                          type="button"
                          className="fv-fold"
                          data-testid={`diff-fold-${item.id}`}
                          onClick={() => toggleFold(item.id)}
                        >
                          {T.viewer.expandContext(item.hidden.length)}
                        </button>
                      ) : item.type === "fold" ? (
                        item.hidden.map((row, rowIndex) => (
                          <SplitPair key={`fold-${item.id}-${rowIndex}`} left={row} right={row} same />
                        ))
                      ) : item.type === "full" ? (
                        <div key={index} className="viewer-line viewer-line--wide">
                          <UnifiedLine row={item.row} bare />
                        </div>
                      ) : (
                        <SplitPair key={index} left={item.left} right={item.right} />
                      ),
                    )}
                  </pre>
                )}
              </>
            ) : effectiveTab === "preview" && route === "markdown" ? (
              <div className="fv-preview" data-testid="viewer-preview">
                <Markdown text={content} />
              </div>
            ) : effectiveTab === "preview" && route === "csv" ? (
              <CsvTable content={content} />
            ) : effectiveTab === "preview" && route === "html" ? (
              <div className="fv-preview fv-preview--html" data-testid="viewer-preview">
                <p className="fv-preview-note">{T.viewer.htmlSandbox}</p>
                <iframe
                  className="fv-preview-frame"
                  data-testid="viewer-html-preview"
                  title={selected}
                  sandbox="allow-scripts"
                  srcDoc={content}
                />
              </div>
            ) : (
              <pre className="fv-code" data-testid="viewer-code">
                {highlighted
                  ? highlighted.map((line, index) => (
                      <div key={index} className="viewer-line">
                        <span className="viewer-ln">{index + 1}</span>
                        <span className="viewer-text" dangerouslySetInnerHTML={{ __html: line }} />
                      </div>
                    ))
                  : content.split("\n").map((line, index) => (
                      <div key={index} className="viewer-line">
                        <span className="viewer-ln">{index + 1}</span>
                        <span className="viewer-text">{line}</span>
                      </div>
                    ))}
              </pre>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

/** A split-view row: one side deleted, the other added (either may be empty). */
function SplitPair({
  left,
  right,
  same = false,
}: {
  left: DiffRow | null;
  right: DiffRow | null;
  same?: boolean;
}) {
  return (
    <div className="viewer-line viewer-line--split">
      <span className={`viewer-half${left && !same ? " is-del" : ""}`}>
        <span className="viewer-ln" aria-hidden>
          {left && !same ? "-" : " "}
        </span>
        <span className="viewer-text">{left ? left.text : ""}</span>
      </span>
      <span className={`viewer-half${right && !same ? " is-add" : ""}`}>
        <span className="viewer-ln" aria-hidden>
          {right && !same ? "+" : " "}
        </span>
        <span className="viewer-text">{right ? right.text : ""}</span>
      </span>
    </div>
  );
}

type FoldItem =
  | { type: "line"; key: number; row: DiffRow }
  | { type: "fold"; id: number; hidden: DiffRow[] };

type SplitItem =
  | { type: "full"; row: DiffRow }
  | { type: "pair"; left: DiffRow | null; right: DiffRow | null }
  | { type: "fold"; id: number; hidden: DiffRow[] };

const CTX_FOLD_MIN = 8;
const CTX_FOLD_KEEP = 2;

/**
 * Long unchanged runs fold to two lines each side behind 展开 N 行上下文 —
 * a review reads as the change, with context on demand (GitHub-style).
 */
function foldContext(rows: DiffRow[]): FoldItem[] {
  const out: FoldItem[] = [];
  let run: DiffRow[] = [];
  let nextKey = 0;
  let foldId = 0;
  const flushRun = () => {
    if (run.length === 0) return;
    if (run.length <= CTX_FOLD_MIN) {
      for (const row of run) out.push({ type: "line", key: nextKey++, row });
    } else {
      const head = run.slice(0, CTX_FOLD_KEEP);
      const tail = run.slice(run.length - CTX_FOLD_KEEP);
      const mid = run.slice(CTX_FOLD_KEEP, run.length - CTX_FOLD_KEEP);
      for (const row of head) out.push({ type: "line", key: nextKey++, row });
      out.push({ type: "fold", id: foldId++, hidden: mid });
      for (const row of tail) out.push({ type: "line", key: nextKey++, row });
    }
    run = [];
  };
  for (const row of rows) {
    if (row.kind === "ctx") {
      run.push(row);
    } else {
      flushRun();
      out.push({ type: "line", key: nextKey++, row });
    }
  }
  flushRun();
  return out;
}

/**
 * Split (对照) rows: consecutive del/add runs pair up index-wise — deleted
 * text left, added text right — while hunk headers and context span both.
 */
function toSplit(items: FoldItem[]): SplitItem[] {
  const out: SplitItem[] = [];
  let dels: DiffRow[] = [];
  let adds: DiffRow[] = [];
  const flushPairs = () => {
    const n = Math.max(dels.length, adds.length);
    for (let i = 0; i < n; i += 1) {
      out.push({ type: "pair", left: dels[i] ?? null, right: adds[i] ?? null });
    }
    dels = [];
    adds = [];
  };
  for (const item of items) {
    if (item.type === "fold") {
      flushPairs();
      out.push(item);
      continue;
    }
    const row = item.row;
    if (row.kind === "del") dels.push(row);
    else if (row.kind === "add") adds.push(row);
    else {
      flushPairs();
      out.push({ type: "full", row });
    }
  }
  flushPairs();
  return out;
}

/**
 * Preview for files the viewer cannot render inline — PDF, Office, oversized
 * images, binaries. States what the file is, how big, and hands off to the
 * system default application rather than pretending to preview it.
 */
function KindCard({ projectPath, probe, path }: { projectPath: string; probe: ContextProbe; path: string }) {
  const look = fileLook(path, probe.kind);
  const Icon = look.Icon;
  const [failed, setFailed] = useState(false);
  return (
    <div className="fv-kind-card" data-testid="viewer-kind-card">
      <span className="fv-kind-figure" style={{ color: look.color }} aria-hidden>
        <Icon size={30} strokeWidth={1.6} />
      </span>
      <span className="fv-kind-badge" style={{ background: look.color }}>
        {look.badge}
      </span>
      <span className="fv-kind-name" title={path}>
        {probe.name}
      </span>
      <span className="fv-kind-size">{formatSize(probe.size)}</span>
      <button
        type="button"
        className="btn btn--sm"
        data-testid="viewer-open-external"
        onClick={() => {
          setFailed(false);
          void openProjectFile(projectPath, path).catch(() => setFailed(true));
        }}
      >
        {T.viewer.openExternal}
      </button>
      {failed && (
        <p className="fv-note fv-note--error" role="alert">
          {T.viewer.error}
        </p>
      )}
    </div>
  );
}

/** RFC-4180-ish CSV cell split: commas, double-quoted fields, "" escapes. */
function parseCsvLine(line: string): string[] {
  const cells: string[] = [];
  let cell = "";
  let inQuotes = false;
  for (let i = 0; i < line.length; i += 1) {
    const char = line[i];
    if (inQuotes) {
      if (char === '"') {
        if (line[i + 1] === '"') {
          cell += '"';
          i += 1;
        } else {
          inQuotes = false;
        }
      } else {
        cell += char;
      }
    } else if (char === '"') {
      inQuotes = true;
    } else if (char === ",") {
      cells.push(cell);
      cell = "";
    } else {
      cell += char;
    }
  }
  cells.push(cell);
  return cells;
}

/** First line as header, the rest as body rows. */
function CsvTable({ content }: { content: string }) {
  const lines = content.replace(/\r\n/g, "\n").split("\n");
  while (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  if (lines.length === 0) return <p className="fv-note">{T.viewer.empty}</p>;
  const [head, ...body] = lines;
  return (
    <div className="fv-csv-wrap" data-testid="viewer-csv">
      <table className="fv-csv">
        <thead>
          <tr>
            {parseCsvLine(head).map((cell, index) => (
              <th key={index} scope="col">
                {cell}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {body.map((line, rowIndex) => (
            <tr key={rowIndex}>
              {parseCsvLine(line).map((cell, cellIndex) => (
                <td key={cellIndex}>{cell}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
