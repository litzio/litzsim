use super::{CircuitV1, Document, Error, Mirror, Part, part_definition};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Operation {
    Add {
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        grid: Option<[i32; 2]>,
    },
    Move {
        id: String,
        grid: [i32; 2],
    },
    Rotate {
        id: String,
    },
    Mirror {
        id: String,
        axis: MirrorAxis,
    },
    SetParameter {
        id: String,
        key: String,
        value: f64,
    },
    Connect {
        a: String,
        b: String,
    },
    Disconnect {
        pin: String,
    },
    Delete {
        id: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MirrorAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Serialize)]
pub struct EditResult {
    pub document: Document,
    pub receipt: Receipt,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub before_revision: String,
    pub after_revision: String,
    pub created: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
    pub nets: BTreeMap<String, NetChange>,
}

#[derive(Debug, Serialize)]
pub struct NetChange {
    pub before: Option<Vec<String>>,
    pub after: Option<Vec<String>>,
}

pub fn apply(
    document: &Document,
    expected_revision: &str,
    ops: &[Operation],
) -> Result<EditResult, Error> {
    let before = check_revision(document, expected_revision)?;
    let mut next = document.clone();
    for (index, op) in ops.iter().enumerate() {
        next.0.apply(op).map_err(|mut error| {
            error.code = "operationFailed";
            error.operation = Some(index + 1);
            error
        })?;
    }
    Ok(EditResult {
        receipt: receipt(document, &next, before),
        document: next,
    })
}

pub fn undo(
    current: &Document,
    expected_after: &str,
    expected_before: &str,
    previous: &Document,
) -> Result<EditResult, Error> {
    let before = check_revision(current, expected_after)?;
    let previous_revision = previous.revision();
    if previous_revision != expected_before {
        let mut error = Error::conflict(
            "previousMismatch",
            Some(expected_before),
            Some(&previous_revision),
        );
        error.target_id = Some("previous".into());
        error.message = "Previous snapshot does not match the receipt's before revision".into();
        return Err(error);
    }
    Ok(EditResult {
        document: previous.clone(),
        receipt: receipt(current, previous, before),
    })
}

fn check_revision(document: &Document, expected: &str) -> Result<String, Error> {
    let actual = document.revision();
    if actual != expected {
        return Err(Error::conflict(
            "revisionConflict",
            Some(expected),
            Some(&actual),
        ));
    }
    Ok(actual)
}

impl CircuitV1 {
    fn apply(&mut self, op: &Operation) -> Result<(), Error> {
        match op {
            Operation::Add { kind, grid } => {
                let definition = part_definition(kind)
                    .ok_or_else(|| Error::invalid(kind, "unknown part type"))?;
                let id = next_id(kind, &self.parts);
                let parameters = definition
                    .defaults
                    .iter()
                    .map(|&(key, value)| (key.to_owned(), value.into()))
                    .collect();
                let [x, y] = grid.unwrap_or_default();
                self.layout.insert(
                    id.clone(),
                    super::Placement {
                        x,
                        y,
                        ..Default::default()
                    },
                );
                self.parts.insert(
                    id,
                    Part {
                        kind: kind.clone(),
                        parameters,
                    },
                );
            }
            Operation::Move { id, grid: [x, y] } => {
                self.require_part(id)?;
                let position = self.layout.entry(id.clone()).or_default();
                position.x = *x;
                position.y = *y;
            }
            Operation::Rotate { id } => {
                self.require_part(id)?;
                let position = self.layout.entry(id.clone()).or_default();
                position.rotation = Some((position.rotation.unwrap_or_default() + 90) % 360);
                // Rotate the current pose, including any previous reflection.
                position.mirror = match position.mirror {
                    Some(Mirror::Horizontal) => Some(Mirror::Vertical),
                    Some(Mirror::Vertical) => Some(Mirror::Horizontal),
                    other => other,
                };
                position.normalize_orientation();
            }
            Operation::Mirror { id, axis } => {
                self.require_part(id)?;
                let position = self.layout.entry(id.clone()).or_default();
                use MirrorAxis::{Horizontal, Vertical};
                position.mirror =
                    match (position.mirror, axis) {
                        (None, Horizontal) => Some(Mirror::Horizontal),
                        (None, Vertical) => Some(Mirror::Vertical),
                        (Some(Mirror::Horizontal), Vertical)
                        | (Some(Mirror::Vertical), Horizontal) => Some(Mirror::Both),
                        (Some(Mirror::Horizontal), Horizontal)
                        | (Some(Mirror::Vertical), Vertical) => None,
                        (Some(Mirror::Both), _) => unreachable!("normalized mirror"),
                    };
                position.normalize_orientation();
            }
            Operation::SetParameter { id, key, value } => {
                self.require_part(id)?;
                let parameter = self
                    .parts
                    .get_mut(id)
                    .unwrap()
                    .parameters
                    .get_mut(key)
                    .ok_or_else(|| Error::invalid(id, format!("unknown parameter: {key}")))?;
                if !value.is_finite() {
                    return Err(Error::invalid(
                        id,
                        format!("invalid value for parameter: {key}"),
                    ));
                }
                *parameter = (*value).into();
            }
            Operation::Connect { a, b } => self.connect(a, b)?,
            Operation::Disconnect { pin } => {
                self.require_pin(pin)?;
                self.disconnect(pin);
            }
            Operation::Delete { id } => {
                let part = self.require_part(id)?;
                let definition = part_definition(&part.kind).expect("validated part");
                for pin in definition.pins {
                    let terminal = format!("{id}.{pin}");
                    self.disconnect(&terminal);
                    if let Some(bends) = &mut self.bends {
                        bends.remove(&terminal);
                    }
                }
                self.parts.remove(id);
                self.layout.remove(id);
            }
        }
        Ok(())
    }

