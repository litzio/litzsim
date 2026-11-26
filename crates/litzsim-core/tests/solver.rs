use litzsim_core::{
    circuit::Document,
    solver::{simulate, simulate_with_warnings},
};
use serde_json::{Value, json};
use std::time::Instant;

const RC: &str = include_str!("../../../examples/rc.litzsim");
const RLC: &str = include_str!("../../../examples/rlc.litzsim");
const BUCK: &str = include_str!("../../../examples/buck.litzsim");
const CONTROL: &str = include_str!("../../../examples/control-signals.litzsim");

fn document(value: &Value) -> Document {
    Document::from_bytes(&serde_json::to_vec(value).unwrap()).unwrap()
}

fn channel(wave: &litzsim_core::waves::Waveform, name: &str) -> usize {
    wave.channels().iter().position(|c| c.name == name).unwrap()
}

#[test]
fn control_blocks_sample_in_dependency_order_and_hold_between_ticks() {
    let wave = simulate(&Document::from_bytes(CONTROL.as_bytes()).unwrap(), || false).unwrap();
    let columns = ["CONST1", "RAMP1", "STEP1", "SUM1", "SUM2", "GAIN1", "LIM1"].map(|id| {
        let col = channel(&wave, &format!("{id}_out"));
        assert_eq!(wave.channels()[col].unit, "1");
        col
    });
    // Sum1 = 2 - ramp, Sum2 = step - Sum1. Gain = -2 * Sum2.
    let expected = [
        [2.0, 0.0, 1.0, 2.0, -1.0, 2.0, 1.25],
        [2.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.25],
        [2.0, 2.5, -1.0, -0.5, -0.5, 1.0, 1.0],
        [2.0, 3.0, -1.0, -1.0, 0.0, 0.0, 0.25],
    ];
    // Ts = 3/8 s, h = 1/4 s. Two ticks split steps, two land on the grid.
    // A step command at 5/8 s changes only at 3/4 s, the following tick.
    let schedule = [
        (0.0, 0),
        (0.25, 0),
        (0.375, 0),
        (0.375, 1),
        (0.5, 1),
        (0.75, 1),
        (0.75, 2),
        (1.0, 2),
        (1.125, 2),
        (1.125, 3),
        (1.25, 3),
        (1.5, 3),
    ];
    assert_eq!(wave.rows().len(), schedule.len());
    for (row, (time, values)) in wave.rows().zip(schedule) {
        assert_eq!(row[0], time);
        assert_eq!(columns.map(|c| row[c]), expected[values], "at {time} s");
    }
    let measured = wave.measure(&["LIM1_out".into()], 0.3, 0.45).unwrap();
    assert!((measured.channels[0].average.unwrap() - 0.75).abs() <= 1e-12);

    let mut decimated: Value = serde_json::from_str(CONTROL).unwrap();
    decimated["simulation"]["outputStep_s"] = json!(0.5);
    decimated["parts"]["RAMP1"]["parameters"]["finalValue"] = json!(6);
    let wave = simulate(&document(&decimated), || false).unwrap();
    let ramp = channel(&wave, "RAMP1_out");
    let samples: Vec<_> = wave.rows().map(|row| (row[0], row[ramp])).collect();
    // Off-grid ticks still update the held value. At stop the ramp changes
    // from (1.125 - 0.125)*4 to (1.5 - 0.125)*4, so both limits are recorded.
    assert_eq!(
        samples,
        [(0.0, 0.0), (0.5, 1.0), (1.0, 2.5), (1.5, 4.0), (1.5, 5.5)]
    );

    let mut decimal: Value = serde_json::from_str(CONTROL).unwrap();
    decimal["simulation"]["controlSamplePeriod_s"] = json!(0.3);
    decimal["simulation"]["stopTime_s"] = json!(1.0);
    decimal["parts"]["STEP1"]["parameters"]["stepTime_s"] = json!(0.9);
    let wave = simulate(&document(&decimal), || false).unwrap();
    let step = channel(&wave, "STEP1_out");
    assert_eq!(wave.rows().last().unwrap()[step], -1.0);
}

#[test]
fn sampled_sensors_match_meters_and_use_the_left_limit_at_pwm_edges() {
    let mut value: Value = serde_json::from_str(BUCK).unwrap();
    value["simulation"]["controlSamplePeriod_s"] = json!(50e-6);
    value["simulation"]["stopTime_s"] = json!(160e-6);
    for (id, kind, gain) in [("VS1", "voltageSensor", 3.0), ("CS1", "currentSensor", 2.0)] {
        value["parts"][id] = json!({
            "type": kind, "parameters": {"gain": gain}, "record": {"output": true}
        });
    }
    value["parts"]["VG"] = json!({"type": "voltmeter", "parameters": {}});
    value["parts"]["AM1"] = json!({"type": "ammeter", "parameters": {}});
    value["nets"]["gate"]
        .as_array_mut()
        .unwrap()
        .extend([json!("VS1.n"), json!("VG.n")]);
    let pins = value["nets"]["sw"].as_array_mut().unwrap();
    *pins.iter_mut().find(|pin| **pin == "L1.p").unwrap() = json!("AM1.p");
    pins.extend([json!("VS1.p"), json!("VG.p")]);
    value["nets"]["meter"] = json!(["AM1.n", "CS1.n"]);
    value["nets"]["coil"] = json!(["CS1.p", "L1.p"]);

    let wave = simulate(&document(&value), || false).unwrap();
    let [vg, am, vs, cs] = ["VG_V", "AM1_A", "VS1_out", "CS1_out"].map(|name| channel(&wave, name));
    let mut rows = wave.rows().peekable();
    let mut updates = 0;
    let mut held = [0.0; 2];
    while let Some(left) = rows.next() {
        let time = left[0];
        let right = if rows.peek().is_some_and(|row| row[0] == time) {
            rows.next().unwrap()
        } else {
            left
        };
        if time > 0.0 {
            assert_eq!([left[vs], left[cs]], held);
        }
        if (time - updates as f64 * 50e-6).abs() < 1e-18 {
            held = [3.0 * left[vg], -2.0 * left[am]];
            updates += 1;
        }
        assert_eq!(right[vs], held[0]);
        assert!((right[cs] - held[1]).abs() <= 1e-12);
        held[1] = right[cs];
        if (time - 50e-6).abs() < 1e-18 {
            assert_eq!([left[vg], right[vg], right[vs]], [-10.0, 0.0, -30.0]);
            assert!(right[cs] < 0.0);
        }
        assert!(rows.peek().is_none_or(|row| row[0] > time));
    }
    assert_eq!(updates, 4);
}

