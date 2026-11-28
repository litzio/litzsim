import React from "react";
import { formatSI, parseSI } from "./parts.js";
import {
  TRACE_COLORS, applyToAll, autoRange, axisFor, defaultRun, formatValue, keepSignals, keepView, niceStep, panRange,
  readTimeAxis, ticks, timeAxisProblem, zoomAxis, zoomRange,
} from "./plot.js";
import { errorNotice, failureText } from "./rules.js";

// PSIM-style waveform window. Every probe of a completed run is recorded; the user
// adds empty graphs and picks the signals of each. Graphs, axes, grids and cursors
// are screen state only and never written to the circuit. All data comes from the
// core: `wave` (per-pixel extremes of original rows) and `measure` (cursor rows and
// interval statistics). Canvases are drawn directly from input handlers and core
// replies, not through React renders, and a reply for an older view is dropped.
// The window stays mounted behind the circuit tab; while hidden it neither asks
// the core nor draws, and catches up when shown again.
const MARGIN = { left: 64, right: 64, top: 22, bottom: 20 };
const WHEEL = 0.0015;
// Grid lines are about this many CSS pixels apart when their step is automatic.
const TIME_SPACING = 110;
const LEVEL_SPACING = 28;
const AXES = ["left", "right"];
const AXIS_NAME = { left: "왼쪽", right: "오른쪽" };
const freshAxis = () => ({ auto: true, min: null, max: null, step: null });
const freshSettings = () => ({ major: true, minor: false, left: freshAxis(), right: freshAxis() });

const border = "var(--border-width) solid var(--id-color-border)";
const subtle = "var(--border-width) solid var(--id-color-border-subtle)";
const styles = {
  body: { position: "relative", display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: "var(--space-2)" },
  graphs: { flex: 1, minHeight: 0, overflow: "auto", display: "flex", flexDirection: "column", gap: "var(--space-2)" },
  graph: { flex: "1 1 0", minHeight: "calc(var(--layout-screen-fill-min-height) * 0.75)", display: "flex", flexDirection: "column", border, borderRadius: "var(--id-radius-md)", background: "var(--id-color-surface)", overflow: "hidden" },
  head: { display: "flex", flexWrap: "wrap", alignItems: "center", gap: "var(--space-2)", padding: "var(--space-1) var(--space-2)", borderBottom: subtle },
  title: { margin: 0, marginRight: "var(--space-2)" },
  chip: { display: "inline-flex", alignItems: "center", gap: "var(--space-1)", border: subtle, borderRadius: "var(--id-radius-sm)", paddingLeft: "var(--space-2)", fontSize: "var(--font-size-sm)" },
  swatch: { width: "var(--space-4)", height: "var(--space-1)", borderRadius: "var(--id-radius-sm)" },
  plot: { position: "relative", flex: 1, minHeight: 0 },
  canvas: { position: "absolute", inset: 0, width: "100%", height: "100%", display: "block", touchAction: "none", outline: "none", cursor: "crosshair", fontSize: "var(--font-size-xs)", fontFamily: "var(--font-family-sans)" },
  readout: { display: "flex", flexDirection: "column", gap: "var(--space-1)", fontSize: "var(--font-size-sm)" },
  facts: { display: "flex", flexWrap: "wrap", gap: "var(--space-4)", fontVariantNumeric: "tabular-nums" },
  tableWrap: { overflow: "auto", maxHeight: "var(--layout-reading-preview-height)", border, borderRadius: "var(--id-radius-md)" },
  table: { width: "100%", borderCollapse: "collapse", fontVariantNumeric: "tabular-nums" },
  th: { textAlign: "left", fontWeight: "var(--font-weight-semibold)", padding: "var(--space-1) var(--space-3)", borderBottom: subtle, whiteSpace: "nowrap" },
  thNumber: { textAlign: "right", fontWeight: "var(--font-weight-semibold)", padding: "var(--space-1) var(--space-3)", borderBottom: subtle, whiteSpace: "nowrap" },
  td: { textAlign: "right", padding: "var(--space-1) var(--space-3)", borderBottom: subtle, whiteSpace: "nowrap" },
  status: { display: "flex", alignItems: "center", justifyContent: "space-between", gap: "var(--space-4)", padding: "var(--space-1) var(--space-3)", border, borderRadius: "var(--id-radius-md)", fontSize: "var(--font-size-sm)", color: "var(--id-color-text-muted)", minHeight: "var(--size-control)" },
  dialog: { position: "absolute", top: 0, right: 0, zIndex: 3, width: "var(--layout-figure-popover-width)", maxWidth: "100%", maxHeight: "100%", overflow: "auto", display: "flex", flexDirection: "column", gap: "var(--space-3)", padding: "var(--space-4)", border, borderRadius: "var(--id-radius-md)", background: "var(--id-color-surface)", boxShadow: "var(--shadow-overlay)" },
  form: { display: "grid", gridTemplateColumns: "max-content 1fr", alignItems: "center", columnGap: "var(--space-4)", rowGap: "var(--space-2)", fontSize: "var(--font-size-sm)" },
  formLabel: { color: "var(--id-color-text-muted)" },
  formHeading: { gridColumn: "1 / -1", margin: 0, marginTop: "var(--space-2)", paddingTop: "var(--space-2)", borderTop: subtle },
  note: { margin: 0, fontSize: "var(--font-size-sm)", color: "var(--id-color-text-muted)" },
  inline: { display: "inline-flex", alignItems: "center", gap: "var(--space-1)", whiteSpace: "nowrap" },
  pair: { display: "flex", alignItems: "center", gap: "var(--space-2)" },
  actions: { display: "flex", flexWrap: "wrap", justifyContent: "flex-end", gap: "var(--space-2)" },
  invalid: { color: "var(--id-color-danger)", fontSize: "var(--font-size-sm)", margin: 0 },
  tools: { display: "flex", flexWrap: "wrap", alignItems: "center", gap: "var(--space-2)" },
};

