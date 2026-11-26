import React from "react";
import { CATEGORIES, PARTS, PARAMETERS, ROTATE_SHORTCUT, formatSI, parseSI, shortcutOf } from "./parts.js";
import { Schematic, sheetView, contentBounds } from "./schematic.jsx";
import { EMPTY_SELECTION, deleteSelection, isDirty, nearestPair, selectInBox, selectedSegmentIndex, selectionBox } from "./rules.js";
import { Waveform } from "./waveform.jsx";

// The editor owns only screen state: selection, drag previews, hover, the
// collapsed library and the undo snapshots it hands back to the core. Every
// circuit change is one core `apply`; every drawing is the core `layout`.
const GRID = 32;
const POLL_MS = 250;
const ZOOM = { min: 0.25, max: 4, fitMax: 1.25, step: 1.25, margin: 48 };
const ACTIVE = ["running", "stopping"];
const BY_KEY = Object.fromEntries(Object.values(PARTS).map(part => [part.key, part]));

const border = "var(--border-width) solid var(--id-color-border)";
const styles = {
  body: { display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: "var(--space-2)" },
  frame: { display: "flex", flex: 1, minHeight: 0, border, borderRadius: "var(--id-radius-md)", overflow: "hidden", background: "var(--id-color-surface)" },
  rail: { width: "var(--layout-activity-bar-width-icons)", flex: "none", display: "flex", flexDirection: "column", alignItems: "center", gap: "var(--space-1)", padding: "var(--space-2) 0", borderRight: border, background: "var(--id-color-surface-subtle)" },
  library: { width: "var(--layout-sidebar-width-narrow)", flex: "none", overflow: "auto", borderRight: border, padding: "var(--space-3)", display: "flex", flexDirection: "column", gap: "var(--space-3)" },
  canvas: { flex: 1, minWidth: 0, overflow: "auto", background: "var(--id-color-canvas)", outline: "none" },
  panel: { width: "var(--layout-sidebar-width)", flex: "none", overflow: "auto", borderLeft: border, padding: "var(--space-3)", display: "flex", flexDirection: "column", gap: "var(--space-4)" },
  status: { display: "flex", alignItems: "center", justifyContent: "space-between", gap: "var(--space-4)", padding: "var(--space-1) var(--space-3)", border, borderRadius: "var(--id-radius-md)", fontSize: "var(--font-size-sm)", color: "var(--id-color-text-muted)", minHeight: "var(--size-control)" },
  group: { display: "flex", flexDirection: "column", gap: "var(--space-1)" },
  groupTitle: { fontSize: "var(--font-size-xs)", fontWeight: "var(--font-weight-semibold)", color: "var(--id-color-text-muted)", margin: 0 },
  item: { display: "flex", alignItems: "center", justifyContent: "space-between", gap: "var(--space-2)", width: "100%", padding: "var(--space-1) var(--space-2)", border: "none", borderRadius: "var(--id-radius-sm)", background: "transparent", color: "var(--id-color-text)", font: "inherit", fontSize: "var(--font-size-sm)", cursor: "grab", touchAction: "none", textAlign: "left" },
  kbd: { fontFamily: "var(--font-family-mono)", fontSize: "var(--font-size-xs)", color: "var(--id-color-text-muted)", border: "var(--border-width) solid var(--id-color-border-subtle)", borderRadius: "var(--id-radius-sm)", padding: "0 var(--space-1)", whiteSpace: "nowrap" },
  railButton: { width: "var(--size-control)", height: "var(--size-control)", display: "grid", placeItems: "center", border: "none", borderRadius: "var(--id-radius-sm)", background: "transparent", color: "var(--id-color-text-muted)", cursor: "pointer" },
  field: { display: "flex", flexDirection: "column", gap: "var(--space-1)", fontSize: "var(--font-size-sm)" },
  fact: { display: "flex", flexWrap: "wrap", alignItems: "center", justifyContent: "space-between", gap: "var(--space-2)", fontSize: "var(--font-size-sm)" },
  hint: { position: "absolute", transform: "translate(-50%, -100%)", zIndex: 1 },
  stage: { position: "relative", flex: 1, minWidth: 0, display: "flex" },
  zoom: { position: "absolute", right: "var(--space-3)", bottom: "var(--space-3)", display: "flex", alignItems: "center", gap: "var(--space-1)", padding: "var(--space-1)", border, borderRadius: "var(--id-radius-md)", background: "var(--id-color-surface)", boxShadow: "var(--shadow-overlay)", fontSize: "var(--font-size-sm)", zIndex: 2 },
  zoomValue: { minWidth: "3.5em", textAlign: "center", fontVariantNumeric: "tabular-nums", color: "var(--id-color-text-muted)" },
};

const ICONS = {
  library: "M4 5h16v14H4zM9 5v14",
  sources: "M12 5a7 7 0 1 0 0 14a7 7 0 1 0 0-14M12 8v4M10 10h4M10 15h4",
  passive: "M2 12h4l2-4 3 8 3-8 3 8 2-4h3",
  switches: "M3 12h6M15 12h6M9 7v10l6-5zM15 7v10",
  control: "M3 16h4V8h5v8h5V8h4",
  meters: "M12 4a8 8 0 1 0 0 16a8 8 0 1 0 0-16M9 15l3-7 3 7M10 13h4",
};

const Icon = ({ name }) => <svg width={18} height={18} viewBox="0 0 24 24" fill="none" stroke="currentColor"
  strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={ICONS[name]} /></svg>;

const label = type => PARTS[type]?.label ?? type;
const formatTime = seconds => `${formatSI(seconds)} s`;
const gridOf = ([x, y]) => [Math.round(x / GRID), Math.round(y / GRID)];
const sheetOf = response => ({ document: response.document, revision: response.layout.revision, layout: response.layout });
const additive = event => event.button === 0 && (event.shiftKey || event.ctrlKey);
const toggle = (items, item) => items.includes(item) ? items.filter(value => value !== item) : [...items, item];

