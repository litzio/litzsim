// Run after `cargo build --release`: node --test plugin
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { startRequest } from "./host.mjs";
import { applyToAll, autoRange, axisFor, defaultRun, formatValue, niceStep, panRange, ticks, zoomAxis, zoomRange } from "./plot.js";
import { CATEGORIES, PARTS, ROTATE_SHORTCUT, shortcutOf } from "./parts.js";
import { deleteSelection, isDirty, nearestPair, selectInBox, selectedSegmentIndex, selectionBox } from "./rules.js";

const root = fileURLToPath(new URL("..", import.meta.url));
const executable = ["litzsim.exe", "litzsim"].map(name => `${root}target/release/${name}`).find(existsSync);
let core;
let lines;

before(() => {
  assert.ok(executable, "build the core first: cargo build --release");
  core = spawn(executable, ["serve"], { stdio: ["pipe", "pipe", "inherit"] });
  lines = createInterface({ input: core.stdout })[Symbol.asyncIterator]();
});
after(() => core.stdin.end());

async function call(op, payload = {}) {
  core.stdin.write(JSON.stringify({ ...payload, op }) + "\n");
  const response = JSON.parse((await lines.next()).value);
  assert.equal(response.ok, true, JSON.stringify(response.error));
  return response;
}

async function apply(sheet, ops) {
  const response = await call("apply", { document: sheet.document, expectedRevision: sheet.revision, ops });
  return { document: response.document, revision: response.layout.revision, layout: response.layout };
}

const rc = async () => {
  const opened = await call("inspect", { file: `${root}examples/rc.litzsim` });
  return { document: opened.document, revision: opened.revision };
};

test("selecting one wire sends the core terminals and endpoints, even if segment indices change", async () => {
  const sheet = await apply(await rc(), []);
  const index = sheet.layout.segments.findIndex(wire => wire.netId === "input");
  const wire = sheet.layout.segments[index];
  const reversed = { segments: [...sheet.layout.segments].reverse() };
  assert.equal(selectedSegmentIndex(reversed, wire), reversed.segments.length - 1 - index);
  const ops = deleteSelection({ parts: [], wires: [wire] });
  assert.deepEqual(ops, [{ op: "removeBranch", terminals: wire.terminals, from: wire.from, to: wire.to }]);
  const next = await apply(sheet, ops);
  assert.equal(selectedSegmentIndex(next.layout, wire), -1);
  assert.deepEqual([next.layout.pins["V1.p"].netId, next.layout.pins["R1.p"].netId], [null, null]);
});

test("area selection collects intersecting parts and wires and deletes them in one batch", async () => {
  const sheet = await apply(await rc(), []);
  const selected = selectInBox(sheet.layout, selectionBox([162, 58], [85, 6]));
  assert.deepEqual(selected.parts, ["R1"]);
  assert.deepEqual(selected.wires.map(wire => wire.netId), ["input", "output"]);
  const ops = deleteSelection(selected);
  assert.deepEqual(ops.map(op => op.op), ["removeBranch", "removeBranch", "delete"]);
  const next = await apply(sheet, ops);
  assert.equal(next.document.parts.R1, undefined);
  const undone = await call("undo", {
    document: next.document, expectedAfter: next.revision, expectedBefore: sheet.revision, previous: sheet.document,
  });
  assert.equal(undone.layout.revision, sheet.revision);
  assert.deepEqual(undone.document, sheet.document);
});

test("R places a resistor and Shift+R rotates the selection", () => {
  assert.equal(shortcutOf({ key: "r" }), PARTS.resistor.key);
  assert.equal(PARTS.resistor.key, "R");
  assert.equal(shortcutOf({ key: "R", shiftKey: true }), ROTATE_SHORTCUT);
  assert.equal(ROTATE_SHORTCUT, "Shift+R");
});

test("the measurement library offers the node voltage probe on N", () => {
  const meters = CATEGORIES.find(group => group.id === "meters");
  assert.equal(meters.label, "측정");
  assert.ok(meters.parts.includes(PARTS.voltageProbe));
  assert.equal(PARTS.voltageProbe.label, "노드 전압계");
  assert.equal(PARTS.voltageProbe.key, "N");
  assert.equal(shortcutOf({ key: "n" }), PARTS.voltageProbe.key);
  assert.deepEqual(Object.values(PARTS).filter(part => part.key === "N"), [PARTS.voltageProbe]);
});

test("a new circuit with only a changed simulation setting is unsaved", async () => {
  const created = await call("new");
  const blank = { document: created.document, revision: created.revision };
  assert.equal(isDirty(blank.revision, created.revision), false);
  const timed = await apply(blank, [{ op: "setSimulation", key: "stopTime_s", value: 0.002 }]);
  assert.equal(isDirty(timed.revision, created.revision), true);
  const undone = await call("undo", {
    document: timed.document, expectedAfter: timed.revision, expectedBefore: blank.revision, previous: blank.document,
  });
  assert.equal(isDirty(undone.layout.revision, created.revision), false);
});

