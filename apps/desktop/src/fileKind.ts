/**
 * Shared file classification — one source of truth for "what kind of file
 * is this" and "what does it look like", used by the FileViewer tree and
 * preview panes, the Composer context chips, and (later) anywhere else a
 * file is named.
 *
 * `kindFromPath` mirrors the Rust `context_kind` extension tables in
 * `src-tauri/src/main.rs` so the UI can route before the probe round-trip
 * lands; the probe (`probe_context_file`) is authoritative afterwards and
 * corrects unknown extensions via content sniffing.
 */
import {
  File,
  FileCode,
  FileImage,
  FileSpreadsheet,
  FileText,
  Presentation,
  type LucideIcon,
} from "lucide-react";

/** Probe kinds, mirroring Rust `context_kind` outcomes. */
export type ProbeKind = "image" | "pdf" | "office" | "text" | "other";

/** Preview routing — which pane renders this file. */
export type PreviewRoute = "markdown" | "html" | "csv" | "image" | "pdf" | "office" | "other";

const IMAGE_EXTS = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "bmp",
  "svg",
  "ico",
  "avif",
  "heic",
]);

const OFFICE_EXTS = new Set([
  "doc",
  "docx",
  "rtf",
  "odt",
  "xls",
  "xlsx",
  "ods",
  "ppt",
  "pptx",
  "odp",
]);

const TEXT_EXTS = new Set([
  "txt",
  "md",
  "markdown",
  "json",
  "jsonl",
  "yml",
  "yaml",
  "toml",
  "xml",
  "html",
  "css",
  "js",
  "jsx",
  "ts",
  "tsx",
  "rs",
  "py",
  "go",
  "java",
  "kt",
  "c",
  "h",
  "cpp",
  "hpp",
  "sh",
  "zsh",
  "bash",
  "sql",
  "log",
  "csv",
  "tsv",
  "ini",
  "cfg",
  "conf",
]);

function extOf(path: string): string {
  const base = path.split("/").pop() ?? path;
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : base.toLowerCase();
}

/**
 * Path → kind guess, matching Rust `context_kind` extension tables.
 * Unknown extensions report "text" (the Rust side sniffs content and the
 * probe corrects the guess before any pane decision sticks).
 */
export function kindFromPath(path: string): ProbeKind {
  const ext = extOf(path);
  if (IMAGE_EXTS.has(ext)) return "image";
  if (ext === "pdf") return "pdf";
  if (OFFICE_EXTS.has(ext)) return "office";
  if (TEXT_EXTS.has(ext)) return "text";
  return "text";
}

/** Badge/color/icon for a file chip or tree row. Values came from the
 *  Composer context chip and are kept verbatim for visual continuity. */
export interface FileLook {
  badge: string;
  color: string;
  Icon: LucideIcon;
}

/** Which preview pane handles this file; null → the plain text viewer. */
export function previewRoute(path: string, kind: ProbeKind): PreviewRoute | null {
  if (kind === "image") return "image";
  if (kind === "pdf") return "pdf";
  if (kind === "office") return "office";
  if (kind === "other") return "other";
  const ext = extOf(path);
  if (ext === "md" || ext === "markdown") return "markdown";
  if (ext === "html" || ext === "htm") return "html";
  if (ext === "csv") return "csv";
  return null;
}

/**
 * Badge, accent color and icon for a file. Pass the probe kind when
 * available; without it the path extension still yields a sane guess.
 */
export function fileLook(path: string, kind?: ProbeKind): FileLook {
  const ext = extOf(path);
  switch (kind) {
    case "image":
      return { badge: "IMG", color: "#2563EB", Icon: FileImage };
    case "pdf":
      return { badge: "PDF", color: "#1F2328", Icon: FileText };
    case "office":
      if (ext.startsWith("xls") || ext === "ods" || ext === "csv") {
        return { badge: "XLS", color: "#217346", Icon: FileSpreadsheet };
      }
      if (ext.startsWith("ppt") || ext === "odp") {
        return { badge: "PPT", color: "#D24726", Icon: Presentation };
      }
      return { badge: "DOC", color: "#2B579A", Icon: FileText };
    case "text":
      if (
        ["ts", "tsx", "js", "jsx", "rs", "py", "go", "java", "kt", "c", "h", "cpp", "sh", "sql"].includes(
          ext,
        )
      ) {
        return { badge: "CODE", color: "#667085", Icon: FileCode };
      }
      return { badge: "TXT", color: "#667085", Icon: FileText };
    default: {
      const badge = ext.slice(0, 3).toUpperCase();
      return { badge: badge.length >= 2 ? badge : "BIN", color: "#667085", Icon: File };
    }
  }
}

/** Human byte size for preview headers ("1.2 KB", "3.4 MB"). */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb >= 100 ? Math.round(kb) : kb.toFixed(1)} KB`;
  const mb = kb / 1024;
  return `${mb >= 100 ? Math.round(mb) : mb.toFixed(1)} MB`;
}

/** Render-time gate for clickable inline code: no spaces, no URL scheme, no
 *  `..`, and either a `/` or a real extension (letters first, so `v1.2` and
 *  other version numbers stay plain text). Clicks still probe before opening. */
export function looksLikePath(value: string): boolean {
  if (!value || /\s/.test(value)) return false;
  if (value.startsWith("..")) return false;
  if (/^[a-z][a-z0-9+.-]*:/i.test(value)) return false;
  return value.includes("/") || /\.[a-z][a-z0-9]*$/i.test(value);
}
