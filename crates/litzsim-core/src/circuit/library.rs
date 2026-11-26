use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    pub quantity: &'static str,
    pub unit: &'static str,
    pub from_pin: &'static str,
    pub to_pin: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct PartDefinition {
    /// Named terminals in the order used by the archived circuit library.
    pub pins: &'static [&'static str],
    /// Grid offsets from the symbol center, in terminal order at rotation zero.
    pub pin_offsets: &'static [[i32; 2]],
    pub defaults: &'static [(&'static str, f64)],
    pub measurement: Option<Measurement>,
}

impl PartDefinition {
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
        _ => return None,
    };
    let measurement = match kind {
        "voltmeter" => Some(Measurement {
            quantity: "voltage",
            unit: "V",
            from_pin: pins[0],
            to_pin: pins[1],
        }),
        "ammeter" => Some(Measurement {
            quantity: "current",
            unit: "A",
            from_pin: pins[0],
            to_pin: pins[1],
        }),
        _ => None,
    };
    let pin_offsets: &[_] = match kind {
        "ground" => &[[0, -1]],
        // The body diode's anode is at S and cathode at D.
        "nmos" => &[[0, -1], [-1, 0], [0, 1]],
        "resistor" | "inductor" | "ammeter" => &[[-1, 0], [1, 0]],
        "diode" => &[[0, 1], [0, -1]],
        _ => &[[0, -1], [0, 1]],
    };
    Some(PartDefinition {
        pins,
        pin_offsets,
        defaults,
        measurement,
    })
}
