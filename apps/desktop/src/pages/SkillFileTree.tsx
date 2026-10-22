/**
 * File sidebar for the skill detail modal. The backend lists a skill folder
 * as flat `/`-separated paths; this turns them into folders and files, pins
 * `SKILL.md` first and leaves everything else alphabetical — the order the
 * GPT skill viewer uses.
 */
import { File, FileCode2, FileText, Folder } from "lucide-react";
import type { SkillFileEntry } from "../api";
import { T } from "../i18n";

type FileNode = { kind: "file"; name: string; path: string; size: number };
type DirNode = { kind: "dir"; name: string; path: string; children: TreeNode[] };
type TreeNode = FileNode | DirNode;

function buildTree(files: SkillFileEntry[]): TreeNode[] {
  const root: DirNode = { kind: "dir", name: "", path: "", children: [] };
  for (const file of files) {
    const parts = file.path.split("/").filter(Boolean);
    if (parts.length === 0) continue;
    let dir = root;
    for (const [index, part] of parts.slice(0, -1).entries()) {
      const path = parts.slice(0, index + 1).join("/");
      let child = dir.children.find(
        (node): node is DirNode => node.kind === "dir" && node.name === part,
      );
      if (!child) {
        child = { kind: "dir", name: part, path, children: [] };
        dir.children.push(child);
      }
      dir = child;
    }
    dir.children.push({
      kind: "file",
      name: parts[parts.length - 1],
      path: file.path,
      size: file.size,
    });
  }
  const sort = (nodes: TreeNode[]): TreeNode[] =>
    nodes
      .map((node) => (node.kind === "dir" ? { ...node, children: sort(node.children) } : node))
      .sort((a, b) => {
        if (a.kind === "file" && a.path === "SKILL.md") return -1;
        if (b.kind === "file" && b.path === "SKILL.md") return 1;
        return a.name.localeCompare(b.name);
      });
  return sort(root.children);
}

const CODE_EXT = new Set([
  "bash",
  "css",
  "html",
  "js",
  "json",
  "jsx",
  "py",
  "rs",
  "sh",
  "toml",
  "ts",
  "tsx",
  "yaml",
  "yml",
  "zsh",
]);

function iconFor(name: string) {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (CODE_EXT.has(ext)) return FileCode2;
  if (ext === "md" || ext === "mdx" || ext === "txt") return FileText;
  return File;
}

function Rows({
  nodes,
  depth,
  active,
  disabled,
  onSelect,
}: {
  nodes: TreeNode[];
  depth: number;
  active: string;
  disabled: boolean;
  onSelect: (path: string) => void;
}) {
  return (
    <>
      {nodes.map((node) => {
        const indent = { paddingLeft: 10 + depth * 12 };
        if (node.kind === "dir") {
          return (
            <li key={`dir:${node.path}`}>
              <div className="skill-tree-row skill-tree-row--dir" style={indent}>
                <Folder size={13} strokeWidth={1.8} />
                <span className="skill-tree-name">{node.name}</span>
              </div>
              <ul className="skill-tree-list">
                <Rows
                  nodes={node.children}
                  depth={depth + 1}
                  active={active}
                  disabled={disabled}
                  onSelect={onSelect}
                />
              </ul>
            </li>
          );
        }
        const Icon = iconFor(node.name);
        return (
          <li key={node.path}>
            <button
              type="button"
              className={`skill-tree-row skill-tree-row--file${node.path === active ? " is-active" : ""}`}
              style={indent}
              data-testid="skill-file"
              data-path={node.path}
              aria-current={node.path === active ? "true" : undefined}
              disabled={disabled}
              onClick={() => onSelect(node.path)}
            >
              <Icon size={13} strokeWidth={1.8} />
              <span className="skill-tree-name">{node.name}</span>
            </button>
          </li>
        );
      })}
    </>
  );
}

export function SkillFileTree({
  files,
  active,
  loading = false,
  disabled = false,
  onSelect,
}: {
  files: SkillFileEntry[];
  active: string;
  loading?: boolean;
  /** Skill editing owns the textarea; switching files would drop the draft. */
  disabled?: boolean;
  onSelect: (path: string) => void;
}) {
  return (
    <aside className="skill-file-tree" data-testid="skill-file-tree">
      <div className="skill-file-tree-title">{T.skill.files}</div>
      {loading ? (
        <p className="skill-file-tree-note">{T.skill.loadingFiles}</p>
      ) : (
        <ul className="skill-tree-list">
          <Rows
            nodes={buildTree(files)}
            depth={0}
            active={active}
            disabled={disabled}
            onSelect={onSelect}
          />
        </ul>
      )}
    </aside>
  );
}
