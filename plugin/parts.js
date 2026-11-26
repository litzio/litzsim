// Library entries for the editor. Part types, terminals and defaults belong to
// the core; this file only names, groups and assigns default shortcuts.
export const CATEGORIES = [
  {
    id: "sources", label: "전원", parts: [
      { type: "dc", label: "직류 전압원", key: "V" },
      { type: "sine", label: "사인 전압원", key: "S" },
      { type: "ground", label: "접지", key: "G" },
    ],
  },
  {
    id: "passive", label: "수동 소자", parts: [
      { type: "resistor", label: "저항", key: "R" },
      { type: "inductor", label: "인덕터", key: "L" },
      { type: "capacitor", label: "커패시터", key: "C" },
    ],
  },
  {
    id: "switches", label: "반도체", parts: [
      { type: "diode", label: "다이오드", key: "D" },
      { type: "nmos", label: "N채널 MOSFET", key: "M" },
    ],
  },
  { id: "control", label: "제어", parts: [{ type: "pwm", label: "PWM 신호", key: "P" }] },
  {
    id: "meters", label: "측정", parts: [
      { type: "ammeter", label: "전류계", key: "A" },
      { type: "voltmeter", label: "전압계", key: "Shift+V" },
      { type: "voltageProbe", label: "노드 전압계", key: "N" },
    ],
  },
];

export const PARTS = Object.fromEntries(CATEGORIES.flatMap(group => group.parts.map(part => [part.type, part])));
export const ROTATE_SHORTCUT = "Shift+R";

export const PARAMETERS = {
  resistance_ohm: ["저항", "Ω"],
  inductance_H: ["인덕턴스", "H"],
  initialCurrent_A: ["초기 전류", "A"],
  capacitance_F: ["커패시턴스", "F"],
  initialVoltage_V: ["초기 전압", "V"],
  voltage_V: ["전압", "V"],
  amplitude_V: ["진폭", "V"],
  frequency_Hz: ["주파수", "Hz"],
  duty: ["듀티비", ""],
  high_V: ["High 전압", "V"],
};

const PREFIXES = [[1e9, "G"], [1e6, "M"], [1e3, "k"], [1, ""], [1e-3, "m"], [1e-6, "µ"], [1e-9, "n"], [1e-12, "p"]];
const SCALE = { G: 1e9, M: 1e6, k: 1e3, "": 1, m: 1e-3, u: 1e-6, "µ": 1e-6, n: 1e-9, p: 1e-12 };

export function formatSI(value) {
  if (!Number.isFinite(value) || value === 0) return String(value);
  const magnitude = Math.abs(value);
  const [scale, prefix] = PREFIXES.find(([scale]) => magnitude >= scale * (1 - 1e-9)) ?? PREFIXES.at(-1);
  return `${Number((value / scale).toPrecision(6))}${prefix}`;
}

/** Plain numbers or an SI prefix such as 4.7k, 10u or 2.2µ. */
export function parseSI(text) {
  const match = /^\s*([-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?)\s*([GMkmuµnp]?)\s*$/.exec(text);
  return match ? Number(match[1]) * SCALE[match[2]] : NaN;
}

export function shortcutOf(event) {
  if (event.ctrlKey || event.metaKey || event.altKey || event.key.length !== 1) return "";
  return (event.shiftKey ? "Shift+" : "") + event.key.toUpperCase();
}
