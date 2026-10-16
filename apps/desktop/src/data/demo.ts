/**
 * Deterministic demo state for the pages `demo/conversation.fixture.json` does
 * not describe (Response Trace, Archive, Settings).
 *
 * Everything here is frozen on purpose: docs/design/DEMO_DATA.md forbids live
 * clocks, random ids and real machine state in screenshot fixtures.
 */

import type {
  ArtifactRefDto,
  FileSpanDto,
  ItemDto,
  LlmIoDto,
  McpServerDto,
  McpToolDto,
  MemoryEntryDto,
} from "../api";

export type TimelineRow = {
  n: number;
  time: string;
  duration: string;
  type: "Thinking" | "Search" | "Read" | "Run" | "Output" | "Model" | "Edit" | "Finalize";
  /** Bold lead line of the Details column. */
  title: string;
  /** Muted second line of the Details column. */
  note: string;
  chip?: string;
  chips?: string[];
  delta?: { added: number; removed: number };
  ok?: boolean;
};

export const timeline: TimelineRow[] = [
  {
    n: 1,
    time: "10:24:32",
    duration: "12s",
    type: "Thinking",
    title: "分析需求，制定检查计划",
    note: "梳理 k2k-rust 项目的路径结构和核心模块",
  },
  {
    n: 2,
    time: "10:24:44",
    duration: "8s",
    type: "Search",
    title: "Search codebase",
    note: "在项目中搜索相关实现，定位路由与 heartbeat 处理逻辑",
    chips: ["k2k", "router", "heartbeat"],
  },
  {
    n: 3,
    time: "10:24:52",
    duration: "14s",
    type: "Read",
    title: "Read file",
    note: "查看路由实现与核心逻辑",
    chip: "source/db/source.table",
  },
  {
    n: 4,
    time: "10:25:06",
    duration: "18s",
    type: "Run",
    title: "Run command",
    note: "检查项目编译状态",
    chip: "cargo check --workspace",
  },
  {
    n: 5,
    time: "10:25:24",
    duration: "36s",
    type: "Output",
    title: "Command output",
    note: "编译检查通过：0 errors, 12 warnings",
    ok: true,
  },
  {
    n: 6,
    time: "10:26:00",
    duration: "24s",
    type: "Model",
    title: "Model call #1",
    note: "分析检查结果，制定下一步构建和测试计划",
    chips: ["Claude 3.5 Sonnet", "12.4k tokens"],
  },
  {
    n: 7,
    time: "10:26:24",
    duration: "28s",
    type: "Edit",
    title: "Edit file",
    note: "完善未知表路由的处理逻辑，增加日志和边界检查",
    chip: "src/router/mod.rs",
    delta: { added: 42, removed: 8 },
  },
  {
    n: 8,
    time: "10:26:52",
    duration: "42s",
    type: "Run",
    title: "Run command",
    note: "构建镜像进行集成验证",
    chip: "docker build",
  },
  {
    n: 9,
    time: "10:27:34",
    duration: "52s",
    type: "Model",
    title: "Model call #2",
    note: "基于构建结果进行最终验证，总结修改内容",
    chips: ["Claude 3.5 Sonnet", "14.5k tokens"],
  },
  {
    n: 10,
    time: "10:28:26",
    duration: "8s",
    type: "Finalize",
    title: "Finalize answer",
    note: "整理测试结论与改进建议，生成最终回复",
  },
];

/** Rendered as the collapsed "后续建议" disclosure under the final answer. */
export const responseMeta = {
  replyId: "rep_01J8F4QZ7Y9K3V6M2N8P",
  startedAt: "2024-12-26 10:24:32",
  completedAt: "2024-12-26 10:27:06",
  duration: "2m 34s",
  totalSteps: 10,
  workspace: "/projects/realtime-lakehouse",
  model: "Claude 3.5 Sonnet",
};

export const llmCalls = [
  {
    n: 1,
    model: "Claude 3.5 Sonnet",
    at: "10:26:00",
    inputTokens: "12.4k",
    outputTokens: "2.1k",
    duration: "24s",
  },
  {
    n: 2,
    model: "Claude 3.5 Sonnet",
    at: "10:27:34",
    inputTokens: "14.5k",
    outputTokens: "2.3k",
    duration: "52s",
  },
];

export type ArchivedConversation = {
  id: string;
  title: string;
  summary: string;
  archivedAt: string;
  model: "Claude 3.5" | "GPT-4o";
  added: number;
  removed: number;
};

