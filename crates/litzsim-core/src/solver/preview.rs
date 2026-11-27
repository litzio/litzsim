use super::control::{Equation, TickClock, task_clock};
use super::{Kind, checked_series_resistance, device_kind, same_clock_time, sine_voltage};
use crate::circuit::{Document, Error};
use serde::Serialize;

const MAX_POINTS: usize = 256;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePreview {
    pub points: Vec<[f64; 2]>,
    pub events: Vec<PreviewEvent>,
    pub unit: &'static str,
    pub hold_samples: bool,
    pub decimated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewEvent {
    pub time: f64,
    pub kind: &'static str,
}

/// Evaluate a source on its own, so it can be previewed while wiring the circuit.
/// Points give the programmed source voltage, control output or switch state.
pub fn source_preview(document: &Document, id: &str) -> Result<SourcePreview, Error> {
    let part = document
        .circuit()
        .parts
        .get(id)
        .ok_or_else(|| Error::invalid(id, "Unknown source"))?;
    let stop = document.stop_time_s();
    let mut preview = SourcePreview {
        points: Vec::new(),
        events: Vec::new(),
        unit: "V",
        hold_samples: false,
        decimated: false,
    };
    match part.kind.as_str() {
        "constant" | "step" | "ramp" => {
            let equation = Equation::from_part(id, part)?;
            let clock = task_clock(document)?;
            preview.unit = "1";
            preview.hold_samples = true;
            match equation {
                Equation::Step { at, .. } => preview.mark(at, stop, "step"),
                Equation::Ramp {
                    start,
                    slope,
                    final_value,
                } => {
                    let end = if final_value == 0.0 {
                        start
                    } else {
                        start + final_value / slope
                    };
                    preview.mark(end, stop, "rampEnd");
                }
                _ => {}
            }
            let (points, decimated) = control_points(&equation, clock, stop)?;
            preview.points = points;
            preview.decimated = decimated;
        }
        "dc" | "sine" | "pwm" | "timedSwitch" => {
            checked_series_resistance(id, part)?;
            let mut clocks = Vec::new();
            preview.points = match device_kind(id, part, &mut clocks)? {
                Kind::Dc(voltage) => vec![[0.0, voltage], [stop, voltage]],
                Kind::Sine(amplitude, frequency) => {
                    let end = stop.min(3.0 / frequency.abs());
                    (0..MAX_POINTS)
                        .map(|i| {
                            let time = end * (i as f64 / (MAX_POINTS - 1) as f64);
                            [time, sine_voltage(amplitude, frequency, time)]
                        })
                        .collect()
                }
                Kind::Pulse(index) => {
                    let clock = &clocks[index];
                    let end = stop.min(3.0 / clock.pulse.frequency);
                    let mut start = 0.0;
                    let mut points = Vec::new();
                    // Plot both limits at the exact edge. The solver evaluates
                    // fixed PWM in the interior of each interval as well.
                    loop {
                        let edge = clock.next_edge(start);
                        let at_end = same_clock_time(edge, end);
                        let limit = if at_end { end } else { edge.min(end) };
                        let value = clock.voltage(start + (limit - start) / 2.0);
                        points.extend([[start, value], [limit, value]]);
                        if at_end {
                            let next = clock.next_edge(end);
                            points.push([end, clock.voltage(end + (next - end) / 2.0)]);
                            break;
                        }
                        if edge > end {
                            break;
                        }
                        start = edge;
                    }
                    points
                }
                Kind::TimedSwitch(switch) => {
                    preview.unit = "state";
                    let state = |time| u8::from(switch.closed(time)) as f64;
                    let initial = state(0.0);
                    let final_state = state(stop);
                    let at = if same_clock_time(switch.at, stop) {
                        stop
                    } else {
                        switch.at
                    };
                    let mut points = vec![[0.0, initial]];
                    if initial != final_state {
                        points.extend([[at, initial], [at, final_state]]);
                    }
                    preview.mark(at, stop, "switch");
                    if points.last().expect("initial point")[0] < stop {
                        points.push([stop, final_state]);
                    }
                    points
                }
                _ => unreachable!("expected a time-dependent device"),
            };
        }
        _ => return Err(Error::invalid(id, "This part has no source preview")),
    }
    if preview.points.iter().any(|point| !point[1].is_finite()) {
        return Err(Error::new("simulationFailed", "Non-finite waveform value"));
    }
    Ok(preview)
}

impl SourcePreview {
    fn mark(&mut self, time: f64, stop: f64, kind: &'static str) {
        if time <= stop || same_clock_time(time, stop) {
            self.events.push(PreviewEvent {
                time: time.min(stop),
                kind,
            });
        }
    }
}

fn control_points(
    equation: &Equation,
    clock: TickClock,
    stop: f64,
) -> Result<(Vec<[f64; 2]>, bool), Error> {
    let first = clock.time(0);
    let first_value = equation.source_value(first);
    let mut points = vec![[0.0, if first == 0.0 { first_value } else { 0.0 }]];
    if first > stop && !same_clock_time(first, stop) {
        points.push([stop, 0.0]);
        return Ok((points, false));
    }
    if first > 0.0 && first_value != 0.0 {
        points.push([
            if same_clock_time(first, stop) {
                stop
            } else {
                first
            },
            first_value,
        ]);
    }
    if matches!(equation, Equation::Constant(_)) {
        if points.last().expect("initial point")[0] < stop {
            points.push([stop, first_value]);
        }
        return Ok((points, false));
    }
    let time = |tick| clock.time(tick);
    let value = |tick| equation.source_value(time(tick));
    // Beyond the last consecutive integer in f64 the solver's tick clock
    // itself would stop advancing.
    let upper = (stop / clock.period() - clock.position).ceil() as u64;
    if upper >= 1 << 53 {
        return Err(Error::new(
            "simulationFailed",
            "Time step below clock resolution",
        ));
    }
    let last =
        upper - u64::from(upper > 0 && time(upper) > stop && !same_clock_time(time(upper), stop));
    let last_value = value(last);
    if first_value == last_value {
        if points.last().expect("initial point")[0] < stop {
            points.push([stop, last_value]);
        }
        return Ok((points, false));
    }

    // Step and clamped ramp are monotonic. Find the active span in ticks
    // without iterating through a potentially long leading or trailing hold.
    let first_change = first_tick(last, |tick| value(tick) != first_value);
    let last_change = first_tick(last, |tick| value(tick) == last_value);
    let span = last_change - first_change;
    let intervals = span.min((MAX_POINTS - 3) as u64);
    for i in 0..=intervals {
        let tick = first_change + i * span / intervals.max(1);
        let at = time(tick);
        points.push([
            if same_clock_time(at, stop) { stop } else { at },
            value(tick),
        ]);
    }
    if points.last().expect("initial point")[0] < stop {
        points.push([stop, last_value]);
    }
    Ok((points, intervals < span))
}

/// First tick for which a monotonic predicate becomes true.
fn first_tick(mut high: u64, predicate: impl Fn(u64) -> bool) -> u64 {
    let mut low = 0;
    while low < high {
        let mid = low + (high - low) / 2;
        if predicate(mid) {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}
