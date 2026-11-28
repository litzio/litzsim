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
import {
  applyToAll, autoRange, axisFor, defaultRun, formatValue, keepSignals, keepView, niceStep, panRange, readTimeAxis, ticks,
  timeAxisProblem, zoomAxis, zoomRange,
} from "./plot.js";
import { CATEGORIES, DUTY_UPDATE_LABELS, PARTS, RECORDING_LABELS, ROTATE_SHORTCUT, libraryCategories, partLabel, shortcutOf } from "./parts.js";
import {
  deleteSelection, isDirty, nearestPair, pwmChoices, renameOps, renameProblem, renamedIds, selectInBox, selectedSegmentIndex, selectionBox,
  sourcePreviewRequest, sumParameterOps, undoRenamed, wireStyle,
} from "./rules.js";

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

async function send(op, payload = {}) {
  core.stdin.write(JSON.stringify({ ...payload, op }) + "\n");
  return JSON.parse((await lines.next()).value);
}

async function call(op, payload = {}) {
  const response = await send(op, payload);
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

test("library labels retain separate drawn segments and classify a remote probe as signal", async () => {
  const opened = await call("inspect", { file: `${root}examples/control-signals-labels.litzsim` });
  const sheet = await apply(opened, []);
  assert.ok(libraryCategories(sheet.layout).find(group => group.id === "connections").parts.includes(PARTS.label));
  const remote = await apply(sheet, [
    { op: "add", type: "voltageProbe", grid: [15, -3] },
    { op: "add", type: "label", grid: [12, -3] },
    { op: "connect", a: "LABEL1.p", b: "VP1.p" },
  ]);
  const net = remote.layout.pins["VP1.p"].netId;
  assert.ok(remote.layout.labelWarnings.some(warning => warning.partId === "LABEL1"));
  const joined = await apply(remote, [{ op: "setParameter", id: "LABEL1", key: "text", value: "sTeP" }]);
  assert.equal(joined.layout.labelWarnings, undefined);
  assert.equal(joined.layout.pins["VP1.p"].signalDirection, "input");
  assert.equal(joined.layout.pins["VP1.p"].netId, net);
  assert.ok(joined.layout.labels.some(label => label.partId === "LABEL1" && label.text === "sTeP"));
  assert.deepEqual(joined.layout.segments, remote.layout.segments);
  assert.equal(wireStyle(joined.layout.signalNets, net).stroke, "var(--id-color-accent-blue-paper)");
});

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

test("sampler SOC and control task edits use core PWM references", async () => {
  const sheet = await apply(await rc(), [
    { op: "add", type: "pwm" },
    { op: "add", type: "dutyPwm" },
    { op: "add", type: "sampler", grid: [0, 8] },
  ]);
  assert.deepEqual(pwmChoices(sheet.document.parts).map(choice => choice.value), ["PWM1", "PWM2"]);
  assert.deepEqual(pwmChoices(sheet.document.parts, true).map(choice => choice.value), ["PWM2"]);
  assert.equal(partLabel("sampler"), "샘플러");
  assert.equal(sheet.layout.pins["ZOH1.in"].signalDirection, "input");
  assert.equal(sheet.layout.pins["ZOH1.out"].signalDirection, "output");
  assert.deepEqual(sheet.document.parts.ZOH1.parameters.soc, { mode: "task", position: 0 });
  const timed = await apply(sheet, [
    { op: "setControlTask", task: { mode: "pwm", pwm: "PWM2", position: 0.75 } },
    { op: "setParameter", id: "ZOH1", key: "soc", value: { mode: "onCenter", pwm: "PWM2" } },
    ...renameOps("PWM2", "Carrier"),
  ]);
  assert.equal(timed.document.simulation.controlTask.pwm, "Carrier");
  assert.equal(timed.document.parts.ZOH1.parameters.soc.pwm, "Carrier");
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

test("the editor can connect a node voltage probe to a signal net through the core", async () => {
  const opened = await call("inspect", { file: `${root}examples/control-signals.litzsim` });
  const sheet = await apply(opened, [
    { op: "add", type: "voltageProbe", grid: [-3, 3] },
    { op: "connect", a: "VP1.p", b: "RAMP1.out" },
  ]);
  const { pins, segments, signalNets } = sheet.layout;
  assert.equal(pins["VP1.p"].netId, "ramp");
  assert.equal(pins["VP1.p"].signalDirection, "input");
  assert.ok(signalNets.includes("ramp"));
  const branch = segments.find(segment => segment.terminals[0] === "VP1.p");
  assert.ok(branch);
  assert.equal(wireStyle(signalNets, branch.netId).stroke, "var(--id-color-accent-blue-paper)");

  for (const [type, id] of [["voltmeter", "VM1"], ["ammeter", "AM1"], ["constant", "CONST2"]]) {
    const terminal = type === "constant" ? `${id}.out` : `${id}.p`;
    const refused = await send("apply", {
      document: sheet.document, expectedRevision: sheet.revision,
      ops: [{ op: "add", type }, { op: "connect", a: "VP1.p", b: terminal }],
    });
    assert.equal(refused.ok, false);
    assert.match(refused.error.message, type === "constant"
      ? /^Signal net has multiple outputs:/ : /^Cannot connect signal and electrical terminals:/);
  }
  const refused = await send("apply", {
    document: sheet.document, expectedRevision: sheet.revision,
    ops: [{ op: "insertInBranch", type: "ammeter", terminals: branch.terminals, from: branch.from, to: branch.to }],
  });
  assert.equal(refused.ok, false);
  assert.equal(refused.error.message, "expected an electrical branch");
});

test("the control library, recording and wire styles follow the core layout", async () => {
  const sheet = await apply(await rc(), [
    { op: "add", type: "constant", grid: [1, 10] },
    { op: "add", type: "sum", grid: [6, 11] },
    { op: "connect", a: "CONST1.out", b: "SUM1.in1" },
    { op: "setRecording", id: "SUM1", quantity: "output", enabled: true },
  ]);
  const { pins, segments, signalNets, symbols } = sheet.layout;
  const categories = libraryCategories(sheet.layout);
  const control = categories.find(group => group.id === "control");
  assert.equal(control.label, "제어");
  assert.deepEqual(control.parts.map(part => [part.type, part.key]), [
    ["pwm", "P"], ["voltageSensor", undefined], ["currentSensor", undefined], ["constant", undefined],
    ["step", undefined], ["ramp", undefined], ["sum", undefined], ["gain", undefined], ["sampler", undefined], ["limiter", undefined],
    ["integrator", undefined], ["pi", undefined], ["dutyPwm", undefined],
  ]);
  assert.ok(categories.find(group => group.id === "switches").parts.some(part => part.type === "timedSwitch"));
  const future = libraryCategories({ controlKinds: ["futureBlock"] }).find(group => group.id === "control").parts.at(-1);
  assert.deepEqual(future, { type: "futureBlock", label: "futureBlock" });
  const buck = await call("inspect", { file: `${root}examples/buck-voltage-loop.litzsim` });
  const { layout: loop } = await apply(buck, [{ op: "add", type: "pwm", grid: [0, 15] }]);
  assert.equal(DUTY_UPDATE_LABELS[loop.symbols.find(symbol => symbol.id === "PWM").dutyUpdateTiming], "주기 시작");
  assert.equal(loop.symbols.find(symbol => symbol.id === "PWM1").dutyUpdateTiming, undefined);
  const sum = symbols.find(symbol => symbol.id === "SUM1");
  assert.deepEqual(sum.recordable.map(quantity => RECORDING_LABELS[quantity]), ["출력 기록"]);
  assert.deepEqual(sheet.document.parts.SUM1.record, { output: true });

  const signal = pins["CONST1.out"].netId;
  assert.deepEqual(signalNets, [signal]);
  assert.ok(segments.some(segment => segment.netId === signal));
  assert.ok(segments.some(segment => segment.netId === "input"));
  assert.deepEqual(wireStyle(signalNets, signal), { stroke: "var(--id-color-accent-blue-paper)", strokeWidth: 1.5 });
  assert.deepEqual(wireStyle(signalNets, "input"), {});
  const refused = await send("apply", {
    document: sheet.document, expectedRevision: sheet.revision,
    ops: [{ op: "connect", a: "CONST1.out", b: "R1.p" }],
  });
  assert.equal(refused.ok, false);
  assert.match(refused.error.message, /^Cannot connect signal and electrical terminals:/);

  assert.ok(sum.strokes.some(stroke => stroke.kind === "circle"));
  assert.deepEqual(sheet.document.parts.SUM1.parameters, { inputs: "|+-", iconShape: "round" });
  const signs = sumParameterOps("SUM1", "inputs", "|++-");
  const shape = sumParameterOps("SUM1", "iconShape", "rectangular");
  assert.deepEqual(signs, [{ op: "setParameter", id: "SUM1", key: "inputs", value: "|++-" }]);
  assert.deepEqual(shape, [{ op: "setParameter", id: "SUM1", key: "iconShape", value: "rectangular" }]);
  const changed = await apply(sheet, [...signs, ...shape]);
  assert.deepEqual(changed.document.parts.SUM1.parameters, { inputs: "|++-", iconShape: "rectangular" });
  assert.equal(changed.layout.pins["SUM1.in3"].signalDirection, "input");
  assert.equal(changed.layout.symbols.find(symbol => symbol.id === "SUM1").strokes.some(stroke => stroke.kind === "circle"), false);
  const invalid = await send("apply", {
    document: sheet.document, expectedRevision: sheet.revision, ops: sumParameterOps("SUM1", "inputs", "|"),
  });
  assert.equal(invalid.ok, false);
  assert.equal(invalid.error.targetId, "SUM1");
  assert.match(invalid.error.message, /^inputs /);
});

test("new control and switch types use the same display names in the library and properties", async () => {
  const { layout } = await apply(await rc(), []);
  const entries = libraryCategories(layout).flatMap(group => group.parts);
  for (const [type, label] of [
    ["integrator", "적분기"], ["pi", "PI 제어기"], ["dutyPwm", "듀티 입력 PWM"], ["timedSwitch", "시간 스위치"],
  ]) {
    assert.equal(entries.find(part => part.type === type)?.label, label);
    assert.equal(partLabel(type), label);
  }
});

test("properties request a preview for the selected source using the current document", async () => {
  const sheet = await call("inspect", { file: `${root}examples/control-signals.litzsim` });
  const request = sourcePreviewRequest(sheet.document, "RAMP1");
  assert.deepEqual(request, { document: sheet.document, partId: "RAMP1" });
  assert.equal(sourcePreviewRequest(sheet.document, "SUM1"), null);
  const buck = await call("inspect", { file: `${root}examples/buck-voltage-loop.litzsim` });
  assert.equal(sourcePreviewRequest(buck.document, "PWM"), null);
  const before = await call("sourcePreview", request);
  const changed = await apply(sheet, [{ op: "setParameter", id: "RAMP1", key: "slope_per_s", value: 8 }]);
  const after = await call("sourcePreview", sourcePreviewRequest(changed.document, "RAMP1"));
  assert.notDeepEqual(before.points, after.points);
  assert.deepEqual(after.events, [{ kind: "rampEnd", time: 0.5 }]);
  const invalid = await apply(changed, [{ op: "setParameter", id: "RAMP1", key: "slope_per_s", value: -8 }]);
  const rejected = await send("sourcePreview", sourcePreviewRequest(invalid.document, "RAMP1"));
  assert.equal(rejected.error.message, "Ramp slope must point toward finalValue");
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

test("a nearby terminal is suggested by distance and the core validates its net", async () => {
  const opened = await call("inspect", { file: `${root}examples/control-signals.litzsim` });
  const sheet = await apply(opened, [{ op: "add", type: "constant", grid: [2, 4] }]);
  const { a, b, distance } = nearestPair(sheet.layout, "CONST2");
  assert.deepEqual([a, b, distance], ["CONST2.out", "SUM1.in1", 32]);
  const refused = await send("apply", {
    document: sheet.document, expectedRevision: sheet.revision,
    ops: [{ op: "connect", a, b }],
  });
  assert.equal(refused.ok, false);
  assert.equal(refused.error.message, "Signal net has multiple outputs: CONST2.out, CONST1.out");

  // Moving the same terminal beside an output changes the nearest candidate,
  // even though the core would also refuse that connection.
  const nearOutput = await apply(opened, [{ op: "add", type: "constant", grid: [0, 1] }]);
  assert.equal(nearestPair(nearOutput.layout, "CONST2")?.b, "CONST1.out");
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

test("the name field sends one rename, follows the new ID and shows why the core refused a name", async () => {
  const sheet = await apply(await rc(), []);
  assert.deepEqual(renameOps("VC", " VC "), []);
  const ops = renameOps("VC", " Vout ");
  assert.deepEqual(ops, [{ op: "rename", id: "VC", newId: "Vout" }]);
  const renamed = await call("apply", { document: sheet.document, expectedRevision: sheet.revision, ops });
  assert.deepEqual(renamed.receipt.renamed, { VC: "Vout" });
  assert.deepEqual(renamed.receipt.created, []);
  assert.ok(renamed.layout.labels.some(label => label.partId === "Vout" && label.text === "Vout"));
  assert.deepEqual(renamedIds(["VC", "R1"], renamed.receipt.renamed), ["Vout", "R1"]);
  assert.deepEqual(renamedIds(["Vout"], undoRenamed(renamed.receipt)), ["VC"]);
  const constructorNamed = await call("apply", {
    document: sheet.document, expectedRevision: sheet.revision, ops: renameOps("VC", "constructor"),
  });
  assert.deepEqual(renamedIds(["VC"], constructorNamed.receipt.renamed), ["constructor"]);
  assert.deepEqual(renamedIds(["constructor"], {}), ["constructor"]);
  assert.deepEqual(renamedIds(["constructor"], undoRenamed(constructorNamed.receipt)), ["VC"]);
  const undone = await call("undo", {
    document: renamed.document, expectedAfter: renamed.layout.revision, expectedBefore: sheet.revision, previous: sheet.document,
  });
  assert.deepEqual(undone.document, sheet.document);

  for (const [text, reason] of [
    ["r1", "R1이(가) 이미 쓰는 이름입니다. 대소문자만 다른 이름도 쓸 수 없습니다."],
    ["  ", "이름을 입력하세요."],
    ["V out", "영문자로 시작하고 영문자·숫자·밑줄(_)만 쓸 수 있습니다."],
    ["V".repeat(33), "32자 이하로 입력하세요."],
  ]) {
    const refused = await send("apply", { document: sheet.document, expectedRevision: sheet.revision, ops: renameOps("VC", text) });
    assert.equal(refused.ok, false);
    assert.equal(refused.error.targetId, "VC");
    assert.equal(renameProblem(refused.error), reason);
  }
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
  const manual = { major: true, minor: false, left: axis(false, 0, 12, 2), right: axis(false, -1, 1, 0.5) };
  const graphs = [
    { id: 1, signals: [{ name: "VC_V", unit: "V", axis: "left" }], settings: manual },
    { id: 2, signals: [{ name: "VC_V", unit: "V", axis: "left" }, { name: "AM1_A", unit: "A", axis: "right" }], settings: manual },
    { id: 3, signals: [{ name: "VM1_V", unit: "V", axis: "left" }], settings: manual },
  ];
  const chosen = { major: false, minor: true, left: axis(false, 3.5, 5.5), right: axis(true) };
  const applied = applyToAll(graphs, 2, chosen);
  assert.deepEqual(applied[1].settings, chosen);
  for (const graph of applied) assert.deepEqual([graph.settings.major, graph.settings.minor], [false, true]);
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

test("the time axis takes typed times with property-panel prefixes and refuses a range it cannot show", () => {
  const full = [0, 1e-3];
  // Untouched fields arrive as the live numbers and pass unchanged; typed ones are parsed.
  assert.deepEqual(readTimeAxis({ from: 2e-4, to: 4e-4, step: null }, full), { range: [2e-4, 4e-4], step: null });
  assert.deepEqual(readTimeAxis({ from: "0.5m", to: 1e-3, step: "50u" }, full), { range: [5e-4, 1e-3], step: 50 * 1e-6 });
  assert.deepEqual(readTimeAxis({ from: 0, to: 1e-3, step: " " }, full).step, null);
  // A formatted end a hair past the run is the run's end, not "out of range".
  assert.deepEqual(readTimeAxis({ from: 0, to: "1.0000000001m", step: null }, full).range, full);

  const refused = (fields, expected) => assert.deepEqual(Object.keys(readTimeAxis(fields, full).problems ?? {}), expected);
  refused({ from: "0.6m", to: "0.2m", step: null }, ["to"]);
  refused({ from: "0.5m", to: "0.5m", step: null }, ["to"]);
  refused({ from: "-1u", to: "1m", step: null }, ["from"]);
  refused({ from: 0, to: "2m", step: null }, ["to"]);
  refused({ from: "1 ms", to: "abc", step: "0" }, ["from", "to", "step"]);
  refused({ from: 0, to: 1e-3, step: "-1u" }, ["step"]);
  // A step that would draw more than 400 grid lines over the typed range.
  refused({ from: 0, to: 1e-3, step: "1n" }, ["step"]);
  assert.match(timeAxisProblem(readTimeAxis({ from: 0, to: "2m", step: null }, full).problems), /^끝: 실행 구간 0 s – 1 ms/);
});

test("a new run keeps graphs, settings and the zoomed time range; only signals it did not record leave", () => {
  const settings = { major: true, minor: true, left: { auto: false, min: 3.5, max: 5.5, step: 0.5 }, right: { auto: true, min: null, max: null, step: null } };
  const graphs = [
    { id: 1, signals: [{ name: "VC_V", unit: "V", axis: "left" }], settings },
    { id: 2, signals: [{ name: "VC_V", unit: "V", axis: "left" }, { name: "AM1_A", unit: "A", axis: "right" }], settings },
    { id: 3, signals: [], settings },
  ];
  const kept = keepSignals(graphs, [{ name: "L1_A", unit: "A" }, { name: "VC_V", unit: "V" }]);
  assert.deepEqual(kept.map(graph => graph.id), [1, 2, 3]);
  assert.equal(kept[0], graphs[0], "a graph whose signals all remain is untouched");
  assert.deepEqual(kept[1].signals, [{ name: "VC_V", unit: "V", axis: "left" }]);
  assert.equal(kept[1].settings, settings);
  assert.equal(kept[2], graphs[2]);

  // A zoomed view stays; a whole-run view follows the new run; a view past its end is clipped or reset.
  assert.deepEqual(keepView([2e-4, 4e-4], [0, 1e-3], [0, 2e-3]), [2e-4, 4e-4]);
  assert.deepEqual(keepView([0, 1e-3], [0, 1e-3], [0, 2e-3]), [0, 2e-3]);
  assert.deepEqual(keepView([0, 1e-3 * (1 - 1e-12)], [0, 1e-3], [0, 5e-4]), [0, 5e-4]);
  assert.deepEqual(keepView([8e-4, 1e-3 - 1e-5], [0, 1e-3], [0, 9e-4]), [8e-4, 9e-4]);
  assert.deepEqual(keepView([6e-4, 8e-4], [0, 1e-3], [0, 5e-4]), [0, 5e-4]);
});

test("the waveform window opens the newest completed run of the circuit folder and reads it by file", async () => {
  const folder = mkdtempSync(path.join(tmpdir(), "litzsim-wave-screen-"));
  try {
    const file = path.join(folder, "buck.litzsim");
    copyFileSync(`${root}examples/buck.litzsim`, file);
    const opened = await call("inspect", { file });
    assert.deepEqual((await call("runs", { file })).runs, []);
    const recording = { op: "setRecording", id: "L1", quantity: "current" };
    const sheet = await apply(opened, [{ ...recording, enabled: true }]);
    const disabled = await apply(sheet, [{ ...recording, enabled: false }]);
    assert.equal(disabled.document.parts.L1.record, undefined);
    assert.equal(disabled.revision, opened.revision);
    const started = await call("start", { file, document: sheet.document, expectedRevision: sheet.revision });
    let status = started;
    while (["running", "stopping"].includes(status.state)) {
      await new Promise(resolve => setTimeout(resolve, 50));
      status = await call("status", startRequest({ file, runId: started.runId }));
    }
    assert.equal(status.state, "completed");

    const { runs } = await call("runs", { file });
    assert.equal(defaultRun(runs, null), started.runId);
    assert.equal(defaultRun([{ runId: "new", state: "failed" }, ...runs], "missing"), started.runId);
    const [run] = runs;
    assert.equal(run.revision, sheet.revision);
    assert.equal(run.circuit, "buck.litzsim");
    assert.equal(run.pinned, false);
    assert.deepEqual(run.channels.map(channel => channel.name), ["L1_A", "VC_V"]);
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

    // The run history pins, unpins and deletes the shown run by circuit file and run ID.
    const target = { file, runId: run.runId };
    assert.equal((await call("pin", startRequest(target))).pinned, true);
    assert.equal((await call("runs", { file })).runs[0].pinned, true);
    assert.equal((await call("unpin", startRequest(target))).pinned, false);
    assert.equal((await call("delete", startRequest(target))).runId, run.runId);
    assert.deepEqual((await call("runs", { file })).runs, []);
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
});