export const archivedConversations: ArchivedConversation[] = [
  { id: "a1", title: "修复 k2k-rust 未知表路由", summary: "排查并修复 k2k-rust 项目的未知表路由问题", archivedAt: "Today 10:25", model: "Claude 3.5", added: 7, removed: 46 },
  { id: "a2", title: "优化实时数仓搭建流程", summary: "梳理实时数仓架构并优化部署流程", archivedAt: "Today 09:12", model: "Claude 3.5", added: 12, removed: 8 },
  { id: "a3", title: "排查 Flink 任务延迟问题", summary: "分析 Flink 任务延迟的根因，检查背压与状态后端配置", archivedAt: "Apr 28, 2024", model: "GPT-4o", added: 29, removed: 14 },
  { id: "a4", title: "设计数据模型 v2", summary: "设计新版本的数据模型，支持多租户与增量同步", archivedAt: "Apr 27, 2024", model: "Claude 3.5", added: 18, removed: 3 },
  { id: "a5", title: "实现用户行为分析报表", summary: "开发用户行为分析报表，包含留存与漏斗指标", archivedAt: "Apr 26, 2024", model: "Claude 3.5", added: 24, removed: 11 },
  { id: "a6", title: "修复 Kafka 消费位点问题", summary: "解决 Kafka 消费位点重置导致的重复消费", archivedAt: "Apr 25, 2024", model: "GPT-4o", added: 8, removed: 21 },
  { id: "a7", title: "升级 docker base image", summary: "升级基础镜像到 Ubuntu 24.04 并验证兼容性", archivedAt: "Apr 24, 2024", model: "Claude 3.5", added: 6, removed: 4 },
  { id: "a8", title: "重构数据同步模块", summary: "重构数据同步模块，提高稳定性与可测试性", archivedAt: "Apr 22, 2024", model: "Claude 3.5", added: 37, removed: 28 },
  { id: "a9", title: "添加数据质量监控", summary: "实现数据质量监控告警，包括完整性校验", archivedAt: "Apr 21, 2024", model: "Claude 3.5", added: 15, removed: 6 },
  { id: "a10", title: "调研 Iceberg 集成方案", summary: "调研并验证 Iceberg 在实时数仓中的可行性", archivedAt: "Apr 20, 2024", model: "GPT-4o", added: 9, removed: 2 },
  { id: "a11", title: "修复前端页面样式问题", summary: "修复控制台页面的样式兼容性问题", archivedAt: "Apr 18, 2024", model: "Claude 3.5", added: 4, removed: 12 },
  { id: "a12", title: "优化查询性能", summary: "分析并优化大表查询性能，添加合适的索引", archivedAt: "Apr 16, 2024", model: "Claude 3.5", added: 21, removed: 17 },
  { id: "a13", title: "实现离线数据回补任务", summary: "开发离线数据回补的定时任务，支持断点续传", archivedAt: "Apr 14, 2024", model: "Claude 3.5", added: 13, removed: 9 },
  { id: "a14", title: "升级依赖版本", summary: "批量升级项目依赖版本，解决安全告警", archivedAt: "Apr 12, 2024", model: "GPT-4o", added: 3, removed: 3 },
  { id: "a15", title: "添加审计日志", summary: "实现操作审计日志记录，支持查询与导出", archivedAt: "Apr 10, 2024", model: "Claude 3.5", added: 11, removed: 5 },
  { id: "a16", title: "修复任务调度异常", summary: "修复定时任务在特定条件下不执行的问题", archivedAt: "Apr 8, 2024", model: "Claude 3.5", added: 7, removed: 13 },
  { id: "a17", title: "改进错误提示信息", summary: "优化错误提示的友好性和可操作性", archivedAt: "Apr 5, 2024", model: "Claude 3.5", added: 5, removed: 8 },
  { id: "a18", title: "设计多租户权限模型", summary: "设计并实现多租户的权限控制模型", archivedAt: "Apr 3, 2024", model: "Claude 3.5", added: 19, removed: 14 },
  { id: "a19", title: "集成 Prometheus 监控", summary: "集成 Prometheus 监控指标，添加 Grafana 面板", archivedAt: "Apr 1, 2024", model: "GPT-4o", added: 16, removed: 7 },
  { id: "a20", title: "编写部署文档", summary: "整理项目的部署文档和运维指引", archivedAt: "Mar 28, 2024", model: "Claude 3.5", added: 8, removed: 2 },
];

export const archiveFooter = { from: 1, to: 20, total: 42, pages: [1, 2, 3] };

