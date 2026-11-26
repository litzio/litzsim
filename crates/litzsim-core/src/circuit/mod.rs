mod edit;
mod file;
mod library;
#[cfg(test)]
mod tests;

pub use edit::{EditResult, MirrorAxis, NetChange, Operation, Receipt, apply, undo};
#[cfg(windows)]
pub use file::REPLACE_RETRY_DELAYS;
pub(crate) use file::StagedFile;
pub use file::{Opened, SaveMode, open, save};
pub use library::{Measurement, PartDefinition, part_definition};

use serde::de::{Error as _, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// One-based position in the operation batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
}

impl Error {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            target_id: None,
            operation: None,
            expected: None,
            actual: None,
        }
    }

    pub(crate) fn invalid(target: &str, message: impl Into<String>) -> Self {
        let mut error = Self::new("invalidCircuit", message);
        error.target_id = Some(target.into());
        error
    }

    fn conflict(code: &'static str, expected: Option<&str>, actual: Option<&str>) -> Self {
        let mut error = Self::new(code, "Circuit revision conflict");
        error.expected = Some(expected.into());
        error.actual = Some(actual.into());
        error
    }
}

impl fmt::Display for Error {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(operation) = self.operation {
            write!(output, "operation {operation}: ")?;
        }
        if let Some(target) = &self.target_id {
            write!(output, "{target}: ")?;
        }
        output.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Document(CircuitV1);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CircuitV1 {
    #[serde(deserialize_with = "integer")]
    version: i32,
    #[serde(deserialize_with = "unique_map")]
    pub(crate) parts: BTreeMap<String, Part>,
    #[serde(deserialize_with = "unique_map")]
    pub(crate) nets: BTreeMap<String, Vec<String>>,
    #[serde(deserialize_with = "unique_map")]
    layout: BTreeMap<String, Placement>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_map"
    )]
    bends: Option<BTreeMap<String, Point>>,
    pub(crate) simulation: Simulation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Part {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    #[serde(deserialize_with = "unique_map")]
    pub(crate) parameters: BTreeMap<String, Value>,
}

impl Part {
    fn series_resistance_at_default(&self) -> Option<&'static str> {
        let (key, default) = part_definition(&self.kind)?.series_resistance()?;
        (self.parameters.get(key)?.as_f64() == Some(default)).then_some(key)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Placement {
    #[serde(deserialize_with = "integer")]
    pub(crate) x: i32,
    #[serde(deserialize_with = "integer")]
    pub(crate) y: i32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_integer"
    )]
    pub(crate) rotation: Option<i32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub(crate) mirror: Option<Mirror>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Mirror {
    Horizontal,
    Vertical,
    Both,
}

impl Placement {
    fn normalize_orientation(&mut self) {
        let mut rotation = self.rotation.unwrap_or_default();
        if self.mirror == Some(Mirror::Both) {
            self.mirror = None;
            rotation = (rotation + 180) % 360;
        }
        self.rotation = (rotation != 0).then_some(rotation);
    }

    pub(crate) fn orient(self, [x, y]: [i32; 2]) -> [i32; 2] {
        let [mut x, mut y] = match self.rotation.unwrap_or_default() {
            0 => [x, y],
            90 => [-y, x],
            180 => [-x, -y],
            270 => [y, -x],
            _ => unreachable!("validated rotation"),
        };
        if self.mirror == Some(Mirror::Horizontal) {
            x = -x;
        }
        if self.mirror == Some(Mirror::Vertical) {
            y = -y;
        }
        [x, y]
    }