function intersects(a, b) {
  return a.min[0] < b.max[0] && b.min[0] < a.max[0] && a.min[1] < b.max[1] && b.min[1] < a.max[1];
}

/** A screen convenience for click placement: the nearest grid cell clear of symbols. */
function freeSpot(layout, near) {
  const boxes = [...layout.symbols.map(symbol => symbol.bounds), ...layout.labels.map(item => item.bounds)];
  for (let ring = 0; ring < 24; ++ring) {
    for (let dx = -ring; dx <= ring; ++dx) {
      for (let dy = -ring; dy <= ring; ++dy) {
        if (Math.max(Math.abs(dx), Math.abs(dy)) !== ring) continue;
        const at = [near[0] + 3 * dx, near[1] + 3 * dy];
        const box = { min: [at[0] * GRID - 48, at[1] * GRID - 48], max: [at[0] * GRID + 48, at[1] * GRID + 48] };
        if (!boxes.some(other => intersects(other, box))) return at;
      }
    }
  }
  return near;
}

function NumberField({ ui, owner, name, text, unit, value, placeholder, disabled, onCommit }) {
  const shown = value === undefined ? "" : formatSI(value);
  const [draft, setDraft] = React.useState(shown);
  const [invalid, setInvalid] = React.useState(false);
  React.useEffect(() => { setDraft(shown); setInvalid(false); }, [owner, value]);
  function commit() {
    if (draft === shown) return setInvalid(false);
    const next = parseSI(draft);
    if (!Number.isFinite(next)) return setInvalid(true);
    setInvalid(false);
    if (next !== value) onCommit(name, next);
  }
  return <label style={styles.field}>
    <span>{text}{unit && ` (${unit})`}</span>
    <ui.Input value={draft} disabled={disabled} aria-invalid={invalid || undefined} data-parameter={name}
      placeholder={placeholder} onChange={event => setDraft(event.target.value)} onBlur={commit}
      onKeyDown={event => {
        if (event.key === "Enter") { event.preventDefault(); commit(); }
        if (event.key === "Escape") setDraft(shown);
      }} />
    {invalid && <span style={{ color: "var(--id-color-danger)", fontSize: "var(--font-size-xs)" }}>
      숫자 또는 4.7k, 10u 같은 값을 입력하세요.</span>}
  </label>;
}

const SIMULATION = [
  ["timeStep_s", "해석 간격", "s"],
  ["stopTime_s", "종료 시각", "s"],
  ["outputStep_s", "저장 간격", "s", "모든 스텝"],
];

function Properties({ ui, sheet, selectedId, busy, edit }) {
  const part = selectedId && sheet.document.parts[selectedId];
  const pins = Object.entries(sheet.layout.pins);
  if (!part) {
    const missing = pins.filter(([, pin]) => pin.netId === null).map(([id]) => id);
    const { simulation } = sheet.document;
    return <>
      <h3 style={{ margin: 0 }}>회로</h3>
      <div style={styles.group}>
        <div style={styles.fact}><span>소자</span><span>{Object.keys(sheet.document.parts).length}</span></div>
        <div style={styles.fact}><span>네트</span><span>{Object.keys(sheet.document.nets).length}</span></div>
      </div>
      <div style={styles.group}>
        <h4 style={{ margin: 0 }}>해석 설정</h4>
        {SIMULATION.map(([name, text, unit, placeholder]) => <NumberField key={name} ui={ui} owner="simulation"
          name={name} text={text} unit={unit} placeholder={placeholder} value={simulation[name]} disabled={busy}
          onCommit={(key, value) => edit([{ op: "setSimulation", key, value }])} />)}
      </div>
      {missing.length > 0 && <div style={styles.group}>
        <h4 style={{ margin: 0 }}>미접속 단자 {missing.length}</h4>
        <span style={{ fontSize: "var(--font-size-sm)", color: "var(--id-color-warning)", wordBreak: "break-word" }}>{missing.join(", ")}</span>
      </div>}
      <p style={{ margin: 0, fontSize: "var(--font-size-sm)", color: "var(--id-color-text-muted)" }}>
        소자를 누르면 이름·값·단자 연결을 볼 수 있습니다.</p>
    </>;
  }
  const own = pins.filter(([, pin]) => pin.partId === selectedId);
  return <>
    <h3 style={{ margin: 0 }}>{selectedId}</h3>
    <div style={styles.group}>
      <div style={styles.fact}><span>종류</span><span>{label(part.type)}</span></div>
    </div>
    {Object.keys(part.parameters).length > 0 && <div style={styles.group}>
      <h4 style={{ margin: 0 }}>값</h4>
      {Object.entries(part.parameters).map(([name, value]) => <NumberField key={name} ui={ui} owner={selectedId}
        name={name} text={(PARAMETERS[name] ?? [name])[0]} unit={(PARAMETERS[name] ?? [name, ""])[1]} value={value}
        disabled={busy} onCommit={(key, next) => edit([{ op: "setParameter", id: selectedId, key, value: next }])} />)}
    </div>}
    <div style={styles.group}>
      <h4 style={{ margin: 0 }}>배치</h4>
      <div style={{ display: "flex", flexWrap: "wrap", gap: "var(--space-1)" }}>
        <ui.Button size="sm" disabled={busy} onClick={() => edit([{ op: "rotate", id: selectedId }])}>회전 {ROTATE_SHORTCUT}</ui.Button>
        <ui.Button size="sm" disabled={busy} onClick={() => edit([{ op: "mirror", id: selectedId, axis: "horizontal" }])}>좌우 반전 F</ui.Button>
        <ui.Button size="sm" disabled={busy} onClick={() => edit([{ op: "mirror", id: selectedId, axis: "vertical" }])}>상하 반전 Shift+F</ui.Button>
        <ui.Button size="sm" variant="destructive" disabled={busy} onClick={() => edit([{ op: "delete", id: selectedId }])}>삭제</ui.Button>
      </div>
    </div>
    <div style={styles.group}>
      <h4 style={{ margin: 0 }}>단자 연결</h4>
      {own.map(([id, pin]) => <div key={id} style={styles.fact} data-terminal={id}>
        <code>{id.slice(selectedId.length + 1)}</code>
        <span style={{ color: pin.netId ? "var(--id-color-text)" : "var(--id-color-warning)" }}>{pin.netId ?? "미접속"}</span>
        <ui.Button size="sm" disabled={busy || pin.netId === null}
          onClick={() => edit([{ op: "disconnect", pin: id }])}>연결 끊기</ui.Button>
      </div>)}
    </div>
  </>;
}

