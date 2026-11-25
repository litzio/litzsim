import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { once } from 'node:events';
import { createReadStream } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const build = resolve(process.argv[2] ?? 'build');
const fixture = join(build, 'wave-spike');
const cli = join(build, 'litzsim-cli.exe');
const viewer = join(build, 'viewer-test.exe');
const serverScript = fileURLToPath(new URL('server.mjs', import.meta.url));
const foregroundScript = fileURLToPath(new URL('foreground.ps1', import.meta.url));
const load = page => page.evaluate(() => window.waveMeasurement);
const p95 = values => [...values].sort((a, b) => a - b)[Math.ceil(values.length * 0.95) - 1];

async function warm(file) {
  for await (const chunk of createReadStream(file, { highWaterMark: 1024 * 1024 })) {
    assert(chunk.length > 0);
  }
}

async function frameCadence(page) {
  const intervals = await page.evaluate(async () => {
    const next = () => new Promise(resolve => requestAnimationFrame(resolve));
    const intervals = [];
    let previous = await next();
    for (let i = 0; i < 24; ++i) {
      const current = await next();
      intervals.push(current - previous);
      previous = current;
    }
    return intervals;
  });
  return { medianMs: [...intervals].sort((a, b) => a - b)[11], p95Ms: p95(intervals) };
}

function verify(measurement, raw) {
  assert.equal(measurement.open.rows, 10_000_000);
  assert.equal(measurement.firstPlots.length, 4);
  assert(measurement.firstPlots[0].some(([row, x, y]) => row === 256 && x === 256 && y === 23));
  assert.deepEqual(raw.samples, [[255, 255, 0], [256, 256, 23], [257, 257, 0]]);
  const points = measurement.xy;
  assert(points.every(([row], i) => i === 0 || row > points[i - 1][0]));
  const edge = points.findIndex(([row]) => row === 512);
  const loop = points.slice(edge - 1, edge + 5);
  assert.deepEqual(loop.map(([, x, y]) => [x, y]), [[0, 0], [4, 0], [0, 3], [-4, 0], [0, -3], [0, 0]]);
  assert.deepEqual(loop.slice(1, 5).map(([row]) => row), [512, 513, 514, 515]);
  const area = loop.slice(1).reduce((sum, [, x, y], i) =>
    sum + loop[i][1] * y - x * loop[i][2], 0) / 2;
  // The raw path traverses three area-12 cross products and closes via (0, 0).
  assert.equal(area, 18);
  assert.equal(measurement.frames.length, 51);
  for (const [i, sample] of measurement.frames.slice(1).entries()) {
    assert.equal(sample.kind, i % 2 === 0 ? 'pan' : 'zoom');
    assert.notDeepEqual(sample.bounds, measurement.frames[i].bounds);
  }
}

function memory(processInfo, hostPid) {
  const ids = [...processInfo.map(item => item.id), hostPid];
  const script = `
    $native = @(Get-CimInstance Win32_Process -Filter 'ParentProcessId=${hostPid}')
    $ids = @(${ids.join(',')}) + @($native.ProcessId)
    @(Get-Process -Id $ids -ErrorAction SilentlyContinue |
      Select-Object Id, ProcessName, PeakWorkingSet64) | ConvertTo-Json -Compress
  `;
  const stats = JSON.parse(execFileSync('powershell.exe', ['-NoProfile', '-Command', script]));
  const typeOf = new Map(processInfo.map(item => [item.id, item.type]));
  return stats.map(item => ({
    type: typeOf.get(item.Id) ?? (item.Id === hostPid ? 'node' : item.ProcessName),
    peakWorkingSetBytes: item.PeakWorkingSet64,
  }));
}

