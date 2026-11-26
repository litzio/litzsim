use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordQuantity {
    Voltage,
    Current,
    Output,
}

impl RecordQuantity {
    pub(crate) fn unit(self) -> &'static str {
        match self {
            Self::Voltage => "V",
            Self::Current => "A",
            Self::Output => "1",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    pub quantity: &'static str,
    pub unit: &'static str,
    pub from_pin: &'static str,
    /// None uses the circuit's reference ground.
    pub to_pin: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub struct PartDefinition {
    /// Named terminals in the order used by the archived circuit library.
    pub pins: &'static [&'static str],
    /// Grid offsets from the symbol center, in terminal order at rotation zero.
    pub pin_offsets: &'static [[i32; 2]],
    pub defaults: &'static [(&'static str, f64)],
    pub measurement: Option<Measurement>,
    /// Terminals in recording polarity. None means there are no optional signals.
    pub recording_pins: Option<[&'static str; 2]>,
    /// Schematic designator for new parts, numbered from 1 (R1, Q1, GND1).
    pub id_prefix: &'static str,
    /// Forces the voltage between its two terminals; one net on both is a short.
    pub voltage_source: bool,
    /// Terminal roles in pin order. An empty slice means all are electrical.
    pin_kinds: &'static [PinKind],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PinKind {
    Electrical,
    SignalInput,
    SignalOutput,
}

impl PinKind {
    pub(crate) fn is_signal(self) -> bool {
        self != Self::Electrical
    }
}

impl PartDefinition {
    pub(crate) fn pin_kind(self, pin: &str) -> PinKind {
        self.pins
            .iter()
            .zip(self.pin_kinds)
            .find_map(|(&name, &kind)| (name == pin).then_some(kind))
            .unwrap_or(PinKind::Electrical)
    }

    pub(crate) fn is_control(self) -> bool {
        self.pin_kinds.contains(&PinKind::SignalOutput)
    }

    pub(crate) fn recordable(self) -> &'static [RecordQuantity] {
        if self.is_control() {
            &[RecordQuantity::Output]
        } else if self.recording_pins.is_some() {
            &[RecordQuantity::Voltage, RecordQuantity::Current]
        } else {
            &[]
        }
    }

    pub(crate) fn series_resistance(self) -> Option<(&'static str, f64)> {
        self.defaults
            .iter()
            .copied()
            .find(|(key, _)| matches!(*key, "esr_ohm" | "internalResistance_ohm"))
    }
}

