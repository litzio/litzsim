import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { isDeepStrictEqual } from 'node:util';

const [cli, fixtures, examples = 'examples'] = process.argv.slice(2);
if (!cli || !fixtures) throw new Error('Usage: node verify-flow.mjs CLI FIXTURE_DIRECTORY [EXAMPLES]');
const read = file => JSON.parse(readFileSync(file, 'utf8'));
function command(args, request) {
  const execution = spawnSync(cli, args, { input: request && JSON.stringify(request) + '\n', encoding: 'utf8', windowsHide: true });
  assert.equal(execution.status, 0, execution.stderr);
  return execution.stdout ? JSON.parse(execution.stdout) : undefined;
}
for (const name of ['rc', 'buck']) {
  const source = path.join(examples, name + '.litzsim');
  const sheet = command(['web-circuit'], { action: 'open', path: source }).value;
  const [left, top, width, height] = sheet.layout.viewBox;
  const x = Math.round((left + width / 2) / sheet.layout.pitch);
  const y = Math.round((top + height - 40) / sheet.layout.pitch);
  const id = 'resistor1';
  const output = name === 'rc' ? 'R1.n' : 'L1.n';
  const operations = [
    { action: 'add', type: 'resistor', x, y },
    { action: 'move', id, x: x + 2, y },
    { action: 'rotate', id },
    { action: 'setParameter', id, key: 'resistance_ohm', value: 2200 },
    { action: 'connect', a: id + '.p', b: output },
    { action: 'connect', a: id + '.n', b: 'GND.ref' },
  ];
  const request = { action: 'apply', expectedRevision: sheet.revision, document: sheet.document, operations };
  const transaction = command(['web-circuit'], request).value;
  const expected = read(source);
  expected.parts[id] = { type: 'resistor', parameters: { resistance_ohm: 2200 } };
  expected.layout[id] = { x: x + 2, y, rotation: 90 };
  expected.nets.output.push(id + '.p');
  expected.nets.ground.push(id + '.n');
  assert.deepEqual(transaction.document, expected);
  assert.equal(transaction.receipt.before, sheet.revision);
  assert.equal(transaction.receipt.after, transaction.revision);
  assert.deepEqual(transaction.receipt.created, [id]);
  const target = path.join(fixtures, name + '.litzsim');
  const saved = readFileSync(target);
  assert.deepEqual(read(target), expected);
  assert.match(command(['web-circuit'], {
    action: 'save', path: target, document: sheet.document, diskRevision: 'stale',
  }).error, /changed on disk/);
  assert.deepEqual(readFileSync(target), saved);
  assert.match(command(['web-circuit'], { ...request, expectedRevision: 'stale' }).error, /revision changed/);
  const failed = command(['web-circuit'], { ...request, operations: [
    ...operations, { action: 'connect', a: 'unknown.p', b: 'unknown.p' },
  ] });
  assert.deepEqual(Object.keys(failed), ['error']);
  assert.match(failed.error, /Operation 6: Unknown terminal/);
  console.log(name + ' v1 matches the core batch and the independently specified parts, layout and nets.');
}
const runs = readdirSync(path.join(fixtures, 'runs')).map(id => {
  const directory = path.join(fixtures, 'runs', id);
  return { directory, ...read(path.join(directory, 'status.json')) };
});
for (const name of ['rc', 'buck']) {
  const saved = read(path.join(fixtures, name + '.litzsim'));
  const long = read(path.join(fixtures, name + '-long.litzsim'));
  const completed = runs.find(run => run.state === 'completed' &&
    isDeepStrictEqual(read(path.join(run.directory, run.circuit)), saved));
  const canceled = runs.find(run => run.state === 'canceled' &&
    isDeepStrictEqual(read(path.join(run.directory, run.circuit)), long));
  assert.ok(completed && canceled);
  const expectedWave = path.join(completed.directory, 'comparison.wave');
  command(['run', path.join(fixtures, name + '.litzsim'), expectedWave]);
  assert.deepEqual(readFileSync(path.join(completed.directory, completed.result)), readFileSync(expectedWave));
  const snapshot = read(path.join(canceled.directory, canceled.circuit));
  assert.deepEqual(snapshot, long);
  console.log(name + ' completed WAVEs match the direct CLI, and the canceled run preserved its original snapshot.');
}