const READOUT = [["signal", "신호"], ["a", "A 값"], ["b", "B 값"], ["delta", "B − A"], ["average", "A–B 평균"],
  ["rms", "A–B RMS"], ["minimum", "A–B 최소"], ["maximum", "A–B 최대"]];
const shownChannels = graphs => [...new Set(graphs.flatMap(graph => graph.signals.map(signal => signal.name)))];
const colorOf = (run, name) => TRACE_COLORS[Math.max(0, run?.channels.findIndex(channel => channel.name === name) ?? 0) % TRACE_COLORS.length];
const runTime = run => new Date(run.startedAt).toLocaleString("ko-KR", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" });

/** The plot rectangle inside a canvas, in CSS pixels. Hit tests and drawing share it. */
function plotBox(canvas) {
  const [w, h] = [canvas.clientWidth, canvas.clientHeight];
  return { w, h, x0: MARGIN.left, x1: Math.max(MARGIN.left + 1, w - MARGIN.right), y0: MARGIN.top, y1: Math.max(MARGIN.top + 1, h - MARGIN.bottom) };
}

const xOf = (box, [from, to], t) => box.x0 + ((t - from) / (to - from)) * (box.x1 - box.x0);

function timeAt(box, [from, to], x) {
  const fraction = Math.min(1, Math.max(0, (x - box.x0) / (box.x1 - box.x0)));
  return from + fraction * (to - from);
}

function palette(canvas) {
  const style = getComputedStyle(canvas);
  const token = name => style.getPropertyValue(name).trim();
  return {
    text: token("--id-color-text"), muted: token("--id-color-text-muted"), frame: token("--id-color-border"),
    grid: token("--id-color-border-subtle"), plot: token("--id-color-surface"), font: `${style.fontSize} ${style.fontFamily}`,
  };
}

/** Grid steps: the user's step unless it would draw too many lines, else 1-2-5 steps. */
function tickValues(low, high, step, pixels, spacing) {
  const auto = niceStep(high - low, Math.max(2, pixels / spacing));
  const chosen = step && ticks(low, high, step) ? step : auto;
  return { step: chosen, values: ticks(low, high, chosen) ?? [] };
}

/** One graph: grids, traces of the newest exact reply (projected while a new one is pending), axes and cursors. */
function drawGraph(canvas, graph, live) {
  const dpr = window.devicePixelRatio || 1;
  const box = plotBox(canvas);
  const [width, height] = [Math.round(box.w * dpr), Math.round(box.h * dpr)];
  if (canvas.width !== width || canvas.height !== height) [canvas.width, canvas.height] = [width, height];
  const ctx = canvas.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, box.w, box.h);
  const colors = palette(canvas);
  const { x0, x1, y0, y1 } = box;
  const view = live.view;
  const { settings } = graph;
  const data = name => live.exact?.series.get(name);

  const ranges = {};
  for (const axis of AXES) {
    const signals = graph.signals.filter(signal => signal.axis === axis);
    if (!signals.length) continue;
    const set = settings[axis];
    ranges[axis] = !set.auto && set.min < set.max ? [set.min, set.max]
      : autoRange(signals.map(signal => [data(signal.name)?.minimum ?? null, data(signal.name)?.maximum ?? null]));
  }
  live.ranges.set(graph.id, ranges);
  const yOf = ([low, high]) => value => y1 - ((value - low) / (high - low)) * (y1 - y0);
  const gridAxis = ranges.left ? "left" : ranges.right ? "right" : null;

  ctx.fillStyle = colors.plot;
  ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
  const time = tickValues(view[0], view[1], live.timeStep, x1 - x0, TIME_SPACING);
  const level = gridAxis ? tickValues(...ranges[gridAxis], settings[gridAxis].step, y1 - y0, LEVEL_SPACING) : null;
  const vertical = values => values.forEach(t => { const x = Math.round(xOf(box, view, t)) + 0.5; ctx.moveTo(x, y0); ctx.lineTo(x, y1); });
  const horizontal = values => values.forEach(v => { const y = Math.round(yOf(ranges[gridAxis])(v)) + 0.5; ctx.moveTo(x0, y); ctx.lineTo(x1, y); });
  ctx.lineWidth = 1;
  ctx.strokeStyle = colors.grid;
  if (settings.minor) {
    ctx.save();
    ctx.globalAlpha = 0.5;
    ctx.setLineDash([2, 3]);
    ctx.beginPath();
    vertical(ticks(view[0], view[1], time.step / 5) ?? []);
    if (level) horizontal(ticks(...ranges[gridAxis], level.step / 5) ?? []);
    ctx.stroke();
    ctx.restore();
  }
  if (settings.major) {
    ctx.beginPath();
    vertical(time.values);
    if (level) horizontal(level.values);
    ctx.stroke();
  }

  ctx.save();
  ctx.beginPath();
  ctx.rect(x0, y0, x1 - x0, y1 - y0);
  ctx.clip();
  ctx.lineWidth = 1.25;
  ctx.lineJoin = "round";
  for (const signal of graph.signals) {
    const series = data(signal.name);
    if (!series?.points.length) continue;
    const y = yOf(ranges[signal.axis]);
    ctx.strokeStyle = colorOf(live.run, signal.name);
    ctx.beginPath();
    series.points.forEach((point, index) => {
      const x = xOf(box, view, point.t_s);
      if (index) ctx.lineTo(x, y(point.value));
      else ctx.moveTo(x, y(point.value));
    });
    ctx.stroke();
  }
  ctx.restore();

  ctx.strokeStyle = colors.frame;
  ctx.strokeRect(x0 + 0.5, y0 + 0.5, x1 - x0 - 1, y1 - y0 - 1);
  ctx.font = colors.font;
  ctx.fillStyle = colors.muted;
  ctx.textAlign = "center";
  ctx.textBaseline = "top";
  for (const t of time.values) ctx.fillText(formatValue(t, "s", 4), xOf(box, view, t), y1 + 4);
  for (const axis of AXES) {
    if (!ranges[axis]) continue;
    const signals = graph.signals.filter(signal => signal.axis === axis);
    ctx.fillStyle = signals.length === 1 ? colorOf(live.run, signals[0].name) : colors.muted;
    ctx.textAlign = axis === "left" ? "right" : "left";
    const x = axis === "left" ? x0 - 6 : x1 + 6;
    ctx.textBaseline = "middle";
    const marks = tickValues(...ranges[axis], settings[axis].step, y1 - y0, LEVEL_SPACING).values;
    for (const value of marks) ctx.fillText(formatValue(value, "", 4), x, yOf(ranges[axis])(value));
    ctx.textBaseline = "top";
    ctx.fillText([...new Set(signals.map(signal => signal.unit))].join(", "), x, 2);
  }
  if (!graph.signals.length) {
    ctx.fillStyle = colors.muted;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText("위의 [신호 추가]에서 이 그래프에 볼 신호를 고르세요", (x0 + x1) / 2, (y0 + y1) / 2);
  }
  if (live.cursorsOn) {
    live.cursor.forEach((t, index) => {
      const x = Math.round(xOf(box, view, t)) + 0.5;
      if (x < x0 || x > x1) return;
      ctx.strokeStyle = colors.text;
      ctx.setLineDash(index ? [4, 3] : []);
      ctx.beginPath();
      ctx.moveTo(x, y0);
      ctx.lineTo(x, y1);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.fillStyle = colors.text;
      ctx.textAlign = "center";
      ctx.textBaseline = "bottom";
      ctx.fillText(index ? "B" : "A", x, y0 - 1);
    });
  }
}