/**
 * Demo fixture for the MCP page on `/ui-demo/mcp` (plain browser): one
 * enabled stdio server with a masked env value and one disabled http server,
 * so the screenshot shows both transport chips and both toggle states. The
 * masks are display-only stand-ins — a real workspace loads its own
 * `mcp-servers` entry with per-value masks from `mask_mcp_values`.
 */
export const DEMO_MCP_SERVERS: McpServerDto[] = [
  {
    id: "demo-filesystem",
    name: "filesystem",
    enabled: true,
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
    env: { API_TOKEN: "••••9f3a" },
    url: "",
    headers: {},
  },
  {
    id: "demo-remote",
    name: "remote-tools",
    enabled: false,
    transport: "http",
    command: "",
    args: [],
    env: {},
    url: "https://mcp.example.com/mcp",
    headers: { Authorization: "••••c21b" },
  },
];

/**
 * Item-level fixture for the Response Trace page (UI demo / plain browser).
 *
 * Mirrors `timeline` row-for-row so the "地图" tab can draw a 10-node
 * trace without going through `load_session`.
 *
 * Time stamps are frozen (DESIGN §11/DEMO_DATA fixture rule); only relative
 * differences matter for the visual.
 */
export const demoItems: ItemDto[] = [
  {
    id: 1,
    at: 1735194272,
    status: "done",
    duration: 12,
    kind: "reasoning",
    summary: "梳理 k2k-rust 项目的路径结构和核心模块",
  },
  {
    id: 2,
    at: 1735194284,
    status: "done",
    duration: 8,
    kind: "search",
    query: "router heartbeat",
    detail: "搜索路由与 heartbeat 处理实现",
  },
  {
    id: 3,
    at: 1735194292,
    status: "done",
    duration: 14,
    kind: "fileRead",
    path: "source/db/source.table",
    detail: "查看路由实现与核心逻辑",
  },
  {
    id: 4,
    at: 1735194306,
    status: "done",
    duration: 18,
    kind: "commandExecution",
    command: "cargo check --workspace",
    cwd: "/projects/realtime-lakehouse",
    output: "Compiling k2k-router v0.23.0\nFinished release [optimized] target(s)",
    exitCode: 0,
  },
  {
    id: 5,
    at: 1735194324,
    status: "done",
    duration: 36,
    kind: "commandExecution",
    command: "docker build",
    cwd: "/projects/realtime-lakehouse",
    output: "sha256:9f2a… built",
    exitCode: 0,
  },
  {
    id: 6,
    at: 1735194360,
    status: "done",
    duration: 24,
    kind: "modelCall",
    model: "Claude 3.5 Sonnet",
    inputTokens: 12400,
    outputTokens: 2100,
  },
  {
    id: 7,
    at: 1735194384,
    status: "done",
    duration: 28,
    kind: "fileChange",
    changes: [{ path: "src/router/mod.rs", added: 42, removed: 8 }],
  },
  {
    id: 8,
    at: 1735194412,
    status: "failed",
    duration: 42,
    kind: "commandExecution",
    command: "docker run",
    cwd: "/projects/realtime-lakehouse",
    output: "Error response from daemon: port already allocated",
    exitCode: 125,
  },
  {
    id: 9,
    at: 1735194454,
    status: "done",
    duration: 52,
    kind: "modelCall",
    model: "Claude 3.5 Sonnet",
    inputTokens: 14500,
    outputTokens: 2300,
  },
  {
    id: 10,
    at: 1735194506,
    status: "done",
    duration: 8,
    kind: "agentMessage",
    text: "整理测试结论与改进建议，生成最终回复。",
    checks: [],
  },
];

/**
 * File-span captures (before/after) for the demo Edit step (item 7 in
 * `demoItems`). The artifacts tab on `/ui-demo/trace` opens the same
 * `FileSpanModal` the spans tab uses; the live build loads these from
 * `traces/<session>/<turn>/spans/*.json` via `load_file_span`.
 *
 * The `refPath` matches what the live artifact ref would carry, so the
 * same `loadFileSpan`-style lookup keeps working when this fixture is
 * swapped for live data.
 */
export const demoSpans: FileSpanDto[] = [
  {
    path: "src/router/mod.rs",
    contentBefore:
      "fn route(&self, table: &str) -> Option<Route> {\n    self.table_routes.get(table).cloned()\n}",
    contentAfter:
      "fn route(&self, table: &str) -> Option<Route> {\n    if let Some(route) = self.table_routes.get(table) {\n        return Some(route.clone());\n    }\n    if table.starts_with(\"k2k_legacy_\") {\n        warn!(\"falling back to legacy router for {}\", table);\n        return self.legacy_route(table);\n    }\n    None\n}",
    toolCallId: "toolu_demo_007",
    seq: 1,
  },
];

