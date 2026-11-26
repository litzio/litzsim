use super::CLOCK_EPSILON;
use crate::circuit::{Document, Error, Part, PinKind, RecordQuantity, part_definition};
use crate::waves::Channel;
use std::collections::{BTreeMap, VecDeque};

enum Equation {
    Sensor(RecordQuantity, f64),
    Constant(f64),
    Step {
        at: f64,
        before: f64,
        after: f64,
    },
    Ramp {
        start: f64,
        slope: f64,
        final_value: f64,
    },
    Sum(f64, f64),
    Gain(f64),
    Limiter {
        min: f64,
        max: f64,
    },
}

impl Equation {
    fn from_part(id: &str, part: &Part) -> Result<Self, Error> {
        let param = |key: &str| part.parameters[key].as_f64().expect("validated parameter");
        Ok(match part.kind.as_str() {
            "voltageSensor" => Self::Sensor(RecordQuantity::Voltage, param("gain")),
            "currentSensor" => Self::Sensor(RecordQuantity::Current, param("gain")),
            "constant" => Self::Constant(param("value")),
            "step" => {
                let at = param("stepTime_s");
                if at < 0.0 {
                    return Err(Error::invalid(id, "stepTime_s must be nonnegative"));
                }
                Self::Step {
                    at,
                    before: param("before"),
                    after: param("after"),
                }
            }
            "ramp" => {
                let start = param("startTime_s");
                let slope = param("slope_per_s");
                let final_value = param("finalValue");
                if start < 0.0 {
                    return Err(Error::invalid(id, "startTime_s must be nonnegative"));
                }
                if final_value != 0.0 && (slope == 0.0 || slope.signum() != final_value.signum()) {
                    return Err(Error::invalid(
                        id,
                        "Ramp slope must point toward finalValue",
                    ));
                }
                Self::Ramp {
                    start,
                    slope,
                    final_value,
                }
            }
            "sum" => {
                let (a, b) = (param("sign1"), param("sign2"));
                if a.abs() != 1.0 || b.abs() != 1.0 {
                    return Err(Error::invalid(id, "Sum signs must be +1 or -1"));
                }
                Self::Sum(a, b)
            }
            "gain" => Self::Gain(param("gain")),
            "limiter" => {
                let (min, max) = (param("min"), param("max"));
                if min > max {
                    return Err(Error::invalid(id, "Limiter min must not exceed max"));
                }
                Self::Limiter { min, max }
            }
            _ => unreachable!("control library and equations use the same block types"),
        })
    }
}

struct Block<'a> {
    id: &'a str,
    equation: Equation,
    inputs: Vec<usize>,
}

pub(super) struct Control<'a> {
    blocks: Vec<Block<'a>>,
    order: Vec<usize>,
    values: Vec<f64>,
    recorded: Vec<usize>,
    period: f64,
    tick: u64,
}

impl<'a> Control<'a> {
    pub(super) fn compile(document: &'a Document) -> Result<Option<Self>, Error> {
        let circuit = document.circuit();
        let parts: Vec<_> = circuit
            .parts
            .iter()
            .filter(|(_, part)| {
                part_definition(&part.kind)
                    .expect("validated part")
                    .is_control()
            })
            .collect();
        if parts.is_empty() {
            return Ok(None);
        }
        let indices: BTreeMap<_, _> = parts
            .iter()
            .enumerate()
            .map(|(index, (id, _))| (id.as_str(), index))
            .collect();
        let pin_nets = circuit.pin_nets();
        let mut drivers = BTreeMap::new();
        for (net, pins) in &circuit.nets {
            for terminal in pins {
                if circuit.require_pin(terminal)? == PinKind::SignalOutput {
                    let (id, _) = terminal.split_once('.').expect("validated terminal");
                    drivers.insert(net.as_str(), indices[id]);
                }
            }
        }
        let mut blocks = Vec::new();
        let mut recorded = Vec::new();
        for (id, part) in parts {
            let definition = part_definition(&part.kind).expect("validated part");
            let mut inputs = Vec::new();
            for pin in definition
                .pins
                .iter()
                .filter(|pin| definition.pin_kind(pin) == PinKind::SignalInput)
            {
                let terminal = format!("{id}.{pin}");
                let driver = pin_nets[&terminal].and_then(|net| drivers.get(net));
                let driver = driver.ok_or_else(|| {
                    Error::invalid(
                        id,
                        format!("Unconnected signal input {terminal}: expected a signal output"),
                    )
                })?;
                inputs.push(*driver);
            }
            if part.record.output {
                recorded.push(blocks.len());
            }
            blocks.push(Block {
                id,
                equation: Equation::from_part(id, part)?,
                inputs,
            });
        }
        let order = evaluation_order(&blocks)?;
        let period = circuit.simulation.control_sample_period_s.ok_or_else(|| {
            Error::invalid(
                "controlSamplePeriod_s",
                "Control blocks require a controlSamplePeriod_s",
            )
        })?;
        let values = vec![0.0; blocks.len()];
        Ok(Some(Self {
            blocks,
            order,
            values,
            recorded,
            period,
            tick: 0,
        }))
    }

