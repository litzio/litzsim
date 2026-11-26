//! Transient modified nodal analysis for the v1 circuit document.

use crate::circuit::{Document, Error, Part, part_definition};
use crate::waves::{Channel, Waveform};
use std::collections::{BTreeMap, BTreeSet};

type Node = Option<usize>; // Ground has no row or column.
const CLOCK_EPSILON: f64 = 8.0 * f64::EPSILON;
const SWITCH_EPSILON: f64 = 1e-12;

/// Solve from the document's initial conditions. The callback returns true to
/// cancel before assembling the next sample. Cancellation returns no waveform.
pub fn simulate(
    document: &Document,
    mut should_stop: impl FnMut() -> bool,
) -> Result<Waveform, Error> {
    simulate_observed(document, |_| should_stop())
}

/// Report the simulated time reached before each step and check for cancellation.
pub fn simulate_observed(
    document: &Document,
    mut should_stop: impl FnMut(f64) -> bool,
) -> Result<Waveform, Error> {
    let (mut devices, mut system) = assemble(document)?;
    let settings = &document.circuit().simulation;
    let step = settings.time_step_s;
    let stop = settings.stop_time_s;
    let stride = settings
        .output_step_s
        .map(|output| (output / step).round() as u64);
    let probes: Vec<_> = devices
        .iter()
        .enumerate()
        .filter_map(|(index, device)| device.signal_unit().map(|unit| (index, unit)))
        .collect();
    let clocks: Vec<_> = devices.iter().filter_map(Device::pulse).collect();
    let mut wave = Waveform::new(probes.iter().map(|&(index, unit)| Channel {
        name: format!("{}_{unit}", devices[index].id),
        unit: unit.into(),
    }));

    let (mut tick, mut time, mut previous) = (0_u64, 0.0, 0.0);
    loop {
        if should_stop(previous) {
            return Err(Error::new("simulationCanceled", "Simulation canceled"));
        }
        let h = time - previous;
        // Use the interval's interior state. At an edge the sample is the
        // left limit, and the next interval uses the newly switched state.
        solve_sample(&mut devices, &mut system, time, h)?;
        let x = &system.rhs;
        for device in &mut devices {
            match &mut device.kind {
                Kind::Capacitor(_, history) => *history = node(x, device.p) - node(x, device.n),
                Kind::Inductor(_, history) => *history = x[device.branch.expect("inductor branch")],
                _ => {}
            }
        }
        let on_grid = time == tick as f64 * step;
        if stride.is_none_or(|stride| on_grid && tick % stride == 0) || time == stop {
            let signals = probes.iter().map(|&(index, _)| devices[index].signal(x));
            if signals.clone().any(|value| !value.is_finite()) {
                return Err(Error::new("simulationFailed", "Non-finite waveform value"));
            }
            wave.push(time, signals);
        }
        if time == stop {
            return Ok(wave);
        }
        if on_grid {
            tick += 1;
        }
        let boundary = stop.min(tick as f64 * step);
        let edge = clocks
            .iter()
            .fold(boundary, |next, pulse| next.min(pulse.next_edge(time)));
        // Merge clocks separated only by floating-point roundoff.
        let next = if boundary - edge <= CLOCK_EPSILON * boundary {
            boundary
        } else {
            edge
        };
        let next = if stop - next <= CLOCK_EPSILON * stop {
            stop
        } else {
            next
        };
        if next <= time {
            return Err(Error::new(
                "simulationFailed",
                "Time step below clock resolution",
            ));
        }
        previous = time;
        time = next;
    }
}

#[derive(Clone, Copy)]
struct Pulse {
    frequency: f64,
    duty: f64,
    high: f64,
}

impl Pulse {
    fn is_high(self, time: f64) -> bool {
        (time * self.frequency).fract() < self.duty
    }