#[test]
fn final_control_change_at_pwm_edge_matches_the_rows_of_a_longer_run() {
    let mut value: Value = serde_json::from_str(BUCK).unwrap();
    let edge = 50e-6;
    value["simulation"]["timeStep_s"] = json!(3e-7);
    value["parts"]["VG"] = json!({"type": "voltmeter", "parameters": {}});
    value["nets"]["gate"]
        .as_array_mut()
        .unwrap()
        .push(json!("VG.p"));
    value["nets"]["sw"]
        .as_array_mut()
        .unwrap()
        .push(json!("VG.n"));

    let at_edge = |value: &mut Value, short| {
        value["simulation"]["stopTime_s"] = json!(if short { edge } else { 51e-6 });
        let wave = simulate(&document(value), || false).unwrap();
        wave.rows()
            .filter(|row| row[0] == edge)
            .map(<[f64]>::to_vec)
            .collect::<Vec<_>>()
    };
    let electrical_stop = at_edge(&mut value, true);
    assert_eq!(
        electrical_stop.len(),
        1,
        "legacy stop has only its left limit"
    );

    value["simulation"]["controlSamplePeriod_s"] = json!(edge);
    value["parts"]["VS"] = json!({
        "type": "voltageSensor", "parameters": {"gain": 2}, "record": {"output": true}
    });
    value["parts"]["S"] = json!({
        "type": "step", "parameters": {"stepTime_s": edge, "before": 0, "after": 1},
        "record": {"output": true}
    });
    value["nets"]["gate"]
        .as_array_mut()
        .unwrap()
        .push(json!("VS.p"));
    value["nets"]["sw"]
        .as_array_mut()
        .unwrap()
        .push(json!("VS.n"));
    let continued = at_edge(&mut value, false);
    assert_eq!(continued.len(), 2);
    // VG_V drops as the step changes. VS_out holds twice the left-limit VG_V.
    assert_eq!(continued[0][2..], [10.0, 0.0, 20.0]);
    assert_eq!(continued[1][2..], [0.0, 1.0, 20.0]);
    assert_eq!(at_edge(&mut value, true), continued);
}

#[test]
fn final_pwm_control_rows_at_70_us() {
    check_pwm_control_stop(50_000.0, 2.5e-6, 70e-6, 70e-6, 70e-6);
}

#[test]
fn final_pwm_control_rows_at_50_us_on_the_default_grid() {
    check_pwm_control_stop(10_000.0, 1e-7, 50e-6, 50e-6, 50e-6);
}

#[test]
fn final_pwm_control_rows_match_with_one_ulp_clock_offsets() {
    let adjacent = |t: f64| [t.next_down(), t, t.next_up()];
    for (frequency, cycle) in [
        (10_000.0, 0.0),
        (20_000.0, 1.0),
        (40_000.0, 2.0),
        (50_000.0, 3.0),
    ] {
        let edge = (cycle + 0.5) / frequency;
        for ticks in [7.0, 28.0, 500.0] {
            for step in adjacent(edge / ticks) {
                let grid = ticks * step;
                for sample in adjacent(edge) {
                    for boundary in [grid, edge, sample] {
                        for stop in adjacent(boundary) {
                            check_pwm_control_stop(frequency, step, edge, sample, stop);
                        }
                    }
                }
            }
        }
    }
}

fn check_pwm_control_stop(frequency: f64, step: f64, edge: f64, sample: f64, stop: f64) {
    let mut value: Value = serde_json::from_str(BUCK).unwrap();
    value["simulation"]["timeStep_s"] = json!(step);
    value["simulation"]["controlSamplePeriod_s"] = json!(sample);
    value["parts"]["PWM"]["parameters"]["frequency_Hz"] = json!(frequency);
    value["parts"]["VG"] = json!({"type": "voltmeter", "parameters": {}});
    value["parts"]["VS"] = json!({
        "type": "voltageSensor", "parameters": {"gain": 2}, "record": {"output": true}
    });
    value["parts"]["S"] = json!({
        "type": "step", "parameters": {"stepTime_s": sample, "before": 0, "after": 1},
        "record": {"output": true}
    });
    value["nets"]["gate"]
        .as_array_mut()
        .unwrap()
        .extend([json!("VG.p"), json!("VS.p")]);
    value["nets"]["sw"]
        .as_array_mut()
        .unwrap()
        .extend([json!("VG.n"), json!("VS.n")]);
    let context = format!(
        "frequency={frequency}, step={step:?}, edge={edge:?}, Ts={sample:?}, stop={stop:?}"
    );
    let simulate_until = |stop, value: &mut Value| {
        value["simulation"]["stopTime_s"] = json!(stop);
        simulate(&document(value), || false).unwrap_or_else(|error| panic!("{context}: {error}"))
    };
    let short = simulate_until(stop, &mut value);
    let long = simulate_until(edge + 1e-6, &mut value);
    // The short run labels the merged rows with stop; the long run uses the
    // nearby grid or clock boundary. Compare times within roundoff.
    let time_tolerance = 8.0 * f64::EPSILON * edge;
    let at_edge = |wave: &litzsim_core::waves::Waveform| {
        wave.rows()
            .filter(|row| (row[0] - edge).abs() <= time_tolerance)
            .map(<[f64]>::to_vec)
            .collect::<Vec<_>>()
    };
    let final_rows = at_edge(&short);
    let continued = at_edge(&long);
    assert_eq!(final_rows.len(), 2, "{context}: short");
    assert_eq!(continued.len(), 2, "{context}: long");
    assert_eq!(
        short
            .rows()
            .rev()
            .take(2)
            .map(|row| row[0])
            .collect::<Vec<_>>(),
        [stop; 2],
        "{context}"
    );
    assert_eq!(continued[0][2..], [10.0, 0.0, 20.0], "{context}");
    assert_eq!(continued[1][2..], [0.0, 1.0, 20.0], "{context}");
    for (final_row, continued_row) in final_rows.iter().zip(&continued) {
        assert!(
            (final_row[0] - continued_row[0]).abs() <= time_tolerance,
            "{context}"
        );
        // Moving the solve time by one ULP may round the capacitor voltage differently.
        assert!(
            (final_row[1] - continued_row[1]).abs() <= 1e-12,
            "{context}"
        );
        assert_eq!(final_row[2..], continued_row[2..], "{context}");
    }
}