test("suggestions never merge two connected nets (review repro: R1 dragged to [1,4])", async () => {
  const moved = await apply(await rc(), [{ op: "move", id: "R1", grid: [1, 4] }]);
  const { pins } = moved.layout;
  // R1.p (input) lands one grid below V1.n (ground): the old rule offered this pair.
  assert.equal(Math.hypot(pins["R1.p"].at[0] - pins["V1.n"].at[0], pins["R1.p"].at[1] - pins["V1.n"].at[1]), 32);
  assert.deepEqual([pins["R1.p"].netId, pins["V1.n"].netId], ["input", "ground"]);
  assert.equal(nearestPair(moved.layout, "R1"), null);

  // A free terminal dropped onto a connected one is still suggested.
  const added = await apply(await rc(), [{ op: "add", type: "resistor", grid: [9, 1] }]);
  const suggestion = nearestPair(added.layout, "R2");
  assert.deepEqual([suggestion.a, suggestion.b, suggestion.distance], ["R2.p", "C1.p", 0]);
});

test("the core reports a shorted voltage source before the screen keeps the connection", async () => {
  const sheet = await rc();
  const connect = (a, b) => call("apply", {
    document: sheet.document, expectedRevision: sheet.revision, ops: [{ op: "connect", a, b }],
  });
  // The screen asks with these IDs and keeps `sheet` unless the user confirms.
  assert.deepEqual((await connect("R1.p", "V1.n")).receipt.shortedSources, ["V1"]);
  assert.deepEqual((await connect("V1.p", "V1.n")).receipt.shortedSources, ["V1"]);
  assert.deepEqual((await connect("R1.p", "C1.p")).receipt.shortedSources, []);
  assert.deepEqual((await connect("C1.n", "GND.ref")).receipt.shortedSources, []);
});

test("a run of a circuit at a drive root starts in that root, not a drive-relative path", () => {
  const drive = path.parse(process.cwd()).root;
  const payload = { document: {}, expectedRevision: "r" };
  assert.deepEqual(startRequest({ ...payload, file: path.join(drive, "rc.litzsim") }), { ...payload, directory: drive });
  const folder = path.join(drive, "work", "circuits");
  assert.equal(startRequest({ ...payload, file: path.join(folder, "rc.litzsim") }).directory, folder);
  assert.deepEqual(startRequest({ ...payload, directory: "." }), { ...payload, directory: "." });
});

test("waveform grids use 1-2-5 steps, and a custom step that would flood the plot is refused", () => {
  assert.equal(niceStep(1e-3, 10), 1e-4);
  assert.equal(niceStep(7, 4), 2);
  assert.equal(niceStep(0.9, 2), 0.5);
  assert.deepEqual(ticks(0, 1e-3, 2.5e-4), [0, 2.5e-4, 5e-4, 7.5e-4, 1e-3]);
  assert.deepEqual(ticks(-0.3, 0.3, 0.2), [-0.2, 0, 0.2]);
  assert.equal(ticks(0, 1, 1e-6), null);
  assert.equal(formatValue(2e-4, "s"), "200 µs");
  assert.equal(formatValue(0, "V"), "0 V");
  assert.equal(formatValue(-1.25e-3), "-1.25m");
});

test("auto ranges pad the visible extremes; Shift+wheel scales about the pointer value", () => {
  assert.deepEqual(autoRange([[0, 10], [null, null], [-10, 5]]), [-11, 11]);
  assert.deepEqual(autoRange([[5, 5]]), [4.5, 5.5]);
  assert.deepEqual(autoRange([]), [-1, 1]);
  assert.deepEqual(zoomAxis([0, 10], 0.5, 0.2), [1, 6]);
});

test("a second unit goes to the right axis; the same unit stays left", () => {
  const signals = [];
  signals.push({ name: "VC_V", unit: "V", axis: axisFor(signals, "V") });
  signals.push({ name: "VM1_V", unit: "V", axis: axisFor(signals, "V") });
  signals.push({ name: "AM1_A", unit: "A", axis: axisFor(signals, "A") });
  assert.deepEqual(signals.map(signal => signal.axis), ["left", "left", "right"]);
  assert.equal(axisFor([{ unit: "A", axis: "right" }], "V"), "left");
});