export function Editor({ ui, adapter }) {
  const { useState, useRef, useEffect, useLayoutEffect } = React;
  const [sheet, setSheet] = useState(null);
  const [file, setFile] = useState(null);
  const [disk, setDisk] = useState(null);
  const [history, setHistory] = useState([]);
  const [selected, setSelected] = useState(EMPTY_SELECTION);
  const selectedCount = selected.parts.length + selected.wires.length;
  const selectedId = selectedCount === 1 ? selected.parts[0] : undefined;
  const [marquee, setMarquee] = useState(null);
  const [showGrid, setShowGrid] = useState(true);
  const [pendingPin, setPendingPin] = useState(null);
  const [hoverSegment, setHoverSegment] = useState(null);
  const [preview, setPreview] = useState(null);
  const [ghost, setGhost] = useState(null);
  const [suggestion, setSuggestion] = useState(null);
  const [libraryOpen, setLibraryOpen] = useState(true);
  const [query, setQuery] = useState("");
  const [saving, setSaving] = useState(null);
  const [run, setRun] = useState(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  // View controls are screen state only.
  const [zoom, setZoom] = useState(1);
  const [canvasSize, setCanvasSize] = useState([0, 0]);
  const [, setAnchorTick] = useState(0);
  const [panning, setPanning] = useState(false);
  // The waveform window replaces the drawing; the circuit and its undo history stay here.
  const [screen, setScreen] = useState("editor");
  const svgRef = useRef(null);
  const anchor = useRef(null);
  const geometry = useRef(null);
  const shown = useRef(null);
  const fitPending = useRef(false);
  const pan = useRef(null);
  const space = useRef(false);
  const scrollRef = useRef(null);
  const queue = useRef(Promise.resolve());
  const waiting = useRef(0);
  // Queued work reads the newest document and undo snapshots, not a render's copy.
  const latest = useRef({ sheet: null, history: [] });
  const drag = useRef(null);
  const area = useRef(null);
  const libraryDrag = useRef(null);
  const pointer = useRef(null);
  const frame = useRef(null);
  const groups = useRef({});

  // Inputs run one at a time in arrival order; none is dropped while another waits.
  function perform(task) {
    waiting.current += 1;
    setBusy(true);
    const result = queue.current.then(async () => {
      setError("");
      try {
        return await task();
      } catch (failure) {
        setError(failure.code ? `${failure.code}: ${failure.message}` : failure.message);
        return undefined;
      } finally {
        if (--waiting.current === 0) setBusy(false);
        // Buttons disabled while busy drop focus; keep shortcuts on the drawing.
        requestAnimationFrame(() => {
          if (!document.activeElement || document.activeElement === document.body) scrollRef.current?.focus({ preventScroll: true });
        });
      }
    });
    queue.current = result;
    return result;
  }

  function commit(next, history) {
    latest.current = { sheet: next, history };
    setSheet(next);
    setHistory(history);
  }

  function reset(next, nextFile, nextDisk) {
    commit(next, []);
    setFile(nextFile);
    setDisk(nextDisk);
    setSelected(EMPTY_SELECTION);
    setPendingPin(null);
    setSuggestion(null);
    setHoverSegment(null);
    setSaving(null);
    fitPending.current = true;
    // A finished run belongs to the previous circuit; an active one stays stoppable.
    setRun(current => (current && ACTIVE.includes(current.state) ? current : null));
  }

  async function loadNew() {
    const created = await adapter.call("new");
    const drawn = await adapter.call("layout", { document: created.document });
    reset({ document: created.document, revision: created.revision, layout: drawn }, null,
      { revision: null, saved: created.revision });
  }

  useEffect(() => { perform(loadNew); }, []);

  const ready = Boolean(sheet);
  useEffect(() => {
    const pane = scrollRef.current;
    if (!pane) return undefined;
    const observer = new ResizeObserver(() => setCanvasSize([pane.clientWidth, pane.clientHeight]));
    observer.observe(pane);
    // Wheel zoom must cancel scrolling, which a passive React wheel handler cannot do.
    const onWheel = event => {
      event.preventDefault();
      zoomAt(Math.exp(-event.deltaY * 0.0015), event.clientX, event.clientY);
    };
    pane.addEventListener("wheel", onWheel, { passive: false });
    return () => { observer.disconnect(); pane.removeEventListener("wheel", onWheel); };
  }, [ready, screen]);

  // After a zoom change keep the anchored scene point under the same pane pixel.
  useLayoutEffect(() => {
    const pane = scrollRef.current;
    const current = geometry.current;
    if (!pane || !current || !canvasSize[0]) return;
    if (fitPending.current) {
      fitPending.current = false;
      fitAll();
      return;
    }
    const target = anchor.current;
    const last = shown.current;
    shown.current = { x: current.view.x, y: current.view.y, zoom: current.zoom };
    if (target) {
      anchor.current = null;
      pane.scrollLeft = (target.scene[0] - current.view.x) * current.zoom - target.at[0];
      pane.scrollTop = (target.scene[1] - current.view.y) * current.zoom - target.at[1];
    } else if (last && last.zoom === current.zoom) {
      // The padded sheet grew or the pane resized: keep the drawing where it was.
      pane.scrollLeft += (last.x - current.view.x) * current.zoom;
      pane.scrollTop += (last.y - current.view.y) * current.zoom;
    }
  });

  function zoomTo(next, scene, at) {
    anchor.current = { scene, at };
    setZoom(Math.min(ZOOM.max, Math.max(ZOOM.min, next)));
    setAnchorTick(tick => tick + 1);
  }

  /** Zoom by a factor, keeping the scene point under the client position fixed. */
  function zoomAt(factor, clientX, clientY) {
    const pane = scrollRef.current;
    const { view, zoom: current } = geometry.current;
    const box = pane.getBoundingClientRect();
    const at = [clientX - box.left, clientY - box.top];
    const scene = [view.x + (pane.scrollLeft + at[0]) / current, view.y + (pane.scrollTop + at[1]) / current];
    zoomTo(current * factor, scene, at);
  }

  function zoomCenter(factor) {
    const box = scrollRef.current.getBoundingClientRect();
    zoomAt(factor, box.left + box.width / 2, box.top + box.height / 2);
  }

  /** Show the whole drawing centered, no larger than a comfortable reading size. */
  function fitAll() {
    const pane = scrollRef.current;
    const bounds = contentBounds(latest.current.sheet.layout);
    const size = [pane.clientWidth, pane.clientHeight];
    if (!bounds) {
      zoomTo(1, [-2 * GRID, -2 * GRID], [0, 0]);
      return;
    }
    const extent = [0, 1].map(axis => Math.max(GRID, bounds.max[axis] - bounds.min[axis]));
    const fit = Math.min(...[0, 1].map(axis => (size[axis] - 2 * ZOOM.margin) / extent[axis]), ZOOM.fitMax);
    const center = [0, 1].map(axis => (bounds.min[axis] + bounds.max[axis]) / 2);
    zoomTo(fit, center, size.map(value => value / 2));
  }

  function startPan(event) {
    const pane = scrollRef.current;
    pan.current = { x: event.clientX, y: event.clientY, left: pane.scrollLeft, top: pane.scrollTop };
    svgRef.current.setPointerCapture(event.pointerId);
    setPanning(true);
  }

  // Record the first frame that shows a confirmed edit, from the input handler.
  useLayoutEffect(() => {
    const sample = frame.current;
    if (!sample) return undefined;
    frame.current = null;
    const id = requestAnimationFrame(() => {
      const shown = performance.now();
      requestAnimationFrame(() => {
        if (svgRef.current) svgRef.current.dataset.edit = JSON.stringify({
          kind: sample.kind, revision: sample.revision, totalMs: shown - sample.started, coreMs: sample.received - sample.started,
        });
      });
    });
    return () => cancelAnimationFrame(id);
  }, [sheet]);

  function keepSelection(next, createdId) {
    setSelected(current => createdId ? { ...EMPTY_SELECTION, parts: [createdId] } : {
      parts: current.parts.filter(id => next.document.parts[id]),
      wires: current.wires.map(wire => next.layout.segments[selectedSegmentIndex(next.layout, wire)]).filter(Boolean),
    });
  }

  // `review` sees the core result before it replaces the working sheet; false discards it.
  function edit(ops, { kind = ops[0].op, started = performance.now(), review } = {}) {
    return perform(async () => {
      const { sheet: before, history: undoable } = latest.current;
      const response = await adapter.call("apply", { document: before.document, expectedRevision: before.revision, ops });
      if (review && !(await review(response))) return undefined;
      const next = sheetOf(response);
      frame.current = { kind, started, received: performance.now(), revision: next.revision };
      commit(next, [...undoable, { previous: before.document, receipt: response.receipt }]);
      if (!Object.keys(before.document.parts).length && Object.keys(next.document.parts).length) fitPending.current = true;
      setHoverSegment(null);
      keepSelection(next, response.receipt.created[0]);
      return { ...next, created: response.receipt.created };
    });
  }

  function undo() {
    perform(async () => {
      const { sheet: current, history: undoable } = latest.current;
      const last = undoable.at(-1);
      if (!last) return;
      const response = await adapter.call("undo", {
        document: current.document, expectedAfter: last.receipt.afterRevision,
        expectedBefore: last.receipt.beforeRevision, previous: last.previous,
      });
      const next = sheetOf(response);
      commit(next, undoable.slice(0, -1));
      setSuggestion(null);
      setPendingPin(null);
      setHoverSegment(null);
      keepSelection(next);
    });
  }

  const hasParts = Boolean(sheet && Object.keys(sheet.document.parts).length);
  const dirty = Boolean(sheet && disk && isDirty(sheet.revision, disk.saved));
  const running = Boolean(run && ACTIVE.includes(run.state));

  async function discardChanges(title) {
    return !dirty || adapter.confirm({ title, message: "저장하지 않은 변경을 버립니다.", confirmLabel: "변경 버리기" });
  }

  function newCircuit() {
    perform(async () => {
      if (await discardChanges("새 회로를 만들까요?")) await loadNew();
    });
  }

  function openCircuit() {
    perform(async () => {
      if (!(await discardChanges("다른 회로를 열까요?"))) return;
      const path = await adapter.pickCircuit();
      if (!path) return;
      const opened = await adapter.call("inspect", { file: path });
      const drawn = await adapter.call("layout", { document: opened.document });
      reset({ document: opened.document, revision: opened.revision, layout: drawn }, path,
        { revision: opened.diskRevision, saved: opened.revision });
    });
  }

  function save() {
    if (!sheet) return;
    if (!file) {
      setSaving(current => current ?? { name: "circuit.litzsim" });
      return;
    }
    perform(async () => {
      const { sheet: current } = latest.current;
      const saved = await adapter.call("save", { file, document: current.document, expectedDiskRevision: disk.revision });
      setDisk({ revision: saved.diskRevision, saved: current.revision });
    });
  }

  function saveAs() {
    const name = saving.name.trim().replace(/(\.litzsim)?$/i, ".litzsim");
    perform(async () => {
      const folder = await adapter.pickFolder();
      if (!folder) return;
      const { sheet: current } = latest.current;
      const saved = await adapter.call("save", { folder, name, document: current.document });
      setFile(saved.file);
      setDisk({ revision: saved.diskRevision, saved: current.revision });
      setSaving(null);
    });
  }

  function startRun() {
    perform(async () => {
      const { sheet: current } = latest.current;
      setRun(await adapter.call("start", { file, document: current.document, expectedRevision: current.revision }));
    });
  }

  async function stopRun() {
    try {
      setRun(await adapter.call("stop", { runId: run.runId }));
    } catch (failure) {
      setError(failure.message);
    }
  }

  useEffect(() => {
    if (!running) return undefined;
    let live = true;
    let waiting = false;
    const timer = setInterval(async () => {
      if (waiting) return;
      waiting = true;
      try {
        const status = await adapter.call("status", { runId: run.runId });
        if (live) setRun(status);
      } catch (failure) {
        if (live) setError(failure.message);
      } finally {
        waiting = false;
      }
    }, POLL_MS);
    return () => { live = false; clearInterval(timer); };
  }, [run?.runId, running]);

  function sceneFromClient(x, y) {
    const svg = svgRef.current;
    const box = scrollRef.current?.getBoundingClientRect();
    if (!svg || !box || x < box.left || x > box.right || y < box.top || y > box.bottom) return null;
    const point = new DOMPoint(x, y).matrixTransform(svg.getScreenCTM().inverse());
    return [point.x, point.y];
  }

  function visibleCenter() {
    const box = scrollRef.current.getBoundingClientRect();
    return gridOf(sceneFromClient(box.left + box.width / 2, box.top + box.height / 2) ?? [0, 0]);
  }

  async function place(type, grid, started) {
    const at = grid ?? (pointer.current ? gridOf(pointer.current) : freeSpot(sheet.layout, visibleCenter()));
    setSuggestion(null);
    const next = await edit([{ op: "add", type, grid: at }], { kind: "add", started });
    if (next?.created.length) setSuggestion(nearestPair(next.layout, next.created[0]));
  }

  function insertAmmeter(index) {
    const segment = sheet.layout.segments[index];
    if (!segment) return;
    edit([{ op: "insertInBranch", type: "ammeter", terminals: segment.terminals, from: segment.from, to: segment.to }]);
  }

  /** Connect two terminals; confirm first if a voltage source would end up shorted. */
  /** Connect two terminals; if the core reports a newly shorted voltage source, ask first. */
  function connect(a, b, started) {
    return edit([{ op: "connect", a, b }], {
      kind: "connect",
      started,
      review: ({ receipt }) => !receipt.shortedSources.length || adapter.confirm({
        title: "전압원을 단락할까요?",
        message: `${receipt.shortedSources.join(", ")}의 두 단자가 같은 네트가 됩니다.`,
        confirmLabel: "그래도 연결",
      }),
    });
  }

  async function acceptSuggestion(started = performance.now()) {
    const { a, b } = suggestion;
    const partId = sheet.layout.pins[a].partId;
    setSuggestion(null);
    const next = await connect(a, b, started);
    if (next) setSuggestion(nearestPair(next.layout, partId));
  }

  function onPinDown(id, event) {
    if (space.current || event.button !== 0) return;
    event.stopPropagation();
    const started = performance.now();
    setSuggestion(null);
    if (pendingPin && pendingPin !== id) {
      setPendingPin(null);
      connect(pendingPin, id, started);
    } else {
      setPendingPin(pendingPin === id ? null : id);
    }
  }

  function onSymbolDown(id, event) {
    if (space.current || (event.button !== 0 && event.button !== 2)) return;
    event.stopPropagation();
    scrollRef.current.focus({ preventScroll: true });
    setPendingPin(null);
    setSuggestion(null);
    setHoverSegment(null);
    if (additive(event)) {
      setSelected(current => ({ ...current, parts: toggle(current.parts, id) }));
      return;
    }
    const keep = event.button === 0 && selected.parts.includes(id);
    const ids = keep ? selected.parts : [id];
    if (!keep) setSelected({ ...EMPTY_SELECTION, parts: ids });
    if (event.button !== 0 || busy) return;
    const start = sceneFromClient(event.clientX, event.clientY);
    if (!start) return;
    drag.current = { id, ids, start, revision: sheet.revision, moved: false, dx: 0, dy: 0 };
    svgRef.current.setPointerCapture(event.pointerId);
  }

  function onSegmentDown(index, event) {
    if (space.current || (event.button !== 0 && event.button !== 2)) return;
    event.stopPropagation();
    scrollRef.current.focus({ preventScroll: true });
    const wire = sheet.layout.segments[index];
    if (additive(event)) setSelected(current => ({ ...current, wires: toggle(current.wires, wire) }));
    else setSelected({ ...EMPTY_SELECTION, wires: [wire] });
    setPendingPin(null);
    setSuggestion(null);
    setHoverSegment(index);
  }

  function onBackgroundDown(event) {
    scrollRef.current.focus({ preventScroll: true });
    if (event.button === 1 || (event.button === 0 && space.current)) {
      event.preventDefault();
      startPan(event);
      setHoverSegment(null);
      return;
    }
    if (event.button !== 0 && event.button !== 2) return;
    setSelected(EMPTY_SELECTION);
    setPendingPin(null);
    setSuggestion(null);
    setHoverSegment(null);
    if (event.button === 0) {
      event.preventDefault();
      const start = sceneFromClient(event.clientX, event.clientY);
      if (!start) return;
      area.current = { start };
      svgRef.current.setPointerCapture(event.pointerId);
    }
  }

  function onPointerMove(event) {
    const moving = pan.current;
    if (moving) {
      scrollRef.current.scrollLeft = moving.left - (event.clientX - moving.x);
      scrollRef.current.scrollTop = moving.top - (event.clientY - moving.y);
      return;
    }
    const at = sceneFromClient(event.clientX, event.clientY);
    pointer.current = at;
    if (area.current) {
      if (!at || (!area.current.box && Math.hypot(at[0] - area.current.start[0], at[1] - area.current.start[1]) < 4)) return;
      const box = selectionBox(area.current.start, at);
      area.current.box = box;
      setMarquee(box);
      setSelected(selectInBox(sheet.layout, box));
      return;
    }
    if (event.target.dataset?.segment === undefined && !drag.current) setHoverSegment(null);
    const active = drag.current;
    if (!active || !at) return;
    const [dx, dy] = [0, 1].map(axis => Math.round((at[axis] - active.start[axis]) / GRID) * GRID);
    if (!active.moved && Math.hypot(at[0] - active.start[0], at[1] - active.start[1]) < 4) return;
    active.moved = true;
    if (dx === active.dx && dy === active.dy && preview) return;
    Object.assign(active, { dx, dy });
    setPreview({ ids: active.ids, dx, dy });
    if (active.ids.length === 1) setSuggestion(nearestPair(sheet.layout, active.id, [dx, dy]));
  }

  async function onPointerUp(event) {
    if (pan.current) {
      pan.current = null;
      if (!space.current) setPanning(false);
      if (svgRef.current.hasPointerCapture(event.pointerId)) svgRef.current.releasePointerCapture(event.pointerId);
      return;
    }
    if (area.current) {
      area.current = null;
      setMarquee(null);
      if (svgRef.current.hasPointerCapture(event.pointerId)) svgRef.current.releasePointerCapture(event.pointerId);
      return;
    }
    const active = drag.current;
    if (!active) return;
    const started = performance.now();
    drag.current = null;
    if (svgRef.current.hasPointerCapture(event.pointerId)) svgRef.current.releasePointerCapture(event.pointerId);
    setPreview(null);
    setSuggestion(null);
    if (active.revision !== sheet.revision) return;
    if (!active.moved) setSelected({ ...EMPTY_SELECTION, parts: [active.id] });
    if (!active.dx && !active.dy) return;
    const ops = active.ids.map(id => {
      const symbol = sheet.layout.symbols.find(item => item.id === id);
      return { op: "move", id, grid: gridOf([symbol.center[0] + active.dx, symbol.center[1] + active.dy]) };
    });
    const next = await edit(ops, { kind: "move", started });
    if (next && active.ids.length === 1) setSuggestion(nearestPair(next.layout, active.id));
  }

  function onPointerCancel() {
    pan.current = null;
    area.current = null;
    drag.current = null;
    setMarquee(null);
    setPreview(null);
    setSuggestion(null);
    setPanning(space.current);
  }

  function onLibraryDown(type, event) {
    if (event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    libraryDrag.current = { type, x: event.clientX, y: event.clientY, moved: false };
  }

  function onLibraryMove(event) {
    const active = libraryDrag.current;
    if (!active || (!active.moved && Math.hypot(event.clientX - active.x, event.clientY - active.y) < 5)) return;
    active.moved = true;
    const at = sceneFromClient(event.clientX, event.clientY);
    setGhost(at ? { at: gridOf(at).map(value => value * GRID), label: label(active.type) } : null);
  }

  function onLibraryUp(event) {
    const active = libraryDrag.current;
    libraryDrag.current = null;
    setGhost(null);
    if (!active) return;
    const started = performance.now();
    if (!active.moved) {
      place(active.type, freeSpot(sheet.layout, visibleCenter()), started);
      return;
    }
    const at = sceneFromClient(event.clientX, event.clientY);
    if (at) place(active.type, gridOf(at), started);
  }

  function onKeyDown(event) {
    if (!sheet || event.target.closest?.("input, textarea, select, [contenteditable='true'], [role='menu']")) return;
    const modifier = event.ctrlKey || event.metaKey;
    const key = event.key.toLowerCase();
    let handled = true;
    if (event.key === " ") {
      space.current = true;
      setPanning(true);
    } else if (modifier && !event.shiftKey && key === "z") undo();
    else if (modifier && key === "s") save();
    else if (event.ctrlKey && !event.shiftKey && !event.altKey && key === "'") setShowGrid(show => !show);
    else if (event.key === "Home" || (modifier && key === "0")) fitAll();
    else if (!modifier && (event.key === "+" || event.key === "=")) zoomCenter(ZOOM.step);
    else if (!modifier && event.key === "-") zoomCenter(1 / ZOOM.step);
    else if (event.key === "Escape") {
      onPointerCancel();
      setPendingPin(null);
      setHoverSegment(null);
      setSelected(EMPTY_SELECTION);
    } else if (event.key === "Enter" && suggestion) acceptSuggestion();
    else if ((event.key === "Delete" || event.key === "Backspace") && selectedCount) {
      if (!busy && !marquee) edit(deleteSelection(selected));
    } else {
      const shortcut = shortcutOf(event);
      if (shortcut === ROTATE_SHORTCUT && selectedId) edit([{ op: "rotate", id: selectedId }]);
      else if (shortcut === "F" && selectedId) edit([{ op: "mirror", id: selectedId, axis: "horizontal" }]);
      else if (shortcut === "Shift+F" && selectedId) edit([{ op: "mirror", id: selectedId, axis: "vertical" }]);
      else if (shortcut === "A" && hoverSegment !== null) insertAmmeter(hoverSegment);
      else if (BY_KEY[shortcut]) place(BY_KEY[shortcut].type);
      else handled = false;
    }
    if (handled) event.preventDefault();
  }

  function onKeyUp(event) {
    if (event.key !== " ") return;
    space.current = false;
    if (!pan.current) setPanning(false);
  }

  if (!sheet) {
    return <ui.ScreenLayout fillHeight description="회로를 준비하는 중입니다.">
      {error ? <ui.Notice tone="error">{error}</ui.Notice> : <ui.Progress label="회로 준비" />}
    </ui.ScreenLayout>;
  }

  if (screen === "wave") {
    return <Waveform ui={ui} adapter={adapter} file={file} revision={sheet.revision}
      preferredRun={run?.state === "completed" ? run.runId : null}
      onBack={() => { fitPending.current = true; setScreen("editor"); }} />;
  }

  const view = sheetView(sheet.layout, canvasSize.map(value => value / zoom));
  geometry.current = { view, zoom };
  const scaled = value => value * zoom;
  const count = code => sheet.layout.issues.filter(issue => issue.code === code).length;
  const issues = [count("overlap") && `겹침 ${count("overlap")}`, count("routeFailed") && `배선 실패 ${count("routeFailed")}`]
    .filter(Boolean).join(" · ");
  const selection = selectedId && sheet.document.parts[selectedId];
  const segment = hoverSegment !== null ? sheet.layout.segments[hoverSegment] : null;
  const wire = selectedCount === 1 ? selected.wires[0] : undefined;
  const selectedWires = new Set(selected.wires);
  const runReason = running ? "실행 중입니다. 중지한 뒤 다시 실행할 수 있습니다."
    : !file ? "새 회로는 저장한 뒤 실행할 수 있습니다."
      : !hasParts ? "회로가 비어 있습니다." : "";
  const fileName = file ? file.split(/[\\/]/).at(-1) : "새 회로";
  // `start` answers before the first progress report; times arrive with `status`.
  const timed = Boolean(run && Number.isFinite(run.simulatedTime) && run.stopTime > 0);
  const progress = timed ? Math.min(1, run.simulatedTime / run.stopTime) : 0;
  const runStatus = !run ? null
    : run.state === "running" ? <ui.Status tone="progress" label={`실행 중 ${Math.round(progress * 100)}%`} />
      : run.state === "stopping" ? <ui.Status tone="progress" label="중지하는 중" />
        : run.state === "completed" ? <ui.Status tone="ok" label="실행 완료" />
          : run.state === "canceled" ? <ui.Status tone="neutral" label="중지됨" />
            : <ui.Status tone="error" label="실행 실패" />;

  const query_ = query.trim().toLowerCase();
  const categories = CATEGORIES.map(group => ({
    ...group,
    parts: group.parts.filter(part => !query_ || [part.label, part.type, part.key, group.label]
      .some(text => text.toLowerCase().includes(query_))),
  })).filter(group => group.parts.length);

  const hint = marquee ? `영역 선택 · 소자 ${selected.parts.length} · 도선 ${selected.wires.length}`
    : suggestion && !preview ? `연결 추천: ${suggestion.a} ↔ ${suggestion.b} · Enter 연결 · Esc 취소`
    : preview ? (suggestion ? `놓으면 ${suggestion.a} ↔ ${suggestion.b} 연결을 추천합니다` : "놓을 위치로 끌어 옮기세요")
      : pendingPin ? `${pendingPin}에서 연결할 단자를 누르세요 · Esc 취소`
        : wire ? `도선 선택 · ${wire.terminals.join(" – ")} · Delete / Backspace / 우클릭으로 삭제`
          : selectedCount > 1 ? `소자 ${selected.parts.length} · 도선 ${selected.wires.length} 선택 · Delete 삭제 · Esc 해제`
            : segment ? `가지 ${segment.terminals.join(" – ")} · A 이 가지에 전류계 끼우기`
              : selection ? `${selectedId} 선택 · ${ROTATE_SHORTCUT} 회전 · F 좌우 반전 · Shift+F 상하 반전 · Delete 삭제 · 끌어서 이동`
                : "빈 곳 끌기 영역 선택 · 가운데 버튼 또는 Space+끌기 이동 · R 저항 · Ctrl+' 격자 보기 · Ctrl+Z 되돌리기";

  const menu = [
    { id: "rotate", label: `회전 (${ROTATE_SHORTCUT})`, icon: null, visible: Boolean(selection), disabled: busy, run: () => edit([{ op: "rotate", id: selectedId }]) },
    { id: "mirror-h", label: "좌우 반전 (F)", icon: null, visible: Boolean(selection), disabled: busy, run: () => edit([{ op: "mirror", id: selectedId, axis: "horizontal" }]) },
    { id: "mirror-v", label: "상하 반전 (Shift+F)", icon: null, visible: Boolean(selection), disabled: busy, run: () => edit([{ op: "mirror", id: selectedId, axis: "vertical" }]) },
    { id: "insert", label: "이 가지에 전류계 끼우기 (A)", icon: null, visible: Boolean(segment), run: () => insertAmmeter(hoverSegment) },
    { id: "delete", label: wire ? "도선 삭제" : "삭제 (Delete)", icon: null, danger: true, visible: selectedCount > 0, disabled: busy, run: () => edit(deleteSelection(selected)) },
    { id: "grid", label: "격자 보기 (Ctrl+')", icon: <span aria-label={showGrid ? "켜짐" : "꺼짐"}>{showGrid ? "✓" : "□"}</span>, visible: !selectedCount, run: () => setShowGrid(show => !show) },
  ];

  return <ui.ScreenLayout fillHeight
    description={`${fileName}${dirty ? " · 저장하지 않은 변경" : ""}`}
    status={runStatus}
    actions={<>
      <ui.Button disabled={busy} onClick={newCircuit}>새 회로</ui.Button>
      <ui.Button disabled={busy} onClick={openCircuit}>열기</ui.Button>
      <ui.Button disabled={busy} onClick={save}>저장</ui.Button>
      <ui.Button disabled={busy || !history.length} onClick={undo}>되돌리기</ui.Button>
      <ui.Button variant="default" disabled={busy || Boolean(runReason)} title={runReason || undefined}
        aria-describedby={runReason ? "litzsim-run-reason" : undefined} onClick={startRun}>실행</ui.Button>
      <ui.Button disabled={!running || run.state === "stopping"} onClick={stopRun}>중지</ui.Button>
      <ui.Button disabled={!file} title={file ? "이 회로 폴더의 완료된 실행 파형을 봅니다." : "저장한 회로의 실행 결과를 볼 수 있습니다."}
        onClick={() => setScreen("wave")}>파형 보기</ui.Button>
    </>}>
    <section aria-label="LitzSim 회로 편집기" style={styles.body} onKeyDown={onKeyDown} onKeyUp={onKeyUp}>
      {error && <ui.Notice tone="error">{error}</ui.Notice>}
      {run?.state === "failed" && run.error && <ui.Notice tone="error">{`실행 실패 — ${run.error.message ?? run.error.code}`}</ui.Notice>}
      {saving && <ui.Toolbar label="새 회로 저장">
        <label style={styles.field}>
          <span>파일 이름</span>
          <ui.Input value={saving.name} autoFocus onChange={event => setSaving({ name: event.target.value })}
            onKeyDown={event => { if (event.key === "Enter") saveAs(); if (event.key === "Escape") setSaving(null); }} />
        </label>
        <ui.Button disabled={busy || !saving.name.trim()} onClick={saveAs}>폴더를 고르고 저장</ui.Button>
        <ui.Button variant="ghost" onClick={() => setSaving(null)}>취소</ui.Button>
      </ui.Toolbar>}
      <div style={styles.frame}>
        <nav aria-label="부품 분류" style={styles.rail}>
          <button type="button" style={styles.railButton} aria-pressed={libraryOpen} aria-label={libraryOpen ? "부품 라이브러리 접기" : "부품 라이브러리 펼치기"}
            title={libraryOpen ? "부품 라이브러리 접기" : "부품 라이브러리 펼치기"} onClick={() => setLibraryOpen(open => !open)}>
            <Icon name="library" />
          </button>
          {CATEGORIES.map(group => <button key={group.id} type="button" style={styles.railButton} aria-label={group.label} title={group.label}
            onClick={() => {
              setLibraryOpen(true);
              setQuery("");
              requestAnimationFrame(() => groups.current[group.id]?.scrollIntoView({ block: "start" }));
            }}><Icon name={group.id} /></button>)}
        </nav>
        {libraryOpen && <aside aria-label="부품 라이브러리" style={styles.library}>
          <ui.Input value={query} placeholder="부품 검색" aria-label="부품 검색" onChange={event => setQuery(event.target.value)} />
          {categories.map(group => <div key={group.id} ref={node => { groups.current[group.id] = node; }} style={styles.group}>
            <h4 style={styles.groupTitle}>{group.label}</h4>
            {group.parts.map(part => <button key={part.type} type="button" style={styles.item} data-library={part.type}
              title={`${part.label} — 누르면 빈 자리에, 끌면 놓은 자리에 배치`}
              onPointerDown={event => onLibraryDown(part.type, event)} onPointerMove={onLibraryMove} onPointerUp={onLibraryUp}
              onClick={event => { if (event.detail === 0) place(part.type, freeSpot(sheet.layout, visibleCenter())); }}>
              <span>{part.label}</span><kbd style={styles.kbd}>{part.key}</kbd>
            </button>)}
          </div>)}
          {!categories.length && <span style={{ fontSize: "var(--font-size-sm)", color: "var(--id-color-text-muted)" }}>맞는 부품이 없습니다.</span>}
        </aside>}
        <div style={styles.stage}>
        <ui.ContextMenu actions={menu} trigger={
          <div ref={scrollRef} tabIndex={0} role="application" aria-label="회로도 편집 영역"
            style={{ ...styles.canvas, cursor: panning ? "grab" : undefined }}
            onPointerLeave={() => { pointer.current = null; setHoverSegment(null); }}>
            <div style={{ position: "relative", width: scaled(view.width), height: scaled(view.height) }}>
              <Schematic layout={sheet.layout} view={view} zoom={zoom} svgRef={svgRef} selectedIds={selected.parts} pendingPin={pendingPin}
                hoverSegment={hoverSegment} selectedWires={selectedWires} showGrid={showGrid} marquee={marquee}
                preview={preview} ghost={ghost} suggestion={suggestion}
                onSegmentDown={onSegmentDown}
                onSymbolDown={onSymbolDown} onPinDown={onPinDown} onSegmentEnter={index => {
                  if (!drag.current && !area.current && !pan.current) setHoverSegment(index);
                }}
                onBackgroundDown={onBackgroundDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerCancel} />
              {segment && !preview && <div style={{
                ...styles.hint,
                left: scaled((segment.from[0] + segment.to[0]) / 2 - view.x),
                top: scaled((segment.from[1] + segment.to[1]) / 2 - view.y) - 10,
              }}>
                <ui.Button size="sm" disabled={busy} onClick={() => insertAmmeter(hoverSegment)}>이 가지에 전류계 끼우기 (A)</ui.Button>
              </div>}
              {suggestion && !preview && <div style={{ ...styles.hint, transform: "none", left: scaled(suggestion.to[0] - view.x) + 12, top: scaled(suggestion.to[1] - view.y) + 12 }}>
                <ui.Button size="sm" disabled={busy} onClick={() => acceptSuggestion()}>{`${suggestion.a} ↔ ${suggestion.b} 연결 (Enter)`}</ui.Button>
              </div>}
            </div>
          </div>} />
          <div role="toolbar" aria-label="확대" style={styles.zoom}>
            <ui.Button size="sm" variant="ghost" aria-label="축소" title="축소 (-)" onClick={() => zoomCenter(1 / ZOOM.step)}>−</ui.Button>
            <span style={styles.zoomValue} data-zoom={zoom}>{`${Math.round(zoom * 100)}%`}</span>
            <ui.Button size="sm" variant="ghost" aria-label="확대" title="확대 (+)" onClick={() => zoomCenter(ZOOM.step)}>+</ui.Button>
            <ui.Button size="sm" variant="ghost" title="전체 보기 (Home, Ctrl+0)" onClick={fitAll}>전체 보기</ui.Button>
          </div>
        </div>
        <aside aria-label="속성" style={styles.panel}>
          <Properties ui={ui} sheet={sheet} selectedId={selectedId} busy={busy} edit={edit} />
        </aside>
      </div>
      <footer style={styles.status} aria-live="polite">
        <span data-hint="">{hint}</span>
        <span style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
          {issues && <span style={{ color: "var(--id-color-warning)" }}>{issues}</span>}
          {running && <span style={{ width: "8rem" }}><ui.Progress value={timed ? progress : undefined} label="해석 진행" /></span>}
          {running && timed && <span>{`${formatTime(run.simulatedTime)} / ${formatTime(run.stopTime)}`}</span>}
          {runReason && <span id="litzsim-run-reason">{runReason}</span>}
        </span>
      </footer>
    </section>
  </ui.ScreenLayout>;
}
