/**
 * Build a deterministic, layered DAG layout from a session turn's `items[]`.
 *
 * Two edge families:
 *   - sequential — drawn between consecutive items in array order
 *     (the main spine of the trace)
 *   - planTrigger — drawn from each `planStep.triggeredBy[i]` to the
 *     planStep item, when the backend records sub-task dependencies
 *     (see `apps/desktop/src-tauri/src/view.rs:404-418`)
 *
 * The TS `ItemDto` union is narrower than the wire types the Rust side may
 * emit (it lacks `planStep`, `thinking`, etc.); we read defensively so the
 * layer compiles when the runtime hands us richer shapes than the contract
 * promises today.
 *
 * Layout: longest-incoming-edge layering (Sugiyama-style level assignment
 * simplified to a single pass), then column = position within level. Edges
 * are straight vertical + a short Bezier tail to avoid overlap with the
 * node rect.
 */

import type { ItemDto } from "../api";

export type TraceNodeStatus =
  | "running"
  | "done"
  | "failed"
  | "interrupted"
  | "stopped";

export type TraceEdgeKind = "sequential" | "planTrigger";

export type TraceNodeLayout = {
  /** Stable React key — "item:<u32>" */
  id: string;
  /** Backend item id (used for cross-tab anchors). */
  itemId: number;
  /** Raw `kind` string from the wire (e.g. "reasoning", "modelCall"). */
  kind: string;
  /** Localised label (e.g. "思考", "模型调用"). */
  label: string;
  status: TraceNodeStatus;
  duration: number | null;
  /** Unix seconds (live data) or fixture-stamped time. */
  time: number;
  /** One-line headline: model name, file path, command. */
  detail: string;
  /** Secondary chips (token counts, exit codes, query terms). */
  chips: string[];
  /** Sugiyama level index — vertical coordinate. */
  level: number;
  /** Position inside the level — horizontal coordinate. */
  column: number;
  /** Computed SVG coordinates. */
  x: number;
  y: number;
  width: number;
  height: number;
};

export type TraceEdgeLayout = {
  fromId: string;
  toId: string;
  kind: TraceEdgeKind;
  fromX: number;
  fromY: number;
  toX: number;
  toY: number;
};

export type TraceMapLayout = {
  nodes: TraceNodeLayout[];
  edges: TraceEdgeLayout[];
  width: number;
  height: number;
  /** When the trace exceeds the cap, the tail of the array is hidden. */
  collapsedCount: number;
};

const NODE_WIDTH = 168;
const NODE_HEIGHT = 78;
const COL_GAP = 32;
const ROW_GAP = 28;
const PADDING = 24;
const COLLAPSE_THRESHOLD = 60;

function statusOf(item: ItemDto): TraceNodeStatus {
  // `item.status` is the narrow `StepStatusDto` union today, but the Rust
  // core already emits richer statuses (interrupted/stopped). Treat it as
  // an open string so adding new wire statuses doesn't break this view.
  const status = item.status as string;
  switch (status) {
    case "running":
      return "running";
    case "failed":
      return "failed";
    case "interrupted":
      return "interrupted";
    case "stopped":
      return "stopped";
    case "done":
    default:
      return "done";
  }
}

type NodeProjection = {
  label: string;
  detail: string;
  chips: string[];
};

function projectItem(item: ItemDto): NodeProjection {
  // Read kind via an index signature so we stay compatible with the wider
  // wire set (planStep / thinking / failover / etc.) without extending
  // the public `ItemDto` union just for this view.
  const wire = item as ItemDto & Record<string, unknown>;
  const kind = wire.kind as string;
  switch (kind) {
    case "reasoning":
      return {
        label: "思考",
        detail: wire.summary ? String(wire.summary) : "",
        chips: wire.phase ? [String(wire.phase)] : [],
      };
    case "thinking":
      return {
        label: "思考",
        detail: wire.phase ? String(wire.phase) : "",
        chips: [],
      };
    case "search":
      return {
        label: "搜索代码",
        detail: String(wire.detail ?? ""),
        chips: wire.query ? [String(wire.query)] : [],
      };
    case "fileRead":
      return {
        label: "读取文件",
        detail: String(wire.path ?? ""),
        chips: [],
      };
    case "commandExecution":
      return {
        label: "执行命令",
        detail: String(wire.command ?? ""),
        chips: wire.exitCode != null ? [`exit ${wire.exitCode}`] : [],
      };
    case "modelCall":
      return {
        label: "模型调用",
        detail: String(wire.model ?? ""),
        chips: [`${wire.inputTokens ?? 0} → ${wire.outputTokens ?? 0}`],
      };
    case "fileChange": {
      const changes = Array.isArray(wire.changes) ? wire.changes : [];
      const first = changes[0] as { path?: unknown } | undefined;
      return {
        label: "修改文件",
        detail: first?.path ? String(first.path) : `${changes.length} 个文件`,
        chips: changes.length > 1 ? [`+${changes.length}`] : [],
      };
    }
    case "agentMessage":
      return {
        label: "最终回复",
        detail: String(wire.text ?? "").slice(0, 60),
        chips: [],
      };
    case "planStep":
      return {
        label: String(wire.title ?? "子任务"),
        detail: String(wire.stepKind ?? wire.step_id ?? ""),
        chips: wire.status ? [String(wire.status)] : [],
      };
    default:
      return { label: kind || "步骤", detail: "", chips: [] };
  }
}

