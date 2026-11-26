// Editor decisions that do not draw anything, kept free of React so `node --test plugin`
// can check them against real core output.
export const GRID = 32;
export const SUGGEST_DISTANCE = 1.5 * GRID;

/** A sheet differs from what was last saved, or from the blank/opened circuit it started as. */
export function isDirty(revision, savedRevision) {
  return revision !== savedRevision;
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
