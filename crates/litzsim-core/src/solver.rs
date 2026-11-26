//! Transient modified nodal analysis for the v1 circuit document.

mod control;

use crate::circuit::{Document, Error, Part, Record, RecordQuantity, part_definition};
use crate::waves::{Channel, Waveform};
use control::Control;
use serde::{Deserialize, Serialize};
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

type Node = Option<usize>; // Ground has no row or column.
const CLOCK_EPSILON: f64 = 8.0 * f64::EPSILON;
const SWITCH_EPSILON: f64 = 1e-12;
/// Fraction of the next interval used to settle switch states at an edge.
const EDGE_SETTLE: f64 = 1e-6;

/// Treat roundoff-separated clock boundaries as one event.
fn same_clock_time(a: f64, b: f64) -> bool {
    let difference = (a - b).abs();
    difference.is_finite() && difference <= CLOCK_EPSILON * a.max(b)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    pub code: String,
    pub message: String,
    pub part_ids: Vec<String>,
    #[serde(rename = "timeConstant_s")]
    pub time_constant_s: f64,
    #[serde(rename = "timeStep_s")]
    pub time_step_s: f64,
}

/// Inspect simple voltage-source charging paths before a background run.
pub fn warnings(document: &Document) -> Vec<Warning> {
    match assemble(document) {
        Ok((mut devices, _)) => {
            source_paths(&mut devices, document.circuit().simulation.time_step_s)
        }
        Err(_) => Vec::new(), // The worker reports invalid circuits through the normal error path.
    }
}

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
    should_stop: impl FnMut(f64) -> bool,
) -> Result<Waveform, Error> {
    simulate_with_warnings(document, should_stop).map(|(wave, _)| wave)
}

