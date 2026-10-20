export type SplitDirection = "row" | "column";

export type PaneLeaf = { kind: "leaf"; id: string };

export type PaneSplit = {
  kind: "split";
  id: string;
  direction: SplitDirection;
  /** Share of the split box given to the first child, clamped by the caller. */
  ratio: number;
  children: [PaneNode, PaneNode];
};

export type PaneNode = PaneLeaf | PaneSplit;

export type TerminalTab = {
  id: string;
  name: string;
  root: PaneNode;
  activePaneId: string;
};

export type PaneRect = {
  left: number;
  top: number;
  width: number;
  height: number;
};

let sequence = 0;

/** Stable client-side key; no crypto requirement in the webview. */
export function newId(prefix: string): string {
  sequence += 1;
  return `${prefix}-${Date.now().toString(36)}-${sequence.toString(36)}`;
}

export function paneLeaf(id: string): PaneLeaf {
  return { kind: "leaf", id };
}

export function splitPane(
  node: PaneNode,
  targetId: string,
  next: PaneLeaf,
  direction: SplitDirection,
): PaneNode {
  if (node.kind === "leaf") {
    if (node.id !== targetId) return node;
    return {
      kind: "split",
      id: newId("split"),
      direction,
      ratio: 0.5,
      children: [node, next],
    };
  }
  return {
    ...node,
    children: [
      splitPane(node.children[0], targetId, next, direction),
      splitPane(node.children[1], targetId, next, direction),
    ],
  };
}

export function setSplitRatio(node: PaneNode, splitId: string, ratio: number): PaneNode {
  if (node.kind === "leaf") return node;
  if (node.id === splitId) {
    return { ...node, ratio: Math.min(0.9, Math.max(0.1, ratio)) };
  }
  return {
    ...node,
    children: [
      setSplitRatio(node.children[0], splitId, ratio),
      setSplitRatio(node.children[1], splitId, ratio),
    ],
  };
}

/** Removes a leaf and collapses the parent split into its surviving child. */
export function removePane(node: PaneNode, paneId: string): PaneNode | null {
  if (node.kind === "leaf") return node.id === paneId ? null : node;
  const left = removePane(node.children[0], paneId);
  const right = removePane(node.children[1], paneId);
  if (left && right) return { ...node, children: [left, right] };
  return left ?? right;
}

export function paneLeaves(node: PaneNode): PaneLeaf[] {
  if (node.kind === "leaf") return [node];
  return [...paneLeaves(node.children[0]), ...paneLeaves(node.children[1])];
}

export function firstPaneId(node: PaneNode): string {
  return node.kind === "leaf" ? node.id : firstPaneId(node.children[0]);
}

const FULL: PaneRect = { left: 0, top: 0, width: 100, height: 100 };

/**
 * Absolute percentage rectangles instead of nested flex containers: every
 * pane wrapper keeps the same React parent when the tree changes, so
 * splitting or closing a neighbour never remounts (and never restarts) the
 * shell that already lives in a pane.
 */
export function paneLayout(
  node: PaneNode,
  rect: PaneRect = FULL,
): Array<{ pane: PaneLeaf; rect: PaneRect }> {
  if (node.kind === "leaf") return [{ pane: node, rect }];
  const [first, second] = node.children;
  if (node.direction === "row") {
    const width = rect.width * node.ratio;
    return [
      ...paneLayout(first, { ...rect, width }),
      ...paneLayout(second, {
        ...rect,
        left: rect.left + width,
        width: rect.width - width,
      }),
    ];
  }
  const height = rect.height * node.ratio;
  return [
    ...paneLayout(first, { ...rect, height }),
    ...paneLayout(second, {
      ...rect,
      top: rect.top + height,
      height: rect.height - height,
    }),
  ];
}

export type SplitHandle = {
  split: PaneSplit;
  /** The box the split divides; the handle sits on its ratio boundary. */
  rect: PaneRect;
};

/** Every split boundary, outermost first, ready to render as drag targets. */
export function splitHandles(node: PaneNode, rect: PaneRect = FULL): SplitHandle[] {
  if (node.kind === "leaf") return [];
  const [first, second] = node.children;
  if (node.direction === "row") {
    const width = rect.width * node.ratio;
    return [
      { split: node, rect },
      ...splitHandles(first, { ...rect, width }),
      ...splitHandles(second, {
        ...rect,
        left: rect.left + width,
        width: rect.width - width,
      }),
    ];
  }
  const height = rect.height * node.ratio;
  return [
    { split: node, rect },
    ...splitHandles(first, { ...rect, height }),
    ...splitHandles(second, {
      ...rect,
      top: rect.top + height,
      height: rect.height - height,
    }),
  ];
}
