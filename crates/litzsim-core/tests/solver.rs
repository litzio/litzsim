use litzsim_core::{circuit::Document, solver::simulate};
use serde_json::{Value, json};
use std::time::Instant;

const RC: &str = include_str!("../../../examples/rc.litzsim");
const RLC: &str = include_str!("../../../examples/rlc.litzsim");
const BUCK: &str = include_str!("../../../examples/buck.litzsim");

fn document(value: &Value) -> Document {
    Document::from_bytes(&serde_json::to_vec(value).unwrap()).unwrap()
}

fn channel(wave: &litzsim_core::waves::Waveform, name: &str) -> usize {
    wave.channels().iter().position(|c| c.name == name).unwrap()
}

#[test]
fn rc_and_rlc_converge_to_the_step_response() {
    for (name, example) in [("RC", RC), ("RLC", RLC)] {
        let mut value: Value = serde_json::from_str(example).unwrap();
        let mut previous = f64::INFINITY;
        for h in [10e-6, 5e-6, 2.5e-6] {
            value["simulation"]["timeStep_s"] = json!(h);
            let circuit = document(&value);
            let start = Instant::now();
            let wave = simulate(&circuit, || false).unwrap();
            let elapsed = start.elapsed();
            let vc = channel(&wave, "VC_V");
            assert_eq!(wave.rows().next().unwrap()[vc], 0.0);
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
            // The archived linear test fixed these limits before measurement.
            assert!(
                error <= 0.04 && error <= 0.55 * previous,
                "{error} after {previous}"
            );
            previous = error;
        }
    }
}

fn stiff_buck(initial: f64) -> Value {
    let mut value: Value = serde_json::from_str(BUCK).unwrap();
    value["parts"]["C1"] = json!({"type": "dc", "parameters": {"voltage_V": 3}});
    value["parts"]["L1"]["parameters"]["initialCurrent_A"] = json!(initial);
    value["simulation"] = json!({"timeStep_s": 0.37e-6, "stopTime_s": 100e-6});
    value
}

#[test]
fn buck_matches_ccm_and_dcm_through_two_pwm_periods() {
    const STEP: f64 = 0.37e-6;
    const PERIOD: f64 = 100e-6;
    const FALLING: f64 = PERIOD / 2.0;
    for initial in [0.2, 0.0] {
        let mut value = stiff_buck(initial);
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
            for row in wave.rows() {
                let t = row[0];
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
                if phase > 0.0 && phase <= FALLING {
                    assert!(
                        row[diode].abs() < 1e-9,
                        "diode current during PWM high at {t}"
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
            assert!(maximum_error <= 1e-9, "buck error {maximum_error}");
            assert!((peak - initial - 0.1).abs() <= 1e-9, "peak {peak}");
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
    for row in wave.rows() {
        assert!((row[ammeter] - 0.5).abs() <= 1e-12);
        let voltage = if row[0] <= 50e-6 { -10.0 } else { 0.0 };
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
