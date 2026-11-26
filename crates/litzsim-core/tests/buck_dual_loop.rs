use litzsim_core::{circuit::Document, solver::simulate};
use serde_json::{Value, json};
use std::time::Instant;

const EXAMPLE: &str = include_str!("../../../examples/buck-dual-loop.litzsim");
const PERIOD: f64 = 10e-6;
const CYCLES: usize = 1200;
const LOAD_STEP: usize = 600;
const TARGET: f64 = 5.0;
const LOAD_BEFORE: f64 = 2.5;
const LOAD_AFTER: f64 = 1.0 / (1.0 / LOAD_BEFORE + 1.0 / 5.0);

// Each period stores output voltage (V), inductor current (A), and duty (1).
type CycleMean = [f64; 3];
const CHANNELS: [&str; 3] = ["C1_V", "L1_A", "PI_I_out"];

struct Pi {
    kp: f64,
    ki: f64,
    integral: f64,
    limits: (f64, f64),
}

impl Pi {
    fn sample(&mut self, error: f64) -> f64 {
        let delta = self.ki * error * PERIOD;
        let candidate = self.integral + delta;
        let command = self.kp * error + candidate;
        let (min, max) = self.limits;
        if !(command > max && delta > 0.0 || command < min && delta < 0.0) {
            self.integral = candidate;
        }
        command.clamp(min, max)
    }
}

fn averaged_buck() -> Vec<CycleMean> {
    const SUBSTEPS: usize = 80;
    const H: f64 = PERIOD / SUBSTEPS as f64;
    let mut voltage_pi = Pi {
        kp: 2.212,
        ki: 8338.0,
        integral: 0.0,
        limits: (0.25, 4.0),
    };
    let mut current_pi = Pi {
        kp: 0.262,
        ki: 823.0,
        integral: 0.0,
        limits: (0.0, 0.9),
    };
    let mut state = [0.0, 0.25];
    (0..CYCLES)
        .map(|cycle| {
            let reference = (cycle as f64 * PERIOD * 2500.0).min(TARGET);
            let requested_current = voltage_pi.sample(reference - state[0]).clamp(0.25, 4.0);
            let duty = current_pi.sample(requested_current - state[1]);
            let load = if cycle < LOAD_STEP {
                LOAD_BEFORE
            } else {
                LOAD_AFTER
            };
            // Ideal CCM volt-second and charge balance, independent of the MNA solver.
            let derivative = |[voltage, current]: [f64; 2]| {
                [
                    (current - voltage / load) / 220e-6,
                    (12.0 * duty - voltage) / 100e-6,
                ]
            };
            let advance = |s: [f64; 2], slope: [f64; 2], step| {
                std::array::from_fn(|j| s[j] + slope[j] * step)
            };
            let mut sum = [0.0, 0.0];
            for _ in 0..SUBSTEPS {
                let k1 = derivative(state);
                let k2 = derivative(advance(state, k1, H / 2.0));
                let k3 = derivative(advance(state, k2, H / 2.0));
                let k4 = derivative(advance(state, k3, H));
                for j in 0..2 {
                    let next = state[j] + H / 6.0 * (k1[j] + 2.0 * k2[j] + 2.0 * k3[j] + k4[j]);
                    sum[j] += (state[j] + next) / 2.0;
                    state[j] = next;
                }
            }
            [sum[0] / SUBSTEPS as f64, sum[1] / SUBSTEPS as f64, duty]
        })
        .collect()
}

fn switching_buck(input: &Value) -> Vec<CycleMean> {
    let step = input["simulation"]["timeStep_s"].as_f64().unwrap();
    let document = Document::from_bytes(&serde_json::to_vec(&input).unwrap()).unwrap();
    let start = Instant::now();
    let wave = simulate(&document, || false).unwrap();
    let solve_time = start.elapsed();
    let names = CHANNELS.map(str::to_owned);
    let current = wave
        .channels()
        .iter()
        .position(|c| c.name == names[1])
        .unwrap();
    let minimum_current = wave
        .rows()
        .map(|row| row[current])
        .fold(f64::INFINITY, f64::min);
    assert!(
        minimum_current > 0.0,
        "h={step}: minimum current {minimum_current} A"
    );
    assert_eq!(wave.rows().last().unwrap()[0], 0.012);
    let means = (0..CYCLES)
        .map(|cycle| {
            let from = cycle as f64 * PERIOD;
            let to = (cycle + 1) as f64 * PERIOD;
            let interval = wave.measure(&names, from, to).unwrap();
            std::array::from_fn(|j| interval.channels[j].average.unwrap())
        })
        .collect();
    eprintln!(
        "h={step}: {solve_time:?} solve, {:?} with statistics, {} rows, min I={minimum_current:.6} A",
        start.elapsed(),
        wave.rows().len()
    );
    means
}

