use super::*;
use crate::circuit::{MirrorAxis, Operation, apply, undo};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const RC: &[u8] = include_bytes!("../../../../examples/rc.litzsim");
const RLC: &[u8] = include_bytes!("../../../../examples/rlc.litzsim");
const BUCK: &[u8] = include_bytes!("../../../../examples/buck.litzsim");

fn document(value: &Value) -> Document {
    Document::from_bytes(&serde_json::to_vec(value).unwrap()).unwrap()
}

fn contains(segment: &Segment<'_>, point: Point) -> bool {
    Rect::between(segment.from, segment.to)
        .intersection(Rect::point(point))
        .is_some()
}

fn connected(
    layout: &Layout<'_>,
    net: &str,
    start: Point,
    excluded: Option<usize>,
) -> BTreeSet<Point> {
    let mut reached = BTreeSet::new();
    let mut pending = vec![start];
    while let Some(at) = pending.pop() {
        if !reached.insert(at) {
            continue;
        }
        for (index, wire) in layout
            .segments
            .iter()
            .enumerate()
            .filter(|(_, wire)| wire.net_id == net)
        {
            if Some(index) != excluded {
                if wire.from == at {
                    pending.push(wire.to);
                }
                if wire.to == at {
                    pending.push(wire.from);
                }
            }
        }
    }
    reached
}

fn check_geometry(document: &Document, result: &Layout<'_>) {
    let inspection = document.inspect();
    for (id, pin) in &result.pins {
        assert_eq!(pin.at, scene(inspection.pin_positions[id]));
        assert_eq!(pin.net_id, inspection.pin_nets[id]);
    }
    for (net, ids) in document.layout_nets() {
        if let Some(root) = ids.first() {
            let reached = connected(result, net, result.pins[root].at, None);
            for pin in ids {
                assert!(reached.contains(&result.pins[pin].at), "{net}: {pin}");
            }
        }
    }

    let mut crossings = BTreeSet::new();
    for (i, wire) in result.segments.iter().enumerate() {
        assert_ne!(wire.from, wire.to);
        let horizontal = wire.from[1] == wire.to[1];
        assert!(horizontal || wire.from[0] == wire.to[0]);
        for coord in wire.from.into_iter().chain(wire.to) {
            assert_eq!(coord % GRID_PITCH, 0);
        }
        let [upstream, downstream] = wire.terminals;
        assert_eq!(result.pins[upstream].net_id, Some(wire.net_id));
        assert_eq!(result.pins[downstream].net_id, Some(wire.net_id));
        let from_side = connected(result, wire.net_id, wire.from, Some(i));
        assert!(from_side.contains(&result.pins[upstream].at));
        assert!(!from_side.contains(&result.pins[downstream].at));

        let rect = Rect::between(wire.from, wire.to);
        for label in &result.labels {
            assert!(
                rect.intersection(label.bounds).is_none(),
                "{wire:?} hits {label:?}"
            );
            let metrics = label.metrics;
            assert_eq!(
                label.bounds.max[0] - label.bounds.min[0],
                metrics.advance * label.text.chars().count() as i64 + 2 * metrics.padding
            );
            assert_eq!(
                label.bounds.max[1] - label.bounds.min[1],
                metrics.line_height + 2 * metrics.padding
            );
        }
        for symbol in &result.symbols {
            if let Some(contact) = rect.intersection(symbol.bounds) {
                // Terminal faces and wire endpoints can meet along the boundary.
                let normal = if horizontal { 1 } else { 0 };
                let terminal_endpoint = contact.min == contact.max
                    && [wire.from, wire.to].contains(&contact.min)
                    && result
                        .pins
                        .values()
                        .any(|pin| pin.part_id == symbol.id && pin.at == contact.min);
                assert!(
                    terminal_endpoint
                        || contact.min[normal] == symbol.bounds.min[normal]
                        || contact.min[normal] == symbol.bounds.max[normal],
                    "{wire:?} enters {symbol:?}"
                );
            }
        }
        for other in &result.segments[i + 1..] {
            if wire.net_id == other.net_id {
                continue;
            }
            let Some(crossing) = rect.intersection(Rect::between(other.from, other.to)) else {
                continue;
            };
            assert_eq!(crossing.min, crossing.max);
            let at = crossing.min;
            assert_ne!(horizontal, other.from[1] == other.to[1]);
            for run in [wire, other] {
                assert_ne!(at, run.from);
                assert_ne!(at, run.to);
            }
            let nets = if horizontal {
                (wire.net_id, other.net_id)
            } else {
                (other.net_id, wire.net_id)
            };
            assert!(crossings.insert((at, nets.0, nets.1)));
        }
    }
    let jumps: BTreeSet<_> = result
        .jumps
        .iter()
        .map(|jump| {
            assert_eq!(jump.radius, JUMP_RADIUS);
            let arc = Rect::between(
                add(jump.at, [-jump.radius, -jump.radius]),
                add(jump.at, [jump.radius, 0]),
            );
            assert!(
                result
                    .symbols
                    .iter()
                    .all(|symbol| arc.intersection(symbol.bounds).is_none())
            );
            assert!(
                result
                    .labels
                    .iter()
                    .all(|label| arc.intersection(label.bounds).is_none())
            );
            (jump.at, jump.horizontal_net_id, jump.vertical_net_id)
        })
        .collect();
    assert_eq!(jumps.len(), result.jumps.len());
    assert_eq!(crossings, jumps);

    for junction in &result.junctions {
        let wires: Vec<_> = result
            .segments
            .iter()
            .filter(|wire| contains(wire, junction.at))
            .collect();
        assert!(wires.iter().all(|wire| wire.net_id == junction.net_id));
        let terminals = result
            .pins
            .values()
            .filter(|pin| pin.at == junction.at && pin.net_id == Some(junction.net_id))
            .count();
        assert!(wires.len() + terminals >= 3);
    }
}

