/**
 * Display identifier helpers for the board.
 *
 * Linear-style card identifier: a short `TODO-XXXX` prefix that gives
 * the card a recognisable anchor without leaking the internal UUID.
 *
 * We hash the UUID into a 4-hex suffix — it survives board refresh, doesn't
 * change on reorder, and stays short enough to fit on the card without
 * pushing the title off the visible area. The hash is **display-only**; the
 * real id is the UUID the board store uses for everything else.
 */

function djb2(input: string): number {
  let hash = 5381;
  for (let i = 0; i < input.length; i += 1) {
    // `| 0` forces 32-bit int so JS stays inside the unsigned range.
    hash = ((hash << 5) + hash + input.charCodeAt(i)) | 0;
  }
  return hash >>> 0;
}

/** Stable short hex suffix for the identifier. Same todo id → same suffix. */
export function todoShortId(id: string): string {
  // Low 16 bits: ids that differ only in their last character (todo-1, todo-2)
  // leave the high nibbles untouched, which would collide every card.
  return djb2(id).toString(16).padStart(8, "0").slice(-4).toUpperCase();
}

/** Full display identifier, e.g. `TODO-F47A`. */
export function todoIdentifier(id: string): string {
  return `TODO-${todoShortId(id)}`;
}