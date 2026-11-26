use super::Error;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) const DEFAULT_INPUTS: &str = "|+-";
pub(super) const DEFAULT_SHAPE: &str = "round";

pub(crate) struct Sum {
    slots: Vec<u8>,
    pub(crate) round: bool,
}

impl Sum {
    pub(crate) fn read(id: &str, parameters: &BTreeMap<String, Value>) -> Result<Self, Error> {
        let inputs = parameters
            .get("inputs")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let slots = if !inputs.is_empty() && inputs.bytes().all(|slot| slot.is_ascii_digit()) {
            inputs
                .parse::<u16>()
                .ok()
                .filter(|&count| count > 0)
                .map(|count| vec![b'+'; usize::from(count)])
        } else if u16::try_from(inputs.len()).is_ok()
            && inputs.bytes().all(|slot| matches!(slot, b'+' | b'-' | b'|'))
            && inputs.bytes().any(|slot| slot != b'|')
        {
            Some(inputs.as_bytes().to_vec())
        } else {
            None
        }
        .ok_or_else(|| Error::invalid(id, "inputs must be a list of +, -, | with at least one sign, or a positive integer (up to 65535 slots)"))?;
        let round = match parameters.get("iconShape").and_then(Value::as_str) {
            Some("round") => true,
            Some("rectangular") => false,
            _ => return Err(Error::invalid(id, "iconShape must be round or rectangular")),
        };
        if let Some(key) = parameters
            .keys()
            .find(|key| !matches!(key.as_str(), "inputs" | "iconShape"))
        {
            return Err(Error::invalid(id, format!("unknown parameter: {key}")));
        }
        Ok(Self { slots, round })
    }

    pub(super) fn migrate_legacy(
        id: &str,
        parameters: &mut BTreeMap<String, Value>,
    ) -> Result<bool, Error> {
        if !parameters.contains_key("sign1") && !parameters.contains_key("sign2") {
            return Ok(false);
        }
        if parameters.len() != 2 {
            return Err(Error::invalid(
                id,
                "expected only sign1 and sign2 in a legacy sum",
            ));
        }
        let mut inputs = String::from("|");
        for key in ["sign1", "sign2"] {
            inputs.push(match parameters.get(key).and_then(Value::as_f64) {
                Some(1.0) => '+',
                Some(-1.0) => '-',
                _ => return Err(Error::invalid(id, format!("{key} must be +1 or -1"))),
            });
        }
        *parameters = BTreeMap::from([
            ("inputs".into(), inputs.into()),
            ("iconShape".into(), DEFAULT_SHAPE.into()),
        ]);
        Ok(true)
    }

    pub(crate) fn signs(&self) -> impl Iterator<Item = f64> + '_ {
        self.slots.iter().filter_map(|slot| match slot {
            b'+' => Some(1.0),
            b'-' => Some(-1.0),
            _ => None,
        })
    }

    pub(crate) fn output_column(&self) -> i32 {
        if self.round && self.slots.len() > 4 {
            (self.slots.len() / 2) as i32
        } else {
            1
        }
    }

    pub(crate) fn rows(&self) -> i32 {
        self.slots.len() as i32
    }

    pub(crate) fn ports(&self) -> impl Iterator<Item = (bool, [i32; 2])> + '_ {
        let last = self.slots.len() - 1;
        self.slots
            .iter()
            .enumerate()
            .filter_map(move |(index, &slot)| {
                (slot != b'|').then(|| {
                    let offset = if self.round {
                        // Spread slots over the top, left and bottom. Use a larger
                        // grid radius for dense lists so adjacent ports stay distinct.
                        let angle = std::f64::consts::PI * index as f64 / last.max(1) as f64;
                        let radius = f64::from(self.output_column());
                        [-angle.sin(), -angle.cos()].map(|unit| {
                            let distance = unit * radius;
                            (distance.abs() + 1e-10).round().copysign(distance) as i32
                        })
                    } else {
                        [-1, index as i32 - (self.slots.len() / 2) as i32]
                    };
                    (slot == b'+', offset)
                })
            })
    }
}
