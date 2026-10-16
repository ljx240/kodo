/**
 * Render `buildTraceMap()` as an inline SVG: a layered DAG with sequential
 * edges along the main spine and dashed cross edges when a `planStep` lists
 * `triggeredBy` items.
 *
 * Status uses a class + a word — never a colour alone (UI_ACCEPTANCE §4).
 * All visuals come from `kodo-*` tokens; no hard-coded hex.
 */

import { useMemo } from "react";
import type { ItemDto } from "../api";
import { T } from "../i18n";
import { buildTraceMap, NODE_STATUSES, type TraceNodeLayout, type TraceNodeStatus } from "./build";

const ARROW_ID = "kodo-trace-map-arrow";
const ARROW_TRIGGER_ID = "kodo-trace-map-arrow-trigger";

function statusLabel(status: TraceNodeStatus): string {
  switch (status) {
    case "running":
      return T.lifecycle.working;
    case "done":
      return T.lifecycle.completed;
    case "failed":
      return T.lifecycle.failed;
    case "interrupted":
      return T.lifecycle.interrupted;
    case "stopped":
      return T.lifecycle.stopped;
  }
}

/** Truncate a long detail line so the node rect never overflows. */
function truncate(value: string, max: number): string {
  if (value.length <= max) return value;
  return `${value.slice(0, Math.max(0, max - 1))}…`;
}

/** Cubic-Bezier path: vertical spine, then horizontal arc into the target. */
function edgePath(fromX: number, fromY: number, toX: number, toY: number): string {
  const midY = (fromY + toY) / 2;
  // Keep the curve flat in the middle so vertical arrows read as such.
  return `M ${fromX} ${fromY} C ${fromX} ${midY}, ${toX} ${midY}, ${toX} ${toY}`;
}

export function TraceMap({
  items,
  onSelectNode,
}: {
  items: ItemDto[];
  onSelectNode?: (itemId: number) => void;
}) {
  const layout = useMemo(() => buildTraceMap(items), [items]);

  if (layout.nodes.length === 0) {
    return <p className="empty-note">{T.page.traceMap.empty}</p>;
  }

  return (
    <div className="trace-map">
      <div className="trace-map-legend" role="list" aria-label={T.page.traceMap.legend}>
        {NODE_STATUSES.map((status) => (
          <span
            key={status}
            role="listitem"
            className={`trace-map-legend-item trace-map-legend-item--${status}`}
          >
            <span className="trace-map-legend-swatch" aria-hidden="true" />
            <span>{statusLabel(status)}</span>
          </span>
        ))}
      </div>

      {layout.collapsedCount > 0 && (
        <p className="trace-map-collapsed">
          {T.page.traceMap.collapsedNodes(layout.collapsedCount)}
        </p>
      )}

      <div className="trace-map-svg-wrap">
        <svg
          className="trace-map-svg"
          viewBox={`0 0 ${Math.max(layout.width, 1)} ${Math.max(layout.height, 1)}`}
          preserveAspectRatio="xMidYMid meet"
          role="img"
          aria-label={T.page.responseTrace}
        >
        <defs>
          <marker
            id={ARROW_ID}
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="6"
            markerHeight="6"
            orient="auto"
          >
            <path d="M 0 0 L 10 5 L 0 10 z" className="trace-map-edge-arrow" />
          </marker>
          <marker
            id={ARROW_TRIGGER_ID}
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="6"
            markerHeight="6"
            orient="auto"
          >
            <path d="M 0 0 L 10 5 L 0 10 z" className="trace-map-edge-arrow trace-map-edge-arrow--trigger" />
          </marker>
        </defs>

        <g className="trace-map-edges">
          {layout.edges.map((edge, idx) => {
            const markerEnd =
              edge.kind === "planTrigger"
                ? `url(#${ARROW_TRIGGER_ID})`
                : `url(#${ARROW_ID})`;
            return (
              <path
                key={`${edge.fromId}->${edge.toId}-${idx}`}
                className={`trace-map-edge trace-map-edge--${edge.kind}`}
                d={edgePath(edge.fromX, edge.fromY, edge.toX, edge.toY)}
                markerEnd={markerEnd}
              />
            );
          })}
        </g>

        <g className="trace-map-nodes">
          {layout.nodes.map((node) => (
            <NodeShape
              key={node.id}
              node={node}
              statusText={statusLabel(node.status)}
              onSelect={onSelectNode}
            />
          ))}
        </g>
      </svg>
      </div>
    </div>
  );
}

function NodeShape({
  node,
  statusText,
  onSelect,
}: {
  node: TraceNodeLayout;
  statusText: string;
  onSelect?: (itemId: number) => void;
}) {
  const handleClick = () => {
    if (onSelect) onSelect(node.itemId);
  };
  const handleKey = (e: React.KeyboardEvent<SVGGElement>) => {
    if (!onSelect) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onSelect(node.itemId);
    }
  };

  const detailText = truncate(node.detail, 24);
  const chipsText = node.chips.map((c) => truncate(c, 14)).join(" · ");

  return (
    <g
      className={`trace-map-node trace-map-node--${node.status}`}
      data-item-id={node.itemId}
      tabIndex={onSelect ? 0 : -1}
      role={onSelect ? "button" : "group"}
      aria-label={T.page.traceMap.nodeAria(node.label, statusText)}
      onClick={handleClick}
      onKeyDown={handleKey}
    >
      <rect
        className="trace-map-node-rect"
        x={node.x}
        y={node.y}
        width={node.width}
        height={node.height}
        rx="10"
        ry="10"
      />
      <text
        className="trace-map-node-label"
        x={node.x + 12}
        y={node.y + 22}
      >
        {truncate(node.label, 14)}
      </text>
      <text
        className="trace-map-node-status"
        x={node.x + 12}
        y={node.y + 40}
      >
        {statusText}
      </text>
      <text
        className="trace-map-node-detail"
        x={node.x + 12}
        y={node.y + 58}
      >
        {detailText}
      </text>
      {chipsText ? (
        <text
          className="trace-map-node-chips"
          x={node.x + 12}
          y={node.y + 72}
        >
          {chipsText}
        </text>
      ) : null}
    </g>
  );
}