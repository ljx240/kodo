import { ChevronRight, Container, FileText, Pencil, Search, Sparkles, Terminal } from "lucide-react";
import { useId, useState, type ReactNode } from "react";
import type { ChangedFile } from "../data/types";
import { T, toolAlias } from "../i18n";

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
  // offering to open onto nothing. Collapsible headers are real buttons —
  // keyboard, focus and aria come from the platform, not a role="button" div.
  const [open, setOpen] = useState(true);
  const collapsible = Boolean(children);
  const bodyId = useId();
  const toggle = () => collapsible && setOpen((value) => !value);

  const headInner = (
    <>
      <span className="ins-section-icon">{icon}</span>
      <span className="ins-section-title">{title}</span>
      {count !== undefined && <span className="ins-badge">{count}</span>}
      <span className="spacer" />
      {meta}
      {/* The glyph is left exactly as the reference draws it, in both states:
          rotating it would be a change nothing asked for. */}
      <ChevronRight size={14} strokeWidth={1.9} className="ins-chevron" />
    </>
  );

  return (
    <section className="ins-section">
      {collapsible ? (
        <button
          type="button"
          className="ins-section-head ins-section-head--toggle"
          aria-expanded={open}
          aria-controls={bodyId}
          onClick={toggle}
        >
          {headInner}
        </button>
      ) : (
        <div className="ins-section-head">{headInner}</div>
      )}
      {open && (
        <div className="ins-section-body" id={bodyId}>
          {children}
        </div>
      )}
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
  [T.step.thinking]: Sparkles,
  [T.step.search]: Search,
  [T.step.read]: FileText,
  [T.step.run]: Terminal,
  [T.step.model]: Sparkles,
  [T.step.edit]: Pencil,
  [T.step.finalize]: FileText,
};

export function ToolRow({ name, count }: { name: string; count: number }) {
  const label = toolAlias(name);
  const Icon = TOOL_ICONS[label] ?? Terminal;
  return (
    <div className="tool-row">
      <Icon size={14} strokeWidth={1.7} className="tool-icon" />
      <span className="tool-name">{label}</span>
      <span className="tool-count">{count}×</span>
    </div>
  );
}

export { Container };