#[test]
fn control_execution_reports_the_cycle_and_driverless_inputs() {
    let mut value: Value = serde_json::from_str(CONTROL).unwrap();
    value["nets"]["constant"] = json!(["GAIN1.out", "SUM1.in1", "LIM1.in"]);
    value["nets"].as_object_mut().unwrap().remove("scaled");
    value["nets"]["ramp"] = json!(["RAMP1.out", "SUM1.in2", "SUM2.in1"]);
    value["nets"].as_object_mut().unwrap().remove("step");
    let error = simulate(&document(&value), || false).unwrap_err();
    assert_eq!(error.target_id.as_deref(), Some("GAIN1"));
    assert_eq!(error.message, "Algebraic loop: GAIN1, SUM2, SUM1");

    value["nets"]["constant"] = json!(["SUM1.in1", "LIM1.in"]);
    let error = simulate(&document(&value), || false).unwrap_err();
    assert_eq!(error.target_id.as_deref(), Some("LIM1"));
    assert!(error.message.contains("Unconnected signal input LIM1.in"));
    value["nets"]["constant"] = json!(["CONST1.out", "SUM1.in1"]);
    let error = simulate(&document(&value), || false).unwrap_err();
    assert_eq!(error.target_id.as_deref(), Some("LIM1"));
    assert!(error.message.contains("Unconnected signal input LIM1.in"));
}

const LOAD_STEP: &str = include_str!("../../../examples/rc-load-step.litzsim");

#[test]
fn timed_switch_closes_a_parallel_rc_load_off_grid() {
    let mut circuit: Value = serde_json::from_str(LOAD_STEP).unwrap();
    let edge = 755e-6;
    let before = |time: f64| 5.0 * (1.0 - (-time / 1e-3).exp());
    let after = |time: f64| 2.5 + (before(edge) - 2.5) * (-(time - edge) / 5e-4).exp();
    let wave = simulate(&document(&circuit), || false).unwrap();
    let (v, sw_v, sw_i) = (
        channel(&wave, "C1_V"),
        channel(&wave, "SW1_V"),
        channel(&wave, "SW1_A"),
    );
    let at_edge: Vec<_> = wave.rows().filter(|row| row[0] == edge).collect();
    assert_eq!(at_edge.len(), 2);
    assert_eq!(at_edge[0][v], at_edge[1][v]);
    let mut closed = false;
    for row in wave.rows() {
        let expected = if closed {
            after(row[0])
        } else {
            before(row[0])
        };
        // Backward Euler, h/tau <= 0.004 on either segment. Allow 8 mV
        // for first-order integration error including the carried edge error.
        assert!((row[v] - expected).abs() < 8e-3, "{row:?}");
        if closed {
            assert_eq!(row[sw_v], 0.0);
            assert!((row[sw_i] - row[v] / 1000.0).abs() < 1e-14);
        } else {
            assert_eq!(row[sw_i], 0.0);
            assert!((row[sw_v] - row[v]).abs() < 1e-12);
        }
        closed |= row[0] == edge;
    }
    circuit["simulation"]["stopTime_s"] = json!(edge);
    let short = simulate(&document(&circuit), || false).unwrap();
    assert_eq!(
        short
            .rows()
            .filter(|row| row[0] == edge)
            .collect::<Vec<_>>(),
        at_edge
    );
}

#[test]
fn timed_switch_opens_a_parallel_rl_load_off_grid() {
    let mut circuit: Value = serde_json::from_str(LOAD_STEP).unwrap();
    circuit["parts"]["R1"] = json!({
        "type": "inductor", "parameters": {"inductance_H": 0.001, "initialCurrent_A": 0.5},
        "record": {"current": true}
    });
    circuit["parts"]["C1"] = json!({"type": "resistor", "parameters": {"resistance_ohm": 1}});
    circuit["parts"]["R2"]["parameters"]["resistance_ohm"] = json!(1);
    circuit["parts"]["SW1"]["parameters"]["initiallyClosed"] = json!(1);
    let edge = 755e-6;
    let before = |time: f64| 10.0 + (0.5 - 10.0) * (-time / 2e-3).exp();
    let after = |time: f64| 5.0 + (before(edge) - 5.0) * (-(time - edge) / 1e-3).exp();
    let wave = simulate(&document(&circuit), || false).unwrap();
    let (i, sw_v, sw_i) = (
        channel(&wave, "R1_A"),
        channel(&wave, "SW1_V"),
        channel(&wave, "SW1_A"),
    );
    let at_edge: Vec<_> = wave.rows().filter(|row| row[0] == edge).collect();
    assert_eq!(at_edge.len(), 2);
    assert_eq!(at_edge[0][i], at_edge[1][i]);
    let mut open = false;
    for row in wave.rows() {
        let expected = if open { after(row[0]) } else { before(row[0]) };
        // h/tau <= 0.002: allow 5 mA including error carried across the edge.
        assert!((row[i] - expected).abs() < 5e-3, "{row:?}");
        if open {
            assert_eq!(row[sw_i], 0.0);
            assert!((row[sw_v] - row[i]).abs() < 1e-12);
        } else {
            // The ideal constraint is zero. Pivoting can leave a rounding
            // residual when it subtracts terms of the order of one volt.
            assert!(row[sw_v].abs() < 1e-12);
            assert!((row[sw_i] - row[i] / 2.0).abs() < 1e-12);
        }
        open |= row[0] == edge;
    }
}