test("applying settings to all graphs copies grids and automatic choices, and typed ranges only to the same signals", () => {
  const axis = (auto, min = null, max = null, step = null) => ({ auto, min, max, step });
  const manual = { major: true, minor: false, timeStep: null, left: axis(false, 0, 12, 2), right: axis(false, -1, 1, 0.5) };
  const graphs = [
    { id: 1, signals: [{ name: "VC_V", unit: "V", axis: "left" }], settings: manual },
    { id: 2, signals: [{ name: "VC_V", unit: "V", axis: "left" }, { name: "AM1_A", unit: "A", axis: "right" }], settings: manual },
    { id: 3, signals: [{ name: "VM1_V", unit: "V", axis: "left" }], settings: manual },
  ];
  const chosen = { major: false, minor: true, timeStep: 5e-5, left: axis(false, 3.5, 5.5), right: axis(true) };
  const applied = applyToAll(graphs, 2, chosen);
  assert.deepEqual(applied[1].settings, chosen);
  for (const graph of applied) assert.deepEqual([graph.settings.major, graph.settings.minor, graph.settings.timeStep], [false, true, 5e-5]);
  // VC_V gets the typed range; the gate voltage keeps its own range but takes the automatic step.
  assert.deepEqual(applied[0].settings.left, chosen.left);
  assert.deepEqual(applied[2].settings.left, axis(false, 0, 12, null));
  // An automatic right axis turns every right axis automatic.
  assert.deepEqual(applied.map(graph => graph.settings.right.auto), [true, true, true]);
});

test("time zoom keeps the pointer time and stays inside the run; panning stops at its ends", () => {
  const full = [0, 1e-3];
  const zoomed = zoomRange(full, 0.5, 2.5e-4, full, 1e-6);
  assert.ok(Math.abs(zoomed[0] - 1.25e-4) < 1e-15 && Math.abs(zoomed[1] - 6.25e-4) < 1e-15);
  assert.deepEqual(zoomRange(zoomed, 10, 5e-4, full, 1e-6), full);
  const narrowest = zoomRange(full, 1e-9, 5e-4, full, 4e-7);
  assert.ok(Math.abs(narrowest[1] - narrowest[0] - 4e-7) < 1e-15);
  assert.deepEqual(panRange([2e-4, 4e-4], -1, full), [0, 2e-4]);
  assert.deepEqual(panRange([2e-4, 4e-4], 1, full), [8e-4, 1e-3]);
});

test("the waveform window opens the newest completed run of the circuit folder and reads it by file", async () => {
  const folder = mkdtempSync(path.join(tmpdir(), "litzsim-wave-screen-"));
  try {
    const file = path.join(folder, "buck.litzsim");
    copyFileSync(`${root}examples/buck.litzsim`, file);
    const opened = await call("inspect", { file });
    assert.deepEqual((await call("runs", startRequest({ file }))).runs, []);
    const started = await call("start", startRequest({ file, document: opened.document, expectedRevision: opened.revision }));
    let status = started;
    while (["running", "stopping"].includes(status.state)) {
      await new Promise(resolve => setTimeout(resolve, 50));
      status = await call("status", startRequest({ file, runId: started.runId }));
    }
    assert.equal(status.state, "completed");

    const { runs } = await call("runs", startRequest({ file }));
    assert.equal(defaultRun(runs, null), started.runId);
    assert.equal(defaultRun([{ runId: "new", state: "failed" }, ...runs], "missing"), started.runId);
    const [run] = runs;
    assert.equal(run.revision, opened.revision);
    assert.deepEqual(run.channels.map(channel => channel.name), ["D1_A", "L1_A", "VC_V"]);
    assert.equal(run.timeRange[0], 0);

    // Requests name the circuit file, so a core that did not start the run (after reopening Litzio) finds it.
    const request = { file, runId: run.runId, channels: ["L1_A", "VC_V"] };
    const wave = await call("wave", startRequest({ ...request, from_s: run.timeRange[0], to_s: run.timeRange[1], pixels: 400 }));
    const [current, voltage] = wave.channels;
    const peak = current.points.find(point => point.value === current.maximum);
    assert.ok(peak, "the maximum is one of the drawn points");
    const measured = await call("measure", startRequest({ ...request, from_s: run.timeRange[0], to_s: run.timeRange[1] }));
    assert.equal(measured.channels[0].maximum, current.maximum);
    assert.equal(measured.channels[1].minimum, voltage.minimum);
    assert.equal(measured.channels[1].to.row, run.rows - 1);
    assert.ok(measured.channels[1].average > voltage.minimum && measured.channels[1].average < voltage.maximum);
    // The table shows A (from) and B (to) as sent and the core's differences.
    const swapped = await call("measure", startRequest({ ...request, from_s: run.timeRange[1], to_s: run.timeRange[0] }));
    assert.equal(swapped.deltaTime_s, -measured.deltaTime_s);
    assert.equal(swapped.frequency_Hz, 1 / (run.timeRange[1] - run.timeRange[0]));
    assert.equal(swapped.channels[1].delta, swapped.channels[1].to.value - swapped.channels[1].from.value);
    assert.equal(swapped.channels[1].average, measured.channels[1].average);
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
});
