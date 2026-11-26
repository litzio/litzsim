use super::{
    Arrow, GRID_PITCH, Issue, Label, LabelKind, LabelMetrics, MeterMark, Obstacle, Pin, Point,
    Rect, Stroke, Symbol, Target, add, scene,
};
use crate::circuit::{LayoutPart, Measurement, Mirror, Placement, part_definition};
use std::collections::BTreeMap;

type LocalPoint = [i32; 2];

fn line(points: &[LocalPoint]) -> Stroke {
    Stroke::Polyline {
        points: points.to_vec(),
    }
}

fn diode(x: i32) -> [Stroke; 2] {
    [
        line(&[[x - 8, -7], [x + 8, -7]]),
        Stroke::FilledTriangle {
            points: [[x - 7, 7], [x, -7], [x + 7, 7]],
        },
    ]
}

fn strokes(kind: &str) -> Vec<Stroke> {
    match kind {
        "resistor" => vec![line(&[
            [-32, 0],
            [-16, 0],
            [-13, -5],
            [-8, 5],
            [-3, -5],
            [2, 5],
            [7, -5],
            [12, 5],
            [16, 0],
            [32, 0],
        ])],
        "inductor" => {
            let mut strokes = vec![line(&[[-32, 0], [-24, 0]])];
            strokes.extend([-18, -6, 6, 18].map(|x| Stroke::UpperArc {
                center: [x, 0],
                radii: [6, 9],
            }));
            strokes.push(line(&[[24, 0], [32, 0]]));
            strokes
        }
        "capacitor" | "dc" => {
            let bottom = if kind == "dc" { 8 } else { 15 };
            vec![
                line(&[[0, -32], [0, -4]]),
                line(&[[-15, -4], [15, -4]]),
                line(&[[-bottom, 4], [bottom, 4]]),
                line(&[[0, 4], [0, 32]]),
            ]
        }
        "ground" => vec![
            line(&[[0, -32], [0, -4]]),
            line(&[[-12, -4], [12, -4]]),
            line(&[[-8, 1], [8, 1]]),
            line(&[[-3, 6], [3, 6]]),
        ],
        "diode" => {
            let mut strokes = vec![line(&[[0, 32], [0, 7]]), line(&[[0, -7], [0, -32]])];
            strokes.extend(diode(0));
            strokes
        }
        "nmos" => {
            let mut strokes = vec![
                line(&[[-32, 0], [-22, 0]]),
                line(&[[-22, -24], [-22, 24]]),
                line(&[[-16, -24], [-16, -13]]),
                line(&[[-16, -6], [-16, 6]]),
                line(&[[-16, 13], [-16, 24]]),
                line(&[[0, -32], [0, -20]]),
                line(&[[0, 20], [0, 32]]),
                line(&[[-16, -20], [13, -20], [13, -7]]),
                line(&[[-16, 20], [13, 20], [13, 7]]),
                line(&[[-16, 0], [4, 0], [4, 20]]),
                Stroke::FilledTriangle {
                    points: [[-16, 0], [-8, -4], [-8, 4]],
                },
            ];
            strokes.extend(diode(13));
            strokes.extend([[0, -20], [0, 20], [4, 20]].map(|center| Stroke::Circle {
                center,
                radius: 1,
                filled: true,
            }));
            strokes
        }
        "voltageSensor" | "currentSensor" => {
            let (meter, lead) = if kind == "voltageSensor" {
                ("voltmeter", [[20, 0], [32, 0]])
            } else {
                ("ammeter", [[0, -20], [0, -32]])
            };
            let mut body = strokes(meter);
            body.push(line(&lead));
            body
        }
        "constant" | "step" | "ramp" | "sum" | "gain" | "limiter" | "integrator" | "pi" => {
            let height = if kind == "sum" { 38 } else { 16 };
            let mut body = vec![line(&[
                [-20, -height],
                [20, -height],
                [20, height],
                [-20, height],
                [-20, -height],
            ])];
            let definition = part_definition(kind).expect("validated control block");
            body.extend(
                definition
                    .pin_offsets
                    .iter()
                    .map(|&[x, y]| line(&[[x * 20, y * 32], [x * 32, y * 32]])),
            );
            body
        }
        "ammeter" => vec![
            Stroke::Circle {
                center: [0, 0],
                radius: 20,
                filled: false,
            },
            line(&[[-32, 0], [-20, 0]]),
            line(&[[20, 0], [32, 0]]),
        ],
        "voltageProbe" => vec![
            Stroke::Circle {
                center: [0, 0],
                radius: 12,
                filled: false,
            },
            line(&[[0, 12], [0, 32]]),
            line(&[[-4, 24], [0, 30], [4, 24]]),
        ],
        "voltmeter" | "pwm" | "dutyPwm" | "sine" => {
            let mut strokes = vec![
                Stroke::Circle {
                    center: [0, 0],
                    radius: 20,
                    filled: false,
                },
                line(&[[0, -32], [0, -20]]),
                line(&[[0, 20], [0, 32]]),
            ];
            match kind {
                "voltmeter" => {}
                "pwm" | "dutyPwm" => strokes.push(line(&[
                    [-13, 6],
                    [-7, 6],
                    [-7, -6],
                    [7, -6],
                    [7, 6],
                    [13, 6],
                ])),
                "sine" => strokes.extend([
                    Stroke::Cubic {
                        points: [[-14, 0], [-10, -15], [-5, -15], [0, 0]],
                    },
                    Stroke::Cubic {
                        points: [[0, 0], [5, 15], [10, 15], [14, 0]],
                    },
                ]),
                _ => unreachable!(),
            }
            if kind == "dutyPwm" {
                strokes.push(line(&[[-32, 0], [-20, 0]]));
            }
            strokes
        }
        _ => unreachable!("validated part type"),
    }
}

