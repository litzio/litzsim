const buttonBar = { display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' };

export function Editor({ React, adapter }) {
  const { useEffect, useLayoutEffect, useRef, useState } = React;
  const [file, setFile] = useState(null);
  const [sheet, setSheet] = useState(null);
  const [diskRevision, setDiskRevision] = useState(null);
  const [history, setHistory] = useState([]);
  const [selected, setSelected] = useState('');
  const [terminal, setTerminal] = useState('');
  const [partType, setPartType] = useState('resistor');
  const [draft, setDraft] = useState({});
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [run, setRun] = useState(null);
  const [wave, setWave] = useState(null);
  const [channel, setChannel] = useState(1);
  const [view, setView] = useState(null);
  const [notice, setNotice] = useState('');
  const drawing = useRef(null);
  const canvas = useRef(null);
  const metrics = useRef(null);
  const pending = useRef(null);
  const drag = useRef(null);
  const pan = useRef(null);
  const locked = useRef(false);
  const waveInput = useRef(null);

  async function perform(task) {
    if (locked.current) return;
    locked.current = true;
    setBusy(true);
    setError('');
    try { await task(); }
    catch (e) { setError(e.message); }
    finally {
      locked.current = false;
      setBusy(false);
    }
  }

  function measureEdit(kind, start, response, bridgeMs) {
    pending.current = { kind, start, response, bridgeMs, received: performance.now() };
  }

  useLayoutEffect(() => {
    const sample = pending.current;
    if (!sample || !metrics.current) return;
    pending.current = null;
    const first = requestAnimationFrame(() => {
      const frame = performance.now();
      requestAnimationFrame(() => {
        if (!metrics.current) return;
        metrics.current.dataset.edit = JSON.stringify({
          kind: sample.kind, total: frame - sample.start, drawMs: frame - sample.received,
          bridgeMs: sample.bridgeMs, ...sample.response.timing,
        });
      });
    });
    return () => cancelAnimationFrame(first);
  }, [sheet]);

  async function edit(operation, started = performance.now()) {
    await perform(async () => {
      const began = performance.now();
      const next = await adapter.call('apply', {
        document: sheet.document, expectedRevision: sheet.revision, operations: [operation],
      });
      measureEdit(operation.action, started, next, performance.now() - began);
      setHistory([...history, { document: sheet.document, after: next.revision }]);
      setSheet(next);
      if (next.receipt.created.length) setSelected(next.receipt.created[0]);
      setNotice('');
    });
  }

  function open() {
    perform(async () => {
      const nextFile = await adapter.pickCircuit();
      if (!nextFile) return;
      const nextSheet = await adapter.call('open', { file: nextFile });
      setFile(nextFile);
      setSheet(nextSheet);
      setDiskRevision(nextSheet.diskRevision);
      setSelected('');
      setTerminal('');
      setHistory([]);
      setRun(null);
      setWave(null);
      setNotice('');
    });
  }

  function undo() {
    perform(async () => {
      const last = history.at(-1);
      if (sheet.revision !== last.after) throw new Error('The undo snapshot is stale.');
      const next = await adapter.call('undo', { expectedRevision: last.after, current: { revision: sheet.revision }, previous: last.document });
      setSheet(next);
      setHistory(history.slice(0, -1));
      setSelected('');
      setTerminal('');
    });
  }

  function save() {
    perform(async () => {
      const written = await adapter.call('save', { file, document: sheet.document, diskRevision });
      setDiskRevision(written.diskRevision);
      setNotice('Saved');
    });
  }

  async function runCall(action, input) {
    const started = performance.now();
    const result = await adapter.call(action, input);
    if (metrics.current) metrics.current.dataset[action] = String(performance.now() - started);
    return result;
  }

  useEffect(() => {
    if (run?.state !== 'running') return;
    let disposed = false;
    const poll = async () => {
      try {
        const next = await runCall('status', { id: run.id });
        if (!disposed && next.state !== 'running')
          setRun(current => current?.id === next.id && current.state === 'running' ? next : current);
      } catch (e) { if (!disposed) setError(e.message); }
    };
    const timer = setInterval(poll, 350);
    return () => {
      disposed = true;
      clearInterval(timer);
    };
  }, [run?.id, run?.state]);

  function loadWave(nextChannel) {
    perform(async () => {
      const started = performance.now();
      const next = await adapter.call('wave', { id: run.id, channel: nextChannel });
      setChannel(nextChannel);
      waveInput.current = { kind: 'first', started };
      setWave(next);
      setView([next.from, next.to]);
    });
  }

  useLayoutEffect(() => {
    if (!wave || !view || !canvas.current) return;
    drawWave(canvas.current, wave, channel, view);
    const input = waveInput.current;
    waveInput.current = null;
    if (!input) return;
    const first = requestAnimationFrame(() => {
      const frame = performance.now();
      requestAnimationFrame(() => {
        if (metrics.current) metrics.current.dataset[input.kind] = String(frame - input.started);
      });
    });
    return () => cancelAnimationFrame(first);
  }, [wave, channel, view]);

  function zoom(factor) {
    const mid = (view[0] + view[1]) / 2;
    const half = (view[1] - view[0]) * factor / 2;
    waveInput.current = { kind: 'zoom', started: performance.now() };
    setView(clampView(wave, mid - half, mid + half));
  }

  function moveWave(event) {
    if (!pan.current) return;
    const { left, right, x } = pan.current;
    const delta = (x - event.clientX) * (right - left) / event.currentTarget.clientWidth;
    waveInput.current = { kind: 'pan', started: performance.now() };
    setView(clampView(wave, left + delta, right + delta));
  }

  const part = sheet?.document.parts[selected];
  useEffect(() => { setDraft(part?.parameters ?? {}); }, [part]);
  const layout = sheet?.layout;
  function drop(event) {
    const began = performance.now();
    const active = drag.current;
    drag.current = null;
    if (!active || Math.hypot(event.clientX - active.x, event.clientY - active.y) < 4) return;
    const point = new DOMPoint(event.clientX, event.clientY).matrixTransform(drawing.current.getScreenCTM().inverse());
    edit({ action: 'move', id: active.id, x: Math.round(point.x / layout.pitch), y: Math.round(point.y / layout.pitch) }, began);
  }

  function selectPin(id) {
    const began = performance.now();
    if (terminal && terminal !== id) {
      setTerminal('');
      edit({ action: 'connect', a: terminal, b: id }, began);
    } else setTerminal(terminal ? '' : id);
  }

  return <section aria-label="LitzSim editor" style={{ padding: 16, display: 'grid', gap: 12 }}>
    <span ref={metrics} hidden data-litzsim-metrics="" />
    <div style={buttonBar}>
      <button onClick={open} disabled={busy || run?.state === 'running'}>Open circuit</button>
      <button onClick={save} disabled={busy || !sheet}>Save</button>
      <button onClick={undo} disabled={busy || !history.length}>Undo edit</button>
      <strong>{file?.split(/[\\/]/).at(-1)}</strong>
      <span role="status">{notice}</span>
    </div>
    {error && <p role="alert">{error}</p>}
    {sheet && <>
      <div style={buttonBar}>
        <label>Part type <select value={partType} onChange={e => setPartType(e.target.value)}>
          {sheet.types.map(type => <option key={type}>{type}</option>)}
        </select></label>
        <button disabled={busy} onClick={() => {
          const [x, y, width, height] = layout.viewBox;
          edit({ action: 'add', type: partType, x: Math.round((x + width / 2) / layout.pitch),
            y: Math.round((y + height - 40) / layout.pitch) });
        }}>Add part</button>
        <button disabled={busy || !part} onClick={() => edit({ action: 'rotate', id: selected })}>Rotate</button>
        {part && <strong>{selected}</strong>}
        {part && Object.entries(draft).map(([key, value]) => <form key={key} style={buttonBar} onSubmit={event => {
          event.preventDefault();
          edit({ action: 'setParameter', id: selected, key, value: Number(value) });
        }}>
          <label>{key} <input required type="number" step="any" style={{ width: 96 }}
            value={value} onChange={e => setDraft({ ...draft, [key]: e.target.value })} /></label>
          <button disabled={busy}>Set {key}</button>
        </form>)}
        {terminal && <span>Connect {terminal} to…</span>}
      </div>
      <svg ref={drawing} role="group" aria-label="Circuit drawing" viewBox={layout.viewBox.join(' ')}
        data-revision={sheet.revision} onPointerUp={drop} onPointerCancel={() => { drag.current = null; }}
        style={{ width: '100%', height: 'min(38vh, 370px)', border: '1px solid #cdd7d5', background: '#fbfcfa', touchAction: 'none' }}>
        {layout.shapes.map((shape, index) => shape.d !== undefined
          ? <path key={index} d={shape.d} stroke={shape.id && shape.id === selected ? '#007d72' : shape.stroke}
              strokeWidth={shape.width} strokeLinecap="round" strokeLinejoin="round" fill={shape.fill} />
          : <text key={index} x={shape.x} y={shape.y} fontSize={shape.size} fill={shape.fill}
              fontWeight={shape.bold ? 700 : 400}>{shape.text}</text>)}
        {Object.entries(layout.centers).map(([id, [x, y]]) =>
          <rect key={id} role="button" aria-label={id} data-part={id} x={x - 12} y={y - 12} width="24" height="24"
            fill="transparent" tabIndex={0} style={{ cursor: 'grab' }} onKeyDown={e => {
              if (e.key === 'Enter') setSelected(id);
            }} onPointerDown={event => {
              if (busy || event.button !== 0) return;
              event.currentTarget.setPointerCapture(event.pointerId);
              drag.current = { id, x: event.clientX, y: event.clientY };
              setSelected(id);
            }} />)}
        {layout.pins.map(pin => <circle key={pin.id} role="button" aria-label={pin.id} tabIndex={0}
          data-pin={pin.id} cx={pin.x} cy={pin.y} r="5" stroke="#168c80" strokeWidth="1.5"
          fill={terminal === pin.id ? '#168c80' : '#fbfcfa'} style={{ cursor: 'crosshair' }}
          onClick={() => { if (!busy) selectPin(pin.id); }} onKeyDown={e => {
            if (!busy && e.key === 'Enter') selectPin(pin.id);
          }}><title>{pin.id}</title></circle>)}
      </svg>
      <div style={buttonBar}>
        <button disabled={busy || run?.state === 'running'} onClick={() => perform(async () => {
          const next = await runCall('start', { file, document: sheet.document });
          setRun(next);
          setWave(null);
          setChannel(1);
        })}>Run simulation</button>
        <button disabled={run?.state !== 'running'} onClick={() => perform(async () => {
          setRun(await runCall('stop', { id: run.id }));
        })}>Stop run</button>
        <span aria-live="polite" data-run={run?.id}>{run?.state}</span>
        {run?.state === 'completed' && <button disabled={busy} onClick={() => loadWave(1)}>View waveform</button>}
      </div>
    </>}
    {wave && <>
      <div style={buttonBar}>
        <label>Signal <select value={channel} onChange={e => loadWave(Number(e.target.value))}>
          {wave.channels.slice(1).map((name, i) => <option key={name} value={i + 1}>{name}</option>)}
        </select></label>
        <button onClick={() => zoom(0.5)}>Zoom in</button>
        <button onClick={() => zoom(2)}>Zoom out</button>
        <button onClick={() => setView([wave.from, wave.to])}>Fit waveform</button>
      </div>
      <canvas ref={canvas} width="1000" height="240" aria-label="Waveform plot"
        style={{ width: '100%', height: 240, border: '1px solid #cdd7d5', cursor: 'grab', touchAction: 'none' }}
        onPointerDown={event => {
          event.currentTarget.setPointerCapture(event.pointerId);
          pan.current = { left: view[0], right: view[1], x: event.clientX };
        }} onPointerMove={moveWave} onPointerUp={() => { pan.current = null; }}
        onPointerCancel={() => { pan.current = null; }} />
    </>}
  </section>;
}

function clampView(wave, left, right) {
  const span = right - left;
  if (span >= wave.to - wave.from) return [wave.from, wave.to];
  if (span < (wave.to - wave.from) / 1024) return [left + span / 2 - (wave.to - wave.from) / 2048,
    right - span / 2 + (wave.to - wave.from) / 2048];
  const from = Math.max(wave.from, Math.min(wave.to - span, left));
  return [from, from + span];
}

function drawWave(canvas, wave, channel, [from, to]) {
  const context = canvas.getContext('2d');
  const left = 74, right = canvas.width - 25, top = 34, bottom = canvas.height - 38;
  let min = Infinity, max = -Infinity;
  for (const [, y] of wave.points) {
    min = Math.min(min, y);
    max = Math.max(max, y);
  }
  if (min === max) { min -= 1; max += 1; }
  context.fillStyle = '#fbfcfa';
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.font = '15px system-ui';
  context.fillStyle = '#182622';
  context.textAlign = 'left';
  context.fillText(wave.channels[channel], left, 21);
  for (let n = 0; n <= 4; ++n) {
    const fraction = n / 4;
    const x = left + (right - left) * fraction;
    const y = bottom - (bottom - top) * fraction;
    context.fillStyle = '#dce5e0';
    context.fillRect(left, y, right - left, 1);
    context.fillRect(x, top, 1, bottom - top);
    context.fillStyle = '#182622';
    context.textAlign = 'left';
    context.fillText((min + (max - min) * fraction).toPrecision(3), 5, y + 4);
    context.textAlign = n === 4 ? 'right' : 'center';
    context.fillText((from + (to - from) * fraction).toExponential(2), x, bottom + 21);
  }
  context.textAlign = 'center';
  context.fillText('Time [s]', (right + left) / 2, canvas.height - 2);
  context.save();
  context.beginPath();
  context.rect(left, top, right - left, bottom - top);
  context.clip();
  context.beginPath();
  wave.points.forEach(([time, value], index) => {
    const x = left + (time - from) / ((to - from) || 1) * (right - left);
    const y = bottom - (value - min) / (max - min) * (bottom - top);
    if (index === 0) context.moveTo(x, y);
    else context.lineTo(x, y);
  });
  context.strokeStyle = '#007d72';
  context.lineWidth = 2;
  context.stroke();
  context.restore();
}

export default function Screen({ React, invoke, pickPath }) {
  const adapter = {
    call: (name, input) => invoke(name, input),
    pickCircuit: async () => (await pickPath({ kind: 'file', extensions: ['.litzsim'] }))?.path,
  };
  return <Editor React={React} adapter={adapter} />;
}
