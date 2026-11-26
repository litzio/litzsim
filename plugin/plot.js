// Waveform screen decisions that do not draw: axis ticks, ranges, signal placement
// and the default run. Kept free of React so `node --test plugin` can check them.

/**
 * Trace colors tell signals apart in light and dark themes. Litzio tokens name roles, not
 * series, so these are the documented token exception (README, "화면에서 파형 보기").
 */
export const TRACE_COLORS = ["#2f6fdb", "#d4512b", "#1f8f4e", "#8b4fd6", "#c08410", "#12909e", "#c63d86", "#647488"];

const PREFIXES = [[1e9, "G"], [1e6, "M"], [1e3, "k"], [1, ""], [1e-3, "m"], [1e-6, "µ"], [1e-9, "n"], [1e-12, "p"]];

/** A value with an SI prefix: `formatValue(2e-4, "s")` is "200 µs"; without a unit, "200µ". */
export function formatValue(value, unit = "", digits = 6) {
  if (!Number.isFinite(value)) return "—";
  const magnitude = Math.abs(value);
  const [scale, prefix] = magnitude === 0 ? [1, ""]
    : PREFIXES.find(([scale]) => magnitude >= scale * (1 - 1e-9)) ?? PREFIXES.at(-1);
  const number = String(Number((value / scale).toPrecision(digits)));
  return unit ? `${number} ${prefix}${unit}` : `${number}${prefix}`;
}

/** 1, 2 or 5 × 10^k: the smallest such step dividing `span` into at most `count` intervals. */
export function niceStep(span, count) {
  if (!(span > 0) || !(count >= 1)) return 1;
  const raw = span / count;
  const power = 10 ** Math.floor(Math.log10(raw));
  return [1, 2, 5, 10].map(m => m * power).find(step => step >= raw * (1 - 1e-9));
}

/** Multiples of `step` inside [low, high], or null when there would be more than `limit`. */
export function ticks(low, high, step, limit = 400) {
  if (!(step > 0) || !(high >= low)) return [];
  const first = Math.ceil(low / step - 1e-9);
  const last = Math.floor(high / step + 1e-9);
  if (last - first + 1 > limit) return null;
  const values = [];
  for (let k = first; k <= last; ++k) values.push(Number((k * step).toPrecision(12)));
  return values;
}

/** The visible extremes of the signals on one axis, padded so traces stay inside the frame. */
export function autoRange(extremes) {
  let [low, high] = [Infinity, -Infinity];
  for (const [minimum, maximum] of extremes) {
    if (minimum === null || maximum === null) continue;
    low = Math.min(low, minimum);
    high = Math.max(high, maximum);
  }
  if (!Number.isFinite(low)) return [-1, 1];
  if (low === high) {
    const pad = Math.abs(low) * 0.1 || 1;
    return [low - pad, high + pad];
  }
  const pad = (high - low) * 0.05;
  return [low - pad, high + pad];
}

/**
 * A new signal joins the left axis when that axis is empty or shows the same unit,
 * then the right axis (with or without its unit), so two units get two scales.
 */
export function axisFor(signals, unit) {
  const left = signals.filter(signal => signal.axis === "left");
  return !left.length || left.some(signal => signal.unit === unit) ? "left" : "right";
}

/** The preferred run if it completed, otherwise the newest completed run. */
export function defaultRun(runs, preferred) {
  const completed = runs.filter(run => run.state === "completed");
  return (completed.find(run => run.runId === preferred) ?? completed[0])?.runId ?? null;
}

/** Scale [from, to] by `factor` about time `at`, inside `full` and no narrower than `minimum`. */
export function zoomRange([from, to], factor, at, full, minimum) {
  const span = Math.min(full[1] - full[0], Math.max(minimum, (to - from) * factor));
  const ratio = to > from ? (at - from) / (to - from) : 0.5;
  return clampRange(at - ratio * span, span, full);
}

/** Shift [from, to] by `shift` seconds without leaving `full`. */
export function panRange([from, to], shift, full) {
  return clampRange(from + shift, to - from, full);
}

function clampRange(from, span, [first, last]) {
  const start = Math.min(Math.max(from, first), last - span);
  return [start, start + span];
}

/** Scale an axis range by `factor` about the value at `fraction` of its height from the bottom. */
export function zoomAxis([low, high], factor, fraction) {
  const at = low + fraction * (high - low);
  return [at + (low - at) * factor, at + (high - at) * factor];
}

const signalsOf = (graph, axis) => graph.signals.filter(signal => signal.axis === axis).map(signal => signal.name).sort().join(",");

/**
 * "Apply to all graphs": grids, the shared time step and automatic choices go to every
 * graph. A typed axis range or step goes only to axes showing the same signals, so the
 * range chosen for one voltage never clips another signal.
 */
export function applyToAll(graphs, sourceId, settings) {
  const source = graphs.find(graph => graph.id === sourceId);
  return graphs.map(graph => {
    if (graph.id === sourceId) return { ...graph, settings };
    const next = { ...graph.settings, major: settings.major, minor: settings.minor, timeStep: settings.timeStep };
    for (const axis of ["left", "right"]) {
      const chosen = settings[axis];
      if (signalsOf(source, axis) && signalsOf(graph, axis) === signalsOf(source, axis)) {
        next[axis] = chosen;
        continue;
      }
      next[axis] = { ...next[axis] };
      if (chosen.auto) next[axis].auto = true;
      if (chosen.step === null) next[axis].step = null;
    }
    return { ...graph, settings: next };
  });
}