fn transform(placement: Placement, center: Point, local: LocalPoint) -> Point {
    add(center, placement.orient(local).map(i64::from))
}

fn local_bounds(stroke: &Stroke) -> Rect {
    fn points(points: &[LocalPoint]) -> Rect {
        points
            .iter()
            .map(|p| Rect::point(p.map(i64::from)))
            .reduce(Rect::union)
            .unwrap()
    }
    match stroke {
        Stroke::Polyline { points: vertices } => points(vertices),
        Stroke::FilledTriangle { points: vertices } => points(vertices),
        Stroke::Cubic { points: vertices } => points(vertices),
        Stroke::Circle {
            center: [x, y],
            radius: r,
            ..
        } => points(&[[x - r, y - r], [x + r, y + r]]),
        Stroke::UpperArc {
            center: [x, y],
            radii: [rx, ry],
        } => points(&[[x - rx, y - ry], [x + rx, *y]]),
    }
}

fn transform_bounds(local: Rect, placement: Placement, center: Point) -> Rect {
    Rect::between(
        transform(placement, center, local.min.map(|n| n as i32)),
        transform(placement, center, local.max.map(|n| n as i32)),
    )
}

pub(super) fn ink_bounds(kind: &str, placement: Placement) -> impl Iterator<Item = Rect> {
    let center = scene([placement.x.into(), placement.y.into()]);
    strokes(kind)
        .into_iter()
        .map(move |stroke| transform_bounds(local_bounds(&stroke), placement, center))
}

fn value_text(part: &LayoutPart<'_>) -> Option<String> {
    let (key, unit, scale) = match part.kind {
        "resistor" => ("resistance_ohm", "Ω", 1.0),
        "inductor" => ("inductance_H", "H", 1.0),
        "capacitor" => ("capacitance_F", "F", 1.0),
        "dc" => ("voltage_V", "V", 1.0),
        "sine" => ("amplitude_V", "V RMS", std::f64::consts::FRAC_1_SQRT_2),
        "pwm" => ("frequency_Hz", "Hz", 1.0),
        _ => return None,
    };
    let value = part.parameters[key].as_f64().expect("validated parameter") * scale;
    let (number, prefix) = match value.abs() {
        magnitude if magnitude >= 1000.0 => (value / 1000.0, "k"),
        magnitude if (0.001..1.0).contains(&magnitude) => (value * 1000.0, "m"),
        magnitude if magnitude > 0.0 && magnitude < 0.001 => (value * 1e6, "µ"),
        _ => (value, ""),
    };
    let rendered = if number == 0.0 {
        "0".into()
    } else if (0.001..10000.0).contains(&number.abs()) {
        format!("{number:.3}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    } else {
        format!("{number:.3e}")
    };
    Some(format!("{rendered} {prefix}{unit}"))
}

fn caption<'a>(
    part_id: &'a str,
    kind: LabelKind,
    text: String,
    pin: Option<&str>,
    anchor: Point,
) -> Label<'a> {
    let (role, advance, line_height, font_size, bold) = match kind {
        LabelKind::Id => ("id", 10, 18, 16, true),
        LabelKind::Value => ("value", 8, 15, 13, false),
        LabelKind::Terminal => ("terminal", 8, 15, 13, false),
        LabelKind::Polarity => ("polarity", 14, 22, 20, true),
    };
    let padding = 2;
    let metrics = LabelMetrics {
        advance,
        line_height,
        font_size,
        padding,
        bold,
    };
    let width = advance * text.chars().count() as i64 + 2 * padding;
    let height = line_height + 2 * padding;
    let min = add(anchor, [-width / 2, -height / 2]);
    Label {
        id: match pin {
            Some(pin) => format!("{part_id}.{pin}:{role}"),
            None => format!("{part_id}:{role}"),
        },
        part_id,
        terminal_id: pin.map(|pin| format!("{part_id}.{pin}")),
        kind,
        text,
        bounds: Rect::between(min, add(min, [width, height])),
        metrics,
    }
}