    fn pin_position(self, offset: [i32; 2]) -> [i64; 2] {
        let [x, y] = self.orient(offset);
        [
            i64::from(self.x) + i64::from(x),
            i64::from(self.y) + i64::from(y),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Point {
    #[serde(deserialize_with = "integer")]
    x: i32,
    #[serde(deserialize_with = "integer")]
    y: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Simulation {
    #[serde(rename = "timeStep_s")]
    pub(crate) time_step_s: f64,
    #[serde(rename = "stopTime_s")]
    pub(crate) stop_time_s: f64,
    #[serde(
        rename = "outputStep_s",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub(crate) output_step_s: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    model: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection<'a> {
    pub document: &'a Document,
    pub revision: String,
    pub pin_nets: BTreeMap<String, Option<&'a str>>,
    pub pin_positions: BTreeMap<String, [i64; 2]>,
    pub measurements: BTreeMap<&'a str, Measurement>,
    pub missing_pins: Vec<String>,
}

pub(crate) struct LayoutPart<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub parameters: &'a BTreeMap<String, Value>,
    pub placement: Placement,
}

impl Document {
    pub(crate) fn layout_parts(&self) -> impl Iterator<Item = LayoutPart<'_>> {
        self.0.parts.iter().map(|(id, part)| LayoutPart {
            id,
            kind: &part.kind,
            parameters: &part.parameters,
            placement: self.0.layout.get(id).copied().unwrap_or_default(),
        })
    }

    pub(crate) fn layout_nets(&self) -> &BTreeMap<String, Vec<String>> {
        &self.0.nets
    }

    pub(crate) fn layout_bend(&self, pin: &str) -> Option<[i64; 2]> {
        let bend = self.0.bends.as_ref()?.get(pin)?;
        Some([i64::from(bend.x), i64::from(bend.y)])
    }

    pub fn new() -> Self {
        Self(CircuitV1 {
            version: 1,
            parts: BTreeMap::new(),
            nets: BTreeMap::new(),
            layout: BTreeMap::new(),
            bends: None,
            simulation: Simulation {
                time_step_s: 1e-6,
                stop_time_s: 0.001,
                output_step_s: None,
                model: None,
            },
        })
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Self::read(bytes).map(|(document, _)| document)
    }

    fn read(bytes: &[u8]) -> Result<(Self, bool), Error> {
        let mut circuit: CircuitV1 = serde_json::from_slice(bytes)
            .map_err(|error| Error::new("invalidCircuit", error.to_string()))?;
        circuit.validate()?;
        let mut has_explicit_resistance_defaults = false;
        for position in circuit.layout.values_mut() {
            position.normalize_orientation();
        }
        for part in circuit.parts.values_mut() {
            has_explicit_resistance_defaults |= part.series_resistance_at_default().is_some();
            if let Some((key, default)) = part_definition(&part.kind)
                .expect("validated part")
                .series_resistance()
            {
                if !part.parameters.contains_key(key) {
                    part.parameters.insert(key.into(), default.into());
                }
            }
            for value in part.parameters.values_mut() {
                *value = value.as_f64().expect("validated parameter").into();
            }
        }
        Ok((Self(circuit), has_explicit_resistance_defaults))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        // Inspection and edit responses expose the effective parameters.
        // Files and their canonical revisions omit the new optional defaults.
        let mut stored = self.0.clone();
        for part in stored.parts.values_mut() {
            if let Some(key) = part.series_resistance_at_default() {
                part.parameters.remove(key);
            }
        }
        let mut bytes = serde_json::to_vec_pretty(&stored).expect("serializable circuit");
        bytes.push(b'\n');
        bytes
    }

    pub fn revision(&self) -> String {
        revision(&self.to_bytes())
    }

    pub fn stop_time_s(&self) -> f64 {
        self.0.simulation.stop_time_s
    }

    pub(crate) fn circuit(&self) -> &CircuitV1 {
        &self.0
    }

    /// Missing terminals are diagnosed separately from file/operation validation.
    pub fn inspect(&self) -> Inspection<'_> {
        let pin_nets = self.0.pin_nets();
        let mut pin_positions = BTreeMap::new();
        let mut measurements = BTreeMap::new();
        for (id, part) in &self.0.parts {
            let definition = part_definition(&part.kind).expect("validated part");
            let position = self.0.layout.get(id).copied().unwrap_or_default();
            for (pin, offset) in definition.pins.iter().zip(definition.pin_offsets) {
                let terminal = format!("{id}.{pin}");
                pin_positions.insert(terminal, position.pin_position(*offset));
            }
            if let Some(measurement) = definition.measurement {
                measurements.insert(id.as_str(), measurement);
            }
        }
        let missing_pins = pin_nets
            .iter()
            .filter(|(_, net)| net.is_none())
            .map(|(pin, _)| pin.clone())
            .collect();
        Inspection {
            document: self,
            revision: self.revision(),
            pin_nets,
            pin_positions,
            measurements,
            missing_pins,
        }
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl CircuitV1 {
    pub(crate) fn pin_nets(&self) -> BTreeMap<String, Option<&str>> {
        let mut pin_nets = BTreeMap::new();
        for (id, part) in &self.parts {
            let definition = part_definition(&part.kind).expect("validated part");
            for pin in definition.pins {
                pin_nets.insert(format!("{id}.{pin}"), None);
            }
        }
        for (net, pins) in &self.nets {
            for pin in pins {
                *pin_nets.get_mut(pin).expect("validated terminal") = Some(net.as_str());
            }
        }
        pin_nets
    }

    fn validate(&self) -> Result<(), Error> {
        if self.version != 1 {
            return Err(Error::invalid(
                "version",
                format!("unsupported circuit version: {}", self.version),
            ));
        }
        self.simulation.validate()?;
        for (id, part) in &self.parts {
            if !identifier(id) {
                return Err(Error::invalid(id, "invalid part ID"));
            }
            let definition = part_definition(&part.kind).ok_or_else(|| {
                Error::invalid(id, format!("unsupported part type: {}", part.kind))
            })?;
            for (key, value) in &part.parameters {
                if !definition.defaults.iter().any(|(name, _)| name == key) {
                    return Err(Error::invalid(id, format!("unknown parameter: {key}")));
                }
                if !value.as_f64().is_some_and(f64::is_finite) {
                    return Err(Error::invalid(
                        id,
                        format!("invalid value for parameter: {key}"),
                    ));
                }
            }
            let optional = definition.series_resistance().map(|(key, _)| key);
            for (key, _) in definition.defaults {
                if Some(*key) != optional && !part.parameters.contains_key(*key) {
                    return Err(Error::invalid(id, format!("missing parameter: {key}")));
                }
            }
        }
        let mut connected = std::collections::BTreeSet::new();
        for (id, pins) in &self.nets {
            if !identifier(id) {
                return Err(Error::invalid(id, "invalid net ID"));
            }
            for pin in pins {
                self.require_pin(pin)?;
                if !connected.insert(pin) {
                    return Err(Error::invalid(pin, "multiply connected terminal"));
                }
            }
        }
        for (id, pos) in &self.layout {
            self.require_part(id)?;
            if !matches!(pos.rotation, None | Some(0 | 90 | 180 | 270)) {
                return Err(Error::invalid(id, "invalid rotation"));
            }
        }
        if let Some(bends) = &self.bends {
            for pin in bends.keys() {
                self.require_pin(pin)?;
            }
        }
        Ok(())
    }

    fn require_part(&self, id: &str) -> Result<&Part, Error> {
        self.parts
            .get(id)
            .ok_or_else(|| Error::invalid(id, "unknown part"))
    }

    fn require_pin(&self, terminal: &str) -> Result<(), Error> {
        if let Some((id, pin)) = terminal.split_once('.')
            && self
                .parts
                .get(id)
                .and_then(|part| part_definition(&part.kind))
                .is_some_and(|definition| definition.pins.contains(&pin))
        {
            return Ok(());
        }
        Err(Error::invalid(terminal, "unknown terminal"))
    }
}

impl Simulation {
    fn validate(&self) -> Result<(), Error> {
        if let Some(model) = &self.model
            && model != "mna"
        {
            return Err(Error::invalid(
                "simulation.model",
                format!("unsupported simulation model: {model}"),
            ));
        }
        for (name, value) in [
            ("timeStep_s", self.time_step_s),
            ("stopTime_s", self.stop_time_s),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(Error::invalid(
                    name,
                    "expected positive finite simulation time",
                ));
            }
        }
        if let Some(step) = self.output_step_s {
            let ratio = step / self.time_step_s;
            let rounded = ratio.round();
            if !(1.0..=1e12).contains(&rounded) || (ratio - rounded).abs() > 1e-8 {
                return Err(Error::invalid(
                    "outputStep_s",
                    "must be a positive integer multiple of timeStep_s",
                ));
            }
        }
        Ok(())
    }
}

fn identifier(id: &str) -> bool {
    let mut bytes = id.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn revision(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn integer<'de, D: Deserializer<'de>>(input: D) -> Result<i32, D::Error> {
    let number = f64::deserialize(input)?;
    let integer = number as i32;
    if f64::from(integer) == number {
        Ok(integer)
    } else {
        Err(D::Error::custom("expected a 32-bit integer"))
    }
}

fn optional_integer<'de, D: Deserializer<'de>>(input: D) -> Result<Option<i32>, D::Error> {
    integer(input).map(Some)
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    input: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(input).map(Some)
}

fn optional_map<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    input: D,
) -> Result<Option<BTreeMap<String, T>>, D::Error> {
    unique_map(input).map(Some)
}

fn unique_map<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    input: D,
) -> Result<BTreeMap<String, T>, D::Error> {
    struct Unique<T>(std::marker::PhantomData<T>);

    impl<'de, T: Deserialize<'de>> Visitor<'de> for Unique<T> {
        type Value = BTreeMap<String, T>;

        fn expecting(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
            output.write_str("an object with unique keys")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
            let mut map = BTreeMap::new();
            while let Some(key) = input.next_key::<String>()? {
                if map.contains_key(&key) {
                    return Err(A::Error::custom(format!("duplicate key: {key}")));
                }
                map.insert(key, input.next_value()?);
            }
            Ok(map)
        }
    }

    input.deserialize_map(Unique(std::marker::PhantomData))
}