function AxisSettings({ ui, index, graph, ranges, onApply, onApplyAll, onClose }) {
  const text = value => (value === null || value === undefined ? "" : formatValue(value, "", 6));
  const axisDraft = axis => {
    const set = graph.settings[axis];
    const shown = set.auto ? ranges?.[axis] : [set.min, set.max];
    return { auto: set.auto, min: text(shown?.[0]), max: text(shown?.[1]), stepAuto: set.step === null, step: text(set.step) };
  };
  const [draft, setDraft] = React.useState(() => ({
    major: graph.settings.major, minor: graph.settings.minor, left: axisDraft("left"), right: axisDraft("right"),
  }));
  const [problem, setProblem] = React.useState("");
  const set = patch => setDraft(current => ({ ...current, ...patch }));
  const setAxis = (axis, patch) => setDraft(current => ({ ...current, [axis]: { ...current[axis], ...patch } }));

  function settings() {
    const step = (auto, value, name) => {
      if (auto) return null;
      const parsed = parseSI(value);
      if (!(parsed > 0)) throw new Error(`${name} 간격은 0보다 큰 값이어야 합니다.`);
      return parsed;
    };
    const axis = side => {
      const value = draft[side];
      const result = { auto: value.auto, min: null, max: null, step: step(value.stepAuto, value.step, `${AXIS_NAME[side]} 축`) };
      if (!value.auto) {
        [result.min, result.max] = [parseSI(value.min), parseSI(value.max)];
        if (!(result.min < result.max)) throw new Error(`${AXIS_NAME[side]} 축의 최소는 최대보다 작은 값이어야 합니다.`);
      }
      return result;
    };
    return { major: draft.major, minor: draft.minor, left: axis("left"), right: axis("right") };
  }

  function submit(apply) {
    try {
      apply(settings());
    } catch (failure) {
      setProblem(failure.message);
    }
  }

  const check = (checked, onChange, label) => <label style={styles.inline}>
    <input type="checkbox" checked={checked} onChange={event => onChange(event.target.checked)} />{label}
  </label>;
  const field = (label, value, onChange, disabled, unit) => <label style={styles.inline}>
    <span>{label}</span>
    <ui.Input value={value} disabled={disabled} aria-label={`${label}${unit ? ` (${unit})` : ""}`} size={8} onChange={event => onChange(event.target.value)} />
    {unit && <span>{unit}</span>}
  </label>;
  const signals = axis => graph.signals.filter(signal => signal.axis === axis);

  return <div role="dialog" aria-label={`그래프 ${index + 1} 축·격자 설정`} style={styles.dialog}
    onKeyDown={event => { if (event.key === "Escape") { event.stopPropagation(); onClose(); } }}>
    <h3 style={{ margin: 0 }}>{`그래프 ${index + 1} 축·격자`}</h3>
    <div style={styles.form}>
      <span style={styles.formLabel}>격자</span>
      <span style={styles.pair}>
        {check(draft.major, major => set({ major }), "주 격자")}
        {check(draft.minor, minor => set({ minor }), "보조 격자 (주 격자의 1/5)")}
      </span>
      {AXES.map(axis => <React.Fragment key={axis}>
        <h4 style={styles.formHeading}>
          {`${AXIS_NAME[axis]} 축 · ${signals(axis).map(signal => signal.name).join(", ") || "신호 없음"}`}</h4>
        <span style={styles.formLabel}>범위</span>
        <span style={styles.pair}>
          {check(draft[axis].auto, auto => setAxis(axis, { auto }), "자동")}
          {field("최소", draft[axis].min, min => setAxis(axis, { min }), draft[axis].auto)}
          {field("최대", draft[axis].max, max => setAxis(axis, { max }), draft[axis].auto)}
        </span>
        <span style={styles.formLabel}>격자 간격</span>
        <span style={styles.pair}>
          {check(draft[axis].stepAuto, stepAuto => setAxis(axis, { stepAuto }), "자동")}
          {field("간격", draft[axis].step, step => setAxis(axis, { step }), draft[axis].stepAuto)}
        </span>
      </React.Fragment>)}
    </div>
    <p style={styles.note}>시간축은 모든 그래프가 함께 쓰며 위의 X축에서 정합니다. 범위 자동은 보이는 시간 구간의 최솟값·최댓값에 맞춥니다.
      [모든 그래프에 적용]은 격자·자동 선택을 모든 그래프에, 직접 입력한 축 범위와 간격은 같은 신호를 보이는 축에만 적용합니다.</p>
    {problem && <p role="alert" style={styles.invalid}>{problem}</p>}
    <div style={styles.actions}>
      <ui.Button variant="ghost" onClick={onClose}>취소</ui.Button>
      <ui.Button onClick={() => submit(onApplyAll)}>모든 그래프에 적용</ui.Button>
      <ui.Button onClick={() => submit(onApply)}>적용</ui.Button>
    </div>
  </div>;
}

