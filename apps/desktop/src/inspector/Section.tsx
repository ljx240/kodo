import { ChevronRight, Container, FileText, Pencil, Search, Terminal } from "lucide-react";
import { useState, type ReactNode } from "react";
import type { ChangedFile } from "../data/types";

export function Section({
  icon,
  title,
  count,
  meta,
  children,
}: {
  icon: ReactNode;
  title: string;
  count?: number;
  meta?: ReactNode;
  children?: ReactNode;
}) {
  // The chevron the reference draws on every card is a disclosure, and it was
  // drawn but dead. A card with nothing under it stays inert rather than
  // offering to open onto nothing.
  const [open, setOpen] = useState(true);
  const collapsible = Boolean(children);
  const toggle = () => collapsible && setOpen((value) => !value);

  return (
    <section className="ins-section">
      <header
        className={`ins-section-head${collapsible ? " ins-section-head--toggle" : ""}`}
        role={collapsible ? "button" : undefined}
        tabIndex={collapsible ? 0 : undefined}
        aria-expanded={collapsible ? open : undefined}
        onClick={toggle}
        onKeyDown={(event) => {
          if (!collapsible) return;
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            toggle();
          }
        }}
      >
        <span className="ins-section-icon">{icon}</span>
        <h3>{title}</h3>
        {count !== undefined && <span className="ins-badge">{count}</span>}
        <span className="spacer" />
        {meta}
        {/* The glyph is left exactly as the reference draws it, in both states:
            rotating it would be a change nothing asked for. */}
        <ChevronRight size={14} strokeWidth={1.9} className="ins-chevron" />
      </header>
      {open && children}
    </section>
  );
}

export function MetaRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="meta-row">
      <span className="meta-label">{label}</span>
      <span className="meta-value">{children}</span>
    </div>
  );
}

export function FileRow({ file }: { file: ChangedFile }) {
  return (
    <div className="file-row">
      <FileText size={14} strokeWidth={1.7} className="file-icon" />
      <span className="file-path">{file.path}</span>
      <span className="delta-add">+{file.added}</span>
      <span className="delta-del">-{file.removed}</span>
    </div>
  );
}

const TOOL_ICONS: Record<string, typeof Search> = {
  "Search codebase": Search,
  "Read file": FileText,
  "Run command": Terminal,
  Docker: Container,
  "Edit file": Pencil,
};

export function ToolRow({ name, count }: { name: string; count: number }) {
  const Icon = TOOL_ICONS[name] ?? Terminal;
  return (
    <div className="tool-row">
      <Icon size={14} strokeWidth={1.7} className="tool-icon" />
      <span className="tool-name">{name}</span>
      <span className="tool-count">{count}×</span>
    </div>
  );
}