pub(super) fn geometry<'a>(
    part: LayoutPart<'a>,
    measurement: Option<&Measurement>,
) -> (Symbol<'a>, Vec<Label<'a>>) {
    let LayoutPart {
        id,
        kind,
        placement,
        ..
    } = part;
    let center = scene([placement.x.into(), placement.y.into()]);
    let at = |point| transform(placement, center, point);
    let definition = part_definition(kind).expect("validated part");
    let offset = |pin| {
        let index = definition
            .pins
            .iter()
            .position(|name| *name == pin)
            .expect("defined terminal");
        definition.pin_offsets[index].map(|n| n * GRID_PITCH as i32)
    };
    let strokes = strokes(kind);
    let local = strokes
        .iter()
        .map(local_bounds)
        .reduce(Rect::union)
        .unwrap();
    let bounds = transform_bounds(local, placement, center);

    let mut labels = Vec::new();
    if kind == "nmos" {
        for (pin, mark, delta) in [
            ("d", "D", [15, -13]),
            ("g", "G", [-2, 17]),
            ("s", "S", [15, 13]),
        ] {
            let [x, y] = offset(pin);
            labels.push(caption(
                id,
                LabelKind::Terminal,
                mark.into(),
                Some(pin),
                at([x + delta[0], y + delta[1]]),
            ));
        }
    }
    let mut current_arrow = None;
    let mut meter_mark = None;
    if let Some(measurement) = measurement {
        if measurement.quantity == "current" {
            let from = offset(measurement.from_pin);
            let to = offset(measurement.to_pin.expect("current meter return pin"));
            let tip = to[0] / 3;
            let back = tip - 5 * (to[0] - from[0]).signum();
            current_arrow = Some(Arrow {
                from: at([from[0] / 3, 9]),
                to: at([tip, 9]),
                head: [at([back, 5]), at([back, 13])],
            });
            meter_mark = Some(MeterMark {
                at: at([0, -7]),
                text: "A",
            });
        } else {
            meter_mark = Some(MeterMark {
                at: center,
                text: "V",
            });
            if let Some(to_pin) = measurement.to_pin {
                for (pin, mark) in [(measurement.from_pin, "+"), (to_pin, "−")] {
                    labels.push(caption(
                        id,
                        LabelKind::Polarity,
                        mark.into(),
                        Some(pin),
                        at([-35, offset(pin)[1] / 2]),
                    ));
                }
            }
        }
    }

    let horizontal = matches!(kind, "resistor" | "ammeter" | "inductor")
        ^ (placement.rotation.unwrap_or_default() % 180 != 0);
    let width = 10 * id.len() as i64 + 4;
    let value = value_text(&part);
    let (id_pos, value_pos) = if kind == "nmos" {
        let [x, y] = placement.orient([1, 0]);
        let away = if x == 0 { 48 } else { 40 + width / 2 };
        (
            add(center, [i64::from(x) * away, i64::from(y) * away]),
            center,
        )
    } else if horizontal {
        (
            [center[0], bounds.min[1] - 14],
            [center[0], bounds.max[1] + 13],
        )
    } else {
        let value_width = value
            .as_ref()
            .map_or(0, |value| 8 * value.chars().count() as i64 + 4);
        (
            [bounds.max[0] + 5 + width / 2, center[1] - 11],
            [bounds.max[0] + 5 + value_width / 2, center[1] + 12],
        )
    };
    labels.push(caption(id, LabelKind::Id, id.into(), None, id_pos));
    if let Some(value) = value {
        labels.push(caption(id, LabelKind::Value, value, None, value_pos));
    }

    let symbol = Symbol {
        id,
        kind,
        center,
        rotation: placement.rotation.unwrap_or_default(),
        mirror: match placement.mirror {
            Some(Mirror::Horizontal) => Some("horizontal"),
            Some(Mirror::Vertical) => Some("vertical"),
            _ => None,
        },
        bounds,
        strokes,
        recordable: definition.recordable(),
        current_arrow,
        meter_mark,
    };
    (symbol, labels)
}