export function buildTraceMap(items: ItemDto[]): TraceMapLayout {
  if (items.length === 0) {
    return { nodes: [], edges: [], width: 0, height: 0, collapsedCount: 0 };
  }

  const visible = items.length > COLLAPSE_THRESHOLD
    ? items.slice(0, COLLAPSE_THRESHOLD)
    : items;
  const collapsedCount = items.length - visible.length;

  const nodes: TraceNodeLayout[] = visible.map((item) => {
    const meta = projectItem(item);
    return {
      id: `item:${item.id}`,
      itemId: item.id,
      kind: (item as Record<string, unknown>).kind as string,
      label: meta.label,
      status: statusOf(item),
      duration: item.duration ?? null,
      time: item.at,
      detail: meta.detail,
      chips: meta.chips,
      level: 0,
      column: 0,
      x: 0,
      y: 0,
      width: NODE_WIDTH,
      height: NODE_HEIGHT,
    };
  });

  const edges: TraceEdgeLayout[] = [];
  // Sequential spine — one edge between every consecutive pair.
  for (let i = 0; i < nodes.length - 1; i++) {
    edges.push({
      fromId: nodes[i].id,
      toId: nodes[i + 1].id,
      kind: "sequential",
      fromX: 0,
      fromY: 0,
      toX: 0,
      toY: 0,
    });
  }

  // PlanStep cross edges.
  for (const [index, item] of visible.entries()) {
    const node = nodes[index];
    if (node.kind !== "planStep") continue;
    const wire = item as ItemDto & { triggeredBy?: number[] };
    const triggered = Array.isArray(wire.triggeredBy) ? wire.triggeredBy : [];
    for (const srcId of triggered) {
      const src = nodes.find((n) => n.itemId === srcId);
      if (!src) continue;
      // Skip duplicates already drawn as sequential edges (same pair).
      const exists = edges.some(
        (e) =>
          e.fromId === src.id &&
          e.toId === node.id &&
          e.kind === "sequential",
      );
      if (exists) continue;
      edges.push({
        fromId: src.id,
        toId: node.id,
        kind: "planTrigger",
        fromX: 0,
        fromY: 0,
        toX: 0,
        toY: 0,
      });
    }
  }

  // Sugiyama level = max(level of incoming sources) + 1.
  const levelOf = new Map<string, number>();
  for (const node of nodes) {
    const incoming = edges.filter((e) => e.toId === node.id);
    if (incoming.length === 0) {
      levelOf.set(node.id, 0);
      continue;
    }
    let maxL = 0;
    for (const e of incoming) {
      const l = levelOf.get(e.fromId) ?? 0;
      if (l > maxL) maxL = l;
    }
    levelOf.set(node.id, maxL + 1);
  }
  for (const node of nodes) node.level = levelOf.get(node.id) ?? 0;

  // Column = index inside the level (preserve array order).
  const byLevel = new Map<number, TraceNodeLayout[]>();
  for (const node of nodes) {
    const arr = byLevel.get(node.level) ?? [];
    arr.push(node);
    byLevel.set(node.level, arr);
  }
  for (const arr of byLevel.values()) {
    arr.forEach((n, i) => {
      n.column = i;
    });
  }

  const maxColumn = Math.max(0, ...Array.from(byLevel.values(), (arr) => arr.length));
  const maxLevel = Math.max(0, ...Array.from(byLevel.keys()));

  // Wrap tall linear chains: when a layout is one column wide and N rows
  // tall, fold into rows of `MAX_ROWS_PER_BAND` levels so the canvas fits
  // a normal viewport. The re-mapping is purely visual — edge geometry
  // is recomputed below.
  const MAX_ROWS_PER_BAND = 8;
  const wrappedLevels = maxColumn <= 1 && maxLevel >= MAX_ROWS_PER_BAND;
  if (wrappedLevels) {
    for (const node of nodes) {
      const band = Math.floor(node.level / MAX_ROWS_PER_BAND);
      const slot = node.level % MAX_ROWS_PER_BAND;
      node.level = band;
      node.column = slot;
    }
  }

  const finalMaxColumn = Math.max(0, ...nodes.map((n) => n.column));
  const finalMaxLevel = Math.max(0, ...nodes.map((n) => n.level));
  const width = PADDING * 2 + (finalMaxColumn + 1) * NODE_WIDTH + finalMaxColumn * COL_GAP;
  const height = PADDING * 2 + (finalMaxLevel + 1) * NODE_HEIGHT + finalMaxLevel * ROW_GAP;

  for (const node of nodes) {
    node.x = PADDING + node.column * (NODE_WIDTH + COL_GAP);
    node.y = PADDING + node.level * (NODE_HEIGHT + ROW_GAP);
  }

  for (const edge of edges) {
    const from = nodes.find((n) => n.id === edge.fromId);
    const to = nodes.find((n) => n.id === edge.toId);
    if (!from || !to) continue;
    edge.fromX = from.x + NODE_WIDTH / 2;
    edge.fromY = from.y + NODE_HEIGHT;
    edge.toX = to.x + NODE_WIDTH / 2;
    edge.toY = to.y;
  }

  return { nodes, edges, width, height, collapsedCount };
}

/** Status → i18n key under `T.lifecycle.*` (callers look it up). */
export const NODE_STATUSES: ReadonlyArray<TraceNodeStatus> = [
  "running",
  "done",
  "failed",
  "interrupted",
  "stopped",
];