    fn next_edge(self, time: f64) -> f64 {
        let cycles = time * self.frequency;
        let cycle = cycles.floor();
        let threshold = cycles + CLOCK_EPSILON * cycles;
        if cycle + self.duty > threshold {
            (cycle + self.duty) / self.frequency
        } else if cycle + 1.0 > threshold {
            (cycle + 1.0) / self.frequency
        } else {
            (cycle + 1.0 + self.duty) / self.frequency
        }
    }
}

enum Kind {
    Resistance(f64),
    Capacitor(f64, f64),
    Inductor(f64, f64),
    Dc(f64),
    Sine(f64, f64),
    Pulse(Pulse),
    Diode,
    Nmos(Option<Pulse>),
    Voltmeter,
    Ammeter,
}

struct Device<'a> {
    id: &'a str,
    kind: Kind,
    p: Node,
    n: Node,
    branch: Option<usize>,
    probe_unit: Option<&'static str>,
    driven: bool,
    conducting: bool,
}

impl Device<'_> {
    fn branch(&self) -> usize {
        self.branch.expect("device has a branch equation")
    }

    fn pulse(&self) -> Option<Pulse> {
        match self.kind {
            Kind::Pulse(pulse) => Some(pulse),
            _ => None,
        }
    }

    fn signal_unit(&self) -> Option<&'static str> {
        self.probe_unit.or(match self.kind {
            Kind::Inductor(..) | Kind::Diode => Some("A"),
            _ => None,
        })
    }

    fn signal(&self, x: &[f64]) -> f64 {
        match self.kind {
            Kind::Voltmeter => node(x, self.p) - node(x, self.n),
            _ => x[self.branch()],
        }
    }
}

fn assemble(document: &Document) -> Result<(Vec<Device<'_>>, System), Error> {
    let circuit = document.circuit();
    let pin_nets = circuit.pin_nets();
    let missing: Vec<_> = pin_nets
        .iter()
        .filter_map(|(pin, net)| net.is_none().then_some(pin.as_str()))
        .collect();
    if let Some(first) = missing.first() {
        return Err(Error::invalid(
            first,
            format!("Unconnected pins: {}", missing.join(", ")),
        ));
    }
    let pin_net = |id, pin| pin_nets[format!("{id}.{pin}").as_str()].expect("connected terminal");
    let grounds: BTreeSet<_> = circuit
        .parts
        .iter()
        .filter(|(_, part)| part.kind == "ground")
        .map(|(id, _)| pin_net(id.as_str(), "ref"))
        .collect();
    if grounds.is_empty() {
        return Err(Error::invalid("ground", "Missing reference ground"));
    }

    let mut size = 0;
    let nodes: BTreeMap<_, _> = circuit
        .nets
        .iter()
        .filter(|(_, pins)| !pins.is_empty())
        .map(|(net, _)| {
            (
                net.as_str(),
                (!grounds.contains(net.as_str())).then(|| take_index(&mut size)),
            )
        })
        .collect();
    let pin_node = |id, pin| nodes[pin_net(id, pin)];
    let mut devices = Vec::new();
    for (id, part) in &circuit.parts {
        if part.kind == "ground" {
            continue;
        }
        let definition = part_definition(&part.kind).expect("validated part");
        let kind = device_kind(id, part)?;
        let (p, n) = match definition.measurement {
            Some(probe) => (probe.from_pin, probe.to_pin),
            None if matches!(kind, Kind::Nmos(_)) => ("d", "s"),
            None => (definition.pins[0], definition.pins[1]),
        };
        let branch =
            (!matches!(kind, Kind::Resistance(_) | Kind::Voltmeter)).then(|| take_index(&mut size));
        devices.push(Device {
            id,
            kind,
            p: pin_node(id.as_str(), p),
            n: pin_node(id.as_str(), n),
            branch,
            probe_unit: definition.measurement.map(|probe| probe.unit),
            driven: false,
            conducting: false,
        });
    }
    let drivers: BTreeMap<_, _> = devices
        .iter()
        .filter_map(|device| device.pulse().map(|pulse| ((device.p, device.n), pulse)))
        .collect();
    for device in &mut devices {
        if let Kind::Nmos(driver) = &mut device.kind {
            let gate = pin_node(device.id, "g");
            *driver = Some(*drivers.get(&(gate, device.n)).ok_or_else(|| {
                Error::invalid(device.id, "NMOS requires a gate-to-source PWM driver")
            })?);
        }
    }
    Ok((devices, System::new(size)))
}