#[test]
fn examples_are_connected_clear_and_deterministic() {
    for bytes in [RC, RLC, BUCK] {
        let document = Document::from_bytes(bytes).unwrap();
        let result = layout(&document);
        assert!(result.issues.is_empty(), "{:?}", result.issues);
        check_geometry(&document, &result);
        assert_eq!(
            serde_json::to_vec(&result).unwrap(),
            serde_json::to_vec(&layout(&document)).unwrap()
        );
    }
    let buck = Document::from_bytes(BUCK).unwrap();
    let result = layout(&buck);
    let bend = scene(buck.layout_bend("Q1.d").unwrap());
    assert!(
        result
            .segments
            .iter()
            .filter(|segment| segment.net_id == "input")
            .filter(|segment| segment.from == bend || segment.to == bend)
            .count()
            == 2
    );
}

#[test]
fn example_voltmeters_take_manhattan_paths_to_their_rails() {
    for (name, bytes) in [("rc", RC), ("rlc", RLC), ("buck", BUCK)] {
        let doc = Document::from_bytes(bytes).unwrap();
        let drawing = layout(&doc);
        for id in ["VC.p", "VC.n"] {
            let pin = &drawing.pins[id];
            let mut previous = None;
            let mut at = pin.at;
            let mut length = 0;
            let mut turns = 0;
            let mut previous_axis = None;
            loop {
                let next = drawing
                    .segments
                    .iter()
                    .filter(|wire| Some(wire.net_id) == pin.net_id)
                    .filter_map(|wire| {
                        if wire.from == at {
                            Some(wire.to)
                        } else if wire.to == at {
                            Some(wire.from)
                        } else {
                            None
                        }
                    })
                    .find(|next| Some(*next) != previous)
                    .unwrap_or_else(|| panic!("{name} {id}: disconnected at {at:?}"));
                let axis = usize::from(next[0] == at[0]);
                turns += usize::from(previous_axis.is_some_and(|prior| prior != axis));
                length += (next[axis] - at[axis]).abs();
                previous_axis = Some(axis);
                previous = Some(at);
                at = next;
                let at_rail = drawing
                    .junctions
                    .iter()
                    .any(|j| j.at == at && Some(j.net_id) == pin.net_id)
                    || drawing
                        .pins
                        .values()
                        .any(|p| p.at == at && p.net_id == pin.net_id);
                if at_rail {
                    break;
                }
            }
            let manhattan = (at[0] - pin.at[0]).abs() + (at[1] - pin.at[1]).abs();
            assert_eq!(length, manhattan, "{name} {id} to {at:?}: {turns} turns");
            assert!(turns <= 1, "{name} {id} to {at:?}: {turns} turns");
        }
    }
}