fn rmse(a: &[CycleMean], b: &[CycleMean]) -> CycleMean {
    assert_eq!(a.len(), b.len());
    std::array::from_fn(|j| {
        let squares = a.iter().zip(b).map(|(x, y)| (x[j] - y[j]).powi(2));
        (squares.sum::<f64>() / a.len() as f64).sqrt()
    })
}

fn load_response(cycles: &[CycleMean]) -> (f64, f64) {
    let minimum = cycles
        .iter()
        .map(|row| row[0])
        .fold(f64::INFINITY, f64::min);
    let first_settled = cycles
        .iter()
        .rposition(|row| (row[0] - TARGET).abs() > 0.050)
        .map_or(0, |last_outside| last_outside + 1);
    assert!(first_settled < cycles.len(), "output never settled");
    (TARGET - minimum, first_settled as f64 * PERIOD)
}

#[test]
fn dual_loop_buck_tracks_the_reference_and_recovers_from_a_ccm_load_step() {
    let start = Instant::now();
    let reference = averaged_buck();
    eprintln!("averaged buck with statistics: {:?}", start.elapsed());
    let mut input: Value = serde_json::from_str(EXAMPLE).unwrap();
    let step = input["simulation"]["timeStep_s"].as_f64().unwrap();
    assert_eq!(step, 1e-7);
    let nominal = switching_buck(&input);
    input["simulation"]["timeStep_s"] = json!(2.0 * step);
    let coarse = switching_buck(&input);
    input["simulation"]["timeStep_s"] = json!(step / 2.0);
    let fine = switching_buck(&input);

    for (window, resistance) in [(500..600, LOAD_BEFORE), (1000..1200, LOAD_AFTER)] {
        let cycles = &nominal[window.clone()];
        let mean = |j: usize| cycles.iter().map(|row| row[j]).sum::<f64>() / cycles.len() as f64;
        let (voltage, inductor) = (mean(0), mean(1));
        let load = voltage / resistance;
        eprintln!("cycles {window:?}: V={voltage:.6}, I={inductor:.6}, load={load:.6}");
        assert!((voltage - TARGET).abs() <= 0.025, "mean output {voltage} V");
        assert!(
            (inductor - load).abs() <= 0.005,
            "mean current {inductor} A, load {load} A"
        );
    }

    let switched_step = &nominal[LOAD_STEP..];
    let reference_step = &reference[LOAD_STEP..];
    let errors = rmse(switched_step, reference_step);
    let (sag, recovery) = load_response(switched_step);
    let (reference_sag, reference_recovery) = load_response(reference_step);
    eprintln!(
        "post-step RMSE {errors:?}, switched (sag, recovery) {sag:.6} V / {recovery:.6} s, average {reference_sag:.6} V / {reference_recovery:.6} s"
    );
    for ((error, limit), channel) in errors.into_iter().zip([0.050, 0.200, 0.05]).zip(CHANNELS) {
        assert!(error <= limit, "{channel} RMSE {error} > {limit}");
    }
    assert!(sag <= 0.300 && (sag - reference_sag).abs() <= 0.075);
    assert!(recovery <= 2e-3 && (recovery - reference_recovery).abs() <= 0.3e-3);

    let coarse_delta = rmse(&coarse, &nominal);
    let fine_delta = rmse(&nominal, &fine);
    eprintln!("convergence RMSE {coarse_delta:?} -> {fine_delta:?}");
    for ((before, after), channel) in coarse_delta.into_iter().zip(fine_delta).zip(CHANNELS) {
        assert!(
            after <= 0.8 * before,
            "{channel} step convergence: {before} -> {after}"
        );
    }
}