fn take_index(size: &mut usize) -> usize {
    let index = *size;
    *size += 1;
    index
}

fn device_kind(id: &str, part: &Part) -> Result<Kind, Error> {
    let param = |key: &str| part.parameters[key].as_f64().expect("validated parameter");
    let positive = |key| {
        let value = param(key);
        if value > 0.0 {
            Ok(value)
        } else {
            Err(Error::invalid(id, format!("{key} must be positive")))
        }
    };
    Ok(match part.kind.as_str() {
        "resistor" => Kind::Resistance(positive("resistance_ohm")?),
        "capacitor" => Kind::Capacitor(positive("capacitance_F")?, param("initialVoltage_V")),
        "inductor" => Kind::Inductor(positive("inductance_H")?, param("initialCurrent_A")),
        "dc" => Kind::Dc(param("voltage_V")),
        "sine" => Kind::Sine(param("amplitude_V"), param("frequency_Hz")),
        "pwm" => {
            let duty = param("duty");
            if !(0.0 < duty && duty < 1.0) {
                return Err(Error::invalid(id, "PWM duty must be in (0, 1)"));
            }
            Kind::Pulse(Pulse {
                frequency: positive("frequency_Hz")?,
                high: positive("high_V")?,
                duty,
            })
        }
        "diode" => Kind::Diode,
        "nmos" => Kind::Nmos(None),
        "voltmeter" => Kind::Voltmeter,
        "ammeter" => Kind::Ammeter,
        kind => {
            return Err(Error::invalid(
                id,
                format!("Unsupported solver device: {kind}"),
            ));
        }
    })
}