function monitorWindow(browserPid) {
  const child = spawn('powershell.exe', [
    '-NoProfile', '-File', foregroundScript, String(browserPid),
  ], { windowsHide: true });
  const output = createInterface({ input: child.stdout });
  const closed = once(child, 'close');
  child.stderr.pipe(process.stderr, { end: false });
  const ready = Promise.race([
    once(output, 'line').then(([line]) => line),
    closed.then(() => { throw new Error('Window monitor stopped before its first sample'); }),
  ]);
  let lastLine;
  output.on('line', line => { lastLine = line; });
  return {
    ready,
    async stop() {
      child.stdin.end();
      const [code] = await closed;
      assert.equal(code, 0);
      return JSON.parse(lastLine);
    },
  };
}

async function trial(file, name, i, drawMode = 'direct') {
  await warm(file);
  const host = spawn(process.execPath, [serverScript, cli, file, join(fixture, 'web')], { windowsHide: true });
  const closed = once(host, 'close');
  const output = createInterface({ input: host.stdout });
  host.stderr.pipe(process.stderr, { end: false });
  let browser;
  let windowMonitor;
  try {
    const [port] = await Promise.race([
      once(output, 'line'),
      closed.then(() => { throw new Error('Local server closed during startup'); }),
    ]);
    browser = await chromium.launch({
      headless: false,
      ignoreDefaultArgs: ['--disable-backgrounding-occluded-windows',
        '--disable-background-timer-throttling', '--disable-renderer-backgrounding'],
    });
    const cdp = await browser.newBrowserCDPSession();
    const activeProcess = await cdp.send('SystemInfo.getProcessInfo');
    const browserPid = activeProcess.processInfo.find(item => item.type === 'browser').id;
    const page = await browser.newPage({ viewport: { width: 1280, height: 780 } });
    await page.bringToFront();
    windowMonitor = monitorWindow(browserPid);
    assert.equal(await windowMonitor.ready, 'sampling');
    await page.addInitScript(() => {
      let lostFocus = 0;
      window.focusLosses = () => lostFocus;
      addEventListener('blur', () => { ++lostFocus; });
      document.addEventListener('visibilitychange', () => {
        if (document.hidden) ++lostFocus;
      });
    });
    await page.goto(`http://127.0.0.1:${port}/?measure&draw=${drawMode}`);
    await page.waitForFunction(() => window.waveMeasurement?.xy);
    const plots = await page.locator('canvas').all();
    assert.equal(plots.length, 5);
    for (const plot of plots) {
      const box = await plot.boundingBox();
      assert(box && box.height >= 100 && box.y >= 0 && box.y + box.height <= 780);
    }
    const raw = await page.evaluate(async () => {
      const response = await fetch('/api/wave', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ op: 'at', channel: 1, rows: [255, 256, 257] }),
      });
      return response.json();
    });
    if (i === 0 && drawMode === 'direct')
      await page.screenshot({ path: join(fixture, `${name}-direct-windows.png`) });
    const cadenceBefore = await frameCadence(page);
    for (let j = 0; j < 50; ++j) {
      const direction = j % 4 < 2 ? 1 : -1;
      await page.mouse.move(640, 100);
      if (j % 2 === 0) {
        await page.mouse.down();
        await page.mouse.move(640 + direction * 20, 100);
        await page.mouse.up();
      } else {
        await page.mouse.wheel(0, direction * 120);
      }
      await page.waitForFunction(count => window.waveMeasurement.frames.length === count, j + 2);
    }
    const measured = await load(page);
    const cadenceAfter = await frameCadence(page);
    const foreground = await windowMonitor.stop();
    windowMonitor = undefined;
    verify(measured, raw);
    const pageFocus = {
      losses: await page.evaluate(() => window.focusLosses()),
      unfocusedInputs: measured.frames.filter(frame => !frame.inputFocused).length,
      unfocusedExactFrames: measured.frames.filter(frame => !frame.presentedFocused).length,
      unfocusedPreviews: measured.frames.filter(frame => frame.preview && !frame.preview.focused).length,
    };
    const dpr = await page.evaluate(() => window.devicePixelRatio);
    const { processInfo } = await cdp.send('SystemInfo.getProcessInfo');
    const peaks = memory(processInfo, host.pid);
    assert(peaks.some(p => p.type === 'litzsim-cli'));
    const { frames, open, xyTiming } = measured;
    for (const frame of frames) {
      frame.pipeOverheadMs = frame.pipeMs - frame.queryMs - frame.encodeMs;
      frame.httpOverheadMs = frame.fetchMs - frame.pipeMs;
      frame.frameWaitMs = frame.endToFrameMs - frame.startDelayMs - frame.fetchMs - frame.decodeMs - frame.drawMs;
    }
    const [first, ...interactions] = frames;
    const previews = interactions.map(item => item.preview?.endToFrameMs).filter(Number.isFinite);
    if (drawMode !== 'baseline') {
      assert(previews.length > 0);
    }
    const previewP95Ms = previews.length ? p95(previews) : null;
    const summary = {
      browser: browser.version(), dpr, drawMode, foreground, pageFocus, cadenceBefore, cadenceAfter,
      open, first, xyTiming, interactions, peaks,
      previewCount: previews.length, previewP95Ms,
      p95: Object.fromEntries(['endToFrameMs', 'queryMs', 'encodeMs', 'pipeOverheadMs', 'httpOverheadMs',
        'decodeMs', 'drawMs', 'startDelayMs', 'frameWaitMs', 'preDrawWaitMs', 'postDrawWaitMs']
        .map(key => [key, p95(interactions.map(frame => frame[key]))])),
      byInput: Object.fromEntries(['pan', 'zoom'].map(kind =>
        [kind, p95(interactions.filter(frame => frame.kind === kind).map(frame => frame.endToFrameMs))])),
      maxBytes: Math.max(...interactions.map(frame => frame.bytes)),
    };
    console.log(name, i + 1, drawMode, JSON.stringify({
      firstMs: first.endToFrameMs, previewP95Ms, previewCount: previews.length,
      ...summary.p95, maxBytes: summary.maxBytes, foreground, pageFocus, cadenceBefore, cadenceAfter,
    }));
    return summary;
  } finally {
    try {
      await windowMonitor?.stop();
    } finally {
      try {
        await browser?.close();
      } finally {
        host.stdin.end();
        await closed;
      }
    }
  }
}

