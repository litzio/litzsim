// Run after `cargo build --release`: node --test plugin
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { createInterface } from "node:readline";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { isDirty, nearestPair, shortedSources } from "./rules.js";

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

test("connections that would short a voltage source are flagged before confirming", async () => {
  const { document } = await rc();
  assert.deepEqual(shortedSources(document, "R1.p", "V1.n"), ["V1"]);
  assert.deepEqual(shortedSources(document, "V1.p", "V1.n"), ["V1"]);
  assert.deepEqual(shortedSources(document, "R1.p", "C1.p"), []);
  assert.deepEqual(shortedSources(document, "C1.n", "GND.ref"), []);
});
