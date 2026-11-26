import React from "react";

// Publication decision: the sheet is white paper with black ink in every theme.
// These two values are the documented exception to Litzio color tokens.
const INK = "#000";
const PAPER = "#fff";
const ACCENT = "var(--id-color-accent)";
const WARNING = "var(--id-color-warning)";
const LINE = 2;
// Labels use the Litzio body face; the core sizes each label box (docs/layout.md).
const SANS = { fontFamily: "var(--font-family-sans)" };

const join = points => points.map(([x, y]) => `${x},${y}`).join(" ");

function Stroke({ stroke }) {
  switch (stroke.kind) {
    case "polyline":
      return <polyline points={join(stroke.points)} fill="none" />;
    case "filledTriangle":
      return <polygon points={join(stroke.points)} fill={INK} />;
    case "circle":
      return <circle cx={stroke.center[0]} cy={stroke.center[1]} r={stroke.radius} fill={stroke.filled ? INK : "none"} />;
    case "upperArc": {
      const [[x, y], [rx, ry]] = [stroke.center, stroke.radii];
      return <path d={`M${x - rx} ${y}A${rx} ${ry} 0 0 1 ${x + rx} ${y}`} fill="none" />;
    }
    case "cubic": {
      const [a, b, c, d] = stroke.points;
      return <path d={`M${a}C${b} ${c} ${d}`.replaceAll(",", " ")} fill="none" />;
    }
    default:
      return null;
  }
}

/** Core strokes are centered at the origin: rotate, mirror on sheet axes, then translate. */
function symbolTransform(symbol) {
  const sx = symbol.mirror === "horizontal" ? -1 : 1;
  const sy = symbol.mirror === "vertical" ? -1 : 1;
  return `translate(${symbol.center[0]} ${symbol.center[1]}) scale(${sx} ${sy}) rotate(${symbol.rotation})`;
}

function Symbol({ symbol }) {
  const arrow = symbol.currentArrow;
  return <g>
    <g transform={symbolTransform(symbol)}>
      {symbol.strokes.map((stroke, index) => <Stroke key={index} stroke={stroke} />)}
    </g>
    {arrow && <g fill="none">
      <line x1={arrow.from[0]} y1={arrow.from[1]} x2={arrow.to[0]} y2={arrow.to[1]} />
      <polyline points={join([arrow.head[0], arrow.to, arrow.head[1]])} />
    </g>}
    {symbol.meterMark && <text x={symbol.meterMark.at[0]} y={symbol.meterMark.at[1]} fontSize={14}
      textAnchor="middle" dominantBaseline="central" fill={INK} stroke="none" style={SANS}>{symbol.meterMark.text}</text>}
  </g>;
}

/** Horizontal wires hop over the crossings the core reported as jumps. */
function wirePath(segment, jumps) {
  const [[x1, y1], [x2, y2]] = [segment.from, segment.to];
  if (y1 !== y2) return `M${x1} ${y1}L${x2} ${y2}`;
  const [low, high] = [Math.min(x1, x2), Math.max(x1, x2)];
  let path = `M${low} ${y1}`;
  for (const jump of jumps) {
    const [x, y] = jump.at;
    if (jump.horizontalNetId === segment.netId && y === y1 && x > low && x < high) {
      path += `L${x - jump.radius} ${y}A${jump.radius} ${jump.radius} 0 0 1 ${x + jump.radius} ${y}`;
    }
  }
  return `${path}L${high} ${y1}`;
}

function Label({ label }) {
  const { min, max } = label.bounds;
  const { fontSize, bold, padding } = label.metrics;
  return <text x={(min[0] + max[0]) / 2} y={(min[1] + max[1]) / 2} fontSize={fontSize}
    fontWeight={bold ? 700 : 400} textAnchor="middle" dominantBaseline="central"
    fill={INK} stroke="none" data-label={label.id} data-room={max[0] - min[0] - 2 * padding}>{label.text}</text>;
}

/** Keep each label's ink inside the padding of its core box; squeeze only if a glyph run is wider. */
function fitLabels(group) {
  for (const text of group.querySelectorAll("text[data-room]")) {
    text.removeAttribute("textLength");
    text.removeAttribute("lengthAdjust");
    const room = Number(text.dataset.room);
    if (text.getComputedTextLength() > room) {
      text.setAttribute("textLength", room);
      text.setAttribute("lengthAdjust", "spacingAndGlyphs");
    }
  }
}

const boxProps = ({ min, max }, pad = 0) => ({
  x: min[0] - pad, y: min[1] - pad, width: max[0] - min[0] + 2 * pad, height: max[1] - min[1] + 2 * pad,
});