/**
 * LLM I/O captures for the demo Model call steps (items 6 and 9 in
 * `demoItems`). The LLM I/O tab on `/ui-demo/trace` reads these directly
 * when in demo mode so the page shows real content rather than the
 * "no capture" empty note.
 */
export const demoLlmIo: LlmIoDto[] = [
  {
    seq: 1,
    model: "Claude 3.5 Sonnet",
    protocol: "anthropic-messages",
    retryIndex: 0,
    providerLatencyMs: 24000,
    finishReason: "end_turn",
    inputTokens: 12400,
    outputTokens: 2100,
    sawReasoning: true,
    at: 1735194360,
    request: {
      system:
        "你是 Kodo，一名工作于 <kodo> 环境的代码助手。\n以中文回答，命令、模型名、路径保留原文。",
      messages: [
        { role: "user", content: "分析 cargo check / docker build 的输出，构建校验报告。" },
        { role: "assistant", content: "编译通过；构建镜像成功。让我整理校验报告…" },
      ],
      tools: [
        { name: "edit_file", description: "Make selective edits with string replacement." },
      ],
    },
    response: {
      text: "校验报告：\n- cargo check --workspace 通过（0 errors, 12 warnings）\n- docker build 镜像构建成功\n- 建议下一步：补 unit tests 并接 CI。",
      reasoningContent:
        "I should summarize the build results and propose the next concrete step (add tests + CI).",
      nativeToolCalls: [
        { name: "edit_file", arguments: { path: "src/router/mod.rs" } },
      ],
    },
  },
  {
    seq: 2,
    model: "Claude 3.5 Sonnet",
    protocol: "anthropic-messages",
    retryIndex: 0,
    providerLatencyMs: 52000,
    finishReason: "end_turn",
    inputTokens: 14500,
    outputTokens: 2300,
    sawReasoning: true,
    at: 1735194454,
    request: {
      system:
        "你是 Kodo，一名工作于 <kodo> 环境的代码助手。\n以中文回答，命令、模型名、路径保留原文。",
      messages: [
        { role: "user", content: "基于构建结果做最终验证，并总结修改内容。" },
      ],
      tools: [
        { name: "edit_file", description: "Make selective edits with string replacement." },
      ],
    },
    response: {
      text: "最终验证：路由改动覆盖 k2k_legacy_* 路径，集成测试通过。变更摘要：+737 / -46 行。",
      reasoningContent:
        "All targeted paths verified; aggregate change +737/-46 matches demo summary.",
      nativeToolCalls: [],
    },
  },
];

/**
 * Artifact refs (what `list_artifacts` returns in the live build). The
 * LLM I/O tab on `/ui-demo/trace` walks this list to render the call bar.
 */
export const demoArtifactRefs: ArtifactRefDto[] = [
  { kind: "llm_io", seq: 1, label: "demo-llm-io/1.json", sizeBytes: 4096, at: 1735194360 },
  { kind: "llm_io", seq: 2, label: "demo-llm-io/2.json", sizeBytes: 5120, at: 1735194454 },
  { kind: "file_span", seq: 1, label: "demo-file-span/1.json", sizeBytes: 2048, at: 1735194384 },
];

/**
 * Demo fixture for the per-server tool list on `/ui-demo/mcp` (plain browser):
 * names and descriptions only — the real page reads these through
 * `mcp_list_tools`, which never hands schemas or secrets to the webview.
 * Tool names are wire-adjacent code; descriptions stay English like the rest
 * of the recorded fixture data (DESIGN §10).
 */
export const DEMO_MCP_TOOLS: Record<string, McpToolDto[]> = {
  "demo-filesystem": [
    { name: "read_file", description: "Read the complete contents of a file." },
    { name: "read_multiple_files", description: "Read several files in one call." },
    { name: "write_file", description: "Create or overwrite a file with new content." },
    { name: "edit_file", description: "Make selective edits with string replacement." },
    { name: "list_directory", description: "List entries with [FILE] / [DIR] markers." },
    { name: "directory_tree", description: "Return a recursive JSON tree of a directory." },
    { name: "search_files", description: "Search files matching a text pattern." },
    { name: "get_file_info", description: "Report size, timestamps and permissions." },
  ],
  "demo-remote": [
    { name: "fetch_ticket", description: "Fetch one ticket with its comments." },
    { name: "search_tickets", description: "Search tickets by query string." },
  ],
};