pub fn part_definition(kind: &str) -> Option<PartDefinition> {
    use PinKind::{Electrical, SignalInput, SignalOutput};

    let (pins, defaults): (&[_], &[_]) = match kind {
        "dc" => (
            &["p", "n"],
            &[("voltage_V", 5.0), ("internalResistance_ohm", 0.0)],
        ),
        "sine" => (
            &["p", "n"],
            &[
                ("amplitude_V", 1.0),
                ("frequency_Hz", 50.0),
                ("internalResistance_ohm", 0.0),
            ],
        ),
        "resistor" => (&["p", "n"], &[("resistance_ohm", 1000.0)]),
        "inductor" => (
            &["p", "n"],
            &[("inductance_H", 1e-3), ("initialCurrent_A", 0.0)],
        ),
        "capacitor" => (
            &["p", "n"],
            &[
                ("capacitance_F", 1e-6),
                ("initialVoltage_V", 0.0),
                ("esr_ohm", 1e-6),
            ],
        ),
        "ground" => (&["ref"], &[]),
        "diode" => (&["a", "k"], &[]),
        "nmos" => (&["d", "g", "s"], &[]),
        "timedSwitch" => (
            &["p", "n"],
            &[("initiallyClosed", 0.0), ("switchTime_s", 1e-3)],
        ),
        "pwm" => (
            &["out", "ref"],
            &[("frequency_Hz", 10000.0), ("duty", 0.5), ("high_V", 10.0)],
        ),
        "dutyPwm" => (
            &["out", "ref", "duty"],
            &[("frequency_Hz", 10000.0), ("high_V", 10.0)],
        ),
        "voltmeter" | "ammeter" => (&["p", "n"], &[]),
        "voltageProbe" => (&["p"], &[]),
        "voltageSensor" | "currentSensor" => (&["p", "n", "out"], &[("gain", 1.0)]),
        "constant" => (&["out"], &[("value", 0.0)]),
        "step" => (
            &["out"],
            &[("stepTime_s", 0.0), ("before", 0.0), ("after", 1.0)],
        ),
        "ramp" => (
            &["out"],
            &[
                ("startTime_s", 0.0),
                ("slope_per_s", 1.0),
                ("finalValue", 1.0),
            ],
        ),
        "sum" => (&["in1", "in2", "out"], &[("sign1", 1.0), ("sign2", -1.0)]),
        "gain" => (&["in", "out"], &[("gain", 1.0)]),
        "limiter" => (&["in", "out"], &[("min", 0.0), ("max", 1.0)]),
        "integrator" => (
            &["in", "out"],
            &[("initialValue", 0.0), ("min", -1.0), ("max", 1.0)],
        ),
        "pi" => (
            &["in", "out"],
            &[
                ("kp", 1.0),
                ("ki", 1.0),
                ("initialIntegral", 0.0),
                ("min", 0.0),
                ("max", 1.0),
            ],
        ),
        _ => return None,
    };
    let measurement = match kind {
        "voltmeter" | "voltageProbe" | "voltageSensor" => Some(Measurement {
            quantity: "voltage",
            unit: "V",
            from_pin: pins[0],
            to_pin: pins.get(1).copied(),
        }),
        "ammeter" | "currentSensor" => Some(Measurement {
            quantity: "current",
            unit: "A",
            from_pin: pins[0],
            to_pin: Some(pins[1]),
        }),
        _ => None,
    };
    let pin_offsets: &[_] = match kind {
        "ground" => &[[0, -1]],
        "voltageProbe" => &[[0, 1]],
        // The body diode's anode is at S and cathode at D.
        "nmos" => &[[0, -1], [-1, 0], [0, 1]],
        "resistor" | "inductor" | "ammeter" => &[[-1, 0], [1, 0]],
        "diode" => &[[0, 1], [0, -1]],
        "voltageSensor" => &[[0, -1], [0, 1], [1, 0]],
        "currentSensor" => &[[-1, 0], [1, 0], [0, -1]],
        "constant" | "step" | "ramp" => &[[1, 0]],
        "sum" => &[[-1, -1], [-1, 1], [1, 0]],
        "gain" | "limiter" | "integrator" | "pi" => &[[-1, 0], [1, 0]],
        "dutyPwm" => &[[0, -1], [0, 1], [-1, 0]],
        _ => &[[0, -1], [0, 1]],
    };
    let id_prefix = match kind {
        "dc" | "sine" => "V",
        "resistor" => "R",
        "inductor" => "L",
        "capacitor" => "C",
        "ground" => "GND",
        "diode" => "D",
        "nmos" => "Q",
        "timedSwitch" => "SW",
        "pwm" | "dutyPwm" => "PWM",
        "voltmeter" => "VM",
        "voltageProbe" => "VP",
        "voltageSensor" => "VS",
        "currentSensor" => "CS",
        "constant" => "CONST",
        "step" => "STEP",
        "ramp" => "RAMP",
        "sum" => "SUM",
        "gain" => "GAIN",
        "limiter" => "LIM",
        "integrator" => "INT",
        "pi" => "PI",
        _ => "AM",
    };
    Some(PartDefinition {
        pins,
        pin_offsets,
        defaults,
        measurement,
        recording_pins: match kind {
            "nmos" => Some(["d", "s"]),
            "dc" | "sine" | "resistor" | "inductor" | "capacitor" | "diode" | "timedSwitch" => {
                Some([pins[0], pins[1]])
            }
            _ => None,
        },
        id_prefix,
        voltage_source: matches!(kind, "dc" | "sine" | "pwm" | "dutyPwm"),
        pin_kinds: match kind {
            "voltageSensor" | "currentSensor" => &[Electrical, Electrical, SignalOutput],
            "constant" | "step" | "ramp" => &[SignalOutput],
            "sum" => &[SignalInput, SignalInput, SignalOutput],
            "gain" | "limiter" | "integrator" | "pi" => &[SignalInput, SignalOutput],
            "dutyPwm" => &[Electrical, Electrical, SignalInput],
            _ => &[],
        },
    })
}