export function Schematic({
  layout, view, zoom, svgRef, selectedIds, pendingPin, hoverSegment, selectedWires, showGrid, marquee, preview, ghost, suggestion,
  onSymbolDown, onPinDown, onSegmentEnter, onSegmentDown, onBackgroundDown, onPointerMove, onPointerUp, onPointerCancel,
}) {
  const labels = React.useRef(null);
  React.useLayoutEffect(() => {
    const group = labels.current;
    if (!group) return undefined;
    fitLabels(group);
    let live = true;
    document.fonts?.ready.then(() => { if (live) fitLabels(group); });
    return () => { live = false; };
  });
  const moving = preview ? layout.symbols.filter(symbol => preview.ids.includes(symbol.id)) : [];
  const sorted = [...layout.jumps].sort((a, b) => a.at[0] - b.at[0]);
  const hovered = layout.segments[hoverSegment];
  return <svg ref={svgRef} role="group" aria-label="회로도" data-revision={layout.revision}
    viewBox={`${view.x} ${view.y} ${view.width} ${view.height}`} width={view.width * zoom} height={view.height * zoom}
    style={{ display: "block", background: PAPER, touchAction: "none", userSelect: "none" }}
    onPointerDown={onBackgroundDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerCancel}>
    <defs>
      <pattern id="litzsim-grid" width={32} height={32} patternUnits="userSpaceOnUse" x={0} y={0}>
        <circle cx={0} cy={0} r={1} fill="var(--id-color-border)" />
      </pattern>
    </defs>
    <rect {...boxProps({ min: [view.x, view.y], max: [view.x + view.width, view.y + view.height] })}
      fill={showGrid ? "url(#litzsim-grid)" : PAPER} data-background="" />

    {hovered && !selectedWires.has(hovered) && <path d={wirePath(hovered, sorted)}
      stroke={ACCENT} strokeWidth={8} strokeOpacity={0.35} fill="none" strokeLinecap="round" pointerEvents="none" />}
    {[...selectedWires].map((segment, index) => <path key={index} d={wirePath(segment, sorted)}
      stroke={ACCENT} strokeWidth={8} strokeOpacity={0.9} fill="none" strokeLinecap="round" pointerEvents="none" />)}
    {layout.symbols.filter(symbol => selectedIds.includes(symbol.id)).map(symbol =>
      <rect key={symbol.id} {...boxProps(symbol.bounds, 6)} fill="var(--id-color-surface-selected)" fillOpacity={0.5}
        stroke={ACCENT} strokeWidth={1.5} strokeDasharray="4 3" pointerEvents="none" />)}

    <g stroke={INK} strokeWidth={LINE} strokeLinecap="square" strokeLinejoin="miter" fill="none" pointerEvents="none">
      {layout.segments.map((segment, index) => <path key={index} d={wirePath(segment, sorted)} />)}
      {layout.symbols.map(symbol => <Symbol key={symbol.id} symbol={symbol} />)}
    </g>
    <g fill={INK} stroke="none" pointerEvents="none">
      {layout.junctions.map((junction, index) => <circle key={index} cx={junction.at[0]} cy={junction.at[1]} r={3.5} />)}
    </g>
    <g ref={labels} style={SANS} pointerEvents="none">
      {layout.labels.map(label => <Label key={label.id} label={label} />)}
    </g>
    <g fill={PAPER} stroke={INK} strokeWidth={1.5} pointerEvents="none">
      {Object.entries(layout.pins).filter(([, pin]) => pin.netId === null).map(([id, pin]) =>
        <circle key={id} cx={pin.at[0]} cy={pin.at[1]} r={3} />)}
    </g>
    <g fill="none" stroke={WARNING} strokeWidth={1.5} strokeDasharray="3 3" pointerEvents="none">
      {layout.issues.map((issue, index) => <rect key={index} data-issue={issue.code} {...boxProps(issue.area, 4)}>
        <title>{`${issue.code === "overlap" ? "겹침" : "배선 실패"}: ${issue.targets.map(target => target.id).join(", ")}`}</title>
      </rect>)}
    </g>

    {/* Hit targets, drawn transparent above the ink. */}
    <g fill="none" stroke="transparent" strokeWidth={12}>
      {layout.segments.map((segment, index) => <path key={index} data-segment={index} data-terminals={segment.terminals.join(" ")} data-net={segment.netId}
        data-selected={selectedWires.has(segment) || undefined} d={wirePath(segment, sorted)}
        pointerEvents="stroke" style={{ cursor: "pointer" }} onPointerEnter={() => onSegmentEnter(index)}
        onPointerDown={event => onSegmentDown(index, event)} />)}
    </g>
    <g fill="transparent">
      {layout.symbols.map(symbol => <rect key={symbol.id} data-part={symbol.id} data-selected={selectedIds.includes(symbol.id) || undefined}
        data-center={symbol.center.join(" ")} {...boxProps(symbol.bounds)}
        style={{ cursor: "move" }} onPointerDown={event => onSymbolDown(symbol.id, event)} />)}
    </g>
    <g>
      {Object.entries(layout.pins).map(([id, pin]) => <circle key={id} data-pin={id} cx={pin.at[0]} cy={pin.at[1]} r={7}
        fill={id === pendingPin ? ACCENT : "transparent"} fillOpacity={id === pendingPin ? 0.35 : 1}
        stroke={id === pendingPin ? ACCENT : "none"} strokeWidth={1.5} style={{ cursor: "crosshair" }}
        onPointerDown={event => onPinDown(id, event)}><title>{id}</title></circle>)}
    </g>

    {moving.length > 0 && <g transform={`translate(${preview.dx} ${preview.dy})`} opacity={0.55} pointerEvents="none"
      stroke={ACCENT} strokeWidth={LINE} fill="none">
      {moving.map(symbol => <g key={symbol.id}>
        <g transform={symbolTransform(symbol)}>{symbol.strokes.map((stroke, index) => <Stroke key={index} stroke={stroke} />)}</g>
        <rect {...boxProps(symbol.bounds, 6)} strokeWidth={1} strokeDasharray="4 3" />
      </g>)}
    </g>}
    {ghost && <g pointerEvents="none" stroke={ACCENT} fill="none" strokeDasharray="4 3">
      <rect x={ghost.at[0] - 32} y={ghost.at[1] - 32} width={64} height={64} />
      <text x={ghost.at[0]} y={ghost.at[1]} textAnchor="middle" dominantBaseline="central" stroke="none"
        fill={ACCENT} fontSize={13} style={SANS}>{ghost.label}</text>
    </g>}
    {suggestion && <g pointerEvents="none" stroke={ACCENT} fill="none">
      <line x1={suggestion.from[0]} y1={suggestion.from[1]} x2={suggestion.to[0]} y2={suggestion.to[1]}
        strokeWidth={2} strokeDasharray="5 4" />
      <circle cx={suggestion.from[0]} cy={suggestion.from[1]} r={7} strokeWidth={2} />
      <circle cx={suggestion.to[0]} cy={suggestion.to[1]} r={7} strokeWidth={2} />
    </g>}
    {marquee && <rect {...boxProps(marquee)} data-marquee="" fill={ACCENT} fillOpacity={0.08}
      stroke={ACCENT} strokeWidth={1.5} strokeDasharray="4 3" pointerEvents="none" />}
  </svg>;
}