fn solve_sample(
    devices: &mut [Device<'_>],
    system: &mut System,
    time: f64,
    h: f64,
) -> Result<(), Error> {
    let interior = time - h / 2.0;
    let mut turned_on = false;
    for device in devices.iter_mut() {
        if let Kind::Nmos(Some(pulse)) = device.kind {
            let high = pulse.is_high(interior);
            turned_on |= high && !device.driven;
            device.driven = high;
            if high {
                device.conducting = false;
            }
        }
    }
    for iteration in 0..=32 {
        system.clear();
        for device in devices.iter() {
            stamp(system, device, time, interior, h);
        }
        if !system.solve() {
            // A newly driven channel can short a voltage source through a
            // previously conducting ideal diode. Release that state once.
            if turned_on && iteration == 0 && devices.iter().any(|d| d.conducting) {
                for device in devices.iter_mut() {
                    device.conducting = false;
                }
                continue;
            }
            return Err(Error::new(
                "simulationFailed",
                format!("Singular circuit at {time} s"),
            ));
        }
        if system.rhs.iter().any(|v| !v.is_finite()) {
            return Err(Error::new(
                "simulationFailed",
                format!("Non-finite circuit solution at {time} s"),
            ));
        }
        let changed = devices
            .iter_mut()
            .any(|device| update_switch(device, &system.rhs));
        if !changed {
            return Ok(());
        }
    }
    Err(Error::new(
        "simulationFailed",
        format!("Ideal switch states did not converge at {time} s"),
    ))
}

fn update_switch(device: &mut Device<'_>, x: &[f64]) -> bool {
    let sign = match device.kind {
        Kind::Diode => 1.0,
        Kind::Nmos(_) if !device.driven => -1.0, // Body diode: source to drain.
        _ => return false,
    };
    let conducts = if device.conducting {
        sign * x[device.branch()] >= -SWITCH_EPSILON
    } else {
        sign * (node(x, device.p) - node(x, device.n)) > SWITCH_EPSILON
    };
    let changed = conducts != device.conducting;
    device.conducting = conducts;
    changed
}

fn stamp(system: &mut System, device: &Device<'_>, time: f64, interior: f64, h: f64) {
    let (p, n) = (device.p, device.n);
    match device.kind {
        Kind::Voltmeter => return,
        Kind::Resistance(resistance) => {
            let g = 1.0 / resistance;
            system.add(p, p, g);
            system.add(p, n, -g);
            system.add(n, p, -g);
            system.add(n, n, g);
            return;
        }
        _ => {}
    }
    let branch = device.branch();
    let k = Some(branch);
    system.add(p, k, 1.0);
    system.add(n, k, -1.0);
    let (voltage, coefficient, rhs) = match device.kind {
        Kind::Diode | Kind::Nmos(_) if !device.driven && !device.conducting => (false, 1.0, 0.0),
        Kind::Inductor(_, current) if h == 0.0 => (false, 1.0, current),
        Kind::Inductor(inductance, current) => {
            let z = inductance / h;
            (true, -z, -z * current)
        }
        Kind::Capacitor(capacitance, voltage) => (true, -h / capacitance, voltage),
        Kind::Dc(voltage) => (true, 0.0, voltage),
        Kind::Sine(amplitude, frequency) => (
            true,
            0.0,
            amplitude * (std::f64::consts::TAU * time * frequency).sin(),
        ),
        Kind::Pulse(pulse) => (
            true,
            0.0,
            if pulse.is_high(interior) {
                pulse.high
            } else {
                0.0
            },
        ),
        Kind::Ammeter | Kind::Diode | Kind::Nmos(_) => (true, 0.0, 0.0),
        Kind::Voltmeter | Kind::Resistance(_) => unreachable!(),
    };
    if voltage {
        system.add(k, p, 1.0);
        system.add(k, n, -1.0);
    }
    system.matrix[branch][branch] += coefficient;
    system.rhs[branch] = rhs;
}

fn node(x: &[f64], index: Node) -> f64 {
    index.map_or(0.0, |index| x[index])
}

struct System {
    matrix: Vec<Vec<f64>>,
    rhs: Vec<f64>,
}

impl System {
    fn new(size: usize) -> Self {
        Self {
            matrix: vec![vec![0.0; size]; size],
            rhs: vec![0.0; size],
        }
    }

    fn clear(&mut self) {
        for row in &mut self.matrix {
            row.fill(0.0);
        }
        self.rhs.fill(0.0);
    }

    fn add(&mut self, row: Node, column: Node, value: f64) {
        if let (Some(row), Some(column)) = (row, column) {
            self.matrix[row][column] += value;
        }
    }

    /// Gaussian elimination with partial pivoting. The solution replaces rhs.
    fn solve(&mut self) -> bool {
        let size = self.rhs.len();
        for k in 0..size {
            let mut pivot = k;
            for i in k + 1..size {
                if self.matrix[i][k].abs() > self.matrix[pivot][k].abs() {
                    pivot = i;
                }
            }
            if self.matrix[pivot][k] == 0.0 {
                return false;
            }
            self.matrix.swap(k, pivot);
            self.rhs.swap(k, pivot);
            let (head, tail) = self.matrix.split_at_mut(k + 1);
            let upper = &head[k];
            for (index, row) in tail.iter_mut().enumerate() {
                let factor = row[k] / upper[k];
                // Only the trailing part of the elimination row is read again.
                for (value, coefficient) in row[k + 1..].iter_mut().zip(&upper[k + 1..]) {
                    *value -= factor * coefficient;
                }
                self.rhs[k + 1 + index] -= factor * self.rhs[k];
            }
        }
        for i in (0..size).rev() {
            for j in i + 1..size {
                self.rhs[i] -= self.matrix[i][j] * self.rhs[j];
            }
            self.rhs[i] /= self.matrix[i][i];
        }
        true
    }
}
