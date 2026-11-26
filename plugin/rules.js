// Editor decisions that do not draw anything, kept free of React so `node --test plugin`
// can check them against real core output.
export const GRID = 32;
export const SUGGEST_DISTANCE = 1.5 * GRID;
export const EMPTY_SELECTION = { parts: [], wires: [] };

/** A sheet differs from what was last saved, or from the blank/opened circuit it started as. */
export function isDirty(revision, savedRevision) {
  return revision !== savedRevision;
}

/** A layout can reorder segments after an edit. Match the selected wire by its geometry. */
export function selectedSegmentIndex(layout, selected) {
  if (!selected) return -1;
  return layout.segments.findIndex(segment => segment.netId === selected.netId
    && ["terminals", "from", "to"].every(key => segment[key].every((value, i) => value === selected[key][i])));
}

export function selectionBox(start, end) {
  return {
    min: start.map((value, axis) => Math.min(value, end[axis])),
    max: start.map((value, axis) => Math.max(value, end[axis])),
  };
}

/** Select symbols and orthogonal wire centerlines that touch the marquee. */
export function selectInBox(layout, box) {
  const touches = bounds => box.min.every((min, axis) => min <= bounds.max[axis] && bounds.min[axis] <= box.max[axis]);
  return {
    parts: layout.symbols.filter(symbol => touches(symbol.bounds)).map(symbol => symbol.id),
    wires: layout.segments.filter(wire => touches(selectionBox(wire.from, wire.to))),
  };
}

export function deleteSelection({ parts, wires }) {
  // Wire geometry comes from before the part deletions.
  return [
    ...wires.map(({ terminals, from, to }) => ({ op: "removeBranch", terminals, from, to })),
    ...parts.map(id => ({ op: "delete", id })),
  ];
}

/**
 * Closest terminal of another part for a part moved by offset. A suggestion only
 * attaches a free terminal: merging two existing nets is left to explicit clicks.
 */
export function nearestPair(layout, partId, [dx, dy] = [0, 0]) {
  const pins = Object.entries(layout.pins);
  let best = null;
  for (const [a, pin] of pins.filter(([, pin]) => pin.partId === partId)) {
    const from = [pin.at[0] + dx, pin.at[1] + dy];
    for (const [b, other] of pins) {
      if (other.partId === partId || (pin.netId !== null && other.netId !== null)) continue;
      const distance = Math.hypot(from[0] - other.at[0], from[1] - other.at[1]);
      if (distance <= SUGGEST_DISTANCE && (!best || distance < best.distance)) best = { a, b, from, to: other.at, distance };
    }
  }
  return best;
}