#[test]
fn timed_switch_and_control_share_final_rows_with_one_ulp_clock_offsets() {
    let adjacent = |time: f64| [time.next_down(), time, time.next_up()];
    let mut circuit: Value = serde_json::from_str(LOAD_STEP).unwrap();
    let edge = 750e-6;
    circuit["parts"]["S"] = json!({
        "type": "step", "parameters": {"stepTime_s": edge, "before": 0, "after": 1},
        "record": {"output": true}
    });
    for step in [edge / 7.0, edge / 500.0].into_iter().flat_map(adjacent) {
        circuit["simulation"]["timeStep_s"] = json!(step);
        for at in adjacent(edge) {
            circuit["parts"]["SW1"]["parameters"]["switchTime_s"] = json!(at);
            for sample in adjacent(edge) {
                circuit["simulation"]["controlSamplePeriod_s"] = json!(sample);
                circuit["simulation"]["stopTime_s"] = json!(0.001);
                let long = simulate(&document(&circuit), || false).unwrap();
                let continued: Vec<_> = long
                    .rows()
                    .filter(|row| (row[0] - edge).abs() <= 8.0 * f64::EPSILON * edge)
                    .collect();
                assert_eq!(continued.len(), 2);
                for stop in adjacent(edge) {
                    circuit["simulation"]["stopTime_s"] = json!(stop);
                    let short = simulate(&document(&circuit), || false).unwrap();
                    let final_rows: Vec<_> = short.rows().filter(|row| row[0] == stop).collect();
                    assert_eq!(final_rows.len(), 2);
                    assert_eq!(short.rows().last().unwrap()[0], stop);
                    let (v, sw_v, sw_i, out) = (
                        channel(&short, "C1_V"),
                        channel(&short, "SW1_V"),
                        channel(&short, "SW1_A"),
                        channel(&short, "S_out"),
                    );
                    assert_eq!(final_rows[0][out], 0.0);
                    assert_eq!(final_rows[1][out], 1.0);
                    assert_eq!(final_rows[0][sw_i], 0.0);
                    assert!(final_rows[0][sw_v] > 2.0);
                    assert_eq!(final_rows[1][sw_v], 0.0);
                    assert!(final_rows[1][sw_i] > 2e-3);
                    for (short, long) in final_rows.into_iter().zip(&continued) {
                        for index in [v, sw_v, sw_i] {
                            assert!((short[index] - long[index]).abs() <= 1e-12);
                        }
                        assert_eq!(short[out], long[out]);
                    }
                }
            }
        }
    }
}

#[test]
fn timed_switch_opening_reports_the_floating_node_at_the_edge() {
    let mut circuit: Value = serde_json::from_str(LOAD_STEP).unwrap();
    circuit["parts"].as_object_mut().unwrap().remove("R2");
    circuit["parts"]["VM"] = json!({"type": "voltmeter", "parameters": {}});
    circuit["layout"].as_object_mut().unwrap().remove("R2");
    circuit["nets"]["output"] = json!(["R1.n", "C1.p"]);
    circuit["nets"]["branch"] = json!(["SW1.p", "VM.p"]);
    circuit["nets"]["ground"]
        .as_array_mut()
        .unwrap()
        .push(json!("VM.n"));
    circuit["parts"]["SW1"]["parameters"]["initiallyClosed"] = json!(1);
    circuit["simulation"]["stopTime_s"] = json!(755e-6);
    let error = simulate(&document(&circuit), || false).unwrap_err();
    assert_eq!(error.code, "singularCircuit");
    assert_eq!(error.target_id.as_deref(), Some("SW1"));
    assert!(
        error
            .message
            .contains("Floating node or island isolated by open switches at 0.000755 s")
    );
}

#[test]
fn buck_records_only_the_installed_meters_by_default() {
    let mut value: Value = serde_json::from_str(BUCK).unwrap();
    value["parts"]["AM1"] = json!({"type": "ammeter", "parameters": {}});
    let pins = value["nets"]["sw"].as_array_mut().unwrap();
    *pins.iter_mut().find(|pin| **pin == "L1.p").unwrap() = json!("AM1.p");
    value["nets"]["meter"] = json!(["AM1.n", "L1.p"]);
    let wave = simulate(&document(&value), || false).unwrap();
    let channels: Vec<_> = wave.channels().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(channels, ["time", "AM1_A", "VC_V"]);
}

#[test]
fn optional_signals_match_meters_in_terminal_direction_at_each_row() {
    let buck: Value = serde_json::from_str(BUCK).unwrap();
    let mut sine: Value = serde_json::from_str(RC).unwrap();
    sine["parts"]["V1"] = json!({"type": "sine", "parameters": {
        "amplitude_V": 5.0, "frequency_Hz": 50.0, "internalResistance_ohm": 0.5
    }});
    let cases = [
        (&buck, "V1", "p", "n", "input", "ground"),
        (&buck, "R1", "p", "n", "output", "ground"),
        (&buck, "C1", "p", "n", "output", "ground"),
        (&buck, "L1", "p", "n", "sw", "output"),
        (&buck, "D1", "a", "k", "ground", "sw"),
        (&buck, "Q1", "d", "s", "input", "sw"),
        (&sine, "V1", "p", "n", "input", "ground"),
    ];
    for (example, id, p, n, from, to) in cases {
        let mut value = example.clone();
        value["parts"][id]["record"] = json!({"voltage": true, "current": true});
        value["parts"]["AM1"] = json!({"type": "ammeter", "parameters": {}});
        value["parts"]["VM1"] = json!({"type": "voltmeter", "parameters": {}});
        let first = format!("{id}.{p}");
        let pins = value["nets"][from].as_array_mut().unwrap();
        *pins.iter_mut().find(|pin| **pin == first).unwrap() = json!("AM1.p");
        value["nets"][to]
            .as_array_mut()
            .unwrap()
            .push(json!("VM1.n"));
        value["nets"]["meter"] = json!(["AM1.n", first, "VM1.p"]);
        assert!(
            value["nets"][to]
                .as_array()
                .unwrap()
                .contains(&json!(format!("{id}.{n}")))
        );
        let wave = simulate(&document(&value), || false).unwrap();
        let [voltage, current, vm, am] = [
            format!("{id}_V"),
            format!("{id}_A"),
            "VM1_V".into(),
            "AM1_A".into(),
        ]
        .map(|name| channel(&wave, &name));
        let mut nonzero = [false; 2];
        for row in wave.rows() {
            assert_eq!(
                row[voltage].to_bits(),
                row[vm].to_bits(),
                "{id} at {}",
                row[0]
            );
            // A series meter adds MNA rows. Allow roundoff in the current
            // from the separate branch equation (or Ohm's law for a resistor).
            assert!(
                (row[current] - row[am]).abs() <= 1e-12,
                "{id} at {}: {} vs {}",
                row[0],
                row[current],
                row[am]
            );
            nonzero[0] |= row[voltage].abs() > 1e-12;
            nonzero[1] |= row[current].abs() > 1e-12;
        }
        assert_eq!(nonzero, [true, true], "both quantities exercised for {id}");
    }
}

