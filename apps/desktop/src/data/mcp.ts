/**
 * Connector-catalog templates — the `PROVIDER_TEMPLATES` pattern applied to
 * the MCP page's card grid. Each entry prefills command/args or URL from the
 * well-known community server; secret halves (env / header values) stay blank
 * for the user to fill before saving, so no key ever lives in this file.
 * `icon` is a key into the page's `MCP_ICONS` map (identifiers, not copy).
 */

export type McpTemplate = {
  id: string;
  /** Display name seeded into the editor; the user can rename it. */
  name: string;
  transport: "stdio" | "http";
  /** stdio: the executable to spawn. */
  command: string;
  /** stdio: argv after the executable (whitespace-joined in the editor). */
  args: string[];
  /** http: the Streamable HTTP endpoint. */
  url: string;
  /** Secret env keys (stdio) — pre-created as blank rows in the editor. */
  envKeys: string[];
  /** Secret header keys (http). */
  headerKeys: string[];
  /** Key into MCP_ICONS on the MCP page. */
  icon: string;
};

export const MCP_TEMPLATES: McpTemplate[] = [
  {
    id: "filesystem",
    name: "filesystem",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allowed"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "folder",
  },
  {
    id: "github",
    name: "github",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-github"],
    url: "",
    envKeys: ["GITHUB_PERSONAL_ACCESS_TOKEN"],
    headerKeys: [],
    icon: "git-branch",
  },
  {
    id: "gitlab",
    name: "gitlab",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-gitlab"],
    url: "",
    envKeys: ["GITLAB_PERSONAL_ACCESS_TOKEN"],
    headerKeys: [],
    icon: "git-merge",
  },
  {
    id: "postgres",
    name: "postgres",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-postgres", "postgresql://user:pass@localhost:5432/db"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "database",
  },
  {
    id: "sqlite",
    name: "sqlite",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-sqlite", "/path/to/db.sqlite"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "table",
  },
  {
    id: "memory",
    name: "memory",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-memory"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "brain",
  },
  {
    id: "puppeteer",
    name: "puppeteer",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-puppeteer"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "monitor-play",
  },
  {
    id: "brave-search",
    name: "brave-search",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-brave-search"],
    url: "",
    envKeys: ["BRAVE_API_KEY"],
    headerKeys: [],
    icon: "shield",
  },
  {
    id: "slack",
    name: "slack",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-slack"],
    url: "",
    envKeys: ["SLACK_BOT_TOKEN", "SLACK_TEAM_ID"],
    headerKeys: [],
    icon: "message-square",
  },
  {
    id: "notion",
    name: "notion",
    transport: "http",
    command: "",
    args: [],
    url: "https://mcp.notion.com/mcp",
    envKeys: [],
    headerKeys: ["Authorization"],
    icon: "file-text",
  },
  {
    id: "sequential-thinking",
    name: "sequential-thinking",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-sequential-thinking"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "list-tree",
  },
  {
    id: "context7",
    name: "context7",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@upstash/context7-mcp"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "book-open",
  },
  {
    id: "playwright",
    name: "playwright",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@playwright/mcp@latest"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "play",
  },
  {
    id: "google-drive",
    name: "google-drive",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-gdrive"],
    url: "",
    envKeys: ["GDRIVE_CREDENTIALS"],
    headerKeys: [],
    icon: "hard-drive",
  },
  {
    id: "redis",
    name: "redis",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-redis", "redis://localhost:6379"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "layers",
  },
  {
    id: "everything",
    name: "everything",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-everything"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "boxes",
  },
  {
    id: "figma",
    name: "figma",
    transport: "stdio",
    command: "npx",
    args: ["-y", "figma-developer-mcp"],
    url: "",
    envKeys: ["FIGMA_API_KEY"],
    headerKeys: [],
    icon: "pen-tool",
  },
  {
    id: "linear",
    name: "linear",
    transport: "http",
    command: "",
    args: [],
    url: "https://mcp.linear.app/mcp",
    envKeys: [],
    headerKeys: ["Authorization"],
    icon: "square-kanban",
  },
  {
    id: "fetch",
    name: "fetch",
    transport: "stdio",
    command: "uvx",
    args: ["mcp-server-fetch"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "globe",
  },
  {
    id: "time",
    name: "time",
    transport: "stdio",
    command: "uvx",
    args: ["mcp-server-time"],
    url: "",
    envKeys: [],
    headerKeys: [],
    icon: "clock",
  },
];

export function mcpTemplateById(id: string): McpTemplate | undefined {
  return MCP_TEMPLATES.find((t) => t.id === id);
}