const mode = process.argv[3] ?? 'all';
assert(['web', 'qt', 'all', 'compare'].includes(mode));
for (const [name, wave] of [['engine', 'native.wave'], ['csv', 'samples.wave']]) {
  const file = join(fixture, wave);
  const browserResults = join(fixture, `${name}-browser.json`);
  if (mode === 'compare') {
    const modes = ['baseline', 'scheduled', 'direct'];
    const comparisons = [];
    for (let i = 0; i < 3; ++i) {
      for (let j = 0; j < modes.length; ++j)
        comparisons.push(await trial(file, name, i, modes[(i + j) % modes.length]));
    }
    await writeFile(join(fixture, `${name}-comparison.json`), JSON.stringify(comparisons, null, 2));
    continue;
  }
  if (mode !== 'qt') {
    const web = [];
    for (let i = 0; i < 3; ++i) web.push(await trial(file, name, i));
    await writeFile(browserResults, JSON.stringify(web, null, 2));
  }
  if (mode === 'web') continue;
  const web = JSON.parse(await readFile(browserResults, 'utf8'));
  for (const [i, sample] of web.entries()) {
    const ranges = join(fixture, 'ranges.txt');
    await writeFile(ranges, sample.interactions.map(frame => frame.bounds.join(' ')).join('\n') + '\n');
    await warm(file);
    const qt = execFileSync(viewer, ['--benchmark', file, ranges], {
      env: { ...process.env, QT_QPA_PLATFORM: 'offscreen' },
      encoding: 'utf8',
    });
    console.log(name, i + 1, qt.trim());
  }
}
