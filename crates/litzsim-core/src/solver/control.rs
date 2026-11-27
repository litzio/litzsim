use super::same_clock_time;
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
    Sum(Vec<f64>),
    Gain(f64),
    Limiter {
        min: f64,
        max: f64,
    },
    Integrator {
        state: f64,
        min: f64,
        max: f64,
    },
    Pi {
        state: f64,
        kp: f64,
        ki: f64,
        min: f64,
        max: f64,
    },
    DutyPwm,
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
            "sum" => Self::Sum(part.sum().expect("validated sum").signs().collect()),
            "gain" => Self::Gain(param("gain")),
            "limiter" | "integrator" | "pi" => {
                let (min, max) = (param("min"), param("max"));
                if min > max {
                    let name = match part.kind.as_str() {
                        "limiter" => "Limiter",
                        "integrator" => "Integrator",
                        _ => "PI",
                    };
                    return Err(Error::invalid(
                        id,
                        format!("{name} min must not exceed max"),
                    ));
                }
                match part.kind.as_str() {
                    "integrator" => Self::Integrator {
                        state: param("initialValue").clamp(min, max),
                        min,
                        max,
                    },
                    "pi" => Self::Pi {
                        state: param("initialIntegral"),
                        kp: param("kp"),
                        ki: param("ki"),
                        min,
                        max,
                    },
                    _ => Self::Limiter { min, max },
                }
            }
            "dutyPwm" => Self::DutyPwm,
            _ => unreachable!("control library and equations use the same block types"),
        })
    }
}

struct Block<'a> {
    id: &'a str,
    equation: Equation,
    inputs: Vec<usize>,
    needs_sense: bool,
    observed: bool,
}

pub(super) struct Control<'a> {
    blocks: Vec<Block<'a>>,
    order: Vec<usize>,
    values: Vec<f64>,
    recorded: Vec<usize>,
    probes: Vec<(&'a str, usize)>,
    pwm_inputs: BTreeMap<&'a str, usize>,
    period: f64,
    tick: u64,
}