#[test]
fn buck_probe_matches_a_two_pin_voltmeter_without_changing_existing_samples() {
    let original = simulate(&Document::from_bytes(BUCK.as_bytes()).unwrap(), || false).unwrap();
    let mut value: Value =
        serde_json::from_str(include_str!("../../../examples/buck-probe.litzsim")).unwrap();
    value["parts"]["VM1"] = json!({"type": "voltmeter", "parameters": {}});
    value["nets"]["sw"]
        .as_array_mut()
        .unwrap()
        .push(json!("VM1.p"));
    value["nets"]["ground"]
        .as_array_mut()
        .unwrap()
        .push(json!("VM1.n"));
    let measured = simulate(&document(&value), || false).unwrap();
    let columns: Vec<_> = original
        .channels()
        .iter()
        .map(|c| channel(&measured, &c.name))
        .collect();
    let probe = channel(&measured, "VP1_V");
    let meter = channel(&measured, "VM1_V");
    assert_eq!(measured.channels()[probe].unit, "V");
    assert_eq!(measured.rows().count(), original.rows().count());
    for (before, after) in original.rows().zip(measured.rows()) {
        assert_eq!(after[probe].to_bits(), after[meter].to_bits());
        assert!(
            columns
                .iter()
                .enumerate()
                .all(|(index, &column)| before[index].to_bits() == after[column].to_bits())
        );
    }
}

fn singular_circuit(parts: Value, nets: Value, reason: &str, ids: &[&str]) {
    let mut value = json!({
        "version": 1,
        "layout": {},
        "simulation": {"timeStep_s": 1e-6, "stopTime_s": 1e-5},
        "parts": parts,
        "nets": nets
    });
    value["parts"]["G"] = json!({"type": "ground", "parameters": {}});
    let error = simulate(&document(&value), || false).unwrap_err();
    assert_eq!(error.code, "singularCircuit", "{error:?}");
    assert!(
        ids.contains(&error.target_id.as_deref().unwrap()),
        "{error:?}"
    );
    let (diagnosis, related) = error.message.split_once(" at 0 s: ").unwrap();
    assert!(diagnosis.starts_with(reason), "{error:?}");
    let mut related: Vec<_> = related.split(", ").collect();
    let mut expected = ids.to_vec();
    related.sort_unstable();
    expected.sort_unstable();
    assert_eq!(related, expected);
}

#[test]
fn identifies_parallel_initial_voltage_constraints_and_voltage_source_loops() {
    singular_circuit(
        json!({
            "Vin": {"type": "dc", "parameters": {"voltage_V": 5}},
            "Cin": {"type": "capacitor", "parameters": {"capacitance_F": 1e-6, "initialVoltage_V": 0, "esr_ohm": 0}}
        }),
        json!({"input": ["Vin.p", "Cin.p"], "gnd": ["G.ref", "Vin.n", "Cin.n"]}),
        "Ideal voltage constraint loop (add capacitor ESR or source internal resistance)",
        &["Vin", "Cin"],
    );
    singular_circuit(
        json!({
            "Vin": {"type": "dc", "parameters": {"voltage_V": 5}},
            "Vdrop": {"type": "dc", "parameters": {"voltage_V": 2}},
            "Vout": {"type": "dc", "parameters": {"voltage_V": 3}}
        }),
        json!({
            "input": ["Vin.p", "Vdrop.p"],
            "output": ["Vdrop.n", "Vout.p"],
            "gnd": ["G.ref", "Vin.n", "Vout.n"]
        }),
        "Ideal voltage constraint loop",
        &["Vin", "Vdrop", "Vout"],
    );
}

#[test]
fn identifies_initial_inductor_current_cut_off_by_an_open_diode() {
    singular_circuit(
        json!({
            "Vin": {"type": "dc", "parameters": {"voltage_V": 5}},
            "L1": {"type": "inductor", "parameters": {"inductance_H": 1e-3, "initialCurrent_A": 0.2}},
            "D1": {"type": "diode", "parameters": {}}
        }),
        json!({
            "input": ["Vin.p", "L1.p"],
            "isolated": ["L1.n", "D1.a"],
            "gnd": ["G.ref", "Vin.n", "D1.k"]
        }),
        "Initial inductor current cutset",
        &["L1", "D1"],
    );
}

#[test]
fn identifies_floating_node_between_open_diodes() {
    singular_circuit(
        json!({
            "Vin": {"type": "dc", "parameters": {"voltage_V": 5}},
            "D1": {"type": "diode", "parameters": {}},
            "D2": {"type": "diode", "parameters": {}},
            "VM": {"type": "voltmeter", "parameters": {}}
        }),
        json!({
            "input": ["Vin.p", "D1.a"],
            "isolated": ["D1.k", "D2.a", "VM.p"],
            "gnd": ["G.ref", "Vin.n", "D2.k", "VM.n"]
        }),
        "Floating node",
        &["D1", "D2"],
    );
}

#[test]
fn rc_and_rlc_converge_to_the_step_response() {
    for (name, example, archive_mv) in [
        ("RC", RC, [9.159, 4.589, 2.297]),
        ("RLC", RLC, [19.435, 9.751, 4.884]),
    ] {
        let mut value: Value = serde_json::from_str(example).unwrap();
        let mut previous = f64::INFINITY;
        for (h, reference_mv) in [10e-6, 5e-6, 2.5e-6].into_iter().zip(archive_mv) {
            value["simulation"]["timeStep_s"] = json!(h);
            let circuit = document(&value);
            let start = Instant::now();
            let wave = simulate(&circuit, || false).unwrap();
            let elapsed = start.elapsed();
            let vc = channel(&wave, "VC_V");
            // At t=0 the ideal capacitor voltage is zero. The terminal
            // voltage includes the ESR drop from the RC divider; the RLC
            // inductor instead holds the initial series current at zero.
            let initial_terminal = if name == "RC" {
                5.0 * 1e-6 / (1000.0 + 1e-6)
            } else {
                0.0
            };
            assert!((wave.rows().next().unwrap()[vc] - initial_terminal).abs() <= 1e-14);
            assert_eq!(wave.rows().last().unwrap()[0], 5e-3);
            let wd = 750_000.0_f64.sqrt();
            let error = wave
                .rows()
                .map(|row| {
                    let t = row[0];
                    let reference = if name == "RC" {
                        5.0 * (1.0 - (-1000.0 * t).exp())
                    } else {
                        5.0 * (1.0
                            - (-500.0 * t).exp() * ((wd * t).cos() + 500.0 / wd * (wd * t).sin()))
                    };
                    (row[vc] - reference).abs()
                })
                .fold(0.0, f64::max);
            println!(
                "{name} h={h:e}s max_error={:.6}mV solve={elapsed:?}",
                error * 1000.0
            );
            // Issue #53 adopts the archived error with a relative 1e-3 tolerance.
            let reference_error = reference_mv * 1e-3;
            assert!((error - reference_error).abs() <= reference_error * 1e-3);
            assert!(
                error <= 0.04 && error <= 0.55 * previous,
                "{error} after {previous}"
            );
            previous = error;
        }
    }
}

