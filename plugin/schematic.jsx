import React from "react";

// Publication decision: the sheet is white paper with black ink in every theme.
// These two values are the documented exception to Litzio color tokens.
const INK = "#000";
const PAPER = "#fff";
const ACCENT = "var(--id-color-accent)";
const WARNING = "var(--id-color-warning)";
const LINE = 2;

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
      textAnchor="middle" dominantBaseline="central" fill={INK} stroke="none">{symbol.meterMark.text}</text>}
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
  return <text x={(min[0] + max[0]) / 2} y={(min[1] + max[1]) / 2} fontSize={label.metrics.fontSize}
    fontWeight={label.metrics.bold ? 700 : 400} textAnchor="middle" dominantBaseline="central"
    fill={INK} stroke="none">{label.text}</text>;
}

const boxProps = ({ min, max }, pad = 0) => ({
  x: min[0] - pad, y: min[1] - pad, width: max[0] - min[0] + 2 * pad, height: max[1] - min[1] + 2 * pad,
});

export function Schematic({
  layout, view, svgRef, selectedId, pendingPin, hoverSegment, preview, ghost, suggestion,
  onSymbolDown, onPinDown, onSegmentEnter, onBackgroundDown, onPointerMove, onPointerUp, metrics,
}) {
  const jumps = layout.jumps;
  const moving = preview && layout.symbols.find(symbol => symbol.id === preview.id);
  const sorted = [...layout.jumps].sort((a, b) => a.at[0] - b.at[0]);
  return <svg ref={svgRef} role="group" aria-label="회로도" data-revision={layout.revision} data-edit={metrics}
    viewBox={`${view.x} ${view.y} ${view.width} ${view.height}`} width={view.width} height={view.height}
    style={{ display: "block", background: PAPER, touchAction: "none", userSelect: "none" }}
    onPointerDown={onBackgroundDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp}>
    <defs>
      <pattern id="litzsim-grid" width={32} height={32} patternUnits="userSpaceOnUse" x={0} y={0}>
        <circle cx={0} cy={0} r={1} fill="var(--id-color-border)" />
      </pattern>
    </defs>
    <rect {...boxProps({ min: [view.x, view.y], max: [view.x + view.width, view.y + view.height] })}
      fill="url(#litzsim-grid)" data-background="" />

    {hoverSegment !== null && layout.segments[hoverSegment] && <path d={wirePath(layout.segments[hoverSegment], jumps)}
      stroke={ACCENT} strokeWidth={8} strokeOpacity={0.35} fill="none" strokeLinecap="round" pointerEvents="none" />}
    {selectedId && layout.symbols.filter(symbol => symbol.id === selectedId).map(symbol =>
      <rect key={symbol.id} {...boxProps(symbol.bounds, 6)} fill="var(--id-color-surface-selected)" fillOpacity={0.5}
        stroke={ACCENT} strokeWidth={1.5} strokeDasharray="4 3" pointerEvents="none" />)}

    <g stroke={INK} strokeWidth={LINE} strokeLinecap="square" strokeLinejoin="miter" fill="none" pointerEvents="none">
      {layout.segments.map((segment, index) => <path key={index} d={wirePath(segment, sorted)} />)}
      {layout.symbols.map(symbol => <Symbol key={symbol.id} symbol={symbol} />)}
    </g>
    <g fill={INK} stroke="none" pointerEvents="none">
      {layout.junctions.map((junction, index) => <circle key={index} cx={junction.at[0]} cy={junction.at[1]} r={3.5} />)}
    </g>
    <g fontFamily="var(--font-family-mono)" pointerEvents="none">
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
      {layout.segments.map((segment, index) => <line key={index} data-segment={index} data-terminals={segment.terminals.join(" ")} data-net={segment.netId}
        x1={segment.from[0]} y1={segment.from[1]} x2={segment.to[0]} y2={segment.to[1]}
        pointerEvents="stroke" onPointerEnter={() => onSegmentEnter(index)} />)}
    </g>
    <g fill="transparent">
      {layout.symbols.map(symbol => <rect key={symbol.id} data-part={symbol.id} data-center={symbol.center.join(" ")} {...boxProps(symbol.bounds)}
        style={{ cursor: "move" }} onPointerDown={event => onSymbolDown(symbol.id, event)} />)}
    </g>
    <g>
      {Object.entries(layout.pins).map(([id, pin]) => <circle key={id} data-pin={id} cx={pin.at[0]} cy={pin.at[1]} r={7}
        fill={id === pendingPin ? ACCENT : "transparent"} fillOpacity={id === pendingPin ? 0.35 : 1}
        stroke={id === pendingPin ? ACCENT : "none"} strokeWidth={1.5} style={{ cursor: "crosshair" }}
        onPointerDown={event => onPinDown(id, event)}><title>{id}</title></circle>)}
    </g>

    {moving && <g transform={`translate(${preview.dx} ${preview.dy})`} opacity={0.55} pointerEvents="none"
      stroke={ACCENT} strokeWidth={LINE} fill="none">
      <g transform={symbolTransform(moving)}>{moving.strokes.map((stroke, index) => <Stroke key={index} stroke={stroke} />)}</g>
      <rect {...boxProps(moving.bounds, 6)} strokeWidth={1} strokeDasharray="4 3" />
    </g>}
    {ghost && <g pointerEvents="none" stroke={ACCENT} fill="none" strokeDasharray="4 3">
      <rect x={ghost.at[0] - 32} y={ghost.at[1] - 32} width={64} height={64} />
      <text x={ghost.at[0]} y={ghost.at[1]} textAnchor="middle" dominantBaseline="central" stroke="none"
        fill={ACCENT} fontSize={13}>{ghost.label}</text>
    </g>}
    {suggestion && <g pointerEvents="none" stroke={ACCENT} fill="none">
      <line x1={suggestion.from[0]} y1={suggestion.from[1]} x2={suggestion.to[0]} y2={suggestion.to[1]}
        strokeWidth={2} strokeDasharray="5 4" />
      <circle cx={suggestion.from[0]} cy={suggestion.from[1]} r={7} strokeWidth={2} />
      <circle cx={suggestion.to[0]} cy={suggestion.to[1]} r={7} strokeWidth={2} />
    </g>}
  </svg>;
}

/** Scene rectangle covering the drawing plus a margin, never smaller than the default sheet. */
export function sheetView(layout) {
  const grid = 32;
  let [x0, y0, x1, y1] = [-3 * grid, -3 * grid, 30 * grid, 18 * grid];
  const extend = ({ min, max }) => {
    x0 = Math.min(x0, min[0] - 4 * grid); y0 = Math.min(y0, min[1] - 4 * grid);
    x1 = Math.max(x1, max[0] + 4 * grid); y1 = Math.max(y1, max[1] + 4 * grid);
  };
  layout.symbols.forEach(symbol => extend(symbol.bounds));
  layout.labels.forEach(label => extend(label.bounds));
  layout.segments.forEach(segment => [segment.from, segment.to].forEach(at => extend({ min: at, max: at })));
  const snap = value => Math.floor(value / grid) * grid;
  return { x: snap(x0), y: snap(y0), width: snap(x1) - snap(x0) + grid, height: snap(y1) - snap(y0) + grid };
}
