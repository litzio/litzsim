mod routing;
mod symbols;

use crate::circuit::{Document, part_definition};
use serde::Serialize;
use std::collections::BTreeMap;

pub const GRID_PITCH: i64 = 32;
pub const JUMP_RADIUS: i64 = 8;
pub type Point = [i64; 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

impl Rect {
    fn between(a: Point, b: Point) -> Self {
        Self {
            min: [a[0].min(b[0]), a[1].min(b[1])],
            max: [a[0].max(b[0]), a[1].max(b[1])],
        }
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let min = [self.min[0].max(other.min[0]), self.min[1].max(other.min[1])];
        let max = [self.max[0].min(other.max[0]), self.max[1].min(other.max[1])];
        (min[0] <= max[0] && min[1] <= max[1]).then_some(Self { min, max })
    }

    fn overlap(self, other: Self) -> Option<Self> {
        self.intersection(other)
            .filter(|rect| rect.min[0] < rect.max[0] && rect.min[1] < rect.max[1])
    }

    fn union(self, other: Self) -> Self {
        Self {
            min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])],
            max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])],
        }
    }

    fn point(at: Point) -> Self {
        Self::between(at, at)
    }
}

fn scene([x, y]: Point) -> Point {
    [x * GRID_PITCH, y * GRID_PITCH]
}

fn add([x, y]: Point, [dx, dy]: Point) -> Point {
    [x + dx, y + dy]
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout<'a> {
    pub revision: String,
    pub symbols: Vec<Symbol<'a>>,
    pub pins: BTreeMap<String, Pin<'a>>,
    pub segments: Vec<Segment<'a>>,
    pub junctions: Vec<Junction<'a>>,
    pub jumps: Vec<Jump<'a>>,
    pub labels: Vec<Label<'a>>,
    pub issues: Vec<Issue>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Symbol<'a> {
    pub id: &'a str,
    #[serde(rename = "type")]
    pub kind: &'a str,
    pub center: Point,
    pub rotation: i32,
    pub mirror: Option<&'static str>,
    pub bounds: Rect,
    pub strokes: Vec<Stroke>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_arrow: Option<Arrow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meter_mark: Option<MeterMark>,
}

#[derive(Debug, Serialize)]
pub struct Arrow {
    pub from: Point,
    pub to: Point,
    pub head: [Point; 2],
}

#[derive(Debug, Serialize)]
pub struct MeterMark {
    pub at: Point,
    pub text: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Stroke {
    Polyline {
        points: Vec<[i32; 2]>,
    },
    FilledTriangle {
        points: [[i32; 2]; 3],
    },
    Circle {
        center: [i32; 2],
        radius: i32,
        filled: bool,
    },
    UpperArc {
        center: [i32; 2],
        radii: [i32; 2],
    },
    Cubic {
        points: [[i32; 2]; 4],
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin<'a> {
    pub part_id: &'a str,
    pub at: Point,
    pub net_id: Option<&'a str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment<'a> {
    pub from: Point,
    pub to: Point,
    pub net_id: &'a str,
    pub terminals: [&'a str; 2],
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Junction<'a> {
    pub at: Point,
    pub net_id: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Jump<'a> {
    pub at: Point,
    pub radius: i64,
    pub horizontal_net_id: &'a str,
    pub vertical_net_id: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Label<'a> {
    pub id: String,
    pub part_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_id: Option<String>,
    pub kind: LabelKind,
    pub text: String,
    pub bounds: Rect,
    pub metrics: LabelMetrics,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelKind {
    Id,
    Value,
    Terminal,
    Polarity,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelMetrics {
    pub advance: i64,
    pub line_height: i64,
    pub font_size: i64,
    pub padding: i64,
    pub bold: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub code: &'static str,
    pub targets: [Target; 2],
    pub area: Rect,
}

#[derive(Clone, Debug, Serialize)]
pub struct Target {
    pub kind: &'static str,
    pub id: String,
}

impl Target {
    fn new(kind: &'static str, id: &str) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }
}

struct Obstacle<'a> {
    target: Target,
    bounds: Rect,
    symbol: Option<(&'a str, Point)>,
}

/// Compute the schematic from the normalized document, in scene units.
pub fn layout(document: &Document) -> Layout<'_> {
    let inspection = document.inspect();
    let mut result = Layout {
        revision: inspection.revision,
        symbols: Vec::new(),
        pins: BTreeMap::new(),
        labels: Vec::new(),
        segments: Vec::new(),
        junctions: Vec::new(),
        jumps: Vec::new(),
        issues: Vec::new(),
    };

    let mut captions = Vec::new();
    for part in document.layout_parts() {
        let measurement = inspection.measurements.get(part.id);
        let (symbol, labels) = symbols::geometry(part, measurement);
        for pin in part_definition(symbol.kind).expect("validated part").pins {
            let id = format!("{}.{pin}", symbol.id);
            let at = scene(inspection.pin_positions[&id]);
            let net_id = inspection.pin_nets[&id];
            result.pins.insert(
                id,
                Pin {
                    part_id: symbol.id,
                    at,
                    net_id,
                },
            );
        }
        result.symbols.push(symbol);
        captions.extend(labels);
    }
    let mut obstacles: Vec<_> = result
        .symbols
        .iter()
        .map(|symbol| Obstacle {
            target: Target::new("symbol", symbol.id),
            bounds: symbol.bounds,
            symbol: Some((symbol.id, symbol.center)),
        })
        .collect();
    result.labels = symbols::place_labels(captions, &result.symbols, &result.pins, &mut obstacles);
    for (i, a) in obstacles.iter().enumerate() {
        for b in &obstacles[i + 1..] {
            let contact = if a.symbol.is_some() && b.symbol.is_some() {
                a.bounds.overlap(b.bounds)
            } else {
                a.bounds.intersection(b.bounds)
            };
            if let Some(area) = contact {
                result.issues.push(Issue {
                    code: "overlap",
                    targets: [a.target.clone(), b.target.clone()],
                    area,
                });
            }
        }
    }
    routing::route(document, &obstacles, &mut result);
    result
}

#[cfg(test)]
mod tests;