fn inrush(step: f64, initial: f64) -> Value {
    json!({
        "version": 1,
        "layout": {},
        "simulation": {"timeStep_s": step, "stopTime_s": 5e-6},
        "parts": {
            "G": {"type": "ground", "parameters": {}},
            "V": {"type": "dc", "parameters": {"voltage_V": 5}},
            "C": {"type": "capacitor", "parameters": {"capacitance_F": 1e-6, "initialVoltage_V": initial}},
            "I": {"type": "ammeter", "parameters": {}}
        },
        "nets": {
            "ground": ["G.ref", "V.n", "C.n"],
            "supply": ["V.p", "I.p"],
            "input": ["I.n", "C.p"]
        }
    })
}

#[test]
fn inrush_current_converges_and_charges_the_capacitor() {
    let mut previous = None;
    for step in [10e-9, 5e-9, 2.5e-9] {
        let mut value = inrush(step, 0.0);
        value["parts"]["C"]["parameters"]["esr_ohm"] = json!(1.0);
        let start = Instant::now();
        let (wave, warnings) = simulate_with_warnings(&document(&value), |_| false).unwrap();
        let elapsed = start.elapsed();
        assert!(warnings.is_empty());
        let i = channel(&wave, "I_A");
        assert_eq!(wave.rows().next().unwrap()[i], 5.0);
        let (mut error, mut charge, mut time) = (0.0_f64, 0.0, 0.0);
        for row in wave.rows() {
            let reference = 5.0 * (-row[0] / 1e-6).exp();
            error = error.max((row[i] - reference).abs());
            charge += row[i] * (row[0] - time);
            time = row[0];
        }
        if let Some(coarser) = previous {
            assert!(
                (0.48..=0.52).contains(&(error / coarser)),
                "{error} after {coarser}"
            );
        }
        let full_charge = 5e-6;
        let reference_charge = full_charge * (1.0 - (-5.0_f64).exp());
        assert!((charge - reference_charge).abs() <= 2e-4 * full_charge);
        assert!((charge - full_charge).abs() <= 0.01 * full_charge);
        println!("inrush h={step:e}s max_error={error:e}A charge={charge:e}C elapsed={elapsed:?}");
        previous = Some(error);
    }
}

#[test]
fn aligned_ideal_dc_initializes_parallel_capacitors_at_zero_current() {
    let mut value = inrush(1e-6, 5.0);
    value["parts"]["C"]["parameters"]["esr_ohm"] = json!(0.0);
    // A second capacitor, directly connected with reversed terminal polarity.
    value["parts"]["C2"] = value["parts"]["C"].clone();
    value["parts"]["C2"]["parameters"]["initialVoltage_V"] = json!(-5.0);
    value["nets"]["supply"]
        .as_array_mut()
        .unwrap()
        .push(json!("C2.n"));
    value["nets"]["ground"]
        .as_array_mut()
        .unwrap()
        .push(json!("C2.p"));
    let (wave, warnings) = simulate_with_warnings(&document(&value), |_| false).unwrap();
    assert!(warnings.is_empty());
    let i = channel(&wave, "I_A");
    assert!(wave.rows().all(|row| row[i] == 0.0));
}

#[test]
fn source_internal_resistance_applies_to_dc_and_sine() {
    let mut value = inrush(1e-6, 0.0);
    value["parts"]["C"]["parameters"]["esr_ohm"] = json!(0.0);
    value["parts"]["V"]["parameters"]["internalResistance_ohm"] = json!(2.0);
    let wave = simulate(&document(&value), || false).unwrap();
    let i = channel(&wave, "I_A");
    assert_eq!(wave.rows().next().unwrap()[i], 2.5);
    assert!((wave.rows().nth(1).unwrap()[i] - 5.0 / 3.0).abs() < 1e-12);

    value["parts"]["V"] = json!({"type": "sine", "parameters": {
        "amplitude_V": 5, "frequency_Hz": 250_000, "internalResistance_ohm": 2
    }});
    let wave = simulate(&document(&value), || false).unwrap();
    assert_eq!(wave.rows().next().unwrap()[i], 0.0);
    assert!((wave.rows().nth(1).unwrap()[i] - 5.0 / 3.0).abs() < 1e-12);
}

fn stiff_buck(initial: f64) -> Value {
    let mut value: Value = serde_json::from_str(BUCK).unwrap();
    value["parts"]["C1"] = json!({"type": "dc", "parameters": {"voltage_V": 3}});
    value["parts"]["L1"]["parameters"]["initialCurrent_A"] = json!(initial);
    value["simulation"] = json!({"timeStep_s": 0.37e-6, "stopTime_s": 100e-6});
    value
}

fn record_buck_currents(value: &mut Value) {
    for id in ["L1", "D1"] {
        value["parts"][id]["record"] = json!({"current": true});
    }
}

