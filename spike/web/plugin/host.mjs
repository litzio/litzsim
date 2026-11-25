import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { accessSync, existsSync, mkdirSync, writeFileSync, renameSync } from 'node:fs';
import path from 'node:path';
import { createInterface } from 'node:readline';

export function activate({ settings }) {
  const { executable, persistentCore } = settings;
  const runs = new Map();
  const children = new Set();
  let core;
  let coreQueue = Promise.resolve();
  let activeRun;

  function launch(args) {
    if (!path.isAbsolute(executable ?? '')) throw new Error('Set an absolute LitzSim executable path.');
    const child = spawn(executable, args, { windowsHide: true });
    const done = new Promise(resolve => child.once('close', (code, signal) => {
      children.delete(child);
      resolve({ code, signal });
    }));
    children.add(child);
    child.on('error', () => {});
    child.stdin.on('error', () => {});
    child.stderr.resume();
    return { child, done };
  }

  async function command(args, message) {
    const started = performance.now();
    const { child, done } = launch(args);
    let spawnMs;
    child.once('spawn', () => { spawnMs = performance.now() - started; });
    child.stdout.setEncoding('utf8');
    let output = '';
    child.stdout.on('data', chunk => { output += chunk; });
    child.stdin.end(message === undefined ? undefined : JSON.stringify(message) + '\n');
    if ((await done).code !== 0) throw new Error('LitzSim CLI failed. Check its executable and circuit settings.');
    return { ...JSON.parse(output), cliMs: performance.now() - started, spawnMs };
  }

  function sendToCore(message) {
    const started = performance.now();
    if (!core) {
      const process = launch(['web-circuit', '--persistent']);
      const lines = createInterface({ input: process.child.stdout })[Symbol.asyncIterator]();
      core = { ...process, lines };
      process.done.then(() => { if (core?.child === process.child) core = undefined; });
    }
    const owner = core;
    owner.child.stdin.write(JSON.stringify(message) + '\n');
    return Promise.race([
      owner.lines.next().then(({ value, done }) => {
        if (done) throw new Error('The circuit core process exited.');
        return { ...JSON.parse(value), cliMs: performance.now() - started, spawnMs: 0 };
      }),
      owner.done.then(() => { throw new Error('The circuit core process exited.'); }),
    ]);
  }

  async function circuit(message) {
    const began = performance.now();
    const task = coreQueue.then(() => persistentCore
      ? sendToCore(message)
      : command(['web-circuit'], message));
    coreQueue = task.catch(() => {});
    const { value, error, cliMs, spawnMs } = await task;
    if (error) throw new Error(error);
    return { ...value, timing: { ...value.timing, cliMs, spawnMs, handlerMs: performance.now() - began } };
  }

  function getRun(id) {
    const run = runs.get(id);
    if (!run) throw new Error('Unknown run.');
    return run;
  }

  function record(run, state) {
    run.state = state;
    const target = path.join(run.directory, 'status.json');
    writeFileSync(target + '.tmp', JSON.stringify({
      id: run.id, state, circuit: 'circuit.litzsim', result: 'result.wave',
    }));
    renameSync(target + '.tmp', target);
  }

  const snapshot = ({ id, state }) => ({ id, state });

  async function start({ file, document }) {
    if (activeRun) throw new Error('A run is already active.');
    const id = randomUUID();
    const directory = path.join(path.dirname(file), 'runs', id);
    const input = path.join(directory, 'circuit.litzsim');
    const result = path.join(directory, 'result.wave');
    const run = { id, directory, result, state: 'starting' };
    activeRun = run;
    try {
      mkdirSync(directory, { recursive: true });
      await circuit({ action: 'snapshot', path: input, document });
      const { child, done } = launch(['run', input, result]);
      child.stdout.resume();
      child.stdin.end();
      run.child = child;
      run.finished = done.then(({ code }) => {
        const state = run.state === 'stopping' ? 'canceled' : code === 0 && existsSync(result) ? 'completed' : 'failed';
        try { record(run, state); }
        finally { activeRun = undefined; }
      });
      run.finished.catch(() => { run.state = 'failed'; });
      runs.set(id, run);
      record(run, 'running');
      return snapshot(run);
    } catch (error) {
      if (run.child) run.child.kill();
      else activeRun = undefined;
      throw error;
    }
  }

  const opened = new Set();
  function openedFile(file) {
    if (!opened.has(file)) throw new Error('Open the circuit first.');
    return file;
  }

  return {
    status: () => {
      try {
        if (!path.isAbsolute(executable ?? '')) throw new Error();
        accessSync(executable);
        return { available: true, message: 'LitzSim CLI configured' };
      } catch {
        return { available: false, message: 'Set the absolute path of the LitzSim CLI in plugin settings.' };
      }
    },
    operations: {
      async open({ file }) {
        if (!path.isAbsolute(file) || path.extname(file).toLowerCase() !== '.litzsim')
          throw new Error('Select a .litzsim file.');
        const value = await circuit({ action: 'open', path: file });
        opened.add(file);
        return value;
      },
      apply: ({ document, expectedRevision, operations }) => circuit({ action: 'apply', document, expectedRevision, operations }),
      undo: ({ current, expectedRevision, previous }) => {
        if (current.revision !== expectedRevision) throw new Error('The undo snapshot is stale.');
        return circuit({ action: 'layout', document: previous });
      },
      save: ({ file, document, diskRevision }) => circuit({ action: 'save', path: openedFile(file), document, diskRevision }),
      start: ({ file, document }) => start({ file: openedFile(file), document }),
      status: ({ id }) => snapshot(getRun(id)),
      async stop({ id }) {
        const run = getRun(id);
        if (run.state === 'running') {
          record(run, 'stopping');
          run.child.kill();
          await run.finished;
        }
        return snapshot(run);
      },
      async wave({ id, channel }) {
        const run = getRun(id);
        if (run.state !== 'completed') throw new Error('The run has no completed result.');
        return command(['web-wave', run.result, String(channel)]);
      },
    },
    async dispose() {
      const pending = [...children];
      if (activeRun?.state === 'running') record(activeRun, 'stopping');
      await Promise.all(pending.map(child => new Promise(resolve => {
        child.once('close', resolve);
        child.kill();
      })));
    },
  };
}
