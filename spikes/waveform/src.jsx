import React, { useEffect, useRef } from 'react';
import { createRoot } from 'react-dom/client';
import './style.css';

const channels = [1, 2, 3, 4];
const xy = [2, 3];
const left = 76;
const right = 36;
const colors = ['#604286', '#12786f', '#aa4d20', '#3069a0', '#604286'];

async function query(request) {
  const start = performance.now();
  const response = await fetch('/api/wave', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  });
  const bytes = await response.arrayBuffer();
  const received = performance.now();
  const result = JSON.parse(new TextDecoder().decode(bytes));
  const decodeMs = performance.now() - received;
  if (!response.ok || result.error) throw new Error(result.error ?? response.statusText);
  const timing = Object.fromEntries(response.headers.get('Server-Timing').split(',').map((part) => {
    const [name, duration] = part.trim().split(';dur=');
    return [name + 'Ms', Number(duration)];
  }));
  return { result, timing: { ...timing, fetchMs: received - start, decodeMs, bytes: bytes.byteLength } };
}

function extent(points, column) {
  let lo = Infinity;
  let hi = -Infinity;
  for (const point of points) {
    lo = Math.min(lo, point[column]);
    hi = Math.max(hi, point[column]);
  }
  if (!points.length) return [0, 1];
  const pad = lo === hi ? Math.max(1e-9, Math.abs(lo) * 0.05) : 0;
  return [lo - pad, hi + pad];
}

function draw(canvas, points, x, title, xLabel, color) {
  const { clientWidth: width, clientHeight: height } = canvas;
  const dpr = window.devicePixelRatio;
  if (canvas.width !== Math.round(width * dpr) || canvas.height !== Math.round(height * dpr)) {
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
  }
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);
  const [x0, x1] = x;
  const [y0, y1] = extent(points, 2);
  const top = 22;
  const bottom = height - 25;
  const plotWidth = width - left - right;
  const plotHeight = bottom - top;
  const label = value => Number(value.toPrecision(4)).toString();
  ctx.font = '12px system-ui';
  ctx.fillStyle = color;
  ctx.textAlign = 'left';
  ctx.fillText(title, left, 15);
  ctx.textAlign = 'right';
  ctx.fillText(xLabel, width - right, 15);
  ctx.strokeStyle = '#e0e3e9';
  ctx.fillStyle = '#505765';
  ctx.beginPath();
  for (let i = 0; i <= 4; ++i) {
    const px = left + plotWidth * i / 4;
    const py = bottom - plotHeight * i / 4;
    ctx.moveTo(px, top);
    ctx.lineTo(px, bottom);
    ctx.moveTo(left, py);
    ctx.lineTo(width - right, py);
    ctx.textAlign = 'right';
    ctx.fillText(label(y0 + (y1 - y0) * i / 4), left - 7, py + 4);
    ctx.textAlign = 'center';
    ctx.fillText(label(x0 + (x1 - x0) * i / 4), px, bottom + 16);
  }
  ctx.stroke();
  ctx.save();
  ctx.beginPath();
  ctx.rect(left - 1, top - 1, plotWidth + 2, plotHeight + 2);
  ctx.clip();
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  points.forEach(([, xValue, yValue], i) => {
    const px = left + (xValue - x0) / (x1 - x0) * plotWidth;
    const py = bottom - (yValue - y0) / (y1 - y0) * plotHeight;
    if (i === 0) ctx.moveTo(px, py);
    else ctx.lineTo(px, py);
  });
  ctx.stroke();
  ctx.restore();
}

function frame() {
  return new Promise(resolve => requestAnimationFrame(() => resolve(performance.now())));
}