pub fn simulate_with_warnings(
    document: &Document,
    mut should_stop: impl FnMut(f64) -> bool,
) -> Result<(Waveform, Vec<Warning>), Error> {
    let mut control = Control::compile(document)?;
    let (mut devices, size) = assemble(document)?;
    let mut system = System::new(size);
    let settings = &document.circuit().simulation;
    let step = settings.time_step_s;
    let warnings = source_paths(&mut devices, step);
    let stop = settings.stop_time_s;
    let stride = settings
        .output_step_s
        .map(|output| (output / step).round() as u64);
    let probes: Vec<_> = devices
        .iter()
        .enumerate()
        .flat_map(|(index, device)| {
            [RecordQuantity::Voltage, RecordQuantity::Current]
                .into_iter()
                .filter(move |&quantity| device.record.enabled(quantity))
                .map(move |quantity| (index, quantity))
        })
        .collect();
    let clocks: Vec<_> = devices.iter().filter_map(Device::pulse).collect();
    let electrical_channels = probes.iter().map(|&(index, quantity)| {
        let unit = quantity.unit();
        Channel {
            name: format!("{}_{unit}", devices[index].id),
            unit: unit.into(),
        }
    });
    let mut wave =
        Waveform::new(electrical_channels.chain(control.iter().flat_map(Control::channels)));

    let (mut tick, mut time, mut previous) = (0_u64, 0.0, 0.0);
    loop {
        if should_stop(previous) {
            return Err(Error::new("simulationCanceled", "Simulation canceled"));
        }
        let h = time - previous;
        // Use the interval's interior state. At an edge the sample is the
        // left limit, and the next interval uses the newly switched state.
        solve_sample(&mut devices, &mut system, Sample::step(time, h))?;
        let x = &system.rhs;
        for device in &mut devices {
            let branch = device.branch;
            match &mut device.kind {
                // Integrate the ideal capacitor voltage inside the ESR branch.
                Kind::Capacitor(capacitance, history) => {
                    *history += (h / *capacitance) * x[branch.expect("capacitor branch")];
                }
                Kind::Inductor(_, history) => *history = x[device.branch.expect("inductor branch")],
                _ => {}
            }
        }
        // An endpoint can represent the grid tick a few ULPs to either side.
        let on_grid = same_clock_time(time, tick as f64 * step);
        let record = stride.is_none_or(|stride| on_grid && tick % stride == 0) || time == stop;
        // A control tick uses the electrical left limit. Record the held
        // outputs first, except at zero where there is no previous value.
        if record && time != 0.0 {
            push_row(&mut wave, &devices, &probes, control.as_ref(), time, x)?;
        }
        let mut control_changed = false;
        if let Some(control) = &mut control
            && control.due(time)
        {
            control_changed = control.sample(|id, quantity| {
                let index = devices
                    .binary_search_by_key(&id, |device| device.id)
                    .expect("sensor has an electrical device");
                devices[index].signal(quantity, x)
            })?;
        }
        if record && time == 0.0 {
            push_row(&mut wave, &devices, &probes, control.as_ref(), time, x)?;
        }
        let at_stop = time == stop;
        if at_stop && !control_changed {
            return Ok((wave, warnings));
        }
        if on_grid {
            tick += 1;
        }
        // A control change at stop needs a right-limit row. Use the interval
        // that would follow this sample to find the new PWM state.
        let grid = tick as f64 * step;
        let boundary = if at_stop { grid } else { stop.min(grid) };
        let edge = clocks
            .iter()
            .fold(boundary, |next, pulse| next.min(pulse.next_edge(time)));
        let edge = control
            .as_ref()
            .map_or(edge, |control| edge.min(control.next_sample()));
        // Merge clocks separated only by floating-point roundoff.
        let next = if same_clock_time(boundary, edge) {
            boundary
        } else {
            edge
        };
        let next = if !at_stop && same_clock_time(stop, next) {
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
        // A PWM edge at this sample switches the next interval. Record the
        // right limit as a second row at the same time: capacitor voltages and
        // inductor currents stay, and the circuit is solved with the switch
        // states of the next interval. Switch states are restored afterwards so
        // the following steps are computed exactly as without this row.
        // Include any updated control outputs in this same right-limit row.
        let right = (time + next) / 2.0;
        let switching = record
            && clocks
                .iter()
                .any(|pulse| pulse.is_high(time - h / 2.0) != pulse.is_high(right));
        if switching {
            let switches: Vec<_> = devices.iter().map(|d| (d.driven, d.conducting)).collect();
            let limit = Sample {
                time,
                h: 0.0,
                interior: right,
                initial: false,
            };
            // With inductors as exact current sources an off diode can leave
            // no path for them. A vanishing step first settles which diodes
            // conduct; its values are not used.
            let settle = Sample {
                h: (next - time) * EDGE_SETTLE,
                ..limit
            };
            solve_sample(&mut devices, &mut system, settle)?;
            solve_sample(&mut devices, &mut system, limit)?;
            push_row(
                &mut wave,
                &devices,
                &probes,
                control.as_ref(),
                time,
                &system.rhs,
            )?;
            for (device, (driven, conducting)) in devices.iter_mut().zip(switches) {
                (device.driven, device.conducting) = (driven, conducting);
            }
        } else if record && time != 0.0 && control_changed {
            push_row(&mut wave, &devices, &probes, control.as_ref(), time, x)?;
        }
        if at_stop {
            return Ok((wave, warnings));
        }
        previous = time;
        time = next;
    }
}

fn push_row(
    wave: &mut Waveform,
    devices: &[Device<'_>],
    probes: &[(usize, RecordQuantity)],
    control: Option<&Control<'_>>,
    time: f64,
    x: &[f64],
) -> Result<(), Error> {
    let signals = probes
        .iter()
        .map(|&(index, quantity)| devices[index].signal(quantity, x))
        .chain(control.into_iter().flat_map(Control::recorded_values));
    if signals.clone().any(|value| !value.is_finite()) {
        return Err(Error::new("simulationFailed", "Non-finite waveform value"));
    }
    wave.push(time, signals);
    Ok(())
}

/// One circuit solve. `h` is the step from the previous sample: zero at the
/// start and for the right limit at a PWM edge, where inductors hold their
/// current and capacitors their voltage. PWM states are those at `interior`.
/// `initial` applies the start-only capacitor rules.
#[derive(Clone, Copy)]
struct Sample {
    time: f64,
    h: f64,
    interior: f64,
    initial: bool,
}

impl Sample {
    fn step(time: f64, h: f64) -> Self {
        Self {
            time,
            h,
            interior: time - h / 2.0,
            initial: h == 0.0,
        }
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
        let falling = (cycle + self.duty) / self.frequency;
        let rising = (cycle + 1.0) / self.frequency;
        if falling > time && !same_clock_time(time, falling) {
            falling
        } else if rising > time && !same_clock_time(time, rising) {
            rising
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum InitialCapacitor {
    FixedVoltage,
    ZeroCurrent,
    SourceConflict,
}

struct Device<'a> {
    id: &'a str,
    kind: Kind,
    p: Node,
    n: Node,
    branch: Option<usize>,
    record: Record,
    series_resistance: f64,
    initial_capacitor: InitialCapacitor,
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

    fn is_open_switch(&self) -> bool {
        matches!(self.kind, Kind::Diode | Kind::Nmos(_)) && !self.driven && !self.conducting
    }

    fn voltage(&self, x: &[f64]) -> f64 {
        node(x, self.p) - node(x, self.n)
    }

    fn signal(&self, quantity: RecordQuantity, x: &[f64]) -> f64 {
        match (quantity, &self.kind) {
            (RecordQuantity::Voltage, _) => self.voltage(x),
            (RecordQuantity::Current, Kind::Resistance(ohm)) => self.voltage(x) / ohm,
            (RecordQuantity::Current, _) => x[self.branch()],
            (RecordQuantity::Output, _) => unreachable!("control output is evaluated separately"),
        }
    }
}

/// Reject ground-referenced probes before a background run is admitted.
pub fn validate_probe_reference(document: &Document) -> Result<(), Error> {
    let parts = &document.circuit().parts;
    if !parts.values().any(|part| part.kind == "ground")
        && let Some((id, _)) = parts.iter().find(|(_, part)| {
            part_definition(&part.kind)
                .and_then(|definition| definition.measurement)
                .is_some_and(|measurement| measurement.to_pin.is_none())
        })
    {
        return Err(Error::invalid(
            id,
            format!("Voltage probe {id} requires a reference ground"),
        ));
    }
    Ok(())
}

fn assemble(document: &Document) -> Result<(Vec<Device<'_>>, usize), Error> {
    validate_probe_reference(document)?;
    let circuit = document.circuit();
    let pin_nets = circuit.pin_nets();
    let missing: Vec<_> = pin_nets
        .iter()
        .filter_map(|(pin, net)| {
            (net.is_none()
                && !circuit
                    .require_pin(pin)
                    .expect("validated terminal")
                    .is_signal())
            .then_some(pin.as_str())
        })
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
    let signal_only = !circuit.parts.is_empty()
        && circuit.parts.values().all(|part| {
            let definition = part_definition(&part.kind).expect("validated part");
            definition.is_control() && definition.measurement.is_none()
        });
    if grounds.is_empty() && !signal_only {
        return Err(Error::invalid("ground", "Missing reference ground"));
    }

    let mut size = 0;
    let nodes: BTreeMap<_, _> = circuit
        .nets
        .iter()
        .filter(|(_, pins)| {
            pins.first().is_some_and(|pin| {
                !circuit
                    .require_pin(pin)
                    .expect("validated terminal")
                    .is_signal()
            })
        })
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
        let is_control = definition.is_control();
        if is_control && definition.measurement.is_none() {
            continue;
        }
        let mut series_resistance = 0.0;
        if let Some((key, _)) = definition.series_resistance() {
            series_resistance = part.parameters[key].as_f64().expect("validated parameter");
            if series_resistance < 0.0 {
                return Err(Error::invalid(id, format!("{key} must be nonnegative")));
            }
        }
        let kind = device_kind(id, part)?;
        let (p, n) = match definition.measurement {
            Some(probe) => (probe.from_pin, probe.to_pin),
            None => {
                let [p, n] = definition
                    .recording_pins
                    .unwrap_or_else(|| [definition.pins[0], definition.pins[1]]);
                (p, Some(n))
            }
        };
        let branch =
            (!matches!(kind, Kind::Resistance(_) | Kind::Voltmeter)).then(|| take_index(&mut size));
        let record = Record {
            voltage: part.record.voltage || (!is_control && matches!(kind, Kind::Voltmeter)),
            current: part.record.current || (!is_control && matches!(kind, Kind::Ammeter)),
            ..Default::default()
        };
        devices.push(Device {
            id,
            kind,
            p: pin_node(id.as_str(), p),
            n: n.and_then(|pin| pin_node(id.as_str(), pin)),
            branch,
            record,
            series_resistance,
            initial_capacitor: InitialCapacitor::FixedVoltage,
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
    Ok((devices, size))
}

/// Follow unbranched series resistors and ideal ammeters on either side of a
/// capacitor, then look for a voltage source across the resulting node pair.
fn source_paths(devices: &mut [Device<'_>], step: f64) -> Vec<Warning> {
    let mut attached = BTreeMap::<Node, Vec<usize>>::new();
    let mut sources = BTreeMap::<(Node, Node), Vec<usize>>::new();
    for (index, device) in devices.iter().enumerate() {
        if matches!(device.kind, Kind::Voltmeter) {
            continue;
        }
        attached.entry(device.p).or_default().push(index);
        attached.entry(device.n).or_default().push(index);
        if matches!(device.kind, Kind::Dc(_) | Kind::Sine(..)) {
            let pair = (device.p.min(device.n), device.p.max(device.n));
            sources.entry(pair).or_default().push(index);
        }
    }
    let mut warnings = Vec::new();
    for index in 0..devices.len() {
        let cap = &devices[index];
        let Kind::Capacitor(capacitance, initial) = cap.kind else {
            continue;
        };
        let mut resistance = cap.series_resistance;
        let mut path = vec![cap.id];
        let mut trace = |mut node, mut previous| {
            while let [a, b] = attached[&node].as_slice() {
                let next = if *a == previous { *b } else { *a };
                let device = &devices[next];
                match device.kind {
                    Kind::Resistance(ohm) => resistance += ohm,
                    Kind::Ammeter => {}
                    _ => break,
                }
                path.push(device.id);
                node = if node == device.p { device.n } else { device.p };
                previous = next;
            }
            node
        };
        let p = trace(cap.p, index);
        let n = trace(cap.n, index);
        if p == n {
            continue;
        }
        let Some([source]) = sources.get(&(p.min(n), p.max(n))).map(Vec::as_slice) else {
            continue;
        };
        let source = &devices[*source];
        resistance += source.series_resistance;
        let time_constant = resistance * capacitance;
        if resistance > 0.0 && time_constant < step / 10.0 {
            path.push(source.id);
            warnings.push(Warning {
                code: "inrushTimeStep".into(),
                message: "Inrush current peak depends on the time step".into(),
                part_ids: path.into_iter().map(str::to_owned).collect(),
                time_constant_s: time_constant,
                time_step_s: step,
            });
        }
        if resistance == 0.0 {
            let aligned_dc = match source.kind {
                Kind::Dc(voltage) if p == source.p => initial == voltage,
                Kind::Dc(voltage) => initial == -voltage,
                _ => false,
            };
            devices[index].initial_capacitor = if aligned_dc {
                InitialCapacitor::ZeroCurrent
            } else {
                InitialCapacitor::SourceConflict
            };
        }
    }
    warnings
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
        "voltmeter" | "voltageProbe" | "voltageSensor" => Kind::Voltmeter,
        "ammeter" | "currentSensor" => Kind::Ammeter,
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
    sample: Sample,
) -> Result<(), Error> {
    let Sample {
        time,
        h,
        interior,
        initial,
    } = sample;
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
            stamp(system, device, sample);
        }
        if let Err(column) = system.solve() {
            // A newly driven channel can short a voltage source through a
            // previously conducting ideal diode. Release that state once.
            if turned_on && iteration == 0 && devices.iter().any(|d| d.conducting) {
                for device in devices.iter_mut() {
                    device.conducting = false;
                }
                continue;
            }
            return Err(diagnose_zero_pivot(devices, column, time, h, initial));
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

fn diagnose_zero_pivot(
    devices: &[Device<'_>],
    column: usize,
    time: f64,
    h: f64,
    initial: bool,
) -> Error {
    let failed_branch = devices.iter().find(|device| device.branch == Some(column));
    let initial_inductor_current =
        |device: &Device<'_>| h == 0.0 && matches!(device.kind, Kind::Inductor(..));
    let free_initial_capacitor =
        |device: &Device<'_>| initial && device.initial_capacitor == InitialCapacitor::ZeroCurrent;
    let ideal_voltage = |device: &Device<'_>| {
        !device.is_open_switch()
            && match device.kind {
                Kind::Resistance(_) | Kind::Voltmeter | Kind::Inductor(..) => false,
                Kind::Capacitor(..) => {
                    h == 0.0 && device.series_resistance == 0.0 && !free_initial_capacitor(device)
                }
                Kind::Dc(_) | Kind::Sine(..) => device.series_resistance == 0.0,
                _ => true,
            }
    };

    // For a branch column, look for a second path of ideal voltage constraints.
    // For a node column, find all nodes whose voltages are coupled to it.
    let mut connections = BTreeMap::<Node, Vec<(Node, &str)>>::new();
    for device in devices {
        let connects = if failed_branch.is_some() {
            device.branch != Some(column) && ideal_voltage(device)
        } else {
            !device.is_open_switch()
                && !initial_inductor_current(device)
                && !free_initial_capacitor(device)
                && !matches!(device.kind, Kind::Voltmeter)
        };
        if !connects {
            continue;
        }
        connections
            .entry(device.p)
            .or_default()
            .push((device.n, device.id));
        connections
            .entry(device.n)
            .or_default()
            .push((device.p, device.id));
    }
    let start = failed_branch.map_or(Some(column), |device| device.p);
    let mut paths = BTreeMap::from([(start, None)]);
    let mut pending = vec![start];
    while let Some(node) = pending.pop() {
        for &(next, id) in connections.get(&node).into_iter().flatten() {
            if let Entry::Vacant(entry) = paths.entry(next) {
                entry.insert(Some((node, id)));
                pending.push(next);
            }
        }
    }

    let report = |reason: &str, target: &str, ids: &[&str]| {
        let mut error = Error::new(
            "singularCircuit",
            format!("{reason} at {time} s: {}", ids.join(", ")),
        );
        error.target_id = Some(target.into());
        error
    };
    if let Some(device) = failed_branch {
        if ideal_voltage(device) && paths.contains_key(&device.n) {
            let mut ids = vec![device.id];
            let mut node = device.n;
            while let Some((previous, id)) = paths[&node] {
                ids.push(id);
                node = previous;
            }
            let loop_devices = || devices.iter().filter(|device| ids.contains(&device.id));
            let reason = if loop_devices().any(|device| {
                initial && device.initial_capacitor == InitialCapacitor::SourceConflict
            }) || (loop_devices()
                .any(|device| matches!(device.kind, Kind::Capacitor(..)))
                && loop_devices().any(|device| matches!(device.kind, Kind::Dc(_) | Kind::Sine(..))))
            {
                "Ideal voltage constraint loop (add capacitor ESR or source internal resistance)"
            } else {
                "Ideal voltage constraint loop"
            };
            return report(reason, device.id, &ids);
        }
    } else if !paths.contains_key(&None) {
        let boundary: Vec<_> = devices
            .iter()
            .filter(|device| {
                (device.is_open_switch() || initial_inductor_current(device))
                    && paths.contains_key(&device.p) != paths.contains_key(&device.n)
            })
            .collect();
        let ids: Vec<_> = boundary.iter().map(|device| device.id).collect();
        if let Some(inductor) = boundary
            .iter()
            .find(|device| initial_inductor_current(device))
        {
            return report(
                "Initial inductor current cutset with no voltage-defined return path",
                inductor.id,
                &ids,
            );
        }
        if let Some(id) = ids.first() {
            return report(
                "Floating node or island isolated by open switches",
                id,
                &ids,
            );
        }
        let ids: Vec<_> = devices
            .iter()
            .filter(|device| paths.contains_key(&device.p) || paths.contains_key(&device.n))
            .map(|device| device.id)
            .collect();
        if let Some(id) = ids.first() {
            return report("Floating node or island with no ground reference", id, &ids);
        }
    }

    let mut error = Error::new(
        "simulationFailed",
        format!("Circuit matrix has a zero pivot at {time} s"),
    );
    error.target_id = failed_branch.map(|device| device.id.into());
    error
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

fn stamp(system: &mut System, device: &Device<'_>, sample: Sample) {
    let Sample {
        time,
        h,
        interior,
        initial,
    } = sample;
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
        Kind::Diode | Kind::Nmos(_) if device.is_open_switch() => (false, 1.0, 0.0),
        Kind::Inductor(_, current) if h == 0.0 => (false, 1.0, current),
        Kind::Inductor(inductance, current) => {
            let z = inductance / h;
            (true, -z, -z * current)
        }
        Kind::Capacitor(..)
            if initial && device.initial_capacitor == InitialCapacitor::ZeroCurrent =>
        {
            (false, 1.0, 0.0)
        }
        Kind::Capacitor(capacitance, voltage) => {
            (true, -(device.series_resistance + h / capacitance), voltage)
        }
        Kind::Dc(voltage) => (true, -device.series_resistance, voltage),
        Kind::Sine(amplitude, frequency) => (
            true,
            -device.series_resistance,
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
    /// A zero pivot returns its column in the original unknown ordering.
    fn solve(&mut self) -> Result<(), usize> {
        let size = self.rhs.len();
        for k in 0..size {
            let mut pivot = k;
            for i in k + 1..size {
                if self.matrix[i][k].abs() > self.matrix[pivot][k].abs() {
                    pivot = i;
                }
            }
            if self.matrix[pivot][k] == 0.0 {
                return Err(k);
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
        Ok(())
    }
}
