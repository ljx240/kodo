/**
 * Block parser for the conversation markdown renderer.
 *
 * COMPONENTS.md asks FinalResponse to render markdown; this stays a small
 * local parser — fences, headings, GFM pipe tables, rules, block formulas,
 * lists (nested + task), quotes (nested via `>>`), footnotes, callouts,
 * paragraphs — not a full CommonMark engine. Deliberate omission: setext
 * headings (`text` + `---` reads as paragraph then rule). Inline markup
 * (code, bold, strikethrough, links, `$math$`) is applied at render time by
 * `inline()` in index.tsx.
 */

export type ColAlign = "left" | "center" | "right" | null;

export type ListItem = {
  text: string;
  ordered: boolean;
  /** Original number for ordered items (`5.` → 5); null for bullets. */
  start: number | null;
  depth: number;
  task: null | "todo" | "done";
};

/** One line inside a quote: nesting depth (1 = `>`), null text = blank `>`. */
export type QuoteLine = { depth: number; text: string | null };

export type CalloutKind = "note" | "tip" | "warning" | "important";

export type Block =
  | { kind: "code"; lang: string; body: string }
  | { kind: "heading"; level: number; text: string }
  | { kind: "list"; items: ListItem[] }
  | { kind: "quote"; lines: QuoteLine[]; callout: CalloutKind | null }
  | { kind: "footnote"; id: string; text: string }
  | { kind: "formula"; tex: string }
  | { kind: "table"; header: string[]; align: ColAlign[]; rows: string[][] }
  | { kind: "hr" }
  | { kind: "para"; text: string };

const HEADING = /^(#{1,6})\s+(.*)$/;
const HR = /^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/;
const LIST = /^(\s*)([-*+]|\d+\.)\s+(.*)$/;
const TASK = /^\[([ xX])\]\s+/;
const SEPARATOR_CELL = /^:?-+:?$/;
const FOOTNOTE_DEF = /^\[\^([^\]]+)\]:\s?([\s\S]*)$/;
const CALLOUT = /^\[!(NOTE|TIP|WARNING|IMPORTANT)\]/;
/** `>`, `> >`, `>>> …` → depth = `>` count (capped). */
const QUOTE_LINE = /^(\s*)((?:>\s*)+)(.*)$/;

