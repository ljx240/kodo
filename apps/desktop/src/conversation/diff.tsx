/**
 * Shared unified-diff rendering — the single parser and row component for
 * every surface that shows a change (file viewer, trace, Inspector).
 *
 * `parseDiff` is the only parse source: it drops the `---`/`+++` file headers
 * (the producer in checkpoint.rs always emits exactly those) and keeps hunk,
 * add, del and context rows. Folding and split view stay file-viewer-only —
 * review wants collapsed context; a trace/Inspector glance wants the body.
 */
import { useMemo } from "react";

export type DiffRow = {
  kind: "hunk" | "add" | "del" | "ctx";
  /** Body text without the +/- marker; hunk headers keep their @@ line. */
  text: string;
};

/** Unified-diff rows minus the file headers and the trailing-newline artifact. */
export function parseDiff(diff: string): DiffRow[] {
  const out: DiffRow[] = [];
  for (const line of diff.split("\n")) {
    if (line === "") continue;
    if (line.startsWith("+++ ") || line.startsWith("--- ")) continue;
    if (line.startsWith("@@")) out.push({ kind: "hunk", text: line });
    else if (line.startsWith("+")) out.push({ kind: "add", text: line.slice(1) });
    else if (line.startsWith("-")) out.push({ kind: "del", text: line.slice(1) });
    else out.push({ kind: "ctx", text: line.startsWith(" ") ? line.slice(1) : line });
  }
  return out;
}

/** One unified-diff line. `bare` drops the outer flex when nested in split. */
export function UnifiedLine({ row, bare = false }: { row: DiffRow; bare?: boolean }) {
  return (
    <div
      className={
        (bare ? "viewer-line-inner" : "viewer-line") +
        (row.kind === "add" ? " is-add" : row.kind === "del" ? " is-del" : row.kind === "hunk" ? " is-hunk" : "")
      }
    >
      <span className="viewer-ln" aria-hidden>
        {row.kind === "add" ? "+" : row.kind === "del" ? "-" : " "}
      </span>
      <span className="viewer-text">{row.text}</span>
    </div>
  );
}

/** Raw colored diff rows (no folding) for trace and Inspector panels. */
export function DiffRows({
  diff,
  className,
  testid,
}: {
  diff: string;
  className?: string;
  testid?: string;
}) {
  const rows = useMemo(() => parseDiff(diff), [diff]);
  return (
    <pre className={className} data-testid={testid}>
      {rows.map((row, index) => (
        <UnifiedLine key={index} row={row} />
      ))}
    </pre>
  );
}
