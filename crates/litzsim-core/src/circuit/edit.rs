use super::{CircuitV1, Document, Error, Mirror, Part, RecordQuantity, part_definition};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
    SetRecording {
        id: String,
        quantity: RecordQuantity,
        enabled: bool,
    },
    /// `timeStep_s`, `stopTime_s` or `outputStep_s` in `simulation`.
    SetSimulation {
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
    /// Give a part a new ID, updating its terminals in `nets`, `layout` and `bends`.
    Rename {
        id: String,
        #[serde(rename = "newId")]
        new_id: String,
    },
    /// Cut one wire segment from `layout` without inserting a part.
    RemoveBranch {
        terminals: [String; 2],
        from: [i64; 2],
        to: [i64; 2],
    },
    /// Cut one wire segment from `layout` and put a two-terminal part in series.
    InsertInBranch {
        #[serde(rename = "type")]
        kind: String,
        /// The segment's `terminals`; the part's first pin faces `terminals[0]`.
        terminals: [String; 2],
        /// The segment's scene endpoints, which tell apart cuts with equal terminals.
        from: [i64; 2],
        to: [i64; 2],
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
    /// Surviving parts with a new ID, as old ID → new ID.
    pub renamed: BTreeMap<String, String>,
    /// Voltage sources whose two terminals share one net after, but not before.
    pub shorted_sources: Vec<String>,
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
    // Current ID → original ID, only for surviving parts from before the batch.
    let mut origins: BTreeMap<&str, &str> = document
        .0
        .parts
        .keys()
        .map(|id| (id.as_str(), id.as_str()))
        .collect();
    let mut remaining = ops.iter().enumerate().peekable();
    while let Some((index, op)) = remaining.next() {
        let result = match op {
            Operation::RemoveBranch { .. } => {
                // All consecutive wire selections refer to the same drawn layout.
                let cuts = std::iter::once((index, op)).chain(std::iter::from_fn(|| {
                    remaining.next_if(|(_, op)| matches!(op, Operation::RemoveBranch { .. }))
                }));
                remove_branches(&mut next, cuts)?;
                continue;
            }
            Operation::InsertInBranch {
                kind,
                terminals,
                from,
                to,
            } => insert_in_branch(&mut next, kind, terminals, *from, *to),
            Operation::Rename { id, new_id } => next.0.rename(id, new_id).map(|()| {
                if let Some(origin) = origins.remove(id.as_str()) {
                    origins.insert(new_id, origin);
                }
            }),
            Operation::Delete { id } => next.0.apply(op).map(|()| {
                origins.remove(id.as_str());
            }),
            _ => next.0.apply(op),
        };
        result.map_err(|error| operation_failed(error, index))?;
    }
    Ok(EditResult {
        receipt: receipt(document, &next, before, &origins),
        document: next,
    })
}

fn operation_failed(mut error: Error, index: usize) -> Error {
    error.code = "operationFailed";
    error.operation = Some(index + 1);
    error
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
    // Undo compares snapshots, matching parts by ID.
    let origins = previous
        .0
        .parts
        .keys()
        .filter(|id| current.0.parts.contains_key(*id))
        .map(|id| (id.as_str(), id.as_str()))
        .collect();
    Ok(EditResult {
        document: previous.clone(),
        receipt: receipt(current, previous, before, &origins),
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
                let id = next_id(definition.id_prefix, &self.parts);
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
                        record: Default::default(),
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
            Operation::SetRecording {
                id,
                quantity,
                enabled,
            } => {
                let part = self.require_part(id)?;
                let definition = part_definition(&part.kind).expect("validated part");
                if definition.recording_pins.is_none() {
                    return Err(Error::invalid(
                        id,
                        "part does not support waveform recording",
                    ));
                }
                let record = &mut self.parts.get_mut(id).unwrap().record;
                match quantity {
                    RecordQuantity::Voltage => record.voltage = *enabled,
                    RecordQuantity::Current => record.current = *enabled,
                }
            }
            Operation::SetSimulation { key, value } => {
                let simulation = &mut self.simulation;
                match key.as_str() {
                    "timeStep_s" => simulation.time_step_s = *value,
                    "stopTime_s" => simulation.stop_time_s = *value,
                    "outputStep_s" => simulation.output_step_s = Some(*value),
                    _ => {
                        return Err(Error::invalid(
                            "simulation",
                            format!("unknown simulation setting: {key}"),
                        ));
                    }
                }
                simulation.validate()?;
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
            Operation::RemoveBranch { .. }
            | Operation::InsertInBranch { .. }
            | Operation::Rename { .. } => {
                unreachable!("applied by `apply`")
            }
        }
        Ok(())
    }

    fn rename(&mut self, id: &str, new_id: &str) -> Result<(), Error> {
        let part = self.require_part(id)?;
        let pins = part_definition(&part.kind).expect("validated part").pins;
        if new_id == id {
            return Err(Error::invalid(id, "new ID is the same as the current ID"));
        }
        if new_id.is_empty() {
            return Err(Error::invalid(id, "new ID is empty"));
        }
        if new_id.len() > MAX_PART_ID_LENGTH {
            return Err(Error::invalid(
                id,
                format!("new ID is longer than {MAX_PART_ID_LENGTH} characters: {new_id}"),
            ));
        }
        // Waveform channels are named `<ID>_<unit>`, so keep to channel-safe characters.
        let mut bytes = new_id.bytes();
        if !(bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
            && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
        {
            return Err(Error::invalid(
                id,
                format!(
                    "invalid new ID: {new_id} (use letters, digits and underscores, starting with a letter)"
                ),
            ));
        }
        if let Some(existing) = self
            .parts
            .keys()
            .find(|other| *other != id && other.eq_ignore_ascii_case(new_id))
        {
            return Err(Error::invalid(
                id,
                format!("part ID already in use: {existing}"),
            ));
        }
        let part = self.parts.remove(id).expect("required part");
        self.parts.insert(new_id.into(), part);
        if let Some(placement) = self.layout.remove(id) {
            self.layout.insert(new_id.into(), placement);
        }
        for pin in pins {
            let (old, new) = (format!("{id}.{pin}"), format!("{new_id}.{pin}"));
            for terminal in self.nets.values_mut().flatten() {
                if *terminal == old {
                    terminal.clone_from(&new);
                }
            }
            if let Some(bends) = &mut self.bends
                && let Some(bend) = bends.remove(&old)
            {
                bends.insert(new, bend);
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

fn remove_branches<'a>(
    document: &mut Document,
    cuts: impl Iterator<Item = (usize, &'a Operation)>,
) -> Result<(), Error> {
    let mut partitions = BTreeMap::new();
    for (index, op) in cuts {
        let Operation::RemoveBranch {
            terminals,
            from,
            to,
        } = op
        else {
            unreachable!("only wire cuts are grouped");
        };
        let (net, far) = crate::layout::branch_cut(document, terminals, *from, *to)
            .map_err(|error| operation_failed(error, index))?;
        let groups = partitions
            .entry(net.clone())
            .or_insert_with(|| vec![document.0.nets[&net].clone()]);
        // Each cut divides any group that still has terminals on both sides.
        // Keep the original ID with the first group and preserve file order.
        let mut splits = Vec::new();
        for group in groups.iter_mut() {
            let moved: Vec<_> = group
                .iter()
                .filter(|pin| far.contains(pin))
                .cloned()
                .collect();
            if !moved.is_empty() && moved.len() < group.len() {
                group.retain(|pin| !far.contains(pin));
                splits.push(moved);
            }
        }
        groups.extend(splits);
    }
    let nets = &mut document.0.nets;
    let mut disconnected = Vec::new();
    for (net, groups) in partitions {
        let mut groups = groups.into_iter();
        let near = groups.next().expect("original net");
        if near.len() >= 2 {
            nets.insert(net, near);
        } else {
            disconnected.push(net);
        }
        for far in groups.filter(|pins| pins.len() >= 2) {
            nets.insert(next_id("net", nets), far);
        }
    }
    for net in disconnected {
        nets.remove(&net);
    }
    Ok(())
}

fn insert_in_branch(
    document: &mut Document,
    kind: &str,
    terminals: &[String; 2],
    from: [i64; 2],
    to: [i64; 2],
) -> Result<(), Error> {
    let definition =
        part_definition(kind).ok_or_else(|| Error::invalid(kind, "unknown part type"))?;
    let &[first, second] = definition.pins else {
        return Err(Error::invalid(kind, "expected a two-terminal part"));
    };
    let (net, far) = crate::layout::branch_cut(document, terminals, from, to)?;
    // Center the part on the segment and turn its first pin toward `from`.
    let pitch = crate::layout::GRID_PITCH;
    let center = |axis: usize| {
        let grid = (from[axis] + to[axis] + pitch).div_euclid(2 * pitch);
        i32::try_from(grid).map_err(|_| Error::invalid(&terminals[1], "segment out of range"))
    };
    let toward = [0, 1].map(|axis| (from[axis] - to[axis]).signum() as i32);
    let mut placement = super::Placement {
        x: center(0)?,
        y: center(1)?,
        ..Default::default()
    };
    placement.rotation = [0, 90, 180, 270]
        .into_iter()
        .find(|&rotation| {
            let turned = super::Placement {
                rotation: Some(rotation),
                ..placement
            };
            turned.orient(definition.pin_offsets[0]) == toward
        })
        .filter(|&rotation| rotation != 0);
    let length = from[0].abs_diff(to[0]) + from[1].abs_diff(to[1]);
    if kind == "ammeter"
        && length <= (3 * pitch) as u64
        && let Some(neighbor) = crate::layout::ammeter_overlap(document, placement, [from, to])
    {
        return Err(Error::invalid(
            neighbor,
            format!("ammeter would overlap {neighbor} on this short segment"),
        ));
    }
    let circuit = &mut document.0;
    let id = next_id(definition.id_prefix, &circuit.parts);
    circuit.layout.insert(id.clone(), placement);
    circuit.parts.insert(
        id.clone(),
        Part {
            kind: kind.into(),
            record: Default::default(),
            parameters: definition
                .defaults
                .iter()
                .map(|&(key, value)| (key.to_owned(), value.into()))
                .collect(),
        },
    );
    let pins = circuit.nets.get_mut(&net).expect("segment net");
    pins.retain(|pin| !far.contains(pin));
    pins.push(format!("{id}.{first}"));
    let split = next_id("net", &circuit.nets);
    let far_side = std::iter::once(format!("{id}.{second}"))
        .chain(far)
        .collect();
    circuit.nets.insert(split, far_side);
    Ok(())
}

/// Renamed parts may use any case, so skip numbers taken in another case too.
fn next_id<T>(prefix: &str, entries: &BTreeMap<String, T>) -> String {
    (1..)
        .map(|index| format!("{prefix}{index}"))
        .find(|id| !entries.keys().any(|used| used.eq_ignore_ascii_case(id)))
        .expect("available ID")
}

const MAX_PART_ID_LENGTH: usize = 32;

fn receipt(
    before: &Document,
    after: &Document,
    before_revision: String,
    origins: &BTreeMap<&str, &str>,
) -> Receipt {
    let mut receipt = Receipt {
        before_revision,
        after_revision: after.revision(),
        created: Vec::new(),
        modified: Vec::new(),
        deleted: Vec::new(),
        nets: BTreeMap::new(),
        renamed: origins
            .iter()
            .filter(|(id, old)| id != old)
            .map(|(&id, &old)| (old.to_owned(), id.to_owned()))
            .collect(),
        shorted_sources: Vec::new(),
    };
    for (id, part) in &after.0.parts {
        match origins.get(id.as_str()).copied() {
            None => receipt.created.push(id.clone()),
            Some(old)
                if &before.0.parts[old] != part
                    || before.0.layout.get(old) != after.0.layout.get(id)
                    || part_definition(&part.kind).unwrap().pins.iter().any(|pin| {
                        let bend = |document: &Document, id: &str| {
                            let bends = document.0.bends.as_ref()?;
                            bends.get(&format!("{id}.{pin}")).cloned()
                        };
                        bend(before, old) != bend(after, id)
                    }) =>
            {
                receipt.modified.push(id.clone())
            }
            Some(_) => {}
        }
    }
    let survivors: BTreeSet<&str> = origins.values().copied().collect();
    receipt.deleted = before
        .0
        .parts
        .keys()
        .filter(|id| !survivors.contains(id.as_str()))
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
    receipt.shorted_sources = after
        .0
        .parts
        .iter()
        .filter(|(id, part)| {
            let definition = part_definition(&part.kind).unwrap();
            let shorted = |circuit: &CircuitV1, id: &str| {
                let [first, second] = definition.pins else {
                    return false;
                };
                let net = |pin| circuit.net_of(&format!("{id}.{pin}")).map(str::to_owned);
                net(first).is_some() && net(first) == net(second)
            };
            definition.voltage_source
                && shorted(&after.0, id)
                && !origins
                    .get(id.as_str())
                    .is_some_and(|old| shorted(&before.0, old))
        })
        .map(|(id, _)| id.clone())
        .collect();
    receipt
}