impl<'a> Control<'a> {
    pub(super) fn compile(
        document: &'a Document,
        signal_probes: &BTreeMap<&'a str, &'a str>,
    ) -> Result<Option<Self>, Error> {
        let circuit = document.circuit();
        let parts: Vec<_> = circuit
            .parts
            .iter()
            .filter(|(_, part)| {
                part_definition(&part.kind)
                    .expect("validated part")
                    .is_control()
                    || part.kind == "dutyPwm"
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
        let mut pwm_inputs = BTreeMap::new();
        for (id, part) in parts {
            let mut inputs = Vec::new();
            for (pin, _, _) in part
                .pins()
                .into_iter()
                .filter(|(_, _, kind)| *kind == PinKind::SignalInput)
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
            if part.kind == "dutyPwm" {
                pwm_inputs.insert(id.as_str(), blocks.len());
            }
            blocks.push(Block {
                id,
                equation: Equation::from_part(id, part)?,
                inputs,
                needs_sense: false,
                observed: part.record.output,
            });
        }
        let order = evaluation_order(&blocks)?;
        for &index in &order {
            let block = &blocks[index];
            blocks[index].needs_sense = matches!(block.equation, Equation::Sensor(..))
                || block
                    .inputs
                    .iter()
                    .any(|&driver| blocks[driver].needs_sense);
        }
        let period = circuit.simulation.control_sample_period_s.ok_or_else(|| {
            Error::invalid(
                "controlSamplePeriod_s",
                "Control blocks require a controlSamplePeriod_s",
            )
        })?;
        // Probes share the driver's held value and recording events without
        // participating in the dependency graph.
        let probes = signal_probes
            .iter()
            .map(|(&id, net)| {
                let &driver = drivers.get(net).ok_or_else(|| {
                    Error::invalid(
                        id,
                        format!("Unconnected signal input {id}.p: expected a signal output"),
                    )
                })?;
                blocks[driver].observed = true;
                Ok((id, driver))
            })
            .collect::<Result<_, Error>>()?;
        let values = vec![0.0; blocks.len()];
        Ok(Some(Self {
            blocks,
            order,
            values,
            recorded,
            probes,
            pwm_inputs,
            period,
            tick: 0,
        }))
    }

    pub(super) fn next_sample(&self) -> f64 {
        self.tick as f64 * self.period
    }

    pub(super) fn due(&self, time: f64) -> bool {
        same_clock_time(self.next_sample(), time)
    }

    pub(super) fn pwm_input(&self, id: &str) -> Option<usize> {
        self.pwm_inputs.get(id).copied()
    }

    pub(super) fn held(&self, index: usize) -> f64 {
        self.values[index]
    }

    /// Seed duties that can be determined without the initial electrical solve.
    pub(super) fn prepare_initial(&mut self) -> Result<(), Error> {
        self.evaluate(Some(false), false, |_, _| {
            unreachable!("sensor-independent block")
        })?;
        Ok(())
    }

    /// Evaluate direct dependencies in order, then hold all values until the next tick.
    pub(super) fn sample(
        &mut self,
        initial_prepared: bool,
        include_unrecorded_changes: bool,
        sense: impl FnMut(&str, RecordQuantity) -> f64,
    ) -> Result<bool, Error> {
        let changed = self.evaluate(
            initial_prepared.then_some(true),
            include_unrecorded_changes,
            sense,
        )?;
        self.tick += 1;
        Ok(changed)
    }

    fn evaluate(
        &mut self,
        needs_sense: Option<bool>,
        include_unrecorded_changes: bool,
        mut sense: impl FnMut(&str, RecordQuantity) -> f64,
    ) -> Result<bool, Error> {
        let time = self.next_sample();
        let mut changed = false;
        for &index in &self.order {
            let block = &mut self.blocks[index];
            if needs_sense.is_some_and(|required| block.needs_sense != required) {
                continue;
            }
            let input = |pin: usize| self.values[block.inputs[pin]];
            let value = match &mut block.equation {
                Equation::Sensor(quantity, gain) => sense(block.id, *quantity) * *gain,
                Equation::Constant(value) => *value,
                Equation::Step { at, before, after } => {
                    if *at > time && !same_clock_time(*at, time) {
                        *before
                    } else {
                        *after
                    }
                }
                Equation::Ramp {
                    start,
                    slope,
                    final_value,
                } => ((time - *start).max(0.0) * *slope)
                    .clamp(final_value.min(0.0), final_value.max(0.0)),
                Equation::Sum(signs) => {
                    let mut terms = signs
                        .iter()
                        .enumerate()
                        .map(|(index, sign)| sign * input(index));
                    // Starting with the first signed term preserves the old sum's
                    // arithmetic, including a possible negative zero.
                    let first = terms.next().expect("sum has at least one input");
                    terms.fold(first, |total, term| total + term)
                }
                Equation::Gain(gain) => *gain * input(0),
                Equation::Limiter { min, max } => input(0).clamp(*min, *max),
                Equation::Integrator { state, min, max } => {
                    *state = (*state + input(0) * self.period).clamp(*min, *max);
                    *state
                }
                Equation::Pi {
                    state,
                    kp,
                    ki,
                    min,
                    max,
                } => {
                    let error = input(0);
                    let increment = (*ki * error) * self.period;
                    let integral = *state + increment;
                    let output = *kp * error + integral;
                    if !((output > *max && increment > 0.0) || (output < *min && increment < 0.0)) {
                        *state = integral;
                    }
                    output.clamp(*min, *max)
                }
                Equation::DutyPwm => input(0).clamp(0.0, 1.0),
            };
            if !value.is_finite() {
                return Err(Error::invalid(
                    block.id,
                    format!("Non-finite control output at {time} s"),
                ));
            }
            if value != self.values[index] && (include_unrecorded_changes || block.observed) {
                changed = true;
            }
            self.values[index] = value;
        }
        Ok(changed)
    }

    pub(super) fn channels(&self) -> impl Iterator<Item = Channel> + '_ {
        self.recorded
            .iter()
            .map(|&index| format!("{}_out", self.blocks[index].id))
            .chain(self.probes.iter().map(|&(id, _)| id.into()))
            .map(|name| Channel {
                name,
                unit: RecordQuantity::Output.unit().into(),
            })
    }

    pub(super) fn recorded_values(&self) -> impl Iterator<Item = f64> + Clone + '_ {
        self.recorded
            .iter()
            .copied()
            .chain(self.probes.iter().map(|&(_, driver)| driver))
            .map(|index| self.values[index])
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
