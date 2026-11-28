// Editor decisions that do not draw anything, kept free of React so `node --test plugin`
// can check them against real core output.
export const GRID = 32;
export const SUGGEST_DISTANCE = 1.5 * GRID;
export const EMPTY_SELECTION = { parts: [], wires: [] };

/** Apply the paper accent only to nets classified as signals by the core. */
export const SIGNAL_INK = "var(--id-color-accent-blue-paper)";

export function wireStyle(signalNets, netId) {
  return signalNets.includes(netId) ? { stroke: SIGNAL_INK, strokeWidth: 1.5 } : {};
}

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
 * The core validates the connection when the candidate is accepted.
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

/** The core request for a typed part name; none when it names the part as it is. The core checks the name. */
export function renameOps(id, text) {
  const newId = text.trim();
  return newId === id ? [] : [{ op: "rename", id, newId }];
}

/** Selected IDs after an edit whose receipt renamed parts (`renamed` maps old ID → new ID). */
export function renamedIds(ids, renamed = {}) {
  return ids.map(id => Object.hasOwn(renamed, id) ? renamed[id] : id);
}

/** Undoing an edit gives its renamed parts their old IDs back. */
export function undoRenamed(receipt) {
  return Object.fromEntries(Object.entries(receipt.renamed ?? {}).map(([from, to]) => [to, from]));
}

/** The name field's reason for the core refusing a rename; other failures keep the core message. */
export function renameProblem(error) {
  const message = error?.message ?? "";
  const taken = /already in use: (\S+)/.exec(message);
  if (taken) return `${taken[1]}이(가) 이미 쓰는 이름입니다. 대소문자만 다른 이름도 쓸 수 없습니다.`;
  const length = /longer than (\d+)/.exec(message);
  if (length) return `${length[1]}자 이하로 입력하세요.`;
  if (/new ID is empty/.test(message)) return "이름을 입력하세요.";
  if (/invalid new ID/.test(message)) return "영문자로 시작하고 영문자·숫자·밑줄(_)만 쓸 수 있습니다.";
  return message || "이름을 바꾸지 못했습니다.";
}
export function sumParameterOps(id, key, value) {
  return [{ op: "setParameter", id, key, value }];
}

export function pwmChoices(parts, dutyInputOnly = false) {
  return Object.entries(parts).filter(([, part]) => part.type === "dutyPwm" || (!dutyInputOnly && part.type === "pwm"))
    .map(([id]) => ({ value: id, label: id }));
}

const PREVIEW_SOURCES = new Set(["constant", "step", "ramp", "dc", "sine", "pwm", "timedSwitch"]);

export function sourcePreviewRequest(document, partId) {
  return PREVIEW_SOURCES.has(document.parts[partId]?.type) ? { document, partId } : null;
}

// Window notifications (`ui.Notifications`, SDK 0.23.0). The center hides a dismissed card
// while its ID stays listed, so an ID is reused only to update the same cause.

/** The text a card shows for a core or Host failure; never empty. */
export function failureText(failure) {
  const message = failure?.code ? `${failure.code}: ${failure.message ?? ""}` : failure?.message;
  return message || "알 수 없는 오류가 발생했습니다.";
}

/**
 * The one error card of a slot: a different message replaces the previous card under a new ID
 * (`<kind>:<seq>`), the same message keeps the card (repeated polling or redraw failures stay
 * dismissed), and an empty message clears it.
 */
export function errorNotice(previous, kind, seq, message) {
  if (!message) return null;
  if (previous?.message === message) return previous;
  return { id: `${kind}:${seq}`, message, severity: "error" };
}

/** A run started with unpaired net labels warns once for that run; the run still goes ahead. */
export function startNotice(runId, labelWarnings) {
  if (!labelWarnings?.length) return null;
  const labels = labelWarnings.map(({ partId, text }) => `${partId} (${text})`).join(", ");
  return { id: `start:${runId}`, message: `짝 없는 네트 라벨 ${labelWarnings.length}개가 있는 채로 실행합니다: ${labels}`, severity: "warning" };
}

/** Cards of the editor: its error, then the shown run's start warning and failure (one each per run). */
export function editorNotices(error, run, start) {
  const failed = run?.state === "failed" && run.error
    ? { id: `fail:${run.runId}`, message: `실행 실패 — ${failureText(run.error)}`, severity: "error" } : null;
  return [error, run && start?.id === `start:${run.runId}` ? start : null, failed].filter(Boolean);
}