#[test]
fn crossing_has_one_horizontal_jump_and_continuous_vertical_run_in_both_net_orders() {
    for (horizontal, vertical) in [("a", "b"), ("b", "a")] {
        let mut fixture = json!({
            "version": 1,
            "parts": {
                "left": {"type":"ground","parameters":{}},
                "right": {"type":"ground","parameters":{}},
                "top": {"type":"dc","parameters":{"voltage_V":5}},
                "bottom": {"type":"dc","parameters":{"voltage_V":5}}
            },
            "nets": {},
            "layout": {"left":{"x":-4,"y":1},"right":{"x":4,"y":1},
                       "top":{"x":0,"y":-4},"bottom":{"x":0,"y":4}},
            "simulation": {"timeStep_s":0.001,"stopTime_s":1}
        });
        fixture["nets"][horizontal] = json!(["left.ref", "right.ref"]);
        fixture["nets"][vertical] = json!(["top.n", "bottom.p"]);
        let document = document(&fixture);
        let result = layout(&document);
        assert!(result.issues.is_empty(), "{:?}", result.issues);
        check_geometry(&document, &result);
        assert_eq!(result.jumps.len(), 1);
        let jump = &result.jumps[0];
        assert_eq!(jump.at, [0, 0]);
        assert_eq!(jump.horizontal_net_id, horizontal);
        assert_eq!(jump.vertical_net_id, vertical);
        assert!(result.segments.iter().any(|wire| {
            wire.net_id == vertical && wire.from == [0, -96] && wire.to == [0, 96]
        }));
        assert!(result.junctions.is_empty());

        fixture["nets"] = json!({"tree": ["left.ref", "right.ref", "top.n"]});
        let doc = self::document(&fixture);
        let tee = layout(&doc);
        check_geometry(&doc, &tee);
        assert!(tee.issues.is_empty());
        assert!(tee.jumps.is_empty());
        assert_eq!(tee.junctions.len(), 1);
        assert_eq!(tee.junctions[0].at, [0, 0]);
        assert_eq!(tee.junctions[0].net_id, "tree");
        assert_eq!(
            tee.segments
                .iter()
                .filter(|wire| [wire.from, wire.to].contains(&[0, 0]))
                .count(),
            3
        );
    }

    let mut buck: Value = serde_json::from_slice(BUCK).unwrap();
    buck["bends"]["Q1.g"] = json!({"x":5,"y":0});
    let doc = document(&buck);
    let result = layout(&doc);
    assert!(result.issues.is_empty(), "{:?}", result.issues);
    check_geometry(&doc, &result);
    assert!(
        result
            .jumps
            .iter()
            .any(|jump| { jump.horizontal_net_id == "input" && jump.vertical_net_id == "gate" })
    );
}

#[test]
fn move_rotate_and_mirror_update_overlap_regions_and_undo_restores_layout() {
    let original = Document::from_bytes(BUCK).unwrap();
    let mut current = original.clone();
    let operations = [
        Operation::Move {
            id: "Q1".into(),
            grid: [4, 3],
        },
        Operation::Rotate { id: "Q1".into() },
        Operation::Mirror {
            id: "Q1".into(),
            axis: MirrorAxis::Vertical,
        },
    ];
    for op in operations {
        let before = layout(&current);
        let applied = apply(&current, &before.revision, &[op]).unwrap();
        let after = layout(&applied.document);
        assert_eq!(after.revision, applied.receipt.after_revision);
        assert_ne!(json!(before.issues), json!(after.issues));
        assert!(after.issues.iter().any(|issue| {
            issue.code == "overlap"
                && issue.targets.iter().all(|target| target.kind == "symbol")
                && issue.targets.iter().any(|target| target.id == "Q1")
                && issue.targets.iter().any(|target| target.id == "PWM")
                && issue.area.min != issue.area.max
        }));
        let undone = undo(
            &applied.document,
            &after.revision,
            &before.revision,
            &current,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_vec(&layout(&undone.document)).unwrap(),
            serde_json::to_vec(&before).unwrap()
        );
        current = applied.document;
    }
}