export function parseBlocks(source: string): Block[] {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (line.startsWith("```")) {
      const lang = line.slice(3).trim();
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !lines[i].startsWith("```")) {
        body.push(lines[i]);
        i += 1;
      }
      i += 1;
      blocks.push({ kind: "code", lang, body: body.join("\n") });
      continue;
    }

    const heading = HEADING.exec(line);
    if (heading) {
      blocks.push({ kind: "heading", level: heading[1].length, text: heading[2] });
      i += 1;
      continue;
    }

    // A table needs a `|` header row directly above a separator row with the
    // same cell count; anything looser stays prose.
    const table = tableAt(lines, i);
    if (table) {
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && isTableRow(lines[i], table.align.length)) {
        rows.push(cellsOf(lines[i], table.align.length));
        i += 1;
      }
      blocks.push({ kind: "table", header: table.header, align: table.align, rows });
      continue;
    }

    // Before the list branch: `* * *` would otherwise read as a bullet.
    if (HR.test(line)) {
      blocks.push({ kind: "hr" });
      i += 1;
      continue;
    }

    const trimmed = line.trimStart();
    if (trimmed.startsWith("$$")) {
      const first = trimmed.slice(2);
      const tex: string[] = [];
      const closeOnFirst = first.indexOf("$$");
      if (closeOnFirst >= 0) {
        tex.push(first.slice(0, closeOnFirst));
        i += 1;
      } else {
        if (first.trim()) tex.push(first);
        i += 1;
        while (i < lines.length) {
          const close = lines[i].indexOf("$$");
          if (close >= 0) {
            const before = lines[i].slice(0, close).trimEnd();
            if (before) tex.push(before);
            i += 1;
            break;
          }
          tex.push(lines[i]);
          i += 1;
        }
      }
      // Unterminated blocks still parse; the renderer falls back to literals.
      blocks.push({ kind: "formula", tex: tex.join("\n") });
      continue;
    }

    const list = LIST.exec(line.replace(/\t/g, "  "));
    if (list) {
      const items: ListItem[] = [];
      while (i < lines.length) {
        const item = LIST.exec(lines[i].replace(/\t/g, "  "));
        if (!item) break;
        let text = item[3];
        let task: ListItem["task"] = null;
        const check = TASK.exec(text);
        if (check) {
          task = check[1] === " " ? "todo" : "done";
          text = text.slice(check[0].length);
        }
        const ordered = /^\d+\.$/.test(item[2]);
        items.push({
          text,
          ordered,
          start: ordered ? Number.parseInt(item[2], 10) : null,
          depth: Math.min(4, Math.floor(item[1].length / 2)),
          task,
        });
        i += 1;
      }
      blocks.push({ kind: "list", items });
      continue;
    }

    if (line.startsWith(">")) {
      const raw: QuoteLine[] = [];
      while (i < lines.length && lines[i].startsWith(">")) {
        const marked = QUOTE_LINE.exec(lines[i]);
        if (marked) {
          const depth = Math.min(6, marked[2].replace(/\s/g, "").length);
          const text = marked[3].trim();
          raw.push({ depth, text: text === "" ? null : text });
        }
        i += 1;
      }
      // `> [!NOTE]` on the first content line turns the quote into a callout.
      let callout: CalloutKind | null = null;
      const firstContent = raw.find((entry) => entry.text !== null);
      if (firstContent && firstContent.depth === 1) {
        const kind = CALLOUT.exec(firstContent.text ?? "");
        if (kind) {
          callout = kind[1].toLowerCase() as CalloutKind;
          // A bare marker line is only the marker; anything after it stays as
          // the callout's first body line (`> [!NOTE] 同行标题`).
          const rest = (firstContent.text ?? "").slice(kind[0].length).trim();
          if (rest) {
            firstContent.text = rest;
          } else {
            raw.splice(raw.indexOf(firstContent), 1);
          }
        }
      }
      blocks.push({ kind: "quote", lines: raw, callout });
      continue;
    }

    if (line.trim() === "") {
      i += 1;
      continue;
    }

    // `[^1]: definition` opens a footnote block; the rest of the paragraph
    // (indented continuation lines included) is the definition text.
    const footnote = FOOTNOTE_DEF.exec(line);
    if (footnote) {
      const body: string[] = [footnote[2]];
      i += 1;
      while (i < lines.length && !stopsParagraph(lines, i)) {
        body.push(lines[i]);
        i += 1;
      }
      const text = body.join("\n").trim();
      blocks.push({ kind: "footnote", id: footnote[1], text });
      continue;
    }

    const para: string[] = [];
    while (i < lines.length && !stopsParagraph(lines, i)) {
      para.push(lines[i]);
      i += 1;
    }
    blocks.push({ kind: "para", text: para.join("\n") });
  }

  return blocks;
}

/** A paragraph ends before any block start (or another paragraph's blank). */
function stopsParagraph(lines: string[], j: number): boolean {
  const line = lines[j];
  if (line.trim() === "") return true;
  if (line.startsWith("```")) return true;
  if (HEADING.test(line)) return true;
  if (HR.test(line)) return true;
  if (LIST.test(line.replace(/\t/g, "  "))) return true;
  if (line.startsWith(">")) return true;
  if (line.trimStart().startsWith("$$")) return true;
  return tableAt(lines, j) !== null;
}

/** Header row + separator row at `index`, or null when it is not a table. */
function tableAt(
  lines: string[],
  index: number,
): { header: string[]; align: ColAlign[] } | null {
  if (index + 1 >= lines.length) return null;
  if (!lines[index].includes("|")) return null;
  const align = separatorAlign(lines[index + 1]);
  if (!align) return null;
  const header = splitRow(lines[index]);
  if (header.length !== align.length) return null;
  return { header, align };
}

function separatorAlign(row: string): ColAlign[] | null {
  const cells = splitRow(row);
  if (cells.length === 0) return null;
  const align: ColAlign[] = [];
  for (const cell of cells) {
    if (!SEPARATOR_CELL.test(cell)) return null;
    const left = cell.startsWith(":");
    const right = cell.endsWith(":");
    align.push(left && right ? "center" : right ? "right" : left ? "left" : null);
  }
  return align;
}

/** Body rows must keep the header's cell count — looser lines end the table. */
function isTableRow(line: string, cells: number): boolean {
  if (line.trim() === "" || !line.includes("|")) return false;
  if (line.startsWith("```") || HR.test(line)) return false;
  return splitRow(line).length === cells;
}

/** Trim leading/trailing pipes, then split; missing cells pad, extras drop. */
function cellsOf(line: string, cells: number): string[] {
  const row = splitRow(line);
  return Array.from({ length: cells }, (_, index) => row[index] ?? "");
}

function splitRow(row: string): string[] {
  let raw = row.trim();
  if (raw.startsWith("|")) raw = raw.slice(1);
  if (raw.endsWith("|")) raw = raw.slice(0, -1);
  return raw.split("|").map((cell) => cell.trim());
}