/**
 * The shared time axis typed as numbers. It shows the live view (wheel, drag and double
 * click change the same range) except in fields being typed; Enter or [적용] applies all
 * three, and a refused field keeps its text and says why.
 */
function TimeAxis({ ui, live, full, step, onApply, onAuto }) {
  const [view, setShown] = React.useState(() => [...live.view]);
  const [draft, setDraft] = React.useState({});
  const [problems, setProblems] = React.useState({});
  React.useEffect(() => {
    live.onView = next => setShown([...next]);
    setShown([...live.view]);
    return () => { live.onView = null; };
  }, []);
  const values = { from: draft.from ?? view[0], to: draft.to ?? view[1], step: draft.step ?? step };
  const shown = value => (typeof value === "string" ? value : value === null ? "" : formatSI(value));
  const clear = () => { setDraft({}); setProblems({}); };

  function apply() {
    const result = readTimeAxis(values, full);
    if (result.problems) {
      setProblems(result.problems);
      return;
    }
    clear();
    onApply(result.range, result.step);
  }

  const problem = timeAxisProblem(problems);
  const field = (key, label, placeholder) => <label style={styles.inline}>
    <span>{label}</span>
    <ui.Input value={shown(values[key])} size={8} placeholder={placeholder} aria-label={`X축 ${label} (s)`}
      aria-invalid={problems[key] ? true : undefined} aria-describedby={problems[key] ? "litzsim-time-axis-problem" : undefined}
      onChange={event => {
        const text = event.target.value;
        setDraft(current => ({ ...current, [key]: text }));
        setProblems(current => ({ ...current, [key]: undefined }));
      }}
      onKeyDown={event => {
        if (event.key === "Enter") { event.preventDefault(); apply(); }
        if (event.key === "Escape") clear();
      }} />
    <span>s</span>
  </label>;
  return <div role="group" aria-label="X축(시간)" style={styles.tools}>
    <span style={styles.formLabel}>X축</span>
    {field("from", "시작")}
    {field("to", "끝")}
    {field("step", "눈금 간격", "자동")}
    <ui.Button onClick={apply}>적용</ui.Button>
    <ui.Button title="실행 전체 시간과 자동 눈금 간격으로 되돌립니다." onClick={() => { clear(); onAuto(); }}>자동</ui.Button>
    {problem && <p id="litzsim-time-axis-problem" role="alert" style={styles.invalid}>{problem}</p>}
  </div>;
}

