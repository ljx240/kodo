/**
 * Deterministic demo state for the pages `demo/conversation.fixture.json` does
 * not describe (Response Trace, Archive, Settings).
 *
 * Everything here is frozen on purpose: docs/design/DEMO_DATA.md forbids live
 * clocks, random ids and real machine state in screenshot fixtures.
 */

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
export const followUps = [
  "建议为未知表路由补充集成测试，覆盖静态路由优先的场景",
  "建议在 README 中补充 topic 首次投递前需创建的前置说明",
];

export const responseMeta = {
  replyId: "rep_01J8F4QZ7Y9K3V6M2N8P",
  startedAt: "2024-12-26 10:24:32",
  completedAt: "2024-12-26 10:27:06",
  duration: "2m 34s",
  totalSteps: 10,
  workspace: "/projects/realtime-lakehouse",
  model: "Claude 3.5 Sonnet",
};

export const terminalOutput = {
  command: "docker build",
  exitCode: 0,
  lines: [
    "[+] Building 42.3s (12/12) FINISHED",
    " => [internal] load build context                                     0.5s",
    " => [1/8] FROM rust:1.75 as builder                                   2.1s",
    " => [2/8] WORKDIR /app                                                0.1s",
    " => [3/8] COPY . .                                                    0.4s",
    " => [4/8] RUN cargo build --release                                  38.7s",
    " => [5/8] FROM debian:bookworm-slim                                   0.8s",
    " => [6/8] COPY --from=builder /app/target/...                         0.2s",
    " => [7/8] RUN useradd -m k2k                                          0.1s",
    " => [8/8] CMD [\"/usr/local/bin/k2k-rust\"]                            0.2s",
    " => exporting to image                                                0.2s",
    " => => writing image sha256:9f2c...:72ad7                             0.2s",
    "Successfully built k2k-rust:latest",
  ],
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