/** Union of symbols, labels and wires in scene units, or null for an empty sheet. */
export function contentBounds(layout) {
  let box = null;
  const extend = ({ min, max }) => {
    box = box ? {
      min: [Math.min(box.min[0], min[0]), Math.min(box.min[1], min[1])],
      max: [Math.max(box.max[0], max[0]), Math.max(box.max[1], max[1])],
    } : { min, max };
  };
  layout.symbols.forEach(symbol => extend(symbol.bounds));
  layout.labels.forEach(label => extend(label.bounds));
  layout.segments.forEach(segment => [segment.from, segment.to].forEach(at => extend({ min: at, max: at })));
  return box;
}

/**
 * Scene rectangle covering the drawing plus a margin, never smaller than the
 * default sheet or than the pane at the current zoom (so any point can be centered).
 */
export function sheetView(layout, [paneWidth, paneHeight] = [0, 0]) {
  const grid = 32;
  let [x0, y0, x1, y1] = [-3 * grid, -3 * grid, 30 * grid, 18 * grid];
  const extend = ({ min, max }) => {
    x0 = Math.min(x0, min[0] - 4 * grid); y0 = Math.min(y0, min[1] - 4 * grid);
    x1 = Math.max(x1, max[0] + 4 * grid); y1 = Math.max(y1, max[1] + 4 * grid);
  };
  layout.symbols.forEach(symbol => extend(symbol.bounds));
  layout.labels.forEach(label => extend(label.bounds));
  layout.segments.forEach(segment => [segment.from, segment.to].forEach(at => extend({ min: at, max: at })));
  // Pad by a full pane on each side so fitting and wheel zoom can center any point.
  x0 -= paneWidth; x1 += paneWidth; y0 -= paneHeight; y1 += paneHeight;
  const snap = value => Math.floor(value / grid) * grid;
  return { x: snap(x0), y: snap(y0), width: snap(x1) - snap(x0) + grid, height: snap(y1) - snap(y0) + grid };
}
