/**
 * The app's single highlight.js entry — components must not import
 * highlight.js directly.
 *
 * Safety contract for returned HTML (consumers use dangerouslySetInnerHTML):
 * language ids come only from the white-listed maps below or from hljs's own
 * registered grammars (an id selects a grammar, never file content), and
 * highlight.js escapes every text character it emits — output contains only
 * balanced `<span class="hljs-…">` tags. `highlightLines` re-balances each
 * line so every emitted string is standalone-safe.
 */
import hljs from "highlight.js";

/** Extension → hljs language id (full grammar set registers every id below). */
const EXT_LANG: Record<string, string> = {
  ts: "typescript",
  tsx: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  json: "json",
  jsonl: "json",
  md: "markdown",
  markdown: "markdown",
  css: "css",
  scss: "scss",
  html: "xml", // the xml grammar covers HTML tags/attrs
  htm: "xml",
  xml: "xml",
  svg: "xml",
  rs: "rust",
  py: "python",
  go: "go",
  java: "java",
  kt: "kotlin",
  c: "c",
  h: "c",
  cpp: "cpp",
  hpp: "cpp",
  cc: "cpp",
  cxx: "cpp",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  sql: "sql",
  yml: "yaml",
  yaml: "yaml",
  toml: "toml",
  ini: "ini",
  cfg: "ini",
  conf: "ini",
  diff: "diff",
  patch: "diff",
  rb: "ruby",
  php: "php",
  lua: "lua",
  gql: "graphql",
  graphql: "graphql",
  makefile: "makefile",
  mk: "makefile",
};

/** Bare fence names (```ts) and common aliases → hljs language id. */
const NAME_LANG: Record<string, string> = {
  ...EXT_LANG,
  shell: "bash",
  typescriptreact: "typescript",
  javascriptreact: "javascript",
  "c++": "cpp",
  "c#": "csharp",
};

/** Fence header language (```ts) → hljs id, or null for plain text. */
export function resolveLanguage(name: string): string | null {
  const key = name.trim().toLowerCase();
  if (!key) return null;
  const mapped = NAME_LANG[key];
  if (mapped) return mapped;
  return hljs.getLanguage(key) ? key : null;
}

/** File path → hljs language id (extension based), or null for plain text. */
export function languageForPath(path: string): string | null {
  const base = path.split("/").pop() ?? path;
  const dot = base.lastIndexOf(".");
  const key = (dot > 0 ? base.slice(dot + 1) : base).toLowerCase();
  return EXT_LANG[key] ?? (hljs.getLanguage(key) ? key : null);
}

/**
 * Files above this size skip highlighting and render as plain text — one
 * hljs pass stays in the tens-of-ms range for typical sources, but an
 * unbounded file must not jank the viewer.
 */
const HIGHLIGHT_MAX_CHARS = 400_000;

const TAG_RE = /<span class="([^"]+)">|<\/span>/g;

/**
 * Highlight `code` once, then re-balance the HTML across its lines: each
 * returned string re-opens the spans still live at its start and closes
 * everything it opened, so a line is safe to embed on its own. Per-line
 * highlighting would sever multi-line tokens (block comments, docstrings);
 * a raw split of one highlighted blob would leave dangling tags.
 *
 * Returns null when there is nothing to do — unknown language, oversized
 * input, or an hljs failure — and the caller falls back to plain text.
 */
export function highlightLines(code: string, language: string | null): string[] | null {
  if (!language || code.length > HIGHLIGHT_MAX_CHARS) return null;
  if (!hljs.getLanguage(language)) return null;

  let value: string;
  try {
    value = hljs.highlight(code, { language, ignoreIllegals: true }).value;
  } catch {
    return null;
  }

  const out: string[] = [];
  /** Span classes open at the start of the line being built. */
  let carried: string[] = [];
  for (const line of value.split("\n")) {
    // Events in document order: an open pushes, a close pops (hljs spans nest,
    // so stack order is exact). counts come from the same match — the line's
    // own events decide both the closes appended here and the next line's
    // carried prefix.
    let opens = 0;
    let closes = 0;
    const events: Array<string | null> = [];
    for (const match of line.matchAll(TAG_RE)) {
      if (match[1] !== undefined) {
        opens += 1;
        events.push(match[1]);
      } else {
        closes += 1;
        events.push(null);
      }
    }
    const prefix = carried.map((cls) => `<span class="${cls}">`).join("");
    // Balance the emitted line: it opens (carried + opens) spans and carries
    // `closes` of its own, so append the rest.
    const append = carried.length + opens - closes;
    out.push(prefix + line + "</span>".repeat(append));
    for (const event of events) {
      if (event === null) carried.pop();
      else carried.push(event);
    }
  }
  return out;
}