#[test]
fn buck_load_step_and_pwm_falling_edge_share_one_pair_of_rows() {
    let edge = 50e-6;
    let mut circuit = stiff_buck(0.2);
    record_buck_currents(&mut circuit);
    let load_step: Value = serde_json::from_str(LOAD_STEP).unwrap();
    circuit["parts"]["R2"] = load_step["parts"]["R2"].clone();
    circuit["parts"]["SW1"] = load_step["parts"]["SW1"].clone();
    circuit["parts"]["SW1"]["parameters"]["switchTime_s"] = json!(edge);
    circuit["nets"]["branch"] = load_step["nets"]["branch"].clone();
    circuit["nets"]["output"]
        .as_array_mut()
        .unwrap()
        .push(json!("R2.p"));
    circuit["nets"]["ground"]
        .as_array_mut()
        .unwrap()
        .push(json!("SW1.n"));
    circuit["simulation"]["stopTime_s"] = json!(edge);
    let wave = simulate(&document(&circuit), || false).unwrap();
    let at_edge: Vec<_> = wave.rows().filter(|row| row[0] == edge).collect();
    assert_eq!(at_edge.len(), 2);
    let (il, diode, sw_v, sw_i) = (
        channel(&wave, "L1_A"),
        channel(&wave, "D1_A"),
        channel(&wave, "SW1_V"),
        channel(&wave, "SW1_A"),
    );
    // With the output clamped at 3 V, the first high interval adds
    // (5 V - 3 V) * 50 us / 1 mH = 0.1 A to the initial 0.2 A.
    for row in &at_edge {
        assert!((row[il] - 0.3).abs() < 1e-9);
    }
    assert!(at_edge[0][diode].abs() < 1e-9);
    assert!((at_edge[1][diode] - 0.3).abs() < 1e-9);
    for (row, voltage, current) in [(at_edge[0], 3.0, 0.0), (at_edge[1], 0.0, 0.003)] {
        assert!((row[sw_v] - voltage).abs() < 1e-12);
        assert!((row[sw_i] - current).abs() < 1e-12);
    }
}

#[test]
fn buck_matches_ccm_and_dcm_through_two_pwm_periods() {
    const STEP: f64 = 0.37e-6;
    const PERIOD: f64 = 100e-6;
    const FALLING: f64 = PERIOD / 2.0;
    for (initial, reference_peak) in [(0.2, 0.30), (0.0, 0.10)] {
        let mut value = stiff_buck(initial);
        record_buck_currents(&mut value);
        for cycles in [1, 2] {
            value["simulation"]["stopTime_s"] = json!(cycles as f64 * PERIOD);
            let circuit = document(&value);
            let start = Instant::now();
            let wave = simulate(&circuit, || false).unwrap();
            let elapsed = start.elapsed();
            let il = channel(&wave, "L1_A");
            let diode = channel(&wave, "D1_A");
            let mut peak = 0.0_f64;
            let mut zero = None;
            let mut maximum_error = 0.0_f64;
            let mut saw_falling_edge = false;
            let mut saw_rising_edge = false;
            let mut previous = f64::NAN;
            let mut right_limits = 0;
            for row in wave.rows() {
                let t = row[0];
                // A second row at an edge time is its right limit: the switch
                // state of the next interval with the same inductor current.
                let right = t == previous;
                previous = t;
                let (phase, start) = if t <= PERIOD {
                    (t, initial)
                } else {
                    (t - PERIOD, (initial - 0.05).max(0.0))
                };
                let reference = if phase <= FALLING {
                    start + 2000.0 * phase
                } else {
                    (start + 0.1 - 3000.0 * (phase - FALLING)).max(0.0)
                };
                maximum_error = maximum_error.max((row[il] - reference).abs());
                assert!(row[diode] >= -1e-9, "reverse diode current at {t}");
                let at = |edge: f64| (phase - edge).abs() <= 1e-12;
                if phase > 0.0 && phase <= FALLING && !(right && at(FALLING)) {
                    assert!(
                        row[diode].abs() < 1e-9,
                        "diode current during PWM high at {t}"
                    );
                }
                if right && at(FALLING) {
                    assert_eq!(row[diode], row[il], "diode takes the current at {t}");
                }
                right_limits += usize::from(right);
                if right && (at(0.0) || at(PERIOD)) {
                    assert!(
                        row[diode].abs() < 1e-9,
                        "diode current after turn-on at {t}"
                    );
                }
                if t <= PERIOD {
                    peak = peak.max(row[il]);
                    if t > FALLING && row[il] < 1e-10 && zero.is_none() {
                        zero = Some(t);
                    }
                }
                saw_falling_edge |= t == FALLING;
                saw_rising_edge |= t == PERIOD;
            }
            assert!(saw_falling_edge && saw_rising_edge, "PWM edges absent");
            // Falling edges each period and rising edges between periods.
            assert_eq!(right_limits, 2 * cycles - 1, "right-limit rows");
            assert!(maximum_error <= 1e-9, "buck error {maximum_error}");
            assert!((peak - reference_peak).abs() <= 1e-9, "peak {peak}");
            if initial == 0.0 {
                assert!((zero.unwrap() - (FALLING + 0.1 / 3000.0)).abs() <= STEP);
            }
            println!(
                "buck {initial}A cycles={cycles} max_error={maximum_error:e}A peak={peak:.8}A zero={zero:?}s solve={elapsed:?}"
            );
        }
    }
}

#[test]
fn output_stride_preserves_grid_samples_and_the_off_grid_stop() {
    let mut value = stiff_buck(0.2);
    let step = 0.37e-6;
    let end = 100e-6;
    let all = simulate(&document(&value), || false).unwrap();
    for every in [1, 10] {
        value["simulation"]["outputStep_s"] = json!(step * every as f64);
        let sparse = simulate(&document(&value), || false).unwrap();
        let mut original = all.rows();
        for (index, row) in sparse.rows().enumerate() {
            let expected = f64::min(step * (index * every) as f64, end);
            assert_eq!(row[0], expected);
            assert_eq!(row, original.find(|sample| sample[0] == expected).unwrap());
        }
        assert_eq!(
            sparse.rows().len(),
            1 + (end / (step * every as f64)).ceil() as usize
        );
        assert_eq!(sparse.rows().last().unwrap()[0], end);
    }
}