/**
 * Memory entries backing the Inspector Memory tab on `/ui-demo/*`.
 * The real Memory engine is Tauri-side and only runs in the desktop
 * shell; in the browser the `memoryList` stub returns these rows so the
 * visual layout / row chrome can be reviewed without a Tauri host.
 *
 * Frozen timestamps so screenshot baselines stay byte-identical; see
 * docs/design/DEMO_DATA.md for the clock rule.
 */
export const demoMemoryEntries: MemoryEntryDto[] = [
  {
    id: "mem-0001",
    kind: "semantic",
    status: "active",
    authority: "user_rule",
    sourceTrust: "test_verified",
    title: "k2k-rust 未知表路由走 Debezium 动态注册",
    abstractText:
      "新增表的 topic 在路由层不要硬编码; 通过 Debezium source.db/source.table 生成, 静态路由优先, 首次投递前需创建目标 topic。",
    body: null,
    contentHash: "h-7a3f",
    confidence: 0.92,
    verified: true,
    conditions: ["routing == dynamic", "source.kind == debezium"],
    sourceUris: ["repo:k2k-rust/src/router/mod.rs"],
    scopeProjectId: "realtime-lakehouse",
    scopeRepositoryId: "k2k-rust",
    scopeBranch: "main",
    useCount: 7,
    version: 3,
    createdAt: 1_700_000_100,
    updatedAt: 1_700_000_900,
  },
  {
    id: "mem-0002",
    kind: "procedural",
    status: "active",
    authority: "historical",
    sourceTrust: "tool_verified",
    title: "Flink 任务延迟排查：先看 heartbeat, 再看 checkpoint",
    abstractText:
      "延迟上升先看 heartbeat.rs 的处理频率; 若心跳正常则对比最近 24h checkpoint 时长, 单点 > 5s 多为背压。",
    body: null,
    contentHash: "h-2c81",
    confidence: 0.81,
    verified: true,
    conditions: ["symptom == flink_latency"],
    sourceUris: ["repo:k2k-rust/src/heartbeat.rs", "runbook:flink-latency"],
    scopeProjectId: "realtime-lakehouse",
    scopeRepositoryId: "k2k-rust",
    scopeBranch: "main",
    useCount: 4,
    version: 2,
    createdAt: 1_700_001_000,
    updatedAt: 1_700_001_500,
  },
  {
    id: "mem-0003",
    kind: "evidence_ref",
    status: "active",
    authority: "historical",
    sourceTrust: "tool_verified",
    title: "实时数仓 schema 注册流程",
    abstractText:
      "DDL → schema-registry 校验 → topic 创建 → Flink Source 订阅, 缺一步下游就拿不到字段。",
    body: null,
    contentHash: "h-91ab",
    confidence: 0.7,
    verified: false,
    conditions: [],
    sourceUris: ["docs:realtime-warehouse/schema.md"],
    scopeProjectId: "realtime-lakehouse",
    scopeRepositoryId: null,
    scopeBranch: null,
    useCount: 1,
    version: 1,
    createdAt: 1_700_002_000,
    updatedAt: 1_700_002_000,
  },
  {
    id: "mem-0004",
    kind: "rule",
    status: "active",
    authority: "user_rule",
    sourceTrust: "user_confirmed",
    title: "PR 提交前必须跑全量本地测试 + 离线检查",
    abstractText: "Kodo 自动注入到 pre-push hook, 若跳过会被 PR 检查拒收。",
    body: null,
    contentHash: "h-rule-1",
    confidence: 1,
    verified: true,
    conditions: ["stage == pr_open"],
    sourceUris: ["settings:pre-push"],
    scopeProjectId: null,
    scopeRepositoryId: null,
    scopeBranch: null,
    useCount: 12,
    version: 1,
    createdAt: 1_700_003_000,
    updatedAt: 1_700_003_000,
  },
  {
    id: "mem-0005",
    kind: "episodic",
    status: "candidate",
    authority: "historical",
    sourceTrust: "untrusted",
    title: "5-28 排查 Flink 任务延迟的临时结论",
    abstractText: "怀疑是 source.db 重新订阅后 heartbeat 没跟上, 监控已加重试; 待下次复发再确认。",
    body: null,
    contentHash: "h-epi-5",
    confidence: 0.4,
    verified: false,
    conditions: [],
    sourceUris: ["session:c3#turn-2"],
    scopeProjectId: "realtime-lakehouse",
    scopeRepositoryId: "k2k-rust",
    scopeBranch: "main",
    useCount: 0,
    version: 1,
    createdAt: 1_700_004_000,
    updatedAt: 1_700_004_000,
  },
];