    fn net_of(&self, pin: &str) -> Option<&str> {
        self.nets.iter().find_map(|(id, pins)| {
            pins.iter()
                .any(|candidate| candidate == pin)
                .then_some(id.as_str())
        })
    }

    fn connect(&mut self, a: &str, b: &str) -> Result<(), Error> {
        self.require_pin(a)?;
        self.require_pin(b)?;
        let left = self.net_of(a);
        let right = self.net_of(b);
        if a == b || left.is_some() && left == right {
            return Ok(());
        }
        // Keep the first pin's net when merging, as in the archived editor.
        let id = left
            .or(right)
            .map(str::to_owned)
            .unwrap_or_else(|| next_id("net", &self.nets));
        let merged = if left.is_some() {
            right
                .map(str::to_owned)
                .and_then(|right| self.nets.remove(&right))
        } else {
            None
        };
        let pins = self.nets.entry(id).or_default();
        pins.extend(merged.into_iter().flatten());
        for pin in [a, b] {
            if !pins.iter().any(|existing| existing == pin) {
                pins.push(pin.into());
            }
        }
        Ok(())
    }

    fn disconnect(&mut self, pin: &str) {
        if let Some(net) = self.net_of(pin).map(str::to_owned) {
            let pins = self.nets.get_mut(&net).unwrap();
            pins.retain(|existing| existing != pin);
            if pins.is_empty() {
                self.nets.remove(&net);
            }
        }
    }
}

fn next_id<T>(prefix: &str, entries: &BTreeMap<String, T>) -> String {
    (1..)
        .map(|index| format!("{prefix}{index}"))
        .find(|id| !entries.contains_key(id))
        .expect("available ID")
}

fn receipt(before: &Document, after: &Document, before_revision: String) -> Receipt {
    let mut receipt = Receipt {
        before_revision,
        after_revision: after.revision(),
        created: Vec::new(),
        modified: Vec::new(),
        deleted: Vec::new(),
        nets: BTreeMap::new(),
    };
    for (id, part) in &after.0.parts {
        match before.0.parts.get(id) {
            None => receipt.created.push(id.clone()),
            Some(prior)
                if prior != part
                    || before.0.layout.get(id) != after.0.layout.get(id)
                    || part_definition(&part.kind).unwrap().pins.iter().any(|pin| {
                        let pin = format!("{id}.{pin}");
                        let old = before.0.bends.as_ref().and_then(|bends| bends.get(&pin));
                        let new = after.0.bends.as_ref().and_then(|bends| bends.get(&pin));
                        old != new
                    }) =>
            {
                receipt.modified.push(id.clone())
            }
            Some(_) => {}
        }
    }
    receipt.deleted = before
        .0
        .parts
        .keys()
        .filter(|id| !after.0.parts.contains_key(*id))
        .cloned()
        .collect();
    for id in before.0.nets.keys().chain(
        after
            .0
            .nets
            .keys()
            .filter(|id| !before.0.nets.contains_key(*id)),
    ) {
        let old = before.0.nets.get(id);
        let new = after.0.nets.get(id);
        if old != new {
            receipt.nets.insert(
                id.clone(),
                NetChange {
                    before: old.cloned(),
                    after: new.cloned(),
                },
            );
        }
    }
    receipt
}