    pub(super) fn next_sample(&self) -> f64 {
        self.tick as f64 * self.period
    }

    pub(super) fn due(&self, time: f64) -> bool {
        (self.next_sample() - time).abs() <= CLOCK_EPSILON * time
    }

    /// Evaluate direct dependencies in order, then hold all values until the next tick.
    pub(super) fn sample(
        &mut self,
        mut sense: impl FnMut(&str, RecordQuantity) -> f64,
    ) -> Result<bool, Error> {
        let time = self.next_sample();
        let mut changed = false;
        for &index in &self.order {
            let block = &self.blocks[index];
            let input = |pin: usize| self.values[block.inputs[pin]];
            let value = match block.equation {
                Equation::Sensor(quantity, gain) => sense(block.id, quantity) * gain,
                Equation::Constant(value) => value,
                Equation::Step { at, before, after } => {
                    if at - time > CLOCK_EPSILON * time {
                        before
                    } else {
                        after
                    }
                }
                Equation::Ramp {
                    start,
                    slope,
                    final_value,
                } => ((time - start).max(0.0) * slope)
                    .clamp(final_value.min(0.0), final_value.max(0.0)),
                Equation::Sum(a, b) => a * input(0) + b * input(1),
                Equation::Gain(gain) => gain * input(0),
                Equation::Limiter { min, max } => input(0).clamp(min, max),
            };
            if !value.is_finite() {
                return Err(Error::invalid(
                    block.id,
                    format!("Non-finite control output at {time} s"),
                ));
            }
            if value != self.values[index] && self.recorded.binary_search(&index).is_ok() {
                changed = true;
            }
            self.values[index] = value;
        }
        self.tick += 1;
        Ok(changed)
    }

    pub(super) fn channels(&self) -> impl Iterator<Item = Channel> + '_ {
        self.recorded.iter().map(|&index| Channel {
            name: format!("{}_out", self.blocks[index].id),
            unit: RecordQuantity::Output.unit().into(),
        })
    }

    pub(super) fn recorded_values(&self) -> impl Iterator<Item = f64> + Clone + '_ {
        self.recorded.iter().map(|&index| self.values[index])
    }
}

fn evaluation_order(blocks: &[Block<'_>]) -> Result<Vec<usize>, Error> {
    let mut remaining: Vec<_> = blocks.iter().map(|block| block.inputs.len()).collect();
    let mut consumers = vec![Vec::new(); blocks.len()];
    let mut ready = VecDeque::new();
    for (index, block) in blocks.iter().enumerate() {
        if block.inputs.is_empty() {
            ready.push_back(index);
        }
        for &driver in &block.inputs {
            consumers[driver].push(index);
        }
    }
    let mut order = Vec::with_capacity(blocks.len());
    while let Some(index) = ready.pop_front() {
        order.push(index);
        for &consumer in &consumers[index] {
            remaining[consumer] -= 1;
            if remaining[consumer] == 0 {
                ready.push_back(consumer);
            }
        }
    }
    if let Some(mut index) = remaining.iter().position(|&count| count > 0) {
        // Walk the remaining dependencies until a block repeats. Report the
        // cycle itself even when the first blocked node is downstream of it.
        let mut path = Vec::new();
        let mut seen = BTreeMap::new();
        loop {
            if let Some(start) = seen.insert(index, path.len()) {
                let ids: Vec<_> = path[start..]
                    .iter()
                    .map(|&i: &usize| blocks[i].id)
                    .collect();
                return Err(Error::invalid(
                    blocks[index].id,
                    format!("Algebraic loop: {}", ids.join(", ")),
                ));
            }
            path.push(index);
            index = *blocks[index]
                .inputs
                .iter()
                .find(|&&driver| remaining[driver] > 0)
                .expect("unresolved dependency");
        }
    }
    Ok(order)
}
