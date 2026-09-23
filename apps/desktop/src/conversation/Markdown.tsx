import { Fragment, useId, useState, type ReactNode } from "react";
import { T } from "../i18n";

/**
 * Compact conversation markdown.
 *
 * COMPONENTS.md asks FinalResponse to render markdown with mono code blocks.
 * The conversation column is IDE-dense, so this stays a small local parser —
 * headings, lists, fenced/inline code, bold — not a full CommonMark engine.
 * Long code folds behind 显示完整代码 and many headings get a small TOC, so a
 * long answer reads as a summary first.
 */
export function Markdown({ text }: { text: string }) {
  const blocks = parseBlocks(text);
  const headings = blocks.flatMap((block, index) =>
    block.kind === "heading" ? [{ index, text: block.text, id: headingId(index) }] : [],
  );
  const showToc = headings.length >= 4;

  return (
    <div className="md">
      {showToc && (
        <nav className="md-toc" aria-label={T.reply.toc} data-testid="md-toc">
          <span className="md-toc-title">{T.reply.toc}</span>
          <ol className="md-toc-list">
            {headings.map((heading) => (
              <li key={heading.id}>
                <a href={`#${heading.id}`}>{heading.text}</a>
              </li>
            ))}
          </ol>
        </nav>
      )}
      {blocks.map((block, index) => renderBlock(block, index, headingId(index)))}
    </div>
  );
}

function headingId(index: number): string {
  return `md-h-${index}`;
}

type Block =
  | { kind: "code"; lang: string; body: string }
  | { kind: "heading"; level: number; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "quote"; text: string }
  | { kind: "para"; text: string };

function parseBlocks(source: string): Block[] {
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

    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      blocks.push({ kind: "heading", level: heading[1].length, text: heading[2] });
      i += 1;
      continue;
    }

    if (/^\s*([-*+]|\d+\.)\s+/.test(line)) {
      const ordered = /^\s*\d+\.\s+/.test(line);
      const items: string[] = [];
      while (i < lines.length && /^\s*([-*+]|\d+\.)\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*([-*+]|\d+\.)\s+/, ""));
        i += 1;
      }
      blocks.push({ kind: "list", ordered, items });
      continue;
    }

    if (line.startsWith(">")) {
      const quote: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) {
        quote.push(lines[i].replace(/^>\s?/, ""));
        i += 1;
      }
      blocks.push({ kind: "quote", text: quote.join("\n") });
      continue;
    }

    if (line.trim() === "") {
      i += 1;
      continue;
    }

    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].startsWith("```") &&
      !/^#{1,6}\s+/.test(lines[i]) &&
      !/^\s*([-*+]|\d+\.)\s+/.test(lines[i]) &&
      !lines[i].startsWith(">")
    ) {
      para.push(lines[i]);
      i += 1;
    }
    blocks.push({ kind: "para", text: para.join("\n") });
  }

  return blocks;
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
  const shown = !long || expanded ? body : lines.slice(0, CODE_SHOWN_LINES).join("\n");

  return (
    <div className="md-code-wrap">
      <pre className="md-code" id={preId} data-lang={lang || undefined}>
        <code>{shown}</code>
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

function renderBlock(block: Block, index: number, id: string): ReactNode {
  switch (block.kind) {
    case "code":
      return <CodeBlock key={index} body={block.body} lang={block.lang} />;
    case "heading": {
      const Tag = (`h${Math.min(4, Math.max(3, block.level + 1))}` as const) as "h3" | "h4";
      return (
        <Tag key={index} className="md-heading" id={id}>
          {inline(block.text)}
        </Tag>
      );
    }
    case "list": {
      const ListTag = block.ordered ? "ol" : "ul";
      return (
        <ListTag key={index} className="md-list">
          {block.items.map((item, itemIndex) => (
            <li key={itemIndex}>{inline(item)}</li>
          ))}
        </ListTag>
      );
    }
    case "quote":
      return (
        <blockquote key={index} className="md-quote">
          {inline(block.text)}
        </blockquote>
      );
    default:
      return (
        <p key={index} className="md-para">
          {inline(block.text)}
        </p>
      );
  }
}

/** Bold + inline code + fenced markers stripped for emphasis. */
function inline(text: string): ReactNode[] {
  const parts: ReactNode[] = [];
  const pattern = /(`[^`]+`)|(\*\*[^*]+\*\*)|(\*[^*]+\*)/g;
  let last = 0;
  let match: RegExpExecArray | null;
  let key = 0;

  while ((match = pattern.exec(text)) !== null) {
    if (match.index > last) parts.push(text.slice(last, match.index));
    const token = match[0];
    if (token.startsWith("`")) {
      parts.push(
        <code key={key++} className="md-inline-code">
          {token.slice(1, -1)}
        </code>,
      );
    } else if (token.startsWith("**")) {
      parts.push(<strong key={key++}>{token.slice(2, -2)}</strong>);
    } else {
      parts.push(<em key={key++}>{token.slice(1, -1)}</em>);
    }
    last = match.index + token.length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts.map((node, index) => <Fragment key={index}>{node}</Fragment>);
}
