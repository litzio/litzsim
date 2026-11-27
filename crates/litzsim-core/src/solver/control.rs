use super::same_clock_time;
use crate::circuit::{
    ControlTask, Document, Error, Part, PinKind, RecordQuantity, Soc, part_definition,
};
use crate::waves::Channel;
use std::collections::{BTreeMap, VecDeque};

pub(super) enum Equation {
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
    Sampler,
}

impl Equation {
    pub(super) fn from_part(id: &str, part: &Part) -> Result<Self, Error> {
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
            "sampler" => Self::Sampler,
            _ => unreachable!("control library and equations use the same block types"),
        })
    }

    pub(super) fn source_value(&self, time: f64) -> f64 {
        match *self {
            Self::Constant(value) => value,
            Self::Step { at, before, after } => {
                if at > time && !same_clock_time(at, time) {
                    before
                } else {
                    after
                }
            }
            Self::Ramp {
                start,
                slope,
                final_value,
            } => {
                ((time - start).max(0.0) * slope).clamp(final_value.min(0.0), final_value.max(0.0))
            }
            _ => unreachable!("expected an independent control source"),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct TickClock {
    period: f64,
    frequency: Option<f64>,
    pub(super) position: f64,
}

impl TickClock {
    fn fixed(period: f64, position: f64) -> Self {
        Self {
            period,
            frequency: None,
            position,
        }
    }

    fn pwm(frequency: f64, position: f64) -> Self {
        Self {
            period: 1.0 / frequency,
            frequency: Some(frequency),
            position,
        }
    }

    fn at_position(self, position: f64) -> Self {
        Self { position, ..self }
    }

    pub(super) fn time(self, tick: u64) -> f64 {
        let cycle = tick as f64 + self.position;
        self.frequency
            .map_or_else(|| cycle * self.period, |frequency| cycle / frequency)
    }

    pub(super) fn period(self) -> f64 {
        self.period
    }
}

fn pwm_frequency(document: &Document, id: &str) -> Result<f64, Error> {
    let frequency = document.circuit().parts[id].parameters["frequency_Hz"]
        .as_f64()
        .expect("validated parameter");
    if frequency <= 0.0 || !(1.0 / frequency).is_finite() {
        return Err(Error::invalid(
            id,
            "PWM frequency must have a positive, finite period",
        ));
    }
    Ok(frequency)
}

pub(super) fn task_clock(document: &Document) -> Result<TickClock, Error> {
    Ok(match &document.circuit().simulation.control_task {
        Some(ControlTask::Fixed { period_s, phase }) => TickClock::fixed(*period_s, *phase),
        Some(ControlTask::Pwm { pwm, position }) => {
            TickClock::pwm(pwm_frequency(document, pwm)?, *position)
        }
        None => {
            return Err(Error::invalid(
                "controlTask",
                "Control blocks require a controlTask",
            ));
        }
    })
}

enum SampleClock {
    Periodic { clock: TickClock, tick: u64 },
    PwmCenter { pwm: String, on: bool, next: f64 },
}

impl SampleClock {
    fn new(document: &Document, task: TickClock, soc: Soc) -> Result<Self, Error> {
        let (pwm, on) = match soc {
            Soc::Task { position } => {
                return Ok(Self::Periodic {
                    clock: task.at_position(position),
                    tick: 0,
                });
            }
            Soc::OnCenter { pwm } => (pwm, true),
            Soc::OffCenter { pwm } => (pwm, false),
        };
        let part = &document.circuit().parts[&pwm];
        if part.kind == "dutyPwm" {
            Ok(Self::PwmCenter {
                pwm,
                on,
                next: f64::INFINITY,
            })
        } else {
            let duty = part.parameters["duty"]
                .as_f64()
                .expect("validated parameter");
            Ok(Self::Periodic {
                clock: TickClock::pwm(pwm_frequency(document, &pwm)?, midpoint(on, duty)),
                tick: 0,
            })
        }
    }

    fn next(&self) -> f64 {
        match *self {
            Self::Periodic { clock, tick } => clock.time(tick),
            Self::PwmCenter { next, .. } => next,
        }
    }

    fn advance(&mut self) {
        match self {
            Self::Periodic { tick, .. } => *tick += 1,
            Self::PwmCenter { next, .. } => *next = f64::INFINITY,
        }
    }
}

fn midpoint(on: bool, duty: f64) -> f64 {
    if on { duty / 2.0 } else { (1.0 + duty) / 2.0 }
}

struct Sampler {
    index: usize,
    clock: SampleClock,
    boundary_input: Option<f64>,
}

struct Block<'a> {
    id: &'a str,
    equation: Equation,
    inputs: Vec<usize>,
    deferred: bool,
    observed: bool,
}

pub(super) struct Control<'a> {
    blocks: Vec<Block<'a>>,
    order: Vec<usize>,
    values: Vec<f64>,
    recorded: Vec<usize>,
    probes: Vec<(&'a str, usize)>,
    pwm_inputs: BTreeMap<&'a str, usize>,
    clock: TickClock,
    tick: u64,
    samplers: Vec<Sampler>,
    captures: Vec<(usize, f64)>,
}

impl<'a> Control<'a> {
    pub(super) fn compile(
        document: &'a Document,
        connections: &crate::circuit::Connections<'a>,
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
        let mut drivers = BTreeMap::new();
        for (&id, net) in connections.nets.iter().filter(|(_, net)| net.signal) {
            for terminal in &net.terminals {
                if circuit.require_pin(terminal)? == PinKind::SignalOutput {
                    let (part, _) = terminal.split_once('.').expect("validated terminal");
                    drivers.insert(id, indices[part]);
                }
            }
        }
        let clock = task_clock(document)?;
        let mut blocks = Vec::new();
        let mut samplers = Vec::new();
        let mut recorded = Vec::new();
        let mut pwm_inputs = BTreeMap::new();
        for &(id, part) in &parts {
            let mut inputs = Vec::new();
            for (pin, _, _) in part
                .pins()
                .into_iter()
                .filter(|(_, _, kind)| *kind == PinKind::SignalInput)
            {
                let terminal = format!("{id}.{pin}");
                let net = connections.net_of(&terminal);
                let driver = net.and_then(|net| drivers.get(net));
                let driver = driver.ok_or_else(|| {
                    let error = Error::invalid(
                        id,
                        format!("Unconnected signal input {terminal}: expected a signal output"),
                    );
                    match net {
                        Some(net) => connections.nets[net].describe_labels(error, false),
                        None => error,
                    }
                })?;
                let (sensor_id, source) = parts[*driver];
                if part.kind != "sampler"
                    && matches!(source.kind.as_str(), "voltageSensor" | "currentSensor")
                {
                    let error = Error::invalid(
                        sensor_id,
                        format!(
                            "Sensor {sensor_id} must feed a sampler before control input {terminal}"
                        ),
                    );
                    return Err(
                        connections.nets[net.expect("driven net")].describe_labels(error, false)
                    );
                }
                inputs.push(*driver);
            }
            if part.record.output {
                recorded.push(blocks.len());
            }
            if part.kind == "dutyPwm" {
                pwm_inputs.insert(id.as_str(), blocks.len());
            }
            if part.kind == "sampler" {
                samplers.push(Sampler {
                    index: blocks.len(),
                    clock: SampleClock::new(document, clock, Soc::read(id, &part.parameters)?)?,
                    boundary_input: None,
                });
            }
            blocks.push(Block {
                id,
                equation: Equation::from_part(id, part)?,
                inputs,
                deferred: false,
                observed: part.record.output,
            });
        }
        let order = evaluation_order(&blocks)?;
        for &index in &order {
            let block = &blocks[index];
            blocks[index].deferred =
                matches!(block.equation, Equation::Sensor(..) | Equation::Sampler)
                    || block.inputs.iter().any(|&driver| blocks[driver].deferred);
        }
        // Probes share the driver's held value and recording events without
        // participating in the dependency graph.
        let probes = signal_probes
            .iter()
            .map(|(&id, net)| {
                let &driver = drivers.get(net).ok_or_else(|| {
                    connections.nets[net].describe_labels(
                        Error::invalid(
                            id,
                            format!("Unconnected signal input {id}.p: expected a signal output"),
                        ),
                        false,
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
            clock,
            tick: 0,
            captures: Vec::with_capacity(samplers.len()),
            samplers,
        }))
    }

    fn next_task(&self) -> f64 {
        self.clock.time(self.tick)
    }

    pub(super) fn next_event(&self) -> f64 {
        self.samplers
            .iter()
            .fold(self.next_task(), |next, sampler| {
                next.min(sampler.clock.next())
            })
    }

    pub(super) fn due(&self, time: f64) -> bool {
        same_clock_time(self.next_task(), time)
    }

    pub(super) fn pwm_input(&self, id: &str) -> Option<usize> {
        self.pwm_inputs.get(id).copied()
    }

    pub(super) fn held(&self, index: usize) -> f64 {
        self.values[index]
    }

    pub(super) fn initially_known(&self, index: usize) -> bool {
        self.due(0.0) && !self.blocks[index].deferred
    }

    pub(super) fn preserve_pwm_inputs(&mut self, id: &str) {
        for sampler in &mut self.samplers {
            if matches!(&sampler.clock, SampleClock::PwmCenter { pwm, .. } if pwm == id) {
                let driver = self.blocks[sampler.index].inputs[0];
                if !matches!(self.blocks[driver].equation, Equation::Sensor(..)) {
                    sampler.boundary_input = Some(self.values[driver]);
                }
            }
        }
    }

    pub(super) fn latch_pwm(&mut self, id: &str, cycle: u64, frequency: f64, duty: f64) {
        for sampler in &mut self.samplers {
            if let SampleClock::PwmCenter { pwm, on, next } = &mut sampler.clock
                && pwm == id
            {
                *next = TickClock::pwm(frequency, midpoint(*on, duty)).time(cycle);
            }
        }
    }

    pub(super) fn capture(
        &mut self,
        time: f64,
        include_unrecorded_changes: bool,
        sense: impl FnMut(&str, RecordQuantity) -> f64,
    ) -> Result<bool, Error> {
        self.capture_due(time, include_unrecorded_changes, false, sense)
    }

    pub(super) fn capture_after_latch(
        &mut self,
        time: f64,
        include_unrecorded_changes: bool,
        sense: impl FnMut(&str, RecordQuantity) -> f64,
    ) -> Result<bool, Error> {
        self.capture_due(time, include_unrecorded_changes, true, sense)
    }

    fn capture_due(
        &mut self,
        time: f64,
        include_unrecorded_changes: bool,
        after_latch: bool,
        mut sense: impl FnMut(&str, RecordQuantity) -> f64,
    ) -> Result<bool, Error> {
        // Capture simultaneously from the pre-event electrical and held signal values.
        for sampler in &mut self.samplers {
            // Discard the saved input after this boundary even if the SOC is later.
            let boundary_input = if after_latch {
                sampler.boundary_input.take()
            } else {
                None
            };
            if same_clock_time(sampler.clock.next(), time) {
                let driver = self.blocks[sampler.index].inputs[0];
                let source = &self.blocks[driver];
                let value = match source.equation {
                    Equation::Sensor(quantity, gain) => sense(source.id, quantity) * gain,
                    _ => boundary_input.unwrap_or(self.values[driver]),
                };
                self.captures.push((sampler.index, value));
                sampler.clock.advance();
            }
        }
        let mut changed = false;
        for (index, value) in self.captures.drain(..) {
            let block = &self.blocks[index];
            if !value.is_finite() {
                return Err(Error::invalid(
                    block.id,
                    format!("Non-finite sampler output at {time} s"),
                ));
            }
            changed |=
                value != self.values[index] && (include_unrecorded_changes || block.observed);
            self.values[index] = value;
        }
        Ok(changed)
    }

    /// Seed duties that can be determined without the initial electrical solve.
    pub(super) fn prepare_initial(&mut self) -> Result<(), Error> {
        self.evaluate(Some(false), false, |_, _| {
            unreachable!("sensor-independent block")
        })?;
        Ok(())
    }

    /// Evaluate direct dependencies in order, then hold all values until the next tick.
    pub(super) fn run_task(
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
        deferred: Option<bool>,
        include_unrecorded_changes: bool,
        mut sense: impl FnMut(&str, RecordQuantity) -> f64,
    ) -> Result<bool, Error> {
        let time = self.next_task();
        let mut changed = false;
        let period = self.clock.period();
        for &index in &self.order {
            let block = &mut self.blocks[index];
            if matches!(block.equation, Equation::Sampler)
                || deferred.is_some_and(|required| block.deferred != required)
            {
                continue;
            }
            let input = |pin: usize| self.values[block.inputs[pin]];
            let value = match &mut block.equation {
                Equation::Sensor(quantity, gain) => sense(block.id, *quantity) * *gain,
                Equation::Constant(_) | Equation::Step { .. } | Equation::Ramp { .. } => {
                    block.equation.source_value(time)
                }
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
                    *state = (*state + input(0) * period).clamp(*min, *max);
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
                    let increment = (*ki * error) * period;
                    let integral = *state + increment;
                    let output = *kp * error + integral;
                    if !((output > *max && increment > 0.0) || (output < *min && increment < 0.0)) {
                        *state = integral;
                    }
                    output.clamp(*min, *max)
                }
                Equation::DutyPwm => input(0).clamp(0.0, 1.0),
                Equation::Sampler => unreachable!("samplers capture separately"),
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

fn task_inputs<'a>(block: &'a Block<'_>) -> &'a [usize] {
    if matches!(block.equation, Equation::Sampler) {
        &[]
    } else {
        &block.inputs
    }
}

fn evaluation_order(blocks: &[Block<'_>]) -> Result<Vec<usize>, Error> {
    let mut remaining: Vec<_> = blocks
        .iter()
        .map(|block| task_inputs(block).len())
        .collect();
    let mut consumers = vec![Vec::new(); blocks.len()];
    let mut ready = VecDeque::new();
    for (index, block) in blocks.iter().enumerate() {
        if task_inputs(block).is_empty() {
            ready.push_back(index);
        }
        for &driver in task_inputs(block) {
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
            index = *task_inputs(&blocks[index])
                .iter()
                .find(|&&driver| remaining[driver] > 0)
                .expect("unresolved dependency");
        }
    }
    Ok(order)
}
