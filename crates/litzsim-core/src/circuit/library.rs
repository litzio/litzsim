use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordQuantity {
    Voltage,
    Current,
}

impl RecordQuantity {
    pub(crate) fn unit(self) -> &'static str {
        match self {
            Self::Voltage => "V",
            Self::Current => "A",
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
}

impl PartDefinition {
    pub(crate) fn recordable(self) -> &'static [RecordQuantity] {
        if self.recording_pins.is_some() {
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
        "pwm" => (
            &["out", "ref"],
            &[("frequency_Hz", 10000.0), ("duty", 0.5), ("high_V", 10.0)],
        ),
        "voltmeter" | "ammeter" => (&["p", "n"], &[]),
        "voltageProbe" => (&["p"], &[]),
        _ => return None,
    };
    let measurement = match kind {
        "voltmeter" | "voltageProbe" => Some(Measurement {
            quantity: "voltage",
            unit: "V",
            from_pin: pins[0],
            to_pin: pins.get(1).copied(),
        }),
        "ammeter" => Some(Measurement {
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
        "pwm" => "PWM",
        "voltmeter" => "VM",
        "voltageProbe" => "VP",
        _ => "AM",
    };
    Some(PartDefinition {
        pins,
        pin_offsets,
        defaults,
        measurement,
        recording_pins: match kind {
            "nmos" => Some(["d", "s"]),
            "dc" | "sine" | "resistor" | "inductor" | "capacitor" | "diode" => {
                Some([pins[0], pins[1]])
            }
            _ => None,
        },
        id_prefix,
        voltage_source: matches!(kind, "dc" | "sine" | "pwm"),
    })
}
