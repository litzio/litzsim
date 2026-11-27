import React from "react";
import { formatValue } from "./plot.js";

const muted = "var(--id-color-text-muted)";
const labelStyle = { margin: 0, color: muted, fontSize: "var(--font-size-xs)" };
const eventLabels = { step: "계단 지정", rampEnd: "램프 끝", switch: "스위치 전환" };

export function SourcePreview({ adapter, request }) {
  const [result, setResult] = React.useState(null);
  React.useEffect(() => {
    let active = true;
    adapter.call("sourcePreview", request).then(
      preview => { if (active) setResult({ preview }); },
      error => { if (active) setResult({ error: error.message }); },
    );
    return () => { active = false; };
  }, [adapter, request.document, request.partId]);

  return <section aria-label="출력 미리보기" style={{ display: "flex", flexDirection: "column", gap: "var(--space-1)" }}>
    <h4 style={{ margin: 0 }}>출력 미리보기</h4>
    {result?.preview ? <PreviewPlot {...result.preview} />
      : result?.error ? <p role="alert" style={{ ...labelStyle, color: "var(--id-color-warning)" }}>{result.error}</p>
      : <p role="status" style={labelStyle}>불러오는 중…</p>}
  </section>;
}

function PreviewPlot({ points, unit, holdSamples, decimated, events }) {
  const end = points.at(-1)[0];
  const values = points.map(([, value]) => value);
  const [low, high] = unit === "state" ? [0, 1] : [Math.min(...values), Math.max(...values)];
  const constant = low === high;
  const x = time => 6 + time / end * 228;
  const span = high - low;
  const fraction = value => Number.isFinite(span) ? (value - low) / span
    : (value / 2 - low / 2) / (high / 2 - low / 2);
  const y = value => constant ? 44 : 78 - fraction(value) * 68;
  const path = points.map(([time, value], index) => {
    if (!index) return `M${x(time)},${y(value)}`;
    return holdSamples ? `H${x(time)}V${y(value)}` : `L${x(time)},${y(value)}`;
  }).join(" ");
  const showValue = value => unit === "state" ? (value ? "닫힘" : "열림")
    : formatValue(value, unit === "V" ? unit : "", 4);
  return <>
    {unit === "V" && <p style={labelStyle}>설정 전압 (V)</p>}
    <div style={{ display: "grid", gridTemplateColumns: "max-content minmax(0, 1fr)", gap: "var(--space-1)" }}>
      <div style={{ ...labelStyle, display: "flex", flexDirection: "column",
        justifyContent: constant ? "center" : "space-between", textAlign: "right", whiteSpace: "nowrap", padding: "3px 0" }}>
        <span>{showValue(high)}</span>
        {!constant && <span>{showValue(low)}</span>}
      </div>
      <svg role="img" aria-label="시간에 따른 출력" data-source-preview="" viewBox="0 0 240 88"
        preserveAspectRatio="none" style={{ width: "100%", height: 88, background: "var(--id-color-surface-subtle)", borderRadius: "var(--id-radius-sm)" }}>
        <g stroke="var(--id-color-border)" strokeWidth="1" fill="none">
          <path d="M6 10H234 M6 44H234 M6 78H234" />
          {low < 0 && high > 0 && <path d={`M6 ${y(0)}H234`} stroke={muted} />}
        </g>
        {events.map(({ time, kind }) => <line key={kind} x1={x(time)} x2={x(time)} y1="5" y2="83"
          stroke={muted} strokeDasharray="3 3" />)}
        <path data-preview-trace="" d={path} fill="none" stroke="var(--id-color-interaction)" strokeWidth="2"
          vectorEffect="non-scaling-stroke" strokeLinejoin="round" />
      </svg>
      <div style={{ ...labelStyle, gridColumn: 2, display: "flex", justifyContent: "space-between", paddingInline: "2.5%" }}>
        <span>0 s</span><span>{formatValue(end, "s", 4)}</span>
      </div>
    </div>
    {events.map(({ time, kind }) => <p key={kind} style={labelStyle}>{eventLabels[kind]} · {formatValue(time, "s")}</p>)}
    {decimated && <p style={labelStyle}>일부 제어 샘플 생략</p>}
  </>;
}