#[test]
fn bend_on_first_terminal_is_required_and_blocked_waypoints_are_reported() {
    let mut fixture: Value = serde_json::from_slice(RC).unwrap();
    fixture["bends"] = json!({ "V1.p": {"x": -3, "y": -2} });
    let with_bend = document(&fixture);
    let drawing = layout(&with_bend);
    check_geometry(&with_bend, &drawing);
    let bend = scene([-3, -2]);
    assert!(
        drawing
            .segments
            .iter()
            .any(|wire| wire.net_id == "input" && wire.to == bend)
    );

    fixture["bends"]["V1.p"] = json!({ "x": 0, "y": 2 });
    let blocked = document(&fixture);
    let drawing = layout(&blocked);
    assert!(!drawing.segments.iter().any(|wire| wire.net_id == "input"));
    assert!(drawing.issues.iter().any(|issue| {
        issue.code == "routeFailed"
            && issue.targets.iter().any(|target| target.id == "V1.p")
            && issue.targets.iter().any(|target| target.id == "R1.p")
            && issue
                .area
                .intersection(Rect::point(scene([0, 2])))
                .is_some()
    }));
}

#[test]
fn coincident_terminal_with_an_unmet_bend_does_not_create_a_cycle() {
    let mut fixture: Value = serde_json::from_slice(RC).unwrap();
    fixture["parts"]["G2"] = fixture["parts"]["GND"].clone();
    fixture["layout"]["G2"] = fixture["layout"]["GND"].clone();
    fixture["nets"]["ground"] = json!(["V1.n", "GND.ref", "G2.ref"]);
    fixture["bends"] = json!({"G2.ref":{"x":2,"y":5}});
    let doc = document(&fixture);
    let result = layout(&doc);
    assert!(
        result
            .issues
            .iter()
            .any(|issue| { issue.code == "routeFailed" && issue.targets[0].id == "G2.ref" })
    );
    check_geometry(&doc, &result);
}

#[test]
fn reports_label_overlap_when_all_nearby_positions_are_occupied() {
    let mut fixture = json!(Document::new());
    for index in 0..10 {
        let id = format!("ground{}{}", "_".repeat(50), index);
        fixture["parts"][id] = json!({"type":"ground","parameters":{}});
    }
    let doc = document(&fixture);
    let result = layout(&doc);
    assert!(result.issues.iter().any(|issue| {
        issue.code == "overlap"
            && issue.targets.iter().all(|target| target.kind == "label")
            && issue.targets[0].id != issue.targets[1].id
            && {
                let bounds = |id: &str| {
                    result
                        .labels
                        .iter()
                        .find(|label| label.id == id)
                        .unwrap()
                        .bounds
                };
                bounds(&issue.targets[0].id).intersection(bounds(&issue.targets[1].id))
                    == Some(issue.area)
            }
    }));
}

#[test]
fn measurement_marks_follow_terminal_identity_after_rotation_and_mirror() {
    let mut fixture: Value = serde_json::from_slice(RC).unwrap();
    fixture["parts"]["IA"] = json!({"type":"ammeter","parameters":{}});
    fixture["layout"]["IA"] = json!({"x":24,"y":2});
    for rotation in [0, 90, 180, 270] {
        for mirror in ["horizontal", "vertical"] {
            for id in ["VC", "IA"] {
                fixture["layout"][id]["rotation"] = json!(rotation);
                fixture["layout"][id]["mirror"] = json!(mirror);
            }
            let doc = document(&fixture);
            let result = layout(&doc);
            assert!(result.issues.is_empty(), "{:?}", result.issues);
            let arrow = result
                .symbols
                .iter()
                .find(|symbol| symbol.id == "IA")
                .unwrap()
                .current_arrow
                .as_ref()
                .unwrap();
            let from = result.pins["IA.p"].at;
            let to = result.pins["IA.n"].at;
            for axis in [0, 1] {
                assert_eq!(
                    (arrow.to[axis] - arrow.from[axis]).signum(),
                    (to[axis] - from[axis]).signum()
                );
            }
            for (id, other, text) in [("VC.p", "VC.n", "+"), ("VC.n", "VC.p", "−")] {
                let mark = result
                    .labels
                    .iter()
                    .find(|label| label.terminal_id.as_deref() == Some(id))
                    .unwrap();
                assert_eq!(mark.text, text);
                let at = [0, 1].map(|axis| (mark.bounds.min[axis] + mark.bounds.max[axis]) / 2);
                let distance = |pin: &str| {
                    (0..2)
                        .map(|axis| (at[axis] - result.pins[pin].at[axis]).abs())
                        .sum::<i64>()
                };
                assert!(distance(id) < distance(other));
            }
        }
    }
}