#[test]
fn nmos_body_diode_conducts_source_to_drain_while_gate_is_low() {
    let value = json!({
        "version": 1,
        "layout": {},
        "simulation": {"timeStep_s": 0.37e-6, "stopTime_s": 100e-6},
        "parts": {
            "G": {"type": "ground", "parameters": {}},
            "Vin": {"type": "dc", "parameters": {"voltage_V": 5}},
            "R": {"type": "resistor", "parameters": {"resistance_ohm": 10}},
            "Q": {"type": "nmos", "parameters": {}},
            "PWM": {"type": "pwm", "parameters": {"frequency_Hz": 10000, "duty": 0.5, "high_V": 10}},
            "I": {"type": "ammeter", "parameters": {}},
            "V": {"type": "voltmeter", "parameters": {}}
        },
        "nets": {
            "gnd": ["G.ref", "Vin.n", "I.n", "V.p"],
            "input": ["Vin.p", "R.p"],
            "source": ["R.n", "Q.s", "PWM.ref"],
            "drain": ["Q.d", "I.p"],
            "gate": ["PWM.out", "Q.g", "V.n"]
        }
    });
    let wave = simulate(&document(&value), || false).unwrap();
    let ammeter = channel(&wave, "I_A");
    let voltmeter = channel(&wave, "V_V");
    let mut previous = f64::NAN;
    for row in wave.rows() {
        assert!((row[ammeter] - 0.5).abs() <= 1e-12);
        // The second row at the 50 µs edge is its right limit, with the gate low.
        let right = row[0] == previous;
        previous = row[0];
        let voltage = if row[0] < 50e-6 || (row[0] == 50e-6 && !right) {
            -10.0
        } else {
            0.0
        };
        assert!((row[voltmeter] - voltage).abs() <= 1e-12);
    }
}

#[test]
fn stops_between_samples_and_rejects_an_unsupported_gate_driver_before_sampling() {
    let rc = Document::from_bytes(RC.as_bytes()).unwrap();
    let mut calls = 0;
    let canceled = simulate(&rc, || {
        calls += 1;
        calls == 3
    })
    .unwrap_err();
    assert_eq!(canceled.code, "simulationCanceled");
    assert_eq!(calls, 3);

    let mut buck = stiff_buck(0.0);
    buck["parts"]["PWM"] = json!({"type": "dc", "parameters": {"voltage_V": 10}});
    buck["nets"]["gate"][0] = json!("PWM.p");
    let nodes = buck["nets"]["sw"].as_array_mut().unwrap();
    *nodes.iter_mut().find(|pin| **pin == "PWM.ref").unwrap() = json!("PWM.n");
    let error = simulate(&document(&buck), || {
        panic!("unsupported circuit entered sample loop")
    })
    .unwrap_err();
    assert_eq!(error.target_id.as_deref(), Some("Q1"));
    assert!(error.message.contains("PWM"));
}

/// Review repro: buck with a gate–source voltmeter VG. After the 50 µs falling
/// edge the gate is 0 V; the joined rows must not invent a ramp across it.
#[test]
fn switching_edges_are_recorded_on_both_sides_and_measured_as_discontinuities() {
    let mut value: Value = serde_json::from_str(BUCK).unwrap();
    record_buck_currents(&mut value);
    value["parts"]["VG"] = json!({"type": "voltmeter", "parameters": {}});
    value["nets"]["gate"]
        .as_array_mut()
        .unwrap()
        .push(json!("VG.p"));
    value["nets"]["sw"]
        .as_array_mut()
        .unwrap()
        .push(json!("VG.n"));
    let wave = simulate(&document(&value), || false).unwrap();
    let rows: Vec<_> = wave.rows().collect();
    let [vg, vc, il, diode] = ["VG_V", "VC_V", "L1_A", "D1_A"].map(|name| channel(&wave, name));
    let channels: Vec<String> = ["VG_V", "VC_V", "L1_A"].map(String::from).to_vec();

    // The first falling edge has a left (gate high) and a right (gate low) row.
    let left = (0..rows.len() - 1)
        .find(|&r| rows[r][0] == rows[r + 1][0])
        .unwrap();
    let (te, right) = (rows[left][0], left + 1);
    assert!((te - 50e-6).abs() < 1e-18);
    assert_eq!((rows[left][vg], rows[right][vg]), (10.0, 0.0));
    assert_eq!(rows[right][diode], rows[right][il]);
    for column in [vc, il] {
        assert_eq!(rows[left][column], rows[right][column], "continuous signal");
    }

    // 50.02–50.08 µs lies after the edge: the gate is 0 V throughout.
    let after = wave.measure(&channels, 50.02e-6, 50.08e-6).unwrap();
    let gate = &after.channels[0];
    for stat in [gate.average, gate.rms, gate.minimum, gate.maximum] {
        assert!(stat.unwrap().abs() <= 1e-12, "{stat:?}");
    }
    assert_eq!(gate.from.row, right);

    // Across the edge: 10 V up to te, 0 V after, so by hand mean 10·(te−a)/(b−a).
    let (a, b) = (49.95e-6, 50.05e-6);
    let across = wave.measure(&channels, a, b).unwrap();
    let gate = &across.channels[0];
    let high = (te - a) / (b - a);
    assert!((gate.average.unwrap() - 10.0 * high).abs() <= 1e-9);
    assert!((gate.rms.unwrap() - (100.0 * high).sqrt()).abs() <= 1e-9);
    assert_eq!((gate.minimum, gate.maximum), (Some(0.0), Some(10.0)));
    // A cursor exactly on the edge reads the left limit.
    assert_eq!(
        wave.measure(&channels, te, te).unwrap().channels[0]
            .from
            .row,
        left
    );

    // Continuous signals: the same straight-line statistics as without the
    // right-limit row, computed here from the rows with that row removed.
    let joined: Vec<_> = (0..rows.len()).filter(|&r| r != right).collect();
    for (index, column) in [(1, vc), (2, il)] {
        let at = |t: f64| {
            let k = joined.iter().position(|&r| rows[r][0] > t).unwrap();
            let (p, q) = (rows[joined[k - 1]], rows[joined[k]]);
            p[column] + (q[column] - p[column]) * (t - p[0]) / (q[0] - p[0])
        };
        let mut knots = vec![(a, at(a))];
        knots.extend(
            joined
                .iter()
                .filter(|&&r| rows[r][0] > a && rows[r][0] < b)
                .map(|&r| (rows[r][0], rows[r][column])),
        );
        knots.push((b, at(b)));
        let (mut area, mut square) = (0.0, 0.0);
        for pair in knots.windows(2) {
            let (h, p, q) = (pair[1].0 - pair[0].0, pair[0].1, pair[1].1);
            area += h * (p + q) / 2.0;
            square += h * (p * p + p * q + q * q) / 3.0;
        }
        let stats = &across.channels[index];
        let scale = rows.iter().map(|row| row[column].abs()).fold(0.0, f64::max);
        assert!((stats.average.unwrap() - area / (b - a)).abs() <= 1e-12 * scale);
        assert!((stats.rms.unwrap() - (square / (b - a)).sqrt()).abs() <= 1e-12 * scale);
    }
}