function WaveformWindow({ ui, adapter, file, revision, completedRun, active }) {
  const { useState, useRef, useEffect, useLayoutEffect } = React;
  const [runs, setRuns] = useState(null);
  const [runId, setRunId] = useState(null);
  const [graphs, setGraphs] = useState([]);
  const [cursorsOn, setCursorsOn] = useState(false);
  const [readout, setReadout] = useState(null);
  const [editing, setEditing] = useState(null);
  const [timeStep, setTimeStep] = useState(null);
  // One error card in the window notification center; a different failure replaces it.
  const [error, setErrorNotice] = useState(null);
  const errorSeq = useRef(0);
  const rootRef = useRef(null);
  const areaRef = useRef(null);
  const canvases = useRef(new Map());
  const canvasRefs = useRef(new Map());
  const detach = useRef(new Map());
  const nextId = useRef(1);
  // Live drawing state, read by native input handlers between React renders.
  const live = useRef({
    view: [0, 1], full: [0, 1], exact: null, ranges: new Map(), cursor: [0, 0], seq: 0, frames: 0,
    waiting: false, measuring: false, measureAgain: false, measureSeq: 0, input: null, drag: null, width: 0, closed: false,
    stale: false, measureStale: false, runsKey: null, completed: null, onView: null,
  }).current;
  const run = runs?.find(item => item.runId === runId) ?? null;
  Object.assign(live, { runs, run, graphs, cursorsOn, timeStep, active });
  // The newest run this session completed; a run that starts later does not clear it.
  if (completedRun) live.completed = completedRun;
  const preferred = live.completed;
  const channelKey = shownChannels(graphs).join("\n");

  function setError(message) {
    const seq = ++errorSeq.current;
    setErrorNotice(previous => errorNotice(previous, "wave", seq, message));
  }
  const fail = failure => setError(failureText(failure));

  useEffect(() => {
    live.closed = false;
    return () => { live.closed = true; };
  }, []);

  // Runs of the circuit file (and older runs of unknown circuit) are read when the window
  // is shown for a new circuit file or a newly completed run, which then becomes the
  // shown run. Graphs keep their signals (see `runId`).
  useEffect(() => {
    if (!active || !file) return;
    const key = `${file}\n${preferred ?? ""}`;
    if (live.runsKey === key) return;
    live.runsKey = key;
    adapter.call("runs", { file }).then(response => {
      if (live.closed || live.runsKey !== key) return;
      setError("");
      setRuns(response.runs);
      setRunId(defaultRun(response.runs, preferred));
    }).catch(failure => {
      if (live.closed) return;
      live.runsKey = null;
      fail(failure);
    });
  }, [active, file, preferred]);

  /** Pinning and deleting are core operations; the window only shows their result. */
  function togglePin() {
    const current = live.run;
    adapter.call(current.pinned ? "unpin" : "pin", { file, runId: current.runId }).then(response => {
      if (live.closed) return;
      setError("");
      setRuns(list => list.map(item => (item.runId === response.runId ? { ...item, pinned: response.pinned } : item)));
    }).catch(failure => {
      if (!live.closed) fail(failure);
    });
  }

  async function deleteRun() {
    const current = live.run;
    try {
      const confirmed = await adapter.confirm({
        title: "실행 삭제",
        message: `${runTime(current)} 실행의 회로 스냅샷과 파형을 지웁니다. 되돌릴 수 없습니다.`,
        confirmLabel: "삭제",
      });
      if (!confirmed || live.closed) return;
      await adapter.call("delete", { file, runId: current.runId });
      if (live.closed) return;
      setError("");
      const remaining = live.runs.filter(item => item.runId !== current.runId);
      setRuns(remaining);
      setRunId(defaultRun(remaining, live.completed));
    } catch (failure) {
      if (!live.closed) fail(failure);
    }
  }

  function drawAll() {
    if (!live.active) return;
    for (const graph of live.graphs) {
      const canvas = canvases.current.get(graph.id);
      if (canvas?.clientWidth) drawGraph(canvas, graph, live);
    }
  }

  /** Ask the core for the newest view; one request at a time, older replies are dropped. */
  function fetchExact() {
    const current = live.run;
    const channels = shownChannels(live.graphs);
    const canvas = canvases.current.values().next().value;
    if (!current || !channels.length || !canvas?.clientWidth || live.closed) return;
    if (!live.active) {
      live.stale = true;
      return;
    }
    if (live.waiting) return;
    const seq = live.seq;
    const view = [...live.view];
    const box = plotBox(canvas);
    live.waiting = true;
    const sent = performance.now();
    adapter.call("wave", {
      file, runId: current.runId, channels, from_s: view[0], to_s: view[1], pixels: Math.max(1, Math.round(box.x1 - box.x0)),
    }).then(response => {
      if (seq !== live.seq || live.run?.runId !== current.runId || live.closed) return;
      const received = performance.now();
      live.exact = { view, pixels: Math.round(box.x1 - box.x0), series: new Map(response.channels.map(series => [series.name, series])) };
      drawAll();
      shown(seq, sent, received);
    }).catch(failure => { if (!live.closed) fail(failure); })
      .finally(() => {
        live.waiting = false;
        if (seq !== live.seq) fetchExact();
      });
  }

  /** Record the first frame that shows the exact reply for an input (stale replies never get here). */
  function shown(seq, sent, received) {
    const input = live.input;
    if (!input || input.seq !== seq) return;
    live.input = null;
    const drawn = performance.now();
    requestAnimationFrame(() => {
      const frame = performance.now();
      live.frames += 1;
      rootRef.current?.setAttribute("data-wave-frame", JSON.stringify({
        n: live.frames, kind: input.kind, totalMs: frame - input.at, queuedMs: sent - input.at,
        coreMs: received - sent, drawMs: drawn - received,
      }));
    });
  }

  /** Cursor values and statistics from the core for A (from) and B (to); a reply for older cursors is dropped. */
  function fetchMeasure() {
    const current = live.run;
    const channels = shownChannels(live.graphs);
    if (!current || !live.cursorsOn || live.closed) return;
    if (!live.active) {
      live.measureStale = true;
      return;
    }
    live.measureSeq += 1;
    if (!channels.length) {
      setReadout(null);
      return;
    }
    if (live.measuring) {
      live.measureAgain = true;
      return;
    }
    live.measuring = true;
    live.measureAgain = false;
    const seq = live.measureSeq;
    const cursor = [...live.cursor];
    adapter.call("measure", { file, runId: current.runId, channels, from_s: cursor[0], to_s: cursor[1] }).then(response => {
      if (seq === live.measureSeq && live.run?.runId === current.runId && !live.closed) setReadout({ cursor, ...response });
    }).catch(failure => { if (!live.closed) fail(failure); })
      .finally(() => {
        live.measuring = false;
        if (live.measureAgain) fetchMeasure();
      });
  }

  function setView(view, kind, at) {
    live.view = view;
    live.onView?.(view);
    live.seq += 1;
    live.input = { seq: live.seq, kind, at };
    drawAll();
    fetchExact();
  }

  function fullView(at, graphId) {
    setGraphs(list => list.map(graph => (graphId !== undefined && graph.id !== graphId ? graph
      : { ...graph, settings: { ...graph.settings, left: { ...graph.settings.left, auto: true }, right: { ...graph.settings.right, auto: true } } })));
    setView([...live.full], "full", at);
  }

  const minimumSpan = () => (live.run ? ((live.full[1] - live.full[0]) / Math.max(1, live.run.rows - 1)) * 4 : 0);

  function zoomTime(factor, at, event) {
    setView(zoomRange(live.view, factor, at, live.full, minimumSpan()), "wheel", event.timeStamp);
  }

  /** Shift+wheel: zoom the graph's vertical axes about the pointer, which makes them manual. */
  function zoomValues(graphId, factor, fraction) {
    const ranges = live.ranges.get(graphId) ?? {};
    setGraphs(list => list.map(graph => {
      if (graph.id !== graphId) return graph;
      const settings = { ...graph.settings };
      for (const axis of AXES) {
        if (!ranges[axis]) continue;
        const [min, max] = zoomAxis(ranges[axis], factor, fraction);
        settings[axis] = { ...settings[axis], auto: false, min, max };
      }
      return { ...graph, settings };
    }));
  }

  // Native listeners: the wheel must cancel page scrolling, and drags update the canvas without React.
  const handlers = {
    wheel(graphId, canvas, event) {
      event.preventDefault();
      if (!live.run) return;
      const box = plotBox(canvas);
      const rect = canvas.getBoundingClientRect();
      // Windows delivers Shift+wheel as horizontal scrolling.
      const delta = event.deltaY || event.deltaX;
      if (!delta) return;
      const factor = Math.exp(delta * WHEEL);
      if (event.shiftKey) {
        zoomValues(graphId, factor, (box.y1 - (event.clientY - rect.top)) / (box.y1 - box.y0));
        return;
      }
      zoomTime(factor, timeAt(box, live.view, event.clientX - rect.left), event);
    },
    pointerdown(graphId, canvas, event) {
      if (event.button !== 0 || !live.run) return;
      canvas.focus({ preventScroll: true });
      const box = plotBox(canvas);
      const x = event.clientX - canvas.getBoundingClientRect().left;
      const near = live.cursorsOn ? live.cursor.map((t, index) => [Math.abs(xOf(box, live.view, t) - x), index])
        .filter(([distance]) => distance <= 6).sort((a, b) => a[0] - b[0])[0] : null;
      live.drag = near ? { cursor: near[1] } : { x: event.clientX, view: [...live.view], width: box.x1 - box.x0 };
      canvas.setPointerCapture(event.pointerId);
      event.preventDefault();
    },
    pointermove(graphId, canvas, event) {
      const drag = live.drag;
      const box = plotBox(canvas);
      const x = event.clientX - canvas.getBoundingClientRect().left;
      if (!drag) {
        const onCursor = live.cursorsOn && live.cursor.some(t => Math.abs(xOf(box, live.view, t) - x) <= 6);
        canvas.style.cursor = onCursor ? "ew-resize" : "crosshair";
        return;
      }
      if (drag.cursor !== undefined) {
        live.cursor[drag.cursor] = timeAt(box, live.view, x);
        drawAll();
        fetchMeasure();
        return;
      }
      const shift = (-(event.clientX - drag.x) / drag.width) * (drag.view[1] - drag.view[0]);
      const next = panRange(drag.view, shift, live.full);
      if (next[0] !== live.view[0]) setView(next, "drag", event.timeStamp);
    },
    pointerup(graphId, canvas, event) {
      live.drag = null;
      if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    },
    pointercancel(graphId, canvas) {
      live.drag = null;
    },
    dblclick(graphId, canvas, event) {
      event.preventDefault();
      if (live.run) fullView(event.timeStamp, graphId);
    },
    keydown(graphId, canvas, event) {
      if (!live.run) return;
      const middle = (live.view[0] + live.view[1]) / 2;
      if (event.key === "Home") fullView(event.timeStamp);
      else if (event.key === "+" || event.key === "=") zoomTime(1 / 1.25, middle, event);
      else if (event.key === "-") zoomTime(1.25, middle, event);
      else return;
      event.preventDefault();
    },
  };
  const handlerRef = useRef(handlers);
  handlerRef.current = handlers;

  function canvasRef(graphId) {
    if (!canvasRefs.current.has(graphId)) {
      canvasRefs.current.set(graphId, canvas => {
        detach.current.get(graphId)?.();
        detach.current.delete(graphId);
        if (!canvas) {
          canvases.current.delete(graphId);
          return;
        }
        canvases.current.set(graphId, canvas);
        const off = ["wheel", "pointerdown", "pointermove", "pointerup", "pointercancel", "dblclick", "keydown"].map(type => {
          const listener = event => handlerRef.current[type](graphId, canvas, event);
          const options = type === "wheel" ? { passive: false } : undefined;
          canvas.addEventListener(type, listener, options);
          return () => canvas.removeEventListener(type, listener, options);
        });
        detach.current.set(graphId, () => off.forEach(remove => remove()));
      });
    }
    return canvasRefs.current.get(graphId);
  }

  // Coming back to the window draws what it has and asks for what it skipped while hidden.
  useLayoutEffect(() => {
    if (!active) return;
    drawAll();
    if (live.stale) {
      live.stale = false;
      fetchExact();
    }
    if (live.measureStale) {
      live.measureStale = false;
      fetchMeasure();
    }
  }, [active]);

  // Another run replaces only the data: graphs, settings, the time range and cursors stay
  // where the run still has rows, and signals it did not record leave their graphs.
  useLayoutEffect(() => {
    if (!run) return;
    const previous = live.full;
    live.full = [...run.timeRange];
    live.view = keepView(live.view, previous, live.full);
    live.onView?.(live.view);
    live.cursor = live.cursor.map(t => Math.min(Math.max(t, live.full[0]), live.full[1]));
    live.exact = null;
    live.seq += 1;
    setGraphs(list => keepSignals(list, run.channels));
    setReadout(null);
  }, [runId]);

  useLayoutEffect(() => {
    live.seq += 1;
    fetchExact();
  }, [runId, channelKey]);

  // Settings, cursors and graph changes redraw from the live state after each render.
  useLayoutEffect(() => drawAll());

  useEffect(() => {
    if (cursorsOn) fetchMeasure();
    else setReadout(null);
  }, [runId, channelKey, cursorsOn]);

  useEffect(() => {
    const area = areaRef.current;
    if (!area) return undefined;
    const observer = new ResizeObserver(() => {
      const canvas = canvases.current.values().next().value;
      const width = canvas ? Math.round(plotBox(canvas).x1 - plotBox(canvas).x0) : 0;
      drawAll();
      if (width && width !== live.width) {
        live.width = width;
        live.seq += 1;
        fetchExact();
      }
    });
    observer.observe(area);
    return () => observer.disconnect();
  }, [Boolean(run)]);

  // Verification hook: the exact reply the canvases draw, for comparison with original rows.
  useEffect(() => {
    if (rootRef.current) rootRef.current.litzsimWave = () => live.exact;
  });

  function addGraph() {
    const id = nextId.current++;
    setGraphs(list => [...list, { id, signals: [], settings: freshSettings() }]);
  }

  function removeGraph(graphId) {
    setGraphs(list => list.filter(graph => graph.id !== graphId));
    canvasRefs.current.delete(graphId);
    if (editing === graphId) setEditing(null);
  }

  function addSignal(graphId, name) {
    const unit = run.channels.find(channel => channel.name === name)?.unit ?? "";
    setGraphs(list => list.map(graph => (graph.id !== graphId || graph.signals.some(signal => signal.name === name) ? graph
      : { ...graph, signals: [...graph.signals, { name, unit, axis: axisFor(graph.signals, unit) }] })));
  }

  function changeSignal(graphId, name, change) {
    setGraphs(list => list.map(graph => (graph.id !== graphId ? graph
      : { ...graph, signals: graph.signals.flatMap(signal => (signal.name !== name ? [signal] : change(signal))) })));
  }

  function toggleCursors() {
    if (!cursorsOn) {
      const [from, to] = live.view;
      live.cursor = [from + (to - from) / 3, from + (2 * (to - from)) / 3];
    }
    setCursorsOn(on => !on);
  }

  function applyTimeAxis(range, step) {
    setTimeStep(step);
    setView(range, "axis", performance.now());
  }

  function autoTimeAxis() {
    setTimeStep(null);
    setView([...live.full], "axis", performance.now());
  }

  function applySettings(graphId, settings, all) {
    setGraphs(list => (all ? applyToAll(list, graphId, settings)
      : list.map(graph => (graph.id === graphId ? { ...graph, settings } : graph))));
    setEditing(null);
  }

  const completed = runs?.filter(item => item.state === "completed") ?? [];
  const runOptions = completed.map(item => ({
    value: item.runId,
    label: `${runTime(item)} · ${formatValue(item.timeRange[1] - item.timeRange[0], "s", 4)}${item.revision === revision ? " · 현재 회로" : ""}${item.circuit === null ? " · 이전 형식" : ""}${item.pinned ? " · 고정" : ""}`,
  }));
  const editingIndex = graphs.findIndex(graph => graph.id === editing);
  // The core sends A as `from`, B as `to` and their differences; the screen only formats them.
  const readoutRows = readout?.channels.map(series => {
    const value = number => formatValue(number, series.unit);
    return {
      signal: series.name, a: value(series.from.value), b: value(series.to.value), delta: value(series.delta),
      average: value(series.average), rms: value(series.rms), minimum: value(series.minimum), maximum: value(series.maximum),
    };
  }) ?? [];

  if (!runs || !run) {
    return <section aria-label="LitzSim 파형 창" style={styles.body}>
      {error ? <ui.Notice tone="error">{error.message}</ui.Notice>
        : !runs ? <ui.Progress label="실행 기록 읽기" />
          : <ui.EmptyState title="완료된 실행이 없습니다" description="[회로] 탭에서 [실행]을 누르고 완료되면 이 탭에서 파형을 볼 수 있습니다." />}
    </section>;
  }

  return <section ref={rootRef} aria-label="LitzSim 파형 창" style={styles.body}>
    <div style={styles.tools}>
      <ui.Select label="실행 기록" hideLabel value={runId} options={runOptions} onValueChange={setRunId} />
      <ui.Button aria-pressed={run.pinned} title="고정한 실행은 자동 정리에서 빠집니다. 고정하지 않은 실행은 회로마다 최근 10개만 남습니다."
        onClick={togglePin}>{run.pinned ? "고정 해제" : "고정"}</ui.Button>
      <ui.Button variant="destructive" title="이 실행의 폴더를 지웁니다." onClick={deleteRun}>삭제</ui.Button>
      {run.revision !== revision && <ui.Status tone="warning" label="현재 회로와 다른 실행" />}
      <span style={{ flex: 1 }} />
      <ui.Button aria-pressed={cursorsOn} onClick={toggleCursors}>{cursorsOn ? "커서 끄기" : "커서"}</ui.Button>
      <ui.Button onClick={() => fullView(performance.now())}>전체 보기</ui.Button>
      <ui.Button disabled title="수식으로 새 신호를 만드는 기능은 후속 단계에서 제공합니다.">수식 신호</ui.Button>
      <ui.Button variant="default" onClick={addGraph}>그래프 추가</ui.Button>
    </div>
    <TimeAxis ui={ui} live={live} full={run.timeRange} step={timeStep} onApply={applyTimeAxis} onAuto={autoTimeAxis} />
    <ui.Notifications items={error ? [error] : []} />
    <div ref={areaRef} style={styles.graphs}>
      {!graphs.length && <ui.EmptyState title="그래프를 추가하세요"
        description="[그래프 추가]로 빈 그래프를 만들고 그래프마다 볼 신호를 고릅니다. 한 그래프에 단위가 다른 두 신호를 넣으면 왼쪽·오른쪽 축을 나눠 씁니다." />}
      {graphs.map((graph, index) => {
        const available = run.channels.filter(channel => !graph.signals.some(signal => signal.name === channel.name))
          .map(channel => ({ value: channel.name, label: `${channel.name} (${channel.unit})` }));
        return <div key={graph.id} style={styles.graph} data-graph={index + 1}>
          <div style={styles.head}>
            <h3 style={styles.title}>{`그래프 ${index + 1}`}</h3>
            {graph.signals.map(signal => <span key={signal.name} style={styles.chip} data-signal={signal.name}>
              <span aria-hidden="true" style={{ ...styles.swatch, background: colorOf(run, signal.name) }} />
              <span>{signal.name}</span>
              <ui.Button size="sm" variant="ghost" title={`${AXIS_NAME[signal.axis === "left" ? "right" : "left"]} 축으로 옮기기`}
                aria-label={`${signal.name} ${AXIS_NAME[signal.axis === "left" ? "right" : "left"]} 축으로 옮기기`}
                onClick={() => changeSignal(graph.id, signal.name, item => [{ ...item, axis: item.axis === "left" ? "right" : "left" }])}>
                {signal.axis === "left" ? "좌" : "우"}</ui.Button>
              <ui.Button size="sm" variant="ghost" aria-label={`${signal.name} 빼기`} title="이 그래프에서 빼기"
                onClick={() => changeSignal(graph.id, signal.name, () => [])}>×</ui.Button>
            </span>)}
            {available.length > 0 && <ui.Select label={`그래프 ${index + 1}에 신호 추가`} hideLabel placeholder="신호 추가" value=""
              options={available} onValueChange={name => addSignal(graph.id, name)} />}
            <span style={{ flex: 1 }} />
            <ui.Button size="sm" variant="ghost" onClick={() => setEditing(graph.id)}>축·격자</ui.Button>
            <ui.Button size="sm" variant="ghost" aria-label={`그래프 ${index + 1} 삭제`} onClick={() => removeGraph(graph.id)}>삭제</ui.Button>
          </div>
          <div style={styles.plot}>
            <canvas ref={canvasRef(graph.id)} style={styles.canvas} tabIndex={0}
              aria-label={`그래프 ${index + 1}: ${graph.signals.map(signal => signal.name).join(", ") || "신호 없음"}`} />
          </div>
        </div>;
      })}
    </div>
    {cursorsOn && <div style={styles.readout} aria-live="polite">
      <div style={styles.facts}>
        {readout ? <>
          <span>{`A ${formatValue(readout.cursor[0], "s")}`}</span>
          <span>{`B ${formatValue(readout.cursor[1], "s")}`}</span>
          <span>{`Δt ${formatValue(readout.deltaTime_s, "s")}`}</span>
          <span>{`1/|Δt| ${readout.frequency_Hz === null ? "—" : formatValue(readout.frequency_Hz, "Hz")}`}</span>
        </> : <span>그래프에 신호를 추가하면 커서 값을 표시합니다.</span>}
      </div>
      {readoutRows.length > 0 && <div style={styles.tableWrap}>
        <table style={styles.table} aria-label="커서 값">
          <thead><tr>{READOUT.map(([key, label]) => <th key={key} scope="col" style={key === "signal" ? styles.th : styles.thNumber}>{label}</th>)}</tr></thead>
          <tbody>{readoutRows.map(row => <tr key={row.signal}>
            {READOUT.map(([key]) => (key === "signal"
              ? <th key={key} scope="row" style={styles.th}><span style={styles.inline}>
                <span aria-hidden="true" style={{ ...styles.swatch, background: colorOf(run, row.signal) }} />{row.signal}</span></th>
              : <td key={key} style={styles.td}>{row[key]}</td>))}
          </tr>)}</tbody>
        </table>
      </div>}
    </div>}
    {editingIndex >= 0 && <AxisSettings key={editing} ui={ui} index={editingIndex} graph={graphs[editingIndex]}
      ranges={live.ranges.get(editing)} onClose={() => setEditing(null)}
      onApply={settings => applySettings(editing, settings)} onApplyAll={settings => applySettings(editing, settings, true)} />}
    <footer style={styles.status}>
      <span>휠 시간축 확대·축소(모든 그래프) · Shift+휠 그 그래프 세로축 · 끌기 이동 · 더블클릭 전체 보기 · 커서 선은 끌어서 옮김</span>
      <span>{`${run.channels.length}개 신호 · 원본 ${run.rows.toLocaleString("ko-KR")}행`}</span>
    </footer>
  </section>;
}

// The editor re-renders on every hover and run poll; the window only needs its own props.
export const Waveform = React.memo(WaveformWindow);
