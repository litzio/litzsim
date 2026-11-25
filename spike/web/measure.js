async page => {
  /* CONFIG */
  const editor = page.getByRole('region', { name: 'LitzSim editor' });
  const button = name => editor.getByRole('button', { name, exact: true });
  const svg = editor.getByRole('group', { name: 'Circuit drawing' });
  const revision = () => svg.getAttribute('data-revision');
  const measurements = editor.locator('[data-litzsim-metrics]');
  const metric = key => measurements.getAttribute('data-' + key);
  const reset = key => measurements.evaluate((node, key) => node.removeAttribute('data-' + key), key);
  const waitForMetric = key => page.waitForFunction(key =>
    document.querySelector('[data-litzsim-metrics]')?.hasAttribute('data-' + key), key);
  const wait = ms => page.waitForTimeout(ms);
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  const report = {};
  let stage;
  const open = async name => {
    await button('Open circuit').click();
    await page.getByRole(config.host ? 'option' : 'button', { name, exact: true }).click();
    if (config.host) await page.getByRole('dialog').getByRole('button', { name: /^(Open|Select|Choose|열기|선택)$/ }).click();
    await editor.getByText(name, { exact: true }).waitFor();
    await page.waitForFunction(() => ![...document.querySelectorAll('button')].find(b => b.textContent === 'Open circuit')?.disabled);
  };
  const checkEdit = async input => {
    const before = await revision();
    await input();
    await page.waitForFunction(before => document.querySelector('[data-revision]')?.dataset.revision !== before, before);
  };
  const pin = id => svg.locator('[data-pin="' + id + '"]');
  const part = id => svg.locator('[data-part="' + id + '"]');
  async function dragPart(id, dx, dy) {
    const points = await part(id).evaluate((node, [dx, dy]) => {
      const center = new DOMPoint(+node.getAttribute('x') + 12, +node.getAttribute('y') + 12);
      const matrix = node.ownerSVGElement.getScreenCTM();
      return [center.matrixTransform(matrix), new DOMPoint(center.x + dx * 32, center.y + dy * 32).matrixTransform(matrix)]
        .map(({ x, y }) => ({ x, y }));
    }, [dx, dy]);
    await page.mouse.move(points[0].x, points[0].y);
    await page.mouse.down();
    await page.mouse.move(points[1].x, points[1].y, { steps: 5 });
    await page.mouse.up();
  }
  async function connect(a, b) {
    await pin(a).click();
    await wait(120);
    await pin(b).click();
  }
  function quantile(values, q = 0.95) {
    const sorted = [...values].sort((a, b) => a - b);
    return Math.round(sorted[Math.ceil(q * sorted.length) - 1] * 100) / 100;
  }
  if (config.benchmark) {
    await open('rc.litzsim');
    await checkEdit(() => button('Add part').click());
    const baseline = await revision();
    for (const kind of ['move', 'connect']) {
      const samples = [];
      for (let i = 0; i < config.count; ++i) {
        const last = await metric('edit');
        await checkEdit(() => kind === 'move' ? dragPart('resistor1', 1, 0) : connect('resistor1.p', 'R1.n'));
        await page.waitForFunction(({ last, kind }) => {
          const next = document.querySelector('[data-litzsim-metrics]')?.getAttribute('data-edit');
          return next && next !== last && JSON.parse(next).kind === kind;
        }, { last, kind });
        const timing = JSON.parse(await metric('edit'));
        check(timing.kind === kind, 'Unexpected edit timing.');
        samples.push({ ...timing, transportMs: timing.bridgeMs - timing.handlerMs,
          processOtherMs: timing.cliMs - timing.applyMs - timing.layoutMs });
        await wait(130);
        await checkEdit(() => button('Undo edit').click());
        check(await revision() === baseline, 'Undo did not restore the original revision.');
        await wait(130);
      }
      report[kind] = {
        n: samples.length,
        p95: Object.fromEntries(Object.keys(samples[0]).filter(key => key !== 'kind')
          .map(key => [key, quantile(samples.map(s => s[key]))])),
      };
    }
  }
  if (config.flow) for (const name of config.circuits ?? ['rc', 'buck']) {
    try { await workflow(name); }
    catch (e) { throw new Error(name + ' / ' + stage + ': ' + e.message); }
  }
  return report;

  async function workflow(name) {
    stage = 'open ' + name;
    await open(name + '.litzsim');
    await checkEdit(() => button('Add part').click());
    stage = 'move added resistor';
    await checkEdit(() => dragPart('resistor1', 2, 0));
    stage = 'rotate';
    await checkEdit(() => button('Rotate').click());
    await editor.getByRole('spinbutton', { name: 'resistance_ohm' }).fill('2200');
    await checkEdit(() => button('Set resistance_ohm').click());
    await checkEdit(() => connect('resistor1.p', name === 'rc' ? 'R1.n' : 'L1.n'));
    await checkEdit(() => connect('resistor1.n', 'GND.ref'));
    if (config.capture && name === 'buck') await editor.screenshot({ path: 'spike-edit.png' });
    stage = 'save';
    await button('Save').click();
    await editor.getByText('Saved', { exact: true }).waitFor();
    const saved = await revision();
    stage = 'open ' + name;
    await open(name + '.litzsim');
    check(await revision() === saved, 'Save / reopen changed the circuit revision.');
    await button('Run simulation').click();
    await reset('first');
    stage = 'wave first display';
    await button('View waveform').click();
    const wave = editor.locator('canvas');
    await wave.waitFor();
    await waitForMetric('first');
    const first = Number(await metric('first'));
    if (config.capture && name === 'buck') {
      await editor.screenshot({ path: 'spike-wave.png' });
    }
    const zoomSamples = [], panSamples = [];
    for (let i = 0; i < 20; ++i) {
      await button('Fit waveform').click();
      stage = 'zoom sample ' + i;
      await reset('zoom');
      await button('Zoom in').click();
      await waitForMetric('zoom');
      zoomSamples.push(Number(await metric('zoom')));
      stage = 'pan sample ' + i;
      await reset('pan');
      const r = await wave.boundingBox();
      await page.mouse.move(r.x + r.width * 0.55, r.y + 100);
      await page.mouse.down();
      await page.mouse.move(r.x + r.width * 0.45, r.y + 100);
      await page.mouse.up();
      await waitForMetric('pan');
      panSamples.push(Number(await metric('pan')));
      await wait(110);
    }
    stage = 'open long circuit';
    await open(name + '-long.litzsim');
    const began = Date.now();
    await button('Run simulation').click();
    const run = editor.locator('[data-run]');
    await editor.getByText('running', { exact: true }).waitFor();
    const runId = await run.getAttribute('data-run');
    const start = Number(await metric('start'));
    await part('R1').click();
    stage = 'rotate';
    await checkEdit(() => button('Rotate').click());
    await editor.getByText('running', { exact: true }).waitFor();
    if (config.capture && name === 'buck') await editor.screenshot({ path: 'spike-run.png' });
    stage = 'run for more than 10 seconds';
    await wait(Math.max(0, 11200 - (Date.now() - began)));
    check((await run.textContent()) === 'running', 'Long run ended before the stop check.');
    const status = Number(await metric('status'));
    const runtimeMs = Date.now() - began - start;
    stage = 'stop';
    await button('Stop run').click();
    await editor.getByText('canceled', { exact: true }).waitFor();
    const stop = Number(await metric('stop'));
    check(start < 10000 && status < 10000 && stop < 10000, 'Run RPC exceeded the issue deadline.');
    report[name] = { saved, first, panP95: quantile(panSamples), zoomP95: quantile(zoomSamples), samplesPerGesture: panSamples.length,
      start, status, stop, runtimeMs, runId };
  }
}
