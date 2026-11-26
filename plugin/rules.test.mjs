// Run after `cargo build --release`: node --test plugin
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { createInterface } from "node:readline";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { startRequest } from "./host.mjs";
import { PARTS, ROTATE_SHORTCUT, shortcutOf } from "./parts.js";
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