pub(super) fn place_labels<'a>(
    captions: Vec<Label<'a>>,
    symbols: &[Symbol<'_>],
    pins: &BTreeMap<String, Pin<'_>>,
    obstacles: &mut Vec<Obstacle<'a>>,
    issues: &mut Vec<Issue>,
) -> Vec<Label<'a>> {
    let owners: BTreeMap<_, _> = symbols.iter().map(|symbol| (symbol.id, symbol)).collect();
    let pin_clearance: Vec<_> = pins
        .iter()
        .map(|(id, pin)| {
            let center = owners[pin.part_id].center;
            let direction = [pin.at[0] - center[0], pin.at[1] - center[1]];
            // Leave room for a wire to leave a terminal along its lead.
            let end = add(pin.at, direction.map(|n| n / 2));
            let bounds = Rect::between(pin.at, end);
            (
                id,
                Rect::between(add(bounds.min, [-2; 2]), add(bounds.max, [2; 2])),
            )
        })
        .collect();
    captions
        .into_iter()
        .map(|mut label| {
            let owner = owners[label.part_id];
            let terminal = label.terminal_id.as_ref().map(|id| &pins[id]);
            let clear_of_obstacles = |bounds: Rect| {
                obstacles
                    .iter()
                    .all(|obstacle| bounds.intersection(obstacle.bounds).is_none())
            };
            let chosen = label_candidates(label.bounds, owner.bounds).find(|&bounds| {
                let near_terminal = terminal.is_none_or(|pin| {
                    if matches!(label.kind, LabelKind::Polarity) {
                        let origin = owner.center;
                        let axis = usize::from(pin.at[1] != origin[1]);
                        let clear_rail = if pin.at[axis] < origin[axis] {
                            bounds.min[axis] > pin.at[axis]
                        } else {
                            bounds.max[axis] < pin.at[axis]
                        };
                        if !clear_rail {
                            return false;
                        }
                    }
                    let center = [0, 1].map(|axis| (bounds.min[axis] + bounds.max[axis]) / 2);
                    let distance =
                        |at: Point| (at[0] - center[0]).abs() + (at[1] - center[1]).abs();
                    pins.values()
                        .filter(|other| other.part_id == pin.part_id)
                        .all(|other| distance(pin.at) <= distance(other.at))
                });
                near_terminal
                    && clear_of_obstacles(bounds)
                    && pin_clearance
                        .iter()
                        .all(|&(_, pin)| bounds.intersection(pin).is_none())
            });
            if let Some(bounds) = chosen {
                label.bounds = bounds;
            } else if clear_of_obstacles(label.bounds) {
                // Box overlaps are reported by layout(); a blocked lead can
                // also be the only reason no candidate fits.
                for &(id, clearance) in &pin_clearance {
                    if let Some(area) = label.bounds.intersection(clearance) {
                        issues.push(Issue {
                            code: "overlap",
                            targets: [Target::new("label", &label.id), Target::new("pin", id)],
                            area,
                        });
                    }
                }
            }
            obstacles.push(Obstacle {
                target: Target::new("label", &label.id),
                bounds: label.bounds,
                symbol: None,
            });
            label
        })
        .collect()
}

fn label_candidates(preferred: Rect, owner: Rect) -> impl Iterator<Item = Rect> {
    let size = [0, 1].map(|axis| preferred.max[axis] - preferred.min[axis]);
    let edge = move |axis: usize, positive| {
        if positive {
            owner.max[axis]
        } else {
            owner.min[axis] - size[axis]
        }
    };
    let mut sides = [(1, false), (1, true), (0, false), (0, true)];
    sides.sort_by_key(|&(axis, positive)| edge(axis, positive).abs_diff(preferred.min[axis]));
    std::iter::once(preferred).chain([4, 16].into_iter().flat_map(move |gap| {
        sides.into_iter().flat_map(move |(axis, positive)| {
            let tangent = 1 - axis;
            let center = preferred.min[tangent] + size[tangent] / 2;
            [
                center.clamp(owner.min[tangent], owner.max[tangent]),
                owner.min[tangent],
                owner.max[tangent],
            ]
            .into_iter()
            .map(move |along| {
                let mut min = [0; 2];
                min[tangent] = along - size[tangent] / 2;
                min[axis] = edge(axis, positive) + if positive { gap } else { -gap };
                Rect::between(min, add(min, size))
            })
        })
    }))
}