function Waveform() {
  const container = useRef(null);
  const status = useRef(null);
  useEffect(() => {
    const canvases = [...container.current.querySelectorAll('canvas')];
    const timeArea = canvases[0].parentElement;
    const metrics = new URLSearchParams(location.search).has('measure')
      ? (window.waveMeasurement = { frames: [] }) : null;
    let metadata;
    let range;
    let revision = 0;
    let previousX;
    const pixels = () => Math.min(4096, Math.max(1, canvases[0].clientWidth - left - right));
    const report = error => { status.current.textContent = error.message; };

    async function refresh(inputAt, kind) {
      const current = ++revision;
      const bounds = range;
      const requestAt = performance.now();
      const request = { op: 'viewport', from: bounds[0], to: bounds[1], pixels: pixels(), channels };
      const { result, timing } = await query(request);
      const drawAt = await frame();
      if (current !== revision) return;
      result.plots.forEach((points, i) =>
        draw(canvases[i], points, bounds, metadata.channels[channels[i]], metadata.channels[0], colors[i]));
      const drawMs = performance.now() - drawAt;
      // Draw in one animation callback, then sample the next rendering opportunity.
      const presentedAt = await frame();
      if (metrics) {
        metrics.frames.push({ kind, bounds, pixels: request.pixels, ...timing, drawMs,
          startDelayMs: requestAt - inputAt, endToFrameMs: presentedAt - inputAt });
        if (kind === 'first') metrics.firstPlots = result.plots;
      }
    }

    async function start() {
      const opened = await query({ op: 'open' });
      metadata = opened.result;
      range = metadata.domain;
      if (range[0] === range[1]) range = [range[0] - 1, range[1] + 1];
      if (metrics) metrics.open = { ...opened.timing, mapMs: metadata.openMs, rows: metadata.rows };
      await refresh(0, 'first');
      const begin = performance.now();
      const { result, timing } = await query({
        op: 'viewport', from: metadata.domain[0], to: metadata.domain[1], pixels: pixels(), channels: [1], xy,
      });
      const drawAt = await frame();
      draw(canvases[4], result.xy, extent(result.xy, 1),
        `XY · ${metadata.channels[xy[1]]}`, metadata.channels[xy[0]], colors[4]);
      const drawMs = performance.now() - drawAt;
      const presentedAt = await frame();
      status.current.textContent = `${metadata.rows.toLocaleString()} samples`;
      if (metrics) {
        metrics.xy = result.xy;
        metrics.xyTiming = { ...timing, drawMs, endToFrameMs: presentedAt - begin };
      }
    }

    function wheel(event) {
      if (!range) return;
      event.preventDefault();
      const fraction = (event.clientX - timeArea.getBoundingClientRect().left - left) / pixels();
      const anchor = range[0] + fraction * (range[1] - range[0]);
      const scale = event.deltaY < 0 ? 0.8 : 1.25;
      range = range.map(limit => anchor + (limit - anchor) * scale);
      refresh(event.timeStamp, 'zoom').catch(report);
    }
    function pointerdown(event) {
      if (event.button !== 0 || !range) return;
      previousX = event.clientX;
      timeArea.setPointerCapture(event.pointerId);
    }
    function pointermove(event) {
      if (previousX === undefined) return;
      const delta = (previousX - event.clientX) * (range[1] - range[0]) / pixels();
      previousX = event.clientX;
      if (delta === 0) return;
      range = range.map(limit => limit + delta);
      refresh(event.timeStamp, 'pan').catch(report);
    }
    function endDrag() { previousX = undefined; }
    timeArea.addEventListener('wheel', wheel, { passive: false });
    timeArea.addEventListener('pointerdown', pointerdown);
    timeArea.addEventListener('pointermove', pointermove);
    timeArea.addEventListener('lostpointercapture', endDrag);
    start().catch(report);
    return () => {
      timeArea.removeEventListener('wheel', wheel);
      timeArea.removeEventListener('pointerdown', pointerdown);
      timeArea.removeEventListener('pointermove', pointermove);
      timeArea.removeEventListener('lostpointercapture', endDrag);
    };
  }, []);
  return <main ref={container}>
    <header><h1>LitzSim 파형</h1><output ref={status}>파형 읽는 중</output></header>
    <section aria-label="시간 파형">
      {channels.map(y => <canvas key={y} aria-label={`시간 plot ${y}`} />)}
    </section>
    <canvas aria-label="XY plot" />
  </main>;
}

createRoot(document.getElementById('root')).render(<Waveform />);
